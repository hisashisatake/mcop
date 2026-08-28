//! `.op505`バンクファイルの「1バンク=1担当ファイル」レジストリ。`Op505PresetBank`
//! （`preset.rs`）がディレクトリ内の全`.op505`を1枚にマージした読み取り専用ビューなのに対し、
//! こちらは各bank番号が「どのファイルに属するか」を保持し、Open/Save/Save As/
//! + New Voice/Deleteのようなファイル単位の編集操作を可能にする。
//!
//! gesture-app（`gesture-app/src-tauri/src/op505_presets.rs`）が最初にこの仕組みを実装し、
//! op505-vstのPRESETSパネルへも同じ仕様を持たせるためにop505-coreへ昇格した。
//! 昇格の判断根拠はspec-fm.md 8章⑤と同型（共有状態がディスク上の`.op505`ファイルそのものであり、
//! 複製すると「gesture-appとVSTのSaveが食い違う」という、正誤の基準が無い乖離を生むため）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{op505_presets_dir, Op505Patch, Op505PresetBank, Op505PresetEntry, Op505PresetFile};

/// bank番号ごとの「担当ファイル」（そのbankを最後に定義したファイル）。Open/Save/Save Asが
/// 直接更新する。フィールドはprivate——ファイルパスと内容は必ずこの型のメソッド経由で
/// 一貫性を保って変更する（`entries_mut`を外部から直接操作させない）。
pub struct Op505BankFile {
    path: PathBuf,
    file: Op505PresetFile,
}

impl Op505BankFile {
    /// Openダイアログで選んだファイルから構築する。**ファイル自身が宣言しているbank番号は
    /// 無視し**、`bank`（今エディタで選択中のbank）へ丸ごと読み込む（gesture-appで
    /// ユーザー確認済みの既定仕様、`ym38x6`版から踏襲）。
    pub fn from_loaded(path: PathBuf, file: Op505PresetFile, bank: u16) -> Self {
        Op505BankFile { path, file: with_bank(file, bank) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name().and_then(|s| s.to_str())
    }

    pub fn bank(&self) -> u16 {
        bank_of(&self.file)
    }

    pub fn entries(&self) -> &[Op505PresetEntry] {
        entries(&self.file)
    }

    /// 現在の内容を常に`Presets` variantとして返す（ディスク上のファイル自身のvariantは
    /// 変更しない）。`Programs`は上書きマージのみでDeleteが伝播しないため、「このbankは
    /// 今このファイルの内容がすべて」を表現できるのは`Presets`だけ——音声側の
    /// `Op505PresetBank`へ反映する際は必ずこちらを経由する。
    pub fn as_presets_file(&self) -> Op505PresetFile {
        Op505PresetFile::Presets { bank: self.bank(), presets: entries(&self.file).to_vec() }
    }

    /// 既存エントリーへ上書き保存する（PRESETSパネルの「Save」用）。対象programが
    /// 存在しないバンクへは使えない（先にAdd/Open/Save Asが必要、という制約を型で表現）。
    pub fn upsert(&mut self, program: u8, name: String, patch: Op505Patch) -> Result<(), String> {
        let list = entries_mut(&mut self.file);
        let entry = list.iter_mut().find(|e| e.program == program).ok_or("保存先エントリが見つかりません")?;
        entry.patch = patch;
        entry.name = name;
        Ok(())
    }

    /// 新規エントリーを追加する（PRESETSリスト末尾の「+ New Voice」用）。program番号は
    /// 既存エントリーの最大値+1（空なら0）、名前は"VoiceNNN"。program番号はu8のため、
    /// 既に255まで埋まっている場合はエラーを返す。
    pub fn add_new_voice(&mut self, patch: Op505Patch) -> Result<Op505PresetEntry, String> {
        let list = entries_mut(&mut self.file);
        let next_program = match list.iter().map(|e| e.program).max() {
            Some(max) => max.checked_add(1).ok_or("これ以上音色を追加できません（program番号が上限に達しました）")?,
            None => 0,
        };
        let entry = Op505PresetEntry { program: next_program, name: format!("Voice{next_program:03}"), patch };
        list.push(entry.clone());
        list.sort_by_key(|e| e.program);
        Ok(entry)
    }

    /// 指定programのエントリーを削除する（DELETEキー用）。
    pub fn remove(&mut self, program: u8) -> Result<(), String> {
        let list = entries_mut(&mut self.file);
        let idx = list.iter().position(|e| e.program == program).ok_or("削除対象のエントリが見つかりません")?;
        list.remove(idx);
        Ok(())
    }

    /// 現在の内容を`path`（担当ファイル）へ上書き保存する。
    pub fn save(&self) -> Result<(), String> {
        let json = self.file.to_json().map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }

    /// Save Asダイアログで選んだ新規パスへ書き出す。`base_entries`（今のbankの担当ファイルが
    /// あればその全エントリー、無ければ空）を複製元とし、`program`のエントリーだけ
    /// 最新の内容へ差し替えて（無ければ追加して）丸ごと書き出す。
    pub fn write_as(
        patch: Op505Patch,
        name: String,
        bank: u16,
        program: u8,
        base_entries: &[Op505PresetEntry],
        path: PathBuf,
    ) -> Result<Self, String> {
        let mut presets: Vec<Op505PresetEntry> = base_entries.to_vec();
        match presets.iter_mut().find(|e| e.program == program) {
            Some(entry) => {
                entry.patch = patch;
                entry.name = name;
            }
            None => presets.push(Op505PresetEntry { program, name, patch }),
        }
        presets.sort_by_key(|e| e.program);
        let file = Op505PresetFile::Presets { bank, presets };
        let json = file.to_json().map_err(|e| e.to_string())?;
        std::fs::write(&path, &json).map_err(|e| e.to_string())?;
        Ok(Op505BankFile { path, file })
    }
}

pub type Op505BankRegistry = HashMap<u16, Op505BankFile>;

/// `dir`内の`.op505`ファイルをファイル名昇順で走査し、bank番号ごとに「最後に処理した
/// ファイル」をレジストリへ記録する。起動時（またはリロード時）に呼ぶ。
pub fn build_op505_registry(dir: &Path) -> Op505BankRegistry {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("op505"))
        .collect();
    paths.sort();

    let mut registry = HashMap::new();
    for path in paths {
        let Ok(json) = std::fs::read_to_string(&path) else { continue };
        let Ok(file) = Op505PresetFile::from_json(&json) else { continue };
        registry.insert(bank_of(&file), Op505BankFile { path, file });
    }
    registry
}

/// Open/Save Asダイアログの初期ディレクトリを決める。指定bankがレジストリに登録済みなら
/// その担当ファイルの親ディレクトリ、未登録なら`op505_presets_dir()`。
pub fn current_open_dir(registry: &Op505BankRegistry, bank: u16) -> PathBuf {
    registry.get(&bank).and_then(|bank_file| bank_file.path.parent().map(PathBuf::from)).unwrap_or_else(op505_presets_dir)
}

/// (bank, program)に対応するパッチを解決する（レジストリ優先、無ければ`Op505PresetBank`。
/// `.op505`には波形メモリ/GM2/プレースホルダーのような代替パッチが無いため、
/// どちらにも無ければ`None`を返す）。
pub fn resolve_patch(registry: &Op505BankRegistry, bank_state: &Op505PresetBank, bank: u16, program: u8) -> Option<Op505Patch> {
    registry
        .get(&bank)
        .and_then(|bank_file| entries(&bank_file.file).iter().find(|e| e.program == program).map(|e| e.patch))
        .or_else(|| bank_state.get(bank, program).map(|preset| preset.patch))
}

/// `Op505PresetFile`のPresets/Programsどちらのvariantでもエントリ一覧への参照を取り出す。
fn entries(file: &Op505PresetFile) -> &Vec<Op505PresetEntry> {
    match file {
        Op505PresetFile::Presets { presets, .. } => presets,
        Op505PresetFile::Programs { programs, .. } => programs,
    }
}

/// `entries`の可変参照版。
fn entries_mut(file: &mut Op505PresetFile) -> &mut Vec<Op505PresetEntry> {
    match file {
        Op505PresetFile::Presets { presets, .. } => presets,
        Op505PresetFile::Programs { programs, .. } => programs,
    }
}

/// `Op505PresetFile`のPresets/Programsどちらのvariantでもbank番号を取り出す。
fn bank_of(file: &Op505PresetFile) -> u16 {
    match file {
        Op505PresetFile::Presets { bank, .. } | Op505PresetFile::Programs { bank, .. } => *bank,
    }
}

/// `file`のbankフィールドを`bank`へ書き換えたものを返す（variant・エントリー内容はそのまま）。
fn with_bank(file: Op505PresetFile, bank: u16) -> Op505PresetFile {
    match file {
        Op505PresetFile::Presets { presets, .. } => Op505PresetFile::Presets { bank, presets },
        Op505PresetFile::Programs { programs, .. } => Op505PresetFile::Programs { bank, programs },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("op505_preset_registry_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_json(bank: u16, program: u8, name: &str) -> String {
        Op505PresetFile::Programs { bank, programs: vec![Op505PresetEntry { program, name: name.to_string(), patch: Op505Patch::default() }] }
            .to_json()
            .unwrap()
    }

    #[test]
    fn build_registry_maps_bank_to_its_file() {
        let dir = unique_temp_dir("registry_basic");
        std::fs::write(dir.join("a.op505"), sample_json(0, 5, "Foo")).unwrap();
        std::fs::write(dir.join("b.op505"), sample_json(1, 2, "Bar")).unwrap();

        let registry = build_op505_registry(&dir);
        assert_eq!(registry.get(&0).unwrap().path(), dir.join("a.op505"));
        assert_eq!(registry.get(&1).unwrap().path(), dir.join("b.op505"));
        assert!(registry.get(&2).is_none(), "存在しないbankは登録されない");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_registry_prefers_last_file_in_sorted_order() {
        let dir = unique_temp_dir("registry_precedence");
        std::fs::write(dir.join("a_first.op505"), sample_json(0, 0, "First")).unwrap();
        std::fs::write(dir.join("b_second.op505"), sample_json(0, 0, "Second")).unwrap();

        let registry = build_op505_registry(&dir);
        assert_eq!(registry.get(&0).unwrap().path(), dir.join("b_second.op505"), "後読みのファイルが優先されるべき");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn with_bank_overrides_bank_and_keeps_entries() {
        let presets =
            Op505PresetFile::Presets { bank: 3, presets: vec![Op505PresetEntry { program: 5, name: "Foo".to_string(), patch: Op505Patch::default() }] };
        let rebanked = with_bank(presets, 9);
        assert_eq!(bank_of(&rebanked), 9);
        assert_eq!(entries(&rebanked).len(), 1);
        assert_eq!(entries(&rebanked)[0].program, 5);

        let programs = Op505PresetFile::Programs { bank: 3, programs: vec![] };
        assert_eq!(bank_of(&with_bank(programs, 9)), 9);
    }

    #[test]
    fn add_new_voice_starts_at_zero_when_empty() {
        let mut bank_file = Op505BankFile { path: PathBuf::from("dummy.op505"), file: Op505PresetFile::Presets { bank: 0, presets: vec![] } };
        let entry = bank_file.add_new_voice(Op505Patch::default()).unwrap();
        assert_eq!(entry.program, 0);
        assert_eq!(entry.name, "Voice000");
        assert_eq!(bank_file.entries().len(), 1);
    }

    #[test]
    fn add_new_voice_uses_max_program_plus_one() {
        let mut bank_file = Op505BankFile {
            path: PathBuf::from("dummy.op505"),
            file: Op505PresetFile::Presets {
                bank: 0,
                presets: vec![
                    Op505PresetEntry { program: 3, name: "A".to_string(), patch: Op505Patch::default() },
                    Op505PresetEntry { program: 31, name: "B".to_string(), patch: Op505Patch::default() },
                ],
            },
        };
        let entry = bank_file.add_new_voice(Op505Patch::default()).unwrap();
        assert_eq!(entry.program, 32);
        assert_eq!(entry.name, "Voice032");
        assert_eq!(bank_file.entries().last().unwrap().program, 32, "sortされ末尾に来るはず");
    }

    #[test]
    fn add_new_voice_copies_given_patch() {
        let mut bank_file = Op505BankFile { path: PathBuf::from("dummy.op505"), file: Op505PresetFile::Presets { bank: 0, presets: vec![] } };
        let mut source = Op505Patch::default();
        source.operators[0].tl = 123;
        let entry = bank_file.add_new_voice(source).unwrap();
        assert_eq!(entry.patch.operators[0].tl, 123, "Shift+クリックのコピー元がそのまま複製されるはず");
    }

    #[test]
    fn add_new_voice_errors_when_program_exhausted() {
        let mut bank_file = Op505BankFile {
            path: PathBuf::from("dummy.op505"),
            file: Op505PresetFile::Presets { bank: 0, presets: vec![Op505PresetEntry { program: 255, name: "Last".to_string(), patch: Op505Patch::default() }] },
        };
        assert!(bank_file.add_new_voice(Op505Patch::default()).is_err());
    }

    #[test]
    fn upsert_errors_when_program_missing() {
        let mut bank_file = Op505BankFile { path: PathBuf::from("dummy.op505"), file: Op505PresetFile::Presets { bank: 0, presets: vec![] } };
        assert!(bank_file.upsert(0, "X".to_string(), Op505Patch::default()).is_err(), "存在しないprogramへのSaveはErrのはず");
    }

    #[test]
    fn as_presets_file_always_returns_presets_variant() {
        // Programs形式で読み込んだファイルでも、as_presets_file()はPresetsを返す
        // （音声側への反映がDelete伝播できる形に統一されることの根拠）。
        let bank_file = Op505BankFile {
            path: PathBuf::from("dummy.op505"),
            file: Op505PresetFile::Programs { bank: 1, programs: vec![Op505PresetEntry { program: 0, name: "X".to_string(), patch: Op505Patch::default() }] },
        };
        match bank_file.as_presets_file() {
            Op505PresetFile::Presets { bank, presets } => {
                assert_eq!(bank, 1);
                assert_eq!(presets.len(), 1);
            }
            Op505PresetFile::Programs { .. } => panic!("as_presets_fileは常にPresets variantを返すはず"),
        }
    }

    #[test]
    fn remove_then_as_presets_file_drops_entry() {
        let mut bank_file = Op505BankFile {
            path: PathBuf::from("dummy.op505"),
            file: Op505PresetFile::Presets {
                bank: 0,
                presets: vec![
                    Op505PresetEntry { program: 0, name: "A".to_string(), patch: Op505Patch::default() },
                    Op505PresetEntry { program: 1, name: "B".to_string(), patch: Op505Patch::default() },
                ],
            },
        };
        bank_file.remove(0).unwrap();
        let Op505PresetFile::Presets { presets, .. } = bank_file.as_presets_file() else { panic!("Presets variantのはず") };
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].program, 1, "削除したprogram=0はas_presets_file()の結果にも残っていないはず");
    }

    #[test]
    fn save_and_reload_round_trips() {
        let dir = unique_temp_dir("save_reload");
        let path = dir.join("roundtrip.op505");
        let mut bank_file = Op505BankFile { path: path.clone(), file: Op505PresetFile::Presets { bank: 7, presets: vec![] } };
        bank_file.add_new_voice(Op505Patch::default()).unwrap();
        bank_file.save().unwrap();

        let registry = build_op505_registry(&dir);
        std::fs::remove_dir_all(&dir).ok();

        let reloaded = registry.get(&7).expect("保存したbankが読み直せるはず");
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].name, "Voice000");
    }

    #[test]
    fn current_open_dir_falls_back_to_presets_dir_when_bank_unregistered() {
        let registry = Op505BankRegistry::new();
        assert_eq!(current_open_dir(&registry, 0), op505_presets_dir());
    }

    #[test]
    fn resolve_patch_prefers_registry_then_bank_state() {
        let mut registry = Op505BankRegistry::new();
        let mut patch_in_registry = Op505Patch::default();
        patch_in_registry.operators[0].tl = 111;
        registry.insert(
            0,
            Op505BankFile {
                path: PathBuf::from("dummy.op505"),
                file: Op505PresetFile::Presets { bank: 0, presets: vec![Op505PresetEntry { program: 3, name: "R".to_string(), patch: patch_in_registry }] },
            },
        );

        let dir = unique_temp_dir("resolve_patch");
        let mut patch_in_bank = Op505Patch::default();
        patch_in_bank.operators[0].tl = 222;
        std::fs::write(
            dir.join("b.op505"),
            Op505PresetFile::Presets { bank: 1, presets: vec![Op505PresetEntry { program: 4, name: "B".to_string(), patch: patch_in_bank }] }.to_json().unwrap(),
        )
        .unwrap();
        let bank_state = Op505PresetBank::load_from_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(resolve_patch(&registry, &bank_state, 0, 3).unwrap().operators[0].tl, 111, "レジストリ優先");
        assert_eq!(resolve_patch(&registry, &bank_state, 1, 4).unwrap().operators[0].tl, 222, "レジストリに無ければbank_state");
        assert!(resolve_patch(&registry, &bank_state, 9, 9).is_none(), "どちらにも無ければNone");
    }
}
