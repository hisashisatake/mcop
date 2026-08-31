//! トレイ起動音色エディタの`eframe::App`実装（Step 1サブステップ3）。
//!
//! PRESETSパネル（Open/Save/Save As/+New Voice/Delete）とMASTER EFFECTSの実配線は
//! 後続のサブステップで追加する。本サブステップでは「パネル表示」「編集対象chセレクタ」
//! 「パッチのpublish」の3点のみを扱う。MASTER EFFECTSのノブ自体は`draw_op505_panel`が
//! 要求する`Op505PanelParams`の一部として表示はされるが、値を動かしてもまだ音には
//! 反映されない（`SharedEditState`のFX APIへはまだ配線していない）。

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::panel_params::{MasterEffectsState, Op505State};
use crate::shared::SharedEditState;

pub struct EditorApp {
    shared: Arc<SharedEditState>,
    op505: Op505State,
    master: Rc<std::cell::RefCell<MasterEffectsState>>,
    master_dirty: Rc<Cell<bool>>,
    /// UIローカルの編集対象ch選択。`None`＝「(なし)」（既定、`SharedEditState::NO_EDIT_CHANNEL`と
    /// 対応）。ウィンドウを開いた直後に演奏中のチャンネルを勝手に上書きしないよう、既定は
    /// 必ず`None`にする（ユーザー確認済み、gesture-appコントローラー化ロードマップのplan参照）。
    edit_channel: Option<usize>,
}

impl EditorApp {
    /// `initial_patch`は`SharedEditState`が現在保持している値（起動直後は`default_patch`、
    /// 前回このエディタで編集した値が残っていればそれ）。エディタを開くたびにゼロから
    /// 組み立て直すのではなく、直前の状態を引き継ぐ。
    pub fn new(shared: Arc<SharedEditState>, initial_patch: op505_core::Op505Patch) -> Self {
        Self {
            shared,
            op505: Op505State::new(initial_patch),
            master: Rc::new(std::cell::RefCell::new(MasterEffectsState::default())),
            master_dirty: Rc::new(Cell::new(false)),
            edit_channel: None,
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let previous_edit_channel = self.edit_channel;

        egui::Panel::top("editor_top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("編集対象ch:");
                let selected_text = match self.edit_channel {
                    None => "(なし)".to_string(),
                    Some(ch) => format!("{}", ch + 1),
                };
                egui::ComboBox::from_id_salt("edit_channel_combo").selected_text(selected_text).show_ui(ui, |ui| {
                    if ui.selectable_label(self.edit_channel.is_none(), "(なし)").clicked() {
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
                        "編集中はこのチャンネルのProgram Changeが無視されます",
                    );
                }
            });
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
    }
}
