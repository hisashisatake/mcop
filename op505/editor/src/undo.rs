//! 音色エディタのUndo/Redoスタック。
//!
//! # 記録方式（ハイブリッド）
//! - パッチ内パラメーター編集（ノブ・EG・波形/enumセレクタ・音色名等）は
//!   [`UndoEntry::StateChange`]（Memento＝操作前後の状態を丸ごと保持）。`EditorSnapshot`は
//!   数百バイト程度で`Op505Patch`/`MasterEffectsState`ともCopy可能なため、フィールド種別ごとに
//!   差分型を書き分けるより単純になる。
//! - バンク操作（+New Voice/Delete）は[`UndoEntry::BankChange`]（Command＝`BankOp`の差分）。
//!   操作種別がAdd/Removeの2つしかなく対象（program番号）が可変という構造のため、
//!   差分の方がバンク全体をコピーするより省メモリになる。
//!
//! # 記録タイミング
//! 「物理的な操作の開始」から「物理的な操作の終了」までを1エントリとする（時間窓による
//! 合併はしない）。ウィジェット側（`ui-core`のハンドル実装）は[`UndoStack::note_begin_edit`]/
//! [`UndoStack::note_end_edit`]を呼ぶだけで、実際のスナップショット取得と比較は
//! フレーム境界の[`UndoStack::begin_frame`]/[`UndoStack::end_frame`]が担う
//! （ウィジェット側は`patch_name`等パッチ外の状態を知らないため）。
//!
//! ```text
//! フレーム先頭  : begin_frame(現在のスナップショット)
//! 描画中(ハンドル): note_begin_edit() → pending_beforeが空ならフレーム先頭のスナップショットを保持
//!                  note_end_edit()   → このフレームでコミット要求フラグを立てる
//! フレーム末尾  : end_frame(現在のスナップショット)
//!                  → コミット要求があれば pending_before と比較し、差があれば1エントリ積む
//! ```
//!
//! ドラッグ操作はbegin_editとend_editが別フレームにまたがるため、`pending_before`は
//! 「まだNoneなら保持する」というガードにより、ドラッグ中の途中フレームで上書きされない。
//! 孤立したend_edit（対応するbegin_editが無い）や二重begin_editも自然に吸収される
//! （それぞれ「何もしない」「最初のbeforeを保持し続ける」という結果になる）。

use op505_core::Op505Patch;

use crate::param_spec::{BoolField, EgSlot, FxInt, IntField, PatchInt};
use crate::patch_source::{read_bool, read_eg, read_int, MasterEffectsState};

/// ある瞬間のエディタ状態をまるごと保持するスナップショット（Memento）。
#[derive(Clone, PartialEq)]
pub struct EditorSnapshot {
    pub patch: Op505Patch,
    pub master: MasterEffectsState,
    pub patch_name: String,
    pub bank: u16,
    pub program: u8,
    pub has_selection: bool,
}

/// `EditorSnapshot::diff_to`が返す、書き換えが必要なフィールドの一覧。
/// VST側のUndo/Redo適用が「変わった値だけ`ParamSetter`へ書く」ために使う
/// （`apply_patch`の一括再適用と違い、DAWへ記録されるgestureを実際の変更分だけに絞る）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotDiff {
    pub ints: Vec<IntField>,
    pub bools: Vec<BoolField>,
    pub egs: Vec<EgSlot>,
}

impl EditorSnapshot {
    /// `self`（現在値）から`target`へ移すために書き換えが必要なフィールドだけを列挙する。
    /// パッチ内パラメーター（`IntField::Patch`/`BoolField`/`EgSlot::Op`+`Fg`）と
    /// MASTER EFFECTS（`IntField::Fx`）の両方を対象にする。
    pub fn diff_to(&self, target: &EditorSnapshot) -> SnapshotDiff {
        let mut ints = Vec::new();
        for field in PatchInt::all() {
            if read_int(&self.patch, field) != read_int(&target.patch, field) {
                ints.push(IntField::Patch(field));
            }
        }
        for fx in FxInt::ALL {
            if self.master.field(fx) != target.master.field(fx) {
                ints.push(IntField::Fx(fx));
            }
        }
        let mut bools = Vec::new();
        for field in BoolField::ALL {
            if read_bool(&self.patch, field) != read_bool(&target.patch, field) {
                bools.push(field);
            }
        }
        let mut egs = Vec::new();
        for slot in EgSlot::ALL {
            if read_eg(&self.patch, slot) != read_eg(&target.patch, slot) {
                egs.push(slot);
            }
        }
        SnapshotDiff { ints, bools, egs }
    }
}

/// バンク構成の変更（+New Voice/Delete）。対象は常にバンク内の1エントリ。
#[derive(Clone, PartialEq)]
pub enum BankOp {
    Add { bank: u16, program: u8, name: String, patch: Op505Patch },
    Remove { bank: u16, program: u8, name: String, patch: Op505Patch },
}

impl BankOp {
    /// 逆操作（Undo適用時に使う。AddのUndoはRemove、RemoveのUndoはAdd）。
    pub fn inverse(&self) -> BankOp {
        match self {
            BankOp::Add { bank, program, name, patch } => {
                BankOp::Remove { bank: *bank, program: *program, name: name.clone(), patch: *patch }
            }
            BankOp::Remove { bank, program, name, patch } => {
                BankOp::Add { bank: *bank, program: *program, name: name.clone(), patch: *patch }
            }
        }
    }
}

/// Undo履歴1件。
enum UndoEntry {
    StateChange { before: EditorSnapshot, after: EditorSnapshot },
    BankChange { op: BankOp, before: EditorSnapshot, after: EditorSnapshot },
}

/// Undo/Redo実行時に呼び出し側が適用すべき内容。
pub struct UndoApply {
    /// 復元すべきエディタ状態。
    pub snapshot: EditorSnapshot,
    /// Some の場合、バンクレジストリへこの操作を適用してから`snapshot`を反映すること。
    pub bank_op: Option<BankOp>,
}

/// 音色エディタのUndo/Redoスタック。履歴に上限は設けない（メモリが許す限り。
/// 1エントリ数百バイト〜1KB程度のため実用上問題にならない）。圧縮もしない。
#[derive(Default)]
pub struct UndoStack {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    /// このフレーム開始時点のスナップショット（`begin_frame`が設定する）。
    frame_start: Option<EditorSnapshot>,
    /// 現在進行中の操作の「操作前」スナップショット。ドラッグ中はフレームをまたいで保持される。
    pending_before: Option<EditorSnapshot>,
    /// このフレームで`note_end_edit`が呼ばれたか。
    commit_requested: bool,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// フレーム先頭で呼ぶ。このフレームのウィジェット描画が始まる前のスナップショットを渡す。
    pub fn begin_frame(&mut self, snapshot: EditorSnapshot) {
        self.frame_start = Some(snapshot);
        self.commit_requested = false;
    }

    /// ハンドルの`begin_edit()`から呼ぶ。既に操作中（`pending_before`が埋まっている）なら何もしない。
    pub fn note_begin_edit(&mut self) {
        if self.pending_before.is_none() {
            self.pending_before = self.frame_start.clone();
        }
    }

    /// ハンドルの`end_edit()`から呼ぶ。
    pub fn note_end_edit(&mut self) {
        self.commit_requested = true;
    }

    /// フレーム末尾で呼ぶ。このフレームのウィジェット描画が終わった後のスナップショットを渡す。
    /// `note_end_edit`が呼ばれていれば、`pending_before`と比較し、差があれば1エントリ積んで
    /// redoスタックをクリアする。
    pub fn end_frame(&mut self, snapshot: EditorSnapshot) {
        if self.commit_requested {
            if let Some(before) = self.pending_before.take() {
                if before != snapshot {
                    self.undo.push(UndoEntry::StateChange { before, after: snapshot });
                    self.redo.clear();
                }
            }
        }
        self.commit_requested = false;
    }

    /// バンク操作（+New Voice/Delete）を1エントリとして積む。呼び出し側は操作の適用と同じ
    /// フレームで呼ぶこと（`before`/`after`は呼び出し側が構築したスナップショット）。
    pub fn push_bank_change(&mut self, op: BankOp, before: EditorSnapshot, after: EditorSnapshot) {
        self.undo.push(UndoEntry::BankChange { op, before, after });
        self.redo.clear();
    }

    /// Open/Save/Save Asで呼ぶ。履歴を空にする。
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending_before = None;
        self.commit_requested = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> Option<UndoApply> {
        let entry = self.undo.pop()?;
        let apply = match &entry {
            UndoEntry::StateChange { before, .. } => UndoApply { snapshot: before.clone(), bank_op: None },
            UndoEntry::BankChange { op, before, .. } => UndoApply { snapshot: before.clone(), bank_op: Some(op.inverse()) },
        };
        self.redo.push(entry);
        Some(apply)
    }

    pub fn redo(&mut self) -> Option<UndoApply> {
        let entry = self.redo.pop()?;
        let apply = match &entry {
            UndoEntry::StateChange { after, .. } => UndoApply { snapshot: after.clone(), bank_op: None },
            UndoEntry::BankChange { op, after, .. } => UndoApply { snapshot: after.clone(), bank_op: Some(op.clone()) },
        };
        self.undo.push(entry);
        Some(apply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param_spec::OpIndex;

    fn snapshot(tl: u8, program: u8) -> EditorSnapshot {
        let mut patch = Op505Patch::default();
        patch.operators[0].tl = tl;
        EditorSnapshot {
            patch,
            master: MasterEffectsState::default(),
            patch_name: "Voice".to_string(),
            bank: 0,
            program,
            has_selection: true,
        }
    }

    #[test]
    fn begin_then_change_then_end_pushes_one_entry() {
        let mut stack = UndoStack::new();
        stack.begin_frame(snapshot(10, 0));
        stack.note_begin_edit();
        stack.note_end_edit();
        stack.end_frame(snapshot(20, 0));

        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn begin_then_end_without_change_pushes_nothing() {
        let mut stack = UndoStack::new();
        stack.begin_frame(snapshot(10, 0));
        stack.note_begin_edit();
        stack.note_end_edit();
        stack.end_frame(snapshot(10, 0));

        assert!(!stack.can_undo(), "値が変わっていないので記録されない");
    }

    #[test]
    fn orphaned_end_pushes_nothing() {
        let mut stack = UndoStack::new();
        stack.begin_frame(snapshot(10, 0));
        // note_begin_editを呼ばずにnote_end_editだけ呼ぶ（孤立end）。
        stack.note_end_edit();
        stack.end_frame(snapshot(20, 0));

        assert!(!stack.can_undo(), "対応するbeginが無いendは記録されない");
    }

    #[test]
    fn double_begin_keeps_first_before() {
        let mut stack = UndoStack::new();
        stack.begin_frame(snapshot(10, 0));
        stack.note_begin_edit();
        stack.note_begin_edit(); // 二重begin
        stack.note_end_edit();
        stack.end_frame(snapshot(30, 0));

        let apply = stack.undo().expect("記録されているはず");
        assert_eq!(apply.snapshot.patch.operators[0].tl, 10, "最初のbeforeが保持される");
    }

    #[test]
    fn drag_spanning_multiple_frames_records_once() {
        let mut stack = UndoStack::new();
        // フレームN: ドラッグ開始
        stack.begin_frame(snapshot(10, 0));
        stack.note_begin_edit();
        stack.end_frame(snapshot(15, 0)); // まだend_editは呼ばれていない

        assert!(!stack.can_undo(), "ドラッグ継続中はまだ記録されない");

        // フレームN+1: ドラッグ継続（begin_editは呼ばれない）
        stack.begin_frame(snapshot(15, 0));
        stack.end_frame(snapshot(18, 0));

        assert!(!stack.can_undo());

        // フレームN+2: ドラッグ終了
        stack.begin_frame(snapshot(18, 0));
        stack.note_end_edit();
        stack.end_frame(snapshot(20, 0));

        assert!(stack.can_undo());
        let apply = stack.undo().expect("記録されているはず");
        assert_eq!(apply.snapshot.patch.operators[0].tl, 10, "ドラッグ開始前の値へ戻る");
    }

    #[test]
    fn undo_redo_round_trip_restores_state() {
        let mut stack = UndoStack::new();
        stack.begin_frame(snapshot(10, 0));
        stack.note_begin_edit();
        stack.note_end_edit();
        stack.end_frame(snapshot(20, 0));

        let undone = stack.undo().unwrap();
        assert_eq!(undone.snapshot.patch.operators[0].tl, 10);
        assert!(stack.can_redo());

        let redone = stack.redo().unwrap();
        assert_eq!(redone.snapshot.patch.operators[0].tl, 20);
        assert!(stack.can_undo());
    }

    #[test]
    fn clear_empties_both_stacks() {
        let mut stack = UndoStack::new();
        stack.begin_frame(snapshot(10, 0));
        stack.note_begin_edit();
        stack.note_end_edit();
        stack.end_frame(snapshot(20, 0));
        stack.undo();

        stack.clear();

        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn bank_add_undo_redo_inverts_correctly() {
        let mut stack = UndoStack::new();
        let before = snapshot(10, 0);
        let mut after = snapshot(10, 1);
        after.patch_name = "Voice001".to_string();
        let op = BankOp::Add { bank: 0, program: 1, name: "Voice001".to_string(), patch: after.patch };

        stack.push_bank_change(op, before.clone(), after.clone());

        let undone = stack.undo().unwrap();
        assert_eq!(undone.snapshot.program, 0, "Undoで元のprogramへ戻る");
        match undone.bank_op {
            Some(BankOp::Remove { program, .. }) => assert_eq!(program, 1, "AddのUndoはRemove"),
            _ => panic!("BankOp::Removeが期待される"),
        }

        let redone = stack.redo().unwrap();
        assert_eq!(redone.snapshot.program, 1);
        match redone.bank_op {
            Some(BankOp::Add { program, .. }) => assert_eq!(program, 1),
            _ => panic!("BankOp::Addが期待される"),
        }
    }

    #[test]
    fn diff_to_identical_snapshots_is_empty() {
        let s = snapshot(10, 0);
        let diff = s.diff_to(&s);
        assert!(diff.ints.is_empty());
        assert!(diff.bools.is_empty());
        assert!(diff.egs.is_empty());
    }

    #[test]
    fn diff_to_single_int_field_change() {
        let before = snapshot(10, 0);
        let mut after = before.clone();
        after.patch.channel.feedback = 200;

        let diff = before.diff_to(&after);
        assert_eq!(diff.ints, vec![IntField::Patch(PatchInt::Feedback)]);
        assert!(diff.bools.is_empty());
        assert!(diff.egs.is_empty());
    }

    #[test]
    fn diff_to_single_bool_field_change() {
        let before = snapshot(10, 0);
        let mut after = before.clone();
        after.patch.channel.fixed_note_enable = !before.patch.channel.fixed_note_enable;

        let diff = before.diff_to(&after);
        assert!(diff.ints.is_empty());
        assert_eq!(diff.bools, vec![BoolField::FixedNoteEnable]);
        assert!(diff.egs.is_empty());
    }

    #[test]
    fn diff_to_single_eg_slot_change() {
        let before = snapshot(10, 0);
        let mut after = before.clone();
        after.patch.operators[2].eg.stage_count = after.patch.operators[2].eg.stage_count.wrapping_add(1);

        let diff = before.diff_to(&after);
        assert!(diff.ints.is_empty());
        assert!(diff.bools.is_empty());
        assert_eq!(diff.egs, vec![EgSlot::Op(OpIndex::Op3)]);
    }

    #[test]
    fn diff_to_master_effects_change_reports_fx_field() {
        let before = snapshot(10, 0);
        let mut after = before.clone();
        after.master.rev_send = after.master.rev_send.wrapping_add(1);

        let diff = before.diff_to(&after);
        assert_eq!(diff.ints, vec![IntField::Fx(FxInt::RevSend)]);
        assert!(diff.bools.is_empty());
        assert!(diff.egs.is_empty());
    }
}
