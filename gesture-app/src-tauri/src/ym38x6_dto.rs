use serde::{Deserialize, Serialize};
use ym38x6_core::{BipolarFg, ChannelParams, EgParams, OperatorParams, Preset, TextureLfo, Ym38x6Patch};

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
    /// Level Scaling（ノート依存の出力レベル減衰、OPL系KSL相当）。0〜255、既定0＝スケーリングなし。
    #[serde(default)]
    pub level_scale: u8,
    /// キャリア出力へのベロシティ音量ゲイン深さ（0〜255、既定255＝フル）。未送信の古い
    /// フロントエンドでもフル（旧チャンネル一括velocity/127と同一）として扱う。
    #[serde(default = "default_velocity_gain")]
    pub velocity_gain: u8,
}

fn default_op_fine_tune() -> u8 {
    128
}

fn default_velocity_gain() -> u8 {
    255
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
            level_scale: dto.level_scale,
            velocity_gain: dto.velocity_gain,
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
            level_scale: op.level_scale,
            velocity_gain: op.velocity_gain,
        }
    }
}

/// フロントエンドから渡される/返すチャンネル単位パラメーター（`ChannelParams`のDTO）。
#[derive(Deserialize, Serialize)]
pub struct ChannelParamsDto {
    pub algorithm: u8,
    pub feedback: u8,
    pub chip_lfo_freq: u8,
    pub chip_lfo_pmd: u8,
    pub chip_lfo_amd: u8,
    pub chip_lfo_delay: u8,
    pub pms: u8,
    pub ams: u8,
    pub filter_cutoff: u8,
    pub filter_resonance: u8,
    pub filter_type: u8,
    pub filter_self_oscillation: bool,
    // Pitch FG（新規、共通EG＋バイポーラDepth）。未送信の古いフロントエンドでも
    // ym38x6-core::default_pitch_fg()相当の「無効」状態になる。
    #[serde(default)]
    pub pitch_fg_ar: u8,
    #[serde(default)]
    pub pitch_fg_d1r: u8,
    #[serde(default = "default_pitch_fg_d1l")]
    pub pitch_fg_d1l: u8,
    #[serde(default)]
    pub pitch_fg_d2r: u8,
    #[serde(default = "default_pitch_fg_rr")]
    pub pitch_fg_rr: u8,
    #[serde(default = "default_bipolar_depth")]
    pub pitch_fg_depth: u8,
    #[serde(default)]
    pub pitch_fg_floor: u8,
    #[serde(default)]
    pub pitch_fg_delay: u8,
    #[serde(default)]
    pub pitch_fg_loop: u8,
    #[serde(default)]
    pub pitch_fg_curve: u8,
    // Cutoff FG（旧Filter EG。depthはバイポーラの直接値、旧unipolar変換式は撤去済み）。
    pub cutoff_fg_ar: u8,
    pub cutoff_fg_d1r: u8,
    pub cutoff_fg_d1l: u8,
    pub cutoff_fg_d2r: u8,
    pub cutoff_fg_rr: u8,
    #[serde(default = "default_bipolar_depth")]
    pub cutoff_fg_depth: u8,
    #[serde(default)]
    pub cutoff_fg_floor: u8,
    #[serde(default)]
    pub cutoff_fg_delay: u8,
    #[serde(default)]
    pub cutoff_fg_loop: u8,
    #[serde(default)]
    pub cutoff_fg_curve: u8,
    // Gain FG（旧VCA EG。音量に負値は無いためDepthを持たずFloorが深さ役）。
    pub gain_fg_ar: u8,
    pub gain_fg_d1r: u8,
    pub gain_fg_d1l: u8,
    pub gain_fg_d2r: u8,
    pub gain_fg_rr: u8,
    #[serde(default)]
    pub gain_fg_floor: u8,
    #[serde(default)]
    pub gain_fg_delay: u8,
    #[serde(default)]
    pub gain_fg_loop: u8,
    #[serde(default)]
    pub gain_fg_curve: u8,
    /// 質感LFO全項目（`ChannelParams.texture_lfo`のDTO表現）。waveform/fade_modeは宣言順
    /// インデックス（waveform=0〜4の直接値、旧8波形経由の変換は撤去済み）、destinationは
    /// 0=Pitch/1=Volume/2=TL/3=Cutoff、offsetは中心128（オフセットなし）の0〜255表現。
    /// rate/depth/delay/destinationは旧DTOでは往復で破棄されていたが、エディタ経由の
    /// send_patchでも実際に反映されるようここで保持する（main.jsのランタイム制御が
    /// 書いた値をエディタが上書きしないようにする役割も兼ねる）。
    #[serde(default)]
    pub texture_lfo_waveform: u8,
    #[serde(default)]
    pub texture_lfo_destination: u8,
    #[serde(default)]
    pub texture_lfo_rate: u8,
    #[serde(default)]
    pub texture_lfo_depth: u8,
    #[serde(default)]
    pub texture_lfo_delay: u8,
    #[serde(default)]
    pub texture_lfo_fade_mode: u8,
    #[serde(default)]
    pub texture_lfo_fade_time: u8,
    #[serde(default = "default_texture_lfo_offset")]
    pub texture_lfo_offset: u8,
}

fn default_bipolar_depth() -> u8 {
    128
}

fn default_pitch_fg_d1l() -> u8 {
    255
}

fn default_pitch_fg_rr() -> u8 {
    255
}

fn default_texture_lfo_offset() -> u8 {
    128
}

impl From<ChannelParamsDto> for ChannelParams {
    fn from(dto: ChannelParamsDto) -> Self {
        Self {
            algorithm: dto.algorithm,
            feedback: dto.feedback,
            chip_lfo_freq: dto.chip_lfo_freq,
            chip_lfo_pmd: dto.chip_lfo_pmd,
            chip_lfo_amd: dto.chip_lfo_amd,
            chip_lfo_delay: dto.chip_lfo_delay,
            pms: dto.pms,
            ams: dto.ams,
            filter_cutoff: dto.filter_cutoff,
            filter_resonance: dto.filter_resonance,
            filter_type: dto.filter_type,
            filter_self_oscillation: dto.filter_self_oscillation,
            pitch_fg: BipolarFg {
                eg: EgParams {
                    ar: dto.pitch_fg_ar,
                    d1r: dto.pitch_fg_d1r,
                    d1l: dto.pitch_fg_d1l,
                    d2r: dto.pitch_fg_d2r,
                    rr: dto.pitch_fg_rr,
                    floor: dto.pitch_fg_floor,
                    loop_enabled: dto.pitch_fg_loop,
                    curve: dto.pitch_fg_curve,
                    delay: dto.pitch_fg_delay,
                },
                depth: dto.pitch_fg_depth,
            },
            cutoff_fg: BipolarFg {
                eg: EgParams {
                    ar: dto.cutoff_fg_ar,
                    d1r: dto.cutoff_fg_d1r,
                    d1l: dto.cutoff_fg_d1l,
                    d2r: dto.cutoff_fg_d2r,
                    rr: dto.cutoff_fg_rr,
                    floor: dto.cutoff_fg_floor,
                    loop_enabled: dto.cutoff_fg_loop,
                    curve: dto.cutoff_fg_curve,
                    delay: dto.cutoff_fg_delay,
                },
                depth: dto.cutoff_fg_depth,
            },
            gain_fg: EgParams {
                ar: dto.gain_fg_ar,
                d1r: dto.gain_fg_d1r,
                d1l: dto.gain_fg_d1l,
                d2r: dto.gain_fg_d2r,
                rr: dto.gain_fg_rr,
                floor: dto.gain_fg_floor,
                loop_enabled: dto.gain_fg_loop,
                curve: dto.gain_fg_curve,
                delay: dto.gain_fg_delay,
            },
            texture_lfo: TextureLfo {
                waveform: dto.texture_lfo_waveform,
                destination: dto.texture_lfo_destination,
                rate: dto.texture_lfo_rate,
                depth: dto.texture_lfo_depth,
                delay: dto.texture_lfo_delay,
                fade_mode: dto.texture_lfo_fade_mode,
                fade_time: dto.texture_lfo_fade_time,
                offset: dto.texture_lfo_offset,
            },
        }
    }
}

impl From<ChannelParams> for ChannelParamsDto {
    fn from(ch: ChannelParams) -> Self {
        Self {
            algorithm: ch.algorithm,
            feedback: ch.feedback,
            chip_lfo_freq: ch.chip_lfo_freq,
            chip_lfo_pmd: ch.chip_lfo_pmd,
            chip_lfo_amd: ch.chip_lfo_amd,
            chip_lfo_delay: ch.chip_lfo_delay,
            pms: ch.pms,
            ams: ch.ams,
            filter_cutoff: ch.filter_cutoff,
            filter_resonance: ch.filter_resonance,
            filter_type: ch.filter_type,
            filter_self_oscillation: ch.filter_self_oscillation,
            pitch_fg_ar: ch.pitch_fg.eg.ar,
            pitch_fg_d1r: ch.pitch_fg.eg.d1r,
            pitch_fg_d1l: ch.pitch_fg.eg.d1l,
            pitch_fg_d2r: ch.pitch_fg.eg.d2r,
            pitch_fg_rr: ch.pitch_fg.eg.rr,
            pitch_fg_depth: ch.pitch_fg.depth,
            pitch_fg_floor: ch.pitch_fg.eg.floor,
            pitch_fg_delay: ch.pitch_fg.eg.delay,
            pitch_fg_loop: ch.pitch_fg.eg.loop_enabled,
            pitch_fg_curve: ch.pitch_fg.eg.curve,
            cutoff_fg_ar: ch.cutoff_fg.eg.ar,
            cutoff_fg_d1r: ch.cutoff_fg.eg.d1r,
            cutoff_fg_d1l: ch.cutoff_fg.eg.d1l,
            cutoff_fg_d2r: ch.cutoff_fg.eg.d2r,
            cutoff_fg_rr: ch.cutoff_fg.eg.rr,
            cutoff_fg_depth: ch.cutoff_fg.depth,
            cutoff_fg_floor: ch.cutoff_fg.eg.floor,
            cutoff_fg_delay: ch.cutoff_fg.eg.delay,
            cutoff_fg_loop: ch.cutoff_fg.eg.loop_enabled,
            cutoff_fg_curve: ch.cutoff_fg.eg.curve,
            gain_fg_ar: ch.gain_fg.ar,
            gain_fg_d1r: ch.gain_fg.d1r,
            gain_fg_d1l: ch.gain_fg.d1l,
            gain_fg_d2r: ch.gain_fg.d2r,
            gain_fg_rr: ch.gain_fg.rr,
            gain_fg_floor: ch.gain_fg.floor,
            gain_fg_delay: ch.gain_fg.delay,
            gain_fg_loop: ch.gain_fg.loop_enabled,
            gain_fg_curve: ch.gain_fg.curve,
            texture_lfo_waveform: ch.texture_lfo.waveform,
            texture_lfo_destination: ch.texture_lfo.destination,
            texture_lfo_rate: ch.texture_lfo.rate,
            texture_lfo_depth: ch.texture_lfo.depth,
            texture_lfo_delay: ch.texture_lfo.delay,
            texture_lfo_fade_mode: ch.texture_lfo.fade_mode,
            texture_lfo_fade_time: ch.texture_lfo.fade_time,
            texture_lfo_offset: ch.texture_lfo.offset,
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
