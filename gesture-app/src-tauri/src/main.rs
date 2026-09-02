#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod midi_out;
mod op505_presets;

use op505_core::{build_op505_registry, op505_presets_dir, Op505BankRegistry, Op505Patch, Op505PresetBank};
use std::sync::Mutex;

/// 発音に使うMIDIチャンネル（声部スロット0〜3）。コード最大4音（7th/maj7）に合わせた固定範囲。
/// `op505_set_program`はこの全チャンネルへBank Select+Program Changeを送る
/// （standalone側は自分が受け取ったチャンネルの`ChannelState`だけを更新するため、
/// 未使用チャンネルへ送っても実害はない）。
const VOICE_CHANNELS: std::ops::Range<u8> = 0..4;

/// マスターエフェクト系NRPN/CCの送信先チャンネル。`NRPN(0,1) Channel Effect Route`を
/// 誰も送らなければ全チャンネルの`effect_route_slot`は既定0のままなので、チャンネル0で
/// 送れば全チャンネルが使う共有MasterEffects（スロット0）に反映される。
const EFFECTS_CHANNEL: u8 = 0;

/// Destination（`0`〜`4`、NRPN(0,0)の生値と同じ並び）。旧`sound_fm::FmLfoDestination`は
/// 質感LFO退役に伴い削除済みのため、gesture-app内だけで使う最小限の解釈をここに持つ
/// （NRPN(0,0)と同じ生値: Unplugged=0/Pitch=1/Volume=2/TlCarrier=3/Cutoff=4）。
#[derive(Clone, Copy, PartialEq)]
enum PerformanceLfoDestination {
    Unplugged,
    Pitch,
    Volume,
    TlCarrier,
    Cutoff,
}

impl PerformanceLfoDestination {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Pitch,
            2 => Self::Volume,
            3 => Self::TlCarrier,
            4 => Self::Cutoff,
            _ => Self::Unplugged,
        }
    }
}

/// このプロジェクトの内部表現（0〜255）を7bit MIDI値（0〜127）へ丸める
/// （`op505_midi::value::cc_byte_to_u8`の逆変換、送信側の共通ヘルパー）。
fn scale_to_7bit(value: u8) -> u8 {
    ((value as u16 * 127 + 127) / 255) as u8
}

/// 発音中の声部（MIDIチャンネル0〜3）が最後にnote_onしたノート番号。note_offで
/// どのノートをNote Offすべきか引くために使う（MIDIのNote Offはノート番号が必要なため、
/// `note_off(channel)`だけを渡すフロントエンドとの橋渡し）。
type LastNotes = Mutex<[Option<u8>; 16]>;

/// 指定チャンネル(声部スロット)へノートオンを送る。`channel`は`note_off`と対にする
/// 安定したスロット番号（フロントエンド側の`activeChannels`参照）。
#[tauri::command]
fn note_on(channel: u8, note: u8, velocity: u8, last_notes: tauri::State<'_, LastNotes>) {
    let idx = (channel as usize).min(15);
    last_notes.lock().unwrap()[idx] = Some(note);
    midi_out::note_on(channel, note, velocity);
}

#[tauri::command]
fn note_off(channel: u8, last_notes: tauri::State<'_, LastNotes>) {
    let idx = (channel as usize).min(15);
    if let Some(note) = last_notes.lock().unwrap()[idx].take() {
        midi_out::note_off(channel, note);
    }
}

/// OP505の演奏系モジュレーション（Vキーのビブラート⇔トレモロ切替）をMIDIで送る。
/// `destination`は`PerformanceLfoDestination`の値（Unplugged=0/Pitch=1/Volume=2/TlCarrier=3/
/// Cutoff=4）。標準MIDIのCC1(Modulation)/CC76(Vibrato Rate)/CC77(Vibrato Depth)/
/// CC78(Vibrato Delay)/CC92(Tremolo Depth)とRPN(0,5)(Modulation Depth Range)へ変換する。
///
/// EGの形（ビブラート/トレモロ/オートワウの波形自体）はもう組み立てない——standalone側の
/// `op505-midi`が「プリセットが形を持たない(stage_count==0)かつCC由来のdepthが正」の
/// ときだけ標準形状を自動生成する（演奏用FGフォールバック、Step 2で実装済み）。
///
/// - Pitch: CC1へ強度を送る（未選択時は0＝preset本来のPitch FG深さのみが残る、CC77は
///   常時0起点加算のためこれで安全に「介入なし」に戻せる）。
/// - Volume/TlCarrier: CC92へ同じ強度を送る（CC92もCC77と同じく0起点加算）。
/// - Cutoff: NRPN(0,26) Cutoff FG Depthの絶対上書き。Cutoff FGには専用CCが無く絶対値でしか
///   動かせないため、**選択中のときだけ送る**（未選択時に0を送るとプリセットが自前で持つ
///   オートワウ深さまで消してしまう。他のNRPN上書きと同じ「Program Changeまで居座る」仕様に
///   委ねる。現状のUIはPitch/Volumeしか選ばないため実際には到達しない分岐）。
#[tauri::command]
fn op505_set_performance_lfo(channel: u8, rate: u8, delay: u8, destination: u8, cc77: u8, cc1: u8, mod_depth_range: u8) {
    let dest = PerformanceLfoDestination::from_u8(destination);
    let intensity = scale_to_7bit(cc1);

    midi_out::control_change(channel, 76, scale_to_7bit(rate));
    midi_out::control_change(channel, 78, scale_to_7bit(delay));
    midi_out::rpn_data_entry(channel, 0, 5, mod_depth_range.min(127));

    let pitch_active = dest == PerformanceLfoDestination::Pitch;
    midi_out::control_change(channel, 1, if pitch_active { intensity } else { 0 });
    midi_out::control_change(channel, 77, if pitch_active { scale_to_7bit(cc77) } else { 0 });

    let volume_active = matches!(dest, PerformanceLfoDestination::Volume | PerformanceLfoDestination::TlCarrier);
    midi_out::control_change(channel, 92, if volume_active { intensity } else { 0 });

    if dest == PerformanceLfoDestination::Cutoff {
        midi_out::nrpn_data_entry(channel, 0, 26, intensity);
    }
}

/// マスターエフェクト（Reverb/Chorus）をMIDIで送る。CC91/93（送りレベル）+
/// NRPN(0,2)〜(0,8)（Reverb Type/Chorus Type/Reverb Time/Chorus Mod Rate/Mod Depth/
/// Feedback/Send To Reverb）。`reverb_type`/`chorus_type`は0〜7（spec.md マスターエフェクト
/// セクションのenum参照）、それ以外は0〜255の内部表現。
#[tauri::command]
fn set_master_effects(
    reverb_send: u8,
    reverb_type: u8,
    reverb_time: u8,
    chorus_send: u8,
    chorus_type: u8,
    chorus_mod_rate: u8,
    chorus_mod_depth: u8,
    chorus_feedback: u8,
    chorus_send_to_reverb: u8,
) {
    midi_out::control_change(EFFECTS_CHANNEL, 91, scale_to_7bit(reverb_send));
    midi_out::control_change(EFFECTS_CHANNEL, 93, scale_to_7bit(chorus_send));
    midi_out::nrpn_data_entry(EFFECTS_CHANNEL, 0, 2, reverb_type.min(7));
    midi_out::nrpn_data_entry(EFFECTS_CHANNEL, 0, 3, chorus_type.min(7));
    midi_out::nrpn_data_entry(EFFECTS_CHANNEL, 0, 4, scale_to_7bit(reverb_time));
    midi_out::nrpn_data_entry(EFFECTS_CHANNEL, 0, 5, scale_to_7bit(chorus_mod_rate));
    midi_out::nrpn_data_entry(EFFECTS_CHANNEL, 0, 6, scale_to_7bit(chorus_mod_depth));
    midi_out::nrpn_data_entry(EFFECTS_CHANNEL, 0, 7, scale_to_7bit(chorus_feedback));
    midi_out::nrpn_data_entry(EFFECTS_CHANNEL, 0, 8, scale_to_7bit(chorus_send_to_reverb));
}

/// (bank, program)に対応する`.op505`プリセットが見つかれば、発音用の全チャンネルへ
/// Bank Select + Program Changeを送る（次のnote-onから適用。standalone側のProgram Change
/// 解決は`op505_presets_dir()`側のファイルを見るため、ローカルの`registry`/`bank_state`は
/// 「見つかったかどうか」の表示用チェックのみに使う）。
/// 見つからなければMIDIは送らずNoneを返す（`Op505Patch::default()`はtl=0で無音のため、
/// 黙って無音へ切り替えるより「見つからない」を呼び出し側に伝えて現在の音を維持するほうが安全）。
#[tauri::command]
fn op505_set_program(
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank_state: tauri::State<'_, Mutex<Op505PresetBank>>,
    bank: u16,
    program: u8,
) -> Option<Op505Patch> {
    let patch = op505_core::resolve_patch(&registry.lock().unwrap(), &bank_state.lock().unwrap(), bank, program)?;
    for ch in VOICE_CHANNELS {
        midi_out::bank_select(ch, bank);
        midi_out::program_change(ch, program);
    }
    Some(patch)
}

/// `op505_presets_dir()`から`.op505`プリセットを読み直し、読み込んだプリセット総数を返す。
/// アプリ起動中に外部（op505_probe/opz2op505等の変換ツールや手編集）でプリセットファイルが
/// 追加・更新された場合に、再起動せず反映するためのコマンド。`Op505PresetBank`（フォールバック用）
/// と`Op505BankRegistry`（表示用の担当ファイル管理）の両方を作り直す。
#[tauri::command]
fn op505_reload_presets(bank_state: tauri::State<'_, Mutex<Op505PresetBank>>, registry: tauri::State<'_, Mutex<Op505BankRegistry>>) -> usize {
    let dir = op505_presets_dir();
    let reloaded = Op505PresetBank::load_from_dir(&dir);
    let count = reloaded.sorted_entries().len();
    *bank_state.lock().unwrap() = reloaded;
    *registry.lock().unwrap() = build_op505_registry(&dir);
    count
}

/// op505-standaloneのトレイ起動音色エディタを開く（既に開いていればフォーカスする）。
/// gesture-appのEキー押下から呼ぶ。
#[tauri::command]
fn op505_open_editor() {
    midi_out::open_editor();
}

fn main() {
    // presets_dir()の読み込みは起動時にここで1回だけ行う（%APPDATA%\op505\presets）。
    // gesture-appはエンジンを持たない読み取り専用のBank/Program解決用途にのみこれを使う
    // （実際の音色解決はstandalone側が自分のpresets_dir()から独立に行う）。
    let op505_bank = Op505PresetBank::load_from_dir(&op505_presets_dir());
    let op505_registry = build_op505_registry(&op505_presets_dir());

    tauri::Builder::default()
        .manage(Mutex::new(op505_bank))
        .manage(Mutex::new(op505_registry))
        .manage(Mutex::new([None::<u8>; 16]))
        .invoke_handler(tauri::generate_handler![
            note_on,
            note_off,
            set_master_effects,
            op505_set_performance_lfo,
            op505_set_program,
            op505_reload_presets,
            op505_open_editor,
            op505_presets::op505_list_bank_entries,
            op505_presets::op505_get_bank_file_name,
            op505_presets::op505_get_bank_program,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
