//! トレイ起動音色エディタの`eframe::App`実装（Step 1、全サブステップ完了）。
//!
//! 「編集対象ch」セレクタ・op505-uiパネル（`draw_op505_panel`）・PRESETSサイドバー・
//! MASTER EFFECTSの4つを1画面にまとめる。値の変更はローカルの`dirty`フラグを立てるだけに
//! 留め、フレーム末尾でまとめて`SharedEditState`へpublishする（ドラッグ中に1サンプルごと
//! 書き込むと過負荷になるための1フレーム1回バッチ処理、gesture-app/op505-vstと同じ方針）。

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::keyboard::{self, KeyboardState};
use super::panel_params::{MasterEffectsState, Op505State};
use super::preset_host::StandalonePresetHost;
use crate::midi_source::MidiSink;
use crate::shared::SharedEditState;
use op505_editor::preset_panel::{draw_presets_panel, EditorPresetState};

/// PRESETSサイドバー（固定幅）の幅。op505-vstの`PRESETS_SIDEBAR_WIDTH`と揃える
/// （Open/Save/Save Asの3ボタンが折り返さず並ぶ幅）。
const PRESETS_SIDEBAR_WIDTH: f32 = 200.0;

pub struct EditorApp {
    shared: Arc<SharedEditState>,
    /// 鍵盤（`keyboard.rs`）が試聴用のNote On/Offを積む先。実際のMIDI入力と同じキューを共有する。
    midi_sink: MidiSink,
    op505: Op505State,
    master: Rc<std::cell::RefCell<MasterEffectsState>>,
    master_dirty: Rc<Cell<bool>>,
    presets: EditorPresetState,
    keyboard: KeyboardState,
    /// UIローカルの編集対象ch選択。`None`＝「(なし)」（既定、`SharedEditState::NO_EDIT_CHANNEL`と
    /// 対応）。ウィンドウを開いた直後に演奏中のチャンネルを勝手に上書きしないよう、既定は
    /// 必ず`None`にする（ユーザー確認済み、gesture-appコントローラー化ロードマップのplan参照）。
    edit_channel: Option<usize>,
}

impl EditorApp {
    /// `initial_patch`は`SharedEditState`が現在保持している値（起動直後は`default_patch`、
    /// 前回このエディタで編集した値が残っていればそれ）。エディタを開くたびにゼロから
    /// 組み立て直すのではなく、直前の状態を引き継ぐ。
    pub fn new(shared: Arc<SharedEditState>, midi_sink: MidiSink, initial_patch: op505_core::Op505Patch) -> Self {
        Self {
            shared,
            midi_sink,
            op505: Op505State::new(initial_patch),
            master: Rc::new(std::cell::RefCell::new(MasterEffectsState::default())),
            master_dirty: Rc::new(Cell::new(false)),
            presets: EditorPresetState::new(),
            keyboard: KeyboardState::new(),
            edit_channel: None,
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let previous_edit_channel = self.edit_channel;

        egui::Panel::top("editor_top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Edit Channel:");
                let selected_text = match self.edit_channel {
                    None => "(None)".to_string(),
                    Some(ch) => format!("{}", ch + 1),
                };
                egui::ComboBox::from_id_salt("edit_channel_combo").selected_text(selected_text).show_ui(ui, |ui| {
                    if ui.selectable_label(self.edit_channel.is_none(), "(None)").clicked() {
                        self.edit_channel = None;
                    }
                    for ch in 0..16usize {
                        let label = format!("{}", ch + 1);
                        if ui.selectable_label(self.edit_channel == Some(ch), label).clicked() {
                            self.edit_channel = Some(ch);
                        }
                    }
                });
                if self.edit_channel.is_some() {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 160, 40),
                        "Program Change is ignored on this channel while editing",
                    );
                }
            });
        });

        egui::Panel::left("presets_panel").resizable(false).exact_size(PRESETS_SIDEBAR_WIDTH).show_inside(ui, |ui| {
            let host = StandalonePresetHost { patch: &self.op505.patch, dirty: &self.op505.dirty, shared: &self.shared };
            draw_presets_panel(ui, &mut self.presets, &host);
        });

        egui::Panel::bottom("editor_keyboard").show_inside(ui, |ui| {
            keyboard::draw_keyboard(ui, &mut self.keyboard, &self.midi_sink, self.edit_channel);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("op505_editor_scroll").show(ui, |ui| {
                let panel = self.op505.build_panel_params(&self.master, &self.master_dirty);
                op505_ui::draw_op505_panel(ui, &panel);
            });
        });

        if self.edit_channel != previous_edit_channel {
            self.shared.set_edit_channel(self.edit_channel);
        }

        if self.op505.dirty.get() {
            self.op505.dirty.set(false);
            self.shared.publish_patch(*self.op505.patch.borrow());
        }

        if self.master_dirty.get() {
            self.master_dirty.set(false);
            let m = self.master.borrow();
            // フィールド順は`shared::FX_*`定数の並びと一致させること（`MasterEffectsState`の
            // フィールド宣言順そのまま）。編集対象は常にスロット0
            // （`op505_midi::EffectControlTarget`のNRPN(0,1) Channel Effect Routeが
            // 未設定のチャンネルが送る既定スロットと同じ。standaloneのエディタは
            // 「編集対象ch」を選ばない限りマルチスロットを意識させない設計のため、
            // スロット選択UIは設けない）。
            let values = [
                m.rev_send as u8,
                m.reverb_type as u8,
                m.reverb_time as u8,
                m.cho_send as u8,
                m.chorus_type as u8,
                m.chorus_mod_rate as u8,
                m.chorus_mod_depth as u8,
                m.chorus_feedback as u8,
                m.chorus_send_to_reverb as u8,
            ];
            self.shared.publish_fx(0, values);
        }
    }
}
