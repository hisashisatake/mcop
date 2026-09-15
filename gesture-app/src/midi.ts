// Tauriコマンドの薄いラッパー。gesture-appは音源を持たず、ここから送った
// MIDIメッセージを名前付きパイプ経由でop505-standaloneが受け取って鳴らす。

import { invoke as tauriInvoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { pushLog } from './midi-log.ts';
import type { PerformanceLfoArgs, ProgramInfo } from './types.ts';

// フォールバックでブラウザ単体でも開ける（Tauri外ではMIDIは飛ばない）。
// 旧実装（window.__TAURI__?.core?.invoke ?? (async () => 0)）と同じく、Tauri外では
// 常に0を返す（呼び出し側の大半は戻り値を無視するが、queryProgramNameは
// 「オブジェクトでない」ことを見て意図的にdisconnected扱いへフォールバックする）。
function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return Promise.resolve(0 as unknown as T);
  return tauriInvoke<T>(cmd, args);
}

/** コード発音チャンネル。src-tauri側の`CHORD_CHANNEL`と一致させること。 */
export const CHORD_CHANNEL = 0;

export function noteOn(channel: number, note: number, velocity: number): Promise<unknown> {
  pushLog(`ch${channel} note_on  note=${note} vel=${velocity}`);
  return invoke('note_on', { channel, note, velocity });
}

export function noteOff(channel: number, note: number): Promise<unknown> {
  pushLog(`ch${channel} note_off note=${note}`);
  return invoke('note_off', { channel, note });
}

export function allNotesOff(channel: number): Promise<unknown> {
  pushLog(`ch${channel} all_notes_off`);
  return invoke('all_notes_off', { channel });
}

export function setProgram(bank: number, program: number): Promise<unknown> {
  pushLog(`program_change bank=${bank} program=${program}`);
  return invoke('op505_set_program', { bank, program });
}

/** standaloneへ問い合わせて、指定チャンネルの現在の音色名を取得する。
 * `status`は"resolved"/"not_found"/"rhythm"/"editing"/"disconnected"のいずれか
 * （Rust側`ProgramInfoDto`参照）。Tauri外（フォールバックのinvoke）では
 * `{status: "disconnected"}`相当を返す。 */
export async function queryProgramName(channel: number): Promise<ProgramInfo> {
  const result = await invoke<ProgramInfo>('op505_query_program_name', { channel });
  if (!result || typeof result !== 'object') {
    return { bank: 0, program: 0, name: '', status: 'disconnected' };
  }
  return result;
}

export function setPerformanceLfo(args: PerformanceLfoArgs): Promise<unknown> {
  return invoke('op505_set_performance_lfo', { ...args });
}

export function tapTempo(bpm: number): Promise<unknown> {
  return invoke('tap_tempo', { bpm });
}

export function openEditor(): Promise<unknown> {
  return invoke('op505_open_editor');
}

export function setMetronomeEnabled(enabled: boolean): Promise<unknown> {
  pushLog(`metronome ${enabled ? 'on' : 'off'}`);
  return invoke('set_metronome_enabled', { enabled });
}

/** Rust側`clock_loop`が拍の頭ごとに送る`sequencer-tick`（payload=小節内の拍番号、0-indexed）を
 * 購読する。刻み自体はRust側が持ち、JSは受け取った拍番号で再生カーソルを描くだけ
 * （JSタイマーは数十msの誤差が出るため刻みには使わない）。Tauri外では何もしない。 */
export function onSequencerTick(callback: (payload: number) => void): void {
  if (!isTauri()) return;
  listen<number>('sequencer-tick', (event) => callback(event.payload));
}

/** リズム画面のグリッドセルのベロシティ段階を設定する。`note`はGM2ノート番号
 * （行の対応表自体はJS側rhythm-screen.jsが持つ）、`level`は0(消音)〜3(弱)。
 * 実際の発音判定・送信はRust側`clock_loop`が持つ共有パターンへの書き込みのみ行う。 */
export function setRhythmStep(note: number, step: number, level: number): Promise<unknown> {
  return invoke('set_rhythm_step', { note, step, level });
}

/** Rust側`clock_loop`が16分音符（6クロック）ごとに送る`rhythm-step`（payload=小節内の
 * ステップ番号0〜15）を購読する。リズム画面の再生カーソル描画専用。 */
export function onRhythmStepTick(callback: (payload: number) => void): void {
  if (!isTauri()) return;
  listen<number>('rhythm-step', (event) => callback(event.payload));
}

/** リズム/メロディ画面共通の再生/停止ボタン。MIDI Clock自体（メトロノーム・
 * TimeEgテンポ同期）は止めず、両パターンの発音・再生カーソル通知だけを止める。 */
export function setSequencerRunning(running: boolean): Promise<unknown> {
  pushLog(`sequencer ${running ? 'play' : 'stop'}`);
  return invoke('set_sequencer_running', { running });
}

/** メロディ画面で新規ノートを作成する。`id`はJS側（melody-screen.js）が採番した
 * 一意な値。実際の発音判定はRust側`clock_loop`が持つ共有ノートリストへの
 * 書き込みのみ行う。 */
export function addMelodyNote(id: number, startStep: number, lengthSteps: number, pitch: number, level: number): Promise<unknown> {
  return invoke('add_melody_note', { id, startStep, lengthSteps, pitch, level });
}

/** メロディ画面でのノート移動・リサイズ・音量サイクルで呼ぶ（全フィールドを
 * 丸ごと書き換える）。 */
export function updateMelodyNote(id: number, startStep: number, lengthSteps: number, pitch: number, level: number): Promise<unknown> {
  return invoke('update_melody_note', { id, startStep, lengthSteps, pitch, level });
}

/** メロディ画面のDELキーでのノート削除。 */
export function deleteMelodyNote(id: number): Promise<unknown> {
  return invoke('delete_melody_note', { id });
}

/** Rust側`clock_loop`が8分音符（12クロック）ごとに送る`melody-step`（payload=
 * ループ全体でのステップ番号0〜63）を購読する。メロディ画面の再生カーソル描画専用。 */
export function onMelodyStepTick(callback: (payload: number) => void): void {
  if (!isTauri()) return;
  listen<number>('melody-step', (event) => callback(event.payload));
}
