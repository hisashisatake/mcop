//! op505-core: ym38x6のEG（レート方式5段）をN点Time/Level方式（`sound_core::TimeEg`）に
//! 全面移行した新チップのコアクレート。
//!
//! EG非依存の安定部分（アルゴリズム結線・波形・チップ内LFO・パラメーターマッピングテーブル・
//! 質感LFO）は`sound-fm`（`ym38x6-core`と共有する兄弟クレート）へ直接依存する
//! （fork-on-write方針。将来op505独自に進化させたくなったら該当モジュールだけコピーして依存を切る）。
//! `ym38x6-core`への依存は`[dev-dependencies]`のみ（`examples/op505_probe.rs`が既存`.38x6`
//! からの変換に使う）。`src/`は一切依存しない（op505/デフォーク計画Phase 4）。
//! EG関連（オペレーターEG・Pitch/Cutoff/Gain FG）は全面的にTimeEg化するため、
//! `operator.rs`とChannel/Engine部のみ複製・改変する。

pub mod eg_convert;
pub mod operator;
pub mod preset;
pub use operator::Op505OperatorParams;
pub use preset::{op505_presets_dir, Op505Preset, Op505PresetBank, Op505PresetEntry, Op505PresetFile};

/// バイポーラFG（Pitch/Cutoff）の無変調レベル。`sound-core`の同名定数の再エクスポート。
///
/// `op505-midi`は依存を`op505-core`/`sound-fm`の2本に絞る規約（`.claude/rules`参照）のため
/// `sound-core`を直接見に行けない。CC78のDelay判定で「無変調の待ち段」を識別するのに必要なので
/// ここから供給する。
pub use sound_core::BIPOLAR_NEUTRAL_RAW;

use std::collections::BTreeMap;

use sound_fm::algorithm::ALGORITHMS;
use sound_fm::chip_lfo::{ams_to_depth, chip_lfo_freq_to_hz, pms_to_cents_range};
use sound_fm::mapping::{
    carrier_velocity_gain, feedback_to_scale_with_max, fixed_note_fine_to_cents, frequency_to_note,
    note_to_frequency, velocity_to_volume_gain, FM_MODULATION_INDEX_SCALE,
};
use sound_fm::waveform::{self, gen_builtin_waveform};
use operator::Operator;
use serde::{Deserialize, Serialize};
use sound_core::{
    bipolar_level, convert_wave_32, cutoff_to_hz,
    effective_cutoff_bipolar_level, tempo_speed_scale,
    FilterType, Svf, TimeEg, TimeEgParams, TimeStage, Vco, WaveTable,
    RETRIGGER_MODE_RESET,
};

// ---------------------------------------------------------------------------
// パッチ（チャンネル + オペレーター4個分のパラメーター一式）
// ---------------------------------------------------------------------------

/// Pitch/Cutoff FG：**バイポーラレベル**のTimeEgParams ＋ 符号を持たない強度Depth。
///
/// 変調量は `bipolar_level(EG出力) × depth` で、**符号はEGのレベル波形が持ち、Depthは
/// 振れ幅の倍率**（0＝変調なし）。レベル生値128が無変調の中心。この役割分担により、
/// FGの形だけで上下対称のサイクル（三角波・パルス等）を1つのDepthで描ける。
///
/// 旧方式（〜2026-08-18）は逆に「レベル＝0〜1の大きさ／Depth＝バイポーラ(中心128)」で、
/// 符号を1音中不変のDepthだけが持っていたため、谷が必ずベース値へ張り付き片側にしか
/// 振れなかった。未リリースのため`.op505`にバージョン互換層は設けず、既存バンクは
/// 変換ツール群で作り直す方針。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op505BipolarFg {
    pub eg: TimeEgParams,
    /// 振れ幅の倍率（0〜255）。0＝変調なし。符号は持たない（EGのレベル側が持つ）。
    pub depth: u8,
}

impl Default for Op505BipolarFg {
    /// 無変調の新規パッチ：Depth=0（振れ幅ゼロ）かつSTAGES=0（無効化、`Voice::tick`が
    /// tick自体をスキップする）。
    fn default() -> Self {
        Self { eg: neutral_bipolar_eg(), depth: 0 }
    }
}

/// バイポーラFG（Pitch/Cutoff）の無変調EG：STAGES=0（無効化）で、段データ自体は
/// 全段中央レベル(128)のまま持たせる。
///
/// stage_count=0にするのは`Voice::tick`が「STAGES=0ならtickをスキップする」設計
/// （`ui_core::TimeEgProfile::min_stages=0`で許容されるFG専用の特殊値、
/// docs/timeeg-fg-disable-plan.md参照）に合わせるため。stagesを128埋めのまま残すのは、
/// STAGES欄を1以上へ戻したときレベルを「全開マイナス」（生値0のバイポーラ解釈）から
/// ではなく中央（無変調）から再開できるようにするため——`.max(1)`で1段扱いされる
/// 旧仕様の名残の安全策で、無効化そのものには影響しない。
pub fn neutral_bipolar_eg() -> TimeEgParams {
    let neutral = TimeStage { time: 0, level: sound_core::BIPOLAR_NEUTRAL_RAW, curve: 0 };
    TimeEgParams { stages: [neutral; sound_core::MAX_STAGES], stage_count: 0, ..TimeEgParams::default() }
}

/// レベルが「0〜255の大きさ」として組まれたEG（振幅系の形・レガシー実機からの変換結果など）を、
/// バイポーラFGのレベル（中心128）へ`direction`方向で読み替える。
///
/// `level → 128 ± level×128/255` の写像で、大きさ0が中央（無変調）・大きさ255が片側の端になる。
/// `positive=true`なら上げ方向（ピッチ上昇／カットオフを開く）、falseなら下げ方向。
///
/// レガシーFM音源の Filter EG や Pitch EG は「深さ＋向き」で表現されているため、
/// 変換ツールはこの関数で向きをレベル側へ畳み込み、Depthには大きさだけを渡す。
/// 分解能は片側128段階になる（`(v-128)/128`慣例の帰結、DT1等と同じ）。
pub fn bipolar_fg_levels_from_magnitude(eg: &mut TimeEgParams, positive: bool) {
    let sign = if positive { 1.0 } else { -1.0 };
    for stage in eg.stages.iter_mut() {
        let centered = sound_core::BIPOLAR_NEUTRAL_RAW as f32
            + sign * stage.level as f32 * sound_core::BIPOLAR_NEUTRAL_RAW as f32 / 255.0;
        stage.level = centered.round().clamp(0.0, 255.0) as u8;
    }
}

/// CHIP LFOのピッチ変調経路（`pms`×`chip_lfo_pmd`の三角波、`chip_lfo_freq`が速度・
/// `chip_lfo_delay`が先頭待機）をPitch FGの2段三角ループへ変換する
/// （CHIP LFO退役の第一段階、memory `project_chip_lfo_retirement_investigation.md`参照）。
///
/// 両者とも波形は三角波固定のため厳密に一致する（`bipolar_level`が`(v-128)/128`慣例で
/// 正側127/128に頭打ちする非対称性のみ残る、DT1等と同じ許容誤差）。段構成は
/// 「中央(128)で`delay_seconds`待機→谷(0)へ瞬時ジャンプ(time=0)→山(255)⇄谷(0)を
/// `1/(2×freq)`秒ずつループ」の4段。CHIP LFO側もdelay終了と同時に位相0(谷)から
/// 上昇を始めるため、delay終了直後に谷へ着地するこの構成と一致する。
///
/// `pms=0`または`chip_lfo_pmd=0`（変調なし）のときは無変調の既定値を返す。
///
/// 呼び出し規約：変換ツールはこの関数の戻り値を`pitch_fg`へ書き込んだら、
/// 移設元の`pms`/`chip_lfo_pmd`を0にクリアする（二重変調を防ぐ）。`chip_lfo_freq`/
/// `chip_lfo_delay`はAM経路（`chip_lfo_amd`/`ams`）と共有するフィールドのため保持したままにする。
pub fn chip_lfo_pitch_to_pitch_fg(
    pms: u8,
    chip_lfo_pmd: u8,
    chip_lfo_freq: u8,
    chip_lfo_delay: u8,
) -> Op505BipolarFg {
    let cents_amplitude = pms_to_cents_range(pms) * (chip_lfo_pmd as f32 / 255.0);
    if cents_amplitude <= 0.0 {
        return Op505BipolarFg::default();
    }

    let depth = ((cents_amplitude / 1200.0) * 255.0).round().clamp(0.0, 255.0) as u8;

    let delay_seconds = chip_lfo_delay as f32 / 255.0 * 10.0;
    let half_period_seconds = 0.5 / chip_lfo_freq_to_hz(chip_lfo_freq);

    let mut stages = [TimeStage::default(); sound_core::MAX_STAGES];
    stages[0] =
        TimeStage { time: sound_core::seconds_to_time(delay_seconds), level: BIPOLAR_NEUTRAL_RAW, curve: 0 };
    stages[1] = TimeStage { time: 0, level: 0, curve: 0 };
    stages[2] =
        TimeStage { time: sound_core::seconds_to_time(half_period_seconds), level: 255, curve: 0 };
    stages[3] =
        TimeStage { time: sound_core::seconds_to_time(half_period_seconds), level: 0, curve: 0 };

    Op505BipolarFg {
        eg: TimeEgParams {
            stages,
            stage_count: 4,
            loop_enabled: 1,
            loop_start: 2,
            release_point: 3,
            retrigger_mode: RETRIGGER_MODE_RESET,
            ..TimeEgParams::default()
        },
        depth,
    }
}

/// Gain FG：Depthなし、TimeEgParamsそのもの（ym38x6の`GainFg`のTimeEg版）。
pub type Op505GainFg = TimeEgParams;

/// CHIP LFOのAM変調経路（`ams`×`chip_lfo_amd`の振幅変調、`chip_lfo_freq`が速度・
/// `chip_lfo_delay`が先頭待機）をGain FGの5段ループへ変換する
/// （CHIP LFO完全退役、memory `project_chip_lfo_retirement_investigation.md`参照）。
///
/// CHIP LFOのAM係数は`amp_factor = (1 - triangle×depth).clamp(0,1)`（`operator.rs`の
/// `amp_factor`計算と同じ式）で、`triangle<0`の半波がクランプされるため実波形は
/// **「平坦T/4→下降T/4→上昇T/4→平坦T/4」**になる（ピッチ経路の素の三角波とは違う形。
/// クランプがあるからこそ設計が分岐する）。Gain FGは`curve=0`のときレベルをそのまま線形補間する
/// （`TimeEg::shaped_output`参照）ため、**線形ゲイン領域どうしで折れ線が完全一致**する
/// （旧`apply_chip_lfo_am_to_eg`はdB領域のオペレーターEGへ畳み込む近似だったが、これは
/// 近似を伴わない厳密変換）。
///
/// 段構成：`[0]`瞬時に255へ（`time=0`、キーオン起点の立ち上げ）→`[1]`
/// `delay_seconds + T/4`平坦保持（255、位相0〜0.25はクランプにより平坦）→`[2]`floorへT/4で下降
/// （位相0.25〜0.5）→`[3]`255へT/4で上昇（位相0.5〜0.75）→`[4]`T/2平坦保持（255、
/// 位相0.75〜1.0と次周期の0〜0.25が連続するため合算してT/2）で`loop_start=2`へ戻る。
/// `release_point=4`（最終段）なのでnote-offはループを止めない
/// （CHIP LFOがリリース中も鳴り続けるのと同じ挙動）。
///
/// `ams=0`または`chip_lfo_amd=0`（変調なし）のときは`None`を返す。Pitch FGと違いGain FGは
/// 既存パッチが別用途（マスターVCA等）で使っている可能性があるため、無変調の既定値では
/// 上書きしない——呼び出し側が`None`のときは`gain_fg`に一切触れない規約にする。
///
/// 呼び出し規約：戻り値が`Some`かつ`am_enable`なオペレーターが1つ以上あるときのみ
/// `gain_fg`へ書き込み、`gain_fg_to_operators=true`・`gain_fg_to_master=false`にする。
/// `am_enable`はそのまま維持する（「チャンネル共通AMを受けるOP」というゲートの意味は
/// CHIP LFO時代と変わらない、源がGAIN FGに替わるだけ）。
pub fn chip_lfo_am_to_gain_fg(
    ams: u8,
    chip_lfo_amd: u8,
    chip_lfo_freq: u8,
    chip_lfo_delay: u8,
) -> Option<Op505GainFg> {
    let depth = ams_to_depth(ams) * (chip_lfo_amd as f32 / 255.0);
    if depth <= 0.0 {
        return None;
    }

    let hz = chip_lfo_freq_to_hz(chip_lfo_freq);
    let quarter_period_seconds = 0.25 / hz;
    let half_period_seconds = 0.5 / hz;
    let delay_seconds = chip_lfo_delay as f32 / 255.0 * 10.0;
    let floor_level = ((1.0 - depth) * 255.0).round().clamp(0.0, 255.0) as u8;

    let mut stages = [TimeStage::default(); sound_core::MAX_STAGES];
    stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
    stages[1] = TimeStage {
        time: sound_core::seconds_to_time(delay_seconds + quarter_period_seconds),
        level: 255,
        curve: 0,
    };
    stages[2] = TimeStage {
        time: sound_core::seconds_to_time(quarter_period_seconds),
        level: floor_level,
        curve: 0,
    };
    stages[3] =
        TimeStage { time: sound_core::seconds_to_time(quarter_period_seconds), level: 255, curve: 0 };
    stages[4] =
        TimeStage { time: sound_core::seconds_to_time(half_period_seconds), level: 255, curve: 0 };

    Some(TimeEgParams {
        stages,
        stage_count: 5,
        loop_enabled: 1,
        loop_start: 2,
        release_point: 4,
        retrigger_mode: RETRIGGER_MODE_RESET,
        ..TimeEgParams::default()
    })
}

/// チャンネル単位パラメーター一式。ym38x6の`ChannelParams`から、旧`ChannelParamsWire`
/// 後方互換層（フィールドリネーム・旧filter_eg_*/vca_eg_*からの移行）を取り除いた素直な新フォーマット。
///
/// CHIP LFO（PMS/PMD/AMS/AMD/FRQ/DLY）は2026-08-20に完全退役し、フィールドごと削除済み。
/// ピッチ経路はPitch FGへ、AM経路はGain FGのOP単位配線（`gain_fg_to_operators`）へ
/// 厳密変換される（memory `project_chip_lfo_retirement_investigation.md`参照）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op505ChannelParams {
    pub algorithm: u8,
    pub feedback: u8,
    pub filter_cutoff: u8,
    pub filter_resonance: u8,
    pub filter_type: u8,
    pub filter_self_oscillation: bool,
    /// Pitch FG：ピッチ変調の一次源。バイポーラDepth(中心128)でキーオン一発のピッチ
    /// 下降/上昇とループ時のビブラートの両方を作れる。
    pub pitch_fg: Op505BipolarFg,
    /// Cutoff FG：バイポーラDepth化されたカットオフ変調。
    pub cutoff_fg: Op505BipolarFg,
    /// Gain FG：静止を挟んだ2値スイッチ（トレモロ/ゲート）等、TimeEgの本命ユースケース。
    pub gain_fg: Op505GainFg,
    /// Gain FGの出力先マスタースイッチ：VCF後段（合成後）へ一括乗算するか。既定true（従来どおり）。
    /// `gain_fg_to_operators`と独立にON/OFFできる（両方true・両方falseも可）。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で従来挙動(true)にする。
    #[serde(default = "default_gain_fg_to_master")]
    pub gain_fg_to_master: bool,
    /// Gain FGの出力先スイッチ：`am_enable`な各オペレーターの`amp_factor`へ乗算するか。既定false。
    /// trueにするとGain FGはOP単位の変調（キャリアなら音量トレモロ、モジュレーターなら
    /// 変調指数のうねり＝倍音構成の周期変化）を作る——CHIP LFO AM経路の厳密代替
    /// （memory `project_chip_lfo_retirement_investigation.md`参照）。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で無効(false)にする。
    #[serde(default)]
    pub gain_fg_to_operators: bool,
    /// 固定音階：trueなら`note_on`で渡された周波数を無視し、`fixed_note`＋`fixed_note_fine`の
    /// ピッチで発音する。GM2リズムチャンネル（ノート番号＝音色選択であってピッチではない）用。
    /// ピッチベンド／Pitch FGはセント加算なので無効化中でも従来どおり効く。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で無効(false)にする。
    #[serde(default)]
    pub fixed_note_enable: bool,
    /// 固定音階のMIDIノート番号（0〜127）。`fixed_note_enable=false`のときは無視される。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default = "default_fixed_note")]`で
    /// C4(60)にする（0だと有効化直後に8.18Hzになり「壊れた」ように聞こえるため）。
    #[serde(default = "default_fixed_note")]
    pub fixed_note: u8,
    /// 固定音階の微調整（0〜255、中心128＝±0、両端±100セント）。dt1/op_fine_tuneと同じ
    /// 「中心128」の慣例に揃えている。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default = "default_fixed_note_fine")]`で
    /// 128（無調整）にする。
    #[serde(default = "default_fixed_note_fine")]
    pub fixed_note_fine: u8,
}

fn default_gain_fg_to_master() -> bool {
    true
}

fn default_fixed_note() -> u8 {
    60
}

fn default_fixed_note_fine() -> u8 {
    sound_core::BIPOLAR_NEUTRAL_RAW
}

/// `Gain FG`の無効(STAGES=0)既定：エンジンは`stage_count==0`を検出すると`gain_fg_out`に
/// 常に**1.0（透過）**を使いtickを回さない（`Voice::tick`参照）ため、ゲートを一切閉じない
/// ＝発音終了は各オペレーターのidle判定のみで行う（ym38x6の`default_gain_fg`の設計意図を踏襲）。
///
/// 全段を透過レベル255で埋めるのは`neutral_bipolar_eg`と同じ理由：再有効化（STAGES 0→N）で
/// 段データがそのまま復元されるため、末尾の段だけ0埋めのままだと「全開→無音」の落差が生じる。
/// `stage_count=0`はOP1〜4 EGでは「1として扱う」特殊値だが、Gain FG適用箇所（`Voice::tick`）
/// だけが「無効」の意味を持つ設計（詳細は`docs/timeeg-fg-disable-plan.md`）。
fn default_gain_fg() -> Op505GainFg {
    TimeEgParams {
        stages: [TimeStage { time: 0, level: 255, curve: 0 }; sound_core::MAX_STAGES],
        stage_count: 0,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 0,
     ..Default::default()}
}

impl Default for Op505ChannelParams {
    fn default() -> Self {
        Self {
            algorithm: 0,
            feedback: 0,
            filter_cutoff: 255,
            filter_resonance: 0,
            filter_type: 0,
            filter_self_oscillation: true,
            pitch_fg: Op505BipolarFg::default(),
            cutoff_fg: Op505BipolarFg::default(),
            gain_fg: default_gain_fg(),
            gain_fg_to_master: true,
            gain_fg_to_operators: false,
            fixed_note_enable: false,
            fixed_note: default_fixed_note(),
            fixed_note_fine: default_fixed_note_fine(),
        }
    }
}

/// 4op分のオペレーターパラメーター + チャンネルパラメーターの一式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Op505Patch {
    pub operators: [Op505OperatorParams; 4],
    pub channel: Op505ChannelParams,
}

// ---------------------------------------------------------------------------
// チャンネル（4オペレーター + アルゴリズム結線）
// ---------------------------------------------------------------------------

/// フィードバック帰還スケールの上限。ym38x6の`set_feedback_scale_max`のような実行時
/// 上書きは複製せず固定値にする（A/B比較用の実験フラグはym38x6側に残す。fork-on-write）。
const FEEDBACK_SCALE_MAX: f32 = 1.8;

struct Channel {
    operators: [Operator; 4],
    channel_params: Op505ChannelParams,
    /// フィードバックオペレーターの直前の出力（自己変調に使う）。
    feedback_buffer: f32,
    /// 2サンプル平均帰還用の、さらに1つ前の出力out[n-2]（実機OPM/OPN/OPZ準拠、常時有効）。
    feedback_buffer2: f32,
    note: u8,
    base_frequency: f32,
    /// note_on/retriggerで渡された生の周波数（固定音階が無効なら`base_frequency`と同じ）。
    /// 固定音階のフィールドがライブ編集で変化したとき、`set_channel_params`が
    /// ここから実効ピッチを再計算する（`effective_pitch`参照）。
    keyed_frequency: f32,
    velocity: u8,
    /// Pitch FGのキーオン連動エンベロープ（TimeEg）。
    pitch_fg_eg: TimeEg,
    /// CC76（Vibrato Rate）由来のPitch FG速さスケール（1.0=無補正）。
    pitch_fg_rate_scale: f32,
    bend_cents: f32,
    channel_gain: f32,
    /// 4op合成後に適用するSVF本体。CutoffのEG変調は`cutoff_fg_eg`が別途計算し、
    /// `effective_cutoff`で合成してから渡す（sound-coreの`VoiceFilter`のような一体型ではなく
    /// 分解結線。sound-core側にEG方式ごとの並行実装を増やさないための設計、plan参照）。
    svf: Svf,
    /// Cutoff FGのキーオン連動エンベロープ（TimeEg）。
    cutoff_fg_eg: TimeEg,
    /// Gain FGのキーオン連動エンベロープ（TimeEg）。`sound_core::VoiceAmp`は使わず、
    /// tick結果をそのままゲイン乗算する（実体は乗算1行のため分解結線で十分）。
    gain_fg_eg: TimeEg,
    note_is_off: bool,
}

/// 固定音階が有効なら`frequency`（note_onで渡された生の周波数）を無視し、
/// `fixed_note`＋`fixed_note_fine`から実効周波数を導出する。KSR/level_scale用の`note`も
/// 同じ実効周波数から`frequency_to_note`で導出し、経路を1本に保つ。
/// `fixed_note_enable=false`のときは`(frequency, frequency_to_note(frequency))`を返し、
/// 従来と完全に同一の挙動になる。
fn effective_pitch(frequency: f32, channel: &Op505ChannelParams) -> (f32, u8) {
    if channel.fixed_note_enable {
        let f = note_to_frequency(channel.fixed_note)
            * 2f32.powf(fixed_note_fine_to_cents(channel.fixed_note_fine) / 1200.0);
        (f, frequency_to_note(f))
    } else {
        (frequency, frequency_to_note(frequency))
    }
}

impl Channel {
    fn new(frequency: f32, velocity: u8, patch: Op505Patch) -> Self {
        let (effective_frequency, note) = effective_pitch(frequency, &patch.channel);
        let algo = &ALGORITHMS[(patch.channel.algorithm as usize).min(7)];
        let operators = std::array::from_fn(|i| {
            let mut op = Operator::new(patch.operators[i]);
            op.set_carrier(algo.carriers.contains(&i));
            op.note_on(effective_frequency, velocity);
            op
        });
        // Pitch/Cutoff FGはレベルをバイポーラ解釈するため、キーオン起点を中央(128)にする
        // （`TimeEg::new()`の0.0起点だと「全開マイナスからのスイープ」で始まってしまう）。
        // Gain FGは振幅系なので従来どおり0.0起点。
        let mut pitch_fg_eg = TimeEg::new_bipolar();
        pitch_fg_eg.note_on();
        let mut cutoff_fg_eg = TimeEg::new_bipolar();
        cutoff_fg_eg.note_on();
        let mut gain_fg_eg = TimeEg::new();
        gain_fg_eg.note_on();
        Self {
            operators,
            channel_params: patch.channel,
            feedback_buffer: 0.0,
            feedback_buffer2: 0.0,
            note,
            base_frequency: effective_frequency,
            keyed_frequency: frequency,
            velocity,
            pitch_fg_eg,
            pitch_fg_rate_scale: 1.0,
            bend_cents: 0.0,
            channel_gain: 1.0,
            svf: Svf::new(),
            cutoff_fg_eg,
            gain_fg_eg,
            note_is_off: false,
        }
    }

    /// 各FGの`eg.retrigger_mode`がRESETなら`note_on()`と同じく0からクリーンに再スタートし、
    /// 既定のCONTINUEなら現在レベルを保ったまま段0へ向かう（実機OPMのKey-On挙動、
    /// オペレーターの`retrigger()`と同じ使い分け）。
    fn retrigger(&mut self, frequency: f32, velocity: u8, patch: Op505Patch) {
        let (effective_frequency, note) = effective_pitch(frequency, &patch.channel);
        let algo = &ALGORITHMS[(patch.channel.algorithm as usize).min(7)];
        for (i, op) in self.operators.iter_mut().enumerate() {
            op.params = patch.operators[i];
            op.set_carrier(algo.carriers.contains(&i));
            op.retrigger(effective_frequency, velocity);
        }
        self.channel_params = patch.channel;
        self.note = note;
        self.base_frequency = effective_frequency;
        self.keyed_frequency = frequency;
        self.velocity = velocity;
        retrigger_time_eg(&mut self.cutoff_fg_eg, &patch.channel.cutoff_fg.eg);
        retrigger_time_eg(&mut self.gain_fg_eg, &patch.channel.gain_fg);
        retrigger_time_eg(&mut self.pitch_fg_eg, &patch.channel.pitch_fg.eg);
        self.note_is_off = false;
    }

    fn note_off(&mut self) {
        for op in self.operators.iter_mut() {
            op.note_off();
        }
        self.cutoff_fg_eg.note_off();
        self.gain_fg_eg.note_off();
        self.pitch_fg_eg.note_off();
        self.note_is_off = true;
    }

    /// ボイススチールの優先度スコア。値が小さいほど先に奪われる。
    fn steal_score(&self) -> f32 {
        let algo = &ALGORITHMS[(self.channel_params.algorithm as usize).min(7)];
        let carrier_level = algo
            .carriers
            .iter()
            .map(|&i| self.operators[i].env_level())
            .fold(0.0f32, f32::max);
        if self.note_is_off {
            carrier_level
        } else {
            carrier_level + 10.0
        }
    }

    fn note_on_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_on(self.base_frequency, self.velocity);
    }

    fn note_off_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_off();
    }

    fn is_idle(&self) -> bool {
        self.operators.iter().all(|op| op.is_idle())
    }

    fn tick(&mut self, sample_rate: f32, wave_tables: &[Option<WaveTable>], tempo_bpm: f32) -> f32 {
        if self.is_idle() {
            return 0.0;
        }

        // Pitch FG：ループ可能TimeEgでビブラート/シンセタムを作る一次源。
        // CC76由来の速度補正(pitch_fg_rate_scale)とテンポ同期(tempo_speed_scale)を乗算で共存させる。
        // レベルはバイポーラ（生値128＝無変調の中心）で符号を持ち、Depthは振れ幅の倍率。
        // これにより1つのDepthのままFGの形だけで上下対称のビブラートが描ける。
        //
        // stage_count==0はUI側のSTAGES=0（無効化、`ui_core::TimeEgProfile::min_stages=0`で
        // 許容されるFG専用の特殊値）に対応する。tick自体を呼ばずに変調量ゼロとして扱うことで、
        // ボイス単位で毎サンプル回るTimeEg::tick()のコストを避ける（既存データはstage_count=0を
        // 持ち得ないため、この分岐は出力をビット単位で変えない）。
        let pitch_fg = self.channel_params.pitch_fg;
        let pitch_fg_cents = if pitch_fg.eg.stage_count == 0 {
            0.0
        } else {
            let pitch_fg_speed = self.pitch_fg_rate_scale * tempo_speed_scale(&pitch_fg.eg, tempo_bpm);
            let pitch_fg_out = self.pitch_fg_eg.tick(sample_rate, pitch_fg.eg, pitch_fg_speed);
            bipolar_level(pitch_fg_out) * (pitch_fg.depth as f32 / 255.0) * 1200.0
        };
        for op in self.operators.iter_mut() {
            op.set_pitch_modulation(self.bend_cents + pitch_fg_cents);
        }

        // Gain FG：VCA（合成後の一括乗算）とOP単位AM（旧CHIP LFO AM経路の厳密代替、
        // memory `project_chip_lfo_retirement_investigation.md`参照）の両方の源になる、
        // 単一のTimeEg。オペレーターループより前にtickし、両方の用途で同じサンプルの値を使う
        // （tick回数は変わらないため既存パッチの出力はビット単位で不変）。
        //
        // stage_count==0（無効化）のときは**1.0（透過）**。0.0にすると無音になってしまう
        // （Pitch/Cutoff FGの「変調量ゼロ」とは中立値が異なる。plan
        // `docs/timeeg-fg-disable-plan.md`の🔴落とし穴参照）。
        let gain_fg = self.channel_params.gain_fg;
        let gain_fg_to_operators = self.channel_params.gain_fg_to_operators;
        let gain_fg_out = if gain_fg.stage_count == 0 {
            1.0
        } else {
            let gain_fg_speed = tempo_speed_scale(&gain_fg, tempo_bpm);
            self.gain_fg_eg.tick(sample_rate, gain_fg, gain_fg_speed)
        };
        for op in self.operators.iter_mut() {
            let factor = if op.params.am_enable && gain_fg_to_operators { gain_fg_out } else { 1.0 };
            op.set_am_factor(factor);
        }

        // アルゴリズム結線に基づく4op合成
        let algo = &ALGORITHMS[(self.channel_params.algorithm as usize).min(7)];
        let mut op_outputs = [0.0f32; 4];
        for &op_idx in algo.eval_order.iter() {
            let mut modulation = 0.0;
            for &(from, to) in algo.routes {
                if to == op_idx {
                    modulation += op_outputs[from] * FM_MODULATION_INDEX_SCALE;
                }
            }
            if op_idx == algo.feedback_op {
                let fb_source = 0.5 * (self.feedback_buffer + self.feedback_buffer2);
                let scale = feedback_to_scale_with_max(self.channel_params.feedback, FEEDBACK_SCALE_MAX);
                modulation += fb_source * scale;
            }
            let wave = wave_table_for(wave_tables, self.operators[op_idx].params.waveform);
            let out = self.operators[op_idx].tick(sample_rate, wave, modulation, self.note, tempo_bpm);
            op_outputs[op_idx] = out;
            if op_idx == algo.feedback_op {
                self.feedback_buffer2 = self.feedback_buffer;
                self.feedback_buffer = out;
            }
        }

        let all_full_velocity_gain =
            algo.carriers.iter().all(|&i| self.operators[i].params.velocity_gain == 255);
        let carrier_sum: f32 = if all_full_velocity_gain {
            let sum: f32 = algo.carriers.iter().map(|&i| op_outputs[i]).sum();
            sum * velocity_to_volume_gain(self.velocity)
        } else {
            algo.carriers
                .iter()
                .map(|&i| {
                    op_outputs[i]
                        * carrier_velocity_gain(self.operators[i].params.velocity_gain, self.velocity)
                })
                .sum()
        };
        let dry = carrier_sum * self.channel_gain;

        // VCF：Cutoff FG（TimeEg）を先にtickし、effective_cutoffで基準Cutoffと合成してからSvfへ。
        // stage_count==0（無効化）のときはtickを呼ばず基準Cutoffをそのまま使う。
        let cp = &self.channel_params;
        let cutoff = if cp.cutoff_fg.eg.stage_count == 0 {
            cp.filter_cutoff
        } else {
            let cutoff_fg_speed = tempo_speed_scale(&cp.cutoff_fg.eg, tempo_bpm);
            let cutoff_level = self.cutoff_fg_eg.tick(sample_rate, cp.cutoff_fg.eg, cutoff_fg_speed);
            effective_cutoff_bipolar_level(cp.filter_cutoff, cutoff_level, cp.cutoff_fg.depth)
        };
        let cutoff_hz = cutoff_to_hz(cutoff);
        let filtered = self.svf.process(
            dry,
            sample_rate,
            cutoff_hz,
            cp.filter_resonance,
            cp.filter_self_oscillation,
            FilterType::from_u8(cp.filter_type),
        );

        // VCA：Gain FGは既にオペレーターループより前でtick済み（`gain_fg_out`）。
        // ここでは合成後へ一括乗算するかどうかを`gain_fg_to_master`で切り替えるだけ。
        filtered * if cp.gain_fg_to_master { gain_fg_out } else { 1.0 }
    }
}

/// FGの`eg.retrigger_mode`に応じてretrigger()/note_on()を使い分ける
/// （`Operator::retrigger()`と同じ分岐をChannel側の3FGへ揃える）。
fn retrigger_time_eg(eg: &mut TimeEg, params: &TimeEgParams) {
    if params.retrigger_mode == RETRIGGER_MODE_RESET {
        eg.note_on();
    } else {
        eg.retrigger();
    }
}

/// 指定スロットの波形テーブルを返す。未割り当てスロットはスロット0（サイン波）にフォールバックする。
fn wave_table_for(wave_tables: &[Option<WaveTable>], slot: u8) -> &WaveTable {
    wave_tables[slot as usize]
        .as_ref()
        .unwrap_or_else(|| wave_tables[0].as_ref().unwrap())
}

// ---------------------------------------------------------------------------
// op505 エンジン
// ---------------------------------------------------------------------------

const TOTAL_SLOTS: usize = 256;

/// ユーザー定義波形（`set_user_wave`）に使える最初のスロット番号。波形スロットの割り当ては
/// 0〜31がビルトイン波形（`BUILTIN_WAVEFORM_COUNT`）、32〜63がノイズ（`NOISE_WAVEFORM_BASE`
/// から`NOISE_LEVELS`段）、64〜255が空きという構成なので、その境界を定数から導出する。
/// リテラルで書かないのは、以前このガードが「ビルトインは8波形」時代の`slot >= 8`のまま
/// 取り残され、スロット8〜31のビルトイン波形を上書きできる状態になっていたため。
///
/// ノイズ帯（32〜63）を除外するのはインデックス保護のためだけではない。`Operator::tick`は
/// `is_noise_waveform`をテーブル参照より先に判定するので、この帯へ書き込んだ波形は
/// 読まれることなくノイズが鳴る＝エラーも出ず無視されるサイレントな失敗になる。
const FIRST_USER_WAVE_SLOT: u8 = waveform::NOISE_WAVEFORM_BASE + waveform::NOISE_LEVELS;

/// 同時発音数の既定上限。ym38x6-coreの実測（[[project_voice_steal_and_eg_lut_optimization]]）に
/// 基づく値をそのまま引き継ぐ（通常演奏でヘッドルームを持ちつつストレス時の負荷増大を抑える）。
const DEFAULT_MAX_VOICES: usize = 64;

pub struct Op505Engine {
    sample_rate: f32,
    /// ボイスID→チャンネル。`BTreeMap`でID昇順の決定論的イテレーション順を保証する
    /// （浮動小数点加算順序を固定し、同一入力で出力WAVがビット一致するようにする）。
    channels: BTreeMap<usize, Channel>,
    wave_tables: Vec<Option<WaveTable>>,
    current_patch: Op505Patch,
    max_voices: usize,
    mix_buf: Vec<f32>,
    /// TimeEgのテンポ同期（`sync_enabled`）に使うBPM。ホストDAWのTransport（VST）や
    /// タップテンポ（gesture-app）から`set_tempo()`経由で設定される。既定120。
    tempo_bpm: f32,
}

impl Op505Engine {
    pub fn new(sample_rate: f32) -> Self {
        let mut wave_tables: Vec<Option<WaveTable>> = (0..TOTAL_SLOTS).map(|_| None).collect();
        for i in 0..waveform::BUILTIN_WAVEFORM_COUNT {
            wave_tables[i as usize] = Some(gen_builtin_waveform(i));
        }
        Self {
            sample_rate,
            channels: BTreeMap::new(),
            wave_tables,
            current_patch: Op505Patch::default(),
            max_voices: DEFAULT_MAX_VOICES,
            mix_buf: Vec::new(),
            tempo_bpm: 120.0,
        }
    }

    pub fn set_max_voices(&mut self, max_voices: usize) {
        self.max_voices = max_voices.max(1);
    }

    fn steal_one_voice(&mut self) {
        if let Some(steal_id) = self
            .channels
            .iter()
            .min_by(|a, b| a.1.steal_score().partial_cmp(&b.1.steal_score()).unwrap())
            .map(|(&id, _)| id)
        {
            self.channels.remove(&steal_id);
        }
    }

    pub fn set_patch(&mut self, patch: Op505Patch) {
        self.current_patch = patch;
    }

    /// `set_patch`と同様に`current_patch`を更新した上で、現在発音中の全チャンネルにも
    /// 反映する（GUI/DAWノブ変更相当）。
    pub fn set_patch_live(&mut self, patch: Op505Patch) {
        self.current_patch = patch;
        let channel_ids: Vec<usize> = self.channels.keys().copied().collect();
        for channel in channel_ids {
            self.set_channel_params(channel, patch.channel);
            for (op_index, op) in patch.operators.iter().enumerate() {
                self.set_operator_params(channel, op_index, *op);
            }
        }
    }

    pub fn current_patch(&self) -> Op505Patch {
        self.current_patch
    }

    /// チャンネルパラメーターを差し替える。固定音階3フィールド（`fixed_note_enable`/
    /// `fixed_note`/`fixed_note_fine`）が変化したときだけ、発音中のピッチを再計算して
    /// 各オペレーターへ反映する（差分検知なので変化が無ければ従来どおり出力はビット不変。
    /// これが無いとエディタでFIXED NOTEノブを回しても発音中の音のピッチが変わらない）。
    pub fn set_channel_params(&mut self, channel: usize, params: Op505ChannelParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            let fixed_note_changed = ch.channel_params.fixed_note_enable != params.fixed_note_enable
                || ch.channel_params.fixed_note != params.fixed_note
                || ch.channel_params.fixed_note_fine != params.fixed_note_fine;
            ch.channel_params = params;
            if fixed_note_changed {
                let (frequency, note) = effective_pitch(ch.keyed_frequency, &params);
                ch.base_frequency = frequency;
                ch.note = note;
                for op in ch.operators.iter_mut() {
                    op.set_base_frequency(frequency);
                }
            }
        }
    }

    pub fn set_operator_params(&mut self, channel: usize, op_index: usize, params: Op505OperatorParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].params = params;
        }
    }

    pub fn set_operator_f_number(&mut self, channel: usize, op_index: usize, f_number: u16) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].set_f_number_override(f_number);
        }
    }

    pub fn set_pitch_fg_rate_scale(&mut self, channel: usize, scale: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.pitch_fg_rate_scale = scale;
        }
    }

    pub fn note_on_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_on_operator(op_index);
        }
    }

    pub fn note_off_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off_operator(op_index);
        }
    }

    /// ユーザー定義波形を書き込む。`slot`は`FIRST_USER_WAVE_SLOT`以降でなければならない
    /// （それより手前はビルトイン波形とノイズが占有する予約領域。スロットマップは
    /// `FIRST_USER_WAVE_SLOT`のドキュメントコメント参照）。
    pub fn set_user_wave(&mut self, slot: u8, input: &[i8; 32]) {
        assert!(
            slot >= FIRST_USER_WAVE_SLOT,
            "slots 0-{} are reserved for builtin waves and noise",
            FIRST_USER_WAVE_SLOT - 1
        );
        self.wave_tables[slot as usize] = Some(convert_wave_32(input));
    }

    pub fn silence_group(&mut self, group: usize) {
        self.channels.retain(|id, _| id >> 7 != group);
    }

    pub fn active_voice_count(&self) -> usize {
        self.channels.len()
    }

    /// 発音中のボイスIDを`out`へ書き出す（`out`は呼び出し側が事前に確保する。
    /// オーディオスレッドからの毎ブロック呼び出しでアロケーションが走らないようにするため）。
    pub fn collect_active_channels(&self, out: &mut Vec<usize>) {
        out.clear();
        out.extend(self.channels.keys().copied());
    }
}

impl Vco for Op505Engine {
    fn note_on(&mut self, channel: usize, frequency: f32, velocity: u8) {
        let patch = self.current_patch;
        if let Some(ch) = self.channels.get_mut(&channel) {
            if !ch.is_idle() {
                ch.retrigger(frequency, velocity, patch);
                return;
            }
        } else if self.channels.len() >= self.max_voices {
            self.steal_one_voice();
        }
        self.channels.insert(channel, Channel::new(frequency, velocity, patch));
    }

    fn note_off(&mut self, channel: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off();
        }
    }

    fn set_pitch_bend(&mut self, channel: usize, cents: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.bend_cents = cents;
        }
    }

    fn set_pitch_bend_group(&mut self, group: usize, cents: f32) {
        for (id, ch) in self.channels.iter_mut() {
            if id >> 7 == group {
                ch.bend_cents = cents;
            }
        }
    }

    fn set_channel_volume(&mut self, channel: usize, gain: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.channel_gain = gain.max(0.0);
        }
    }

    fn set_channel_volume_group(&mut self, group: usize, gain: f32) {
        for (id, ch) in self.channels.iter_mut() {
            if id >> 7 == group {
                ch.channel_gain = gain.max(0.0);
            }
        }
    }

    /// TimeEgのテンポ同期（`sync_enabled`）に使うBPMを更新する。0以下は無視する
    /// （ホストDAWのTransportが未再生等で`tempo`を返さない場合の防御、既存値を保持する）。
    fn set_tempo(&mut self, bpm: f32) {
        if bpm > 0.0 {
            self.tempo_bpm = bpm;
        }
    }

    fn render(&mut self, output: &mut [f32], num_channels: usize) {
        let num_channels = num_channels.max(1);
        let sample_rate = self.sample_rate;
        let wave_tables = &self.wave_tables;
        let tempo_bpm = self.tempo_bpm;
        let frames = output.len().div_ceil(num_channels);
        self.mix_buf.clear();
        self.mix_buf.resize(frames, 0.0);
        for ch in self.channels.values_mut() {
            for mix in self.mix_buf.iter_mut() {
                if ch.is_idle() {
                    break;
                }
                *mix += ch.tick(sample_rate, wave_tables, tempo_bpm);
            }
        }
        for (frame, &mix) in output.chunks_mut(num_channels).zip(self.mix_buf.iter()) {
            for s in frame.iter_mut() {
                *s += mix;
            }
        }
        self.channels.retain(|_, ch| !ch.is_idle());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    // `ChipLfo`は本番コードから退役済み（CHIP LFO完全退役）。ここではGain FGへの厳密変換が
    // 実機挙動と一致することを裏取りするオラクルとしてのみ使う（chip_lfo.rs冒頭コメント参照）。
    use sound_fm::chip_lfo::ChipLfo;

    fn stages_with(entries: &[(u8, u8, u8)]) -> [TimeStage; sound_core::MAX_STAGES] {
        let mut stages = [TimeStage::default(); sound_core::MAX_STAGES];
        for (i, &(time, level, curve)) in entries.iter().enumerate() {
            stages[i] = TimeStage { time, level, curve };
        }
        stages
    }

    /// 瞬時に満レベルへ到達しそのまま無限サスティンするEG（ym38x6版loud_patchのAR=255/D1L=255相当）。
    /// 段1はリリース用（OP EGは必ずレベル0へ着地させる。`ui_core::TimeEgProfile`参照）。
    fn instant_sustain_eg() -> TimeEgParams {
        TimeEgParams {
            stages: stages_with(&[(0, 255, 0), (0, 0, 0)]),
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
         ..Default::default()}
    }

    /// 全Opがアルゴリズム7（全並列）で即音量最大・サスティン無限のテスト用パッチ。
    fn loud_patch(velocity_sensitivity: u8) -> Op505Patch {
        let op_params = Op505OperatorParams {
            tl: 255,
            eg: instant_sustain_eg(),
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity,
            waveform: 0,
            op_fine_tune: 128,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        };
        let mut patch = Op505Patch::default();
        patch.operators = [op_params; 4];
        patch.channel.algorithm = 7;
        patch
    }

    #[test]
    fn voice_steal_caps_channel_count() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(4);
        for ch in 0..8 {
            engine.note_on(ch, 440.0, 100);
        }
        assert_eq!(engine.channels.len(), 4, "上限を超えて積み上がってはいけない");
    }

    #[test]
    fn voice_steal_prefers_released_over_held() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(2);
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_off(0);

        engine.note_on(2, 440.0, 100);

        assert!(!engine.channels.contains_key(&0), "release中のボイスが優先的に奪われるはず");
        assert!(engine.channels.contains_key(&1), "発音中のボイスは残るはず");
        assert!(engine.channels.contains_key(&2), "新規ボイスは確保されるはず");
        assert_eq!(engine.channels.len(), 2);
    }

    #[test]
    fn collect_active_channels_reports_only_sounding_voices() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_on(2, 440.0, 100);

        let mut ids = Vec::new();
        engine.collect_active_channels(&mut ids);
        assert_eq!(ids, vec![0, 1, 2], "BTreeMapのキー昇順で決定論的に列挙されるはず");

        engine.silence_group(0);
        engine.collect_active_channels(&mut ids);
        assert!(ids.is_empty(), "silence_groupで消えたチャンネルは含まれないはず");
    }

    #[test]
    fn retrigger_does_not_trigger_steal() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(2);
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_on(0, 880.0, 100);

        assert!(engine.channels.contains_key(&0));
        assert!(engine.channels.contains_key(&1), "retriggerが他ボイスを巻き込んで奪ってはいけない");
        assert_eq!(engine.channels.len(), 2);
    }

    /// ブロック一括レンダリングと1サンプルずつのレンダリングがビット単位で一致することを確認する
    /// （render()のチャンネル外側×サンプル内側ループが数値的に完全等価であることの回帰テスト）。
    /// フィードバック・フィルター・リリース途中のidle化まで通る条件で比較する。
    #[test]
    fn block_render_matches_sample_by_sample_render() {
        let mut patch = loud_patch(0);
        patch.channel.feedback = 100;
        patch.channel.filter_cutoff = 200;
        patch.channel.filter_resonance = 80;
        // 瞬時attack→decay(40)で静止、リリース段(2)から短時間でidleへ落ちる形。
        let eg = TimeEgParams {
            stages: stages_with(&[(0, 255, 0), (60, 40, 0), (60, 0, 0)]),
            stage_count: 3,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 1,
         ..Default::default()};
        for op in patch.operators.iter_mut() {
            op.eg = eg;
        }

        let mut block = Op505Engine::new(44100.0);
        let mut single = Op505Engine::new(44100.0);
        for engine in [&mut block, &mut single] {
            engine.set_patch(patch);
            engine.note_on(0, 220.0, 100);
            engine.note_on(1, 330.0, 90);
            engine.note_on(2, 440.0, 127);
        }

        const HALF: usize = 2048;
        let mut out_block = vec![0.0f32; HALF * 2];
        let mut out_single = vec![0.0f32; HALF * 2];

        block.render(&mut out_block[..HALF], 1);
        for i in 0..HALF {
            single.render(&mut out_single[i..i + 1], 1);
        }
        for engine in [&mut block, &mut single] {
            engine.note_off(0);
            engine.note_off(1);
            engine.note_off(2);
        }
        block.render(&mut out_block[HALF..], 1);
        for i in HALF..HALF * 2 {
            single.render(&mut out_single[i..i + 1], 1);
        }

        assert!(out_block.iter().any(|&s| s != 0.0), "expected non-silent output");
        for (i, (a, b)) in out_block.iter().zip(out_single.iter()).enumerate() {
            assert_eq!(a, b, "sample {i} differs: block={a}, single={b}");
        }
    }

    #[test]
    fn note_on_produces_non_silent_output() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.note_on(0, 440.0, 100);
        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        assert!(buf.iter().any(|&s| s != 0.0), "expected non-silent output");
    }

    // -----------------------------------------------------------------------
    // FG無効化（stage_count==0、`ui_core::TimeEgProfile::min_stages=0`で許容）
    // -----------------------------------------------------------------------

    fn render_512(patch: Op505Patch) -> Vec<f32> {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(patch);
        engine.note_on(0, 440.0, 100);
        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        buf
    }

    /// Gain FG無効時(STAGES=0、`default_gain_fg`の既定そのもの)は**1.0（透過）**のはずで、
    /// 無音にならない。さらに、STAGES=0化より前の旧`.op505`資産が持っていた1段255・透過構成
    /// （ゲートを閉じない旧既定の実際の形）とビット完全一致するはず（「無効」と「(旧)透過既定」は
    /// 音として区別できないのが正しい設計。既存プリセットの再変換で音が変わらないことの根拠）。
    #[test]
    fn disabled_gain_fg_matches_default_transparent_bit_for_bit() {
        let disabled = loud_patch(0); // gain_fgは既にdefault_gain_fg()(STAGES=0)

        let mut legacy_transparent = loud_patch(0);
        legacy_transparent.channel.gain_fg = TimeEgParams {
            stages: {
                let mut stages = [TimeStage::default(); sound_core::MAX_STAGES];
                stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
                stages
            },
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
            ..Default::default()
        };

        let out_disabled = render_512(disabled);
        let out_legacy = render_512(legacy_transparent);
        assert!(out_disabled.iter().any(|&s| s != 0.0), "Gain FG無効時は無音にならないはず");
        assert_eq!(out_disabled, out_legacy, "STAGES=0は旧1段透過既定とビット一致するはず");
    }

    /// Pitch FG無効時(STAGES=0)はdepthの値に関わらず変調量ゼロのはず。depth=200を設定しても
    /// 既定（変調なし）とビット一致することで、「無効はdepthを無視して常にゼロ」を確認する。
    #[test]
    fn disabled_pitch_fg_ignores_depth_and_applies_no_modulation() {
        let mut disabled = loud_patch(0);
        disabled.channel.pitch_fg.eg.stage_count = 0;
        disabled.channel.pitch_fg.depth = 200;

        let neutral_default = loud_patch(0); // pitch_fgは既にneutral_bipolar_eg()+depth=0(変調なし)

        let out_disabled = render_512(disabled);
        let out_default = render_512(neutral_default);
        assert_eq!(out_disabled, out_default, "STAGES=0はdepthを無視し無変調(既定)とビット一致するはず");
    }

    /// Cutoff FG無効時(STAGES=0)はdepthの値に関わらず基準Cutoffのままのはず。
    #[test]
    fn disabled_cutoff_fg_ignores_depth_and_uses_base_cutoff() {
        let mut patch = loud_patch(0);
        patch.channel.filter_resonance = 200; // カットオフの差が音に出やすいよう強調
        let mut disabled = patch.clone();
        disabled.channel.cutoff_fg.eg.stage_count = 0;
        disabled.channel.cutoff_fg.depth = 200;

        let neutral_default = patch; // cutoff_fgは既にneutral_bipolar_eg()+depth=0(変調なし)

        let out_disabled = render_512(disabled);
        let out_default = render_512(neutral_default);
        assert_eq!(out_disabled, out_default, "STAGES=0はdepthを無視し基準Cutoffのまま(既定)とビット一致するはず");
    }

    /// 全8アルゴリズム×フィルター自己発振ONで、長時間レンダリングしてもNaN/Infが出ないことを確認する。
    #[test]
    fn all_algorithms_long_run_no_nan() {
        let sr = 44100.0;
        for algo in 0u8..8 {
            let mut patch = loud_patch(0);
            patch.channel.algorithm = algo;
            patch.channel.feedback = 150;
            patch.channel.filter_cutoff = 180;
            patch.channel.filter_resonance = 255;
            patch.channel.filter_self_oscillation = true;
            let mut engine = Op505Engine::new(sr);
            engine.set_patch(patch);
            engine.note_on(0, 440.0, 100);
            let mut buf = vec![0.0f32; 4096];
            engine.render(&mut buf, 1);
            assert!(buf.iter().all(|s| s.is_finite()), "algo {algo}: expected finite output");
        }
    }

    #[test]
    fn op505_patch_serde_json_round_trips() {
        let mut patch = loud_patch(64);
        patch.channel.gain_fg = TimeEgParams {
            stages: stages_with(&[(15, 230, 0), (40, 230, 0), (15, 40, 0), (40, 40, 0)]),
            stage_count: 4,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 3,
         ..Default::default()};
        let json = serde_json::to_string(&patch).expect("serialize");
        let restored: Op505Patch = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(patch, restored, "Op505Patch should round-trip through JSON unchanged");
    }

    // -----------------------------------------------------------------------
    // 固定音階（fixed_note_enable、GM2リズムチャンネル用）
    // -----------------------------------------------------------------------

    /// 固定音階が有効なら、note_onへ渡す周波数に関わらず出力がビット単位で一致する
    /// （GM2ドラムキットの「ノート番号＝音色選択であってピッチではない」を実現する根拠）。
    #[test]
    fn fixed_note_ignores_note_on_frequency() {
        let mut patch = loud_patch(0);
        patch.channel.fixed_note_enable = true;
        patch.channel.fixed_note = 60;

        let mut low = Op505Engine::new(44100.0);
        low.set_patch(patch);
        low.note_on(0, 220.0, 100);

        let mut high = Op505Engine::new(44100.0);
        high.set_patch(patch);
        high.note_on(0, 880.0, 100);

        let mut buf_low = vec![0.0f32; 512];
        let mut buf_high = vec![0.0f32; 512];
        low.render(&mut buf_low, 1);
        high.render(&mut buf_high, 1);

        assert_eq!(buf_low, buf_high, "固定音階が有効ならnote_onの周波数に関わらず同じ出力になるはず");
    }

    /// `fixed_note_enable=false`（既定）では従来どおりnote_onの周波数で音程が変わる
    /// （新フィールドが常時ONになる回帰を防ぐ）。
    #[test]
    fn fixed_note_disabled_is_unchanged() {
        let patch = loud_patch(0); // fixed_note_enable=falseが既定

        let mut low = Op505Engine::new(44100.0);
        low.set_patch(patch);
        low.note_on(0, 220.0, 100);

        let mut high = Op505Engine::new(44100.0);
        high.set_patch(patch);
        high.note_on(0, 880.0, 100);

        let mut buf_low = vec![0.0f32; 512];
        let mut buf_high = vec![0.0f32; 512];
        low.render(&mut buf_low, 1);
        high.render(&mut buf_high, 1);

        assert_ne!(buf_low, buf_high, "固定音階が無効なら従来どおりnote_onの周波数で音程が変わるはず");
    }

    /// 固定音階が有効でも、ピッチベンド（セント加算）は従来どおり効く
    /// （固定音階はキー周波数だけを差し替え、DT1/op_fine_tune/ピッチ変調と同じ加算経路は
    /// 素通りする設計のため）。
    #[test]
    fn fixed_note_still_responds_to_pitch_bend() {
        let mut patch = loud_patch(0);
        patch.channel.fixed_note_enable = true;
        patch.channel.fixed_note = 60;

        let mut unbent = Op505Engine::new(44100.0);
        unbent.set_patch(patch);
        unbent.note_on(0, 440.0, 100);

        let mut bent = Op505Engine::new(44100.0);
        bent.set_patch(patch);
        bent.note_on(0, 440.0, 100);
        bent.set_pitch_bend(0, 200.0); // +2半音相当

        let mut buf_unbent = vec![0.0f32; 512];
        let mut buf_bent = vec![0.0f32; 512];
        unbent.render(&mut buf_unbent, 1);
        bent.render(&mut buf_bent, 1);

        assert_ne!(buf_unbent, buf_bent, "固定音階中もピッチベンド(セント加算)は従来どおり効くはず");
    }

    // -----------------------------------------------------------------------
    // chip_lfo_pitch_to_pitch_fg（CHIP LFOピッチ経路→Pitch FG移設）
    // -----------------------------------------------------------------------

    #[test]
    fn chip_lfo_pitch_to_pitch_fg_off_when_pms_or_pmd_zero() {
        let default_fg = Op505BipolarFg::default();
        assert_eq!(chip_lfo_pitch_to_pitch_fg(0, 200, 128, 0), default_fg);
        assert_eq!(chip_lfo_pitch_to_pitch_fg(200, 0, 128, 0), default_fg);
    }

    #[test]
    fn chip_lfo_pitch_to_pitch_fg_builds_triangle_loop() {
        let fg = chip_lfo_pitch_to_pitch_fg(255, 255, 128, 0);
        assert_eq!(fg.eg.stage_count, 4);
        assert_eq!(fg.eg.loop_enabled, 1);
        assert_eq!(fg.eg.loop_start, 2);
        assert_eq!(fg.eg.release_point, 3);
        assert_eq!(fg.eg.stages[0].level, BIPOLAR_NEUTRAL_RAW, "delay段は中央(無変調)");
        assert_eq!(fg.eg.stages[1].time, 0, "谷への瞬時ジャンプ");
        assert_eq!(fg.eg.stages[1].level, 0, "谷");
        assert_eq!(fg.eg.stages[2].level, 255, "山");
        assert_eq!(fg.eg.stages[3].level, 0, "谷");
        assert_eq!(fg.eg.stages[2].time, fg.eg.stages[3].time, "半周期は対称");
        // pms=255, pmd=255 → 700cents、depth = 700/1200*255 ≈ 149
        assert_eq!(fg.depth, 149);
    }

    #[test]
    fn chip_lfo_pitch_to_pitch_fg_delay_becomes_leading_neutral_stage() {
        let no_delay = chip_lfo_pitch_to_pitch_fg(200, 200, 128, 0);
        let with_delay = chip_lfo_pitch_to_pitch_fg(200, 200, 128, 200);
        assert_eq!(no_delay.eg.stages[0].time, 0);
        assert!(with_delay.eg.stages[0].time > 0, "delay>0はdelay段のtimeへ反映される");
        // ループ部分（stage2/3の周期）はdelayの有無に依存しない。
        assert_eq!(no_delay.eg.stages[2].time, with_delay.eg.stages[2].time);
    }

    /// 変換後のPitch FGをTimeEgで実際に鳴らし、CHIP LFOの三角波と山谷が一致することを確認する
    /// （厳密な波形一致の裏取り。`bipolar_level`の正側127/128頭打ちのみ許容誤差）。
    #[test]
    fn chip_lfo_pitch_to_pitch_fg_matches_chip_lfo_peak_amplitude() {
        let sr = 44100.0;
        let pms = 180u8;
        let pmd = 220u8;
        let freq = 150u8;
        let expected_cents = pms_to_cents_range(pms) * (pmd as f32 / 255.0);

        let fg = chip_lfo_pitch_to_pitch_fg(pms, pmd, freq, 0);
        let mut eg = TimeEg::new_bipolar();
        eg.note_on();
        let mut max_cents = f32::MIN;
        let mut min_cents = f32::MAX;
        // 数周期分ティックして山谷の到達を確認する。
        let cycle_seconds = 1.0 / chip_lfo_freq_to_hz(freq) as f64;
        let samples = ((cycle_seconds * 4.0) * sr as f64) as usize;
        for _ in 0..samples {
            let out = eg.tick(sr, fg.eg, 1.0);
            let cents = bipolar_level(out) * (fg.depth as f32 / 255.0) * 1200.0;
            max_cents = max_cents.max(cents);
            min_cents = min_cents.min(cents);
        }
        assert!((min_cents - -expected_cents).abs() < 5.0, "min_cents={min_cents} expected=-{expected_cents}");
        assert!((max_cents - expected_cents).abs() < 5.0, "max_cents={max_cents} expected={expected_cents}");
    }

    // -----------------------------------------------------------------------
    // chip_lfo_am_to_gain_fg（CHIP LFO AM経路→Gain FG厳密変換）
    // -----------------------------------------------------------------------

    #[test]
    fn chip_lfo_am_to_gain_fg_off_when_ams_or_amd_zero() {
        assert_eq!(chip_lfo_am_to_gain_fg(0, 200, 128, 0), None);
        assert_eq!(chip_lfo_am_to_gain_fg(200, 0, 128, 0), None);
    }

    #[test]
    fn chip_lfo_am_to_gain_fg_builds_expected_stages() {
        let fg = chip_lfo_am_to_gain_fg(255, 255, 128, 0).expect("depth>0のはず");
        assert_eq!(fg.stage_count, 5);
        assert_eq!(fg.loop_enabled, 1);
        assert_eq!(fg.loop_start, 2);
        assert_eq!(fg.release_point, 4, "最終段＝note-offでループを止めない");
        assert_eq!(fg.stages[0].time, 0, "キーオン直後は瞬時に255へ");
        assert_eq!(fg.stages[0].level, 255);
        assert_eq!(fg.stages[1].level, 255, "位相0〜0.25は平坦（クランプ側）");
        assert_eq!(fg.stages[2].time, fg.stages[3].time, "下降/上昇は対称な1/4周期");
        assert_eq!(fg.stages[3].level, 255, "山へ復帰");
        // ams=255,amd=255 → depth=ams_to_depth(255)≈1.0 → floorはほぼ0
        assert!(fg.stages[2].level <= 2, "深いAMではfloorがほぼ0付近: {}", fg.stages[2].level);
        // 周期境界をまたぐ平坦段（stage4）は1/4周期段（stage2）のほぼ2倍の長さ
        let quarter = sound_core::time_to_seconds(fg.stages[2].time);
        let half = sound_core::time_to_seconds(fg.stages[4].time);
        assert!(
            (half - 2.0 * quarter).abs() / half < 0.06,
            "half={half} quarter={quarter}（量子化許容誤差内で2倍のはず）"
        );
    }

    #[test]
    fn chip_lfo_am_to_gain_fg_delay_extends_leading_flat_stage() {
        let no_delay = chip_lfo_am_to_gain_fg(200, 200, 128, 0).unwrap();
        let with_delay = chip_lfo_am_to_gain_fg(200, 200, 128, 200).unwrap();
        assert!(with_delay.stages[1].time > no_delay.stages[1].time, "delay>0は先頭平坦段を延ばす");
        // ループ部分（stage2〜4の周期）はdelayの有無に依存しない。
        assert_eq!(no_delay.stages[2].time, with_delay.stages[2].time);
        assert_eq!(no_delay.stages[4].time, with_delay.stages[4].time);
    }

    /// 変換後のGain FGを実際にTimeEgで鳴らし、CHIP LFOのAM（`operator.rs`と同じ
    /// `(1-triangle×depth).clamp(0,1)`式）が到達する振幅の谷/山と一致することを確認する
    /// （厳密な波形一致の裏取り。`chip_lfo_pitch_to_pitch_fg_matches_chip_lfo_peak_amplitude`と
    /// 同じ手法：量子化された`time`とCHIP LFOの連続hzは長時間では位相がずれるため、
    /// 位相非依存の指標＝振幅の到達範囲で照合する）。
    #[test]
    fn chip_lfo_am_to_gain_fg_matches_chip_lfo_amplitude_extremes() {
        let sr = 44100.0;
        let ams = 180u8;
        let amd = 200u8;
        let freq = 150u8;
        let depth = ams_to_depth(ams) * (amd as f32 / 255.0);
        let expected_floor = ((1.0 - depth) * 255.0).round();

        let gain_fg = chip_lfo_am_to_gain_fg(ams, amd, freq, 0).expect("depth>0のはず");
        let mut eg = TimeEg::new();
        eg.note_on();
        let mut min_level = f32::MAX;
        let mut max_level = f32::MIN;
        // 数周期分ティックして山谷の到達を確認する。
        let cycle_seconds = 1.0 / chip_lfo_freq_to_hz(freq) as f64;
        let samples = ((cycle_seconds * 4.0) * sr as f64) as usize;
        for _ in 0..samples {
            let level = eg.tick(sr, gain_fg, 1.0) * 255.0;
            min_level = min_level.min(level);
            max_level = max_level.max(level);
        }
        assert!((min_level - expected_floor).abs() < 3.0, "min_level={min_level} expected={expected_floor}");
        assert!((max_level - 255.0).abs() < 1.0, "max_level={max_level}");

        // 参照実装（CHIP LFO本体 + operator.rsと同じamp_factor式）でも同じ谷/山に到達することを
        // 確認する。ChipLfoは連続hzで駆動されGain FGは量子化された`time`で駆動されるため
        // 長時間では位相がずれるが、振幅の到達範囲（位相非依存）はどちらも同じ式
        // `ams_to_depth(ams)×(amd/255)`から導かれるので一致するはず。
        let mut chip_lfo = ChipLfo::new();
        chip_lfo.note_on();
        let mut ref_min = f32::MAX;
        let mut ref_max = f32::MIN;
        for _ in 0..samples {
            let triangle = chip_lfo.tick(sr, freq, 0);
            let chip_amp_mod = triangle * depth;
            let amp_factor = (1.0 - chip_amp_mod).clamp(0.0, 1.0);
            ref_min = ref_min.min(amp_factor * 255.0);
            ref_max = ref_max.max(amp_factor * 255.0);
        }
        assert!((ref_min - expected_floor).abs() < 1.0, "ref_min={ref_min} expected={expected_floor}");
        assert!((min_level - ref_min).abs() < 3.0, "min_level={min_level} ref_min={ref_min}");
        assert!((max_level - ref_max).abs() < 1.0, "max_level={max_level} ref_max={ref_max}");
    }

    /// スロットマップ（0〜31=ビルトイン／32〜63=ノイズ／64〜255=ユーザー）を固定する。
    /// ここが動いたら`set_user_wave`のガードと`sound-fm::waveform`冒頭のスロット表も追随が必要。
    #[test]
    fn first_user_wave_slot_is_after_builtin_and_noise() {
        assert_eq!(FIRST_USER_WAVE_SLOT, 64);
        assert!(FIRST_USER_WAVE_SLOT >= waveform::BUILTIN_WAVEFORM_COUNT);
        assert!(!waveform::is_noise_waveform(FIRST_USER_WAVE_SLOT));
    }

    #[test]
    fn set_user_wave_writes_into_a_free_slot() {
        let mut engine = Op505Engine::new(48_000.0);
        assert!(
            engine.wave_tables[FIRST_USER_WAVE_SLOT as usize].is_none(),
            "FIRST_USER_WAVE_SLOTは初期状態では空きスロットのはず"
        );

        engine.set_user_wave(FIRST_USER_WAVE_SLOT, &[100; 32]);

        let table = engine.wave_tables[FIRST_USER_WAVE_SLOT as usize].as_ref().expect("書き込まれるはず");
        // convert_wave_32は入力を/128.0で正規化する（全要素同値なら補間しても一定値）。
        assert!((table.sample_at(0) - 100.0 / 128.0).abs() < 0.01, "sample_at(0)={}", table.sample_at(0));
    }

    /// 回帰防止: このガードは「ビルトインは8波形」時代の`slot >= 8`のまま取り残されており、
    /// ビルトインが0〜31へ拡張された後もスロット8〜31を上書きできる状態になっていた。
    #[test]
    #[should_panic(expected = "reserved")]
    fn set_user_wave_rejects_builtin_slot() {
        let mut engine = Op505Engine::new(48_000.0);
        engine.set_user_wave(waveform::BUILTIN_WAVEFORM_COUNT - 1, &[100; 32]);
    }

    /// ノイズ帯（32〜63）は`Operator::tick`が`is_noise_waveform`をテーブル参照より先に判定するため、
    /// 書き込めても読まれずノイズが鳴る＝エラーの出ないサイレントな失敗になる。ガードで弾く。
    #[test]
    #[should_panic(expected = "reserved")]
    fn set_user_wave_rejects_noise_slot() {
        let mut engine = Op505Engine::new(48_000.0);
        engine.set_user_wave(waveform::NOISE_WAVEFORM_BASE, &[100; 32]);
    }

    /// ユーザー波形の書き込みがビルトイン波形を壊さないこと（上記ガードの実効性の裏取り）。
    #[test]
    fn set_user_wave_leaves_builtin_waveforms_intact() {
        let mut engine = Op505Engine::new(48_000.0);
        let before: Vec<f32> = (0..waveform::BUILTIN_WAVEFORM_COUNT)
            .map(|i| engine.wave_tables[i as usize].as_ref().expect("ビルトインは初期化済み").sample_at(256))
            .collect();

        engine.set_user_wave(FIRST_USER_WAVE_SLOT, &[100; 32]);

        for (i, expected) in before.iter().enumerate() {
            let actual = engine.wave_tables[i].as_ref().expect("ビルトインが消えていないこと").sample_at(256);
            assert_eq!(actual, *expected, "ビルトイン波形{i}が書き換わっている");
        }
    }
}
