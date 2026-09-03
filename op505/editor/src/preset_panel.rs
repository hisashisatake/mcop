//! PRESETSパネル（Open/Save/Save As/+ New Voice/Delete）。gesture-appの
//! `editor-wasm/src/app.rs`（271〜394行目）と同じレイアウト・操作意味論を、
//! `op505_core::preset_registry`を直接呼ぶ同期処理として実装する。
//!
//! ホスト差分（DAWパラメーター経由か`Rc<RefCell<Op505Patch>>`直接かなど）は[`PresetHost`]の
//! 3操作へ吸収する。`op505-vst`と`op505-standalone`はこの1ファイルを共有する
//! （`.claude/plans/fancy-wishing-toast.md`参照）。
//!
//! `PresetSession`が保持する状態遷移ロジック（`entries`/`default_save_as_file_name`/
//! `save_enabled`/`select_after_delete`等）は純粋関数寄りに保ち`#[cfg(test)]`で検証する。
//! 描画関数`draw_presets_panel`とダイアログ呼び出しはユニットテスト不能なUIコードのため、
//! それらを呼ぶだけに留める。

use op505_core::{
    build_op505_registry, current_open_dir, op505_presets_dir, Op505BankFile, Op505BankRegistry, Op505Patch, Op505PresetEntry, Op505PresetFile,
};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::layout::PRESETS_SIDEBAR_WIDTH;
use crate::undo::BankOp;

/// ホスト差分（DAWパラメーター経由か、`Rc<RefCell<Op505Patch>>`直接かなど）を吸収する3操作。
///
/// # 規約
/// [`current_patch`](Self::current_patch)の戻り値を作った時点で、実装内部のロック/borrowを
/// 解放していること（呼び出し側がその直後に別の操作で同じロック/borrowを取り直す可能性があるため、
/// 値を返した後もロック/borrowを保持したままにしてはならない）。
pub trait PresetHost {
    /// 「今鳴っている音」。
    fn current_patch(&self) -> Op505Patch;
    /// `patch`を丸ごと今の音として反映する。
    fn apply_patch(&self, patch: &Op505Patch);
    /// `bank_file`の内容を音声スレッド（MIDI Program Change解決）側の共有バンクへ即時反映する。
    fn publish_bank(&self, bank_file: &Op505BankFile);
    /// バンク構成の変更（+ New Voice/Delete）を即座にディスクへ保存するか。
    /// VST=true（即save維持のままUndo対応。`apply_bank_op`が逆操作の適用後に再saveするため、
    /// Undo/RedoでバンクファイルもUndoスタックの内容に追従して巻き戻る。VSTはウィンドウclose
    /// を横取りできず未保存確認ダイアログを出せないため、standaloneのような遅延保存化はしない）。
    /// standalone=false（Save/Save Asでのみ保存。ウィンドウclose時に未保存確認ダイアログを
    /// 挟めるため、Undo未確定のバンク変更をディスクへ書く前に確認できる）。
    fn auto_save_bank_edits(&self) -> bool {
        true
    }
}

/// PRESETSパネルが保持するセッション状態（レジストリ＋今編集中の(bank, program)＋表示用文字列）。
pub struct PresetSession {
    registry: Op505BankRegistry,
    bank: u16,
    program: u8,
    file_name: Option<String>,
    patch_name: String,
    /// 名前欄の編集を検知するための簡易マーカー。ノブ操作等パッチ全体の変更までは追跡しない
    /// （毎フレーム現在のパッチとのdiffを取る必要がありコスト・複雑さに見合わないため、
    /// 「音色名を編集した」ことだけを検知する簡易版に留める）。
    unsaved: bool,
    /// Delete確認ダイアログの結果待ち（`request_delete`/`poll_pending_delete`参照）。
    pending_delete: Option<PendingDelete>,
    /// Openファイル選択ダイアログの結果待ち（`request_open`/`poll_pending_open`参照）。
    pending_open: Option<PendingOpen>,
    /// Save Asファイル保存ダイアログの結果待ち（`request_save_as`/`poll_pending_save_as`参照）。
    pending_save_as: Option<PendingSaveAs>,
    /// 直近の操作エラー（担当ファイルが無いbankでの「+ New Voice」等）と表示開始時刻。
    /// モーダルダイアログは描画コールバック内で同期的に出すとホスト（REAPER等）ごとクラッシュ
    /// するため（Delete/Open/Save Asと同じ理由）使えず、egui内で完結する一時テキスト表示で
    /// 代替する（`draw_presets_panel`が`ERROR_DISPLAY_DURATION`経過で自動的に消す）。
    last_error: Option<(String, std::time::Instant)>,
    /// PRESETSリストで実際にユーザーが（またはOpen/Save As/+ New Voiceが）エントリーを選択
    /// したか。falseの間はリストのどの行もハイライトしない。`sync_display_to_registry`は
    /// ファイル名・音色名の表示だけを合わせパッチには触れないため、ここがfalseのままだと
    /// 「先頭エントリーが選択されているように見えるのにパッチが違う」という誤解を
    /// 避けられる（2026-08-28、当初は新規Add直後デフォルト値なら実際に適用する案を試したが、
    /// ユーザー指摘により「選択状態そのものを見せない」方針へ変更。詳細はmemory
    /// `project_vst_preset_registry`「発見された挙動ギャップ」節参照）。
    has_selection: bool,
}

/// `PresetSession::last_error`を表示し続ける時間。
const ERROR_DISPLAY_DURATION: std::time::Duration = std::time::Duration::from_secs(4);

/// Delete確認中に確認先を固定するための情報。ダイアログ表示中にBank欄が操作された場合でも
/// 削除対象がずれないよう、リクエスト時点の(bank, program)を保持する。
struct PendingDelete {
    bank: u16,
    program: u8,
    receiver: mpsc::Receiver<bool>,
}

/// Open実行中に読込先bankを固定するための情報（ダイアログ表示中にBankが操作された場合の対策）。
struct PendingOpen {
    bank: u16,
    receiver: mpsc::Receiver<Option<PathBuf>>,
}

/// Save As実行中に保存対象(bank, program, 音色名)を固定するための情報。
struct PendingSaveAs {
    bank: u16,
    program: u8,
    patch_name: String,
    receiver: mpsc::Receiver<Option<PathBuf>>,
}

impl PresetSession {
    /// 起動直後は何も選択しない（プロジェクトを開いただけでパッチが書き換わる事故を
    /// 避ける、既存の「何も選ばない」挙動を踏襲）。
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

    /// 今のbankの担当ファイルが持つ音色一覧（未登録なら空）。
    fn entries(&self) -> &[Op505PresetEntry] {
        self.registry.get(&self.bank).map(Op505BankFile::entries).unwrap_or(&[])
    }

    /// Saveボタンを有効化してよいか（担当ファイルが無いbankでは保存できない）。
    /// `has_selection`も見る：バンク切り替え直後の未選択状態（`handle_bank_changed`でprogramが
    /// 0へリセットされた状態）でSaveを許すと、今鳴っている音を新バンクのprogram 0へ
    /// 無警告で上書きしてしまう事故につながるため（2026-08-29、handle_bank_changed実装と同時に対処）。
    fn save_enabled(&self) -> bool {
        self.has_selection && self.file_name.is_some()
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
    /// パッチへの反映（`apply_entry`）を行う。
    fn select_after_delete(&mut self, remaining: &[Op505PresetEntry]) -> Option<Op505PresetEntry> {
        let first = remaining.first()?.clone();
        self.program = first.program;
        self.patch_name = first.name.clone();
        self.unsaved = false;
        self.has_selection = true;
        Some(first)
    }

    /// 同じbank内でエントリーを選び直す（PRESETSリストのクリック、+ New Voice追加直後、
    /// 削除後の自動選択で使う）。file_name/bankは変わらない。
    fn select_entry(&mut self, program: u8, patch_name: String) {
        self.program = program;
        self.patch_name = patch_name;
        self.unsaved = false;
        self.has_selection = true;
    }

    /// 別ファイルを読み込んだ（Open）、または別ファイルへ書き出した（Save As）ときに呼ぶ。
    fn open_file(&mut self, program: u8, patch_name: String, file_name: Option<String>) {
        self.program = program;
        self.patch_name = patch_name;
        self.file_name = file_name;
        self.unsaved = false;
        self.has_selection = true;
    }

    /// 現在の担当ファイルへ上書き保存した（Save）ときに呼ぶ。program/patch_nameは変わらない。
    fn on_saved(&mut self, file_name: String) {
        self.file_name = Some(file_name);
        self.unsaved = false;
    }

    /// エディタ生成直後、今のbankに担当ファイルが既にあればファイル名・音色名の**表示だけ**を
    /// 合わせる。`apply_entry`は呼ばない＝パッチ（プロジェクトに保存済みのノブ値）には
    /// 一切触れない。`has_selection`はfalseのまま維持する＝PRESETSリストはどの行もハイライト
    /// しない（表示上「選択されている」ように見えるとパッチと一致していることを
    /// 期待させてしまうため）。gesture-appは起動時にパッチも実際に適用するが、VST/standaloneでは
    /// 「エディタを開いただけで今の音がプリセット内容に上書きされる」事故を
    /// 避けるため、この非対称は意図的（ユーザー確認済み、2026-08-28。当初は新規Add直後
    /// デフォルト値なら実際に適用する案を試したが、それでも「選択済みなのにパラメーターが違う」
    /// 違和感は残るとユーザーから指摘があり、「選択状態を見せない」方針へ変更）。
    fn sync_display_to_registry(&mut self) {
        let Some(bank_file) = self.registry.get(&self.bank) else { return };
        let Some(entry) = bank_file.entries().first() else { return };
        self.program = entry.program;
        self.patch_name = entry.name.clone();
        self.file_name = bank_file.file_name().map(str::to_string);
        self.unsaved = false;
    }
}

/// `entry`をホストの現在のパッチへ反映する（PRESETSパネルの全操作が最後に通る共通処理）。
/// `keep_fg`＝trueなら、Pitch/Cutoff/Gain FG（EG本体＋Depth）は今の設定を保ったまま、それ以外の
/// パラメーターだけ`entry.patch`の内容へ差し替える（PRESETSリストのShift+クリック用）。
fn apply_entry(host: &dyn PresetHost, entry: &Op505PresetEntry, keep_fg: bool) {
    let mut patch = entry.patch;
    if keep_fg {
        let current = host.current_patch();
        patch.channel.pitch_fg = current.channel.pitch_fg;
        patch.channel.cutoff_fg = current.channel.cutoff_fg;
        patch.channel.gain_fg = current.channel.gain_fg;
    }
    host.apply_patch(&patch);
}

/// Openファイル選択ダイアログの表示を要求する。`rfd::FileDialog::pick_file()`もDelete確認と同じく
/// ブロッキング呼び出しであり、egui描画コールバック内で直接呼ぶとホストごとクラッシュする
/// （実機確認で確認済み・2026-08-28、Delete確認ダイアログと同じ原因）。表示は別スレッドへ逃がし、
/// 結果は`poll_pending_open`が毎フレームpollする。
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

/// 毎フレーム呼ぶ。ファイル選択ダイアログの結果が届いていればロードを実行する。
/// 戻り値はUndo履歴クリアの要否（実際にファイルを読み込めたか。キャンセル/読込失敗ならfalse）。
fn poll_pending_open(session: &mut PresetSession, host: &dyn PresetHost) -> bool {
    let path = match session.pending_open.as_ref() {
        Some(pending) => match pending.receiver.try_recv() {
            Ok(path) => path,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => None,
        },
        None => return false,
    };
    let PendingOpen { bank, .. } = session.pending_open.take().expect("Some確認済み");
    let Some(path) = path else { return false }; // ダイアログをキャンセルした
    let Ok(json) = std::fs::read_to_string(&path) else { return false };
    let Ok(file) = Op505PresetFile::from_json(&json) else { return false };
    // ファイル自身が宣言しているbank番号は無視し、リクエスト時点のbankへ丸ごとロードする
    // （gesture-appと同じ、ユーザー確認済みの既定仕様）。
    let bank_file = Op505BankFile::from_loaded(path, file, bank);
    let Some(entry) = bank_file.entries().first().cloned() else { return false };

    // ダイアログ表示中にBankが切り替えられていた場合、パッチ・表示の更新は
    // 今見えているbankとは無関係になるためスキップする（レジストリへの登録自体は常に行う）。
    if bank == session.bank {
        apply_entry(host, &entry, false);
        session.open_file(entry.program, entry.name, bank_file.file_name().map(str::to_string));
    }
    host.publish_bank(&bank_file);
    session.registry.insert(bank, bank_file);
    true
}

/// 戻り値はUndo履歴クリアの要否（実際に保存できたか）。
fn handle_save(session: &mut PresetSession, host: &dyn PresetHost) -> bool {
    let patch = host.current_patch();
    let program = session.program;
    let patch_name = session.patch_name.clone();
    let Some(bank_file) = session.registry.get_mut(&session.bank) else { return false };
    if bank_file.upsert(program, patch_name, patch).is_err() {
        return false;
    }
    if bank_file.save().is_err() {
        return false;
    }
    let file_name = bank_file.file_name().unwrap_or("?").to_string();
    host.publish_bank(bank_file);
    session.on_saved(file_name);
    true
}

/// Save As保存ダイアログの表示を要求する。理由・方式はOpen（`request_open`）と同じ
/// （`rfd::FileDialog::save_file()`も同じブロッキングダイアログのため）。
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

/// 毎フレーム呼ぶ。保存ダイアログの結果が届いていれば書き出しを実行する。
/// 戻り値はUndo履歴クリアの要否（実際に書き出せたか）。
fn poll_pending_save_as(session: &mut PresetSession, host: &dyn PresetHost) -> bool {
    let path = match session.pending_save_as.as_ref() {
        Some(pending) => match pending.receiver.try_recv() {
            Ok(path) => path,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => None,
        },
        None => return false,
    };
    let PendingSaveAs { bank, program, patch_name, .. } = session.pending_save_as.take().expect("Some確認済み");
    let Some(path) = path else { return false }; // ダイアログをキャンセルした

    let patch = host.current_patch();
    let base_entries: Vec<Op505PresetEntry> = session.registry.get(&bank).map(|f| f.entries().to_vec()).unwrap_or_default();
    let Ok(bank_file) = Op505BankFile::write_as(patch, patch_name.clone(), bank, program, &base_entries, path) else { return false };

    let file_name = bank_file.file_name().unwrap_or("patch.op505").to_string();
    host.publish_bank(&bank_file);
    session.registry.insert(bank, bank_file);
    if bank == session.bank {
        session.open_file(program, patch_name, Some(file_name));
    }
    true
}

fn handle_add_new_voice(session: &mut PresetSession, host: &dyn PresetHost, copy_current: bool) -> Option<BankOp> {
    let source_patch = if copy_current { host.current_patch() } else { Op505Patch::default() };
    let Some(bank_file) = session.registry.get_mut(&session.bank) else {
        // gesture-app（`op505_presets.rs`の`op505_add_preset`）と同じ制約：担当ファイルが無い
        // bankでは追加できない（1バンク=1ファイル前提、先にOpen/Save Asでファイルを紐付ける
        // 必要がある）。gesture-app側はエラーダイアログを出すが、VST/standaloneは同期的な
        // モーダルが使えないため一時テキスト表示で代替する（2026-08-28、以前は無言でreturnしていた）。
        session.last_error =
            Some(("This bank has no file yet. Use Open or Save As first.".to_string(), std::time::Instant::now()));
        return None;
    };
    let Ok(entry) = bank_file.add_new_voice(source_patch) else { return None };
    if host.auto_save_bank_edits() && bank_file.save().is_err() {
        return None;
    }
    session.last_error = None;
    host.publish_bank(bank_file);
    apply_entry(host, &entry, false);
    let op = BankOp::Add { bank: session.bank, program: entry.program, name: entry.name.clone(), patch: entry.patch };
    session.select_entry(entry.program, entry.name);
    Some(op)
}

/// Delete確認ダイアログの表示を要求する。`rfd::MessageDialog::show()`はブロッキング呼び出しで、
/// egui描画コールバック（＝ホストのウィンドウプロシージャから呼ばれている最中）の内側で直接
/// 呼ぶとダイアログが回すネストしたメッセージループがプラグインウィンドウへ再入し、ホストごと
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

/// 毎フレーム呼ぶ。確認スレッドの結果が届いていれば削除を実行する（結果が未到着ならNone）。
fn poll_pending_delete(session: &mut PresetSession, host: &dyn PresetHost) -> Option<BankOp> {
    let confirmed = match session.pending_delete.as_ref() {
        Some(pending) => match pending.receiver.try_recv() {
            Ok(confirmed) => confirmed,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => false,
        },
        None => return None,
    };
    let PendingDelete { bank, program, .. } = session.pending_delete.take().expect("Some確認済み");
    if !confirmed {
        return None;
    }

    let Some(bank_file) = session.registry.get_mut(&bank) else { return None };
    // Undo対象化のため、削除前にエントリー内容を退避する（BankOp::Remove/Undo時の再挿入用）。
    let Some(removed) = bank_file.entries().iter().find(|e| e.program == program).cloned() else { return None };
    if bank_file.remove(program).is_err() {
        return None;
    }
    if host.auto_save_bank_edits() && bank_file.save().is_err() {
        return None;
    }
    let remaining = bank_file.entries().to_vec();
    host.publish_bank(bank_file);

    // ダイアログ表示中にBankが切り替えられていた場合、削除対象はもう画面に見えていないので
    // パッチへの反映（現在選択への追従）は行わない。
    if bank == session.bank {
        if let Some(next) = session.select_after_delete(&remaining) {
            apply_entry(host, &next, false);
        }
    }
    Some(BankOp::Remove { bank, program, name: removed.name, patch: removed.patch })
}

/// Bank欄のハンドル。ノブ下の数値欄と同じ見た目（`ui_core::spin_control`）にするための橋渡し。
/// `IntParamHandle::set`は`&self`のためCellで内部可変性を持たせる（gesture-appの`Rc<Cell<u16>>`に
/// 相当。呼び出し側は毎フレーム同期呼び出しで完結するのでRcは不要）。
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

/// Bank欄の数値が変わったときの表示更新：パッチには一切触れず、担当ファイルの
/// ファイル名だけを表示に反映する（`sync_display_to_registry`と同じ「選択状態を見せない」
/// 方針）。エントリーは自動選択しない——以前は今のprogramと同じ番号のエントリーがあれば
/// 自動選択・適用（`apply_entry`）していたが、「バンクを跨いでも前のバンクの選択が
/// 持ち越されて選択済みに見える」という違和感の原因になっていたため撤去した
/// （ユーザー指摘、2026-08-29）。
fn handle_bank_changed(session: &mut PresetSession) {
    session.file_name = session.registry.get(&session.bank).and_then(Op505BankFile::file_name).map(str::to_string);
    session.program = 0;
    session.patch_name.clear();
    session.unsaved = false;
    session.has_selection = false;
}

/// [`draw_presets_panel`]が返す、呼び出し側（Undoスタックを持つ側）が反応すべきイベント。
/// パッチ内パラメーターと違い音色名は`PresetSession`が持つためハンドル(`begin_edit`/`end_edit`)
/// 経由の記録ができない。フォーカス取得/喪失を音色名編集1操作の区切りとして呼び出し側へ伝える
/// （`op505-editor`はUndoスタック本体を知らないため、記録自体は行わずイベントの通知に留める）。
#[derive(Default, Clone)]
pub struct PresetsPanelEvents {
    /// 音色名欄がこのフレームでフォーカスを得た。
    pub patch_name_focus_gained: bool,
    /// 音色名欄がこのフレームでフォーカスを失った。
    pub patch_name_focus_lost: bool,
    /// PRESETSリストの通常クリック/Shift+クリックでプリセットがパッチへ適用された
    /// （+ New Voice/Deleteはバンク構成自体を変える別操作のため対象外）。`apply_entry`は
    /// `begin_edit`/`end_edit`を経由しないパッチ全体の差し替えのため、単発操作として
    /// begin/endを同フレームで記録することを呼び出し側へ伝える。
    pub list_selection_applied: bool,
    /// このフレームで+ New Voice/Deleteが実行された場合、その内容
    /// （Undo/Redo時に呼び出し側が`EditorPresetState::apply_bank_op`へ渡す逆操作の元）。
    /// パッチ内パラメーターと違い`op505-editor`はUndoスタック本体を知らないため、記録自体は
    /// 呼び出し側（standaloneの`EditorApp`）へ委ねる（`patch_name_focus_*`と同じ設計）。
    pub bank_op: Option<BankOp>,
    /// このフレームでOpen/Save/Save Asが完了した（キャンセル・失敗は含まない）。
    /// 呼び出し側は`UndoStack::clear()`を呼ぶこと——ファイルの内容がディスク上の実体と
    /// 対応する新しい基点になったので、それ以前のUndo履歴（特にバンクファイルの内容を
    /// 前提とする`BankOp`）は前提が崩れており巻き戻しの意味を持たない。
    pub history_cleared: bool,
    /// Undoボタンが押された。`UndoStack`本体は`op505-editor`が知らないため、記録・適用は
    /// 呼び出し側（standalone/VSTの`EditorApp`相当）へ委ねる（`bank_op`と同じ設計）。
    pub undo_requested: bool,
    /// Redoボタンが押された。
    pub redo_requested: bool,
}

impl PresetsPanelEvents {
    /// [`draw_editor_top_bar`]・[`draw_presets_drawer`]・[`poll_presets_events`]は同じフレームで
    /// 個別に呼ばれるため、呼び出し側がこのメソッドで1つに合成してから処理する。
    /// bool系フィールドはOR、`bank_op`は1フレームに高々1件しか発生しない前提で先勝ちにする。
    pub fn merge(&mut self, other: PresetsPanelEvents) {
        self.patch_name_focus_gained |= other.patch_name_focus_gained;
        self.patch_name_focus_lost |= other.patch_name_focus_lost;
        self.list_selection_applied |= other.list_selection_applied;
        self.bank_op = self.bank_op.take().or(other.bank_op);
        self.history_cleared |= other.history_cleared;
        self.undo_requested |= other.undo_requested;
        self.redo_requested |= other.redo_requested;
    }
}

/// PRESETSパネルのUndo/Redoボタンの有効/無効状態。`UndoStack::can_undo`/`can_redo`から
/// 呼び出し側が組み立てる（`op505-editor`は`UndoStack`本体を持たないため、ボタンの見た目だけを
/// この構造体で受け取る）。
#[derive(Debug, Clone, Copy, Default)]
pub struct UndoUiState {
    pub can_undo: bool,
    pub can_redo: bool,
}

/// エディタ上部のメニューバー（ハンバーガー・Fileメニュー・Undo/Redoアイコン）を描画する。
/// 旧PRESETSサイドバーにあったOpen/Save/Save As・Undo/Redoボタンをここへ集約する
/// （2026-09-03、PRESETSパネルの常設サイドバー→ハンバーガー開閉のオーバーレイ化に伴う移設）。
///
/// `egui::MenuBar`（`ui.horizontal`ではなく）を使うのは、egui公式デモ
/// （<https://github.com/emilk/egui/blob/main/crates/egui_demo_lib/src/demo/demo_app_windows.rs>）
/// と同じ見た目に揃えるため：`MenuBar`は既定で`menu::menu_style`（非ホバー時の背景・枠線を
/// 透明化するスタイル）を子ウィジェットへ適用するので、☰・File・Undo/Redoが「押しっぱなしの
/// ボタン」ではなく「ホバー時だけ反応するフラットなツールバー項目」に見える
/// （egui-0.34.3 `containers/menu.rs`の`menu_style`関数参照）。
///
/// `right_content`はホスト固有の右寄せ項目（standaloneの"Edit Channel"セレクタ等）を同じ行へ
/// 差し込むためのフック。`egui::Layout::right_to_left`は「先に追加した項目ほど右端」に置かれる
/// ため、呼び出し側は表示したい順序と逆順に`ui`へ追加すること（VSTは`|_ui| {}`で何も足さない）。
/// `MenuBar`のフラット化スタイルは`right_content`には適用しない——Edit Channel等は「メニュー
/// バーの一部」ではなく「メニューバーに間借りしている通常のコントロール」という位置づけのため、
/// 他のパネルのコンボボックス等と同じ見た目に戻す（`ui.reset_style()`でMenuBarのスコープ内
/// スタイル上書きを取り消し、`Context`の既定スタイルへ戻す）。
pub fn draw_editor_top_bar(
    ui: &mut egui::Ui,
    state: &mut EditorPresetState,
    host: &dyn PresetHost,
    undo_ui: UndoUiState,
    right_content: impl FnOnce(&mut egui::Ui),
) -> PresetsPanelEvents {
    let mut events = PresetsPanelEvents::default();

    egui::MenuBar::new().ui(ui, |ui| {
        if ui.button("☰").on_hover_text("Presets").clicked() {
            state.drawer_open = !state.drawer_open;
            state.drawer_just_opened = state.drawer_open;
        }

        ui.menu_button("File", |ui| {
            if ui.add_enabled(state.session.pending_open.is_none(), egui::Button::new("Open...")).clicked() {
                request_open(&mut state.session);
                ui.close();
            }
            if ui.add_enabled(state.session.save_enabled(), egui::Button::new("Save")).clicked() {
                events.history_cleared |= handle_save(&mut state.session, host);
                ui.close();
            }
            if ui.add_enabled(state.session.pending_save_as.is_none(), egui::Button::new("Save As...")).clicked() {
                request_save_as(&mut state.session);
                ui.close();
            }
        });

        if ui.add_enabled(undo_ui.can_undo, egui::Button::new("↺")).on_hover_text("Undo (Ctrl+Z)").clicked() {
            events.undo_requested = true;
        }
        if ui.add_enabled(undo_ui.can_redo, egui::Button::new("↻")).on_hover_text("Redo (Ctrl+Y)").clicked() {
            events.redo_requested = true;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.reset_style();
            right_content(ui);
        });
    });

    events
}

/// PRESETSドロワーの中身（Bank欄・ファイル名表示・音色名編集・PRESETSリスト）を描画する。
/// gesture-appのレイアウト（Bank→ファイル名→音色名→区切り線→PRESETSリスト（+ New Voice/Delete
/// はScrollArea内）の順）をそのまま踏襲する。呼び出し側（[`draw_presets_drawer`]）がオーバーレイ
/// 用のArea・背景・開閉アニメーションを用意した上でこの関数を呼ぶ。
/// `ScrollArea::auto_shrink([false,false])`は残り領域を全部占有するため、**ScrollAreaより後に
/// 置いたウィジェットは表示されない**——ここより下に新しいウィジェットを足す場合は必ずScrollArea
/// の中に置くこと（memory `project_preset_list_scrollbar_and_add_delete`参照）。
fn draw_presets_drawer_contents(ui: &mut egui::Ui, state: &mut EditorPresetState, host: &dyn PresetHost) -> PresetsPanelEvents {
    let session = &mut state.session;
    let mut events = PresetsPanelEvents::default();

    ui.horizontal(|ui| {
        ui.label("Bank");
        // gesture-app（`editor-wasm/src/app.rs`のBankField）と同じ±ボタン付き数値欄で揃える。
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
    let patch_name_response = ui.text_edit_singleline(&mut patch_name);
    events.patch_name_focus_gained = patch_name_response.gained_focus();
    events.patch_name_focus_lost = patch_name_response.lost_focus();
    if patch_name_response.changed() {
        session.patch_name = patch_name;
        session.unsaved = true;
    }
    ui.separator();

    // 「+ New Voice」等の操作エラー（担当ファイルが無いbank等）を一定時間だけ表示する。
    // モーダルダイアログは描画コールバック内で同期的に出すとホストごとクラッシュするため使えない
    // （Delete/Open/Save Asと同じ理由）。ScrollAreaより前に置く必要がある点に注意
    // （`auto_shrink([false,false])`の罠、memory `project_preset_list_scrollbar_and_add_delete`）。
    if let Some((msg, shown_at)) = &session.last_error {
        let elapsed = shown_at.elapsed();
        if elapsed < ERROR_DISPLAY_DURATION {
            ui.colored_label(egui::Color32::from_rgb(220, 90, 90), msg);
            // 操作が無く再描画が起きない場合でも確実に消えるよう、残り時間ぶんの再描画を予約する。
            ui.ctx().request_repaint_after(ERROR_DISPLAY_DURATION - elapsed);
        } else {
            session.last_error = None;
        }
    }

    // Deleteキーのみで削除する（gesture-appと同じ、専用ボタンは置かない）。テキスト入力中は
    // 文字削除と衝突するため無効化する。DAW/ホスト環境によってはDeleteキーが自身のショートカット
    // （選択アイテム削除等）に奪われる可能性があるが、gesture-app版とのUI統一を優先する
    // （ユーザー確認済み、2026-08-28）。
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
                apply_entry(host, &entry, keep_fg);
                session.select_entry(entry.program, entry.name);
                events.list_selection_applied = true;
            }
        }
        if ui.selectable_label(false, "+ New Voice").clicked() {
            let copy_current = ui.input(|i| i.modifiers.shift);
            events.bank_op = handle_add_new_voice(session, host, copy_current);
        }
    });

    if delete_by_key {
        request_delete(session);
    }

    events
}

/// PRESETSドロワー本体。ハンバーガーボタンの開閉状態（[`EditorPresetState::drawer_open`]）に
/// 応じて、`area_rect`（通常はCentralPanelが返した残り領域）へオーバーレイでスライドイン/アウト
/// する。押しのけ方式（`egui::Panel::show_animated_inside`）ではなく`egui::Area`を使うのは、
/// コントロール一式に覆いかぶさる見た目をユーザーが明示的に希望したため（メニューバー自体は
/// 押しのけない・ユーザー提示のワイヤーフレーム参照）。
///
/// アニメーション中（`t`が0〜1の間）はArea自体を描画しないため、この関数は非同期ダイアログの
/// 結果回収（Open/Save As/Delete確認のpoll）を行わない——ドロワーを閉じている間もOpen等の
/// 結果を取りこぼさないよう、pollは[`poll_presets_events`]として毎フレーム無条件に呼ぶこと。
///
/// アニメーション時間は0.3秒＋`cubic_out`イージング（動き始めが速く、終端でゆっくり収まる）。
/// 既定の`animate_bool`（0.2秒・線形）だと動きが速すぎて「スライドしている」と体感しづらかった
/// （ユーザー指摘、2026-09-03）。
///
/// ドロワー外クリックで閉じる（[`egui::Response::clicked_elsewhere`]）。ただし開かせた張本人の
/// クリック（ハンバーガーボタン自身、ドロワー矩形の外＝トップバー側にある）を「外側クリック」と
/// 誤認して同フレーム中に閉じ直してしまわないよう、[`EditorPresetState::drawer_just_opened`]で
/// そのクリックだけ1フレーム限り無効化する（`animate_bool`は経過時間ベースの漸増のため、
/// 状態が変化した当フレームでも`t`は既に正の値になっており、素朴に「t==0の間はArea自体が
/// 存在しないので大丈夫」という前提は成り立たなかった——実装時の落とし穴、
/// `egui-0.34.3 animation_manager.rs`の`elapsed`計算参照）。
///
/// **中身は常に`PRESETS_SIDEBAR_WIDTH`ぶんのフル幅でレイアウトし、実際に見える範囲だけを
/// `t`でクリップしてワイプさせる**（左端固定・`ui.shrink_clip_rect`で右端をアニメーション）。
/// 当初は「Uiの幅自体を`t*width`まで縮めてレイアウトする」方式（egui公式の
/// `Panel::get_animated_panel`と同じ「幅が伸びる」方式、
/// <https://github.com/emilk/egui/blob/main/crates/egui_demo_lib/src/backend_panel.rs>
/// が使う`SidePanel::show_animated_inside`の内部実装）を試したが、これだと途中の狭い幅で
/// ファイル名やPRESETSリストの各行テキストが折り返され（`ui.set_width`が小さいほどワード
/// ラップが変わる）、伸びるにつれてラップ位置が何度も変わってガタつく見た目になった
/// （egui公式は代わりに「アニメーション中は中身を描画しない、空箱が伸びるだけ」で回避しており、
/// それも試したが今度は開き切った瞬間に文字が一斉に現れる不自然さがあった。ユーザー指摘、
/// 2026-09-03）。**クリップ方式なら`ui.set_width`は常にフル幅で固定されるためラップは
/// 一切変化せず、確定済みのレイアウトが左から右へワイプして現れる**（egui公式デモの
/// Backendパネルと違い、中身込みで滑らかにスライドして見える）。
pub fn draw_presets_drawer(ctx: &egui::Context, state: &mut EditorPresetState, host: &dyn PresetHost, area_rect: egui::Rect) -> PresetsPanelEvents {
    // 「ドロワーを開かせた張本人のクリック」を今フレーム限りで無効化する（一度読んだら
    // falseへ戻す）。これを先頭で消費しておくことで、以降のどのreturn経路でも取りこぼさない。
    let just_opened = std::mem::take(&mut state.drawer_just_opened);

    let id = egui::Id::new("op505_presets_drawer");
    const ANIM_SECS: f64 = 0.3;
    let now = ctx.input(|i| i.time);
    let anim = &mut state.drawer_anim;
    if anim.last_target != state.drawer_open {
        anim.from = anim.frac;
        anim.started_at = Some(now);
        anim.last_target = state.drawer_open;
    }
    let target = if state.drawer_open { 1.0 } else { 0.0 };
    anim.frac = match anim.started_at {
        Some(start) => {
            let raw = ((now - start) / ANIM_SECS).clamp(0.0, 1.0) as f32;
            if raw >= 1.0 {
                anim.started_at = None;
                target
            } else {
                ctx.request_repaint();
                anim.from + (target - anim.from) * egui::emath::easing::cubic_out(raw)
            }
        }
        None => target,
    };
    let t = anim.frac;
    if t <= 0.0 {
        return PresetsPanelEvents::default();
    }

    let full_width = PRESETS_SIDEBAR_WIDTH.min(area_rect.width());
    let full_rect = egui::Rect::from_min_size(area_rect.min, egui::vec2(full_width, area_rect.height()));
    let visible_rect = egui::Rect::from_min_size(area_rect.min, egui::vec2(t * full_width, area_rect.height()));

    let mut events = PresetsPanelEvents::default();
    let area_response = egui::Area::new(id).order(egui::Order::Middle).fixed_pos(full_rect.min).movable(false).show(ctx, |ui| {
        // Uiの幅は常にフル幅（ラップ位置を固定するため）。
        ui.set_width(full_rect.width());
        ui.set_height(full_rect.height());
        // 背景は「見える範囲（`visible_rect`）だけ」に自前で塗る——`Frame::side_top_panel`の
        // `fill`をそのまま使うとフル幅（`full_rect`）全体に対して塗ってしまい、アニメーション中
        // でも下のCHANNEL等のコントロールが常に最大幅ぶん覆われて見える（ユーザー指摘、
        // 2026-09-03）。marginはFrameから借りるが色は透過にし、塗りは自前でvisible_rect分だけ行う。
        let panel_fill = ui.visuals().panel_fill;
        ui.painter().rect_filled(visible_rect, 0.0, panel_fill);
        // `Frame::window`（角丸+ドロップシャドウ）だとポップアップ/ダイアログに見えてしまう。
        // 実際の`Panel`が使う`Frame::side_top_panel`と同じmargin構成（角丸なし・影なし）を
        // 借りつつ、fill自体は上で塗った分と重複しないようtransparentにする。
        egui::Frame::side_top_panel(ui.style()).fill(egui::Color32::TRANSPARENT).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            // 実際に描画される範囲を`visible_rect`だけに制限する。Uiの幅そのもの
            // （レイアウト・ラップ判定の基準）はフル幅のまま変えないため、ファイル名等の
            // 折り返し位置はアニメーション中も一切変化しない——ここが直前に試した「事後マスク」
            // 方式との違い：マスクは「描いてから隠す」ため背景の塗り自体はフル幅分残ってしまうが、
            // クリップは「そもそも描かせない」ため背景・文字とも`visible_rect`の外には一切出ない。
            ui.shrink_clip_rect(visible_rect);
            events = draw_presets_drawer_contents(ui, state, host);
        });
        // 縁取りは右端（下のコントロールと接する境目）だけでよい——`Frame::stroke`は矩形四辺
        // 全部を囲ってしまい「浮いたカード」に見えてしまうため、`Painter::vline`で右端だけ引く。
        let stroke = ui.visuals().window_stroke();
        ui.painter().vline(visible_rect.right(), visible_rect.y_range(), stroke);
    });

    if !just_opened && area_response.response.clicked_elsewhere() {
        state.drawer_open = false;
    }

    events
}

/// 毎フレーム呼ぶ。PRESETSドロワーの開閉に関わらず、非同期ダイアログ（Open/Save As/Delete確認）の
/// 結果を回収する。ドロワーを閉じている間にOpen/Save Asを実行しても結果を取りこぼさないための
/// 分離（[`draw_presets_drawer`]のdoc参照）。
pub fn poll_presets_events(state: &mut EditorPresetState, host: &dyn PresetHost) -> PresetsPanelEvents {
    let session = &mut state.session;
    let mut events = PresetsPanelEvents::default();
    if let Some(op) = poll_pending_delete(session, host) {
        events.bank_op = Some(op);
    }
    events.history_cleared |= poll_pending_open(session, host);
    events.history_cleared |= poll_pending_save_as(session, host);
    events
}

/// PRESETSドロワー開閉アニメーションの進行状態。[`draw_presets_drawer`]が毎フレーム更新する。
///
/// `egui::Context::animate_bool_with_time_and_easing`を使わない理由：内部実装
/// （egui-0.34.3 `animation_manager.rs`）は「前回このIDを呼んだフレームからの経過時間」を
/// `input.stable_dt`でクランプしながら加算する方式のため、eframeのReactiveモード
/// （入力が無い間は再描画しない）で「しばらく描画が無かった直後の最初の1フレーム」では
/// `stable_dt`自体がその空白期間を反映した大きな値になり、クランプが実質無効化されて
/// アニメーションが1フレームでほぼ完了してしまう（実機で確認済み、2026-09-03）。ここでは
/// トグルした絶対時刻を記録し「今」との差分から経過率を計算する方式にすることで、
/// 直前フレームの間隔に依存しない安定した所要時間にする。
#[derive(Debug, Clone, Copy)]
struct DrawerAnim {
    /// 直近フレームで見た`drawer_open`の値（変化検知用）。
    last_target: bool,
    /// 直近のトグル開始時点での表示率（0.0=完全に閉, 1.0=完全に開）。トグルの度にその時点の
    /// 表示率を保持し直すことで、アニメーション中の再トグル（開閉の連打）でも現在位置から
    /// 滑らかに反転できる。
    from: f32,
    /// トグルした時刻（`ctx.input(|i| i.time)`）。Noneなら現在の`last_target`のまま安定している
    /// （アニメーション不要）。
    started_at: Option<f64>,
    /// 直近フレームで計算した表示率（0.0〜1.0）。次にトグルされた際の`from`になる。
    frac: f32,
}

impl Default for DrawerAnim {
    fn default() -> Self {
        DrawerAnim { last_target: false, from: 0.0, started_at: None, frac: 0.0 }
    }
}

/// PRESETSパネル分の状態。エディタ生成時に一度だけレジストリを構築する。
pub struct EditorPresetState {
    session: PresetSession,
    /// PRESETSドロワー（ハンバーガーボタンで開閉するオーバーレイ）の開閉状態。
    drawer_open: bool,
    /// このフレームでハンバーガーボタンがドロワーを開いた（false→true）ばかりか。
    /// [`draw_presets_drawer`]の「ドロワー外クリックで閉じる」判定用の一回限りガード
    /// （ハンバーガー自身はドロワー矩形の外にあるため、そのクリックを「外側クリック」と
    /// 誤認して同フレーム中に閉じ直してしまうのを防ぐ。詳細は`draw_presets_drawer`のdoc参照）。
    drawer_just_opened: bool,
    /// ドロワー開閉アニメーションの進行状態。
    drawer_anim: DrawerAnim,
}

impl EditorPresetState {
    pub fn new() -> Self {
        let mut session = PresetSession::new(build_op505_registry(&op505_presets_dir()));
        session.sync_display_to_registry();
        EditorPresetState { session, drawer_open: false, drawer_just_opened: false, drawer_anim: DrawerAnim::default() }
    }

    /// PRESETSドロワーが開いているか（アニメーション目標状態。実際の表示率は
    /// [`draw_presets_drawer`]内で`Context::animate_bool_with_time`が補間する）。
    pub fn drawer_open(&self) -> bool {
        self.drawer_open
    }

    /// 現在選択中のbank。Undoスナップショットの構築に使う。
    pub fn bank(&self) -> u16 {
        self.session.bank
    }

    /// 現在選択中のprogram番号。Undoスナップショットの構築に使う。
    pub fn program(&self) -> u8 {
        self.session.program
    }

    /// 現在の音色名（音欄の表示値）。Undoスナップショットの構築に使う。
    pub fn patch_name(&self) -> &str {
        &self.session.patch_name
    }

    /// PRESETSリストで実際に何かが選択されているか。Undoスナップショットの構築に使う。
    pub fn has_selection(&self) -> bool {
        self.session.has_selection
    }

    /// Undo/Redo適用時、スナップショットが持つ選択状態を書き戻す。`file_name`/`unsaved`には
    /// 触れない（Undo/Redoはパッチ内容の巻き戻しであり、ファイルとの対応関係や未保存状態の
    /// 意味論は`select_entry`等の通常の選択操作とは別に扱う）。
    pub fn restore_selection(&mut self, patch_name: String, bank: u16, program: u8, has_selection: bool) {
        self.session.patch_name = patch_name;
        self.session.bank = bank;
        self.session.program = program;
        self.session.has_selection = has_selection;
    }

    /// Undo/Redoが返した[`BankOp`]をバンクレジストリへ再適用する（+ New Voice/Deleteの
    /// 巻き戻し・やり直し）。対象バンクの担当ファイルが無い等の異常系は静かに無視する
    /// （Undoスタック自体がAdd/Removeの整合性を保証しているため通常起きない）。
    /// パッチ選択状態（bank/program/patch_name等）はこのメソッドの対象外——呼び出し側が
    /// `restore_selection`で別途復元する。
    pub fn apply_bank_op(&mut self, op: &BankOp, host: &dyn PresetHost) {
        let (&bank, program) = match op {
            BankOp::Add { bank, program, .. } | BankOp::Remove { bank, program, .. } => (bank, *program),
        };
        let Some(bank_file) = self.session.registry.get_mut(&bank) else { return };
        match op {
            BankOp::Add { name, patch, .. } => bank_file.restore_entry(program, name.clone(), *patch),
            BankOp::Remove { .. } => {
                if bank_file.remove(program).is_err() {
                    return;
                }
            }
        }
        if host.auto_save_bank_edits() && bank_file.save().is_err() {
            return;
        }
        host.publish_bank(bank_file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn entry(program: u8, name: &str) -> Op505PresetEntry {
        Op505PresetEntry { program, name: name.to_string(), patch: Op505Patch::default() }
    }

    /// `apply_bank_op`のテスト用モック。呼ばれた`publish_bank`の回数だけ記録する。
    struct MockHost {
        auto_save: bool,
        published: RefCell<Vec<Op505PresetFile>>,
    }

    impl PresetHost for MockHost {
        fn current_patch(&self) -> Op505Patch {
            Op505Patch::default()
        }
        fn apply_patch(&self, _patch: &Op505Patch) {}
        fn publish_bank(&self, bank_file: &Op505BankFile) {
            self.published.borrow_mut().push(bank_file.as_presets_file());
        }
        fn auto_save_bank_edits(&self) -> bool {
            self.auto_save
        }
    }

    #[test]
    fn apply_bank_op_add_reinserts_entry_at_original_program() {
        let mut registry = Op505BankRegistry::new();
        let file = Op505PresetFile::Presets { bank: 0, presets: vec![entry(0, "Existing")] };
        registry.insert(0, Op505BankFile::from_loaded(PathBuf::from("bank0.op505"), file, 0));
        let mut state = EditorPresetState {
            session: PresetSession::new(registry),
            drawer_open: false,
            drawer_just_opened: false,
            drawer_anim: DrawerAnim::default(),
        };

        let host = MockHost { auto_save: false, published: RefCell::new(vec![]) };
        let op = BankOp::Add { bank: 0, program: 1, name: "Voice001".to_string(), patch: Op505Patch::default() };
        state.apply_bank_op(&op, &host);

        assert_eq!(state.session.entries().len(), 2, "+ New VoiceのRedo/DeleteのUndoで元のprogramへ復元されるはず");
        assert!(state.session.entries().iter().any(|e| e.program == 1 && e.name == "Voice001"));
        assert_eq!(host.published.borrow().len(), 1, "auto_save無効でもpublish_bankは常に呼ぶはず");
    }

    #[test]
    fn apply_bank_op_remove_deletes_entry() {
        let mut registry = Op505BankRegistry::new();
        let file = Op505PresetFile::Presets { bank: 0, presets: vec![entry(0, "A"), entry(1, "B")] };
        registry.insert(0, Op505BankFile::from_loaded(PathBuf::from("bank0.op505"), file, 0));
        let mut state = EditorPresetState {
            session: PresetSession::new(registry),
            drawer_open: false,
            drawer_just_opened: false,
            drawer_anim: DrawerAnim::default(),
        };

        let host = MockHost { auto_save: false, published: RefCell::new(vec![]) };
        let op = BankOp::Remove { bank: 0, program: 0, name: "A".to_string(), patch: Op505Patch::default() };
        state.apply_bank_op(&op, &host);

        assert_eq!(state.session.entries().len(), 1, "+ New VoiceのUndo/DeleteのRedoで対象エントリーが消えるはず");
        assert_eq!(state.session.entries()[0].program, 1);
    }

    #[test]
    fn save_enabled_reflects_file_name() {
        let mut session = PresetSession::new(Op505BankRegistry::new());
        assert!(!session.save_enabled(), "file_name未設定ならSave不可");
        // 実際のUIではSaveボタンはsave_enabled()自身がゲートするため、on_saved()に到達する時点で
        // 既に選択済み（has_selection=true）のはず。テストでも同じ前提を再現する。
        session.select_entry(0, "Voice".to_string());
        session.on_saved("a.op505".to_string());
        assert!(session.save_enabled());
    }

    /// バンク切り替え直後（未選択状態）はfile_nameがあってもSave不可
    /// （今鳴っている音を新バンクのprogram 0へ無警告で上書きする事故を防ぐ）。
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
        assert_eq!(session.program, 3, "先頭エントリーのprogramへ合わせるはず（apply_entryは呼ばない＝パッチは不変）");
        assert!(!session.unsaved);
        assert!(
            !session.has_selection,
            "has_selectionはfalseのままのはず（PRESETSリストのハイライトを表示しない、パッチと一致しない選択済み表示による誤解を避けるため）"
        );
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

    /// バンク切り替え後、旧バンクで選択していたprogram番号と同じ番号のエントリーが
    /// 新バンクに存在しても自動選択・自動適用しない（選択が持ち越されて見える違和感を防ぐ）。
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

    /// 担当ファイルが無いbankへ切り替えたときも、選択なし・ファイル名なしの状態になる。
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
