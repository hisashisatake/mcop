//! 名前付きパイプ`\\.\pipe\op505.mme.v1`経由でop505-standaloneへ標準MIDIバイト列を送信する。
//! フレーム形式・ライタースレッド構成は`op505/mme-driver/src/client.rs`をそのまま踏襲する
//! （standaloneの`sources/pipe_src.rs`は接続元を区別しないため無改造で受けられる）。
//!
//! gesture-appはジェスチャーをMIDIへ変換してstandaloneへ送るだけのコントローラーであり、
//! エンジン・音声出力は一切持たない（詳細はCLAUDE.md gesture-app節、
//! memory `project_gesture_app_controller_roadmap.md`参照）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Mutex, Once, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const PIPE_PATH: &str = r"\\.\pipe\op505.mme.v1";
const CHANNEL_CAPACITY: usize = 256;
const RECONNECT_INTERVAL: Duration = Duration::from_millis(200);

const FRAME_VERSION: u8 = 1;
const FRAME_KIND_SHORT: u8 = 0;
/// トレイ起動音色エディタを開く/フォーカスする制御フレーム（payload無し）。
/// `op505/standalone/src/sources/pipe_src.rs`のOpenEditorフレーム（kind=3）と対になる。
const FRAME_KIND_OPEN_EDITOR: u8 = 3;
const DEVICE_ID: u8 = 0; // サーバー側は無視する（pipe_src.rsの_device_id）ため固定値で十分。

static SENDER: OnceLock<SyncSender<Vec<u8>>> = OnceLock::new();
static WRITER_THREAD_INIT: Once = Once::new();

fn try_connect() -> Option<File> {
    OpenOptions::new().write(true).open(PIPE_PATH).ok()
}

/// 初回送信前に一度だけ呼ぶ。パイプへ未接続なら、gesture-appと同じワークスペースの
/// `target/<profile>/`に並んでいるはずの`op505-standalone.exe`を起動してみる
/// （単一インスタンスMutexがあるため既に起動済みでも二重起動にはならない）。
/// 見つからない/起動に失敗しても致命的ではない（ライタースレッドが200ms間隔で
/// 再接続を試み続けるだけ）ため、エラーはログのみに留める。
fn ensure_started() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if try_connect().is_some() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let Some(dir) = exe.parent() else { return };
        let standalone = dir.join("op505-standalone.exe");
        if !standalone.exists() {
            eprintln!("midi_out: op505-standalone.exe が見つかりません（{}）。手動で起動してください。", standalone.display());
            return;
        }
        if let Err(e) = std::process::Command::new(&standalone).spawn() {
            eprintln!("midi_out: op505-standalone.exeの起動に失敗しました: {e}");
        }
    });
}

fn ensure_writer_thread() {
    WRITER_THREAD_INIT.call_once(|| {
        let (tx, rx) = sync_channel::<Vec<u8>>(CHANNEL_CAPACITY);
        if SENDER.set(tx).is_err() {
            eprintln!("midi_out: SENDER already set (unexpected)");
        }
        std::thread::spawn(move || writer_loop(rx));
    });
}

fn writer_loop(rx: Receiver<Vec<u8>>) {
    let mut file = try_connect();
    loop {
        match rx.recv_timeout(RECONNECT_INTERVAL) {
            Ok(frame) => {
                if file.is_none() {
                    file = try_connect();
                }
                if let Some(f) = file.as_mut() {
                    if f.write_all(&frame).is_err() {
                        file = None;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if file.is_none() {
                    file = try_connect();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn build_frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(FRAME_VERSION);
    frame.push(kind);
    frame.push(DEVICE_ID);
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// フレームを送信キューへ積む。キューが溢れていれば黙って破棄する
/// （フロントエンドのイベントループをブロックしないため）。
fn send_frame(kind: u8, payload: &[u8]) {
    ensure_started();
    ensure_writer_thread();
    let Some(tx) = SENDER.get() else { return };
    if let Err(TrySendError::Full(_)) = tx.try_send(build_frame(kind, payload)) {
        eprintln!("midi_out: send queue full, dropping frame");
    }
}

/// 1〜3バイトのデコード済みMIDIメッセージを送信キューへ積む。
fn send_short(bytes: &[u8]) {
    send_frame(FRAME_KIND_SHORT, bytes);
}

pub fn note_on(channel: u8, note: u8, velocity: u8) {
    send_short(&[0x90 | (channel & 0x0F), note & 0x7F, velocity.clamp(1, 127)]);
}

pub fn note_off(channel: u8, note: u8) {
    send_short(&[0x80 | (channel & 0x0F), note & 0x7F, 0]);
}

pub fn control_change(channel: u8, cc: u8, value: u8) {
    send_short(&[0xB0 | (channel & 0x0F), cc & 0x7F, value & 0x7F]);
}

pub fn program_change(channel: u8, program: u8) {
    send_short(&[0xC0 | (channel & 0x0F), program & 0x7F]);
}

/// CC0(Bank Select MSB) + CC32(Bank Select LSB)。`bank`は14bit(0〜16383)。
pub fn bank_select(channel: u8, bank: u16) {
    let bank = bank.min(16383);
    control_change(channel, 0, (bank >> 7) as u8);
    control_change(channel, 32, (bank & 0x7F) as u8);
}

/// RPN(param_msb, param_lsb)を選択してData Entry MSB(CC6)へ`value`を書き込む
/// （CC101/100/6の3メッセージ）。LSBまで必要な14bit値はCC38(Data Entry LSB)を別途呼ぶこと。
pub fn rpn_data_entry(channel: u8, param_msb: u8, param_lsb: u8, value: u8) {
    control_change(channel, 101, param_msb & 0x7F);
    control_change(channel, 100, param_lsb & 0x7F);
    control_change(channel, 6, value & 0x7F);
}

/// NRPN(param_msb, param_lsb)を選択してData Entry MSB(CC6)へ`value`を書き込む
/// （CC99/98/6の3メッセージ）。
pub fn nrpn_data_entry(channel: u8, param_msb: u8, param_lsb: u8, value: u8) {
    control_change(channel, 99, param_msb & 0x7F);
    control_change(channel, 98, param_lsb & 0x7F);
    control_change(channel, 6, value & 0x7F);
}

/// op505-standaloneのトレイ起動音色エディタを開く（既に開いていればフォーカスするだけ）よう
/// 要求する。gesture-app側のEキー押下から呼ぶ制御フレーム（MIDIメッセージではない）。
pub fn open_editor() {
    send_frame(FRAME_KIND_OPEN_EDITOR, &[]);
}

// ─────────────────────────────────────────────
// MIDI Clock送信（タップテンポ）+ ステップシーケンサー土台（フェーズ3）
// gesture-appがマスターとなり24 PPQNのクロックパルス(0xF8)をstandaloneへ送り続ける。
// standalone側の`TempoClock`（`op505/standalone/src/tempo_clock.rs`）がパルス間隔から
// BPMを算出しTimeEgのテンポ同期(sync_enabled)へ反映する。
// 同じループ内で拍を数え、メトロノーム（GM2 note33=Click/34=Bell、ch9=リズムチャンネル）と
// 再生位置のフロントエンド通知（Tauriイベント`sequencer-tick`）を行う。JS側のリズム画面
// （フェーズ4）はこのイベントを購読して再生カーソルを描くだけで、刻み自体はRust側が持つ
// （JSタイマーは数十msの誤差が出るため）。
// ─────────────────────────────────────────────
const CLOCK_PPQN: u32 = 24;
const BEATS_PER_BAR: u32 = 4;
const CLOCKS_PER_BAR: u32 = CLOCK_PPQN * BEATS_PER_BAR;

/// GM2リズムチャンネル（standalone側の`op505-midi::rhythm`がch10＝0-indexed 9をGM2リズムと
/// 解釈する、CLAUDE.md「GM2リズムチャンネル」節参照）。
const RHYTHM_CHANNEL: u8 = 9;
const METRONOME_CLICK_NOTE: u8 = 33;
const METRONOME_BELL_NOTE: u8 = 34;
const METRONOME_VELOCITY: u8 = 100;

/// f32::to_bits()で格納。0は「未タップ（クロック未送出）」を表す番兵。
static CLOCK_BPM_BITS: AtomicU32 = AtomicU32::new(0);
static CLOCK_THREAD_INIT: Once = Once::new();
static METRONOME_ENABLED: AtomicBool = AtomicBool::new(false);
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

// ─────────────────────────────────────────────
// ステップシーケンサー（フェーズ4：リズム画面本体、グリッド解像度細分化で1パルス=1/96小節へ）
// 1小節=96パルス（1パルス=1/96小節、CLAUDE.mdのグリッド解像度細分化参照）。行は
// op505/tools/patchlab/python/gm2_drum_kit.pyのSTANDARD_KIT（メトロノーム用note33/34を
// 除く12音色）と対応させる。パターンの編集はJS側（rhythm-screen.ts）が発生源で、
// `set_rhythm_range`/`set_rhythm_rows`経由でここへミラーするだけ
// （読み出しはこのクロックスレッドのみ）。発音の判定・送信自体は必ずこのスレッドが
// 行う（JS側のrequestAnimationFrameは数十msの誤差が出るため刻みに使わない）。
// ─────────────────────────────────────────────
const RHYTHM_STEPS: usize = CLOCKS_PER_BAR as usize; // 96
/// 再生カーソル通知(`rhythm-step`)の間引き間隔。旧16分音符間隔(6パルス)をそのまま
/// 維持する（理由は`clock_loop`のドキュメントコメント参照）。発音判定自体は
/// パルス粒度のデータに合わせて毎パルス行う。
const RHYTHM_STEP_EVENT_PULSES: u32 = 6;

const RHYTHM_VELOCITY_NORMAL: u8 = 95;
const RHYTHM_VELOCITY_ACCENT: u8 = 127;
const RHYTHM_VELOCITY_WEAK: u8 = 55;

/// リズムパターン本体。毎パルス128ノートを走査すると重いため、`hit_counts[pulse]`
/// （そのパルスで鳴る非0ノート数）を同じMutex内に持ち、0ならO(1)でスキップできるように
/// する（set_rhythm_*側で差分更新、走査はclock_loopのみ）。
struct RhythmState {
    /// [note][pulse] = 0(消音)/1(通常)/2(アクセント)/3(弱)。GM2ノート番号(0〜127)で直接引く
    /// （フェーズ3で行の固定12個制約を撤廃。どのノート番号をどの見た目の行として表示するかは
    /// JS側（rhythm-screen.ts）だけの関心事にし、Rust側は行の概念を持たない）。
    pattern: [[u8; RHYTHM_STEPS]; 128],
    hit_counts: [u16; RHYTHM_STEPS],
}

static RHYTHM_STATE: OnceLock<Mutex<RhythmState>> = OnceLock::new();

fn rhythm_state() -> &'static Mutex<RhythmState> {
    RHYTHM_STATE.get_or_init(|| Mutex::new(RhythmState { pattern: [[0; RHYTHM_STEPS]; 128], hit_counts: [0; RHYTHM_STEPS] }))
}

fn apply_rhythm_level(state: &mut RhythmState, note: usize, pulse: usize, level: u8) {
    let old = state.pattern[note][pulse];
    if old == 0 && level != 0 {
        state.hit_counts[pulse] += 1;
    } else if old != 0 && level == 0 {
        state.hit_counts[pulse] -= 1;
    }
    state.pattern[note][pulse] = level;
}

/// 「見たまま＝鳴る」の粗い倍率での範囲編集用（`rhythm-screen.ts`のクリック処理が
/// 1マス分の範囲をまとめて書き換える際に呼ぶ）。[start, start+len)を`level`で一括上書きする。
pub fn set_rhythm_range(note: u8, start: u16, len: u16, level: u8) {
    let note = note as usize;
    if note >= 128 {
        return;
    }
    let level = level.min(3);
    let start = start as usize;
    let end = (start + len as usize).min(RHYTHM_STEPS);
    let mut state = rhythm_state().lock().unwrap();
    for pulse in start..end {
        apply_rhythm_level(&mut state, note, pulse, level);
    }
}

/// JSから受け取る1行分のバルク置換データ（`set_rhythm_rows`）。JS側はcamelCaseで
/// 送るため`rename_all`で変換する。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RhythmRowInput {
    note: u8,
    steps: Vec<u8>,
}

/// リズム画面の全行を一括で置き換える。Undo/Redo・ファイル読込で使う（96パルス化で
/// 差分invokeループだと1回あたり最大1000回超のinvokeが飛びうるため、バルク化した）。
pub fn set_rhythm_rows(rows: Vec<RhythmRowInput>) {
    let mut state = rhythm_state().lock().unwrap();
    state.pattern = [[0; RHYTHM_STEPS]; 128];
    state.hit_counts = [0; RHYTHM_STEPS];
    for row in rows {
        let note = row.note as usize;
        if note >= 128 {
            continue;
        }
        for (pulse, &level) in row.steps.iter().take(RHYTHM_STEPS).enumerate() {
            let level = level.min(3);
            state.pattern[note][pulse] = level;
            if level != 0 {
                state.hit_counts[pulse] += 1;
            }
        }
    }
}

// ─────────────────────────────────────────────
// ピアノロール（フェーズ6：メロディ画面本体、グリッド解像度細分化で1パルス=1/96小節へ）
// リズムの「行=固定グリッド」と違い、メロディは音オブジェクト（可変の開始位置・長さ・
// 音高を持つノート）のリストで表現する。1小節=96パルス（1パルス=1/96小節）、ループ長は
// 基本8小節（`MELODY_BARS`）。将来「8小節パーツを複数組み合わせる」構想があるため、
// 小節数はこの1定数に閉じ込めてある。
// 編集の発生源はJS側（melody-screen.ts）で、IDはJS側が採番してRustへ渡す
// （Rustが非同期でID発行するとJS側が往復待ちになるため、rhythmと同じfire-and-forget
// 方式に揃える）。
// ─────────────────────────────────────────────
const MELODY_CHANNEL: u8 = 1; // ch2（0-indexed）
const MELODY_STEPS_PER_BAR: u32 = CLOCKS_PER_BAR; // 96
const MELODY_BARS: u32 = 8;
const MELODY_TOTAL_STEPS: u32 = MELODY_STEPS_PER_BAR * MELODY_BARS; // 768
/// 再生カーソル通知(`melody-step`)の間引き間隔。旧8分音符間隔(12パルス)をそのまま
/// 維持する（毎パルス送出すると`handle.emit`頻度がタイミングクリティカルなclock
/// スレッド上で大幅に増え、既知のタイマー精度問題を悪化させうるため）。発音判定自体は
/// パルス粒度のデータに合わせて毎パルス行う（このスレッド最下部のメロディ処理参照）。
const MELODY_STEP_EVENT_PULSES: u32 = 12;

/// 音オブジェクト1個分。`level`はリズムと同じ3段階（1=通常/2=アクセント/3=弱）で、
/// 0(消音)は「このVecに存在しない」で表現するためここには出てこない。
#[derive(Clone, Copy)]
struct MelodyNote {
    id: u32,
    start_step: u16,
    length_steps: u16,
    pitch: u8,
    level: u8,
}

static MELODY_NOTES: OnceLock<Mutex<Vec<MelodyNote>>> = OnceLock::new();

fn melody_notes() -> &'static Mutex<Vec<MelodyNote>> {
    MELODY_NOTES.get_or_init(|| Mutex::new(Vec::new()))
}

/// メロディ画面でノートを新規作成したときに呼ばれる。`id`はJS側が採番した一意な値。
pub fn add_melody_note(id: u32, start_step: u16, length_steps: u16, pitch: u8, level: u8) {
    melody_notes().lock().unwrap().push(MelodyNote {
        id,
        start_step,
        length_steps: length_steps.max(1),
        pitch,
        level: level.clamp(1, 3),
    });
}

/// ノートの移動・リサイズ・音量サイクルで呼ばれる（全フィールドを丸ごと書き換える）。
/// `id`が見つからなければ何もしない。
pub fn update_melody_note(id: u32, start_step: u16, length_steps: u16, pitch: u8, level: u8) {
    let mut notes = melody_notes().lock().unwrap();
    if let Some(n) = notes.iter_mut().find(|n| n.id == id) {
        n.start_step = start_step;
        n.length_steps = length_steps.max(1);
        n.pitch = pitch;
        n.level = level.clamp(1, 3);
    }
}

/// DELキーでの削除。
pub fn delete_melody_note(id: u32) {
    melody_notes().lock().unwrap().retain(|n| n.id != id);
}

/// JSから受け取るノート1個分のバルク置換データ（`set_melody_notes`）。JS側の
/// `MelodyNote`型はcamelCase（startStep/lengthSteps）のため`rename_all`で変換する。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MelodyNoteInput {
    id: u32,
    start_step: u16,
    length_steps: u16,
    pitch: u8,
    level: u8,
}

/// メロディ画面の全ノートを一括で置き換える。Undo/Redo・ファイル読込で使う
/// （768パルス化で差分invokeループが編集していないUndo/Redoでも大量に飛びうるため、
/// バルク化した）。
pub fn set_melody_notes(notes_in: Vec<MelodyNoteInput>) {
    let mut notes = melody_notes().lock().unwrap();
    notes.clear();
    notes.extend(notes_in.into_iter().map(|n| MelodyNote {
        id: n.id,
        start_step: n.start_step,
        length_steps: n.length_steps.max(1),
        pitch: n.pitch,
        level: n.level.clamp(1, 3),
    }));
}

/// リズム画面・メロディ画面共通の再生/停止ボタンの状態。MIDI Clock自体（メトロノーム・
/// TimeEgテンポ同期が依存する）は止めず、両パターンの発音とステップ通知だけをゲートする。
/// 既定は停止中（テンポをタップしただけでは鳴らず、再生ボタンを押すまでパターンは待機する）。
static SEQUENCER_RUNNING: AtomicBool = AtomicBool::new(false);

/// リズム/メロディ画面共通の再生/停止ボタン。停止中はステップの発音・
/// `rhythm-step`/`melody-step`イベント送出を両方止める（クロック自体・メトロノームは
/// 影響を受けない）。停止した瞬間、鳴りっぱなしのメロディノートは`clock_loop`側で
/// note_offされる（`active_melody_notes`参照）。
pub fn set_sequencer_running(running: bool) {
    SEQUENCER_RUNNING.store(running, Ordering::Relaxed);
}

/// `sequencer-tick`イベントの送信先。`main.rs`の`setup`から一度だけ渡す。
pub fn set_app_handle(handle: AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

/// メトロノームのON/OFF。ドラムパッチはauto_release=1（GM2リズムキット共通仕様）のため
/// note_offのタイミングは音の長さに影響しない。
pub fn set_metronome_enabled(enabled: bool) {
    METRONOME_ENABLED.store(enabled, Ordering::Relaxed);
}

/// タップテンポで確定したBPMを設定する。初回呼び出し時にクロック送信スレッドを起動する。
pub fn set_clock_bpm(bpm: f32) {
    CLOCK_BPM_BITS.store(bpm.to_bits(), Ordering::Relaxed);
    ensure_clock_thread();
}

fn ensure_clock_thread() {
    CLOCK_THREAD_INIT.call_once(|| {
        std::thread::spawn(clock_loop);
    });
}

/// BPM未設定の間は100ms間隔で設定の有無だけポーリングし、設定後は24 PPQN間隔で
/// 0xF8(Timing Clock)を送り続ける。`thread::sleep`のジッターはstandalone側の
/// 移動平均で吸収される想定のため、高精度タイマーは使わない。
///
/// 位置は1本の`clock_total`（0〜`CLOCKS_PER_BAR * MELODY_BARS - 1`、メロディの
/// ループ全体をカバーする）で管理する。リズム・メトロノームは`clock_total % CLOCKS_PER_BAR`
/// （＝`clock_in_bar`、0〜95）を使い、これまで通り1小節ごとに繰り返す。拍の頭
/// （24クロックごと）でメトロノームのNote On/Offと`sequencer-tick`を送出する。
///
/// グリッド解像度細分化（1パルス=1/96小節）以降、リズム・メロディともパターンの
/// データ粒度がパルス単位になったため、発音判定自体は**毎パルス**行う
/// （`clock_in_bar`・`clock_total`をそのまま参照、旧来の6/12クロックごとの間引きは
/// 発音判定からは無くした）。一方、`rhythm-step`/`melody-step`イベントの送出頻度は
/// 旧来の間隔（6/12パルスごと）を維持する（毎パルス送出すると`handle.emit`頻度が
/// タイミングクリティカルなclockスレッド上で大幅に増え、既知のタイマー精度問題
/// （memory `project_bpm_supply_midi_clock.md`）を悪化させうるため）。ペイロードも旧スケール
/// （rhythm:0〜15、melody:0〜63）のまま据え置き、JS側の再生カーソル描画コードは
/// 無改修で済ませる。
fn clock_loop() {
    let mut clock_total: u32 = 0;
    // 発音中（note_on済みでまだnote_offしていない）メロディノートの(pitch, 終了ステップ)。
    // このスレッドだけが読み書きする（Mutex越しではない）ローカル状態。
    let mut active_melody_notes: Vec<(u8, u16)> = Vec::new();
    let mut was_running = false;
    loop {
        let bits = CLOCK_BPM_BITS.load(Ordering::Relaxed);
        if bits == 0 {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let bpm = f32::from_bits(bits);
        let clock_in_bar = clock_total % CLOCKS_PER_BAR;

        if clock_in_bar % CLOCK_PPQN == 0 {
            let beat_in_bar = clock_in_bar / CLOCK_PPQN;
            if METRONOME_ENABLED.load(Ordering::Relaxed) {
                let note = if beat_in_bar == 0 { METRONOME_BELL_NOTE } else { METRONOME_CLICK_NOTE };
                note_on(RHYTHM_CHANNEL, note, METRONOME_VELOCITY);
                note_off(RHYTHM_CHANNEL, note);
            }
            if let Some(handle) = APP_HANDLE.get() {
                let _ = handle.emit("sequencer-tick", beat_in_bar);
            }
        }

        let running = SEQUENCER_RUNNING.load(Ordering::Relaxed);

        if running {
            let pulse = clock_in_bar as usize; // 0..95、1パルス=1マス
            {
                let state = rhythm_state().lock().unwrap();
                if state.hit_counts[pulse] != 0 {
                    for note in 0..128usize {
                        let level = state.pattern[note][pulse];
                        if level == 0 {
                            continue;
                        }
                        let velocity = match level {
                            2 => RHYTHM_VELOCITY_ACCENT,
                            3 => RHYTHM_VELOCITY_WEAK,
                            _ => RHYTHM_VELOCITY_NORMAL,
                        };
                        note_on(RHYTHM_CHANNEL, note as u8, velocity);
                        note_off(RHYTHM_CHANNEL, note as u8);
                    }
                }
            }
            if clock_in_bar % RHYTHM_STEP_EVENT_PULSES == 0 {
                if let Some(handle) = APP_HANDLE.get() {
                    let _ = handle.emit("rhythm-step", pulse / RHYTHM_STEP_EVENT_PULSES as usize);
                }
            }
        }

        // 停止した瞬間、鳴りっぱなしのメロディノートを止める（リズムはnote_on直後に
        // note_offしているため、この種のケアは不要）。
        if was_running && !running {
            for (pitch, _) in active_melody_notes.drain(..) {
                note_off(MELODY_CHANNEL, pitch);
            }
        }
        was_running = running;

        if running {
            // clock_totalは既にMELODY_TOTAL_STEPS周期でラップ済み（ループ末尾参照）なので、
            // そのままパルス位置として使える。
            let melody_pulse = clock_total as u16;
            active_melody_notes.retain(|&(pitch, end_pulse)| {
                if end_pulse == melody_pulse {
                    note_off(MELODY_CHANNEL, pitch);
                    false
                } else {
                    true
                }
            });
            {
                let notes = melody_notes().lock().unwrap();
                for n in notes.iter().filter(|n| n.start_step == melody_pulse) {
                    let velocity = match n.level {
                        2 => RHYTHM_VELOCITY_ACCENT,
                        3 => RHYTHM_VELOCITY_WEAK,
                        _ => RHYTHM_VELOCITY_NORMAL,
                    };
                    note_on(MELODY_CHANNEL, n.pitch, velocity);
                    let end = (n.start_step + n.length_steps).min(MELODY_TOTAL_STEPS as u16);
                    active_melody_notes.push((n.pitch, end));
                }
            }
            if clock_total % MELODY_STEP_EVENT_PULSES == 0 {
                if let Some(handle) = APP_HANDLE.get() {
                    let _ = handle.emit("melody-step", (melody_pulse as u32 / MELODY_STEP_EVENT_PULSES) as u16);
                }
            }
        }

        send_short(&[0xF8]);
        let interval = Duration::from_secs_f32(60.0 / bpm / CLOCK_PPQN as f32);
        std::thread::sleep(interval);

        clock_total = (clock_total + 1) % (CLOCKS_PER_BAR * MELODY_BARS);
    }
}
