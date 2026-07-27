pub mod algorithm;
pub mod chip_lfo;
pub mod mapping;
pub mod operator;
pub mod preset;
pub mod waveform;

use std::collections::BTreeMap;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use algorithm::ALGORITHMS;
use mapping::{
    carrier_velocity_gain, feedback_to_scale_with_max, frequency_to_note, velocity_to_volume_gain,
    FM_MODULATION_INDEX_SCALE,
};
use operator::Operator;

// ---------------------------------------------------------------------------
// フィードバック帰還方式: 2サンプル平均（実機OPM/OPN/OPZ準拠、既定）
//
// 実機OPM/OPN/OPZは feedback 経路を (out[n-1]+out[n-2])/2 の2サンプル平均で帰還する。
// 38x6は従来 out[n-1] のみの1サンプル帰還だった。
//
// 【2026-06-20の誤判定について】当時「2サンプル平均は音色帯(基音415Hz)で1サンプルと
// ほぼ完全一致し、OPZ忠実度を上げる効果は無い」と判定したが、これは opz2x6 側の
// mod_tl_cap=180（モジュレーターTL上限）が変調を潰した状態での測定だった。
// 変調を解放した状態（opz2x6 mod_cap=None、2026-07-18〜既定）で再測定すると、
// opzref(ymfm実機参照)基準で2サンプル平均の方がハイハット系のノイズ忠実度・
// OPNベースのピッチ安定性ともに1サンプルより実機に近いことを確認し、既定を true へ変更した。
// 1サンプルとのA/B比較用に set_feedback_two_sample(false) は残す
// （各コンバーターの --fb-1sample / --fb-2sample から到達可能）。
// ---------------------------------------------------------------------------

/// true で feedback 経路を (out[n-1]+out[n-2])/2 の2サンプル平均にする（既定 true、実機準拠）。
static FEEDBACK_TWO_SAMPLE: AtomicBool = AtomicBool::new(true);
/// feedback_to_scale の最大値 override。f32 を to_bits で格納し 0 を「未設定（=1.8）」とする。
static FEEDBACK_SCALE_MAX_BITS: AtomicU32 = AtomicU32::new(0);

/// feedback 帰還方式を切り替える（既定 true=2サンプル平均、実機準拠）。
/// false にすると旧1サンプル帰還に戻る（A/B比較・診断用）。
pub fn set_feedback_two_sample(enabled: bool) {
    FEEDBACK_TWO_SAMPLE.store(enabled, Ordering::Relaxed);
}

/// 実験用: feedback_to_scale の最大値を override する。None で既定(1.8)へ戻す。
pub fn set_feedback_scale_max(max: Option<f32>) {
    let bits = match max {
        Some(m) if m > 0.0 => m.to_bits(),
        _ => 0,
    };
    FEEDBACK_SCALE_MAX_BITS.store(bits, Ordering::Relaxed);
}

fn current_feedback_scale_max() -> f32 {
    let bits = FEEDBACK_SCALE_MAX_BITS.load(Ordering::Relaxed);
    if bits == 0 {
        1.8
    } else {
        f32::from_bits(bits)
    }
}
// Ym38x6Patch::operators / set_operator_paramsの型として外部に公開する
pub use operator::OperatorParams;
pub use preset::{
    gm2_bank0_patch, placeholder_patch, presets_dir, waveform_memory_patch, Preset, PresetBank,
    PresetEntry, PresetFile, WAVEFORM_MEMORY_BANK,
};
use serde::{Deserialize, Serialize};
use sound_core::{apply_lfo_modulation, convert_wave_32, Eg, PerformanceLfo, PerformanceLfoTarget, WaveTable};
use chip_lfo::{ams_to_depth, pms_to_cents_range, ChipLfo};
use waveform::gen_builtin_waveform;

// 呼び出し側がsound-coreに直接依存しなくて済むようre-export
// （`Vco`トレイトは同クレート内の`impl Vco for Ym38x6Engine`でも使う）
pub use sound_core::{
    cc76_to_rate_scale, lfo_fade_mode_from_index, lfo_offset_from_param, lfo_offset_to_param,
    lfo_waveform_from_index, pitch_depth_cents, volume_depth, cutoff_depth, AdsrParams,
    AudioProcessor, BipolarFg, ChorusType, EgParams, FilterType, GainFg, LfoDestination,
    LfoFadeMode, LfoWaveform, MasterEffects, PerformanceLfoShape, ReverbType, Vca, Vcf, Vco,
    VoiceAmp, VoiceFilter,
};

// ---------------------------------------------------------------------------
// パッチ（チャンネル + オペレーター4個分のパラメーター一式）
// ---------------------------------------------------------------------------

/// 質感LFO（spec-sound.md「質感LFO（5波形専用・焼き込み）」節）。旧「チャンネルLFO」を再編し、
/// FGのループ（Floor⇄peak）では表せない5波形（矩形/台形/S&H/Random/Chaos）だけを担う、
/// 焼き込み専用（演奏CCによる補正を受けない）の1基。全項目を`ChannelParams`が所有する。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureLfo {
    /// 0=矩形波/1=台形波/2=S&H/3=Random/4=Chaos。
    pub waveform: u8,
    /// 0=Pitch/1=Volume/2=TL（キャリア一括）/3=Cutoff/4=未接続（`Ym38x6LfoDestination`と同じ並び）。
    pub destination: u8,
    pub rate: u8,
    pub depth: u8,
    pub delay: u8,
    /// 0=ON-IN/1=ON-OUT/2=OFF-IN/3=OFF-OUT。
    pub fade_mode: u8,
    pub fade_time: u8,
    /// 波形の中心シフト(0〜255、中心128＝オフセットなし)。
    pub offset: u8,
}

impl Default for TextureLfo {
    /// 既定Depth=0で鳴らない（旧パッチ挙動と互換の「無効」状態）。
    fn default() -> Self {
        Self { waveform: 0, destination: 0, rate: 0, depth: 0, delay: 0, fade_mode: 0, fade_time: 0, offset: 128 }
    }
}

/// 質感LFOの5波形インデックス(0〜4)を、内部で再利用する`sound_core::PerformanceLfo`の
/// 8波形`LfoWaveform`へ写像する（三角/サイン/のこぎりには写像しない＝新エンコーディングの範囲外）。
fn texture_lfo_waveform_to_engine(waveform: u8) -> LfoWaveform {
    match waveform {
        1 => LfoWaveform::Trapezoid,
        2 => LfoWaveform::SampleHold,
        3 => LfoWaveform::Random,
        4 => LfoWaveform::Chaos,
        _ => LfoWaveform::Square,
    }
}

/// [texture_lfo_waveform_to_engine]の逆方向。旧`perf_lfo_shape`からの後方互換マイグレーション専用
/// （三角/サイン/のこぎりは質感LFOのパレット外のため`None`を返す）。
fn texture_lfo_waveform_from_engine(waveform: LfoWaveform) -> Option<u8> {
    match waveform {
        LfoWaveform::Square => Some(0),
        LfoWaveform::Trapezoid => Some(1),
        LfoWaveform::SampleHold => Some(2),
        LfoWaveform::Random => Some(3),
        LfoWaveform::Chaos => Some(4),
        LfoWaveform::Triangle | LfoWaveform::Sine | LfoWaveform::Saw => None,
    }
}

/// 質感LFOの波形/Fade/Offset設定を、`sound_core::PerformanceLfo`が受け取る
/// `PerformanceLfoShape`へ変換する（rate/delay/destination/depthは別途セッターで設定する）。
fn texture_lfo_to_shape(texture_lfo: TextureLfo) -> PerformanceLfoShape {
    PerformanceLfoShape {
        waveform: texture_lfo_waveform_to_engine(texture_lfo.waveform),
        fade_mode: lfo_fade_mode_from_index(texture_lfo.fade_mode),
        fade_time: texture_lfo.fade_time,
        offset: lfo_offset_from_param(texture_lfo.offset),
    }
}

/// `ChannelParams`の一部（`chip_lfo_*`等）を除く、チャンネル単位パラメーター一式。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ChannelParams {
    /// アルゴリズム番号(0〜7)。
    pub algorithm: u8,
    /// フィードバック深さ(0〜255)。
    pub feedback: u8,
    /// チップ内LFO周波数。JSON上の旧名`tone_lfo_freq`を維持する（既存`.38x6`との後方互換）。
    #[serde(rename = "tone_lfo_freq")]
    pub chip_lfo_freq: u8,
    /// チップ内LFO ピッチ変調深さ。JSON上の旧名`tone_lfo_pmd`を維持する。
    #[serde(rename = "tone_lfo_pmd")]
    pub chip_lfo_pmd: u8,
    /// チップ内LFO 振幅変調深さ。JSON上の旧名`tone_lfo_amd`を維持する。
    #[serde(rename = "tone_lfo_amd")]
    pub chip_lfo_amd: u8,
    /// チップ内LFO Delay。JSON上の旧名`tone_lfo_delay`を維持する。
    #[serde(rename = "tone_lfo_delay")]
    pub chip_lfo_delay: u8,
    /// PM感度（チップ内LFOのピッチ変調感度）。
    pub pms: u8,
    /// AM感度（チップ内LFOの振幅変調感度）。
    pub ams: u8,
    /// フィルターCutoff(0〜255、spec.md「フィルター」セクション参照)。
    pub filter_cutoff: u8,
    /// フィルターResonance(0〜255)。
    pub filter_resonance: u8,
    /// フィルタータイプ(0=LP/1=HP/2=BP)。
    pub filter_type: u8,
    /// Self-Oscillation有効フラグ。
    pub filter_self_oscillation: bool,
    /// Pitch FG（新規）：ピッチ変調の一次源。バイポーラDepth(中心128)で
    /// キーオン一発のピッチ下降/上昇（シンセタム）とループ時のビブラートの両方を作れる。
    pub pitch_fg: BipolarFg,
    /// Cutoff FG（旧Filter EGの後継）：バイポーラDepth化されたカットオフ変調。
    pub cutoff_fg: BipolarFg,
    /// Gain FG（旧VCA EGの後継）：Depthなし、Floorが深さ役。
    pub gain_fg: GainFg,
    /// 質感LFO（旧チャンネルLFOを5波形に絞って再編、焼き込み専用）。
    pub texture_lfo: TextureLfo,
}

/// `Pitch FG`の既定値（ar=0/d1r=0/d1l=255/d2r=0/rr=255/depth=128/floor=0/loop=0/curve=0）。
/// spec-sound.mdのJSON例と一致する「無効（変調なし）」状態。
fn default_pitch_fg() -> BipolarFg {
    BipolarFg { eg: EgParams { ar: 0, d1r: 0, d1l: 255, d2r: 0, rr: 255, floor: 0, loop_enabled: 0, curve: 0, delay: 0 }, depth: 128 }
}

/// `Gain FG`の既定値（ar=255/d1l=255/rr=0）。アタックは数サンプルで完了しゲイン1.0に張り付く。
/// リリースは最遅(rr=0≈284.9秒＝実質ゲートを閉じない)にして、離鍵後の減衰を各オペレーターの
/// 本来のRRに委ねる（VCAが全チャンネルを速いRRで閉じるとキャリアのリリース尾を打ち消す＝瞬断の原因。
/// 発音終了時のチャンネル回収はオペレーターのidle判定のみで行うためVCAを閉じなくても安全）。
/// 旧既定はrr=255だったが、速いリリースは「透過的」ではなくFM本来のEGへの二重EG化になっていた。
fn default_gain_fg() -> GainFg {
    EgParams { ar: 255, d1r: 0, d1l: 255, d2r: 0, rr: 0, floor: 0, loop_enabled: 0, curve: 0, delay: 0 }
}

impl Default for ChannelParams {
    /// filter_cutoff=255（フィルター全開）/ filter_self_oscillation=true（spec.md準拠）以外は
    /// すべて0相当（チップ内LFO・Cutoff FGとも無効、アルゴリズム0）。FG3スロット/質感LFOの既定値は
    /// spec-sound.mdのJSON例（pitch_fg/cutoff_fg/gain_fg/texture_lfo）と厳密に一致させる。
    fn default() -> Self {
        Self {
            algorithm: 0,
            feedback: 0,
            chip_lfo_freq: 0,
            chip_lfo_pmd: 0,
            chip_lfo_amd: 0,
            chip_lfo_delay: 0,
            pms: 0,
            ams: 0,
            filter_cutoff: 255,
            filter_resonance: 0,
            filter_type: 0,
            filter_self_oscillation: true,
            pitch_fg: default_pitch_fg(),
            cutoff_fg: BipolarFg::default(),
            gain_fg: default_gain_fg(),
            texture_lfo: TextureLfo::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// ChannelParamsの後方互換Deserialize（旧filter_eg_*/vca_eg_*/perf_lfo_shapeからの移行）
// ---------------------------------------------------------------------------

/// `ChannelParams`のDeserialize専用シャドー構造体。新スキーマ（`pitch_fg`等）と
/// 旧スキーマ（`filter_eg_*`等）の両方を任意（`Option`）で受け取り、`From`実装で
/// 新スキーマを優先しつつ、無ければ旧フィールドから移行構築する。
#[derive(Deserialize)]
struct ChannelParamsWire {
    algorithm: u8,
    feedback: u8,
    #[serde(rename = "tone_lfo_freq")]
    chip_lfo_freq: u8,
    #[serde(rename = "tone_lfo_pmd")]
    chip_lfo_pmd: u8,
    #[serde(rename = "tone_lfo_amd")]
    chip_lfo_amd: u8,
    #[serde(rename = "tone_lfo_delay")]
    chip_lfo_delay: u8,
    pms: u8,
    ams: u8,
    filter_cutoff: u8,
    filter_resonance: u8,
    filter_type: u8,
    filter_self_oscillation: bool,

    // 新スキーマ（あれば最優先）
    #[serde(default)]
    pitch_fg: Option<BipolarFg>,
    #[serde(default)]
    cutoff_fg: Option<BipolarFg>,
    #[serde(default)]
    gain_fg: Option<GainFg>,
    #[serde(default)]
    texture_lfo: Option<TextureLfo>,

    // 旧スキーマ（新スキーマが無い場合のみ移行元として使う）
    #[serde(default)]
    filter_eg_ar: Option<u8>,
    #[serde(default)]
    filter_eg_d1r: Option<u8>,
    #[serde(default)]
    filter_eg_d1l: Option<u8>,
    #[serde(default)]
    filter_eg_d2r: Option<u8>,
    #[serde(default)]
    filter_eg_rr: Option<u8>,
    #[serde(default)]
    filter_eg_depth: Option<u8>,
    #[serde(default)]
    vca_eg_ar: Option<u8>,
    #[serde(default)]
    vca_eg_d1r: Option<u8>,
    #[serde(default)]
    vca_eg_d1l: Option<u8>,
    #[serde(default)]
    vca_eg_d2r: Option<u8>,
    #[serde(default)]
    vca_eg_rr: Option<u8>,
    #[serde(default)]
    perf_lfo_shape: Option<PerformanceLfoShape>,
}

impl From<ChannelParamsWire> for ChannelParams {
    fn from(wire: ChannelParamsWire) -> Self {
        let cutoff_fg = wire.cutoff_fg.unwrap_or_else(|| {
            // 旧unipolar Filter EG Depth(0〜255) → 新bipolar Depth(中心128)。
            // 「常に開く方向」だった旧挙動を128超側の半分として保つ変換式。
            let old_depth = wire.filter_eg_depth.unwrap_or(0);
            BipolarFg {
                eg: EgParams {
                    ar: wire.filter_eg_ar.unwrap_or(0),
                    d1r: wire.filter_eg_d1r.unwrap_or(0),
                    d1l: wire.filter_eg_d1l.unwrap_or(0),
                    d2r: wire.filter_eg_d2r.unwrap_or(0),
                    rr: wire.filter_eg_rr.unwrap_or(0),
                    floor: 0,
                    loop_enabled: 0,
                    curve: 0,
                    delay: 0,
                },
                depth: (128.0 + old_depth as f32 * 128.0 / 255.0).clamp(0.0, 255.0) as u8,
            }
        });
        let gain_fg = wire.gain_fg.unwrap_or_else(|| EgParams {
            ar: wire.vca_eg_ar.unwrap_or(255),
            d1r: wire.vca_eg_d1r.unwrap_or(0),
            d1l: wire.vca_eg_d1l.unwrap_or(255),
            d2r: wire.vca_eg_d2r.unwrap_or(0),
            // 未指定時は透過既定(rr=0=最遅)へ。旧既定255は離鍵で全チャンネルを8.71msで
            // 閉じ、オペレーター本来のリリース尾を打ち消していた（default_gain_fg参照）。
            rr: wire.vca_eg_rr.unwrap_or(0),
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            delay: 0,
        });
        let texture_lfo = wire.texture_lfo.unwrap_or_else(|| match wire.perf_lfo_shape {
            Some(shape) => match texture_lfo_waveform_from_engine(shape.waveform) {
                // 質感LFOの5波形パレットに該当する場合のみ波形/Fade/Offsetを移行する。
                // rate/depth/destinationは旧スキーマではランタイム専用でファイルに保存されて
                // いなかったため既定（無効）のままとする。
                Some(waveform) => TextureLfo {
                    waveform,
                    fade_mode: shape.fade_mode as u8,
                    fade_time: shape.fade_time,
                    offset: lfo_offset_to_param(shape.offset),
                    ..TextureLfo::default()
                },
                None => TextureLfo::default(),
            },
            None => TextureLfo::default(),
        });
        Self {
            algorithm: wire.algorithm,
            feedback: wire.feedback,
            chip_lfo_freq: wire.chip_lfo_freq,
            chip_lfo_pmd: wire.chip_lfo_pmd,
            chip_lfo_amd: wire.chip_lfo_amd,
            chip_lfo_delay: wire.chip_lfo_delay,
            pms: wire.pms,
            ams: wire.ams,
            filter_cutoff: wire.filter_cutoff,
            filter_resonance: wire.filter_resonance,
            filter_type: wire.filter_type,
            filter_self_oscillation: wire.filter_self_oscillation,
            pitch_fg: wire.pitch_fg.unwrap_or_else(default_pitch_fg),
            cutoff_fg,
            gain_fg,
            texture_lfo,
        }
    }
}

impl<'de> Deserialize<'de> for ChannelParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        ChannelParamsWire::deserialize(deserializer).map(Into::into)
    }
}

/// 4op分のオペレーターパラメーター + チャンネルパラメーターの一式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ym38x6Patch {
    pub operators: [OperatorParams; 4],
    pub channel: ChannelParams,
}

// ---------------------------------------------------------------------------
// パフォーマンスLFOの適用先（38x6拡張Destination）
// ---------------------------------------------------------------------------

/// パフォーマンスLFOの適用先。共通Destination（Pitch/Volume）に加え、
/// 38x6固有の拡張Destination（TLキャリア一括）を持つ。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Ym38x6LfoDestination {
    #[default]
    Pitch,
    Volume,
    TlCarrier,
    /// フィルターCutoffへの持続的な変調（オートワウ）。Filter EG Depth（キーオン一発の変調）
    /// とは独立に積み重なる（`Channel::tick`でLFOがシフトした基準Cutoffを、Filter EGがさらに変調する）。
    Cutoff,
    /// どこにも接続されていない（質感LFOパッチベイでケーブルをTEXTURE LFOパネル自身へ
    /// ドロップした状態）。LFOは`tick`し続けるが、いずれの変調ターゲットへも出力しない。
    Unplugged,
}

impl Ym38x6LfoDestination {
    /// 0〜255からの変換（質感LFOの`Destination`フィールド用、`FilterType::from_u8`と同じ慣習）。
    /// 0=Pitch/1=Volume/2=TL（キャリア一括）/3=Cutoff/4=未接続/5以上=Pitchへフォールバック。
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Volume,
            2 => Self::TlCarrier,
            3 => Self::Cutoff,
            4 => Self::Unplugged,
            _ => Self::Pitch,
        }
    }
}

// ---------------------------------------------------------------------------
// チャンネル（4オペレーター + アルゴリズム結線）
// ---------------------------------------------------------------------------

struct Channel {
    operators: [Operator; 4],
    channel_params: ChannelParams,
    /// フィードバックオペレーターの直前の出力（自己変調に使う）。
    feedback_buffer: f32,
    /// EXPERIMENT(fb-2sample): 2サンプル平均帰還用の、さらに1つ前の出力 out[n-2]。
    feedback_buffer2: f32,
    /// KSR計算用のノート番号（Note-On時の周波数から近似）。
    note: u8,
    /// OP単位キーオン/オフ（CC102〜105）でのリトリガー用に保持するNote-On時の周波数。
    base_frequency: f32,
    /// OP単位キーオン/オフでのリトリガー用に保持するNote-On時のベロシティ。
    velocity: u8,
    perf_lfo: PerformanceLfo,
    /// Pitch FG（新規）のキーオン連動エンベロープ。バイポーラDepthで`channel_params.pitch_fg`から
    /// ピッチ変調セントを作る（spec-sound.md「ファンクションジェネレーター」節）。
    pitch_fg_eg: Eg,
    /// CC76（Vibrato Rate）由来のPitch FG速さスケール（1.0=無補正、`set_pitch_fg_rate_scale`で
    /// 設定）。`pitch_fg_eg.tick`の`rate_scale`引数へそのまま渡す。
    pitch_fg_rate_scale: f32,
    pitch_mod_cents: f32,
    /// MIDIピッチベンド/RPN0によるピッチオフセット（セント、毎tickでpitch_mod_centsに加算）。
    /// パフォーマンスLFO（pitch_mod_cents）が毎tick上書きされるのと独立に保持する。
    bend_cents: f32,
    volume_mod_delta: f32,
    /// 拡張Destination=TlCarrier用：キャリア出力にかかる乗算ゲインのオフセット。
    tl_carrier_mod_delta: f32,
    /// 拡張Destination=Cutoff用：フィルターCutoffへ加算するデルタ（オートワウ、0〜255単位系）。
    cutoff_mod_delta: f32,
    /// CC7/CC11（Channel Volume × Expression）の積ゲイン（0.0〜1.0、GM2二乗カーブ適用済み）。
    /// VST/smf2wav が `set_channel_volume` で書き込み、LFO音量変調（volume_mod_delta）とは独立。
    channel_gain: f32,
    /// チップ内LFO本体（PMS/AMS×PMD/AMD、spec.md「チップ内LFO」セクション参照）。
    chip_lfo: ChipLfo,
    /// 4op合成後に適用するVCFセクション（sound-core::VoiceFilter、spec.md「フィルター」セクション参照）。
    vcf: VoiceFilter,
    /// 4op合成後に適用するVCAオーバーレイ（sound-core::VoiceAmp）。
    vca: VoiceAmp,
    /// このボイス全体がnote_off済みか（CC103〜106によるOP単位キーオフとは独立）。
    /// ボイススチール（`steal_score`）でrelease中のボイスを優先して奪うために使う。
    note_is_off: bool,
}

impl Channel {
    fn new(frequency: f32, velocity: u8, patch: Ym38x6Patch) -> Self {
        let note = frequency_to_note(frequency);
        // アルゴリズムからキャリア/モジュレーターを判定し、各OPに伝える
        // （Velocity Sensitivity=明るさをモジュレーターにのみ効かせるため）。
        let algo = &ALGORITHMS[(patch.channel.algorithm as usize).min(7)];
        let operators = std::array::from_fn(|i| {
            let mut op = Operator::new(patch.operators[i]);
            op.set_carrier(algo.carriers.contains(&i));
            op.note_on(frequency, velocity);
            op
        });
        let mut vcf = VoiceFilter::new();
        vcf.note_on();
        let mut vca = VoiceAmp::new();
        vca.note_on();
        let mut pitch_fg_eg = Eg::new();
        pitch_fg_eg.note_on();
        let mut perf_lfo = PerformanceLfo::new();
        perf_lfo.set_rate(patch.channel.texture_lfo.rate);
        perf_lfo.set_delay(patch.channel.texture_lfo.delay);
        perf_lfo.set_shape(texture_lfo_to_shape(patch.channel.texture_lfo));
        perf_lfo.note_on();
        Self {
            operators,
            channel_params: patch.channel,
            feedback_buffer: 0.0,
            feedback_buffer2: 0.0,
            note,
            base_frequency: frequency,
            velocity,
            perf_lfo,
            pitch_fg_eg,
            pitch_fg_rate_scale: 1.0,
            pitch_mod_cents: 0.0,
            bend_cents: 0.0,
            volume_mod_delta: 0.0,
            tl_carrier_mod_delta: 0.0,
            cutoff_mod_delta: 0.0,
            channel_gain: 1.0,
            chip_lfo: ChipLfo::new(),
            vcf,
            vca,
            note_is_off: false,
        }
    }

    /// 発音/リリース中のチャンネルを、残響レベルを保持したまま再キーオンする
    /// （実機 OPM の Key-On 挙動）。env_level を 0 に落とさず現在値からアタックを
    /// 再開するため、同音連打でもプチノイズが出ず、残響ドラッグが再現される。
    /// 新しいパッチ・周波数・ベロシティは適用し直す。ピッチベンド量は呼び出し側が
    /// 別途設定するため、ここでは触らない（既存値を維持）。
    fn retrigger(&mut self, frequency: f32, velocity: u8, patch: Ym38x6Patch) {
        let note = frequency_to_note(frequency);
        let algo = &ALGORITHMS[(patch.channel.algorithm as usize).min(7)];
        for (i, op) in self.operators.iter_mut().enumerate() {
            op.params = patch.operators[i];
            op.set_carrier(algo.carriers.contains(&i));
            op.retrigger(frequency, velocity);
        }
        self.channel_params = patch.channel;
        self.note = note;
        self.base_frequency = frequency;
        self.velocity = velocity;
        self.vcf.note_on();
        self.vca.note_on();
        self.pitch_fg_eg.note_on();
        self.perf_lfo.set_rate(patch.channel.texture_lfo.rate);
        self.perf_lfo.set_delay(patch.channel.texture_lfo.delay);
        self.perf_lfo.set_shape(texture_lfo_to_shape(patch.channel.texture_lfo));
        self.perf_lfo.note_on();
        self.note_is_off = false;
    }

    fn note_off(&mut self) {
        for op in self.operators.iter_mut() {
            op.note_off();
        }
        self.vcf.note_off();
        self.vca.note_off();
        self.pitch_fg_eg.note_off();
        self.perf_lfo.note_off();
        self.note_is_off = true;
    }

    /// ボイススチールの優先度スコア。値が小さいほど先に奪われる
    /// （release中（note_off済み）のボイスを優先し、その中では最も静かなキャリアを優先する。
    /// 発音中のボイスは大きなペナルティを足して奪われにくくする）。
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

    /// CC103〜106（≧64）：指定オペレーター(0〜3)をNote-On時の周波数/ベロシティでキーオンする。
    fn note_on_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_on(self.base_frequency, self.velocity);
    }

    /// CC103〜106（<64）：指定オペレーター(0〜3)をキーオフする（全OP独立。Op3も特別扱いしない）。
    fn note_off_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_off();
    }

    fn is_idle(&self) -> bool {
        self.operators.iter().all(|op| op.is_idle())
    }

    fn tick(&mut self, sample_rate: f32, wave_tables: &[Option<WaveTable>]) -> f32 {
        if self.is_idle() {
            return 0.0;
        }

        // 質感LFO（5波形専用・焼き込み、旧パフォーマンスLFO）。CC層を持たないため
        // 各destinationの実単位への変換はcc1=0固定でpitch_depth_cents/volume_depth/cutoff_depthを
        // 再利用する（それらは元々CC77(ベース)+CC1(加算)を取る関数で、cc1=0なら実質ベース値のみ）。
        let texture_lfo_value = self.perf_lfo.tick(sample_rate);
        let texture_lfo = self.channel_params.texture_lfo;
        // 行き先は排他的（同時に1個のみ）。前tickで別destinationが書いたデルタが残らないよう、
        // matchで書くのは選ばれたdestinationの1個だけにし、残りは明示的に0へ戻す
        // （Unplugged＝「4個とも0のまま」が正しく無変調になるための前提でもある）。
        self.pitch_mod_cents = 0.0;
        self.volume_mod_delta = 0.0;
        self.tl_carrier_mod_delta = 0.0;
        self.cutoff_mod_delta = 0.0;
        match Ym38x6LfoDestination::from_u8(texture_lfo.destination) {
            Ym38x6LfoDestination::Pitch => {
                let cents = pitch_depth_cents(texture_lfo.depth, 0, 0);
                apply_lfo_modulation(texture_lfo_value, LfoDestination::Pitch, cents, self);
            }
            Ym38x6LfoDestination::Volume => {
                let depth = volume_depth(texture_lfo.depth, 0);
                apply_lfo_modulation(texture_lfo_value, LfoDestination::Volume, depth, self);
            }
            Ym38x6LfoDestination::TlCarrier => {
                self.tl_carrier_mod_delta = texture_lfo_value * volume_depth(texture_lfo.depth, 0);
            }
            Ym38x6LfoDestination::Cutoff => {
                self.cutoff_mod_delta = texture_lfo_value * cutoff_depth(texture_lfo.depth, 0);
            }
            Ym38x6LfoDestination::Unplugged => {}
        }

        // Pitch FG（新規）：ループ可能EGでビブラート/シンセタムを作る一次源。
        // バイポーラDepth(中心128)で±1200セント(1オクターブ)までピッチを揺らす。
        let pitch_fg = self.channel_params.pitch_fg;
        let pitch_fg_out = self.pitch_fg_eg.tick(sample_rate, pitch_fg.eg, self.pitch_fg_rate_scale);
        let pitch_fg_cents = pitch_fg_out * (pitch_fg.depth as f32 - 128.0) / 128.0 * 1200.0;
        for op in self.operators.iter_mut() {
            op.set_pitch_modulation(self.pitch_mod_cents + self.bend_cents + pitch_fg_cents);
        }

        // チップ内LFO（プリセット・NRPNで設定する音作り用、PMS/AMS×PMD/AMD）
        let chip_lfo_value = self.chip_lfo.tick(
            sample_rate,
            self.channel_params.chip_lfo_freq,
            self.channel_params.chip_lfo_delay,
        );
        let chip_pitch_mod_cents = chip_lfo_value
            * pms_to_cents_range(self.channel_params.pms)
            * (self.channel_params.chip_lfo_pmd as f32 / 255.0);
        let chip_amp_mod = chip_lfo_value
            * ams_to_depth(self.channel_params.ams)
            * (self.channel_params.chip_lfo_amd as f32 / 255.0);
        for op in self.operators.iter_mut() {
            let am = if op.params.am_enable { chip_amp_mod } else { 0.0 };
            op.set_chip_lfo_modulation(chip_pitch_mod_cents, am);
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
                // 帰還方式の切替（既定は2サンプル平均、set_feedback_two_sample(false)で1サンプルに戻せる）。
                let fb_source = if FEEDBACK_TWO_SAMPLE.load(Ordering::Relaxed) {
                    // 実機OPM/OPN/OPZ準拠: 直近2サンプルの平均で帰還（帰還経路の1次ローパス）
                    0.5 * (self.feedback_buffer + self.feedback_buffer2)
                } else {
                    self.feedback_buffer
                };
                let scale =
                    feedback_to_scale_with_max(self.channel_params.feedback, current_feedback_scale_max());
                modulation += fb_source * scale;
            }
            let wave = wave_table_for(wave_tables, self.operators[op_idx].params.waveform);
            let out = self.operators[op_idx].tick(sample_rate, wave, modulation, self.note);
            op_outputs[op_idx] = out;
            if op_idx == algo.feedback_op {
                self.feedback_buffer2 = self.feedback_buffer;
                self.feedback_buffer = out;
            }
        }

        // ベロシティは音量のみに作用（通常のMIDI楽器と同じ常時ONの挙動）。音色は変えない。
        // キャリアごとの`velocity_gain`深さ（既定255＝フル）で効き具合を調整できる。
        // 全キャリアが既定255（＝既存`.38x6`はすべてこう）なら、旧チャンネル一括の計算経路を
        // そのまま使い、浮動小数点演算の順序まで従来と完全に同一にする（後方互換）。
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
        let tl_carrier_gain = (1.0 + self.tl_carrier_mod_delta).max(0.0);
        let volume_gain = (1.0 + self.volume_mod_delta).max(0.0);
        let dry = carrier_sum * tl_carrier_gain * volume_gain * self.channel_gain;

        // VCF（4op合成後、Cutoff FG） + VCA（TVAオーバーレイ、Gain FG）
        let cp = &self.channel_params;
        // 拡張Destination=Cutoffの質感LFO変調を基準Cutoffへ加算してからVcfへ渡す
        // （Cutoff FG Depthはvcf.process内部でこの基準値をさらにキーオン一発/ループで変調する）。
        let modulated_cutoff =
            (cp.filter_cutoff as f32 + self.cutoff_mod_delta).round().clamp(0.0, 255.0) as u8;
        let filtered = self.vcf.process(
            dry,
            sample_rate,
            modulated_cutoff,
            cp.filter_resonance,
            FilterType::from_u8(cp.filter_type),
            cp.filter_self_oscillation,
            cp.cutoff_fg.eg,
            cp.cutoff_fg.depth,
        );
        self.vca.process(filtered, sample_rate, cp.gain_fg)
    }
}

impl PerformanceLfoTarget for Channel {
    fn apply_pitch_modulation(&mut self, cents: f32) {
        self.pitch_mod_cents = cents;
    }

    fn apply_volume_modulation(&mut self, delta: f32) {
        self.volume_mod_delta = delta;
    }
}

/// 指定スロットの波形テーブルを返す。未割り当てスロット（ユーザー波形未設定）の場合は
/// 常に存在するスロット0（サイン波）にフォールバックする。
fn wave_table_for(wave_tables: &[Option<WaveTable>], slot: u8) -> &WaveTable {
    wave_tables[slot as usize]
        .as_ref()
        .unwrap_or_else(|| wave_tables[0].as_ref().unwrap())
}

// ---------------------------------------------------------------------------
// 38x6 エンジン
// ---------------------------------------------------------------------------

const TOTAL_SLOTS: usize = 256;

/// 同時発音数の既定上限。「無制限チャンネル管理」の安全弁で、通常の演奏では
/// 到達しない値に設定してある（一般的なMIDIの瞬間可聴ポリは数〜十数）。
/// 長いリリース（RR）を持つパッチ×高密度なノート列では、聴感上無音のリリース裾ボイスが
/// `is_idle()`に達するまで（数十秒）チャンネルに残り続け、上限が無いと際限なく積み上がって
/// レンダリング負荷が膨らむ。上限到達時は最も奪ってよいボイス（release中かつ最も静かなもの）
/// を1つ削除してから新規ボイスを確保する（`Channel::steal_score`参照）。
///
/// 【2026-07-28 48→64】実測（tools/smf2wav `--max-voices`）で、通常曲（LastWave_single_track.mid、
/// opz Bank Aのリリースの長い音色含む）はピーク31〜36止まりで48にすら到達せず、48/64/128の
/// どれでも速度差が無いことを確認した。一方、意図的に同時発音を384まで積み上げるストレスSMFでは
/// 上限に比例してレンダリング時間が伸びる（48=1.0倍・64=1.31倍・128=2.52倍、上限に実際到達時のみ）。
/// 通常演奏への影響がほぼ無い範囲でヘッドルームを増やすため、きりの良い64へ引き上げた。
const DEFAULT_MAX_VOICES: usize = 64;

pub struct Ym38x6Engine {
    sample_rate: f32,
    /// ボイスID→チャンネル。`BTreeMap`はID昇順の決定論的イテレーション順を保証する
    /// （旧`HashMap`はプロセスごとにランダムなシード由来の順序で全ボイスを合算しており、
    /// 浮動小数点加算の順序が実行ごとに変わる＝同一入力でも出力WAVがビット一致しなかった）。
    channels: BTreeMap<usize, Channel>,
    wave_tables: Vec<Option<WaveTable>>,
    current_patch: Ym38x6Patch,
    max_voices: usize,
    /// `render()`のモノラルミックス作業バッファ（ブロックごとに再利用、`Vec`再確保を避ける）。
    mix_buf: Vec<f32>,
}

impl Ym38x6Engine {
    pub fn new(sample_rate: f32) -> Self {
        let mut wave_tables: Vec<Option<WaveTable>> = (0..TOTAL_SLOTS).map(|_| None).collect();
        for i in 0..waveform::BUILTIN_WAVEFORM_COUNT {
            wave_tables[i as usize] = Some(gen_builtin_waveform(i));
        }
        Self {
            sample_rate,
            channels: BTreeMap::new(),
            wave_tables,
            current_patch: Ym38x6Patch::default(),
            max_voices: DEFAULT_MAX_VOICES,
            mix_buf: Vec::new(),
        }
    }

    /// 同時発音数の上限を変更する（既定`DEFAULT_MAX_VOICES`=64）。診断・テスト用途にも使う。
    pub fn set_max_voices(&mut self, max_voices: usize) {
        self.max_voices = max_voices.max(1);
    }

    /// 上限到達時に、最も奪ってよいボイス（`Channel::steal_score`が最小のもの）を1つ除去する。
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

    /// パッチをエンジンに記憶させる（gesture-app用の「保存済みパッチ」）。フロントエンドが
    /// 発音のたびにパッチ全体を送らずに済むよう、`set_patch`で事前設定→`current_patch`で取り出す。
    /// 発音中チャンネルには影響しない（Program Change相当、次のnote-onから適用される）。
    pub fn set_patch(&mut self, patch: Ym38x6Patch) {
        self.current_patch = patch;
    }

    /// `set_patch`と同様に`current_patch`を更新した上で、現在発音中の全チャンネルにも
    /// `set_channel_params`/`set_operator_params`と同じ内容を伝播する（GUI/DAWノブ変更相当、
    /// ym38x6-vstの`process()`が毎ブロック行っている伝播をgesture-appのノブ操作向けに一度で行う版）。
    /// Bank/Program切り替え（Program Change相当）で発音中の音色が変わらないようにするため、
    /// `set_patch`とは使い分ける。
    pub fn set_patch_live(&mut self, patch: Ym38x6Patch) {
        self.current_patch = patch;
        let channel_ids: Vec<usize> = self.channels.keys().copied().collect();
        for channel in channel_ids {
            self.set_channel_params(channel, patch.channel);
            for (op_index, op) in patch.operators.iter().enumerate() {
                self.set_operator_params(channel, op_index, *op);
            }
        }
    }

    /// `set_patch`で記憶させたカレントパッチを返す（`Ym38x6Patch`はCopy）。
    /// gesture-appのTauri層が`note_on`へ渡すパッチとして使う。
    pub fn current_patch(&self) -> Ym38x6Patch {
        self.current_patch
    }

    /// 発音中チャンネルのチャンネルパラメーターを更新する（DAWオートメーション/NRPN用）。
    /// `texture_lfo`（rate/delay/波形/Fade/Offset/destination/depth）は他フィールドと同様に
    /// リアルタイムで`perf_lfo`へ伝播する（旧仕様のrate/delay/destination/depthランタイム専用
    /// API=`set_performance_lfo`は廃止、質感LFOは完全にパッチ所有になったため）。
    pub fn set_channel_params(&mut self, channel: usize, params: ChannelParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.perf_lfo.set_rate(params.texture_lfo.rate);
            ch.perf_lfo.set_delay(params.texture_lfo.delay);
            ch.perf_lfo.set_shape(texture_lfo_to_shape(params.texture_lfo));
            ch.channel_params = params;
        }
    }

    /// 発音中チャンネルの指定オペレーターのパラメーターを更新する。
    pub fn set_operator_params(&mut self, channel: usize, op_index: usize, params: OperatorParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].params = params;
        }
    }

    /// 発音中チャンネルの指定オペレーターのF-Numberを上書きする（NRPN Operator F-Number、0〜8191）。
    pub fn set_operator_f_number(&mut self, channel: usize, op_index: usize, f_number: u16) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].set_f_number_override(f_number);
        }
    }

    /// 発音中チャンネルのPitch FG速さスケール（CC76 Vibrato Rate、1.0=無補正）を設定する。
    /// `Vco`トレイトではなく`set_operator_f_number`と同じ38x6固有拡張として持つ
    /// （Pitch FGはym38x6-core固有の概念で、発振原理非依存の`Vco`契約には含めない）。
    /// `set_pitch_bend`/`set_channel_volume`と同様、note_on直後に単一ボイスへ即時反映する
    /// 用途にも使う。
    pub fn set_pitch_fg_rate_scale(&mut self, channel: usize, scale: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.pitch_fg_rate_scale = scale;
        }
    }

    /// CC103〜106（≧64）：指定チャンネルの指定オペレーター(0〜3)をキーオンする。
    pub fn note_on_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_on_operator(op_index);
        }
    }

    /// CC103〜106（<64）：指定チャンネルの指定オペレーター(0〜3)をキーオフする（全OP独立）。
    pub fn note_off_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off_operator(op_index);
        }
    }

    /// スロット8〜255にユーザー定義波形をロードする。
    /// 注意: 波形番号32〜63はノイズ生成器の予約レンジ。そこへロードしても
    /// オシレーターはノイズ分岐を優先するため、ここで設定したテーブルは無視される
    /// （実用的なユーザー定義波形スロットは64〜255）。
    pub fn set_user_wave(&mut self, slot: u8, input: &[i8; 32]) {
        assert!(slot >= 8, "slots 0-7 are reserved for builtin waves");
        self.wave_tables[slot as usize] = Some(convert_wave_32(input));
    }

    /// 指定グループ（`channel >> 7`が一致する全ボイス）を即座に除去する
    /// （All Sound Off / CC120）。note_offのReleaseを経ず、残響も無く無音になる
    /// （GM2 All Sound Off準拠。Releaseして自然減衰させるCC123 All Notes Offとは区別する）。
    pub fn silence_group(&mut self, group: usize) {
        self.channels.retain(|id, _| id >> 7 != group);
    }

    /// 現在生存している（idleでない）ボイス数。負荷診断用（ボイススチール上限との比較等）。
    pub fn active_voice_count(&self) -> usize {
        self.channels.len()
    }
}

/// 発振源（VCO）としての演奏ライフサイクル層（spec.md「VCO抽象とモジュレーション層」）。
/// 発振原理に依存しない発音/停止/レンダリング/ピッチ・音量制御のみを提供する。
/// 音色は`set_patch`で設定した`current_patch`を使う（トレイトはパッチ型を持たない）。
impl Vco for Ym38x6Engine {
    /// 指定チャンネルIDへ`set_patch`で設定済みのカレントパッチでNote-Onする。
    /// 既に同じIDで発音中/リリース中のチャンネルがあれば、env_levelを保持したまま
    /// 残響から再アタックする（実機OPMのKey-On挙動。プチノイズが出ず、モジュレーターの
    /// 残響ドラッグによるFMらしい明るさも再現される）。
    /// ボイスごとに音色を変える呼び出し側（VST等）は、各Note-Onの直前に`set_patch`する。
    fn note_on(&mut self, channel: usize, frequency: f32, velocity: u8) {
        let patch = self.current_patch;
        if let Some(ch) = self.channels.get_mut(&channel) {
            if !ch.is_idle() {
                ch.retrigger(frequency, velocity, patch);
                return;
            }
        } else if self.channels.len() >= self.max_voices {
            // 既存スロットの上書き（idleなチャンネルへのnote-on）はボイス数を増やさないため
            // スチール対象外。ここに来るのは新規スロットが上限に到達している場合のみ。
            self.steal_one_voice();
        }
        self.channels.insert(channel, Channel::new(frequency, velocity, patch));
    }

    fn note_off(&mut self, channel: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off();
        }
    }

    /// 発音中チャンネルのピッチベンド量（セント）を設定する（MIDI Pitch Bend / RPN0）。
    fn set_pitch_bend(&mut self, channel: usize, cents: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.bend_cents = cents;
        }
    }

    /// 指定グループ（`channel >> 7`が一致する全チャンネル）のピッチベンド量を一括設定する。
    /// VST側のID符号化 `midi_ch*128 + note` において、MIDIチャンネル単位でベンドを
    /// かけるために使う（和音の全ノートが一緒に滑らかに上下する）。
    fn set_pitch_bend_group(&mut self, group: usize, cents: f32) {
        for (id, ch) in self.channels.iter_mut() {
            if id >> 7 == group {
                ch.bend_cents = cents;
            }
        }
    }

    /// 指定ボイスのチャンネル音量ゲインを設定する（CC7/CC11 のGM2積ゲイン、0.0〜1.0）。
    /// note-on 直後に呼んで、新ボイスへ現在のCC7/CC11を即時反映させる用途にも使う。
    fn set_channel_volume(&mut self, channel: usize, gain: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.channel_gain = gain.max(0.0);
        }
    }

    /// 指定グループ（`channel >> 7`が一致する全チャンネル）の音量ゲインを一括設定する。
    /// `set_pitch_bend_group` と同じIDパターン（`midi_ch*128+note`）でMIDIチャンネル単位に作用する。
    fn set_channel_volume_group(&mut self, group: usize, gain: f32) {
        for (id, ch) in self.channels.iter_mut() {
            if id >> 7 == group {
                ch.channel_gain = gain.max(0.0);
            }
        }
    }

    /// ブロック単位のレンダリング。ループはチャンネル外側×サンプル内側
    /// （旧実装のサンプル外側×チャンネル内側から入れ替え）。1ボイスの状態が内側ループの間
    /// キャッシュに乗り続けるため、リリース裾で数十ボイスが積み上がった時の実効速度が大きく上がる。
    /// 各サンプルへの加算はチャンネルのイテレーション順（ID昇順）で旧実装と同一のため、
    /// 出力は従来とビット単位で一致する（ミックスバッファ経由でも `0.0 + a + b + …` の順序は不変）。
    fn render(&mut self, output: &mut [f32], num_channels: usize) {
        let num_channels = num_channels.max(1);
        let sample_rate = self.sample_rate;
        let wave_tables = &self.wave_tables;
        let frames = output.len().div_ceil(num_channels);
        self.mix_buf.clear();
        self.mix_buf.resize(frames, 0.0);
        for ch in self.channels.values_mut() {
            for mix in self.mix_buf.iter_mut() {
                // ブロック途中でリリースが完了したら（is_idle）以降のサンプルは無音のため打ち切る
                // （旧実装のサンプルごとのis_idle()スキップと等価）。
                if ch.is_idle() {
                    break;
                }
                *mix += ch.tick(sample_rate, wave_tables);
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

    /// 全Opがアルゴリズム7（全並列）で即音量最大・サスティン無限のテスト用パッチ。
    fn loud_patch(velocity_sensitivity: u8) -> Ym38x6Patch {
        let op_params = OperatorParams {
            tl: 255,
            ar: 255,
            d1r: 0,
            d2r: 0,
            d1l: 255,
            rr: 255,
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity,
            waveform: 0,
            op_fine_tune: 128,
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        };
        let mut patch = Ym38x6Patch::default();
        patch.operators = [op_params; 4];
        patch.channel.algorithm = 7;
        patch
    }

    #[test]
    fn voice_steal_caps_channel_count() {
        let mut engine = Ym38x6Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(4);
        for ch in 0..8 {
            engine.note_on(ch, 440.0, 100);
        }
        assert_eq!(engine.channels.len(), 4, "上限を超えて積み上がってはいけない");
    }

    #[test]
    fn voice_steal_prefers_released_over_held() {
        let mut engine = Ym38x6Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(2);
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_off(0); // ch0だけrelease中にする

        engine.note_on(2, 440.0, 100); // 上限到達→ch0(release中)が奪われるはず

        assert!(!engine.channels.contains_key(&0), "release中のボイスが優先的に奪われるはず");
        assert!(engine.channels.contains_key(&1), "発音中のボイスは残るはず");
        assert!(engine.channels.contains_key(&2), "新規ボイスは確保されるはず");
        assert_eq!(engine.channels.len(), 2);
    }

    #[test]
    fn retrigger_does_not_trigger_steal() {
        let mut engine = Ym38x6Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(2);
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_on(0, 880.0, 100); // 同一IDへの再note-on＝retrigger（新規スロットではない）

        assert!(engine.channels.contains_key(&0));
        assert!(engine.channels.contains_key(&1), "retriggerが他ボイスを巻き込んで奪ってはいけない");
        assert_eq!(engine.channels.len(), 2);
    }

    /// ブロック一括レンダリングと1サンプルずつのレンダリングがビット単位で一致することを確認する
    /// （render()のチャンネル外側×サンプル内側へのループ入れ替えが、旧来のサンプル外側処理と
    /// 数値的に完全等価であることの回帰テスト）。フィードバック・フィルター・リリース途中の
    /// idle化まで通る条件で、複数ボイス＋途中note_offを含めて比較する。
    #[test]
    fn block_render_matches_sample_by_sample_render() {
        let mut patch = loud_patch(0);
        patch.channel.feedback = 100; // フィードバック経路（feedback_buffer2含む）を通す
        patch.channel.filter_cutoff = 200;
        patch.channel.filter_resonance = 80;
        for op in patch.operators.iter_mut() {
            op.d1r = 120;
            op.d1l = 180;
            op.rr = 200; // 後半ブロック内でリリースが完了しidle化する速さ
        }

        let mut block = Ym38x6Engine::new(44100.0);
        let mut single = Ym38x6Engine::new(44100.0);
        for engine in [&mut block, &mut single] {
            engine.set_patch(patch);
            engine.note_on(0, 220.0, 100);
            engine.note_on(1, 330.0, 90);
            engine.note_on(2, 440.0, 127);
        }

        const HALF: usize = 2048;
        let mut out_block = vec![0.0f32; HALF * 2];
        let mut out_single = vec![0.0f32; HALF * 2];

        // 前半: 3ボイス発音中
        block.render(&mut out_block[..HALF], 1);
        for i in 0..HALF {
            single.render(&mut out_single[i..i + 1], 1);
        }
        // 同一サンプル位置でnote_offし、後半でリリース〜idle化まで比較する
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
        let mut engine = Ym38x6Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.note_on(0, 440.0, 100);
        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        assert!(buf.iter().any(|&s| s != 0.0), "expected non-silent output");
    }

    #[test]
    fn note_off_operator_silences_single_op_and_note_on_operator_retriggers() {
        let mut engine = Ym38x6Engine::new(44100.0);
        let ch = 0;
        engine.set_patch(loud_patch(0));
        engine.note_on(ch, 440.0, 127);
        engine.note_off_operator(ch, 0);
        // チャンネル全体は他のOpが鳴り続けるためidleにならない
        assert!(!engine.channels[&ch].is_idle());

        // rr=255（最速リリース）でOp0が即座にidleになる
        let mut buf = vec![0.0f32; 100];
        engine.render(&mut buf, 1);
        assert!(engine.channels[&ch].operators[0].is_idle());

        // Op0を再キーオン → idleではなくなる
        engine.note_on_operator(ch, 0);
        assert!(!engine.channels[&ch].operators[0].is_idle());
    }

    #[test]
    fn note_off_operator_on_op3_is_independent_like_other_ops() {
        let mut engine = Ym38x6Engine::new(44100.0);
        let ch = 0;
        engine.set_patch(loud_patch(0));
        engine.note_on(ch, 440.0, 127);
        engine.note_off_operator(ch, 3);

        // Op3は他Opと同じ独立扱い：チャンネルは消えず、他Opは鳴り続ける（Op3マスター廃止）
        assert!(engine.channels.contains_key(&ch), "Op3 key-off must not remove the channel");
        assert!(!engine.channels[&ch].is_idle());

        // rr=255（最速リリース）でOp3のみ即idle、他Opは生存
        let mut buf = vec![0.0f32; 100];
        engine.render(&mut buf, 1);
        assert!(engine.channels[&ch].operators[3].is_idle());
        assert!(!engine.channels[&ch].operators[0].is_idle());
    }

    #[test]
    fn set_operator_f_number_overrides_single_operator_frequency() {
        let mut engine = Ym38x6Engine::new(44100.0);
        let ch = 0;
        engine.set_patch(loud_patch(0));
        engine.note_on(ch, 440.0, 127);
        engine.set_operator_f_number(ch, 0, crate::mapping::F_NUMBER_CENTER / 2);

        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        assert!(buf.iter().all(|&s| s.is_finite()), "expected finite output");
        assert!(buf.iter().any(|&s| s != 0.0), "expected non-silent output");
    }

    #[test]
    fn velocity_changes_output_volume_regardless_of_sensitivity() {
        // Velocity Sensitivity=0（明るさ無効）でも、ベロシティは音量に常時作用する。
        let mut patch = loud_patch(0);
        for op in patch.operators.iter_mut() {
            op.tl = 200;
        }

        let mut engine_lo = Ym38x6Engine::new(44100.0);
        let mut engine_hi = Ym38x6Engine::new(44100.0);
        engine_lo.set_patch(patch);
        engine_lo.note_on(0, 440.0, 40);
        engine_hi.set_patch(patch);
        engine_hi.note_on(0, 440.0, 120);
        let mut buf_lo = vec![0.0f32; 100];
        let mut buf_hi = vec![0.0f32; 100];
        engine_lo.render(&mut buf_lo, 1);
        engine_hi.render(&mut buf_hi, 1);

        let peak_lo = buf_lo.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak_hi = buf_hi.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(peak_hi > peak_lo, "higher velocity should be louder: {peak_hi} vs {peak_lo}");
    }

    #[test]
    fn velocity_sensitivity_ignored_on_carriers() {
        // 全並列(algo7)＝全OPキャリア。Velocity Sensitivityは明るさ専用なので、
        // 同一ベロシティならvel_sensの値に関わらずキャリアの音量は変わらない。
        let mut patch_zero = loud_patch(0);
        let mut patch_max = loud_patch(255);
        for op in patch_zero.operators.iter_mut() {
            op.tl = 200;
        }
        for op in patch_max.operators.iter_mut() {
            op.tl = 200;
        }

        let mut engine_zero = Ym38x6Engine::new(44100.0);
        let mut engine_max = Ym38x6Engine::new(44100.0);
        engine_zero.set_patch(patch_zero);
        engine_zero.note_on(0, 440.0, 100);
        engine_max.set_patch(patch_max);
        engine_max.note_on(0, 440.0, 100);
        let mut buf_zero = vec![0.0f32; 100];
        let mut buf_max = vec![0.0f32; 100];
        engine_zero.render(&mut buf_zero, 1);
        engine_max.render(&mut buf_max, 1);

        let peak_zero = buf_zero.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak_max = buf_max.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            (peak_zero - peak_max).abs() < 1e-6,
            "velocity sensitivity must not change carrier volume: {peak_zero} vs {peak_max}"
        );
    }

    #[test]
    fn carrier_velocity_gain_default_matches_legacy_channel_wide_gain() {
        // 全キャリアが既定255のとき、velocity_changes_output_volume_regardless_of_sensitivity
        // と同じ挙動（＝旧チャンネル一括velocity_gain）になることを確認する。
        let mut patch = loud_patch(0);
        for op in patch.operators.iter_mut() {
            op.tl = 200;
            assert_eq!(op.velocity_gain, 255, "既定は255のはず");
        }

        let mut engine_lo = Ym38x6Engine::new(44100.0);
        let mut engine_hi = Ym38x6Engine::new(44100.0);
        engine_lo.set_patch(patch);
        engine_lo.note_on(0, 440.0, 40);
        engine_hi.set_patch(patch);
        engine_hi.note_on(0, 440.0, 120);
        let mut buf_lo = vec![0.0f32; 100];
        let mut buf_hi = vec![0.0f32; 100];
        engine_lo.render(&mut buf_lo, 1);
        engine_hi.render(&mut buf_hi, 1);

        let peak_lo = buf_lo.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak_hi = buf_hi.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(peak_hi > peak_lo, "higher velocity should be louder: {peak_hi} vs {peak_lo}");
    }

    #[test]
    fn carrier_velocity_gain_zero_depth_keeps_volume_constant() {
        // 全キャリアのvelocity_gain=0なら、ベロシティに関わらず音量が一定になる
        // （オルガン的運用）。
        let mut patch = loud_patch(0);
        for op in patch.operators.iter_mut() {
            op.tl = 200;
            op.velocity_gain = 0;
        }

        let mut engine_lo = Ym38x6Engine::new(44100.0);
        let mut engine_hi = Ym38x6Engine::new(44100.0);
        engine_lo.set_patch(patch);
        engine_lo.note_on(0, 440.0, 1);
        engine_hi.set_patch(patch);
        engine_hi.note_on(0, 440.0, 127);
        let mut buf_lo = vec![0.0f32; 100];
        let mut buf_hi = vec![0.0f32; 100];
        engine_lo.render(&mut buf_lo, 1);
        engine_hi.render(&mut buf_hi, 1);

        let peak_lo = buf_lo.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak_hi = buf_hi.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            (peak_lo - peak_hi).abs() < 1e-6,
            "velocity_gain=0 should make volume velocity-independent: {peak_lo} vs {peak_hi}"
        );
    }

    #[test]
    fn velocity_sensitivity_affects_modulator_brightness() {
        // algo0: O1->O2->O3->O4。O1はモジュレーター。
        // モジュレーターのVelocity Sensitivityは変調量（明るさ）を変えるので、
        // 同一ベロシティでもvel_sensの有無で出力波形が変わる。
        let base = OperatorParams {
            tl: 100,
            ar: 255,
            d1r: 0,
            d2r: 0,
            d1l: 255,
            rr: 255,
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        };
        let make = |op0_sens: u8| {
            let mut patch = Ym38x6Patch::default();
            patch.operators = [base; 4];
            patch.operators[0].velocity_sensitivity = op0_sens;
            patch.channel.algorithm = 0;
            patch.channel.feedback = 0;
            patch
        };

        let mut engine_flat = Ym38x6Engine::new(44100.0);
        let mut engine_bright = Ym38x6Engine::new(44100.0);
        engine_flat.set_patch(make(0));
        engine_flat.note_on(0, 440.0, 100);
        engine_bright.set_patch(make(255));
        engine_bright.note_on(0, 440.0, 100);
        let mut buf_flat = vec![0.0f32; 512];
        let mut buf_bright = vec![0.0f32; 512];
        engine_flat.render(&mut buf_flat, 1);
        engine_bright.render(&mut buf_bright, 1);

        let differs = buf_flat
            .iter()
            .zip(buf_bright.iter())
            .any(|(a, b)| (a - b).abs() > 1e-6);
        assert!(differs, "modulator velocity sensitivity should change the timbre");
    }

    #[test]
    fn all_algorithms_long_run_no_nan() {
        let op_params = OperatorParams {
            tl: 200,
            ar: 255,
            d1r: 100,
            d2r: 80,
            d1l: 180,
            rr: 150,
            mul: 1,
            dt1: 128,
            ksr: 64,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        };

        for algorithm in 0u8..8 {
            let mut patch = Ym38x6Patch::default();
            patch.operators = [op_params; 4];
            patch.channel.algorithm = algorithm;
            patch.channel.feedback = 128;

            let mut engine = Ym38x6Engine::new(44100.0);
            let ch = 0;
            engine.set_patch(patch);
            engine.note_on(ch, 440.0, 100);
            let mut buf = vec![0.0f32; 44100];
            engine.render(&mut buf, 1);
            engine.note_off(ch);

            let mut buf2 = vec![0.0f32; 44100 * 2];
            engine.render(&mut buf2, 1);

            for &s in buf.iter().chain(buf2.iter()) {
                assert!(s.is_finite(), "algorithm {algorithm}: non-finite sample {s}");
            }
        }
    }

    #[test]
    fn filter_self_oscillation_long_run_no_nan() {
        let op_params = OperatorParams {
            tl: 200,
            ar: 255,
            d1r: 100,
            d2r: 80,
            d1l: 180,
            rr: 150,
            mul: 1,
            dt1: 128,
            ksr: 64,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        };

        for filter_type in 0u8..3 {
            let mut patch = Ym38x6Patch::default();
            patch.operators = [op_params; 4];
            patch.channel.algorithm = 7;
            patch.channel.filter_type = filter_type;
            patch.channel.filter_cutoff = 64;
            patch.channel.filter_resonance = 255;
            patch.channel.filter_self_oscillation = true;
            patch.channel.cutoff_fg = BipolarFg {
                eg: EgParams { ar: 200, d1r: 150, d1l: 128, d2r: 0, rr: 150, floor: 0, loop_enabled: 0, curve: 0, delay: 0 },
                depth: 255,
            };

            let mut engine = Ym38x6Engine::new(44100.0);
            let ch = 0;
            engine.set_patch(patch);
            engine.note_on(ch, 440.0, 100);
            let mut buf = vec![0.0f32; 44100];
            engine.render(&mut buf, 1);
            engine.note_off(ch);

            let mut buf2 = vec![0.0f32; 44100 * 2];
            engine.render(&mut buf2, 1);

            for &s in buf.iter().chain(buf2.iter()) {
                assert!(s.is_finite(), "filter_type {filter_type}: non-finite sample {s}");
            }
        }
    }

    #[test]
    fn chip_lfo_modulates_output_amplitude_periodically() {
        let op_params = OperatorParams {
            tl: 255,
            ar: 255,
            d1r: 0,
            d2r: 0,
            d1l: 255,
            rr: 255,
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: true,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        };
        let mut patch = Ym38x6Patch::default();
        patch.operators = [op_params; 4];
        patch.channel.algorithm = 7; // 全並列
        patch.channel.chip_lfo_freq = 200; // 速めのLFO（テストを短時間で完結させる）
        patch.channel.chip_lfo_pmd = 255;
        patch.channel.chip_lfo_amd = 255;
        patch.channel.pms = 255;
        patch.channel.ams = 255;

        let mut engine = Ym38x6Engine::new(44100.0);
        engine.set_patch(patch);
        engine.note_on(0, 440.0, 127);
        let mut buf = vec![0.0f32; 4410]; // 0.1秒（音色LFO数周期分）
        engine.render(&mut buf, 1);

        // ウィンドウごとの最大振幅を比較し、音色LFOのAMにより振幅が周期的に変化することを確認
        let window = 200;
        let peaks: Vec<f32> = buf
            .chunks(window)
            .map(|chunk| chunk.iter().fold(0.0f32, |a, &b| a.max(b.abs())))
            .collect();

        let max_peak = peaks.iter().cloned().fold(0.0f32, f32::max);
        let min_peak = peaks.iter().cloned().fold(f32::MAX, f32::min);

        assert!(max_peak > 0.5, "expected a loud window: max_peak={max_peak}");
        assert!(min_peak < max_peak * 0.6, "expected amplitude to vary with LFO: min={min_peak} max={max_peak}");
    }

    #[test]
    fn channel_gain_scales_output() {
        // gain=0.5 で出力が半分、gain=0.0 で無音になることを確認。
        let make = || {
            let mut e = Ym38x6Engine::new(44100.0);
            e.set_patch(loud_patch(0));
            e.note_on(0, 440.0, 127);
            e
        };

        // gain=1.0（既定）
        let mut e1 = make();
        let mut buf1 = vec![0.0f32; 256];
        e1.render(&mut buf1, 1);
        let peak1 = buf1.iter().fold(0.0f32, |a, &b| a.max(b.abs()));

        // gain=0.5
        let mut e5 = make();
        e5.set_channel_volume(0, 0.5);
        let mut buf5 = vec![0.0f32; 256];
        e5.render(&mut buf5, 1);
        let peak5 = buf5.iter().fold(0.0f32, |a, &b| a.max(b.abs()));

        // gain=0.0（無音）
        let mut e0 = make();
        e0.set_channel_volume(0, 0.0);
        let mut buf0 = vec![0.0f32; 256];
        e0.render(&mut buf0, 1);
        let peak0 = buf0.iter().fold(0.0f32, |a, &b| a.max(b.abs()));

        assert!(peak1 > 0.0, "gain=1.0 should produce sound");
        assert!((peak5 / peak1 - 0.5).abs() < 0.01, "gain=0.5 should halve output: {peak5}/{peak1}");
        assert_eq!(peak0, 0.0, "gain=0.0 should be silent");
    }

    #[test]
    fn channel_volume_group_applies_to_midi_channel() {
        // set_channel_volume_group はグループ（id>>7）が一致する全ボイスにのみ適用される。
        let mut engine = Ym38x6Engine::new(44100.0);
        // ch0 (MIDI ch0, note0): id=0
        // ch1 (MIDI ch1, note0): id=128
        engine.set_patch(loud_patch(0));
        engine.note_on(0, 440.0, 127);
        engine.set_patch(loud_patch(0));
        engine.note_on(128, 440.0, 127);
        // MIDI ch0 を無音化
        engine.set_channel_volume_group(0, 0.0);

        let mut buf = vec![0.0f32; 256];
        engine.render(&mut buf, 1);
        let peak = buf.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        // MIDI ch1（id=128）は gain=1.0 のまま → 出力が残る
        assert!(peak > 0.0, "MIDI ch1 should still produce sound after silencing ch0");
    }

    /// 旧`filter_eg_*`/`vca_eg_*`/`perf_lfo_shape`を持つ旧`.38x6`相当のJSONが、
    /// 新スキーマ（`cutoff_fg`/`gain_fg`/`texture_lfo`）へ正しく移行読み込みされることを確認する
    /// （後方互換マイグレーション、spec-sound.md「実装状況」節の「後方互換規則」参照）。
    #[test]
    fn channel_params_deserializes_old_schema_with_migration() {
        let old_json = r#"{
            "algorithm": 4,
            "feedback": 100,
            "tone_lfo_freq": 10,
            "tone_lfo_pmd": 20,
            "tone_lfo_amd": 30,
            "tone_lfo_delay": 0,
            "pms": 5,
            "ams": 6,
            "filter_cutoff": 200,
            "filter_resonance": 50,
            "filter_type": 0,
            "filter_self_oscillation": false,
            "filter_eg_ar": 111,
            "filter_eg_d1r": 122,
            "filter_eg_d1l": 133,
            "filter_eg_d2r": 144,
            "filter_eg_rr": 155,
            "filter_eg_depth": 255,
            "vca_eg_ar": 200,
            "vca_eg_d1r": 10,
            "vca_eg_d1l": 220,
            "vca_eg_d2r": 5,
            "vca_eg_rr": 210,
            "perf_lfo_shape": { "waveform": "Square", "fade_mode": "OnIn", "fade_time": 40, "offset": 0 }
        }"#;

        let params: ChannelParams = serde_json::from_str(old_json).expect("should deserialize old schema");

        assert_eq!(params.algorithm, 4);
        assert_eq!(params.feedback, 100);
        assert_eq!(params.chip_lfo_freq, 10);

        // Cutoff FG: filter_eg_*から直接コピー、depthはunipolar(255)→bipolar(128+255*128/255=256→clamp255)。
        assert_eq!(params.cutoff_fg.eg.ar, 111);
        assert_eq!(params.cutoff_fg.eg.d1r, 122);
        assert_eq!(params.cutoff_fg.eg.d1l, 133);
        assert_eq!(params.cutoff_fg.eg.d2r, 144);
        assert_eq!(params.cutoff_fg.eg.rr, 155);
        assert_eq!(params.cutoff_fg.eg.loop_enabled, 0);
        assert_eq!(params.cutoff_fg.eg.floor, 0);
        assert_eq!(params.cutoff_fg.depth, 255);

        // Gain FG: vca_eg_*から直接コピー（depthフィールドなし）。
        assert_eq!(params.gain_fg.ar, 200);
        assert_eq!(params.gain_fg.d1r, 10);
        assert_eq!(params.gain_fg.d1l, 220);
        assert_eq!(params.gain_fg.d2r, 5);
        assert_eq!(params.gain_fg.rr, 210);

        // Pitch FG: 旧スキーマに対応データが無いため常にデフォルト。
        assert_eq!(params.pitch_fg, default_pitch_fg());

        // 質感LFO: perf_lfo_shape.waveform=Squareは新5波形パレットに該当するので移行される。
        // destination/rate/depthは旧スキーマに保存されていなかったため既定(0)のまま。
        assert_eq!(params.texture_lfo.waveform, 0);
        assert_eq!(params.texture_lfo.fade_mode, 0);
        assert_eq!(params.texture_lfo.fade_time, 40);
        assert_eq!(params.texture_lfo.offset, 128);
        assert_eq!(params.texture_lfo.destination, 0);
        assert_eq!(params.texture_lfo.rate, 0);
        assert_eq!(params.texture_lfo.depth, 0);
    }

    /// 旧`perf_lfo_shape.waveform`が新質感LFOのパレット外（Triangle/Sine/Saw）だった場合は、
    /// 移行先を`.38x6`ファイル単体からは判定できないため、質感LFOのデフォルト（無効）へ
    /// フォールバックする（既知の制約、旧スキーマではdepth/rateも保存されておらず実害は小さい）。
    #[test]
    fn channel_params_old_schema_out_of_palette_waveform_falls_back_to_default_texture_lfo() {
        let old_json = r#"{
            "algorithm": 0,
            "feedback": 0,
            "tone_lfo_freq": 0,
            "tone_lfo_pmd": 0,
            "tone_lfo_amd": 0,
            "tone_lfo_delay": 0,
            "pms": 0,
            "ams": 0,
            "filter_cutoff": 255,
            "filter_resonance": 0,
            "filter_type": 0,
            "filter_self_oscillation": true,
            "filter_eg_ar": 0,
            "filter_eg_d1r": 0,
            "filter_eg_d1l": 0,
            "filter_eg_d2r": 0,
            "filter_eg_rr": 0,
            "filter_eg_depth": 0,
            "vca_eg_ar": 255,
            "vca_eg_d1r": 0,
            "vca_eg_d1l": 255,
            "vca_eg_d2r": 0,
            "vca_eg_rr": 255,
            "perf_lfo_shape": { "waveform": "Sine", "fade_mode": "OnIn", "fade_time": 0, "offset": 0 }
        }"#;

        let params: ChannelParams = serde_json::from_str(old_json).expect("should deserialize old schema");
        assert_eq!(params.texture_lfo, TextureLfo::default());
    }

    /// 新スキーマ（`cutoff_fg`/`gain_fg`/`texture_lfo`を含むJSON）はそのまま読み込め、
    /// シリアライズ→デシリアライズの往復でも値が保たれる。
    #[test]
    fn channel_params_new_schema_round_trips() {
        let mut params = ChannelParams::default();
        params.pitch_fg.depth = 200;
        params.cutoff_fg.eg.loop_enabled = 1;
        params.cutoff_fg.eg.floor = 64;
        params.gain_fg.curve = 1;
        params.texture_lfo.waveform = 4;
        params.texture_lfo.depth = 128;

        let json = serde_json::to_string(&params).expect("serialize");
        let round_tripped: ChannelParams = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(params, round_tripped);
    }
}
