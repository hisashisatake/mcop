//! MIDI Clock（0xF8 Timing Clock、24パルス/四分音符）からBPMを算出する。
//!
//! standaloneはDAWホストを持たないため`op505-vst`の`context.transport().tempo`に相当する
//! BPM取得手段が無く、TimeEgのテンポ同期（`sync_enabled`）が常に既定120BPM固定でしか
//! 動かなかった。Dominoのようなシーケンサーがマスターとして送るMIDI Clockを受信して
//! BPMを算出し、`Op505Engine::set_tempo()`へ配線するために使う。
//!
//! `on_clock_pulse`/`on_transport_reset`は受信スレッド（`sources::midir_src`/
//! `sources::pipe_src`）から呼ぶ。パルスの到着時刻はここで`Instant::now()`を取る必要がある
//! （cpalオーディオコールバック内で一括処理するとオーディオバッファ境界に量子化されるため）。
//! `current_bpm`はオーディオコールバック側が毎ブロック呼ぶ非ブロッキング読み出し。
//!
//! VST/smf2op505はこの受信処理を必要としない（VSTはDAWホストAPI経由、smf2op505は
//! ファイルのテンポメタを読むだけでリアルタイムのMIDI Clockという概念が無い）ため、
//! `op505-midi`/`sound-midi`のような共有クレートではなくstandalone内に閉じて実装する。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 24パルス/四分音符（MIDI Clock規格）。
const PULSES_PER_BEAT: f64 = 24.0;
/// パルス間隔の移動平均窓（直近1拍ぶん）。
const WINDOW: usize = 24;
const MIN_BPM: f32 = 20.0;
const MAX_BPM: f32 = 400.0;

/// ログに出す前回のBPM値からこれ以上変化していなければログを出さない
/// （毎パルス（48回/秒@120BPM）ログを出すと大量のI/Oになるため）。
const LOG_CHANGE_THRESHOLD_BPM: f32 = 1.0;

struct ClockState {
    last_pulse: Option<Instant>,
    recent_intervals: VecDeque<Duration>,
    last_logged_bpm: Option<f32>,
}

/// MIDI Clockパルスの到着間隔からBPMを算出し、オーディオスレッドへ橋渡しする。
pub struct TempoClock {
    state: Mutex<ClockState>,
    /// `f32::to_bits()`。0は「まだ算出できていない」を表す番兵（有効なBPMのビット表現が
    /// 0になることはない、`MIN_BPM=20.0`より小さい値は算出されないため）。
    bpm_bits: AtomicU32,
}

impl TempoClock {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ClockState { last_pulse: None, recent_intervals: VecDeque::new(), last_logged_bpm: None }),
            bpm_bits: AtomicU32::new(0),
        }
    }

    /// 0xF8（Timing Clock）受信時に呼ぶ。前回パルスとの間隔を移動平均窓へ積み、
    /// 平均間隔からBPMを算出する。値が前回ログ出力時から`LOG_CHANGE_THRESHOLD_BPM`以上
    /// 変化していれば、standaloneのログファイル（トレイメニューの「Open Log」）へ1行出す
    /// （受信スレッドから呼ばれるため、オーディオコールバック内では絶対に呼ばないこと。
    /// `crate::log`のモジュールdoc参照）。
    pub fn on_clock_pulse(&self) {
        self.on_clock_pulse_at(Instant::now());
    }

    /// テスト用に時刻を注入できる内部実装。
    fn on_clock_pulse_at(&self, now: Instant) {
        let mut to_log: Option<f32> = None;
        {
            let mut state = self.state.lock().unwrap();
            if let Some(prev) = state.last_pulse {
                let interval = now.duration_since(prev);
                state.recent_intervals.push_back(interval);
                if state.recent_intervals.len() > WINDOW {
                    state.recent_intervals.pop_front();
                }
                let count = state.recent_intervals.len();
                let total: Duration = state.recent_intervals.iter().sum();
                let avg_secs = total.as_secs_f64() / count as f64;
                if avg_secs > 0.0 {
                    let bpm = (60.0 / (avg_secs * PULSES_PER_BEAT)) as f32;
                    let bpm = bpm.clamp(MIN_BPM, MAX_BPM);
                    self.bpm_bits.store(bpm.to_bits(), Ordering::Relaxed);

                    let should_log = match state.last_logged_bpm {
                        None => true,
                        Some(prev_logged) => (bpm - prev_logged).abs() >= LOG_CHANGE_THRESHOLD_BPM,
                    };
                    if should_log {
                        state.last_logged_bpm = Some(bpm);
                        to_log = Some(bpm);
                    }
                }
            }
            state.last_pulse = Some(now);
        }
        if let Some(bpm) = to_log {
            crate::log::log(&format!("MIDI Clock受信: BPM={bpm:.1}に同期しました"));
        }
    }

    /// 0xFA(Start)/0xFB(Continue)/0xFC(Stop)受信時に呼ぶ。計測系列だけをリセットし、
    /// 直近算出済みのBPM値は保持する（Stop後もそのテンポのまま発音できるように）。
    pub fn on_transport_reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.last_pulse = None;
        state.recent_intervals.clear();
    }

    /// オーディオコールバック側の非ブロッキング読み出し。一度もクロックを受信していなければ
    /// `None`（呼び出し側は`Op505Engine::set_tempo`を呼ばず既定値のままにする）。
    pub fn current_bpm(&self) -> Option<f32> {
        let bits = self.bpm_bits.load(Ordering::Relaxed);
        if bits == 0 {
            None
        } else {
            Some(f32::from_bits(bits))
        }
    }
}

impl Default for TempoClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pulse_yet_returns_none() {
        let clock = TempoClock::new();
        assert_eq!(clock.current_bpm(), None);
    }

    #[test]
    fn single_pulse_does_not_produce_bpm() {
        let clock = TempoClock::new();
        clock.on_clock_pulse_at(Instant::now());
        assert_eq!(clock.current_bpm(), None, "間隔が1つも無いうちはBPMを算出できない");
    }

    #[test]
    fn steady_120bpm_pulses_produce_120bpm() {
        let clock = TempoClock::new();
        // 120 BPMの1拍 = 0.5秒、24パルスなので1パルス間隔 = 0.5/24秒。
        let pulse_interval = Duration::from_secs_f64(0.5 / PULSES_PER_BEAT);
        let mut now = Instant::now();
        for _ in 0..(WINDOW + 1) {
            clock.on_clock_pulse_at(now);
            now += pulse_interval;
        }
        let bpm = clock.current_bpm().expect("BPMが算出されているはず");
        assert!((bpm - 120.0).abs() < 0.01, "120BPM付近のはず: {bpm}");
    }

    #[test]
    fn steady_90bpm_pulses_produce_90bpm() {
        let clock = TempoClock::new();
        let pulse_interval = Duration::from_secs_f64((60.0 / 90.0) / PULSES_PER_BEAT);
        let mut now = Instant::now();
        for _ in 0..(WINDOW + 1) {
            clock.on_clock_pulse_at(now);
            now += pulse_interval;
        }
        let bpm = clock.current_bpm().expect("BPMが算出されているはず");
        assert!((bpm - 90.0).abs() < 0.01, "90BPM付近のはず: {bpm}");
    }

    #[test]
    fn transport_reset_clears_interval_window_but_keeps_bpm() {
        let clock = TempoClock::new();
        let pulse_interval = Duration::from_secs_f64(0.5 / PULSES_PER_BEAT);
        let mut now = Instant::now();
        for _ in 0..(WINDOW + 1) {
            clock.on_clock_pulse_at(now);
            now += pulse_interval;
        }
        let bpm_before = clock.current_bpm().expect("BPMが算出されているはず");

        clock.on_transport_reset();
        // リセット直後もBPM値自体は保持される。
        assert_eq!(clock.current_bpm(), Some(bpm_before));

        let state = clock.state.lock().unwrap();
        assert!(state.last_pulse.is_none());
        assert!(state.recent_intervals.is_empty());
    }

    #[test]
    fn extremely_fast_pulses_are_clamped_to_max_bpm() {
        let clock = TempoClock::new();
        let mut now = Instant::now();
        for _ in 0..(WINDOW + 1) {
            clock.on_clock_pulse_at(now);
            now += Duration::from_micros(1); // 極端に短い間隔
        }
        let bpm = clock.current_bpm().expect("BPMが算出されているはず");
        assert_eq!(bpm, MAX_BPM);
    }

    #[test]
    fn extremely_slow_pulses_are_clamped_to_min_bpm() {
        let clock = TempoClock::new();
        let mut now = Instant::now();
        for _ in 0..(WINDOW + 1) {
            clock.on_clock_pulse_at(now);
            now += Duration::from_secs(10); // 極端に長い間隔
        }
        let bpm = clock.current_bpm().expect("BPMが算出されているはず");
        assert_eq!(bpm, MIN_BPM);
    }

    #[test]
    fn moving_average_window_forgets_oldest_interval() {
        let clock = TempoClock::new();
        let mut now = Instant::now();
        // まず120BPM相当のパルスでウィンドウを満たす（WINDOW回の呼び出しでWINDOW-1個の間隔）。
        let interval_120 = Duration::from_secs_f64(0.5 / PULSES_PER_BEAT);
        for _ in 0..WINDOW {
            clock.on_clock_pulse_at(now);
            now += interval_120;
        }
        // ループの最後の加算は「次のパルス」のためのものなので、最終パルス時刻に戻してから
        // 60BPM相当の間隔へ切り替える（そうしないと最初の1区間だけ120BPMと60BPMが混ざる）。
        now -= interval_120;

        // 以降はすべて60BPM相当の間隔に切り替える。ウィンドウ分のパルスを送れば
        // 120BPM由来の間隔がすべて押し出され、60BPMへ収束するはず。
        let interval_60 = Duration::from_secs_f64(1.0 / PULSES_PER_BEAT);
        for _ in 0..WINDOW {
            now += interval_60;
            clock.on_clock_pulse_at(now);
        }
        let bpm = clock.current_bpm().expect("BPMが算出されているはず");
        assert!((bpm - 60.0).abs() < 0.01, "60BPMへ収束しているはず: {bpm}");
    }
}
