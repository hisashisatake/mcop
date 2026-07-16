use serde::{Deserialize, Serialize};
use ym38x6_core::{
    BipolarFg, ChannelParams, EgParams, LfoWaveform, OperatorParams, Preset, TextureLfo, Ym38x6Patch,
};

/// フロントエンドの質感LFO波形インデックス(旧8波形、`lfo_waveform_from_index`と同じ規約)を、
/// `TextureLfo`の新5波形パレット(0〜4)へ写像する（パレット外のTriangle/Sine/Sawは矩形へ
/// フォールバック。ym38x6-vstの`lfo_waveform_to_texture_lfo_index`と同じ考え方）。
fn perf_lfo_waveform_to_texture_lfo_index(waveform: u8) -> u8 {
    match ym38x6_core::lfo_waveform_from_index(waveform) {
        LfoWaveform::Square => 0,
        LfoWaveform::Trapezoid => 1,
        LfoWaveform::SampleHold => 2,
        LfoWaveform::Random => 3,
        LfoWaveform::Chaos => 4,
        LfoWaveform::Triangle | LfoWaveform::Sine | LfoWaveform::Saw => 0,
    }
}

/// [perf_lfo_waveform_to_texture_lfo_index]の逆方向。DTOへ書き戻す際に使う。
fn texture_lfo_index_to_perf_lfo_waveform(index: u8) -> u8 {
    let waveform = match index {
        1 => LfoWaveform::Trapezoid,
        2 => LfoWaveform::SampleHold,
        3 => LfoWaveform::Random,
        4 => LfoWaveform::Chaos,
        _ => LfoWaveform::Square,
    };
    waveform as u8
}

/// フロントエンドから渡される/返すオペレーター単位パラメーター（`OperatorParams`のDTO）。
#[derive(Deserialize, Serialize)]
pub struct OperatorParamsDto {
    pub tl: u8,
    pub ar: u8,
    pub d1r: u8,
    pub d2r: u8,
    pub d1l: u8,
    pub rr: u8,
    pub mul: u8,
    pub dt1: u8,
    pub ksr: u8,
    pub am_enable: bool,
    pub velocity_sensitivity: u8,
    pub waveform: u8,
    /// OP単位の追加チューニング（0〜255、中心128＝±0、±1オクターブ）。
    /// 未送信のフロントエンドでも中心128（オフセットなし）として扱う。
    #[serde(default = "default_op_fine_tune")]
    pub op_fine_tune: u8,
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。
    #[serde(default)]
    pub floor: u8,
    /// 0=ワンショット/1=ループ。未送信のフロントエンドでは0（従来のADSR挙動）として扱う。
    #[serde(default)]
    pub loop_enabled: u8,
    /// 0=線形/1=サイン風。
    #[serde(default)]
    pub curve: u8,
    /// EGSFT（TX81Z EG Shift）。EGの減衰レンジ(dB)を圧縮する（0〜255、既定0＝96dBフルレンジ）。
    #[serde(default)]
    pub eg_shift: u8,
}

fn default_op_fine_tune() -> u8 {
    128
}

impl From<OperatorParamsDto> for OperatorParams {
    fn from(dto: OperatorParamsDto) -> Self {
        Self {
            tl: dto.tl,
            ar: dto.ar,
            d1r: dto.d1r,
            d2r: dto.d2r,
            d1l: dto.d1l,
            rr: dto.rr,
            mul: dto.mul,
            dt1: dto.dt1,
            ksr: dto.ksr,
            am_enable: dto.am_enable,
            velocity_sensitivity: dto.velocity_sensitivity,
            waveform: dto.waveform,
            op_fine_tune: dto.op_fine_tune,
            floor: dto.floor,
            loop_enabled: dto.loop_enabled,
            curve: dto.curve,
            eg_shift: dto.eg_shift,
        }
    }
}

impl From<OperatorParams> for OperatorParamsDto {
    fn from(op: OperatorParams) -> Self {
        Self {
            tl: op.tl,
            ar: op.ar,
            d1r: op.d1r,
            d2r: op.d2r,
            d1l: op.d1l,
            rr: op.rr,
            mul: op.mul,
            dt1: op.dt1,
            ksr: op.ksr,
            am_enable: op.am_enable,
            velocity_sensitivity: op.velocity_sensitivity,
            waveform: op.waveform,
            op_fine_tune: op.op_fine_tune,
            floor: op.floor,
            loop_enabled: op.loop_enabled,
            curve: op.curve,
            eg_shift: op.eg_shift,
        }
    }
}

/// フロントエンドから渡される/返すチャンネル単位パラメーター（`ChannelParams`のDTO）。
#[derive(Deserialize, Serialize)]
pub struct ChannelParamsDto {
    pub algorithm: u8,
    pub feedback: u8,
    pub tone_lfo_freq: u8,
    pub tone_lfo_pmd: u8,
    pub tone_lfo_amd: u8,
    pub tone_lfo_delay: u8,
    pub pms: u8,
    pub ams: u8,
    pub filter_cutoff: u8,
    pub filter_resonance: u8,
    pub filter_type: u8,
    pub filter_self_oscillation: bool,
    pub filter_eg_ar: u8,
    pub filter_eg_d1r: u8,
    pub filter_eg_d1l: u8,
    pub filter_eg_d2r: u8,
    pub filter_eg_rr: u8,
    pub filter_eg_depth: u8,
    pub vca_eg_ar: u8,
    pub vca_eg_d1r: u8,
    pub vca_eg_d1l: u8,
    pub vca_eg_d2r: u8,
    pub vca_eg_rr: u8,
    /// パフォーマンスLFOの波形/Fade/Offset（`ChannelParams.perf_lfo_shape`のDTO表現）。
    /// waveform/fade_modeは宣言順インデックス、offsetは中心128（オフセットなし）の
    /// 0〜255表現（ym38x6-vstのDAWパラメーターと同じ規約、`lfo_offset_from_param`/
    /// `lfo_offset_to_param`で相互変換する）。未送信の古いフロントエンドでも
    /// 旧来のハードエッジ挙動と等価な既定値になる。
    #[serde(default)]
    pub perf_lfo_waveform: u8,
    #[serde(default)]
    pub perf_lfo_fade_mode: u8,
    #[serde(default)]
    pub perf_lfo_fade_time: u8,
    #[serde(default = "default_perf_lfo_offset")]
    pub perf_lfo_offset: u8,
}

fn default_perf_lfo_offset() -> u8 {
    128
}

impl From<ChannelParamsDto> for ChannelParams {
    fn from(dto: ChannelParamsDto) -> Self {
        Self {
            algorithm: dto.algorithm,
            feedback: dto.feedback,
            chip_lfo_freq: dto.tone_lfo_freq,
            chip_lfo_pmd: dto.tone_lfo_pmd,
            chip_lfo_amd: dto.tone_lfo_amd,
            chip_lfo_delay: dto.tone_lfo_delay,
            pms: dto.pms,
            ams: dto.ams,
            filter_cutoff: dto.filter_cutoff,
            filter_resonance: dto.filter_resonance,
            filter_type: dto.filter_type,
            filter_self_oscillation: dto.filter_self_oscillation,
            // Cutoff FG/Gain FG: DTOはまだloop/floor/curve/バイポーラdepthを持たないため
            // （ステップ8でgesture-appのFGエディタUIを追加するまで）、EG本体と旧unipolar
            // depthの変換のみ反映する。Pitch FGはDTOに対応データが無いためデフォルト。
            cutoff_fg: BipolarFg {
                eg: EgParams {
                    ar: dto.filter_eg_ar,
                    d1r: dto.filter_eg_d1r,
                    d1l: dto.filter_eg_d1l,
                    d2r: dto.filter_eg_d2r,
                    rr: dto.filter_eg_rr,
                    floor: 0,
                    loop_enabled: 0,
                    curve: 0,
                },
                depth: (128.0 + dto.filter_eg_depth as f32 * 128.0 / 255.0).clamp(0.0, 255.0) as u8,
            },
            gain_fg: EgParams {
                ar: dto.vca_eg_ar,
                d1r: dto.vca_eg_d1r,
                d1l: dto.vca_eg_d1l,
                d2r: dto.vca_eg_d2r,
                rr: dto.vca_eg_rr,
                floor: 0,
                loop_enabled: 0,
                curve: 0,
            },
            texture_lfo: TextureLfo {
                waveform: perf_lfo_waveform_to_texture_lfo_index(dto.perf_lfo_waveform),
                fade_mode: dto.perf_lfo_fade_mode,
                fade_time: dto.perf_lfo_fade_time,
                offset: dto.perf_lfo_offset,
                ..TextureLfo::default()
            },
            pitch_fg: BipolarFg::default(),
        }
    }
}

impl From<ChannelParams> for ChannelParamsDto {
    fn from(ch: ChannelParams) -> Self {
        Self {
            algorithm: ch.algorithm,
            feedback: ch.feedback,
            tone_lfo_freq: ch.chip_lfo_freq,
            tone_lfo_pmd: ch.chip_lfo_pmd,
            tone_lfo_amd: ch.chip_lfo_amd,
            tone_lfo_delay: ch.chip_lfo_delay,
            pms: ch.pms,
            ams: ch.ams,
            filter_cutoff: ch.filter_cutoff,
            filter_resonance: ch.filter_resonance,
            filter_type: ch.filter_type,
            filter_self_oscillation: ch.filter_self_oscillation,
            filter_eg_ar: ch.cutoff_fg.eg.ar,
            filter_eg_d1r: ch.cutoff_fg.eg.d1r,
            filter_eg_d1l: ch.cutoff_fg.eg.d1l,
            filter_eg_d2r: ch.cutoff_fg.eg.d2r,
            filter_eg_rr: ch.cutoff_fg.eg.rr,
            filter_eg_depth: ch.cutoff_fg.depth,
            vca_eg_ar: ch.gain_fg.ar,
            vca_eg_d1r: ch.gain_fg.d1r,
            vca_eg_d1l: ch.gain_fg.d1l,
            vca_eg_d2r: ch.gain_fg.d2r,
            vca_eg_rr: ch.gain_fg.rr,
            perf_lfo_waveform: texture_lfo_index_to_perf_lfo_waveform(ch.texture_lfo.waveform),
            perf_lfo_fade_mode: ch.texture_lfo.fade_mode,
            perf_lfo_fade_time: ch.texture_lfo.fade_time,
            perf_lfo_offset: ch.texture_lfo.offset,
        }
    }
}

/// `ym38x6_set_patch`で受け渡しする/`get_preset_patch`が返すパッチ一式
/// （`Ym38x6Patch`のDTO）。
#[derive(Deserialize, Serialize)]
pub struct Ym38x6PatchDto {
    pub operators: [OperatorParamsDto; 4],
    pub channel: ChannelParamsDto,
}

impl From<Ym38x6PatchDto> for Ym38x6Patch {
    fn from(dto: Ym38x6PatchDto) -> Self {
        Self {
            operators: dto.operators.map(OperatorParams::from),
            channel: dto.channel.into(),
        }
    }
}

impl From<Ym38x6Patch> for Ym38x6PatchDto {
    fn from(patch: Ym38x6Patch) -> Self {
        Self {
            operators: patch.operators.map(OperatorParamsDto::from),
            channel: patch.channel.into(),
        }
    }
}

/// `list_presets`が返すプリセット一覧の1件（`PresetBank::sorted_entries()`に対応）。
#[derive(Serialize)]
pub struct PresetEntryDto {
    pub bank: u16,
    pub program: u8,
    pub name: String,
}

impl From<((u16, u8), Preset)> for PresetEntryDto {
    fn from(((bank, program), preset): ((u16, u8), Preset)) -> Self {
        Self { bank, program, name: preset.name }
    }
}

/// `open_patch_file`/`get_preset_patch`が返す、読み込んだ音色の内容。
/// `file_name`（実ファイル名、例: "organ_family.38x6"）と`patch_name`（音色名、
/// バンクファイル内の`PresetEntry.name`）は別概念のため分けて返す
/// （1ファイルに複数音色が入っている場合、ファイル名と音色名は一致しない）。
#[derive(Serialize)]
pub struct LoadedPatchDto {
    pub patch: Ym38x6PatchDto,
    pub patch_name: String,
    pub file_name: Option<String>,
    pub bank: u16,
    pub program: u8,
}

/// `save_patch_overwrite`/`save_patch_as`が成功時に返す保存結果。
#[derive(Serialize)]
pub struct SavedFileDto {
    pub patch_name: String,
    pub file_name: String,
    pub bank: u16,
    pub program: u8,
}
