use crate::time_eg::sync_rate_beats;

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
//
// テンポ同期対応（2026-09-04）：ディレイ長は「BPM×指定音価」または従来通り`time`から
// 決まるが、長さが変わるたびにバッファを作り直すと鳴っているエコーが消えてしまう。
// MIDI Clock由来のBPMは常時わずかに揺れる（`project_bpm_supply_midi_clock`参照）ため、
// バッファは`DELAY_MAX_SECONDS`分を固定確保しておき、書き込み位置は動かさず読み出し位置
// （ディレイ長）だけを動かす設計にした。読み出し位置の変更は必ず`DELAY_CROSSFADE_SECONDS`
// かけて旧タップ→新タップへ線形クロスフェードする（作り直し＝瞬時切り替えを廃止）。
//
// BPM由来の変更だけは「ロック中の値から`DELAY_RELOCK_BAND`以上ズレた候補が
// `DELAY_RELOCK_CONFIRM_SECONDS`続いたら」再ロック＋フェード開始、という
// ヒステリシスを掛ける（クロック揺らぎで無駄にフェードし続けるのを防ぐ）。
// 一方、`time`ノブ・SYNC ON/OFF・sync_rateの変更はユーザー操作なので確認時間を待たず
// 即座にフェードを開始する（`retarget_immediate`）。
// ---------------------------------------------------------------------------

const DELAY_TIME_MIN_MS: f32 = 200.0;
const DELAY_TIME_MAX_MS: f32 = 800.0;
const DELAY_FEEDBACK_MIN: f32 = 0.3;
const DELAY_FEEDBACK_MAX: f32 = 0.85;

/// 同期時のディレイ長上限（秒）。これを超える音価（例: 遅いテンポの4/1）は、
/// 収まるまで半分に折り返す（音価のグリッド上に留めたまま長さだけ縮める）。
const DELAY_MAX_SECONDS: f32 = 4.0;
/// BPM由来の再ロックを確定させるバンド（ロック中の値からの相対値）。
const DELAY_RELOCK_BAND: f32 = 0.015;
/// バンド外の候補がこの秒数連続したら再ロックする。
const DELAY_RELOCK_CONFIRM_SECONDS: f32 = 0.25;
/// ディレイ長切り替え時のクロスフェード時間（秒）。
const DELAY_CROSSFADE_SECONDS: f32 = 0.03;

fn time_to_feedback(time: u8) -> f32 {
    DELAY_FEEDBACK_MIN + (time as f32 / 255.0) * (DELAY_FEEDBACK_MAX - DELAY_FEEDBACK_MIN)
}

/// フィードバック経路のダンピング量（ワンポールローパス係数、`FdnReverb::damping`と同じ式）。
/// 実測（`fx_quality_probe`/`analyze_fx2.py`診断、後で撤去済み）で確認した通り、
/// 従来はフィードバック信号に一切のフィルタが無く、エコーが何回巡回しても波形が
/// 変化しない（連続エコーの振幅比が終始厳密に一定）ことが判明していた。実機の
/// テープ/BBDディレイは繰り返すごとに高域が落ちるのが普通で、これが無いと
/// 高フィードバック時に全帯域のエコーが積み重なって濁って聴こえる一因になる
/// （ユーザー報告「短いループでディレイが積み重なって濁る」に対応）。
/// FDNのRoom1(0.50)〜Plate(0.10)の中間程度の値を固定で使う（空間タイプの概念を
/// 持たないDelay/PanningDelayに対し、ユーザー調整可能なパラメーターを新設するほどの
/// 柔軟性は不要と判断）。
const DELAY_DAMPING: f32 = 0.35;

/// ディレイ長切り替え中のクロスフェード状態（`from`→`to`、0.0〜1.0で進行）。
struct DelayFade {
    from_samples: usize,
    to_samples: usize,
    progress: f32,
    increment: f32,
}

/// フィードバックディレイライン。`panning=true`の場合、L入力をRchへ、
/// R入力をLchへ交互にフィードバックするピンポンディレイになる。
///
/// バッファは`DELAY_MAX_SECONDS`分を固定確保する（タイプ切り替え時のみ確保、
/// 以降は書き込み位置を巡回させるだけで作り直さない）。
struct DelayReverb {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    feedback: f32,
    panning: bool,
    sample_rate: f32,

    time: u8,
    sync_enabled: u8,
    sync_rate: u8,
    bpm: f32,

    /// フェード中でないときに実際に読み出しているディレイ長（サンプル）。
    active_delay_samples: usize,
    fade: Option<DelayFade>,

    /// BPM由来の再ロック用ヒステリシス。「現在ロックしている目標秒数」と、
    /// バンド外に出た候補とその継続秒数（`None`なら候補なし＝安定）。
    locked_seconds: f32,
    pending: Option<(f32, f32)>,

    /// フィードバックダンピング（`DELAY_DAMPING`）用のワンポールローパス状態。
    /// 読み出し値そのもの（`out_l`/`out_r`、今回の出力）ではなく、次の周回へ
    /// 書き戻す信号にのみ掛ける（＝今聴こえているエコーの音質はそのまま、
    /// 次に返ってくるエコーから徐々に暗くなる）。
    damping_state_l: f32,
    damping_state_r: f32,
}

impl DelayReverb {
    fn new(sample_rate: f32, time: u8, panning: bool, sync_enabled: u8, sync_rate: u8, bpm: f32) -> Self {
        // +2は端数切り捨て・フェード時の境界アクセスに対する安全マージン。
        let capacity = (DELAY_MAX_SECONDS * sample_rate) as usize + 2;
        let mut reverb = Self {
            buffer_l: vec![0.0; capacity],
            buffer_r: vec![0.0; capacity],
            write_pos: 0,
            feedback: time_to_feedback(time),
            panning,
            sample_rate,
            time,
            sync_enabled,
            sync_rate,
            bpm: bpm.max(1.0),
            active_delay_samples: 1,
            fade: None,
            locked_seconds: 0.0,
            pending: None,
            damping_state_l: 0.0,
            damping_state_r: 0.0,
        };
        let target = reverb.raw_target_seconds();
        reverb.locked_seconds = target;
        reverb.active_delay_samples = reverb.seconds_to_samples(target);
        reverb
    }

    /// 秒→サンプル数。旧実装（作り直し方式）の`(delay_ms/1000.0*sample_rate) as usize + 1`と
    /// 同じ「切り捨て＋1」規約をそのまま踏襲する（末尾コメント参照）。既存の
    /// `delay_type_produces_echo`/`panning_delay_crosses_channels`は「ちょうどdelay_samples回後に
    /// 元の書き込み位置を再び読む」ことを前提にしたタイミングテストで、+1が無いと1サンプル
    /// 早く一致してしまいテストの土台が崩れる（新方式でも旧方式と挙動を合わせるためそのまま維持）。
    fn seconds_to_samples(&self, seconds: f32) -> usize {
        let raw = (seconds * self.sample_rate) as usize + 1;
        raw.clamp(1, self.buffer_l.len() - 1)
    }

    /// 現在のパラメーター（`sync_enabled`に応じて`time`かBPM×音価）から求まる、
    /// フェード等を考慮しない素のディレイ長（秒）。
    fn raw_target_seconds(&self) -> f32 {
        if self.sync_enabled != 0 {
            let mut secs = sync_rate_beats(self.sync_rate) * 60.0 / self.bpm.max(1.0);
            while secs > DELAY_MAX_SECONDS {
                secs *= 0.5;
            }
            secs.max(0.001)
        } else {
            (DELAY_TIME_MIN_MS + (self.time as f32 / 255.0) * (DELAY_TIME_MAX_MS - DELAY_TIME_MIN_MS)) / 1000.0
        }
    }

    /// フェードの有無に関わらず現在読み出しているディレイ長の推定値（サンプル）。
    /// フェード中に新たな変更が来たとき、その時点の中間値から次のフェードを始めるために使う。
    fn current_delay_estimate(&self) -> usize {
        match &self.fade {
            Some(f) => {
                let t = f.progress.min(1.0);
                let blended = f.from_samples as f32 + (f.to_samples as f32 - f.from_samples as f32) * t;
                (blended.round() as i64).clamp(1, (self.buffer_l.len() - 1) as i64) as usize
            }
            None => self.active_delay_samples,
        }
    }

    fn start_fade(&mut self, to_samples: usize) {
        let to_samples = to_samples.clamp(1, self.buffer_l.len() - 1);
        let from_samples = self.current_delay_estimate();
        if from_samples == to_samples {
            self.fade = None;
            self.active_delay_samples = to_samples;
            return;
        }
        let fade_len_samples = (DELAY_CROSSFADE_SECONDS * self.sample_rate).max(1.0);
        self.fade = Some(DelayFade { from_samples, to_samples, progress: 0.0, increment: 1.0 / fade_len_samples });
    }

    /// ユーザー操作由来のパラメーター変更用。確認時間を待たず即座にフェードを開始する。
    fn retarget_immediate(&mut self, target_seconds: f32) {
        self.locked_seconds = target_seconds;
        self.pending = None;
        let to_samples = self.seconds_to_samples(target_seconds);
        self.start_fade(to_samples);
    }

    /// NRPN(0,4) Reverb Time相当。同期無効時はディレイ長＋フィードバックの両方を、
    /// 同期有効時はフィードバックのみを更新する（ディレイ長はBPM×音価が決める）。
    fn set_time(&mut self, time: u8) {
        self.time = time;
        self.feedback = time_to_feedback(time);
        if self.sync_enabled == 0 {
            let target = self.raw_target_seconds();
            self.retarget_immediate(target);
        }
    }

    fn set_sync_enabled(&mut self, value: u8) {
        if self.sync_enabled == value {
            return;
        }
        self.sync_enabled = value;
        let target = self.raw_target_seconds();
        self.retarget_immediate(target);
    }

    fn set_sync_rate(&mut self, rate: u8) {
        if self.sync_rate == rate {
            return;
        }
        self.sync_rate = rate;
        if self.sync_enabled != 0 {
            let target = self.raw_target_seconds();
            self.retarget_immediate(target);
        }
    }

    /// BPM更新。同期無効時は値を保持するだけ（次に同期を有効にしたときに使う）。
    /// 同期有効時は即座に切り替えず、`DELAY_RELOCK_BAND`を`DELAY_RELOCK_CONFIRM_SECONDS`
    /// 連続で外れたときだけ`process()`側でフェードを開始する（ヒステリシス）。
    fn set_tempo(&mut self, bpm: f32) {
        if bpm <= 0.0 {
            return;
        }
        self.bpm = bpm;
        if self.sync_enabled == 0 {
            self.pending = None;
            return;
        }
        let candidate = self.raw_target_seconds();
        let band = self.locked_seconds * DELAY_RELOCK_BAND;
        if (candidate - self.locked_seconds).abs() <= band {
            self.pending = None;
            return;
        }
        match &mut self.pending {
            Some((value, _)) => *value = candidate,
            None => self.pending = Some((candidate, 0.0)),
        }
    }

    fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        // BPM由来の候補が確認時間を満たしたら再ロックしてフェードを開始する。
        if let Some((candidate, elapsed)) = &mut self.pending {
            *elapsed += 1.0 / self.sample_rate;
            if *elapsed >= DELAY_RELOCK_CONFIRM_SECONDS {
                let candidate = *candidate;
                self.pending = None;
                self.locked_seconds = candidate;
                let to_samples = self.seconds_to_samples(candidate);
                self.start_fade(to_samples);
            }
        }

        let capacity = self.buffer_l.len();
        let (out_l, out_r) = match &mut self.fade {
            Some(fade) => {
                let idx_from = (self.write_pos + capacity - fade.from_samples) % capacity;
                let idx_to = (self.write_pos + capacity - fade.to_samples) % capacity;
                let t = fade.progress.min(1.0);
                let l = self.buffer_l[idx_from] + (self.buffer_l[idx_to] - self.buffer_l[idx_from]) * t;
                let r = self.buffer_r[idx_from] + (self.buffer_r[idx_to] - self.buffer_r[idx_from]) * t;

                fade.progress += fade.increment;
                if fade.progress >= 1.0 {
                    self.active_delay_samples = fade.to_samples;
                    self.fade = None;
                }
                (l, r)
            }
            None => {
                let idx = (self.write_pos + capacity - self.active_delay_samples) % capacity;
                (self.buffer_l[idx], self.buffer_r[idx])
            }
        };

        // フィードバックへ戻す信号だけをワンポールローパスへ通す（`FdnReverb`のダンピングと
        // 同じ式）。`out_l`/`out_r`＝今回返す出力そのものは素通しのまま変えない。
        self.damping_state_l = out_l * (1.0 - DELAY_DAMPING) + self.damping_state_l * DELAY_DAMPING;
        self.damping_state_r = out_r * (1.0 - DELAY_DAMPING) + self.damping_state_r * DELAY_DAMPING;

        if self.panning {
            // L入力の遅延出力をRchへ、R入力の遅延出力をLchへ書き戻す（ピンポン）。
            self.buffer_r[self.write_pos] = in_l + self.damping_state_l * self.feedback;
            self.buffer_l[self.write_pos] = in_r + self.damping_state_r * self.feedback;
        } else {
            self.buffer_l[self.write_pos] = in_l + self.damping_state_l * self.feedback;
            self.buffer_r[self.write_pos] = in_r + self.damping_state_r * self.feedback;
        }

        self.write_pos = (self.write_pos + 1) % capacity;
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

/// Delay/PanningDelayのテンポ同期既定レート＝1/4（1拍）。TimeEgの`sync_rate`既定と揃える。
fn default_delay_sync_rate() -> u8 {
    crate::time_eg::sync_note_anchor(10)
}

#[allow(clippy::too_many_arguments)]
fn build_algorithm(
    sample_rate: f32,
    reverb_type: ReverbType,
    time: u8,
    delay_sync_enabled: u8,
    delay_sync_rate: u8,
    bpm: f32,
) -> ReverbAlgorithm {
    match reverb_type {
        ReverbType::Delay => {
            ReverbAlgorithm::Delay(DelayReverb::new(sample_rate, time, false, delay_sync_enabled, delay_sync_rate, bpm))
        }
        ReverbType::PanningDelay => {
            ReverbAlgorithm::Delay(DelayReverb::new(sample_rate, time, true, delay_sync_enabled, delay_sync_rate, bpm))
        }
        _ => {
            let tuning = &FDN_TUNINGS[reverb_type as usize];
            ReverbAlgorithm::Fdn(FdnReverb::new(sample_rate, tuning, time))
        }
    }
}

/// マスターリバーブ。`ReverbType`に応じて拡散リバーブ(FDN)/フィードバックディレイの
/// いずれかのアルゴリズムを内部に保持する。
///
/// `delay_sync_enabled`/`delay_sync_rate`/`bpm`はDelay/PanningDelay専用の設定だが、
/// `time`と同じくタイプ切り替えを跨いで保持する（Room系タイプへ切り替えてまた
/// Delayへ戻したときに同期設定が消えないように）。
pub struct Reverb {
    sample_rate: f32,
    reverb_type: ReverbType,
    time: u8,
    delay_sync_enabled: u8,
    delay_sync_rate: u8,
    bpm: f32,
    algorithm: ReverbAlgorithm,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let reverb_type = ReverbType::default();
        let time = 128;
        let delay_sync_enabled = 0;
        let delay_sync_rate = default_delay_sync_rate();
        let bpm = 120.0;
        let algorithm = build_algorithm(sample_rate, reverb_type, time, delay_sync_enabled, delay_sync_rate, bpm);
        Self { sample_rate, reverb_type, time, delay_sync_enabled, delay_sync_rate, bpm, algorithm }
    }

    /// タイプを切り替える。内部アルゴリズムを再構築するため、残響テールはリセットされる。
    pub fn set_type(&mut self, reverb_type: ReverbType) {
        self.reverb_type = reverb_type;
        self.algorithm = build_algorithm(
            self.sample_rate,
            reverb_type,
            self.time,
            self.delay_sync_enabled,
            self.delay_sync_rate,
            self.bpm,
        );
    }

    /// 残響時間を設定する。FDNはフィードバックゲインのみ更新（再構築なし）。
    /// Delay/PanningDelayは同期無効時のみディレイ長＋フィードバックを更新し
    /// （バッファは作り直さずクロスフェード）、同期有効時はフィードバックのみ更新する。
    pub fn set_time(&mut self, time: u8) {
        self.time = time;
        match &mut self.algorithm {
            ReverbAlgorithm::Fdn(reverb) => reverb.set_time(time),
            ReverbAlgorithm::Delay(reverb) => reverb.set_time(time),
        }
    }

    /// NRPN(0,36)相当：Delay/PanningDelayのテンポ同期有効/無効（0=OFF/1以上=ON）。
    /// Room1〜Plateタイプには効果がない。
    pub fn set_delay_sync(&mut self, value: u8) {
        self.delay_sync_enabled = value;
        if let ReverbAlgorithm::Delay(reverb) = &mut self.algorithm {
            reverb.set_sync_enabled(value);
        }
    }

    /// NRPN(0,37)相当：Delay/PanningDelayの同期先レート（`sound_core::sync_rate_beats`と同じ
    /// 0〜255連続値、TimeEgの`sync_rate`と同じ音価アンカーを踏む）。
    pub fn set_delay_sync_rate(&mut self, rate: u8) {
        self.delay_sync_rate = rate;
        if let ReverbAlgorithm::Delay(reverb) = &mut self.algorithm {
            reverb.set_sync_rate(rate);
        }
    }

    /// テンポ（BPM）を設定する。Delay/PanningDelayが同期有効のときのみ使う。
    /// 0以下は無視する（`Op505Engine::set_tempo`と同じガード）。
    pub fn set_tempo(&mut self, bpm: f32) {
        if bpm <= 0.0 {
            return;
        }
        self.bpm = bpm;
        if let ReverbAlgorithm::Delay(reverb) = &mut self.algorithm {
            reverb.set_tempo(bpm);
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

    /// フィードバックダンピング追加の回帰テスト。ダンピングが無ければ単発インパルスの
    /// エコーは常に「厳密に1サンプルだけ非ゼロ」のはずだが、ワンポールローパスを
    /// フィードバック経路に挟むと2回目以降のエコーは指数減衰の裾を引いて
    /// 後続サンプルへ滲み出す（実機テープ/BBDディレイの高域減衰と同じ挙動）。
    #[test]
    fn feedback_damping_smears_second_echo() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Delay);
        reverb.set_time(255); // 最大フィードバック(0.85)でダンピングの効果を目立たせる

        reverb.process(1.0, 1.0);
        let delay_samples = (DELAY_TIME_MAX_MS / 1000.0 * 44100.0) as usize + 1;

        let mut samples = vec![0.0f32; 2 * delay_samples + 20];
        for s in samples.iter_mut() {
            *s = reverb.process(0.0, 0.0).0;
        }

        // 1回目のエコー: ダンピング適用前の生の値をそのまま読むだけなので、
        // 前後1サンプルは厳密に0のまま（滲みは無い）。
        let e1 = delay_samples - 1;
        assert_eq!(samples[e1 - 1], 0.0, "1回目のエコーの直前は無音のはず");
        assert!(samples[e1].abs() > 1e-3, "1回目のエコーが検出できない: {}", samples[e1]);
        assert_eq!(samples[e1 + 1], 0.0, "1回目のエコー自体はダンピング前の生値なので滲まないはず");

        // 2回目のエコー: フィードバック経由でダンピングを1回通過済みなので、
        // ちょうどその位置の直後（未来側、ローパスの因果的な減衰方向）に
        // 無視できない大きさの裾が残るはず。タイミングは1回目と同じ周期の倍数
        // （ダンピングは振幅・スペクトルのみに効き、遅延位置自体はズラさない）。
        let e2 = 2 * delay_samples - 1;
        let peak = samples[e2].abs();
        assert!(peak > 1e-3, "2回目のエコーが検出できない: {}", peak);
        let tail_energy: f32 = samples[e2 + 1..e2 + 6].iter().map(|v| v.abs()).sum();
        assert!(
            tail_energy > peak * 0.01,
            "2回目のエコー後方に減衰の裾が無い（ダンピングが効いていない）: peak={peak}, tail_energy={tail_energy}"
        );
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
    fn sync_produces_expected_echo_interval() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Delay);
        reverb.set_delay_sync(1);
        reverb.set_delay_sync_rate(crate::time_eg::sync_note_anchor(7)); // 1/8
        reverb.set_tempo(120.0);

        // フェード（最大30ms程度）が収まるまで無音で安定させる。
        for _ in 0..5000 {
            reverb.process(0.0, 0.0);
        }

        let (first_l, _) = reverb.process(1.0, 1.0);
        assert_eq!(first_l, 0.0, "ディレイ直後はまだエコーが返ってこないはず");

        // 120BPMの1/8 = 0.25秒。`seconds_to_samples`の「切り捨て+1」規約に合わせる。
        let expected = (0.25 * 44100.0) as usize + 1;
        let mut found_at = None;
        for i in 1..expected + 10 {
            let (l, _) = reverb.process(0.0, 0.0);
            if l.abs() > 1e-6 {
                found_at = Some(i);
                break;
            }
        }
        let i = found_at.expect("同期ディレイのエコーが検出できない");
        assert!((i as i64 - expected as i64).abs() <= 2, "エコー間隔が音価どおりでない: i={i}, expected={expected}");
    }

    #[test]
    fn bpm_jitter_within_band_does_not_relock() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Delay);
        reverb.set_delay_sync(1);
        reverb.set_delay_sync_rate(crate::time_eg::sync_note_anchor(10)); // 1/4
        reverb.set_tempo(120.0);

        for _ in 0..5000 {
            reverb.process(0.0, 0.0);
        }

        // ±0.8%のジッターを与え続ける（再ロックバンド±1.5%の内側）。
        for i in 0..20000 {
            let jitter = if i % 2 == 0 { 1.008 } else { 0.992 };
            reverb.set_tempo(120.0 * jitter);
            reverb.process(0.0, 0.0);
        }

        // ロックが動いていなければ、エコー間隔は最初にロックした1/4@120BPM(0.5秒)のまま。
        let (first_l, _) = reverb.process(1.0, 1.0);
        assert_eq!(first_l, 0.0);

        let expected = (0.5 * 44100.0) as usize + 1;
        let mut found_at = None;
        for i in 1..expected + 10 {
            let (l, _) = reverb.process(0.0, 0.0);
            if l.abs() > 1e-6 {
                found_at = Some(i);
                break;
            }
        }
        let i = found_at.expect("エコーが検出できない");
        assert!(
            (i as i64 - expected as i64).abs() <= 2,
            "BPMジッターでロックが動いてしまった: i={i}, expected={expected}"
        );
    }

    #[test]
    fn bpm_change_relocks_without_total_silence() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Delay);
        reverb.set_delay_sync(1);
        reverb.set_delay_sync_rate(crate::time_eg::sync_note_anchor(10)); // 1/4
        reverb.set_tempo(120.0);
        for _ in 0..5000 {
            reverb.process(0.0, 0.0);
        }

        // 持続的な信号でディレイに常時何か乗っている状態を作る。
        for i in 0..30000 {
            let x = ((i as f32) * 0.05).sin() * 0.2;
            reverb.process(x, x);
        }

        // BPMを120→90へ大きく変える。確認時間(0.25秒=11025サンプル)を十分超えて処理を続け、
        // その間ずっと有限値で、後半にはテールが残っている（無音に落ちない）ことを確認する。
        let mut settled_energy = 0.0f32;
        for i in 0..30000 {
            reverb.set_tempo(90.0);
            let x = ((i as f32) * 0.05).sin() * 0.2;
            let (l, r) = reverb.process(x, x);
            assert!(l.is_finite() && r.is_finite(), "BPM変更中に発散またはNaN: l={l}, r={r}");
            if i > 15000 {
                settled_energy += l * l + r * r;
            }
        }
        assert!(settled_energy > 0.0, "BPM変更後もディレイ出力が鳴り続けているはず（テールが消えていない）");
    }

    #[test]
    fn slow_tempo_long_note_folds_to_fit_capacity() {
        let mut reverb = Reverb::new(44100.0);
        reverb.set_type(ReverbType::Delay);
        reverb.set_delay_sync(1);
        reverb.set_delay_sync_rate(255); // 4/1（最長音価）
        // 4/1 @ 20BPM = 16拍 * 60/20 = 48秒。DELAY_MAX_SECONDS(4秒)を大幅に超えるため、
        // 折り返し処理が正しく効いていないと容量オーバーで発散/パニックするはず。
        reverb.set_tempo(20.0);

        for i in 0..(44100 * 2) {
            let x = if i % 4410 == 0 { 1.0 } else { 0.0 };
            let (l, r) = reverb.process(x, -x);
            assert!(l.is_finite() && r.is_finite(), "発散またはNaN: l={l}, r={r}");
            assert!(l.abs() < 100.0 && r.abs() < 100.0, "発散している: l={l}, r={r}");
        }
    }

    #[test]
    fn no_nan_long_run_with_delay_sync_enabled() {
        for &t in &[ReverbType::Delay, ReverbType::PanningDelay] {
            let mut reverb = Reverb::new(44100.0);
            reverb.set_type(t);
            reverb.set_delay_sync(1);
            reverb.set_delay_sync_rate(200);
            reverb.set_time(255);
            for i in 0..(44100 * 2) {
                // BPMも常時変動させ続けるストレステスト（頻繁な再ロック/フェードの安定性確認）。
                let bpm = 100.0 + 40.0 * ((i as f32) * 0.0003).sin();
                reverb.set_tempo(bpm);
                let input = if i % 4410 == 0 { 1.0 } else { 0.0 };
                let (l, r) = reverb.process(input, -input);
                assert!(l.is_finite() && r.is_finite(), "{t:?}: 発散またはNaN: {l}, {r}");
                assert!(l.abs() < 100.0 && r.abs() < 100.0, "{t:?}: 発散している: {l}, {r}");
            }
        }
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
