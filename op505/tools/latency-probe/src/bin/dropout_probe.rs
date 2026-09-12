//! 持続音（和音ホールド等）の途中で不自然に無音化する「ブツ切れ」現象を、WASAPIプロセス
//! 単位ループバック録音で客観的に検出する診断ツール。`latency-probe`（キー押下→オンセット
//! までのレイテンシ計測）とは目的が異なる別バイナリだが、録音基盤（wasapiクレートの
//! process loopback capture）は同じ方式を再利用する。
//!
//! 使い方: `dropout-probe --pid <対象プロセスPID> --duration-secs <秒> [--out <出力WAVパス>]`
//!
//! 録音開始後は何もせず指定秒数ぶん録音するだけで、キー注入は行わない（発音操作は
//! 呼び出し側=PowerShell/gui-probeが別途行う想定）。録音終了後、10ms窓RMSの時系列を
//! 解析し、「直前・直後は音が出ているのに、その間だけ孤立して無音に落ちる」窓を
//! ブツ切れ（処理落ち・バッファアンダーラン）の兆候として検出する。単純な音の立ち上がり/
//! 減衰（モノトニックな変化）は誤検出しない設計。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: usize = 2;
const CHUNK_FRAMES: usize = 480;
/// 解析窓の長さ（フレーム数、10ms）。
const WINDOW_FRAMES: usize = 480;

struct Args {
    pid: u32,
    duration_secs: f64,
    out: String,
}

fn parse_args() -> Args {
    let mut pid = None;
    let mut duration_secs = 12.0;
    let mut out = "dropout_probe_out.wav".to_string();

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pid" => pid = it.next().and_then(|v| v.parse().ok()),
            "--duration-secs" => duration_secs = it.next().and_then(|v| v.parse().ok()).unwrap_or(duration_secs),
            "--out" => out = it.next().unwrap_or(out),
            other => {
                eprintln!("unknown argument: {other}");
                print_usage_and_exit();
            }
        }
    }

    let Some(pid) = pid else { print_usage_and_exit() };
    Args { pid, duration_secs, out }
}

fn print_usage_and_exit() -> ! {
    eprintln!("usage: dropout-probe --pid <PID> --duration-secs <秒、既定12> [--out <出力WAVパス>]");
    std::process::exit(1);
}

struct Shared {
    /// インターリーブ生float（L,R,L,R,...）。
    samples: Mutex<Vec<f32>>,
    started_at: OnceLock<Instant>,
    error: Mutex<Option<String>>,
}

fn decode_chunk(bytes: &[u8], channels: usize) -> Vec<f32> {
    let frame_bytes = 4 * channels;
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for frame in bytes.chunks_exact(frame_bytes) {
        for ch in 0..channels {
            let o = ch * 4;
            out.push(f32::from_le_bytes([frame[o], frame[o + 1], frame[o + 2], frame[o + 3]]));
        }
    }
    out
}

fn capture_loop(pid: u32, shared: Arc<Shared>) {
    if initialize_mta().ok().is_err() {
        *shared.error.lock().unwrap() = Some("initialize_mta failed".to_string());
        return;
    }

    let desired_format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, CHANNELS, None);
    let blockalign = desired_format.get_blockalign() as usize;

    let mut audio_client = match AudioClient::new_application_loopback_client(pid, true) {
        Ok(c) => c,
        Err(e) => {
            *shared.error.lock().unwrap() = Some(format!("new_application_loopback_client failed: {e}"));
            return;
        }
    };
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    if let Err(e) = audio_client.initialize_client(&desired_format, &Direction::Capture, &mode) {
        *shared.error.lock().unwrap() = Some(format!("initialize_client failed: {e}"));
        return;
    }
    let h_event = match audio_client.set_get_eventhandle() {
        Ok(h) => h,
        Err(e) => {
            *shared.error.lock().unwrap() = Some(format!("set_get_eventhandle failed: {e}"));
            return;
        }
    };
    let capture_client = match audio_client.get_audiocaptureclient() {
        Ok(c) => c,
        Err(e) => {
            *shared.error.lock().unwrap() = Some(format!("get_audiocaptureclient failed: {e}"));
            return;
        }
    };

    let mut sample_queue: VecDeque<u8> = VecDeque::new();
    if let Err(e) = audio_client.start_stream() {
        *shared.error.lock().unwrap() = Some(format!("start_stream failed: {e}"));
        return;
    }
    let _ = shared.started_at.set(Instant::now());

    loop {
        let new_frames = match capture_client.get_next_packet_size() {
            Ok(n) => n.unwrap_or(0),
            Err(_) => break,
        };
        if new_frames > 0 && capture_client.read_from_device_to_deque(&mut sample_queue).is_err() {
            break;
        }
        while sample_queue.len() >= blockalign * CHUNK_FRAMES {
            let mut chunk = vec![0u8; blockalign * CHUNK_FRAMES];
            for b in chunk.iter_mut() {
                *b = sample_queue.pop_front().unwrap();
            }
            let decoded = decode_chunk(&chunk, CHANNELS);
            shared.samples.lock().unwrap().extend(decoded);
        }
        if h_event.wait_for_event(3000).is_err() {
            break;
        }
    }
}

/// 32bit float PCM WAVをインターリーブ生floatから直接書き出す（hound等の追加依存を避け、
/// 標準RIFF/fmt/dataチャンクを手で組む）。
fn write_wav_f32(path: &str, interleaved: &[f32], channels: u16, sample_rate: u32) -> std::io::Result<()> {
    use std::io::Write;
    let bytes_per_sample = 4u32;
    let block_align = channels as u32 * bytes_per_sample;
    let byte_rate = sample_rate * block_align;
    let data_size = interleaved.len() as u32 * bytes_per_sample;
    let riff_size = 4 + (8 + 16) + (8 + data_size);

    let mut f = std::fs::File::create(path)?;
    f.write_all(b"RIFF")?;
    f.write_all(&riff_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?; // WAVE_FORMAT_IEEE_FLOAT
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&(block_align as u16).to_le_bytes())?;
    f.write_all(&(bytes_per_sample as u16 * 8).to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    for s in interleaved {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

fn window_rms(mono: &[f32]) -> f32 {
    if mono.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = mono.iter().map(|s| s * s).sum();
    (sum_sq / mono.len() as f32).sqrt()
}

fn main() {
    let args = parse_args();

    let shared = Arc::new(Shared { samples: Mutex::new(Vec::new()), started_at: OnceLock::new(), error: Mutex::new(None) });
    let shared_for_thread = Arc::clone(&shared);
    let pid = args.pid;
    thread::spawn(move || capture_loop(pid, shared_for_thread));

    let start_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if shared.started_at.get().is_some() {
            break;
        }
        if let Some(err) = shared.error.lock().unwrap().clone() {
            eprintln!("capture failed to start: {err}");
            std::process::exit(1);
        }
        if Instant::now() >= start_deadline {
            eprintln!("timed out waiting for capture stream to start (PID {pid})");
            std::process::exit(1);
        }
        thread::sleep(Duration::from_millis(20));
    }

    println!("capture started for PID {pid}, recording {:.1}s...", args.duration_secs);
    let target_frames = (args.duration_secs * SAMPLE_RATE as f64) as usize;
    let target_samples = target_frames * CHANNELS;
    loop {
        if shared.samples.lock().unwrap().len() >= target_samples {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let interleaved = shared.samples.lock().unwrap().clone();
    if let Err(e) = write_wav_f32(&args.out, &interleaved, CHANNELS as u16, SAMPLE_RATE) {
        eprintln!("failed to write wav: {e}");
    } else {
        println!("wrote {} ({} frames)", args.out, interleaved.len() / CHANNELS);
    }

    // モノラル包絡線 + 10ms窓RMS系列を作る。
    let mono: Vec<f32> = interleaved.chunks_exact(CHANNELS).map(|f| f.iter().map(|s| s.abs()).sum::<f32>() / CHANNELS as f32).collect();
    let num_windows = mono.len() / WINDOW_FRAMES;
    let mut rms_series = Vec::with_capacity(num_windows);
    for w in 0..num_windows {
        let start = w * WINDOW_FRAMES;
        rms_series.push(window_rms(&mono[start..start + WINDOW_FRAMES]));
    }

    let peak_rms = rms_series.iter().cloned().fold(0.0f32, f32::max);
    if peak_rms < 1e-5 {
        println!("analysis: 録音全体が無音でした（音が全く出ていない可能性）。peak_rms={peak_rms:.6}");
        return;
    }
    // 「鳴っている」判定閾値: ピークの8%（onset/release自体は誤検出しないよう緩め）。
    let sound_threshold = peak_rms * 0.08;

    // 孤立した無音窓（直前直後は鳴っているのに、間だけ沈む）を検出する。
    // 連続する無音窓をグループ化し、グループの前後がどちらも「鳴っている」窓なら
    // ブツ切れ候補として記録する。
    let mut dropouts: Vec<(usize, usize)> = Vec::new(); // (開始窓, 長さ窓数)
    let mut i = 0usize;
    while i < rms_series.len() {
        if rms_series[i] < sound_threshold {
            let start = i;
            while i < rms_series.len() && rms_series[i] < sound_threshold {
                i += 1;
            }
            let end = i; // 無音区間は [start, end)
            let has_before = start > 0 && rms_series[start - 1] >= sound_threshold;
            let has_after = end < rms_series.len() && rms_series[end] >= sound_threshold;
            if has_before && has_after {
                dropouts.push((start, end - start));
            }
        } else {
            i += 1;
        }
    }

    println!("--- analysis ---");
    println!("peak_rms={peak_rms:.5} sound_threshold={sound_threshold:.5} windows={} ({:.1}ms each)", rms_series.len(), WINDOW_FRAMES as f64 * 1000.0 / SAMPLE_RATE as f64);
    if dropouts.is_empty() {
        println!("dropouts_detected=0 (ブツ切れの兆候なし: 鳴っている区間の途中で孤立した無音窓は検出されませんでした)");
    } else {
        println!("dropouts_detected={}", dropouts.len());
        for (idx, (start, len)) in dropouts.iter().enumerate() {
            let t_ms = *start as f64 * WINDOW_FRAMES as f64 * 1000.0 / SAMPLE_RATE as f64;
            let dur_ms = *len as f64 * WINDOW_FRAMES as f64 * 1000.0 / SAMPLE_RATE as f64;
            println!("  #{idx}: t={t_ms:.1}ms duration={dur_ms:.1}ms");
        }
    }
}
