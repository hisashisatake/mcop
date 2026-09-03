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
    /// VST=true（従来通り即save）、standalone=false（Save/Save Asでのみ保存、Undoが効くようにするため）。
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
fn poll_pending_open(session: &mut PresetSession, host: &dyn PresetHost) {
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
    // ファイル自身が宣言しているbank番号は無視し、リクエスト時点のbankへ丸ごとロードする
    // （gesture-appと同じ、ユーザー確認済みの既定仕様）。
    let bank_file = Op505BankFile::from_loaded(path, file, bank);
    let Some(entry) = bank_file.entries().first().cloned() else { return };

    // ダイアログ表示中にBankが切り替えられていた場合、パッチ・表示の更新は
    // 今見えているbankとは無関係になるためスキップする（レジストリへの登録自体は常に行う）。
    if bank == session.bank {
        apply_entry(host, &entry, false);
        session.open_file(entry.program, entry.name, bank_file.file_name().map(str::to_string));
    }
    host.publish_bank(&bank_file);
    session.registry.insert(bank, bank_file);
}

fn handle_save(session: &mut PresetSession, host: &dyn PresetHost) {
    let patch = host.current_patch();
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
    host.publish_bank(bank_file);
    session.on_saved(file_name);
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
fn poll_pending_save_as(session: &mut PresetSession, host: &dyn PresetHost) {
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

    let patch = host.current_patch();
    let base_entries: Vec<Op505PresetEntry> = session.registry.get(&bank).map(|f| f.entries().to_vec()).unwrap_or_default();
    let Ok(bank_file) = Op505BankFile::write_as(patch, patch_name.clone(), bank, program, &base_entries, path) else { return };

    let file_name = bank_file.file_name().unwrap_or("patch.op505").to_string();
    host.publish_bank(&bank_file);
    session.registry.insert(bank, bank_file);
    if bank == session.bank {
        session.open_file(program, patch_name, Some(file_name));
    }
}

fn handle_add_new_voice(session: &mut PresetSession, host: &dyn PresetHost, copy_current: bool) {
    let source_patch = if copy_current { host.current_patch() } else { Op505Patch::default() };
    let Some(bank_file) = session.registry.get_mut(&session.bank) else {
        // gesture-app（`op505_presets.rs`の`op505_add_preset`）と同じ制約：担当ファイルが無い
        // bankでは追加できない（1バンク=1ファイル前提、先にOpen/Save Asでファイルを紐付ける
        // 必要がある）。gesture-app側はエラーダイアログを出すが、VST/standaloneは同期的な
        // モーダルが使えないため一時テキスト表示で代替する（2026-08-28、以前は無言でreturnしていた）。
        session.last_error =
            Some(("This bank has no file yet. Use Open or Save As first.".to_string(), std::time::Instant::now()));
        return;
    };
    let Ok(entry) = bank_file.add_new_voice(source_patch) else { return };
    if host.auto_save_bank_edits() && bank_file.save().is_err() {
        return;
    }
    session.last_error = None;
    host.publish_bank(bank_file);
    apply_entry(host, &entry, false);
    session.select_entry(entry.program, entry.name);
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

/// 毎フレーム呼ぶ。確認スレッドの結果が届いていれば削除を実行する（結果が未到着ならno-op）。
fn poll_pending_delete(session: &mut PresetSession, host: &dyn PresetHost) {
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
    if host.auto_save_bank_edits() && bank_file.save().is_err() {
        return;
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
#[derive(Default, Clone, Copy)]
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
}

/// PRESETSパネル本体を描画する。gesture-appのレイアウト（Open/Save/Save As→Bank→ファイル名→
/// 音色名→区切り線→PRESETSリスト（+ New Voice/DeleteはScrollArea内）の順）をそのまま踏襲する。
/// `ScrollArea::auto_shrink([false,false])`は残り領域を全部占有するため、**ScrollAreaより後に
/// 置いたウィジェットは表示されない**——ここより下に新しいウィジェットを足す場合は必ずScrollArea
/// の中に置くこと（memory `project_preset_list_scrollbar_and_add_delete`参照）。
pub fn draw_presets_panel(ui: &mut egui::Ui, state: &mut EditorPresetState, host: &dyn PresetHost) -> PresetsPanelEvents {
    let session = &mut state.session;

    ui.horizontal(|ui| {
        if ui.add_enabled(session.pending_open.is_none(), egui::Button::new("Open")).clicked() {
            request_open(session);
        }
        if ui.add_enabled(session.save_enabled(), egui::Button::new("Save")).clicked() {
            handle_save(session, host);
        }
        if ui.add_enabled(session.pending_save_as.is_none(), egui::Button::new("Save As")).clicked() {
            request_save_as(session);
        }
    });

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
    let mut events = PresetsPanelEvents {
        patch_name_focus_gained: patch_name_response.gained_focus(),
        patch_name_focus_lost: patch_name_response.lost_focus(),
        list_selection_applied: false,
    };
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
            handle_add_new_voice(session, host, copy_current);
        }
    });

    if delete_by_key {
        request_delete(session);
    }
    poll_pending_delete(session, host);
    poll_pending_open(session, host);
    poll_pending_save_as(session, host);

    events
}

/// PRESETSパネル分の状態。エディタ生成時に一度だけレジストリを構築する。
pub struct EditorPresetState {
    session: PresetSession,
}

impl EditorPresetState {
    pub fn new() -> Self {
        let mut session = PresetSession::new(build_op505_registry(&op505_presets_dir()));
        session.sync_display_to_registry();
        EditorPresetState { session }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(program: u8, name: &str) -> Op505PresetEntry {
        Op505PresetEntry { program, name: name.to_string(), patch: Op505Patch::default() }
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
