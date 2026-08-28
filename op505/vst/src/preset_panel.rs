//! PRESETSパネル（Open/Save/Save As/+ New Voice/Delete）。gesture-appの
//! `editor-wasm/src/app.rs`（271〜394行目）と同じレイアウト・操作意味論を、Tauri IPCの代わりに
//! `op505_core::preset_registry`を直接呼ぶ同期処理として実装する（VSTはIPCを介さず同一プロセス内
//! で完結するため）。
//!
//! `PresetSession`が保持する状態遷移ロジック（`entries`/`default_save_as_file_name`/
//! `save_enabled`/`select_after_delete`等）は純粋関数寄りに保ち`#[cfg(test)]`で検証する。
//! 描画関数`draw_presets_panel`とダイアログ呼び出しはユニットテスト不能なUIコードのため、
//! それらを呼ぶだけに留める。

use nice_plug::prelude::*;
use op505_core::{
    build_op505_registry, current_open_dir, op505_presets_dir, Op505BankFile, Op505BankRegistry, Op505Patch, Op505PresetBank, Op505PresetEntry, Op505PresetFile,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, RwLock};

use crate::params::{apply_patch, apply_patch_egs, build_patch, Op505VstParams};

/// PRESETSパネルが保持するセッション状態（レジストリ＋今編集中の(bank, program)＋表示用文字列）。
pub(crate) struct PresetSession {
    registry: Op505BankRegistry,
    bank: u16,
    program: u8,
    file_name: Option<String>,
    patch_name: String,
    /// 名前欄の編集を検知するための簡易マーカー。ノブ操作等パッチ全体の変更までは追跡しない
    /// （毎フレーム`build_patch`とのdiffを取る必要がありコスト・複雑さに見合わないため、
    /// 「音色名を編集した」ことだけを検知する簡易版に留める）。
    unsaved: bool,
    /// Delete確認ダイアログの結果待ち（`request_delete`/`poll_pending_delete`参照）。
    pending_delete: Option<PendingDelete>,
}

/// Delete確認中に確認先を固定するための情報。ダイアログ表示中にBank欄が操作された場合でも
/// 削除対象がずれないよう、リクエスト時点の(bank, program)を保持する。
struct PendingDelete {
    bank: u16,
    program: u8,
    receiver: mpsc::Receiver<bool>,
}

impl PresetSession {
    /// 起動直後は何も選択しない（プロジェクトを開いただけでDAWパラメーターが書き換わる事故を
    /// 避ける、既存の「何も選ばない」挙動を踏襲）。
    fn new(registry: Op505BankRegistry) -> Self {
        PresetSession { registry, bank: 0, program: 0, file_name: None, patch_name: String::new(), unsaved: false, pending_delete: None }
    }

    /// 今のbankの担当ファイルが持つ音色一覧（未登録なら空）。
    fn entries(&self) -> &[Op505PresetEntry] {
        self.registry.get(&self.bank).map(Op505BankFile::entries).unwrap_or(&[])
    }

    /// Saveボタンを有効化してよいか（担当ファイルが無いbankでは保存できない）。
    fn save_enabled(&self) -> bool {
        self.file_name.is_some()
    }

    /// Save Asダイアログの初期提案ファイル名。「今開いているファイル名」→無ければ音色名→
    /// それも空なら"patch"の3段フォールバック（gesture-app `app.rs:301-309`と同じ規則）。
    fn default_save_as_file_name(&self) -> String {
        self.file_name
            .as_deref()
            .and_then(|f| f.strip_suffix(".op505"))
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .or_else(|| (!self.patch_name.is_empty()).then(|| self.patch_name.clone()))
            .unwrap_or_else(|| "patch".to_string())
    }

    /// 削除後の選択を決める：残っていれば先頭のエントリーへ切り替え、空なら現状維持
    /// （gesture-app `app.rs:120-129`と同じ規則）。切り替え先を返すので、呼び出し側が
    /// DAWパラメーターへの反映（`apply_entry`）を行う。
    fn select_after_delete(&mut self, remaining: &[Op505PresetEntry]) -> Option<Op505PresetEntry> {
        let first = remaining.first()?.clone();
        self.program = first.program;
        self.patch_name = first.name.clone();
        self.unsaved = false;
        Some(first)
    }

    /// 同じbank内でエントリーを選び直す（PRESETSリストのクリック、+ New Voice追加直後、
    /// 削除後の自動選択で使う）。file_name/bankは変わらない。
    fn select_entry(&mut self, program: u8, patch_name: String) {
        self.program = program;
        self.patch_name = patch_name;
        self.unsaved = false;
    }

    /// 別ファイルを読み込んだ（Open）、または別ファイルへ書き出した（Save As）ときに呼ぶ。
    fn open_file(&mut self, program: u8, patch_name: String, file_name: Option<String>) {
        self.program = program;
        self.patch_name = patch_name;
        self.file_name = file_name;
        self.unsaved = false;
    }

    /// 現在の担当ファイルへ上書き保存した（Save）ときに呼ぶ。program/patch_nameは変わらない。
    fn on_saved(&mut self, file_name: String) {
        self.file_name = Some(file_name);
        self.unsaved = false;
    }
}

/// `entry`をDAWパラメーター＋TimeEg 7本へ反映する（PRESETSパネルの全操作が最後に通る共通処理）。
/// `keep_fg`＝trueなら、Pitch/Cutoff/Gain FG（EG本体＋Depth）は今の設定を保ったまま、それ以外の
/// パラメーターだけ`entry.patch`の内容へ差し替える（PRESETSリストのShift+クリック用）。
fn apply_entry(params: &Op505VstParams, setter: &ParamSetter<'_>, entry: &Op505PresetEntry, keep_fg: bool) {
    let mut patch = entry.patch;
    if keep_fg {
        let current = build_patch(params, &params.egs.read().expect("Poisoned RwLock on read"));
        patch.channel.pitch_fg = current.channel.pitch_fg;
        patch.channel.cutoff_fg = current.channel.cutoff_fg;
        patch.channel.gain_fg = current.channel.gain_fg;
    }
    apply_patch(params, setter, &patch);
    let mut egs = params.egs.write().expect("Poisoned RwLock on write");
    apply_patch_egs(&mut egs, &patch);
}

/// 保存操作の結果を、音声スレッド（MIDI Program Change解決）へ即座に反映する。GUIはblocking
/// writeで良い（`params.egs.write()`と同じ土俵）。書き込み完了後にdirtyを立てる順序が重要
/// （先に立てるとオーディオスレッドが未反映のバンクを読みに行く隙が生まれる）。
fn publish_bank(shared: &Arc<RwLock<Op505PresetBank>>, dirty: &Arc<AtomicBool>, bank_file: &Op505BankFile) {
    shared.write().expect("Poisoned RwLock on write").merge_file(bank_file.as_presets_file());
    dirty.store(true, Ordering::Release);
}

fn handle_open(session: &mut PresetSession, params: &Op505VstParams, setter: &ParamSetter<'_>, shared: &Arc<RwLock<Op505PresetBank>>, dirty: &Arc<AtomicBool>) {
    let start_dir = current_open_dir(&session.registry, session.bank);
    let Some(path) = rfd::FileDialog::new().add_filter("op505", &["op505"]).set_directory(start_dir).pick_file() else { return };
    let Ok(json) = std::fs::read_to_string(&path) else { return };
    let Ok(file) = Op505PresetFile::from_json(&json) else { return };
    // ファイル自身が宣言しているbank番号は無視し、今選択中のbankへ丸ごとロードする
    // （gesture-appと同じ、ユーザー確認済みの既定仕様）。
    let bank_file = Op505BankFile::from_loaded(path, file, session.bank);
    let Some(entry) = bank_file.entries().first().cloned() else { return };

    apply_entry(params, setter, &entry, false);
    publish_bank(shared, dirty, &bank_file);
    session.open_file(entry.program, entry.name, bank_file.file_name().map(str::to_string));
    session.registry.insert(session.bank, bank_file);
}

fn handle_save(session: &mut PresetSession, params: &Op505VstParams, shared: &Arc<RwLock<Op505PresetBank>>, dirty: &Arc<AtomicBool>) {
    let patch = build_patch(params, &params.egs.read().expect("Poisoned RwLock on read"));
    let program = session.program;
    let patch_name = session.patch_name.clone();
    let Some(bank_file) = session.registry.get_mut(&session.bank) else { return };
    if bank_file.upsert(program, patch_name, patch).is_err() {
        return;
    }
    if bank_file.save().is_err() {
        return;
    }
    let file_name = bank_file.file_name().unwrap_or("?").to_string();
    publish_bank(shared, dirty, bank_file);
    session.on_saved(file_name);
}

fn handle_save_as(session: &mut PresetSession, params: &Op505VstParams, shared: &Arc<RwLock<Op505PresetBank>>, dirty: &Arc<AtomicBool>) {
    let start_dir = current_open_dir(&session.registry, session.bank);
    let default_name = session.default_save_as_file_name();
    let Some(path) = rfd::FileDialog::new()
        .add_filter("op505", &["op505"])
        .set_directory(start_dir)
        .set_file_name(format!("{default_name}.op505"))
        .save_file()
    else {
        return;
    };

    let patch = build_patch(params, &params.egs.read().expect("Poisoned RwLock on read"));
    let base_entries: Vec<Op505PresetEntry> = session.registry.get(&session.bank).map(|f| f.entries().to_vec()).unwrap_or_default();
    let patch_name = session.patch_name.clone();
    let (bank, program) = (session.bank, session.program);
    let Ok(bank_file) = Op505BankFile::write_as(patch, patch_name.clone(), bank, program, &base_entries, path) else { return };

    let file_name = bank_file.file_name().unwrap_or("patch.op505").to_string();
    publish_bank(shared, dirty, &bank_file);
    session.registry.insert(bank, bank_file);
    session.open_file(program, patch_name, Some(file_name));
}

fn handle_add_new_voice(
    session: &mut PresetSession,
    params: &Op505VstParams,
    setter: &ParamSetter<'_>,
    shared: &Arc<RwLock<Op505PresetBank>>,
    dirty: &Arc<AtomicBool>,
    copy_current: bool,
) {
    let source_patch =
        if copy_current { build_patch(params, &params.egs.read().expect("Poisoned RwLock on read")) } else { Op505Patch::default() };
    let Some(bank_file) = session.registry.get_mut(&session.bank) else { return };
    let Ok(entry) = bank_file.add_new_voice(source_patch) else { return };
    if bank_file.save().is_err() {
        return;
    }
    publish_bank(shared, dirty, bank_file);
    apply_entry(params, setter, &entry, false);
    session.select_entry(entry.program, entry.name);
}

/// Delete確認ダイアログの表示を要求する。`rfd::MessageDialog::show()`はブロッキング呼び出しで、
/// egui描画コールバック（＝REAPERのウィンドウプロシージャから呼ばれている最中）の内側で直接
/// 呼ぶとダイアログが回すネストしたメッセージループがプラグインウィンドウへ再入し、REAPERごと
/// クラッシュする（実機確認で確認済み・2026-08-28、例外コード0xC0000409）。そのため表示自体は
/// 別スレッドへ逃がし、結果は`poll_pending_delete`が毎フレームpollする。
fn request_delete(session: &mut PresetSession) {
    if session.pending_delete.is_some() {
        return; // 確認中の二重リクエストを防ぐ（ボタン側もdisabledにしているが念のため）
    }
    let Some(bank_file) = session.registry.get(&session.bank) else { return };
    let Some(name) = bank_file.entries().iter().find(|e| e.program == session.program).map(|e| e.name.clone()) else { return };

    let (bank, program) = (session.bank, session.program);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let confirmed = matches!(
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("音色の削除")
                .set_description(format!("音色「{:03} {}」を削除しますか？", program, name))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show(),
            rfd::MessageDialogResult::Yes
        );
        let _ = tx.send(confirmed);
    });
    session.pending_delete = Some(PendingDelete { bank, program, receiver: rx });
}

/// 毎フレーム呼ぶ。確認スレッドの結果が届いていれば削除を実行する（結果が未到着ならno-op）。
fn poll_pending_delete(session: &mut PresetSession, params: &Op505VstParams, setter: &ParamSetter<'_>, shared: &Arc<RwLock<Op505PresetBank>>, dirty: &Arc<AtomicBool>) {
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
    publish_bank(shared, dirty, bank_file);

    // ダイアログ表示中にBankが切り替えられていた場合、削除対象はもう画面に見えていないので
    // DAWパラメーターへの反映（現在選択への追従）は行わない。
    if bank == session.bank {
        if let Some(next) = session.select_after_delete(&remaining) {
            apply_entry(params, setter, &next, false);
        }
    }
}

/// Bank欄の数値が変わったときの再解決：担当ファイルの中に今のprogramがあればそれを、
/// 無ければ先頭エントリーを選ぶ（gesture-appの`handle_navigate`相当。ディスク再検索はしない、
/// レジストリを引くだけ）。担当ファイルが無いbankへ移動した場合は選択を持たない。
fn handle_bank_changed(session: &mut PresetSession, params: &Op505VstParams, setter: &ParamSetter<'_>) {
    let Some(bank_file) = session.registry.get(&session.bank) else {
        session.program = 0;
        session.patch_name.clear();
        session.file_name = None;
        session.unsaved = false;
        return;
    };
    let file_name = bank_file.file_name().map(str::to_string);
    let entry = bank_file.entries().iter().find(|e| e.program == session.program).or_else(|| bank_file.entries().first()).cloned();
    match entry {
        Some(entry) => {
            apply_entry(params, setter, &entry, false);
            session.open_file(entry.program, entry.name, file_name);
        }
        None => {
            session.file_name = file_name;
            session.unsaved = false;
        }
    }
}

/// PRESETSパネル本体を描画する。gesture-appのレイアウト（Open/Save/Save As→Bank→ファイル名→
/// 音色名→区切り線→PRESETSリスト（+ New Voice/DeleteはScrollArea内）の順）をそのまま踏襲する。
/// `ScrollArea::auto_shrink([false,false])`は残り領域を全部占有するため、**ScrollAreaより後に
/// 置いたウィジェットは表示されない**——ここより下に新しいウィジェットを足す場合は必ずScrollArea
/// の中に置くこと（memory `project_preset_list_scrollbar_and_add_delete`参照）。
pub(crate) fn draw_presets_panel(
    ui: &mut egui::Ui,
    state: &mut EditorPresetState,
    params: &Op505VstParams,
    setter: &ParamSetter<'_>,
    shared_preset_bank: &Arc<RwLock<Op505PresetBank>>,
    preset_bank_dirty: &Arc<AtomicBool>,
) {
    let session = &mut state.session;

    ui.horizontal(|ui| {
        if ui.button("Open").clicked() {
            handle_open(session, params, setter, shared_preset_bank, preset_bank_dirty);
        }
        if ui.add_enabled(session.save_enabled(), egui::Button::new("Save")).clicked() {
            handle_save(session, params, shared_preset_bank, preset_bank_dirty);
        }
        if ui.button("Save As").clicked() {
            handle_save_as(session, params, shared_preset_bank, preset_bank_dirty);
        }
    });

    ui.horizontal(|ui| {
        ui.label("Bank");
        let mut bank_i32 = session.bank as i32;
        if ui.add(egui::DragValue::new(&mut bank_i32).range(0..=16383).speed(1.0)).changed() {
            session.bank = bank_i32 as u16;
            handle_bank_changed(session, params, setter);
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

    // Deleteキー（テキスト入力中は無効化）。DAWがキーを奪う場合の保険としてリスト末尾に
    // Deleteボタンも置く（下記ScrollArea内）。
    let any_text_focused = ui.memory(|m| m.focused().is_some());
    let has_entries = !session.entries().is_empty();
    let delete_enabled = has_entries && session.pending_delete.is_none();
    let delete_by_key = !any_text_focused && delete_enabled && ui.input(|i| i.key_pressed(egui::Key::Delete));

    ui.label(egui::RichText::new("PRESETS").strong());
    let mut delete_by_button = false;
    egui::ScrollArea::vertical().id_salt("presets").auto_shrink([false, false]).show(ui, |ui| {
        for entry in session.entries().to_vec() {
            let label = format!("{:03} {}", entry.program, entry.name);
            let selected = entry.program == session.program;
            if ui.selectable_label(selected, &label).clicked() {
                let keep_fg = ui.input(|i| i.modifiers.shift);
                apply_entry(params, setter, &entry, keep_fg);
                session.select_entry(entry.program, entry.name);
            }
        }
        if ui.selectable_label(false, "+ New Voice").clicked() {
            let copy_current = ui.input(|i| i.modifiers.shift);
            handle_add_new_voice(session, params, setter, shared_preset_bank, preset_bank_dirty, copy_current);
        }
        if ui.add_enabled(delete_enabled, egui::Button::new("Delete")).clicked() {
            delete_by_button = true;
        }
    });

    if delete_by_key || delete_by_button {
        request_delete(session);
    }
    poll_pending_delete(session, params, setter, shared_preset_bank, preset_bank_dirty);
}

/// `EditorState`が持つPRESETSパネル分の状態。エディタ生成時に一度だけレジストリを構築する。
pub(crate) struct EditorPresetState {
    session: PresetSession,
}

impl EditorPresetState {
    pub(crate) fn new() -> Self {
        EditorPresetState { session: PresetSession::new(build_op505_registry(&op505_presets_dir())) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(program: u8, name: &str) -> Op505PresetEntry {
        Op505PresetEntry { program, name: name.to_string(), patch: Op505Patch::default() }
    }

    #[test]
    fn save_enabled_reflects_file_name() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        assert!(!session.save_enabled(), "file_name未設定ならSave不可");
        session.on_saved("a.op505".to_string());
        assert!(session.save_enabled());
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
}
