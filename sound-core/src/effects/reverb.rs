// ---------------------------------------------------------------------------
// パラメーターマッピング
// ---------------------------------------------------------------------------

/// time=0 → base（タイプごとの最小残響時間）, time=255 → max（タイプごとの最大残響時間）。
/// 単位は秒（RT60、-60dBまで減衰する時間）。
fn time_to_rt60(time: u8, base: f32, max: f32) -> f32 {
    base + (time as f32 / 255.0) * (max - base)
}

// ---------------------------------------------------------------------------
// Reverb Type
// ---------------------------------------------------------------------------

/// GM2/GS準拠のReverbタイプ（spec.md マスターエフェクトセクション参照）。
/// 宣言順 = NRPN値（0〜7）。Room1〜Plateは拡散リバーブ（FDN方式）、
/// Delay/PanningDelayはフィードバックディレイラインで実装する。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ReverbType {
    Room1,
    Room2,
    Room3,
    #[default]
    Hall1,
    Hall2,
    Plate,
    Delay,
    PanningDelay,
}

impl ReverbType {
    /// NRPN値（0〜7）からの変換。範囲外はPanningDelay（最大値）にclampする。
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => ReverbType::Room1,
            1 => ReverbType::Room2,
            2 => ReverbType::Room3,
            3 => ReverbType::Hall1,
            4 => ReverbType::Hall2,
            5 => ReverbType::Plate,
            6 => ReverbType::Delay,
            _ => ReverbType::PanningDelay,
        }
    }
}

// ---------------------------------------------------------------------------
// 共通部品：単純ディレイライン（プリディレイ・FDNの各ラインで共用）
// ---------------------------------------------------------------------------

/// フィードバックを持たない単純な循環バッファディレイ。
struct DelayLine {
    buffer: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    fn new(delay_samples: usize) -> Self {
        Self { buffer: vec![0.0; delay_samples.max(1)], pos: 0 }
    }

    fn read(&self) -> f32 {
        self.buffer[self.pos]
    }

    fn write(&mut self, value: f32) {
        self.buffer[self.pos] = value;
        self.pos = (self.pos + 1) % self.buffer.len();
    }

    fn len_samples(&self) -> usize {
        self.buffer.len()
    }
}

/// Schroederオールパスフィルター（feedback固定0.5）。入力段の拡散に使う。
struct AllpassFilter {
    buffer: Vec<f32>,
    pos: usize,
    feedback: f32,
}

impl AllpassFilter {
    fn new(delay_samples: usize) -> Self {
        Self { buffer: vec![0.0; delay_samples.max(1)], pos: 0, feedback: 0.5 }
    }

    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.pos];
        let output = buffered - input;
        self.buffer[self.pos] = input + buffered * self.feedback;
        self.pos = (self.pos + 1) % self.buffer.len();
        output
    }
}

// ---------------------------------------------------------------------------
// 拡散リバーブ（Room1〜Plate）：FDN（Feedback Delay Network）方式
//
// Freeverb（並列コム×8 + 直列オールパス×4、L/R独立）から、
// Householder行列によるFDN（8ライン、L/R共用ネットワーク）へ置き換えた。
// 動機: Freeverbは反響密度が低く、プリディレイが無く距離感が出ない、という
// 弱点があった（docs/session_history.txt参照）。FDNはHouseholder行列
// （O(N)の全和引き算のみ、行列積不要）で反響密度を上げつつ、計算量はむしろ
// 従来のL/R独立コム16本より少ない。
//
// 【切り分け済みの誤解】急峻なアタック/リリースを持つ音を通すと過渡的な
// 広帯域ノイズ（「フッ」という聴感）が乗るが、これは`reverb_probe.rs`の実測で
// FDN/Freeverbどちらでも発生する、音源側の振幅急変に起因するリバーブ一般の
// 正常な挙動と確認済み（音源の立ち上がり・切り方が滑らかならリバーブ側の
// 広帯域成分はほぼ消える）。またHouseholder行列による8ライン間の継続的な混合が
// 持続音に音量の脈動（パンピング）を生むのではという懸念も、実際の持続音での
// 試聴で確認されなかった。
// ---------------------------------------------------------------------------

/// FDNのライン数。8本あれば十分な拡散が得られ、計算量も軽い。
const FDN_LINES: usize = 8;

/// FDN各ラインのディレイ長（サンプル数, 44.1kHz基準）。Freeverb由来の
/// 互いに素に近い値をそのまま流用（単純な整数比を避け、共振の重なりを防ぐ）。
const FDN_DELAY_TUNINGS: [usize; FDN_LINES] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];

/// 入力拡散オールパスのディレイ長（サンプル数, 44.1kHz基準）。FDN本体に入る前に
/// アタックのトランジェントを2段で均す。
const INPUT_DIFFUSION_TUNINGS: [usize; 2] = [142, 107];

/// チューニング表の基準サンプルレート。
const REFERENCE_SAMPLE_RATE: f32 = 44100.0;

/// FDN入力段のゲイン。Householder行列は直交変換（エネルギー保存）なので、
/// Freeverbの並列コム総和（構造的に加算で音量が積み上がる）ほど大きい値は不要。
const INPUT_GAIN: f32 = 0.6;

struct FdnTuning {
    /// FDNディレイ長のスケール（44.1kHz基準）。値が大きいほど広い空間。
    delay_scale: f32,
    /// `time`=0のときのRT60（秒、-60dBまでの減衰時間）。
    base_rt60: f32,
    /// `time`=255のときのRT60（秒）。
    max_rt60: f32,
    /// 各ラインのダンピング量（高域減衰の強さ、フィードバック経路の1次ローパス係数）。
    damping: f32,
    /// プリディレイ（ミリ秒）。初期反射までの間＝空間の「距離感」を作る。
    predelay_ms: f32,
}

/// Room1〜Plateのチューニング表（宣言順 = ReverbTypeの宣言順0〜5）。
const FDN_TUNINGS: [FdnTuning; 6] = [
    FdnTuning { delay_scale: 0.40, base_rt60: 0.25, max_rt60: 1.0, damping: 0.50, predelay_ms: 4.0 }, // Room1
    FdnTuning { delay_scale: 0.55, base_rt60: 0.35, max_rt60: 1.4, damping: 0.45, predelay_ms: 6.0 }, // Room2
    FdnTuning { delay_scale: 0.70, base_rt60: 0.45, max_rt60: 1.8, damping: 0.40, predelay_ms: 9.0 }, // Room3
    FdnTuning { delay_scale: 1.00, base_rt60: 0.80, max_rt60: 3.2, damping: 0.35, predelay_ms: 18.0 }, // Hall1
    FdnTuning { delay_scale: 1.30, base_rt60: 1.20, max_rt60: 4.5, damping: 0.25, predelay_ms: 28.0 }, // Hall2
    FdnTuning { delay_scale: 0.85, base_rt60: 0.60, max_rt60: 2.6, damping: 0.10, predelay_ms: 3.0 }, // Plate
];

/// ディレイ長とRT60から、そのラインのフィードバックゲインを求める。
fn rt60_to_gain(delay_samples: usize, sample_rate: f32, rt60: f32) -> f32 {
    let delay_sec = delay_samples as f32 / sample_rate;
    10f32.powf(-3.0 * delay_sec / rt60.max(0.001))
}

/// Householder行列（[1,1,...,1]基準の鏡映変換）によるFDN拡散リバーブ。
/// L/Rで独立したネットワークを持たず、モノラルに落とした入力を1本のFDNへ通し、
/// 出力タップ（偶数ライン→L、奇数ライン→R）の違いだけでステレオ感を作る。
struct FdnReverb {
    predelay: DelayLine,
    diffuser1: AllpassFilter,
    diffuser2: AllpassFilter,
    lines: Vec<DelayLine>,
    filter_store: [f32; FDN_LINES],
    damping: f32,
    gains: [f32; FDN_LINES],
    sample_rate: f32,
    base_rt60: f32,
    max_rt60: f32,
}

impl FdnReverb {
    fn new(sample_rate: f32, tuning: &FdnTuning, time: u8) -> Self {
        let sr_scale = sample_rate / REFERENCE_SAMPLE_RATE;
        let predelay = DelayLine::new((tuning.predelay_ms / 1000.0 * sample_rate) as usize);
        let diffuser1 = AllpassFilter::new((INPUT_DIFFUSION_TUNINGS[0] as f32 * sr_scale) as usize);
        let diffuser2 = AllpassFilter::new((INPUT_DIFFUSION_TUNINGS[1] as f32 * sr_scale) as usize);

        let lines: Vec<DelayLine> = FDN_DELAY_TUNINGS
            .iter()
            .map(|&len| DelayLine::new((len as f32 * tuning.delay_scale * sr_scale) as usize))
            .collect();

        let rt60 = time_to_rt60(time, tuning.base_rt60, tuning.max_rt60);
        let mut gains = [0.0; FDN_LINES];
        for (i, line) in lines.iter().enumerate() {
            gains[i] = rt60_to_gain(line.len_samples(), sample_rate, rt60);
        }

        Self {
            predelay,
            diffuser1,
            diffuser2,
            lines,
            filter_store: [0.0; FDN_LINES],
            damping: tuning.damping,
            gains,
            sample_rate,
            base_rt60: tuning.base_rt60,
            max_rt60: tuning.max_rt60,
        }
    }

    /// `time`に応じて各ラインのフィードバックゲインのみ更新する（バッファは再確保しない）。
    fn set_time(&mut self, time: u8) {
        let rt60 = time_to_rt60(time, self.base_rt60, self.max_rt60);
        for (i, line) in self.lines.iter().enumerate() {
            self.gains[i] = rt60_to_gain(line.len_samples(), self.sample_rate, rt60);
        }
    }

    fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let mono_in = (in_l + in_r) * 0.5;
        let predelayed = self.predelay.read();
        self.predelay.write(mono_in);
        let diffused = self.diffuser2.process(self.diffuser1.process(predelayed));

        let mut s = [0.0; FDN_LINES];
        for (i, line) in self.lines.iter().enumerate() {
            s[i] = line.read();
        }

        let mut out_l = 0.0;
        let mut out_r = 0.0;
        for (i, &v) in s.iter().enumerate() {
            if i % 2 == 0 {
                out_l += v;
            } else {
                out_r += v;
            }
        }
        let tap_count = (FDN_LINES / 2) as f32;
        out_l /= tap_count;
        out_r /= tap_count;

        for (i, &v) in s.iter().enumerate() {
            self.filter_store[i] = v * (1.0 - self.damping) + self.filter_store[i] * self.damping;
        }
        let sum_all: f32 = self.filter_store.iter().sum();
        let two_over_n = 2.0 / FDN_LINES as f32;
        for (i, line) in self.lines.iter_mut().enumerate() {
            let mixed = self.filter_store[i] - two_over_n * sum_all;
            line.write(diffused * INPUT_GAIN + mixed * self.gains[i]);
        }

        (out_l, out_r)
    }
}

// ---------------------------------------------------------------------------
// フィードバックディレイリバーブ（Delay / Panning Delay）
// ---------------------------------------------------------------------------

const DELAY_TIME_MIN_MS: f32 = 200.0;
const DELAY_TIME_MAX_MS: f32 = 800.0;
const DELAY_FEEDBACK_MIN: f32 = 0.3;
const DELAY_FEEDBACK_MAX: f32 = 0.85;

/// フィードバックディレイライン。`panning=true`の場合、L入力をRchへ、
/// R入力をLchへ交互にフィードバックするピンポンディレイになる。
struct DelayReverb {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    pos: usize,
    feedback: f32,
    panning: bool,
}

impl DelayReverb {
    fn new(sample_rate: f32, time: u8, panning: bool) -> Self {
        let delay_ms = DELAY_TIME_MIN_MS + (time as f32 / 255.0) * (DELAY_TIME_MAX_MS - DELAY_TIME_MIN_MS);
        let len = (delay_ms / 1000.0 * sample_rate) as usize + 1;
        let feedback = DELAY_FEEDBACK_MIN + (time as f32 / 255.0) * (DELAY_FEEDBACK_MAX - DELAY_FEEDBACK_MIN);
        Self { buffer_l: vec![0.0; len], buffer_r: vec![0.0; len], pos: 0, feedback, panning }
    }

    fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let out_l = self.buffer_l[self.pos];
        let out_r = self.buffer_r[self.pos];

        if self.panning {
            // L入力の遅延出力をRchへ、R入力の遅延出力をLchへ書き戻す（ピンポン）。
            self.buffer_r[self.pos] = in_l + out_l * self.feedback;
            self.buffer_l[self.pos] = in_r + out_r * self.feedback;
        } else {
            self.buffer_l[self.pos] = in_l + out_l * self.feedback;
            self.buffer_r[self.pos] = in_r + out_r * self.feedback;
        }

        self.pos = (self.pos + 1) % self.buffer_l.len();
        (out_l, out_r)
    }
}

// ---------------------------------------------------------------------------
// Reverb
// ---------------------------------------------------------------------------

enum ReverbAlgorithm {
    Fdn(FdnReverb),
    Delay(DelayReverb),
}

fn build_algorithm(sample_rate: f32, reverb_type: ReverbType, time: u8) -> ReverbAlgorithm {
    match reverb_type {
        ReverbType::Delay => ReverbAlgorithm::Delay(DelayReverb::new(sample_rate, time, false)),
        ReverbType::PanningDelay => ReverbAlgorithm::Delay(DelayReverb::new(sample_rate, time, true)),
        _ => {
            let tuning = &FDN_TUNINGS[reverb_type as usize];
            ReverbAlgorithm::Fdn(FdnReverb::new(sample_rate, tuning, time))
        }
    }
}

/// マスターリバーブ。`ReverbType`に応じて拡散リバーブ(FDN)/フィードバックディレイの
/// いずれかのアルゴリズムを内部に保持する。
pub struct Reverb {
    sample_rate: f32,
    reverb_type: ReverbType,
    time: u8,
    algorithm: ReverbAlgorithm,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let reverb_type = ReverbType::default();
        let time = 128;
        let algorithm = build_algorithm(sample_rate, reverb_type, time);
        Self { sample_rate, reverb_type, time, algorithm }
    }

    /// タイプを切り替える。内部アルゴリズムを再構築するため、残響テールはリセットされる。
    pub fn set_type(&mut self, reverb_type: ReverbType) {
        self.reverb_type = reverb_type;
        self.algorithm = build_algorithm(self.sample_rate, reverb_type, self.time);
    }

    /// 残響時間を設定する。FDNはフィードバックゲインのみ更新（再構築なし）、
    /// ディレイ系はディレイ長自体が変わるため再構築する。
    pub fn set_time(&mut self, time: u8) {
        self.time = time;
        match &mut self.algorithm {
            ReverbAlgorithm::Fdn(reverb) => reverb.set_time(time),
            ReverbAlgorithm::Delay(_) => {
                self.algorithm = build_algorithm(self.sample_rate, self.reverb_type, time);
            }
        }
    }

    /// 1サンプル処理する。
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        match &mut self.algorithm {
            ReverbAlgorithm::Fdn(reverb) => reverb.process(in_l, in_r),
            ReverbAlgorithm::Delay(reverb) => reverb.process(in_l, in_r),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_type_is_hall1() {
        assert_eq!(ReverbType::default(), ReverbType::Hall1);
    }

    #[test]
    fn from_u8_mapping() {
        assert_eq!(ReverbType::from_u8(0), ReverbType::Room1);
        assert_eq!(ReverbType::from_u8(3), ReverbType::Hall1);
        assert_eq!(ReverbType::from_u8(7), ReverbType::PanningDelay);
        assert_eq!(ReverbType::from_u8(255), ReverbType::PanningDelay);
    }

    #[test]
    fn diffuse_impulse_creates_decaying_tail() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Hall1);

        let (first_l, _) = reverb.process(1.0, 1.0);
        assert!(first_l.abs() < 1.0, "インパルス直後の出力は入力より小さいはず: {first_l}");

        let mut tail_energy = 0.0;
        for _ in 0..44100 {
            let (l, r) = reverb.process(0.0, 0.0);
            tail_energy += l * l + r * r;
        }
        assert!(tail_energy > 0.0, "テールが減衰しきって無音になっている");
    }

    #[test]
    fn no_nan_long_run() {
        for &t in &[
            ReverbType::Room1,
            ReverbType::Hall1,
            ReverbType::Hall2,
            ReverbType::Plate,
            ReverbType::Delay,
            ReverbType::PanningDelay,
        ] {
            let mut reverb = Reverb::new(44100.0);
            reverb.set_type(t);
            reverb.set_time(255);
            for i in 0..(44100 * 2) {
                let input = if i % 4410 == 0 { 1.0 } else { 0.0 };
                let (l, r) = reverb.process(input, -input);
                assert!(l.is_finite() && r.is_finite(), "{t:?}: 発散またはNaN: {l}, {r}");
                assert!(l.abs() < 100.0 && r.abs() < 100.0, "{t:?}: 発散している: {l}, {r}");
            }
        }
    }

    #[test]
    fn delay_type_produces_echo() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Delay);
        reverb.set_time(0); // 最短ディレイ（200ms）

        let (first_l, _) = reverb.process(1.0, 1.0);
        assert_eq!(first_l, 0.0, "ディレイ直後はまだエコーが返ってこないはず");

        let delay_samples = (DELAY_TIME_MIN_MS / 1000.0 * 44100.0) as usize;
        let mut found_at = None;
        for i in 1..delay_samples + 10 {
            let (l, _) = reverb.process(0.0, 0.0);
            if l.abs() > 1e-6 {
                found_at = Some(i);
                break;
            }
        }
        let i = found_at.expect("ディレイのエコーが検出できない");
        assert!((i as i64 - delay_samples as i64).abs() <= 5, "エコーのタイミングがおかしい: i={i}, expected≈{delay_samples}");
    }

    #[test]
    fn panning_delay_crosses_channels() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::PanningDelay);
        reverb.set_time(0);

        reverb.process(1.0, 0.0);

        let delay_samples = (DELAY_TIME_MIN_MS / 1000.0 * 44100.0) as usize;
        for _ in 0..delay_samples {
            reverb.process(0.0, 0.0);
        }
        // Lに入力した信号は、1ディレイ周期後にRchから出力される（ピンポン）
        let (out_l, out_r) = reverb.process(0.0, 0.0);
        assert!(out_r.abs() > out_l.abs(), "PanningDelayはL入力をRに出すはず: l={out_l}, r={out_r}");
    }

    #[test]
    fn fdn_predelay_delays_onset() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Hall1);

        let predelay_samples = (18.0 / 1000.0 * 44100.0) as usize;
        for i in 0..predelay_samples {
            let (l, r) = reverb.process(if i == 0 { 1.0 } else { 0.0 }, if i == 0 { 1.0 } else { 0.0 });
            assert_eq!(l, 0.0, "プリディレイ中に音が出ている: i={i}, l={l}");
            assert_eq!(r, 0.0, "プリディレイ中に音が出ている: i={i}, r={r}");
        }

        let mut found = false;
        for _ in 0..4000 {
            let (l, r) = reverb.process(0.0, 0.0);
            if l.abs() > 1e-6 || r.abs() > 1e-6 {
                found = true;
                break;
            }
        }
        assert!(found, "プリディレイ後にテールが立ち上がらない");
    }
}
