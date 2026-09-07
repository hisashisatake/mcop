// フロー方式コード画面の候補生成・履歴モデル。egui・MIDI・Canvas・DOMのいずれにも
// 触れない純粋関数のみを置く（theory.jsと同じ方針）。
//
// 候補生成:
//   12ピッチクラス（トニックから-6〜+5半音）× 現在レイヤーの9種＝108コードを
//   theory.jsのスコアリングで採点し、GREEN/YELLOW/無印（灰）へ分類してスコア降順に並べる。
//   3列を「緑→黄→灰」の1本の連続した列として繋ぎ、rows個ずつに区切って列へ割り当てる
//   （緑が尽きたら自動的に黄が続き、黄も尽きたら灰が続く）。
//
// 履歴モデル:
//   entries配列＋cursorで線形履歴を表す。selectChordはcursor以降を切り捨てて追加する
//   （分岐は保持せず上書き。ユーザーの明示的な決定）。各entryは選択後のキー状態と
//   pendingPivot（ピボット転調の予告）を丸ごと保持するため、jumpToでコード選択と
//   転調の両方をまとめて移動できる。
//
//   このcursorは「現在どこを見ているか（過去/未来クリックでの再生位置）」だけを表し、
//   「新しいコードを選ぶ」という編集操作そのもののUndo/Redo（Ctrl+Z/Ctrl+Y）は、
//   chord-screen.js側でこのstate自体をスナップショットとして積む別スタックで実現する
//   （このモジュールはselectChord/jumpToという状態遷移の定義のみを持ち、
//   「取り消し操作の履歴」という概念は持たない）。

import { ROWS, chordFromSemitone } from './chords.js';
import { classifyProgression, classifyInitial, pivotKeysFor } from './theory.js';

const MIN_SEMITONE = -6;
const MAX_SEMITONE = 5;

/**
 * 108セル（12半音×9行）を採点し、重複コード名を除去した上で
 * GREEN/YELLOW/無印（灰）のスコア降順リストへ分ける。
 * @returns {{green: Array, yellow: Array, gray: Array}}
 */
function classifyAllCells({ lastChord, key, tonicMidi, shiftHeld, ctrlHeld }) {
  const byName = new Map();
  for (let semitone = MIN_SEMITONE; semitone <= MAX_SEMITONE; semitone++) {
    for (let row = 0; row < ROWS; row++) {
      const chord = chordFromSemitone(semitone, row, { tonicMidi, shiftHeld, ctrlHeld });
      const { score, category } = lastChord
        ? classifyProgression(lastChord, chord, key)
        : classifyInitial(chord, key);
      const isPivot = pivotKeysFor(chord, key).length > 0;
      const entry = { chord, score, category, isPivot };
      const existing = byName.get(chord.name);
      if (!existing || entry.score > existing.score) byName.set(chord.name, entry);
    }
  }

  const all = [...byName.values()];
  const byScoreDesc = (a, b) => b.score - a.score;
  return {
    green: all.filter((c) => c.category === 'GREEN').sort(byScoreDesc),
    yellow: all.filter((c) => c.category === 'YELLOW').sort(byScoreDesc),
    gray: all.filter((c) => c.category == null).sort(byScoreDesc),
  };
}

/**
 * 候補を`cols`列×`rows`行のグリッドへ配置する。緑→黄→灰の順に1本の連続列として繋ぎ、
 * 先頭から`cols*rows`件を取り出してcol優先（列ごとにrows件）で敷き詰める。
 * 各セルの色は本来のcategory（GREEN/YELLOW/null）で決まるため、緑列の在庫が尽きて
 * 黄がはみ出した場合はそのセルは黄色で描かれる。
 * @returns {Array<{col: number, row: number, chord, score, category, isPivot}>}
 */
export function computeCandidateGrid({ lastChord, key, tonicMidi, shiftHeld, ctrlHeld, cols, rows }) {
  const { green, yellow, gray } = classifyAllCells({ lastChord, key, tonicMidi, shiftHeld, ctrlHeld });
  const sequence = [...green, ...yellow, ...gray].slice(0, cols * rows);
  return sequence.map((entry, i) => ({
    ...entry,
    col: Math.floor(i / rows),
    row: i % rows,
  }));
}

// ─────────────────────────────────────────────
// 履歴モデル（線形・上書き方式）
// ─────────────────────────────────────────────

/** @param {{tonicMidi: number, mode: string}} initialKey */
export function createHistory(initialKey) {
  return { entries: [], cursor: -1, initialKey };
}

/** cursor時点のキー状態（未選択なら初期キー）。 */
export function keyAt(state) {
  return state.cursor >= 0 ? state.entries[state.cursor].key : state.initialKey;
}

/** cursor時点のエントリ（未選択ならnull）。 */
export function currentEntry(state) {
  return state.cursor >= 0 ? state.entries[state.cursor] : null;
}

/** cursor時点のpendingPivot（未選択またはピボット無しならnull）。 */
export function pendingPivotAt(state) {
  return state.cursor >= 0 ? state.entries[state.cursor].pendingPivot ?? null : null;
}

/**
 * 新しいコードを選択する。cursorより先の履歴（redo可能だった分）は破棄する。
 * @param {{chord, key: {tonicMidi, mode}, pendingPivot: object|null}} entry
 */
export function selectChord(state, entry) {
  const entries = state.entries.slice(0, state.cursor + 1);
  entries.push(entry);
  return { ...state, entries, cursor: entries.length - 1 };
}

/** 過去/未来コードのクリック用。indexは0-indexed（-1=初期状態）。範囲外なら変化なし。 */
export function jumpTo(state, index) {
  if (index < -1 || index > state.entries.length - 1) return state;
  return { ...state, cursor: index };
}
