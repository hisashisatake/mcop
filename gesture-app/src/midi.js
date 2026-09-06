// Tauriコマンドの薄いラッパー。gesture-appは音源を持たず、ここから送った
// MIDIメッセージを名前付きパイプ経由でop505-standaloneが受け取って鳴らす。

import { pushLog } from './midi-log.js';

// フォールバックでブラウザ単体でも開ける（Tauri外ではMIDIは飛ばない）
const invoke = window.__TAURI__?.core?.invoke ?? (async (_cmd, _args) => 0);

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

export function setPerformanceLfo(args) {
  return invoke('op505_set_performance_lfo', args);
}

export function tapTempo(bpm) {
  return invoke('tap_tempo', { bpm });
}

export function openEditor() {
  return invoke('op505_open_editor');
}
