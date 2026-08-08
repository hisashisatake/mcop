//! OP505パネル用の状態とハンドル。`handle.rs`/`state.rs`のym38x6版と異なり、フラットな
//! `EditorState`構造体を新設せず`Op505Patch`（op505-core、`Default`実装済み）をそのまま
//! `Rc<RefCell<>>`で保持する（設計判断はplan参照: 専用DTOを作らずOp505Patchを直接シリアライズ
//! する方針と対を成す。196値ぶんのミラー構造体を手書きしない）。
//!
//! TimeEg（`sound_core::TimeEgParams`）はOp505Patch内の4箇所（4つのOP EG＋Pitch/Cutoff/Gain FG）に
//! ネストしているため、`Op505IntField`（フラットなi32/bool 1個への単純ハンドル）とは別に、
//! 「TimeEgParams全体を読み書きするgetter/setter関数ポインタ＋その中の1フィールドを指すenum」を
//! 組み合わせた`Op505TimeEgIntField`/`Op505TimeEgLoopField`を用意し、4箇所どこでも同じ実装で
//! 構造値（stage_count/loop_enabled/loop_start/loop_end/release_start）のハンドルを作れるようにする。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use op505_core::Op505Patch;
use op505_ui::{
    BoolParamHandle, IntParamHandle, Op505BipolarFgPanelParams, Op505OperatorPanelParams, Op505PanelParams,
    Op505TimeEgPanelParams,
};
use sound_core::TimeEgParams;

/// `Op505Patch`内の1つのi32相当(u8)フィールドへの単純ハンドル。`handle.rs::IntField`のOP505版。
pub struct Op505IntField {
    pub state: Rc<RefCell<Op505Patch>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&Op505Patch) -> i32,
    pub set: fn(&mut Op505Patch, i32),
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub name: &'static str,
}

impl IntParamHandle for Op505IntField {
    fn value(&self) -> i32 {
        (self.get)(&self.state.borrow())
    }
    fn min(&self) -> i32 {
        self.min
    }
    fn max(&self) -> i32 {
        self.max
    }
    fn default(&self) -> i32 {
        self.default
    }
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        let clamped = value.clamp(self.min, self.max);
        (self.set)(&mut self.state.borrow_mut(), clamped);
        self.dirty.set(true);
    }
    fn end_edit(&self) {}
}

/// `Op505Patch`内の1つのboolフィールドへの単純ハンドル。
pub struct Op505BoolField {
    pub state: Rc<RefCell<Op505Patch>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&Op505Patch) -> bool,
    pub set: fn(&mut Op505Patch, bool),
}

impl BoolParamHandle for Op505BoolField {
    fn value(&self) -> bool {
        (self.get)(&self.state.borrow())
    }
    fn begin_edit(&self) {}
    fn set(&self, value: bool) {
        (self.set)(&mut self.state.borrow_mut(), value);
        self.dirty.set(true);
    }
    fn end_edit(&self) {}
}

/// `Op505TimeEgIntField`が指すTimeEgParams内のどのフィールドか。
#[derive(Clone, Copy)]
enum TimeEgIntKind {
    StageCount,
    LoopStart,
    LoopEnd,
    ReleaseStart,
}

/// TimeEgParams全体のgetter/setter(`get_eg`/`set_eg`、Op505Patch内の4箇所どこでも同じ形)を介して
/// その中の1つの構造値フィールドを読み書きするハンドル。個々の段(stages[i])はまだ編集できない
/// （TimeEgエディタ本体はStep 8）。
pub struct Op505TimeEgIntField {
    state: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
    kind: TimeEgIntKind,
    min: i32,
    max: i32,
    default: i32,
    name: &'static str,
}

impl IntParamHandle for Op505TimeEgIntField {
    fn value(&self) -> i32 {
        let eg = (self.get_eg)(&self.state.borrow());
        match self.kind {
            TimeEgIntKind::StageCount => eg.stage_count as i32,
            TimeEgIntKind::LoopStart => eg.loop_start as i32,
            TimeEgIntKind::LoopEnd => eg.loop_end as i32,
            TimeEgIntKind::ReleaseStart => eg.release_start as i32,
        }
    }
    fn min(&self) -> i32 {
        self.min
    }
    fn max(&self) -> i32 {
        self.max
    }
    fn default(&self) -> i32 {
        self.default
    }
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        let clamped = value.clamp(self.min, self.max) as u8;
        let mut patch = self.state.borrow_mut();
        let mut eg = (self.get_eg)(&patch);
        match self.kind {
            TimeEgIntKind::StageCount => eg.stage_count = clamped,
            TimeEgIntKind::LoopStart => eg.loop_start = clamped,
            TimeEgIntKind::LoopEnd => eg.loop_end = clamped,
            TimeEgIntKind::ReleaseStart => eg.release_start = clamped,
        }
        (self.set_eg)(&mut patch, eg);
        drop(patch);
        self.dirty.set(true);
    }
    fn end_edit(&self) {}
}

/// `loop_enabled`(u8だがON/OFFの2値)専用のboolハンドル。
pub struct Op505TimeEgLoopField {
    state: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
}

impl BoolParamHandle for Op505TimeEgLoopField {
    fn value(&self) -> bool {
        (self.get_eg)(&self.state.borrow()).loop_enabled != 0
    }
    fn begin_edit(&self) {}
    fn set(&self, value: bool) {
        let mut patch = self.state.borrow_mut();
        let mut eg = (self.get_eg)(&patch);
        eg.loop_enabled = value as u8;
        (self.set_eg)(&mut patch, eg);
        drop(patch);
        self.dirty.set(true);
    }
    fn end_edit(&self) {}
}

/// TimeEgParams1本ぶんの`Op505TimeEgPanelParams`を組み立てる。`get_eg`/`set_eg`はOp505Patch内の
/// 該当箇所（`operators[I].eg`/`channel.pitch_fg.eg`等）を指す関数ポインタ。
fn time_eg_panel_params(
    state: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
) -> Op505TimeEgPanelParams<'static> {
    let params = get_eg(&state.borrow());
    macro_rules! int_kind {
        ($kind:expr, $name:literal, $min:expr, $max:expr, $default:expr) => {
            Box::new(Op505TimeEgIntField {
                state: state.clone(),
                dirty: dirty.clone(),
                get_eg,
                set_eg,
                kind: $kind,
                min: $min,
                max: $max,
                default: $default,
                name: $name,
            }) as Box<dyn IntParamHandle>
        };
    }
    Op505TimeEgPanelParams {
        params,
        stage_count: int_kind!(TimeEgIntKind::StageCount, "Stages", 1, 8, 1),
        loop_enabled: Box::new(Op505TimeEgLoopField { state: state.clone(), dirty: dirty.clone(), get_eg, set_eg })
            as Box<dyn BoolParamHandle>,
        loop_start: int_kind!(TimeEgIntKind::LoopStart, "Loop Start", 0, 7, 0),
        loop_end: int_kind!(TimeEgIntKind::LoopEnd, "Loop End", 0, 7, 0),
        release_start: int_kind!(TimeEgIntKind::ReleaseStart, "Release Start", 0, 7, 0),
    }
}

/// バイポーラFG（Pitch/Cutoff）1本ぶんの`Op505BipolarFgPanelParams`を組み立てる。
/// `get_depth`/`set_depth`は非キャプチャ関数ポインタである必要があるため（`Op505IntField::get`が
/// `fn`型）、`Op505BipolarFg`経由で導出せず、呼び出し側から直接のフィールドパスを渡してもらう。
fn bipolar_fg_panel_params(
    state: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    get_depth: fn(&Op505Patch) -> i32,
    set_depth: fn(&mut Op505Patch, i32),
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
    depth_name: &'static str,
) -> Op505BipolarFgPanelParams<'static> {
    Op505BipolarFgPanelParams {
        eg: time_eg_panel_params(state, dirty, get_eg, set_eg),
        depth: Box::new(Op505IntField {
            state: state.clone(),
            dirty: dirty.clone(),
            get: get_depth,
            set: set_depth,
            min: 0,
            max: 255,
            default: 128,
            name: depth_name,
        }) as Box<dyn IntParamHandle>,
    }
}

fn operator_panel_params<const I: usize>(
    state: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
) -> Op505OperatorPanelParams<'static> {
    macro_rules! op {
        ($field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
            Box::new(Op505IntField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.operators[I].$field as i32,
                set: |p: &mut Op505Patch, v: i32| p.operators[I].$field = v as u8,
                min: $min,
                max: $max,
                default: $default,
                name: $name,
            }) as Box<dyn IntParamHandle>
        };
    }
    Op505OperatorPanelParams {
        tl: op!(tl, "TL", 0, 255, 200),
        eg: time_eg_panel_params(
            state,
            dirty,
            |p: &Op505Patch| p.operators[I].eg,
            |p: &mut Op505Patch, v: TimeEgParams| p.operators[I].eg = v,
        ),
        mul: op!(mul, "MUL", 0, 15, 1),
        dt1: op!(dt1, "DT1", 0, 255, 128),
        ksr: op!(ksr, "KSR", 0, 255, 64),
        vel_sens: Box::new(Op505IntField {
            state: state.clone(),
            dirty: dirty.clone(),
            get: |p: &Op505Patch| p.operators[I].velocity_sensitivity as i32,
            set: |p: &mut Op505Patch, v: i32| p.operators[I].velocity_sensitivity = v as u8,
            min: 0,
            max: 255,
            default: 0,
            name: "VEL",
        }) as Box<dyn IntParamHandle>,
        op_fine_tune: op!(op_fine_tune, "FINE", 0, 255, 128),
        ame: Box::new(Op505BoolField {
            state: state.clone(),
            dirty: dirty.clone(),
            get: |p: &Op505Patch| p.operators[I].am_enable,
            set: |p: &mut Op505Patch, v: bool| p.operators[I].am_enable = v,
        }) as Box<dyn BoolParamHandle>,
        waveform: op!(waveform, "Waveform", 0, 255, 0),
        eg_shift: op!(eg_shift, "EGSFT", 0, 255, 0),
        level_scale: op!(level_scale, "LEVEL SCALE", 0, 255, 0),
        velocity_gain: op!(velocity_gain, "V.GAIN", 0, 255, 255),
    }
}

/// `Op505Patch`の状態と`app.rs`から呼ぶ組み立て関数一式。
pub struct Op505State {
    pub patch: Rc<RefCell<Op505Patch>>,
    pub dirty: Rc<Cell<bool>>,
}

impl Op505State {
    /// `Op505Patch::default()`で開始しつつ、バックエンドの現在のカレントパッチ
    /// （main.js側のデモ選択/Bank・Program変換で既に設定済みの内容）を非同期で読み込み直す
    /// （`app.rs::EditorApp::new()`の`handle_navigate`初期ロードと同じ「起動直後に一度だけ」パターン）。
    /// これが無いと、エディタを開いた直後に何かノブへ触れた瞬間、`Op505Patch::default()`ベースの
    /// 値でバックエンドの現在パッチ（デモ音色等）を丸ごと上書きしてしまう。
    pub fn new() -> Self {
        let patch = Rc::new(RefCell::new(Op505Patch::default()));
        let dirty = Rc::new(Cell::new(false));
        let patch_for_load = patch.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(current) = crate::ipc::load_op505_current_patch().await {
                *patch_for_load.borrow_mut() = current;
                crate::shift_keys::request_repaint();
            }
        });
        Self { patch, dirty }
    }

    pub fn build_panel_params(&self) -> Op505PanelParams<'static> {
        let state = &self.patch;
        let dirty = &self.dirty;
        macro_rules! ch {
            ($field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
                Box::new(Op505IntField {
                    state: state.clone(),
                    dirty: dirty.clone(),
                    get: |p: &Op505Patch| p.channel.$field as i32,
                    set: |p: &mut Op505Patch, v: i32| p.channel.$field = v as u8,
                    min: $min,
                    max: $max,
                    default: $default,
                    name: $name,
                }) as Box<dyn IntParamHandle>
            };
        }
        macro_rules! tlfo {
            ($field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
                Box::new(Op505IntField {
                    state: state.clone(),
                    dirty: dirty.clone(),
                    get: |p: &Op505Patch| p.channel.texture_lfo.$field as i32,
                    set: |p: &mut Op505Patch, v: i32| p.channel.texture_lfo.$field = v as u8,
                    min: $min,
                    max: $max,
                    default: $default,
                    name: $name,
                }) as Box<dyn IntParamHandle>
            };
        }
        Op505PanelParams {
            algorithm: ch!(algorithm, "Algorithm", 0, 7, 0),
            feedback: ch!(feedback, "Feedback", 0, 255, 0),
            chip_lfo_freq: ch!(chip_lfo_freq, "Chip LFO Freq", 0, 255, 0),
            chip_lfo_pmd: ch!(chip_lfo_pmd, "Chip LFO PMD", 0, 255, 0),
            chip_lfo_amd: ch!(chip_lfo_amd, "Chip LFO AMD", 0, 255, 0),
            chip_lfo_delay: ch!(chip_lfo_delay, "Chip LFO Delay", 0, 255, 0),
            pms: ch!(pms, "PMS", 0, 255, 0),
            ams: ch!(ams, "AMS", 0, 255, 0),
            texture_lfo_rate: tlfo!(rate, "Texture LFO Rate", 0, 255, 0),
            texture_lfo_depth: tlfo!(depth, "Texture LFO Depth", 0, 255, 0),
            texture_lfo_delay: tlfo!(delay, "Texture LFO Delay", 0, 255, 0),
            texture_lfo_waveform: tlfo!(waveform, "Texture LFO Waveform", 0, 4, 0),
            texture_lfo_fade_mode: tlfo!(fade_mode, "Texture LFO Fade Mode", 0, 3, 0),
            texture_lfo_fade_time: tlfo!(fade_time, "Texture LFO Fade Time", 0, 255, 0),
            texture_lfo_offset: tlfo!(offset, "Texture LFO Offset", 0, 255, 128),
            texture_lfo_destination: tlfo!(destination, "Texture LFO Destination", 0, 4, 0),
            cutoff: ch!(filter_cutoff, "Filter Cutoff", 0, 255, 255),
            resonance: ch!(filter_resonance, "Filter Resonance", 0, 255, 0),
            filter_type: ch!(filter_type, "Filter Type", 0, 255, 0),
            filter_self_oscillation: Box::new(Op505BoolField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.channel.filter_self_oscillation,
                set: |p: &mut Op505Patch, v: bool| p.channel.filter_self_oscillation = v,
            }) as Box<dyn BoolParamHandle>,
            pitch_fg: bipolar_fg_panel_params(
                state,
                dirty,
                |p: &Op505Patch| p.channel.pitch_fg.depth as i32,
                |p: &mut Op505Patch, v: i32| p.channel.pitch_fg.depth = v as u8,
                |p: &Op505Patch| p.channel.pitch_fg.eg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.pitch_fg.eg = v,
                "Pitch FG Depth",
            ),
            cutoff_fg: bipolar_fg_panel_params(
                state,
                dirty,
                |p: &Op505Patch| p.channel.cutoff_fg.depth as i32,
                |p: &mut Op505Patch, v: i32| p.channel.cutoff_fg.depth = v as u8,
                |p: &Op505Patch| p.channel.cutoff_fg.eg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.cutoff_fg.eg = v,
                "Cutoff FG Depth",
            ),
            gain_fg: time_eg_panel_params(
                state,
                dirty,
                |p: &Op505Patch| p.channel.gain_fg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.gain_fg = v,
            ),
            operators: [
                operator_panel_params::<0>(state, dirty),
                operator_panel_params::<1>(state, dirty),
                operator_panel_params::<2>(state, dirty),
                operator_panel_params::<3>(state, dirty),
            ],
        }
    }
}

impl Default for Op505State {
    fn default() -> Self {
        Self::new()
    }
}
