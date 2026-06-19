use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::operator::OperatorParams;
use crate::Ym38x6Patch;
use sound_core::AdsrParams;

/// 波形メモリ音色専用のBank Select番号。このバンクを選ぶと、Program番号(0〜127)から
/// `waveform_memory_params_for_program`で(波形, ADSR)を決定し、`waveform_memory_patch`で
/// 1オペレーター音色を生成する（0〜7=ビルトイン波形+ピアノ風ADSR、8〜15=ビルトイン波形+
/// リード風ADSR、16〜127=ユーザー波形スロット+デフォルトADSR）。GM2 Bank0(=0)や暫定
/// プレースホルダーと衝突しないよう、十分大きな予約値を用いる。
pub const WAVEFORM_MEMORY_BANK: u16 = 16383;

/// 旧WMS-1相当の「波形メモリ音色」を生成する。Algorithm 7(全並列・変調なし)で
/// OP1(operators[0])のみを可聴にし、OP2〜4はTL=0(≈-95dB、実質無音)でミュートする。
/// ADSRは`AdsrParams`(0〜255)をOP1のEGへ素直にマッピングする(AR=attack/D1R=decay/
/// D1L=sustain/RR=release、D2R=0で第2減衰なし)。チャンネル側はデフォルト
/// (フィルター全開・音色LFO無効)。WMS-1の線形ADSRとはOPM準拠カーブの分だけ触感が変わる。
pub fn waveform_memory_patch(waveform: u8, adsr: AdsrParams) -> Ym38x6Patch {
    let mut patch = Ym38x6Patch::default();
    patch.channel.algorithm = 7;

    // OP1: 唯一の可聴オペレーター。波形とADSRを反映する。
    patch.operators[0] = OperatorParams {
        tl: 255,
        ar: adsr.attack,
        d1r: adsr.decay,
        d2r: 0,
        d1l: adsr.sustain,
        rr: adsr.release,
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform,
        op_fine_tune: 128,
    };
    // OP2〜4: TL=0でミュート(Algorithm 7では全Opがキャリアのため、音を消すにはTLを最小にする)。
    let muted = OperatorParams { tl: 0, ..patch.operators[0] };
    patch.operators[1] = muted;
    patch.operators[2] = muted;
    patch.operators[3] = muted;
    patch
}

/// 波形メモリ専用バンク（Program 0〜7）の「ピアノ風」ADSR。
/// アタック即時(255)、緩やかな減衰(decay)で中程度のサスティンレベルへ落ち着く
/// （鍵盤を保持していても音量が下がっていく、減衰楽器寄りの質感）。
fn waveform_memory_piano_adsr() -> AdsrParams {
    AdsrParams { attack: 255, decay: 80, sustain: 120, release: 130 }
}

/// 波形メモリ専用バンク（Program 8〜15）の「リード風」ADSR。
/// decay=0・sustain=255で減衰せず、キーを保持する間は最大レベルを維持する
/// 無限サスティン（`lead_1_square_patch`と同じ考え方）。RRはピアノ風ADSRと同じ値にし、
/// キーオフ後の余韻の長さを揃える。
fn waveform_memory_lead_adsr() -> AdsrParams {
    AdsrParams { attack: 255, decay: 0, sustain: 255, release: 130 }
}

/// 波形メモリ専用バンクのProgram番号から`waveform_memory_patch`への
/// (waveform, ADSR)を決定する。
/// - 0〜7: ビルトイン波形(Program) + ピアノ風ADSR
/// - 8〜15: ビルトイン波形(Program-8) + リード風ADSR
/// - 16〜127: ユーザー波形スロット(Program) + デフォルトADSR
fn waveform_memory_params_for_program(program: u8) -> (u8, AdsrParams) {
    match program {
        0..=7 => (program, waveform_memory_piano_adsr()),
        8..=15 => (program - 8, waveform_memory_lead_adsr()),
        _ => (program, AdsrParams::default()),
    }
}

/// ユーザープリセットの読み込み元ディレクトリ（暫定）。
/// `%APPDATA%\ym38x6\presets`が存在すればそちらを使い、無ければExplorerで見つけやすい
/// `%USERPROFILE%\Documents\ym38x6\presets`にフォールバックする。
/// 本格的なプリセット管理UIができるまでの間の配置場所。
pub fn presets_dir() -> PathBuf {
    let appdata_dir = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("ym38x6")
        .join("presets");
    if appdata_dir.is_dir() {
        return appdata_dir;
    }
    std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Documents")
        .join("ym38x6")
        .join("presets")
}

/// Bank Select（CC0×128+CC32）とProgram Change（0〜127）から決定的にパッチを生成する
/// 暫定プレースホルダー。GM2準拠のBank0音色はym38x6-ml（フェーズ5、インバース合成）で、
/// Bank1以降のユーザープリセットはプリセットライブラリ（フェーズ5）で生成・管理する予定。
/// 実データができるまでの間、Bank/Programの値域を一通り確認できるダミーパッチを返す
/// （bank/programの値はseedとして使うのみで、bankによる音色の区別は未実装）。
pub fn placeholder_patch(bank: u16, program: u8) -> Ym38x6Patch {
    let seed = program.wrapping_add(bank as u8);

    let mut patch = Ym38x6Patch::default();
    patch.channel.algorithm = seed % 8;
    patch.channel.feedback = seed.wrapping_mul(2);
    patch.channel.filter_cutoff = 255;
    patch.channel.filter_self_oscillation = true;

    // tests::loud_patchと同じ「即音量最大・サスティン無限」の基本設定（聴感確認用）
    let base = OperatorParams {
        tl: 255,
        ar: 255,
        d1r: 0,
        d2r: 0,
        d1l: 255,
        rr: 255,
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: 128,
    };
    for (i, op) in patch.operators.iter_mut().enumerate() {
        *op = OperatorParams { waveform: seed.wrapping_add(i as u8) % 8, ..base };
    }
    patch
}

/// GM2 Bank0の一部音色について、手動チューニングしたパッチを返す。
/// ym38x6-mlによるML自動生成（フェーズ5「音色作成方針」）が完成するまでの間、
/// 動作確認用に少数の代表音色のみ手動で作成したもの。該当しないprogram番号は
/// Noneを返し、呼び出し側で`placeholder_patch`へフォールバックする。
pub fn gm2_bank0_patch(program: u8) -> Option<Ym38x6Patch> {
    match program {
        0 => Some(acoustic_grand_piano_patch()),
        4 => Some(electric_piano_1_patch()),
        80 => Some(lead_1_square_patch()),
        _ => None,
    }
}

/// Program 0: Acoustic Grand Piano。
/// Algorithm 4（(O1→O2)+(O3→O4)）で2つの倍音グループを構成し、片方をわずかに
/// デチューンしてコーラス感を出す。モジュレーター(O1/O3)はキャリアより速く減衰させ、
/// 打鍵直後だけ倍音が立つハンマーアタックを表現する。
fn acoustic_grand_piano_patch() -> Ym38x6Patch {
    let mut patch = Ym38x6Patch::default();
    patch.channel.algorithm = 4;
    patch.channel.feedback = 40;

    // O1: ペア1のモジュレーター（ハンマーアタックの倍音、速い減衰）
    patch.operators[0] = OperatorParams {
        tl: 200,
        ar: 255,
        d1r: 180,
        d2r: 80,
        d1l: 100,
        rr: 180,
        mul: 1,
        dt1: 128,
        ksr: 100,
        am_enable: false,
        velocity_sensitivity: 80,
        waveform: 0,
        op_fine_tune: 128,
    };
    // O2: ペア1のキャリア（基音、緩やかな自然減衰）
    patch.operators[1] = OperatorParams {
        tl: 255,
        ar: 255,
        d1r: 60,
        d2r: 30,
        d1l: 200,
        rr: 120,
        mul: 1,
        dt1: 128,
        ksr: 120,
        am_enable: false,
        velocity_sensitivity: 40,
        waveform: 0,
        op_fine_tune: 128,
    };
    // O3: ペア2のモジュレーター（高次倍音、わずかに高めデチューン）
    patch.operators[2] = OperatorParams {
        tl: 150,
        ar: 255,
        d1r: 200,
        d2r: 100,
        d1l: 60,
        rr: 200,
        mul: 2,
        dt1: 138,
        ksr: 100,
        am_enable: false,
        velocity_sensitivity: 100,
        waveform: 0,
        op_fine_tune: 128,
    };
    // O4: ペア2のキャリア（基音、わずかに低めデチューンでコーラス感）
    patch.operators[3] = OperatorParams {
        tl: 220,
        ar: 255,
        d1r: 70,
        d2r: 35,
        d1l: 180,
        rr: 130,
        mul: 1,
        dt1: 118,
        ksr: 120,
        am_enable: false,
        velocity_sensitivity: 30,
        waveform: 0,
        op_fine_tune: 128,
    };
    patch
}

/// Program 4: Electric Piano 1。
/// Algorithm 4（(O1→O2)+(O3→O4)）で、O1→O2をベル成分（高次倍音MUL=14 + 強いフィードバック
/// でメタリックな質感）、O3→O4をメインのトーン成分とする、DX7系E.PIANOの定番構成。
fn electric_piano_1_patch() -> Ym38x6Patch {
    let mut patch = Ym38x6Patch::default();
    patch.channel.algorithm = 4;
    patch.channel.feedback = 180;

    // O1: ベル成分のモジュレーター（高次倍音、フィードバック対象）
    patch.operators[0] = OperatorParams {
        tl: 230,
        ar: 255,
        d1r: 220,
        d2r: 150,
        d1l: 40,
        rr: 200,
        mul: 14,
        dt1: 128,
        ksr: 80,
        am_enable: false,
        velocity_sensitivity: 120,
        waveform: 0,
        op_fine_tune: 128,
    };
    // O2: ベル成分のキャリア
    patch.operators[1] = OperatorParams {
        tl: 160,
        ar: 255,
        d1r: 200,
        d2r: 120,
        d1l: 30,
        rr: 180,
        mul: 1,
        dt1: 128,
        ksr: 100,
        am_enable: false,
        velocity_sensitivity: 60,
        waveform: 0,
        op_fine_tune: 128,
    };
    // O3: メイン音のモジュレーター
    patch.operators[2] = OperatorParams {
        tl: 180,
        ar: 255,
        d1r: 100,
        d2r: 50,
        d1l: 150,
        rr: 150,
        mul: 1,
        dt1: 128,
        ksr: 90,
        am_enable: false,
        velocity_sensitivity: 50,
        waveform: 0,
        op_fine_tune: 128,
    };
    // O4: メイン音のキャリア
    patch.operators[3] = OperatorParams {
        tl: 255,
        ar: 255,
        d1r: 50,
        d2r: 25,
        d1l: 220,
        rr: 120,
        mul: 1,
        dt1: 128,
        ksr: 110,
        am_enable: false,
        velocity_sensitivity: 20,
        waveform: 0,
        op_fine_tune: 128,
    };
    patch
}

/// Program 80: Lead 1 (Sync)。
/// Algorithm 7（全並列）でハーフsin²(waveform=3)を3本デチューンして重ね、O4を1オクターブ上で
/// 薄く重ねる、デチューンユニゾン構成のシンセリード。d1l=255 + d2r=0でキーオン中は
/// 減衰しない（無限サスティン）。フィルターでわずかに角を取りつつ共振を効かせる。
fn lead_1_square_patch() -> Ym38x6Patch {
    let mut patch = Ym38x6Patch::default();
    patch.channel.algorithm = 7;
    patch.channel.feedback = 0;
    patch.channel.filter_cutoff = 180;
    patch.channel.filter_resonance = 60;

    let base = OperatorParams {
        tl: 200,
        ar: 255,
        d1r: 0,
        d2r: 0,
        d1l: 255,
        rr: 80,
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 40,
        waveform: 3,
        op_fine_tune: 128,
    };
    patch.operators[0] = OperatorParams { tl: 255, ..base };
    patch.operators[1] = OperatorParams { dt1: 138, ..base };
    patch.operators[2] = OperatorParams { dt1: 118, ..base };
    patch.operators[3] =
        OperatorParams { mul: 2, tl: 140, velocity_sensitivity: 20, ..base };
    patch
}

/// 1音色分のプリセット（名前 + パッチ本体）。`PresetBank`が`(bank, program)`ごとに保持する
/// 内部値型（拡張波形スロット(8〜255)データの埋め込みは、当該機能実装後にフォーマット拡張で対応）。
#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub patch: Ym38x6Patch,
}

/// `.38x6`ファイル内の1プリセット。`bank`は`PresetFile`側で指定され、
/// `program`（Program Change、0〜127）でアドレスを持つ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresetEntry {
    pub program: u8,
    pub name: String,
    pub patch: Ym38x6Patch,
}

/// `.38x6`ファイルの内容。`bank`（Bank Select相当、CC0×128+CC32、0〜16383）と、
/// `presets`/`programs`いずれかのエントリー配列を持つ。
/// - `Presets`（`{"bank":..,"presets":[...]}`）: ロード時にこの`bank`のプリセットのみ
///   初期化して、これらのエントリーで再構築する（他bankは保持される）
/// - `Programs`（`{"bank":..,"programs":[...]}`）: 初期化せず、(bank,program)単位で
///   これらのエントリーを上書きマージする
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PresetFile {
    Presets { bank: u16, presets: Vec<PresetEntry> },
    Programs { bank: u16, programs: Vec<PresetEntry> },
}

impl PresetFile {
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

/// ディレクトリから読み込んだ`.38x6`プリセット集合。Bank Select + Program Changeでの
/// ルックアップに使う。
#[derive(Clone, Debug, Default)]
pub struct PresetBank {
    presets: HashMap<(u16, u8), Preset>,
}

impl PresetBank {
    /// 指定ディレクトリ内の`.38x6`ファイルをファイル名昇順で読み込み、
    /// `PresetFile::Presets`/`Programs`の意味に従ってプリセット集合を構築する。
    /// - `Presets`: そのbankのプリセットのみ初期化し、これらのエントリーで再構築する
    /// - `Programs`: 初期化せず、(bank,program)単位でエントリーを上書きマージする
    /// 同じ(bank,program)が複数回指定された場合は、後から読み込まれたものが優先する。
    /// ディレクトリが存在しない・読めない場合は空の集合を返す
    /// （ユーザープリセット未作成時はplaceholder_patchへフォールバックするため、エラーにしない）。
    pub fn load_from_dir(dir: &Path) -> Self {
        let mut presets = HashMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self { presets };
        };

        let mut paths: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("38x6"))
            .collect();
        paths.sort();

        for path in paths {
            let Ok(json) = std::fs::read_to_string(&path) else { continue };
            let Ok(file) = PresetFile::from_json(&json) else { continue };
            match file {
                PresetFile::Presets { bank, presets: list } => {
                    presets.retain(|&(b, _), _| b != bank);
                    for entry in list {
                        presets.insert((bank, entry.program), Preset { name: entry.name, patch: entry.patch });
                    }
                }
                PresetFile::Programs { bank, programs: list } => {
                    for entry in list {
                        presets.insert((bank, entry.program), Preset { name: entry.name, patch: entry.patch });
                    }
                }
            }
        }
        Self { presets }
    }

    pub fn get(&self, bank: u16, program: u8) -> Option<&Preset> {
        self.presets.get(&(bank, program))
    }

    /// 全プリセットを (bank, program) 昇順でソートして返す（GUI一覧表示用）。
    pub fn sorted_entries(&self) -> Vec<((u16, u8), Preset)> {
        let mut entries: Vec<_> = self.presets.iter().map(|(&k, v)| (k, v.clone())).collect();
        entries.sort_by_key(|(k, _)| *k);
        entries
    }

    /// (bank, program)に対応するパッチを解決する（MIDI Program Change・VST3
    /// Programパラメーター・gesture-appのProgram選択コマンドから共通で使う）。
    /// 優先順位: ユーザープリセット(.38x6) > 波形メモリバンク(`WAVEFORM_MEMORY_BANK`、
    /// `waveform_memory_params_for_program`で(波形,ADSR)を決定し1オペレーター音色を生成) >
    /// GM2 Bank0手動チューニング(該当programのみ) > 暫定プレースホルダーパッチ
    pub fn patch_for_program(&self, bank: u16, program: u8) -> Ym38x6Patch {
        self.get(bank, program)
            .map(|preset| preset.patch)
            .or_else(|| {
                (bank == WAVEFORM_MEMORY_BANK).then(|| {
                    let (waveform, adsr) = waveform_memory_params_for_program(program);
                    waveform_memory_patch(waveform, adsr)
                })
            })
            .or_else(|| (bank == 0).then(|| gm2_bank0_patch(program)).flatten())
            .unwrap_or_else(|| placeholder_patch(bank, program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SoundEngine, Ym38x6Engine};

    #[test]
    fn algorithm_and_waveform_stay_in_range_for_all_programs() {
        for bank in [0u16, 1, 128] {
            for program in 0..=255u8 {
                let patch = placeholder_patch(bank, program);
                assert!(patch.channel.algorithm < 8);
                for op in patch.operators {
                    assert!(op.waveform < 8);
                }
            }
        }
    }

    #[test]
    fn placeholder_patch_is_audible() {
        for (bank, program) in [(0u16, 0u8), (0, 64), (1, 42), (128, 127)] {
            let mut engine = Ym38x6Engine::new(44100.0);
            engine.note_on_with_velocity(0, 440.0, 127, placeholder_patch(bank, program));
            let mut buf = vec![0.0f32; 512];
            engine.render(&mut buf, 1);
            assert!(buf.iter().all(|&s| s.is_finite()));
            assert!(buf.iter().any(|&s| s != 0.0), "bank={bank} program={program} is silent");
        }
    }

    #[test]
    fn gm2_bank0_patch_returns_some_for_implemented_programs_only() {
        assert!(gm2_bank0_patch(0).is_some());
        assert!(gm2_bank0_patch(4).is_some());
        assert!(gm2_bank0_patch(80).is_some());
        assert!(gm2_bank0_patch(1).is_none());
        assert!(gm2_bank0_patch(127).is_none());
    }

    #[test]
    fn gm2_bank0_patch_is_audible_and_finite() {
        for program in [0u8, 4, 80] {
            let mut engine = Ym38x6Engine::new(44100.0);
            let patch = gm2_bank0_patch(program).expect("implemented program");
            engine.note_on_with_velocity(0, 440.0, 100, patch);

            let mut buf = vec![0.0f32; 44100];
            engine.render(&mut buf, 1);
            assert!(buf.iter().all(|&s| s.is_finite()), "program {program}: non-finite sample");
            assert!(buf.iter().any(|&s| s != 0.0), "program {program} is silent");
        }
    }

    #[test]
    fn waveform_memory_patch_has_single_audible_operator() {
        let patch = waveform_memory_patch(3, AdsrParams::default());
        assert_eq!(patch.channel.algorithm, 7);
        assert!(patch.operators[0].tl > 0, "OP1 should be audible");
        assert_eq!(patch.operators[0].waveform, 3);
        for op in &patch.operators[1..] {
            assert_eq!(op.tl, 0, "OP2-4 should be muted");
        }
    }

    #[test]
    fn waveform_memory_patch_is_audible_and_finite() {
        for waveform in 0u8..8 {
            let mut engine = Ym38x6Engine::new(44100.0);
            engine.note_on_with_velocity(0, 440.0, 127, waveform_memory_patch(waveform, AdsrParams::default()));
            let mut buf = vec![0.0f32; 512];
            engine.render(&mut buf, 1);
            assert!(buf.iter().all(|&s| s.is_finite()), "waveform {waveform}: non-finite sample");
            assert!(buf.iter().any(|&s| s != 0.0), "waveform {waveform} is silent");
        }
    }

    #[test]
    fn patch_for_program_resolves_waveform_memory_bank() {
        let bank = PresetBank::default();

        // Program 0〜7: ビルトイン波形(Program) + ピアノ風ADSR（減衰してサスティンへ落ち着く）
        let piano = bank.patch_for_program(WAVEFORM_MEMORY_BANK, 2);
        assert_eq!(piano, waveform_memory_patch(2, waveform_memory_piano_adsr()));
        assert!(piano.operators[0].d1r > 0, "piano ADSR should decay");

        // Program 8〜15: ビルトイン波形(Program-8) + リード風ADSR（減衰なしの無限サスティン）
        let lead = bank.patch_for_program(WAVEFORM_MEMORY_BANK, 10);
        assert_eq!(lead, waveform_memory_patch(2, waveform_memory_lead_adsr()));
        assert_eq!(lead.operators[0].d1r, 0, "lead ADSR should not decay");
        assert_eq!(lead.operators[0].d1l, 255, "lead ADSR sustains at max level");

        // Program 16〜127: ユーザー波形スロット(Program) + デフォルトADSR
        let user_slot = bank.patch_for_program(WAVEFORM_MEMORY_BANK, 20);
        assert_eq!(user_slot, waveform_memory_patch(20, AdsrParams::default()));
    }

    #[test]
    fn preset_file_presets_json_round_trip() {
        let file = PresetFile::Presets {
            bank: 1,
            presets: vec![
                PresetEntry { program: 0, name: "A".to_string(), patch: placeholder_patch(1, 0) },
                PresetEntry { program: 1, name: "B".to_string(), patch: placeholder_patch(1, 1) },
            ],
        };
        let json = file.to_json().expect("serialize");
        assert!(json.contains("\"bank\""));
        assert!(json.contains("\"presets\""));
        let loaded = PresetFile::from_json(&json).expect("deserialize");
        assert_eq!(loaded, file);
    }

    #[test]
    fn preset_file_programs_json_round_trip() {
        let file = PresetFile::Programs {
            bank: 1,
            programs: vec![PresetEntry { program: 5, name: "C".to_string(), patch: placeholder_patch(1, 5) }],
        };
        let json = file.to_json().expect("serialize");
        assert!(json.contains("\"bank\""));
        assert!(json.contains("\"programs\""));
        let loaded = PresetFile::from_json(&json).expect("deserialize");
        assert_eq!(loaded, file);
    }

    #[test]
    fn preset_bank_load_from_dir_reads_matching_files_and_ignores_others() {
        let dir = std::env::temp_dir().join(format!("ym38x6_preset_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let file = PresetFile::Presets {
            bank: 1,
            presets: vec![PresetEntry {
                program: 64,
                name: "My Preset".to_string(),
                patch: placeholder_patch(1, 64),
            }],
        };
        std::fs::write(dir.join("preset.38x6"), file.to_json().unwrap()).unwrap();
        std::fs::write(dir.join("ignore.txt"), "not a preset").unwrap();

        let bank = PresetBank::load_from_dir(&dir);
        std::fs::remove_dir_all(&dir).expect("cleanup temp dir");

        let loaded = bank.get(1, 64).expect("preset should be loaded");
        assert_eq!(loaded.name, "My Preset");
        assert_eq!(loaded.patch, placeholder_patch(1, 64));
        assert!(bank.get(0, 0).is_none());
    }

    #[test]
    fn preset_bank_load_from_dir_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("ym38x6_preset_test_nonexistent_dir");
        let bank = PresetBank::load_from_dir(&dir);
        assert!(bank.get(0, 0).is_none());
    }

    #[test]
    fn preset_bank_load_from_dir_presets_reset_scoped_to_bank_and_programs_merge() {
        let dir = std::env::temp_dir().join(format!("ym38x6_preset_test_merge_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let bank1_base = PresetFile::Presets {
            bank: 1,
            presets: vec![
                PresetEntry { program: 0, name: "A".to_string(), patch: placeholder_patch(1, 0) },
                PresetEntry { program: 1, name: "B".to_string(), patch: placeholder_patch(1, 1) },
            ],
        };
        let bank2_base = PresetFile::Presets {
            bank: 2,
            presets: vec![PresetEntry { program: 0, name: "X".to_string(), patch: placeholder_patch(2, 0) }],
        };
        let bank1_override = PresetFile::Programs {
            bank: 1,
            programs: vec![PresetEntry { program: 1, name: "B2".to_string(), patch: placeholder_patch(1, 1) }],
        };
        std::fs::write(dir.join("00_bank1_base.38x6"), bank1_base.to_json().unwrap()).unwrap();
        std::fs::write(dir.join("01_bank2_base.38x6"), bank2_base.to_json().unwrap()).unwrap();
        std::fs::write(dir.join("02_bank1_override.38x6"), bank1_override.to_json().unwrap()).unwrap();

        let bank = PresetBank::load_from_dir(&dir);
        assert_eq!(bank.get(1, 0).unwrap().name, "A");
        assert_eq!(bank.get(1, 1).unwrap().name, "B2");
        assert_eq!(bank.get(2, 0).unwrap().name, "X");

        let bank1_reset = PresetFile::Presets {
            bank: 1,
            presets: vec![PresetEntry { program: 5, name: "C".to_string(), patch: placeholder_patch(1, 5) }],
        };
        std::fs::write(dir.join("03_bank1_reset.38x6"), bank1_reset.to_json().unwrap()).unwrap();

        let bank = PresetBank::load_from_dir(&dir);
        std::fs::remove_dir_all(&dir).expect("cleanup temp dir");

        assert!(bank.get(1, 0).is_none());
        assert!(bank.get(1, 1).is_none());
        assert_eq!(bank.get(1, 5).unwrap().name, "C");
        assert_eq!(bank.get(2, 0).unwrap().name, "X");
    }
}
