// プロジェクト全体（コード履歴＋リズムパターン＋メロディノート＋BPM）の収集・復元。
// Undo/Redo（undo-manager.js）とファイル保存/読込（project-file.js）の両方が、
// 同じcapture/apply関数を共有する（「巻き戻す」と「ファイルから読み込む」は
// どちらも「プロジェクト全体を丸ごと置き換える」という点で同じ操作のため）。
//
// 各画面モジュール（chord-screen.js等）が返すgetXxxState()の値は、以後その画面側で
// 変更されない不変のスナップショットであること（chord-screen.jsのhistoryは常に新しい
// オブジェクトを返す設計、rhythm/melodyのgetState側は配列・Mapを複製して返す）。
// これによりUndoスタックへ積んだ古いスナップショットが後から書き換わる事故を防ぐ。

import { getChordState, setChordState } from './chord-screen.js';
import { getRows, setRows, patternV1ToRows } from './rhythm-screen.js';
import { getNotes, setNotes } from './melody-screen.js';
import { getBpm, setBpm } from './tempo-state.js';
import { tapTempo } from './midi.js';

// v1→v2: リズムの「12行固定パターン」を「ノート番号キーの可変長rows」へ変更した
// （フェーズ3、MIDI Importで未知のノート番号の行が増減できるようにするため）。
const VERSION = 2;

/** プロジェクト全体の現在状態を1個のプレーンオブジェクトへ集約する。 */
export function captureProjectState() {
  return {
    version: VERSION,
    bpm: getBpm(),
    chord: getChordState(), // { entries, cursor, initialKey }
    rhythm: { rows: getRows() },
    melody: { notes: getNotes() },
  };
}

/** captureProjectState()の形式の状態を各画面へ丸ごと反映する。version 1（rhythm.pattern
 * 形式）のファイルもDEFAULT_ROW_NOTES対応で読み込める。 */
export function applyProjectState(state) {
  setChordState(state.chord);
  const rows = state.version >= 2 ? state.rhythm.rows : patternV1ToRows(state.rhythm.pattern);
  setRows(rows);
  setNotes(state.melody.notes);
  setBpm(state.bpm ?? null);
  if (state.bpm != null) tapTempo(state.bpm);
}

export function serializeProject() {
  return JSON.stringify(captureProjectState(), null, 2);
}

/** JSON文字列をパースして反映する。 */
export function deserializeAndApply(json) {
  const state = JSON.parse(json);
  applyProjectState(state);
}
