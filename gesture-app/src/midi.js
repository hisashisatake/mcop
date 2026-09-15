// Tauriコマンドの薄いラッパー。gesture-appは音源を持たず、ここから送った
// MIDIメッセージを名前付きパイプ経由でop505-standaloneが受け取って鳴らす。

import { pushLog } from './midi-log.js';

// フォールバックでブラウザ単体でも開ける（Tauri外ではMIDIは飛ばない）
const invoke = window.__TAURI__?.core?.invoke ?? (async (_cmd, _args) => 0);
const tauriEvent = window.__TAURI__?.event;

/** コード発音チャンネル。src-tauri側の`CHORD_CHANNEL`と一致させること。 */
export const CHORD_CHANNEL = 0;

export function noteOn(channel, note, velocity) {
  pushLog(`ch${channel} note_on  note=${note} vel=${velocity}`);
  return invoke('note_on', { channel, note, velocity });
}

export function noteOff(channel, note) {
  pushLog(`ch${channel} note_off note=${note}`);
  return invoke('note_off', { channel, note });
}

export function allNotesOff(channel) {
  pushLog(`ch${channel} all_notes_off`);
  return invoke('all_notes_off', { channel });
}

export function setProgram(bank, program) {
  pushLog(`program_change bank=${bank} program=${program}`);
  return invoke('op505_set_program', { bank, program });
}

/** standaloneへ問い合わせて、指定チャンネルの現在の音色名を取得する。
 * `status`は"resolved"/"not_found"/"rhythm"/"editing"/"disconnected"のいずれか
 * （Rust側`ProgramInfoDto`参照）。Tauri外（フォールバックのinvoke）では
 * `{status: "disconnected"}`相当を返す。 */
export async function queryProgramName(channel) {
  const result = await invoke('op505_query_program_name', { channel });
  if (!result || typeof result !== 'object') {
    return { bank: 0, program: 0, name: '', status: 'disconnected' };
  }
  return result;
}

export function setPerformanceLfo(args) {
  return invoke('op505_set_performance_lfo', args);
}

export function tapTempo(bpm) {
  return invoke('tap_tempo', { bpm });
}

export function openEditor() {
  return invoke('op505_open_editor');
}

export function setMetronomeEnabled(enabled) {
  pushLog(`metronome ${enabled ? 'on' : 'off'}`);
  return invoke('set_metronome_enabled', { enabled });
}

/** Rust側`clock_loop`が拍の頭ごとに送る`sequencer-tick`（payload=小節内の拍番号、0-indexed）を
 * 購読する。刻み自体はRust側が持ち、JSは受け取った拍番号で再生カーソルを描くだけ
 * （JSタイマーは数十msの誤差が出るため刻みには使わない）。Tauri外では何もしない。 */
export function onSequencerTick(callback) {
  if (!tauriEvent?.listen) return;
  tauriEvent.listen('sequencer-tick', (event) => callback(event.payload));
}

/** リズム画面のグリッドセルのベロシティ段階を設定する。`note`はGM2ノート番号
 * （行の対応表自体はJS側rhythm-screen.jsが持つ）、`level`は0(消音)〜3(弱)。
 * 実際の発音判定・送信はRust側`clock_loop`が持つ共有パターンへの書き込みのみ行う。 */
export function setRhythmStep(note, step, level) {
  return invoke('set_rhythm_step', { note, step, level });
}

/** Rust側`clock_loop`が16分音符（6クロック）ごとに送る`rhythm-step`（payload=小節内の
 * ステップ番号0〜15）を購読する。リズム画面の再生カーソル描画専用。 */
export function onRhythmStepTick(callback) {
  if (!tauriEvent?.listen) return;
  tauriEvent.listen('rhythm-step', (event) => callback(event.payload));
}

/** リズム/メロディ画面共通の再生/停止ボタン。MIDI Clock自体（メトロノーム・
 * TimeEgテンポ同期）は止めず、両パターンの発音・再生カーソル通知だけを止める。 */
export function setSequencerRunning(running) {
  pushLog(`sequencer ${running ? 'play' : 'stop'}`);
  return invoke('set_sequencer_running', { running });
}

/** メロディ画面で新規ノートを作成する。`id`はJS側（melody-screen.js）が採番した
 * 一意な値。実際の発音判定はRust側`clock_loop`が持つ共有ノートリストへの
 * 書き込みのみ行う。 */
export function addMelodyNote(id, startStep, lengthSteps, pitch, level) {
  return invoke('add_melody_note', { id, startStep, lengthSteps, pitch, level });
}

/** メロディ画面でのノート移動・リサイズ・音量サイクルで呼ぶ（全フィールドを
 * 丸ごと書き換える）。 */
export function updateMelodyNote(id, startStep, lengthSteps, pitch, level) {
  return invoke('update_melody_note', { id, startStep, lengthSteps, pitch, level });
}

/** メロディ画面のDELキーでのノート削除。 */
export function deleteMelodyNote(id) {
  return invoke('delete_melody_note', { id });
}

/** Rust側`clock_loop`が8分音符（12クロック）ごとに送る`melody-step`（payload=
 * ループ全体でのステップ番号0〜63）を購読する。メロディ画面の再生カーソル描画専用。 */
export function onMelodyStepTick(callback) {
  if (!tauriEvent?.listen) return;
  tauriEvent.listen('melody-step', (event) => callback(event.payload));
}
