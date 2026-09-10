//! トレイ起動音色エディタ（Step 1以降）とオーディオスレッドの間で受け渡す共有状態。
//!
//! `Op505Engine`/`MidiState`/`MasterEffects`自体はこのモジュールでもArc/Mutex化しない
//! （main.rsのモジュールdoc参照——オーディオスレッド単独所有という現行設計の最大の美点）。
//! ここに置くのは「オーディオスレッドが毎ブロック覗いて、変化していればキャッシュへ
//! 取り込む入力データ」だけであり、`op505-vst`の`cached_egs`/`shared_preset_bank`+
//! `try_read()`パターン（`op505/vst/src/lib.rs`参照）をstandalone向けに複製したもの。
//!
//! オーディオスレッドは`try_read()`のみを使い、取得に失敗したら（GUIスレッドと競合した
//! 稀なケース）dirtyフラグを再度立てて次ブロックで再挑戦する。書き込み側（GUIスレッド）は
//! `write()`でブロックしてよい——エディタ操作はリアルタイム制約が無いため。
//!
//! egui/eframeに依存しない（`op505-core`型のみを扱う）。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use op505_core::{Op505BankFile, Op505Patch, Op505PresetBank};
use op505_midi::ProgramSelection;
use sound_core::MeterBridge;

/// `edit_channel`の「編集対象なし」を表す番兵値。MIDIチャンネルは0〜15のため衝突しない。
pub const NO_EDIT_CHANNEL: u8 = 0xFF;

/// MASTER EFFECTS共有スロットのインデックス（`sound_core::MasterEffects`の該当setterと対応）。
/// 配列の要素数・並び順はGUI側（後続ステップで実装）と一致させること。
pub const FX_REVERB_SEND: usize = 0;
pub const FX_REVERB_TYPE: usize = 1;
pub const FX_REVERB_TIME: usize = 2;
pub const FX_CHORUS_SEND: usize = 3;
pub const FX_CHORUS_TYPE: usize = 4;
pub const FX_CHORUS_MOD_RATE: usize = 5;
pub const FX_CHORUS_MOD_DEPTH: usize = 6;
pub const FX_CHORUS_FEEDBACK: usize = 7;
pub const FX_CHORUS_SEND_TO_REVERB: usize = 8;
/// マスターボリューム（`sound_core::MasterOutput::set_volume`と対応）。既存9個の後に
/// 追加した欄のため末尾（9番目）に置く（`FxInt::ALL`と同じ並びを保つ）。
pub const FX_MASTER_VOLUME: usize = 9;
/// Delay/Panning Delayのテンポ同期（`sound_core::MasterEffects::set_reverb_delay_sync`/
/// `set_reverb_delay_sync_rate`と対応）。既存10個の後に追加した欄のため末尾に置く。
pub const FX_DELAY_SYNC: usize = 10;
pub const FX_DELAY_SYNC_RATE: usize = 11;
pub const FX_VALUE_COUNT: usize = 12;

/// トレイ起動音色エディタとオーディオスレッドの間で共有する状態。
///
/// `Op505Patch`/プリセットバンクは`RwLock`越しに丸ごと読み書きする。TimeEg部分だけを
/// 分離する動機（`op505-vst`が`#[persist]`状態と分けている理由、DAWパラメーター化すると
/// EG1点操作で29パラメーターへの書き込みが走る問題）はstandaloneに存在しないため、
/// `Op505Patch`（Copy）を1個の単位として扱うのが最小の設計。
pub struct SharedEditState {
    /// エディタが編集中のパッチ本体。GUIが書き込み、オーディオスレッドが
    /// `edit_channel`が指すチャンネルの`base_patch_for`をこれへ差し替える。
    patch: RwLock<Op505Patch>,
    patch_dirty: AtomicBool,

    /// エディタのPRESETSパネルがSaveした後の全プリセット集合。保存時のみ更新される
    /// （発音中の音への即時反映はしない、`main.rs`のapply_live呼び出し方針参照）。
    presets: RwLock<Op505PresetBank>,
    presets_dirty: AtomicBool,

    /// 現在エディタが編集対象としているMIDIチャンネル（0〜15）。`NO_EDIT_CHANNEL`なら
    /// 「編集対象なし」＝全チャンネルが従来どおりProgram Change解決のみで音色を決める。
    edit_channel: AtomicU8,

    /// MASTER EFFECTS 9値の共有シャドウ。ノブが動いた値だけをGUIが書き込む想定だが、
    /// このモジュールでは単純に9値全てを1回のdirtyで一括反映する（VSTの1シャドウ差分
    /// 検知と同型、複雑な部分集合更新は導入しない）。
    fx_values: [AtomicU8; FX_VALUE_COUNT],
    fx_slot: AtomicU8,
    fx_dirty: AtomicBool,

    /// マスター出力の計測値（オーディオスレッド⇄GUIの橋渡し）。`fx_values`等と違いdirty
    /// フラグは使わない——`MeterBridge`自体が`try_lock`ベースの橋渡しを既に実装しているため
    /// （`sound_core::MeterBridge`のdoc参照）。オーディオスレッド・GUIスレッド双方が
    /// この同じ`Arc`を共有する。
    master_meter: Arc<MeterBridge>,

    /// 各MIDIチャンネルの現在のProgram Change選択（`encode_program_selection`でu32へ詰めた
    /// もの）。gesture-appの音色名クエリ（`query_server`）向けの一方向の橋で、`master_meter`と
    /// 同じ理由でdirtyフラグは使わない——オーディオスレッドが毎ブロック`store(Relaxed)`する
    /// だけで、クエリスレッドは常に「その時点の最新値」を`load(Relaxed)`で読めば十分
    /// （1ブロック分古い値を読んでも実害が無い）。
    program_selections: [AtomicU32; 16],
}

/// [`SharedEditState::program_selections`]の1要素の符号化。タグビット(31)で
/// `Melodic`/`Rhythm`を区別する。bank(14bit)・program(7bit)・kit(7bit)はいずれも
/// 32bitに収まるサイズのため、下位ビットへそのまま詰める。
fn encode_program_selection(selection: ProgramSelection) -> u32 {
    match selection {
        ProgramSelection::Melodic { bank, program } => ((bank as u32) << 8) | program as u32,
        ProgramSelection::Rhythm { kit } => (1u32 << 31) | kit as u32,
    }
}

fn decode_program_selection(bits: u32) -> ProgramSelection {
    if bits & (1 << 31) != 0 {
        ProgramSelection::Rhythm { kit: (bits & 0x7f) as u8 }
    } else {
        ProgramSelection::Melodic { bank: ((bits >> 8) & 0x3fff) as u16, program: (bits & 0xff) as u8 }
    }
}

/// [`SharedEditState::resolve_program_name`]が返す、1チャンネル分の音色名解決結果。
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramInfo {
    pub bank: u16,
    pub program: u8,
    pub name: String,
    pub status: ProgramStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramStatus {
    /// `presets`から名前が見つかった。
    Resolved,
    /// 旋律チャンネルで該当(bank, program)が`presets`に無く、`default_patch`へフォールバックする
    /// （`MidiState::base_patch_for`と同じ意味論）。
    NotFound,
    /// リズムチャンネル（kit番号は`program`に入る）。
    Rhythm,
    /// このチャンネルはトレイ起動音色エディタが編集中で、Program Change選択とは無関係に
    /// `edit_patch`が鳴っている（`MidiState::base_patch_for`の`edit_channel`分岐と対応）。
    Editing,
}

impl SharedEditState {
    /// 初期値は現在の既定パッチ・起動時プリセット集合で初期化する。dirtyフラグは全てfalseで
    /// 始まるため、エディタを一度も開かない限りオーディオスレッド側の分岐は素通りする。
    pub fn new(default_patch: Op505Patch, presets: Op505PresetBank) -> Self {
        Self {
            patch: RwLock::new(default_patch),
            patch_dirty: AtomicBool::new(false),
            presets: RwLock::new(presets),
            presets_dirty: AtomicBool::new(false),
            edit_channel: AtomicU8::new(NO_EDIT_CHANNEL),
            fx_values: std::array::from_fn(|_| AtomicU8::new(0)),
            fx_slot: AtomicU8::new(0),
            fx_dirty: AtomicBool::new(false),
            master_meter: Arc::new(MeterBridge::new()),
            program_selections: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// マスター出力の計測値ブリッジを取得する（オーディオスレッド・GUIスレッド双方が使う）。
    pub fn master_meter(&self) -> Arc<MeterBridge> {
        self.master_meter.clone()
    }

    // ---- GUIスレッド側API（ブロッキング可） ----

    /// エディタからパッチを書き込む。次のオーディオブロックで取り込まれる。
    pub fn publish_patch(&self, patch: Op505Patch) {
        *self.patch.write().unwrap() = patch;
        self.patch_dirty.store(true, Ordering::Release);
    }

    /// エディタのPRESETSパネルがOpen/Save/Save As/+ New Voice/Deleteしたバンクを、音声側の
    /// 検索用`Op505PresetBank`へマージする（`op505-vst`の`publish_bank`と同じ操作、
    /// `Op505BankFile::as_presets_file()`が返す「このbankは今このファイルの内容が全て」を
    /// `Op505PresetBank::merge_file`が受け取り、該当bankを丸ごと置き換える）。
    pub fn publish_bank_file(&self, bank_file: &Op505BankFile) {
        self.presets.write().unwrap().merge_file(bank_file.as_presets_file());
        self.presets_dirty.store(true, Ordering::Release);
    }

    /// 編集対象チャンネルを設定する。`None`で「編集対象なし」に戻す
    /// （エディタを閉じるときは必ずこれを呼ぶこと）。
    pub fn set_edit_channel(&self, channel: Option<usize>) {
        let value = channel.map(|c| c as u8).unwrap_or(NO_EDIT_CHANNEL);
        self.edit_channel.store(value, Ordering::Release);
    }

    /// MASTER EFFECTS 9値をまとめて書き込む。
    pub fn publish_fx(&self, slot: u8, values: [u8; FX_VALUE_COUNT]) {
        for (cell, v) in self.fx_values.iter().zip(values.iter()) {
            cell.store(*v, Ordering::Relaxed);
        }
        self.fx_slot.store(slot, Ordering::Relaxed);
        self.fx_dirty.store(true, Ordering::Release);
    }

    /// エディタを開く際の初期値取得用。`take_patch_if_dirty`と違いdirtyフラグには触れない
    /// （「今の値を閲覧するだけ」であり「オーディオスレッドへ反映する」操作ではないため）。
    /// GUIスレッドからのブロッキング`read()`は許容する（オーディオスレッドは呼ばない）。
    pub fn current_patch(&self) -> Op505Patch {
        *self.patch.read().unwrap()
    }

    // ---- オーディオスレッド側API（try_readのみ、ブロックしない） ----

    /// 現在の編集対象チャンネル（`NO_EDIT_CHANNEL`なら`None`）。
    pub fn edit_channel(&self) -> Option<usize> {
        match self.edit_channel.load(Ordering::Acquire) {
            NO_EDIT_CHANNEL => None,
            ch => Some(ch as usize),
        }
    }

    /// dirtyが立っていれば`try_read()`でパッチを取り込む。取得に失敗したら（GUIスレッドと
    /// 競合した稀なケース）dirtyを立て直して次ブロックで再挑戦する。取り込んだ場合`Some`。
    pub fn take_patch_if_dirty(&self) -> Option<Op505Patch> {
        if !self.patch_dirty.swap(false, Ordering::Acquire) {
            return None;
        }
        match self.patch.try_read() {
            Ok(guard) => Some(*guard),
            Err(_) => {
                self.patch_dirty.store(true, Ordering::Release);
                None
            }
        }
    }

    /// dirtyが立っていれば`try_read()`でプリセットバンクを取り込む。
    /// 呼び出し側が`clone()`のコストを払う（Save操作は頻度が低いため許容する、
    /// `op505-vst`の`shared_preset_bank`と同じ判断）。
    pub fn take_presets_if_dirty(&self) -> Option<Op505PresetBank> {
        if !self.presets_dirty.swap(false, Ordering::Acquire) {
            return None;
        }
        match self.presets.try_read() {
            Ok(guard) => Some(guard.clone()),
            Err(_) => {
                self.presets_dirty.store(true, Ordering::Release);
                None
            }
        }
    }

    /// dirtyが立っていればMASTER EFFECTS 9値とスロット番号を取り込む。
    pub fn take_fx_if_dirty(&self) -> Option<(u8, [u8; FX_VALUE_COUNT])> {
        if !self.fx_dirty.swap(false, Ordering::Acquire) {
            return None;
        }
        let slot = self.fx_slot.load(Ordering::Relaxed);
        let values = std::array::from_fn(|i| self.fx_values[i].load(Ordering::Relaxed));
        Some((slot, values))
    }

    // ---- オーディオスレッド→クエリスレッドの橋（`master_meter`と同型、dirty不要） ----

    /// 16チャンネル分のProgram Change選択を公開する。`sync_editor_state`/`drain_midi_queue`より
    /// 後、オーディオコールバックの各ブロックで呼ぶ（PC未送信の初期状態も含め、常に最新の
    /// `state.channels[..].program_state`を反映させるため。呼び出し頻度が高くても
    /// `store(Relaxed)`16回はコストとして無視できる）。
    pub fn publish_program_selections(&self, selections: [ProgramSelection; 16]) {
        for (cell, sel) in self.program_selections.iter().zip(selections.iter()) {
            cell.store(encode_program_selection(*sel), Ordering::Relaxed);
        }
    }

    // ---- クエリスレッド側API（query_server.rsから呼ぶ。ブロックしてよい） ----

    /// 指定チャンネルの現在のProgram Change選択と、それが指す音色名を解決する。
    /// `presets`は`take_presets_if_dirty`と違いconsumeしない読み取りのため、GUIスレッドの
    /// `publish_bank_file`と競合してもこちらはブロックするだけで実害は無い
    /// （クエリスレッドはリアルタイム制約が無い）。
    pub fn resolve_program_name(&self, channel: usize) -> ProgramInfo {
        let bits = self.program_selections[channel.min(15)].load(Ordering::Relaxed);
        let selection = decode_program_selection(bits);

        if self.edit_channel() == Some(channel) {
            let (bank, program) = match selection {
                ProgramSelection::Melodic { bank, program } => (bank, program),
                ProgramSelection::Rhythm { kit } => (0, kit),
            };
            return ProgramInfo { bank, program, name: String::new(), status: ProgramStatus::Editing };
        }

        match selection {
            ProgramSelection::Melodic { bank, program } => {
                let presets = self.presets.read().unwrap();
                match presets.get(bank, program) {
                    Some(preset) => {
                        ProgramInfo { bank, program, name: preset.name.clone(), status: ProgramStatus::Resolved }
                    }
                    None => ProgramInfo { bank, program, name: String::new(), status: ProgramStatus::NotFound },
                }
            }
            ProgramSelection::Rhythm { kit } => {
                ProgramInfo { bank: 0, program: kit, name: String::new(), status: ProgramStatus::Rhythm }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MasterEffectsState::values_in_fx_order()`（`op505-editor`の`FxInt::ALL`順で組み立てる）
    /// が、この`shared.rs`が定義する`FX_*`定数の並びと一致することを凍結する。ずれると
    /// reverb_typeとreverb_timeが入れ替わって無音デバッグ地獄になる（`app.rs`のpublish_fx参照）。
    #[test]
    fn fx_int_order_matches_fx_constants() {
        use op505_editor::patch_source::MasterEffectsState;

        let state = MasterEffectsState {
            rev_send: 1,
            reverb_type: 2,
            reverb_time: 3,
            cho_send: 4,
            chorus_type: 5,
            chorus_mod_rate: 6,
            chorus_mod_depth: 7,
            chorus_feedback: 8,
            chorus_send_to_reverb: 9,
            master_volume: 10,
            delay_sync: 11,
            delay_sync_rate: 12,
        };
        let values = state.values_in_fx_order();
        assert_eq!(values.len(), FX_VALUE_COUNT);
        assert_eq!(values[FX_REVERB_SEND], 1);
        assert_eq!(values[FX_REVERB_TYPE], 2);
        assert_eq!(values[FX_REVERB_TIME], 3);
        assert_eq!(values[FX_CHORUS_SEND], 4);
        assert_eq!(values[FX_CHORUS_TYPE], 5);
        assert_eq!(values[FX_CHORUS_MOD_RATE], 6);
        assert_eq!(values[FX_CHORUS_MOD_DEPTH], 7);
        assert_eq!(values[FX_CHORUS_FEEDBACK], 8);
        assert_eq!(values[FX_CHORUS_SEND_TO_REVERB], 9);
        assert_eq!(values[FX_MASTER_VOLUME], 10);
        assert_eq!(values[FX_DELAY_SYNC], 11);
        assert_eq!(values[FX_DELAY_SYNC_RATE], 12);
    }

    #[test]
    fn unused_state_never_reports_dirty() {
        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        assert_eq!(shared.edit_channel(), None);
        assert!(shared.take_patch_if_dirty().is_none());
        assert!(shared.take_presets_if_dirty().is_none());
        assert!(shared.take_fx_if_dirty().is_none());
    }

    #[test]
    fn publish_patch_round_trips() {
        let mut patch = Op505Patch::default();
        patch.channel.pitch_fg.depth = 42;
        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        shared.publish_patch(patch);
        let taken = shared.take_patch_if_dirty().expect("dirty after publish");
        assert_eq!(taken.channel.pitch_fg.depth, 42);
        // 一度取り込んだら次はNoneに戻る。
        assert!(shared.take_patch_if_dirty().is_none());
    }

    #[test]
    fn edit_channel_round_trips() {
        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        shared.set_edit_channel(Some(3));
        assert_eq!(shared.edit_channel(), Some(3));
        shared.set_edit_channel(None);
        assert_eq!(shared.edit_channel(), None);
    }

    #[test]
    fn publish_bank_file_merges_into_presets() {
        use op505_core::{Op505PresetEntry, Op505PresetFile};
        use std::path::PathBuf;

        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        let file = Op505PresetFile::Presets {
            bank: 5,
            presets: vec![Op505PresetEntry { program: 2, name: "Test".to_string(), patch: Op505Patch::default() }],
        };
        let bank_file = Op505BankFile::from_loaded(PathBuf::from("dummy.op505"), file, 5);
        shared.publish_bank_file(&bank_file);
        let presets = shared.take_presets_if_dirty().expect("dirty after publish");
        assert!(presets.get(5, 2).is_some(), "マージされたエントリーが取り出せるはず");
        assert!(shared.take_presets_if_dirty().is_none());
    }

    #[test]
    fn program_selection_encoding_round_trips() {
        let melodic = ProgramSelection::Melodic { bank: 16383, program: 127 };
        assert_eq!(decode_program_selection(encode_program_selection(melodic)), melodic);

        let melodic_zero = ProgramSelection::Melodic { bank: 0, program: 0 };
        assert_eq!(decode_program_selection(encode_program_selection(melodic_zero)), melodic_zero);

        let rhythm = ProgramSelection::Rhythm { kit: 5 };
        assert_eq!(decode_program_selection(encode_program_selection(rhythm)), rhythm);
    }

    #[test]
    fn resolve_program_name_reflects_published_selection() {
        use op505_core::{Op505PresetEntry, Op505PresetFile};
        use std::path::PathBuf;

        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        let file = Op505PresetFile::Presets {
            bank: 3,
            presets: vec![Op505PresetEntry { program: 7, name: "TestPatch".to_string(), patch: Op505Patch::default() }],
        };
        let bank_file = Op505BankFile::from_loaded(PathBuf::from("dummy.op505"), file, 3);
        shared.publish_bank_file(&bank_file);
        shared.take_presets_if_dirty(); // main.rsのsync_editor_stateと同じく取り込んでおく

        let mut selections = [ProgramSelection::Melodic { bank: 0, program: 0 }; 16];
        selections[2] = ProgramSelection::Melodic { bank: 3, program: 7 };
        shared.publish_program_selections(selections);

        let info = shared.resolve_program_name(2);
        assert_eq!(info.status, ProgramStatus::Resolved);
        assert_eq!(info.name, "TestPatch");
        assert_eq!((info.bank, info.program), (3, 7));
    }

    #[test]
    fn resolve_program_name_not_found_when_preset_missing() {
        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        let mut selections = [ProgramSelection::Melodic { bank: 0, program: 0 }; 16];
        selections[0] = ProgramSelection::Melodic { bank: 99, program: 1 };
        shared.publish_program_selections(selections);

        let info = shared.resolve_program_name(0);
        assert_eq!(info.status, ProgramStatus::NotFound);
    }

    #[test]
    fn resolve_program_name_editing_overrides_selection() {
        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        shared.set_edit_channel(Some(4));
        let mut selections = [ProgramSelection::Melodic { bank: 0, program: 0 }; 16];
        selections[4] = ProgramSelection::Melodic { bank: 1, program: 2 };
        shared.publish_program_selections(selections);

        let info = shared.resolve_program_name(4);
        assert_eq!(info.status, ProgramStatus::Editing);
    }

    #[test]
    fn publish_fx_round_trips() {
        let shared = SharedEditState::new(Op505Patch::default(), Op505PresetBank::default());
        let mut values = [0u8; FX_VALUE_COUNT];
        values[FX_REVERB_SEND] = 200;
        shared.publish_fx(2, values);
        let (slot, taken) = shared.take_fx_if_dirty().expect("dirty after publish");
        assert_eq!(slot, 2);
        assert_eq!(taken[FX_REVERB_SEND], 200);
        assert!(shared.take_fx_if_dirty().is_none());
    }
}
