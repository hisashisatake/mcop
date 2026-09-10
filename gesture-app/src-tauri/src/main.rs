#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod midi_out;
mod query_client;

/// コード発音に使うMIDIチャンネル。1チャンネルへ最大8声を重ねて鳴らす。
/// 旧「声部ごとに固定チャンネル0〜3」方式は、C13など5音以上のテンションコードを
/// 鳴らせない上限が問題になったため廃止した（押し直し時の同音チョークは、同一
/// チャンネル内での同ノート再発音としてstandalone側が処理する）。
const CHORD_CHANNEL: u8 = 0;

/// マスターエフェクト系NRPN/CCの送信先チャンネル。`NRPN(0,1) Channel Effect Route`を
/// 誰も送らなければ全チャンネルの`effect_route_slot`は既定0のままなので、チャンネル0で
/// 送れば全チャンネルが使う共有MasterEffects（スロット0）に反映される。
const EFFECTS_CHANNEL: u8 = 0;

/// GM2のリズムバンク判定に使うBank Select MSBの特別な2値から導いた範囲・合図
/// （`op505-midi::rhythm::{RHYTHM_BANK_MSB, MELODIC_BANK_MSB}`と同じ値。gesture-appは
/// エンジンを持たずop505-midiへ依存しない方針のため、ここでは値だけを複製する）。
/// MSB=120（15360〜15487）はリズムバンク、MSB=121（15488〜）は「明示的な旋律復帰」の合図。
const RHYTHM_BANK_RANGE: std::ops::RangeInclusive<u16> = 15360..=15487;
/// 旋律復帰の合図として送るダミーBank（MSB=121, LSB=0）。
const MELODIC_ESCAPE_BANK: u16 = 15488;

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

/// 指定チャンネルへノートオンを送る。1チャンネルに複数音を重ねられるため、
/// 止める側（`note_off`）はノート番号を明示的に受け取る（標準MIDIと同じ責任分担で、
/// 「今どの音を鳴らしているか」はフロントエンドが持つ）。
#[tauri::command]
fn note_on(channel: u8, note: u8, velocity: u8) {
    midi_out::note_on(channel, note, velocity);
}

/// 指定チャンネルの指定ノートを止める。
#[tauri::command]
fn note_off(channel: u8, note: u8) {
    midi_out::note_off(channel, note);
}

/// 指定チャンネルの発音を全て止める（CC123 All Notes Off）。画面切り替え時や、
/// note_offを取りこぼしたときのパニック用。
#[tauri::command]
fn all_notes_off(channel: u8) {
    midi_out::control_change(channel, 123, 0);
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

/// コード発音チャンネルへBank Select + Program Changeを送る（次のnote-onから適用）。
/// 実際の音色解決はstandalone側が単独で行う（gesture-appはエンジンを持たないため、
/// 音色の有無を判断する材料自体を持たない。見つかったかどうかの表示は
/// `op505_query_program_name`で別途問い合わせる）。
///
/// standalone側のProgram Change解決には、GM1互換のため「一度リズムバンク(MSB=120)へ
/// 入ったチャンネルは、Bank Select MSB=121（旋律復帰の合図）が明示的に来ない限り
/// リズムのまま」という粘りルールがある（実機GM2準拠、`op505-midi::rhythm`参照）。
/// gesture-appのBank欄は生の数値をそのままMSB/LSBへ分解して送るだけのため、この粘りに
/// 引っかかると「Bank欄を0に戻してもリズムキット番号が変わるだけ」になってしまう。
/// 旋律バンクを選ぶときは、先に旋律復帰の合図を明示的に送ってから本来のBank/Programを
/// 送ることで、現在の状態に関わらず必ず狙った音色へ届くようにする
/// （ダミーのProgram Changeは次のnote-onより前に本物のProgram Changeで上書きされるため
/// 発音への影響は無い）。
#[tauri::command]
fn op505_set_program(bank: u16, program: u8) {
    if !RHYTHM_BANK_RANGE.contains(&bank) {
        midi_out::bank_select(CHORD_CHANNEL, MELODIC_ESCAPE_BANK);
        midi_out::program_change(CHORD_CHANNEL, 0);
    }
    midi_out::bank_select(CHORD_CHANNEL, bank);
    midi_out::program_change(CHORD_CHANNEL, program);
}

/// standaloneへ問い合わせて、指定チャンネルの現在の音色名を取得する。`status`は
/// `"resolved"`（名前解決済み）/`"not_found"`（standalone側がdefault_patchへフォールバック中）/
/// `"rhythm"`（リズムチャンネル、`program`にキット番号）/`"editing"`（トレイ起動音色エディタが
/// このチャンネルを編集中）/`"disconnected"`（standalone未接続・応答なし）のいずれか。
#[derive(serde::Serialize)]
struct ProgramInfoDto {
    bank: u16,
    program: u8,
    name: String,
    status: &'static str,
}

#[tauri::command]
fn op505_query_program_name(channel: u8) -> ProgramInfoDto {
    match query_client::query_program(channel) {
        Some(info) => {
            let status = match info.status {
                query_client::ProgramStatus::Resolved => "resolved",
                query_client::ProgramStatus::NotFound => "not_found",
                query_client::ProgramStatus::Rhythm => "rhythm",
                query_client::ProgramStatus::Editing => "editing",
            };
            ProgramInfoDto { bank: info.bank, program: info.program, name: info.name, status }
        }
        None => ProgramInfoDto { bank: 0, program: 0, name: String::new(), status: "disconnected" },
    }
}

/// op505-standaloneのトレイ起動音色エディタを開く（既に開いていればフォーカスする）。
/// gesture-appのEキー押下から呼ぶ。
#[tauri::command]
fn op505_open_editor() {
    midi_out::open_editor();
}

/// タップテンポで確定したBPMを送る。フロントエンド（main.js）がタップ間隔から算出した値を渡すだけで、
/// MIDI Clock(0xF8)の送出自体は`midi_out::set_clock_bpm`が起動するバックグラウンドスレッドが行う。
#[tauri::command]
fn tap_tempo(bpm: f32) {
    midi_out::set_clock_bpm(bpm);
}

/// リズム画面のメトロノームON/OFF。刻み自体は`midi_out::clock_loop`（タップテンポと同じ
/// MIDI Clockスレッド）が担うため、ここではフラグを立てるだけ。
#[tauri::command]
fn set_metronome_enabled(enabled: bool) {
    midi_out::set_metronome_enabled(enabled);
}

/// リズム画面のステップシーケンサーグリッドのクリックで呼ばれる。`level`は0(消音)〜
/// 3(弱)。パターンの発音判定自体は`midi_out::clock_loop`が持つため、ここでは
/// 共有パターンへ書き込むだけ。
#[tauri::command]
fn set_rhythm_step(row: u8, step: u8, level: u8) {
    midi_out::set_rhythm_step(row, step, level);
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            midi_out::set_app_handle(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            note_on,
            note_off,
            all_notes_off,
            set_master_effects,
            op505_set_performance_lfo,
            op505_set_program,
            op505_query_program_name,
            op505_open_editor,
            tap_tempo,
            set_metronome_enabled,
            set_rhythm_step,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
