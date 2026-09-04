use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use op505_core::Op505PresetBank;
use op505_editor::layout::editor_min_size;
use op505_editor::panel_source::build_panel_params;
use op505_editor::param_spec::IntField;
use op505_editor::patch_source::{read_bool, read_eg, read_int, MasterEffectsState};
use op505_editor::preset_panel::{draw_editor_top_bar, draw_presets_drawer, poll_presets_events, EditorPresetState, UndoUiState};
use op505_editor::undo::{EditorSnapshot, SnapshotDiff, UndoApply, UndoStack};
use op505_ui::draw_op505_panel;
use sound_core::MeterBridge;
use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use crate::param_adapter::VstPanelSource;
use crate::params::{bool_param_ref, build_patch, int_param_ref, write_eg_bank_slot, Op505VstParams};
use crate::preset_host::VstPresetHost;

pub(crate) struct EditorState {
    pub(crate) presets: EditorPresetState,
    /// Undo/Redoスタック本体。`create_egui_editor`のユーザー状態は`T: Send`を要求するため
    /// （`Arc<Mutex<T>>`に入る、`nice-plug-egui`側の制約）、standaloneが使う
    /// `Rc<RefCell<UndoStack>>`はそのまま持ち込めない。`RefCell`のみで足りる
    /// （`UndoStack`自体はスレッドをまたがず、この構造体ごとMutexに包まれるため）。
    pub(crate) undo: RefCell<UndoStack>,
}

/// VSTエディタウィンドウの最小高さ。`ResizableWindow`内はスクロールするため小さめでよい
/// （standaloneの固定サイズ720pxとは異なる事情、詳細は`op505_editor::layout`のdoc参照）。
const MIN_HEIGHT: f32 = 480.0;

/// 現在のDAWパラメーター＋TimeEg束＋PRESETS選択状態から、Undo用の[`EditorSnapshot`]を組み立てる。
/// standaloneの`EditorApp::snapshot()`のVST版（`Rc<RefCell<Op505Patch>>`を直接持つstandaloneと違い、
/// VSTはDAWパラメーターが実体のため毎回`build_patch`で組み立て直す）。
fn vst_snapshot(params: &Op505VstParams, presets: &EditorPresetState) -> EditorSnapshot {
    let egs = *params.egs.read().expect("Poisoned RwLock on read");
    let patch = build_patch(params, &egs);
    let master = MasterEffectsState::from_fn(|fx| int_param_ref(params, IntField::Fx(fx)).modulated_plain_value());
    EditorSnapshot {
        patch,
        master,
        patch_name: presets.patch_name().to_string(),
        bank: presets.bank(),
        program: presets.program(),
        has_selection: presets.has_selection(),
    }
}

/// `diff`が指すフィールドだけを`target`の値へ書き換える。DAWパラメーターは`setter`経由
/// （`begin_set_parameter`/`set_parameter`/`end_set_parameter`の3点セット、`apply_patch`と同じ
/// 呼び出し方）、TimeEg 7本は`params.egs`へ直接書く（persist状態のためgesture不要）。
/// 全パラメーターを一括再適用する`apply_patch`と違い、変化したフィールドだけを書くことで
/// Undo 1回がDAWのUndo履歴・オートメーションへ与える影響を実際の変更分だけに絞る。
fn apply_snapshot_diff(params: &Op505VstParams, setter: &ParamSetter<'_>, diff: &SnapshotDiff, target: &EditorSnapshot) {
    for &field in &diff.ints {
        let value = match field {
            IntField::Patch(patch_field) => read_int(&target.patch, patch_field),
            IntField::Fx(fx) => target.master.field(fx),
        };
        let param = int_param_ref(params, field);
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
    for &field in &diff.bools {
        let value = read_bool(&target.patch, field);
        let param = bool_param_ref(params, field);
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
    if !diff.egs.is_empty() {
        let mut egs = params.egs.write().expect("Poisoned RwLock on write");
        for &slot in &diff.egs {
            write_eg_bank_slot(&mut egs, slot, read_eg(&target.patch, slot));
        }
    }
}

/// `UndoStack::undo()`/`redo()`が返した内容を実際に反映する。差分適用（[`apply_snapshot_diff`]）＋
/// PRESETS選択状態の復元＋（あれば）バンク操作の逆適用、の3点セット。VSTは
/// `auto_save_bank_edits() == true`のままのため、バンク操作のUndo/Redoはファイルの保存も伴う
/// （`preset_host.rs`のdoc参照）。
fn apply_vst_undo(
    params: &Op505VstParams,
    setter: &ParamSetter<'_>,
    presets: &mut EditorPresetState,
    shared_bank: &Arc<RwLock<Op505PresetBank>>,
    dirty: &Arc<AtomicBool>,
    apply: UndoApply,
) {
    let current = vst_snapshot(params, presets);
    let diff = current.diff_to(&apply.snapshot);
    apply_snapshot_diff(params, setter, &diff, &apply.snapshot);
    presets.restore_selection(apply.snapshot.patch_name, apply.snapshot.bank, apply.snapshot.program, apply.snapshot.has_selection);
    if let Some(op) = &apply.bank_op {
        let host = VstPresetHost { params, setter, shared_bank, dirty };
        presets.apply_bank_op(op, &host);
    }
}

pub(crate) fn create_editor(
    egui_state: Arc<EguiState>,
    params: Arc<Op505VstParams>,
    shared_preset_bank: Arc<RwLock<Op505PresetBank>>,
    preset_bank_dirty: Arc<AtomicBool>,
    master_meter: Arc<MeterBridge>,
    meter_fps: u32,
) -> Option<Box<dyn Editor>> {
    let resize_state = egui_state.clone();
    create_egui_editor(
        egui_state,
        EditorState { presets: EditorPresetState::new(), undo: RefCell::new(UndoStack::new()) },
        EguiSettings::default(),
        |_ctx, _queue, _state| {},
        move |ui, setter, _queue, state| {
            let EditorState { presets, undo } = state;

            // レベルメーターを継続的に動かすため、egui既定のイベント駆動更新に加えて
            // `meter_fps`間隔での再描画を要求する（standaloneの`EditorApp::ui`と同じ設計）。
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32(1.0 / meter_fps as f32));

            undo.borrow_mut().begin_frame(vst_snapshot(&params, presets));

            // テキスト欄（音色名）にフォーカスがある間はeguiのTextEdit組み込みの文字単位Undoに委ね、
            // パッチ全体のUndo/Redoは発火させない（standaloneの`EditorApp::ui`と同じガード）。
            let any_text_focused = ui.memory(|m| m.focused().is_some());
            if !any_text_focused {
                let want_undo = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z));
                let want_redo = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Y));
                // Ctrl+Z/YはVST3ではホスト依存で届かないことがある（REAPER等がプロジェクトUndoとして
                // 先取りする）。PRESETSパネルのUndo/Redoボタンが確実な代替経路になる。
                if want_undo {
                    if let Some(apply) = undo.borrow_mut().undo() {
                        apply_vst_undo(&params, setter, presets, &shared_preset_bank, &preset_bank_dirty, apply);
                    }
                } else if want_redo {
                    if let Some(apply) = undo.borrow_mut().redo() {
                        apply_vst_undo(&params, setter, presets, &shared_preset_bank, &preset_bank_dirty, apply);
                    }
                }
            }

            ResizableWindow::new("op505_resize").min_size(editor_min_size(MIN_HEIGHT)).show(ui, &resize_state, |ui| {
                let host =
                    VstPresetHost { params: &params, setter, shared_bank: &shared_preset_bank, dirty: &preset_bank_dirty };

                // + New Voice/Delete（BankChange）はハンドルのbegin_edit/end_edit経由で記録できない
                // ため、パネル描画の前後でスナップショットを取り、bank_opが返ってきたときだけ
                // push_bank_changeで明示的に1エントリ積む（standaloneの`EditorApp::ui`と同じ設計）。
                let bank_change_before = vst_snapshot(&params, presets);
                let undo_ui = UndoUiState { can_undo: undo.borrow().can_undo(), can_redo: undo.borrow().can_redo() };

                let mut presets_events = egui::Panel::top("editor_top_bar")
                    .show_inside(ui, |ui| draw_editor_top_bar(ui, presets, &host, undo_ui, None, |_ui| {}))
                    .inner;

                // ---- 残りのパラメーター（縦スクロール、op505-uiの共有レイアウト） ----
                let central_response = egui::CentralPanel::default().show_inside(ui, |ui| {
                    let source = VstPanelSource { params: &params, setter, undo, master_meter: &master_meter };
                    let panel = build_panel_params(&source);
                    draw_op505_panel(ui, &panel);
                });

                // ---- PRESETSドロワー（ハンバーガー開閉のオーバーレイ、CentralPanelの残り領域へ重ねる） ----
                presets_events.merge(draw_presets_drawer(ui.ctx(), presets, &host, central_response.response.rect));
                presets_events.merge(poll_presets_events(presets, &host));

                if let Some(op) = presets_events.bank_op.clone() {
                    undo.borrow_mut().push_bank_change(op, bank_change_before, vst_snapshot(&params, presets));
                }
                if presets_events.history_cleared {
                    undo.borrow_mut().clear();
                }
                if presets_events.patch_name_focus_gained {
                    undo.borrow_mut().note_begin_edit();
                }
                if presets_events.patch_name_focus_lost {
                    undo.borrow_mut().note_end_edit();
                }
                if presets_events.list_selection_applied {
                    let mut u = undo.borrow_mut();
                    u.note_begin_edit();
                    u.note_end_edit();
                }
                if presets_events.undo_requested {
                    if let Some(apply) = undo.borrow_mut().undo() {
                        apply_vst_undo(&params, setter, presets, &shared_preset_bank, &preset_bank_dirty, apply);
                    }
                }
                if presets_events.redo_requested {
                    if let Some(apply) = undo.borrow_mut().redo() {
                        apply_vst_undo(&params, setter, presets, &shared_preset_bank, &preset_bank_dirty, apply);
                    }
                }
            });

            undo.borrow_mut().end_frame(vst_snapshot(&params, presets));
        },
    )
}
