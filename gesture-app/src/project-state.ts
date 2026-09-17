// プロジェクト全体（コード履歴＋リズムパターン＋メロディノート＋BPM）の収集・復元。
// Undo/Redo（undo-manager.js）とファイル保存/読込（project-file.js）の両方が、
// 同じcapture/apply関数を共有する（「巻き戻す」と「ファイルから読み込む」は
// どちらも「プロジェクト全体を丸ごと置き換える」という点で同じ操作のため）。
//
// 各画面モジュール（chord-screen.js等）が返すgetXxxState()の値は、以後その画面側で
// 変更されない不変のスナップショットであること（chord-screen.jsのhistoryは常に新しい
// オブジェクトを返す設計、rhythm/melodyのgetState側は配列・Mapを複製して返す）。
// これによりUndoスタックへ積んだ古いスナップショットが後から書き換わる事故を防ぐ。

import { getChordState, setChordState } from './chord-screen.ts';
import { getRows, setRows, patternV1ToRows } from './rhythm-screen.ts';
import { getNotes, setNotes } from './melody-screen.ts';
import { getBpm, setBpm } from './tempo-state.svelte.ts';
import { tapTempo } from './midi.ts';
import { expandRhythmSteps16To96, expandMelodyNotesV2, repeatRhythmBarToSequence } from './grid-migrate.ts';
import type { ProjectState, RhythmRow, MelodyNote } from './types.ts';

// v1→v2: リズムの「12行固定パターン」を「ノート番号キーの可変長rows」へ変更した
// （フェーズ3、MIDI Importで未知のノート番号の行が増減できるようにするため）。
// v2→v3: リズム/メロディの内部単位を1パルス=1/96小節へ統一した（グリッド解像度細分化）。
// v3→v4: リズムを1小節=96パルスから、MELODY画面と共通の8小節=768パルスへ拡張した
// （タイムライン共通化）。
const VERSION = 4;

/** プロジェクト全体の現在状態を1個のプレーンオブジェクトへ集約する。 */
export function captureProjectState(): ProjectState {
  return {
    version: VERSION,
    bpm: getBpm(),
    chord: getChordState(), // { entries, cursor, initialKey }
    rhythm: { rows: getRows() },
    melody: { notes: getNotes() },
  };
}

/** captureProjectState()の形式の状態を各画面へ丸ごと反映する。version 1（rhythm.pattern
 * 形式）のファイルもDEFAULT_ROW_NOTES対応で読み込める。version 1/2は旧16/8ステップ単位の
 * ため、grid-migrate.tsで1パルス=1/96小節へ展開してから反映する。version 1〜3のリズムは
 * 1小節ループのため、8小節タイムラインへは同じ内容を繰り返して展開する（聞こえ方を保つ）。
 * メロディはv3時点で既に768パルス（8小節）のため、v3→v4での追加変換は不要。 */
export function applyProjectState(state: ProjectState): void {
  setChordState(state.chord);

  let rows: RhythmRow[];
  let notes: MelodyNote[];
  if (state.version === 4) {
    rows = state.rhythm.rows;
    notes = state.melody.notes;
  } else if (state.version === 3) {
    rows = state.rhythm.rows.map((r) => ({ ...r, steps: repeatRhythmBarToSequence(r.steps) }));
    notes = state.melody.notes;
  } else if (state.version === 2) {
    rows = state.rhythm.rows.map((r) => ({ ...r, steps: repeatRhythmBarToSequence(expandRhythmSteps16To96(r.steps)) }));
    notes = expandMelodyNotesV2(state.melody.notes);
  } else {
    rows = patternV1ToRows(state.rhythm.pattern);
    notes = expandMelodyNotesV2(state.melody.notes);
  }

  setRows(rows);
  setNotes(notes);
  setBpm(state.bpm ?? null);
  if (state.bpm != null) tapTempo(state.bpm);
}

export function serializeProject(): string {
  return JSON.stringify(captureProjectState(), null, 2);
}

/** JSON文字列をパースして反映する。 */
export function deserializeAndApply(json: string): void {
  const state = JSON.parse(json) as ProjectState;
  applyProjectState(state);
}
