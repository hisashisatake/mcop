//! 対象プロセス（PID指定）のWASAPIプロセス単位ループバック音声を一定時間録音し、
//! モノラルf32のWAVファイルへ書き出すツール。`op505/tools/latency-probe`のcapture_loop
//! （キー注入レイテンシ計測用）から録音基盤だけを抜き出した用途違い版。
//!
//! MIDI CC/NRPNの演奏効果（ビブラート＝Pitch FG、トレモロ＝Gain FG）を「聴感でなく録音物から
//! 数学的に検証したい」というケース向け（memory `project_editor_keyboard_latency_comparison.md`
//! で想定されていた再利用）。本ツール自身はMIDI送信を行わない。呼び出し側（Pythonスクリプト等）が
//! 別プロセスでMIDIを送るため、標準出力へ`capture started`を1行出してから一定のpreroll区間だけ
//! 待つことでおおよそのタイミングを合わせる（サンプル精度が要る場合は書き出したWAVのRMS包絡線から
//! オンセットを再検出すればよい、`latency-probe`のfind_onsetと同じ手法）。
//!
//! 使い方: `loopback-record --pid <対象プロセスPID> --duration-ms <MS> --out <出力WAVパス>
//! [--preroll-ms MS]`

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};

const SAMPLE_RATE: usize = 48000;
const CHANNELS: usize = 2;
/// キューから取り出す粒度（フレーム数、約10ms、`latency-probe`と同じ）。
const CHUNK_FRAMES: usize = 480;

struct Args {
    pid: u32,
    duration_ms: u64,
    out: String,
    preroll_ms: u64,
}

fn parse_args() -> Args {
    let mut pid = None;
    let mut duration_ms = None;
    let mut out = None;
    let mut preroll_ms = 300u64;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pid" => pid = it.next().and_then(|v| v.parse().ok()),
            "--duration-ms" => duration_ms = it.next().and_then(|v| v.parse().ok()),
            "--out" => out = it.next(),
            "--preroll-ms" => preroll_ms = it.next().and_then(|v| v.parse().ok()).unwrap_or(preroll_ms),
            other => {
                eprintln!("unknown argument: {other}");
                print_usage_and_exit();
            }
        }
    }

    let (Some(pid), Some(duration_ms), Some(out)) = (pid, duration_ms, out) else {
        print_usage_and_exit();
    };
    Args { pid, duration_ms, out, preroll_ms }
}

fn print_usage_and_exit() -> ! {
    eprintln!(
        "usage: loopback-record --pid <PID> --duration-ms <MS> --out <path.wav> [--preroll-ms MS(既定300)]"
    );
    std::process::exit(1);
}

/// 録音スレッドとメインスレッドで共有する状態。`mono`はL/Rを符号付きのまま平均した
/// モノラル波形（`latency-probe`の|振幅|平均な包絡線と違い、F0追跡等で実波形を使うため符号を保つ）。
struct Shared {
    mono: Mutex<Vec<f32>>,
    started_at: OnceLock<Instant>,
    error: Mutex<Option<String>>,
}

fn decode_chunk_to_mono(bytes: &[u8], channels: usize) -> Vec<f32> {
    let frame_bytes = 4 * channels;
    let mut out = Vec::with_capacity(bytes.len() / frame_bytes.max(1));
    for frame in bytes.chunks_exact(frame_bytes) {
        let mut sum = 0.0f32;
        for ch in 0..channels {
            let o = ch * 4;
            let s = f32::from_le_bytes([frame[o], frame[o + 1], frame[o + 2], frame[o + 3]]);
            sum += s;
        }
        out.push(sum / channels as f32);
    }
    out
}

fn capture_loop(pid: u32, shared: Arc<Shared>) {
    if initialize_mta().ok().is_err() {
        *shared.error.lock().unwrap() = Some("initialize_mta failed".to_string());
        return;
    }

    let desired_format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE, CHANNELS, None);
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
            let decoded = decode_chunk_to_mono(&chunk, CHANNELS);
            shared.mono.lock().unwrap().extend(decoded);
        }
        if h_event.wait_for_event(3000).is_err() {
            break;
        }
    }
}

fn main() {
    let args = parse_args();

    let shared = Arc::new(Shared { mono: Mutex::new(Vec::new()), started_at: OnceLock::new(), error: Mutex::new(None) });
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
            eprintln!("timed out waiting for capture stream to start (PID {pid} might not be producing audio yet, or process-loopback capture is unsupported on this Windows version)");
            std::process::exit(1);
        }
        thread::sleep(Duration::from_millis(20));
    }

    // 呼び出し側（MIDI送信スクリプト等）へタイミングの目印を渡す。preroll区間は
    // 「録音は始まっているが対象イベントはまだ起きていない」区間として波形の先頭に残る。
    println!("capture started for PID {pid}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    thread::sleep(Duration::from_millis(args.preroll_ms));

    let target_frames = (args.duration_ms as usize * SAMPLE_RATE) / 1000;
    let deadline = Instant::now() + Duration::from_millis(args.duration_ms + 2000);
    loop {
        if shared.mono.lock().unwrap().len() >= target_frames {
            break;
        }
        if Instant::now() >= deadline {
            eprintln!("warning: timed out waiting for {target_frames} frames, writing what was captured");
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    let mono = shared.mono.lock().unwrap();
    let samples: &[f32] = if mono.len() >= target_frames { &mono[..target_frames] } else { &mono[..] };

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = match hound::WavWriter::create(&args.out, spec) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("failed to create wav writer for {}: {e}", args.out);
            std::process::exit(1);
        }
    };
    for &s in samples {
        if let Err(e) = writer.write_sample(s) {
            eprintln!("failed to write sample: {e}");
            std::process::exit(1);
        }
    }
    if let Err(e) = writer.finalize() {
        eprintln!("failed to finalize wav: {e}");
        std::process::exit(1);
    }
    println!("wrote {} frames ({:.3}s) to {}", samples.len(), samples.len() as f64 / SAMPLE_RATE as f64, args.out);
}
