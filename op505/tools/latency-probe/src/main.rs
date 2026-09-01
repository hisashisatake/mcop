//! op505-standaloneとgesture-appの音色エディタ鍵盤について、「キー押下→実際にスピーカーへ
//! 音が出るまで」のレイテンシをWASAPIループバック録音でブラックボックス計測するツール。
//!
//! コード計測（各アプリ内部にタイムスタンプを仕込む方式、2026-09-01セッションで実施）と違い、
//! キー注入と録音判定をこの1プロセス内の同一クロック（`Instant`）で完結させるため、
//! プロセスをまたいだ時刻合わせが不要になる。対象アプリのソースコード・実装を一切信用しない
//! 独立した検証手段として使う（キーを押したら本当に何ms後に音が出ているか、を外部から実測する）。
//!
//! 録音は`wasapi`クレートの**プロセス単位ループバック**（`AudioClient::new_application_loopback_client`、
//! Windows 10 2004+の機能）を使う。対象プロセスのPIDだけを録音対象にできるため、システム全体の
//! ミックスを録るより他アプリ・通知音等による汚染を受けにくい（音源クレート`op505-core`/
//! `sound-core`には一切依存しない、対象アプリを外部から叩くだけの独立ツール）。
//!
//! キー注入は**マウスクリックではなくキーボードキー**（既定でZキー、`op505-standalone`/
//! `gesture-app`とも音色エディタ鍵盤の白鍵に割り当て済み）を使う。マウスクリックは画面座標に
//! 依存し誤って背後のウィンドウ/キャンバスへ抜けるリスクがあるため
//! （実際にgesture-appで連打時にエディタが閉じる事故が発生し、この方式へ切り替えた）、
//! `keybd_event`でキーコードを直接送る方式にした。毎トライアル前に`SetForegroundWindow`も
//! 強制するため、フォーカスドリフトにも強い。
//!
//! 使い方: `latency-probe --pid <対象プロセスPID> --hwnd <対象ウィンドウHWND> [--key <VKコード>]
//! [--trials N] [--hold-ms MS] [--gap-ms MS] [--preroll-ms MS]`
//!
//! PID・HWNDはこのツール自身では求めない。呼び出し側（PowerShell等）が
//! `Get-Process`/`GetWindowRect`等で対象ウィンドウを特定してから渡す
//! （`.claude/skills/gui-probe`と同じ技法）。`--key`は仮想キーコード（既定0x5A=Z）。
//!
//! 1トライアルの流れ: ①キー押下直前の静音区間からノイズフロア(baseline_rms)を測る
//! →②対象ウィンドウを前面化しキー注入（`press_instant`を記録）→③一定時間分の録音を待つ→
//! ④`press_instant`以降で音量が閾値を超えた最初の瞬間（オンセット）を探す→⑤キー押下から
//! オンセットまでの時間を記録、を`--trials`回繰り返す。トライアル間は`--gap-ms`待ち、
//! 前の音の減衰と次回のベースライン測定の両方に使う。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};

const SAMPLE_RATE: usize = 48000;
const CHANNELS: usize = 2;
/// キューから取り出す粒度（フレーム数、約10ms）。
const CHUNK_FRAMES: usize = 480;
/// 移動RMSのウィンドウ長（フレーム数、約5ms）。
const RMS_WINDOW_FRAMES: usize = 240;
/// オンセット走査時の探索ステップ（フレーム数、ウィンドウの1/4＝約1.25ms刻み）。
const SEARCH_STEP_FRAMES: usize = RMS_WINDOW_FRAMES / 4;
/// baseline_rmsがほぼ0（無音）のときの閾値の絶対フロア（線形振幅、およそ-54dBFS）。
const ABS_THRESHOLD_FLOOR: f32 = 0.002;
/// baseline_rmsに対する閾値の倍率。
const THRESHOLD_FACTOR: f32 = 5.0;

struct Args {
    pid: u32,
    hwnd: isize,
    key: u8,
    trials: u32,
    hold_ms: u64,
    gap_ms: u64,
    preroll_ms: u64,
}

fn parse_args() -> Args {
    let mut pid = None;
    let mut hwnd = None;
    let mut key = 0x5Au8; // VK_Z
    let mut trials = 10u32;
    let mut hold_ms = 300u64;
    let mut gap_ms = 1500u64;
    let mut preroll_ms = 150u64;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pid" => pid = it.next().and_then(|v| v.parse().ok()),
            "--hwnd" => hwnd = it.next().and_then(|v| v.parse().ok()),
            "--key" => {
                key = it
                    .next()
                    .and_then(|v| {
                        let v = v.trim_start_matches("0x");
                        u8::from_str_radix(v, 16).ok()
                    })
                    .unwrap_or(key)
            }
            "--trials" => trials = it.next().and_then(|v| v.parse().ok()).unwrap_or(trials),
            "--hold-ms" => hold_ms = it.next().and_then(|v| v.parse().ok()).unwrap_or(hold_ms),
            "--gap-ms" => gap_ms = it.next().and_then(|v| v.parse().ok()).unwrap_or(gap_ms),
            "--preroll-ms" => preroll_ms = it.next().and_then(|v| v.parse().ok()).unwrap_or(preroll_ms),
            other => {
                eprintln!("unknown argument: {other}");
                print_usage_and_exit();
            }
        }
    }

    let (Some(pid), Some(hwnd)) = (pid, hwnd) else {
        print_usage_and_exit();
    };
    Args { pid, hwnd, key, trials, hold_ms, gap_ms, preroll_ms }
}

fn print_usage_and_exit() -> ! {
    eprintln!(
        "usage: latency-probe --pid <PID> --hwnd <HWND> [--key <VKコード16進、既定0x5A=Z>] [--trials N] [--hold-ms MS] [--gap-ms MS] [--preroll-ms MS]"
    );
    std::process::exit(1);
}

/// キー注入用の最小限のWin32 FFI宣言（op505-standaloneの`tray.rs`/`editor/keyboard.rs`と
/// 同じ方針、必要な関数だけを直接宣言する）。
#[allow(non_snake_case)]
mod win32 {
    extern "system" {
        fn SetForegroundWindow(hWnd: isize) -> i32;
        fn keybd_event(bVk: u8, bScan: u8, dwFlags: u32, dwExtraInfo: usize);
    }

    const KEYEVENTF_KEYUP: u32 = 0x0002;

    pub fn focus(hwnd: isize) {
        unsafe { SetForegroundWindow(hwnd) };
    }

    pub fn key_down(vk: u8) {
        unsafe { keybd_event(vk, 0, 0, 0) };
    }

    pub fn key_up(vk: u8) {
        unsafe { keybd_event(vk, 0, KEYEVENTF_KEYUP, 0) };
    }
}

/// 録音スレッドとメインスレッドで共有する状態。`mono`は録音開始からの経過フレーム数を
/// そのままインデックスとして使えるモノラル包絡線（|L|と|R|の平均）。
struct Shared {
    mono: Mutex<Vec<f32>>,
    started_at: OnceLock<Instant>,
    error: Mutex<Option<String>>,
}

/// 生のインターリーブfloatバイト列をモノラル包絡線（|振幅|の平均）へ変換する。
fn decode_chunk_to_mono(bytes: &[u8], channels: usize) -> Vec<f32> {
    let frame_bytes = 4 * channels;
    let mut out = Vec::with_capacity(bytes.len() / frame_bytes.max(1));
    for frame in bytes.chunks_exact(frame_bytes) {
        let mut sum = 0.0f32;
        for ch in 0..channels {
            let o = ch * 4;
            let s = f32::from_le_bytes([frame[o], frame[o + 1], frame[o + 2], frame[o + 3]]);
            sum += s.abs();
        }
        out.push(sum / channels as f32);
    }
    out
}

fn window_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// `mono[start..]`を`SEARCH_STEP_FRAMES`刻みで走査し、移動RMSが`threshold`を初めて超えた
/// ウィンドウの開始フレーム位置を返す。
fn find_onset(mono: &[f32], start: usize, threshold: f32, window: usize) -> Option<usize> {
    let mut i = start;
    while i + window <= mono.len() {
        if window_rms(&mono[i..i + window]) > threshold {
            return Some(i);
        }
        i += SEARCH_STEP_FRAMES;
    }
    None
}

fn wait_for_frames(mono: &Mutex<Vec<f32>>, target_len: usize, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if mono.lock().unwrap().len() >= target_len {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
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
    let started_at = loop {
        if let Some(t) = shared.started_at.get() {
            break *t;
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
    };

    println!("capture started for PID {pid}, warming up...");
    thread::sleep(Duration::from_millis(args.preroll_ms.max(200)));

    let mut deltas = Vec::new();
    for trial in 1..=args.trials {
        let preroll_frames = (args.preroll_ms as usize * SAMPLE_RATE) / 1000;
        let click_frame = shared.mono.lock().unwrap().len();
        let baseline_rms = {
            let mono = shared.mono.lock().unwrap();
            let start = click_frame.saturating_sub(preroll_frames);
            window_rms(&mono[start..click_frame])
        };
        let threshold = (baseline_rms * THRESHOLD_FACTOR).max(ABS_THRESHOLD_FLOOR);

        win32::focus(args.hwnd);
        thread::sleep(Duration::from_millis(50)); // フォアグラウンド化がキー配送に間に合うための小休止
        let click_instant = Instant::now();
        win32::key_down(args.key);
        thread::sleep(Duration::from_millis(args.hold_ms));
        win32::key_up(args.key);

        let search_frames = ((args.hold_ms + 500) as usize * SAMPLE_RATE) / 1000;
        let target_len = click_frame + search_frames;
        wait_for_frames(&shared.mono, target_len, Duration::from_millis(args.hold_ms + 1500));

        let onset = {
            let mono = shared.mono.lock().unwrap();
            find_onset(&mono, click_frame, threshold, RMS_WINDOW_FRAMES)
        };

        match onset {
            Some(onset_frame) => {
                let click_secs = click_instant.duration_since(started_at).as_secs_f64();
                let onset_secs = onset_frame as f64 / SAMPLE_RATE as f64;
                let delta_ms = (onset_secs - click_secs) * 1000.0;
                println!("trial {trial}: delta_ms={delta_ms:.3} (baseline_rms={baseline_rms:.5}, threshold={threshold:.5})");
                deltas.push(delta_ms);
            }
            None => {
                println!("trial {trial}: no onset detected (baseline_rms={baseline_rms:.5}, threshold={threshold:.5})");
            }
        }

        thread::sleep(Duration::from_millis(args.gap_ms));
    }

    if deltas.is_empty() {
        println!("no successful trials");
        return;
    }
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = deltas.iter().sum::<f64>() / deltas.len() as f64;
    let median = deltas[deltas.len() / 2];
    println!("--- summary (n={}) ---", deltas.len());
    println!(
        "mean={mean:.3}ms median={median:.3}ms min={:.3}ms max={:.3}ms",
        deltas.first().unwrap(),
        deltas.last().unwrap()
    );
}
