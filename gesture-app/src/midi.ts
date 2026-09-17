// Tauriコマンドの薄いラッパー。gesture-appは音源を持たず、ここから送った
// MIDIメッセージを名前付きパイプ経由でop505-standaloneが受け取って鳴らす。

import { invoke as tauriInvoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { pushLog } from './midi-log.svelte.ts';
import type { PerformanceLfoArgs, ProgramInfo, MelodyNote } from './types.ts';

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

/** Rust側`clock_loop`が16分音符（6クロック）ごとに送る`rhythm-step`（payload=小節内の
 * ステップ番号0〜15）を購読する。リズム画面の再生カーソル描画専用。 */
export function onRhythmStepTick(callback: (payload: number) => void): void {
  if (!isTauri()) return;
  listen<number>('rhythm-step', (event) => callback(event.payload));
}

/** リズム画面の全行を一括で置き換える。Undo/Redo・ファイル読込で使う
 * （96パルス化で差分invokeループが1回あたり最大1000回超になりうるため、
 * バルク版をRust側`set_rhythm_rows`に用意した）。 */
export function setRhythmRowsBulk(rows: Array<{ note: number; steps: number[] }>): Promise<unknown> {
  return invoke('set_rhythm_rows', { rows });
}

/** 「見たまま＝鳴る」の粗い倍率でのマスクリック用。[start, start+len)のパルス範囲を
 * `level`で一括上書きする（1クリック＝1invoke）。 */
export function setRhythmRange(note: number, start: number, len: number, level: number): Promise<unknown> {
  return invoke('set_rhythm_range', { note, start, len, level });
}

/** リズム/メロディ画面共通の再生/停止ボタン。MIDI Clock自体（メトロノーム・
 * TimeEgテンポ同期）は止めず、両パターンの発音・再生カーソル通知だけを止める。 */
export function setSequencerRunning(running: boolean): Promise<unknown> {
  pushLog(`sequencer ${running ? 'play' : 'stop'}`);
  return invoke('set_sequencer_running', { running });
}

/** タイムライン・ルーラー行のクリック/ドラッグで次回再生開始位置を設定する。停止中のみ
 * 呼ぶこと（再生中のライブseekはしない設計、詳細はmemory project_gesture_app_timeline_ruler_plan参照）。 */
export function setPlaybackStartPulse(pulse: number): Promise<unknown> {
  return invoke('set_playback_start_pulse', { pulse });
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

/** メロディ画面の全ノートを一括で置き換える。Undo/Redo・ファイル読込で使う
 * （バルク版をRust側`set_melody_notes`に用意した、rhythmのbulk置換と同じ理由）。 */
export function setMelodyNotesBulk(notes: MelodyNote[]): Promise<unknown> {
  return invoke('set_melody_notes', { notes });
}

/** Rust側`clock_loop`が8分音符（12クロック）ごとに送る`melody-step`（payload=
 * ループ全体でのステップ番号0〜63）を購読する。メロディ画面の再生カーソル描画専用。 */
export function onMelodyStepTick(callback: (payload: number) => void): void {
  if (!isTauri()) return;
  listen<number>('melody-step', (event) => callback(event.payload));
}
