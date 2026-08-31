//! PRESETSパネル（Open/Save/Save As/+ New Voice/Delete）。`op505-vst/src/preset_panel.rs`の
//! 移植——gesture-appのレイアウト・操作意味論を`op505_core::preset_registry`へ直接委譲する方式は
//! 同じだが、VSTの`Op505VstParams`+`ParamSetter`（DAWパラメーター経由）の代わりに
//! `Rc<RefCell<Op505Patch>>`+`Rc<Cell<bool>>`（`panel_params::Op505State`と同じ表現）を使う。
//! standaloneはDAWパラメーターを持たないため、VSTの`build_patch`/`apply_patch`/`apply_patch_egs`は
//! 不要——「今のパッチをそのまま読み書きする」だけで足りる。
//!
//! `PresetSession`が保持する状態遷移ロジック（`entries`/`default_save_as_file_name`/
//! `save_enabled`/`select_after_delete`等）はVST版と1バイトも変えていない（fork-on-write方針の
//! 例外的措置——`op505-core::preset_registry`自体が共有クレートであるのと同じ理由。
//! 移植時の書き換え範囲は「DAWパラメーターとのやり取り」の3関数のみに閉じる）。

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

use op505_core::{
    build_op505_registry, current_open_dir, op505_presets_dir, Op505BankFile, Op505BankRegistry, Op505Patch, Op505PresetEntry, Op505PresetFile,
};

use crate::shared::SharedEditState;

/// PRESETSパネルが保持するセッション状態（レジストリ＋今編集中の(bank, program)＋表示用文字列）。
pub(crate) struct PresetSession {
    registry: Op505BankRegistry,
    bank: u16,
    program: u8,
    file_name: Option<String>,
    patch_name: String,
    unsaved: bool,
    pending_delete: Option<PendingDelete>,
    pending_open: Option<PendingOpen>,
    pending_save_as: Option<PendingSaveAs>,
    last_error: Option<(String, std::time::Instant)>,
    has_selection: bool,
}

const ERROR_DISPLAY_DURATION: std::time::Duration = std::time::Duration::from_secs(4);

struct PendingDelete {
    bank: u16,
    program: u8,
    receiver: mpsc::Receiver<bool>,
}

struct PendingOpen {
    bank: u16,
    receiver: mpsc::Receiver<Option<PathBuf>>,
}

struct PendingSaveAs {
    bank: u16,
    program: u8,
    patch_name: String,
    receiver: mpsc::Receiver<Option<PathBuf>>,
}

impl PresetSession {
    fn new(registry: Op505BankRegistry) -> Self {
        PresetSession {
            registry,
            bank: 0,
            program: 0,
            file_name: None,
            patch_name: String::new(),
            unsaved: false,
            pending_delete: None,
            pending_open: None,
            pending_save_as: None,
            last_error: None,
            has_selection: false,
        }
    }

    fn entries(&self) -> &[Op505PresetEntry] {
        self.registry.get(&self.bank).map(Op505BankFile::entries).unwrap_or(&[])
    }

    fn save_enabled(&self) -> bool {
        self.has_selection && self.file_name.is_some()
    }

    fn default_save_as_file_name(&self) -> String {
        self.file_name
            .as_deref()
            .and_then(|f| f.strip_suffix(".op505"))
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .or_else(|| (!self.patch_name.is_empty()).then(|| self.patch_name.clone()))
            .unwrap_or_else(|| "patch".to_string())
    }

    fn select_after_delete(&mut self, remaining: &[Op505PresetEntry]) -> Option<Op505PresetEntry> {
        let first = remaining.first()?.clone();
        self.program = first.program;
        self.patch_name = first.name.clone();
        self.unsaved = false;
        self.has_selection = true;
        Some(first)
    }

    fn select_entry(&mut self, program: u8, patch_name: String) {
        self.program = program;
        self.patch_name = patch_name;
        self.unsaved = false;
        self.has_selection = true;
    }

    fn open_file(&mut self, program: u8, patch_name: String, file_name: Option<String>) {
        self.program = program;
        self.patch_name = patch_name;
        self.file_name = file_name;
        self.unsaved = false;
        self.has_selection = true;
    }

    fn on_saved(&mut self, file_name: String) {
        self.file_name = Some(file_name);
        self.unsaved = false;
    }

    fn sync_display_to_registry(&mut self) {
        let Some(bank_file) = self.registry.get(&self.bank) else { return };
        let Some(entry) = bank_file.entries().first() else { return };
        self.program = entry.program;
        self.patch_name = entry.name.clone();
        self.file_name = bank_file.file_name().map(str::to_string);
        self.unsaved = false;
    }
}

/// `entry`を今のパッチへ反映する（PRESETSパネルの全操作が最後に通る共通処理）。`keep_fg`＝trueなら
/// Pitch/Cutoff/Gain FGは今の設定を保ったまま、それ以外だけ`entry.patch`の内容へ差し替える
/// （PRESETSリストのShift+クリック用、gesture-app/VSTと同じ意味論）。
fn apply_entry(patch: &Rc<RefCell<Op505Patch>>, dirty: &Rc<Cell<bool>>, entry: &Op505PresetEntry, keep_fg: bool) {
    let mut new_patch = entry.patch;
    if keep_fg {
        let current = *patch.borrow();
        new_patch.channel.pitch_fg = current.channel.pitch_fg;
        new_patch.channel.cutoff_fg = current.channel.cutoff_fg;
        new_patch.channel.gain_fg = current.channel.gain_fg;
    }
    *patch.borrow_mut() = new_patch;
    dirty.set(true);
}

fn request_open(session: &mut PresetSession) {
    if session.pending_open.is_some() {
        return;
    }
    let start_dir = current_open_dir(&session.registry, session.bank);
    let bank = session.bank;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let path = rfd::FileDialog::new().add_filter("op505", &["op505"]).set_directory(start_dir).pick_file();
        let _ = tx.send(path);
    });
    session.pending_open = Some(PendingOpen { bank, receiver: rx });
}

fn poll_pending_open(session: &mut PresetSession, patch: &Rc<RefCell<Op505Patch>>, dirty: &Rc<Cell<bool>>, shared: &Arc<SharedEditState>) {
    let path = match session.pending_open.as_ref() {
        Some(pending) => match pending.receiver.try_recv() {
            Ok(path) => path,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        },
        None => return,
    };
    let PendingOpen { bank, .. } = session.pending_open.take().expect("Some確認済み");
    let Some(path) = path else { return }; // ダイアログをキャンセルした
    let Ok(json) = std::fs::read_to_string(&path) else { return };
    let Ok(file) = Op505PresetFile::from_json(&json) else { return };
    let bank_file = Op505BankFile::from_loaded(path, file, bank);
    let Some(entry) = bank_file.entries().first().cloned() else { return };

    if bank == session.bank {
        apply_entry(patch, dirty, &entry, false);
        session.open_file(entry.program, entry.name, bank_file.file_name().map(str::to_string));
    }
    shared.publish_bank_file(&bank_file);
    session.registry.insert(bank, bank_file);
}

fn handle_save(session: &mut PresetSession, patch: &Rc<RefCell<Op505Patch>>, shared: &Arc<SharedEditState>) {
    let current = *patch.borrow();
    let program = session.program;
    let patch_name = session.patch_name.clone();
    let Some(bank_file) = session.registry.get_mut(&session.bank) else { return };
    if bank_file.upsert(program, patch_name, current).is_err() {
        return;
    }
    if bank_file.save().is_err() {
        return;
    }
    let file_name = bank_file.file_name().unwrap_or("?").to_string();
    shared.publish_bank_file(bank_file);
    session.on_saved(file_name);
}

fn request_save_as(session: &mut PresetSession) {
    if session.pending_save_as.is_some() {
        return;
    }
    let start_dir = current_open_dir(&session.registry, session.bank);
    let default_name = session.default_save_as_file_name();
    let (bank, program, patch_name) = (session.bank, session.program, session.patch_name.clone());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .add_filter("op505", &["op505"])
            .set_directory(start_dir)
            .set_file_name(format!("{default_name}.op505"))
            .save_file();
        let _ = tx.send(path);
    });
    session.pending_save_as = Some(PendingSaveAs { bank, program, patch_name, receiver: rx });
}

fn poll_pending_save_as(session: &mut PresetSession, patch: &Rc<RefCell<Op505Patch>>, shared: &Arc<SharedEditState>) {
    let path = match session.pending_save_as.as_ref() {
        Some(pending) => match pending.receiver.try_recv() {
            Ok(path) => path,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        },
        None => return,
    };
    let PendingSaveAs { bank, program, patch_name, .. } = session.pending_save_as.take().expect("Some確認済み");
    let Some(path) = path else { return }; // ダイアログをキャンセルした

    let current = *patch.borrow();
    let base_entries: Vec<Op505PresetEntry> = session.registry.get(&bank).map(|f| f.entries().to_vec()).unwrap_or_default();
    let Ok(bank_file) = Op505BankFile::write_as(current, patch_name.clone(), bank, program, &base_entries, path) else { return };

    let file_name = bank_file.file_name().unwrap_or("patch.op505").to_string();
    shared.publish_bank_file(&bank_file);
    session.registry.insert(bank, bank_file);
    if bank == session.bank {
        session.open_file(program, patch_name, Some(file_name));
    }
}

fn handle_add_new_voice(
    session: &mut PresetSession,
    patch: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    shared: &Arc<SharedEditState>,
    copy_current: bool,
) {
    let source_patch = if copy_current { *patch.borrow() } else { Op505Patch::default() };
    let Some(bank_file) = session.registry.get_mut(&session.bank) else {
        session.last_error =
            Some(("This bank has no file yet. Use Open or Save As first.".to_string(), std::time::Instant::now()));
        return;
    };
    let Ok(entry) = bank_file.add_new_voice(source_patch) else { return };
    if bank_file.save().is_err() {
        return;
    }
    session.last_error = None;
    shared.publish_bank_file(bank_file);
    apply_entry(patch, dirty, &entry, false);
    session.select_entry(entry.program, entry.name);
}

fn request_delete(session: &mut PresetSession) {
    if session.pending_delete.is_some() {
        return;
    }
    let Some(bank_file) = session.registry.get(&session.bank) else { return };
    let Some(name) = bank_file.entries().iter().find(|e| e.program == session.program).map(|e| e.name.clone()) else { return };

    let (bank, program) = (session.bank, session.program);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let confirmed = matches!(
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Delete Voice")
                .set_description(format!("Delete voice \"{:03} {}\"?", program, name))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show(),
            rfd::MessageDialogResult::Yes
        );
        let _ = tx.send(confirmed);
    });
    session.pending_delete = Some(PendingDelete { bank, program, receiver: rx });
}

fn poll_pending_delete(session: &mut PresetSession, patch: &Rc<RefCell<Op505Patch>>, dirty: &Rc<Cell<bool>>, shared: &Arc<SharedEditState>) {
    let confirmed = match session.pending_delete.as_ref() {
        Some(pending) => match pending.receiver.try_recv() {
            Ok(confirmed) => confirmed,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => false,
        },
        None => return,
    };
    let PendingDelete { bank, program, .. } = session.pending_delete.take().expect("Some確認済み");
    if !confirmed {
        return;
    }

    let Some(bank_file) = session.registry.get_mut(&bank) else { return };
    if bank_file.remove(program).is_err() {
        return;
    }
    if bank_file.save().is_err() {
        return;
    }
    let remaining = bank_file.entries().to_vec();
    shared.publish_bank_file(bank_file);

    if bank == session.bank {
        if let Some(next) = session.select_after_delete(&remaining) {
            apply_entry(patch, dirty, &next, false);
        }
    }
}

/// Bank欄のハンドル。ノブ下の数値欄と同じ見た目（`ui_core::spin_control`）にするための橋渡し。
struct BankFieldHandle {
    bank: Cell<i32>,
}

impl op505_ui::IntParamHandle for BankFieldHandle {
    fn value(&self) -> i32 {
        self.bank.get()
    }
    fn min(&self) -> i32 {
        0
    }
    fn max(&self) -> i32 {
        16383
    }
    fn default(&self) -> i32 {
        0
    }
    fn name(&self) -> String {
        "Bank".to_string()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        self.bank.set(value.clamp(0, 16383));
    }
    fn end_edit(&self) {}
}

fn handle_bank_changed(session: &mut PresetSession) {
    session.file_name = session.registry.get(&session.bank).and_then(Op505BankFile::file_name).map(str::to_string);
    session.program = 0;
    session.patch_name.clear();
    session.unsaved = false;
    session.has_selection = false;
}

/// PRESETSパネル本体を描画する。gesture-app/op505-vstと同じレイアウト（Open/Save/Save As→Bank→
/// ファイル名→音色名→区切り線→PRESETSリスト（+ New Voice/DeleteはScrollArea内）の順）。
/// `ScrollArea::auto_shrink([false,false])`は残り領域を全部占有するため、**ScrollAreaより後に
/// 置いたウィジェットは表示されない**点に注意（VST側と同じ罠）。
pub(crate) fn draw_presets_panel(
    ui: &mut egui::Ui,
    state: &mut EditorPresetState,
    patch: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    shared: &Arc<SharedEditState>,
) {
    let session = &mut state.session;

    ui.horizontal(|ui| {
        if ui.add_enabled(session.pending_open.is_none(), egui::Button::new("Open")).clicked() {
            request_open(session);
        }
        if ui.add_enabled(session.save_enabled(), egui::Button::new("Save")).clicked() {
            handle_save(session, patch, shared);
        }
        if ui.add_enabled(session.pending_save_as.is_none(), egui::Button::new("Save As")).clicked() {
            request_save_as(session);
        }
    });

    ui.horizontal(|ui| {
        ui.label("Bank");
        let bank_handle = BankFieldHandle { bank: Cell::new(session.bank as i32) };
        ui_core::spin_control(ui, &bank_handle, egui::TextStyle::Body, 44.0);
        let new_bank = bank_handle.bank.get() as u16;
        if new_bank != session.bank {
            session.bank = new_bank;
            handle_bank_changed(session);
        }
    });

    let file_label = session.file_name.clone().unwrap_or_else(|| "(unsaved)".to_string());
    let mark = if session.unsaved { "*" } else { "" };
    ui.label(format!("{file_label}{mark}"));

    let mut patch_name = session.patch_name.clone();
    if ui.text_edit_singleline(&mut patch_name).changed() {
        session.patch_name = patch_name;
        session.unsaved = true;
    }
    ui.separator();

    if let Some((msg, shown_at)) = &session.last_error {
        let elapsed = shown_at.elapsed();
        if elapsed < ERROR_DISPLAY_DURATION {
            ui.colored_label(egui::Color32::from_rgb(220, 90, 90), msg);
            ui.ctx().request_repaint_after(ERROR_DISPLAY_DURATION - elapsed);
        } else {
            session.last_error = None;
        }
    }

    let any_text_focused = ui.memory(|m| m.focused().is_some());
    let has_entries = !session.entries().is_empty();
    let delete_enabled = has_entries && session.pending_delete.is_none();
    let delete_by_key = !any_text_focused && delete_enabled && ui.input(|i| i.key_pressed(egui::Key::Delete));

    ui.label(egui::RichText::new("PRESETS").strong());
    egui::ScrollArea::vertical().id_salt("presets").auto_shrink([false, false]).show(ui, |ui| {
        for entry in session.entries().to_vec() {
            let label = format!("{:03} {}", entry.program, entry.name);
            let selected = session.has_selection && entry.program == session.program;
            if ui.selectable_label(selected, &label).clicked() {
                let keep_fg = ui.input(|i| i.modifiers.shift);
                apply_entry(patch, dirty, &entry, keep_fg);
                session.select_entry(entry.program, entry.name);
            }
        }
        if ui.selectable_label(false, "+ New Voice").clicked() {
            let copy_current = ui.input(|i| i.modifiers.shift);
            handle_add_new_voice(session, patch, dirty, shared, copy_current);
        }
    });

    if delete_by_key {
        request_delete(session);
    }
    poll_pending_delete(session, patch, dirty, shared);
    poll_pending_open(session, patch, dirty, shared);
    poll_pending_save_as(session, patch, shared);
}

/// エディタが持つPRESETSパネル分の状態。エディタ生成時に一度だけレジストリを構築する。
pub(crate) struct EditorPresetState {
    session: PresetSession,
}

impl EditorPresetState {
    pub(crate) fn new() -> Self {
        let mut session = PresetSession::new(build_op505_registry(&op505_presets_dir()));
        session.sync_display_to_registry();
        EditorPresetState { session }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use op505_core::Op505BankRegistry;
    use std::path::PathBuf;

    fn entry(program: u8, name: &str) -> Op505PresetEntry {
        Op505PresetEntry { program, name: name.to_string(), patch: Op505Patch::default() }
    }

    #[test]
    fn save_enabled_reflects_file_name() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        assert!(!session.save_enabled(), "file_name未設定ならSave不可");
        session.select_entry(0, "Voice".to_string());
        session.on_saved("a.op505".to_string());
        assert!(session.save_enabled());
    }

    #[test]
    fn save_enabled_is_false_after_bank_changed_even_with_file_name() {
        let mut registry = Op505BankRegistry::new();
        let file = Op505PresetFile::Presets { bank: 1, presets: vec![entry(0, "Existing")] };
        registry.insert(1, Op505BankFile::from_loaded(PathBuf::from("bank1.op505"), file, 1));

        let mut session = PresetSession::new(registry);
        session.select_entry(0, "Voice".to_string());
        session.on_saved("a.op505".to_string());
        assert!(session.save_enabled(), "選択済みならSave可");

        session.bank = 1;
        handle_bank_changed(&mut session);
        assert!(!session.save_enabled(), "バンク切り替え直後の未選択状態ではSave不可のはず");
    }

    #[test]
    fn default_save_as_file_name_prefers_current_file_name() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        session.patch_name = "MyVoice".to_string();
        assert_eq!(session.default_save_as_file_name(), "MyVoice", "ファイル名未設定なら音色名");

        session.open_file(0, "MyVoice".to_string(), Some("organ_family.op505".to_string()));
        assert_eq!(session.default_save_as_file_name(), "organ_family", "ファイル名優先・拡張子除去");
    }

    #[test]
    fn default_save_as_file_name_falls_back_to_patch_when_no_extension() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        session.patch_name = String::new();
        assert_eq!(session.default_save_as_file_name(), "patch", "ファイル名・音色名とも空なら\"patch\"");
    }

    #[test]
    fn select_after_delete_picks_first_remaining_entry() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        let remaining = vec![entry(1, "A"), entry(2, "B")];
        let selected = session.select_after_delete(&remaining).expect("残りがあれば選ばれるはず");
        assert_eq!(selected.program, 1);
        assert_eq!(session.program, 1);
        assert_eq!(session.patch_name, "A");
        assert!(!session.unsaved);
    }

    #[test]
    fn select_after_delete_keeps_state_when_nothing_remains() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        session.select_entry(9, "Kept".to_string());
        assert!(session.select_after_delete(&[]).is_none(), "残りが無ければNoneを返すはず");
        assert_eq!(session.program, 9, "現状維持されるはず");
        assert_eq!(session.patch_name, "Kept");
    }

    #[test]
    fn entries_are_scoped_to_current_bank_and_empty_when_unregistered() {
        let session = PresetSession::new(Op505BankRegistry::new());
        assert!(session.entries().is_empty(), "未登録bankは空のはず");
    }

    #[test]
    fn on_saved_clears_unsaved_and_sets_file_name() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        session.unsaved = true;
        session.on_saved("saved.op505".to_string());
        assert!(!session.unsaved);
        assert_eq!(session.file_name.as_deref(), Some("saved.op505"));
    }

    #[test]
    fn select_entry_does_not_touch_file_name() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        session.open_file(0, "A".to_string(), Some("bank.op505".to_string()));
        session.select_entry(1, "B".to_string());
        assert_eq!(session.file_name.as_deref(), Some("bank.op505"), "同じファイル内の選択切り替えではfile_nameは不変のはず");
        assert_eq!(session.program, 1);
    }

    #[test]
    fn sync_display_to_registry_shows_file_name_without_touching_params() {
        let mut registry = Op505BankRegistry::new();
        let file = Op505PresetFile::Presets { bank: 0, presets: vec![entry(3, "Organ")] };
        let bank_file = Op505BankFile::from_loaded(PathBuf::from("organ.op505"), file, 0);
        registry.insert(0, bank_file);

        let mut session = PresetSession::new(registry);
        session.sync_display_to_registry();

        assert_eq!(session.file_name.as_deref(), Some("organ.op505"), "先頭エントリーの担当ファイル名を表示するはず");
        assert_eq!(session.patch_name, "Organ");
        assert_eq!(session.program, 3, "先頭エントリーのprogramへ合わせるはず（apply_entryは呼ばない）");
        assert!(!session.unsaved);
        assert!(!session.has_selection, "has_selectionはfalseのままのはず（選択状態を見せない方針）");
    }

    #[test]
    fn sync_display_to_registry_is_noop_when_bank_unregistered() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        session.sync_display_to_registry();
        assert_eq!(session.file_name, None, "担当ファイルが無ければ(unsaved)表示のままのはず");
        assert_eq!(session.program, 0);
    }

    #[test]
    fn select_entry_sets_has_selection() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        assert!(!session.has_selection, "初期状態はfalseのはず");
        session.select_entry(1, "B".to_string());
        assert!(session.has_selection, "実際にエントリーを選ぶとtrueになるはず");
    }

    #[test]
    fn handle_bank_changed_does_not_carry_over_selection_even_when_program_matches() {
        let mut registry = Op505BankRegistry::new();
        let file0 = Op505PresetFile::Presets { bank: 0, presets: vec![entry(5, "OldVoice")] };
        registry.insert(0, Op505BankFile::from_loaded(PathBuf::from("bank0.op505"), file0, 0));
        let file1 = Op505PresetFile::Presets { bank: 1, presets: vec![entry(5, "SameNumberDifferentVoice")] };
        registry.insert(1, Op505BankFile::from_loaded(PathBuf::from("bank1.op505"), file1, 1));

        let mut session = PresetSession::new(registry);
        session.bank = 0;
        session.select_entry(5, "OldVoice".to_string());
        assert!(session.has_selection);

        session.bank = 1;
        handle_bank_changed(&mut session);

        assert!(!session.has_selection, "バンクを跨いだら選択状態を持たないはず");
        assert_eq!(session.program, 0, "選択を持たない状態へリセットされるはず");
        assert_eq!(session.patch_name, "", "選択を持たない状態では音色名表示も空になるはず");
        assert_eq!(session.file_name.as_deref(), Some("bank1.op505"), "新バンクの担当ファイル名は表示するはず");
    }

    #[test]
    fn handle_bank_changed_clears_state_when_new_bank_has_no_file() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        session.select_entry(5, "Voice".to_string());
        session.bank = 99;

        handle_bank_changed(&mut session);

        assert!(!session.has_selection);
        assert_eq!(session.program, 0);
        assert_eq!(session.patch_name, "");
        assert_eq!(session.file_name, None);
    }
}
