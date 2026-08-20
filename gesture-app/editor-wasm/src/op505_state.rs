//! OP505パネル用の状態とハンドル。フラットなミラー構造体を新設せず`Op505Patch`
//! （op505-core、`Default`実装済み）をそのまま`Rc<RefCell<>>`で保持する
//! （設計判断はplan参照: 専用DTOを作らずOp505Patchを直接シリアライズする方針と対を成す。
//! 196値ぶんのミラー構造体を手書きしない）。
//!
//! TimeEg（`sound_core::TimeEgParams`）はOp505Patch内の7箇所（4つのOP EG＋Pitch/Cutoff/Gain FG）に
//! ネストしているため、`Op505IntField`（フラットなi32/bool 1個への単純ハンドル）とは別に、
//! `Op505TimeEgHandle`（`ui_core::TimeEgHandle`実装、TimeEgParams全体を読み書きするgetter/setter
//! 関数ポインタを保持）を用意し、7箇所どこでも同じ実装でハンドルを作れるようにする
//! （個々の段×フィールドのハンドルは`ui_core::time_eg_editor`が内部で都度導出するため、
//! ここでは持たない）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use op505_core::Op505Patch;
use op505_ui::{BoolParamHandle, IntParamHandle, Op505BipolarFgPanelParams, Op505OperatorPanelParams, Op505PanelParams, TimeEgHandle};
use sound_core::TimeEgParams;

use crate::handle::int_field;
use crate::state::MasterEffectsState;

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

/// TimeEgParams全体（Op505Patch内の7箇所どこか）への`ui_core::TimeEgHandle`実装。
/// `get_eg`/`set_eg`は非キャプチャ関数ポインタ（Op505Patch内の該当箇所を指す）。
/// `name`はegui memoryのId salt兼パネル見出しに使われる（`time_eg_editor`参照）ため、
/// "OP1 EG"/"PITCH FG"等、7本で一意な値を渡すこと。
pub struct Op505TimeEgHandle {
    state: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
    name: &'static str,
}

impl TimeEgHandle for Op505TimeEgHandle {
    fn params(&self) -> TimeEgParams {
        (self.get_eg)(&self.state.borrow())
    }
    fn set_params(&self, params: TimeEgParams) {
        (self.set_eg)(&mut self.state.borrow_mut(), params);
        self.dirty.set(true);
    }
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn begin_edit(&self) {}
    fn end_edit(&self) {}
}

/// TimeEgParams1本ぶんの`Op505TimeEgHandle`を組み立てる。`get_eg`/`set_eg`はOp505Patch内の
/// 該当箇所（`operators[I].eg`/`channel.pitch_fg.eg`等）を指す関数ポインタ。
fn time_eg_handle(
    state: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
    name: &'static str,
) -> Box<dyn TimeEgHandle> {
    Box::new(Op505TimeEgHandle { state: state.clone(), dirty: dirty.clone(), get_eg, set_eg, name })
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
    eg_name: &'static str,
    depth_name: &'static str,
) -> Op505BipolarFgPanelParams<'static> {
    Op505BipolarFgPanelParams {
        eg: time_eg_handle(state, dirty, get_eg, set_eg, eg_name),
        depth: Box::new(Op505IntField {
            state: state.clone(),
            dirty: dirty.clone(),
            get: get_depth,
            set: set_depth,
            min: 0,
            max: 255,
            // 符号を持たない振れ幅の倍率。0＝変調なし（符号はFGのレベル波形側が持つ）。
            default: 0,
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
    let eg_name: &'static str = match I {
        0 => "OP1 EG",
        1 => "OP2 EG",
        2 => "OP3 EG",
        3 => "OP4 EG",
        _ => "OP EG",
    };
    Op505OperatorPanelParams {
        tl: op!(tl, "TL", 0, 255, 200),
        eg: time_eg_handle(
            state,
            dirty,
            |p: &Op505Patch| p.operators[I].eg,
            |p: &mut Op505Patch, v: TimeEgParams| p.operators[I].eg = v,
            eg_name,
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
    /// `Op505Patch::default()`で開始する。バックエンドの現在パッチ（main.js側のデモ選択/Bank変換で
    /// 既に設定済みの内容）との同期は`EditorApp::new()`の初期`handle_navigate`
    /// （PRESETSサイドバー経由のbankベースロード）が肩代わりするため、ここでの個別フェッチは不要
    /// （旧`spawn_reload`はここにあったが、engine_sync::SELECTION_STALE経由の再ナビゲートへ統合された）。
    pub fn new() -> Self {
        let patch = Rc::new(RefCell::new(Op505Patch::default()));
        let dirty = Rc::new(Cell::new(false));
        Self { patch, dirty }
    }

    /// `master_state`/`master_dirty`はエンジン非依存のMASTER EFFECTS状態
    /// （`app.rs`の`self.master_effects`/`self.master_effects_dirty`をそのまま渡す）。
    pub fn build_panel_params(
        &self,
        master_state: &Rc<RefCell<MasterEffectsState>>,
        master_dirty: &Rc<Cell<bool>>,
    ) -> Op505PanelParams<'static> {
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
            texture_lfo_rate: tlfo!(rate, "Texture LFO Rate", 0, 255, 0),
            texture_lfo_depth: tlfo!(depth, "Texture LFO Depth", 0, 255, 0),
            texture_lfo_delay: tlfo!(delay, "Texture LFO Delay", 0, 255, 0),
            texture_lfo_waveform: tlfo!(waveform, "Texture LFO Waveform", 0, 4, 0),
            texture_lfo_fade_mode: tlfo!(fade_mode, "Texture LFO Fade Mode", 0, 3, 0),
            texture_lfo_fade_time: tlfo!(fade_time, "Texture LFO Fade Time", 0, 255, 0),
            texture_lfo_offset: tlfo!(offset, "Texture LFO Offset", 0, 255, 128),
            // default=0はFmLfoDestination::Unplugged（ダブルクリック/Ctrl+クリックでのリセット先。
            // 2026-08-18並べ替え後、Unplugged=0）。
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
                "PITCH FG",
                "Pitch FG Depth",
            ),
            cutoff_fg: bipolar_fg_panel_params(
                state,
                dirty,
                |p: &Op505Patch| p.channel.cutoff_fg.depth as i32,
                |p: &mut Op505Patch, v: i32| p.channel.cutoff_fg.depth = v as u8,
                |p: &Op505Patch| p.channel.cutoff_fg.eg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.cutoff_fg.eg = v,
                "CUTOFF FG",
                "Cutoff FG Depth",
            ),
            gain_fg: time_eg_handle(
                state,
                dirty,
                |p: &Op505Patch| p.channel.gain_fg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.gain_fg = v,
                "GAIN FG",
            ),
            gain_fg_to_master: Box::new(Op505BoolField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.channel.gain_fg_to_master,
                set: |p: &mut Op505Patch, v: bool| p.channel.gain_fg_to_master = v,
            }) as Box<dyn BoolParamHandle>,
            gain_fg_to_operators: Box::new(Op505BoolField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.channel.gain_fg_to_operators,
                set: |p: &mut Op505Patch, v: bool| p.channel.gain_fg_to_operators = v,
            }) as Box<dyn BoolParamHandle>,
            rev_send: int_field!(master_state, master_dirty, rev_send, "Reverb Send", 0, 255, 0),
            reverb_type: int_field!(master_state, master_dirty, reverb_type, "Reverb Type", 0, 7, 3),
            reverb_time: int_field!(master_state, master_dirty, reverb_time, "Reverb Time", 0, 255, 128),
            cho_send: int_field!(master_state, master_dirty, cho_send, "Chorus Send", 0, 255, 0),
            chorus_type: int_field!(master_state, master_dirty, chorus_type, "Chorus Type", 0, 7, 0),
            chorus_mod_rate: int_field!(master_state, master_dirty, chorus_mod_rate, "Chorus Mod Rate", 0, 255, 128),
            chorus_mod_depth: int_field!(master_state, master_dirty, chorus_mod_depth, "Chorus Mod Depth", 0, 255, 128),
            chorus_feedback: int_field!(master_state, master_dirty, chorus_feedback, "Chorus Feedback", 0, 255, 0),
            chorus_send_to_reverb: int_field!(master_state, master_dirty, chorus_send_to_reverb, "Chorus Send To Reverb", 0, 255, 0),
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
