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
import { classifyProgression, classifyInitial, pivotKeysFor, confirmsModulation, normalizeFamily } from './theory.js';

const MIN_SEMITONE = -6;
const MAX_SEMITONE = 5;

const mod12 = (n) => ((n % 12) + 12) % 12;

/**
 * 108セル（12半音×9行）を採点し、重複コード名を除去した上で
 * GREEN/YELLOW/無印（灰）のスコア降順リストへ分ける。
 * @returns {{green: Array, yellow: Array, gray: Array}}
 */
function classifyAllCells({ lastChord, key, tonicMidi, shiftHeld, ctrlHeld, pendingPivot }) {
  const byName = new Map();
  for (let semitone = MIN_SEMITONE; semitone <= MAX_SEMITONE; semitone++) {
    for (let row = 0; row < ROWS; row++) {
      const chord = chordFromSemitone(semitone, row, { tonicMidi, shiftHeld, ctrlHeld });
      const { score, category } = lastChord
        ? classifyProgression(lastChord, chord, key)
        : classifyInitial(chord, key);
      // 直前のコードが無い1手目は「転調予告」という概念自体が成立しない
      // （ダイアトニックコードのほとんどが何らかの近親調のピボットになりうるため、
      // 1手目でも立てるとほぼ全セルが青枠になり情報として機能しない）。
      const isPivot = lastChord ? pivotKeysFor(chord, key).length > 0 : false;
      // pendingPivot（直前までの手でセカンダリードミナント等により絞り込まれた候補調）が
      // あるとき、そのトニックそのものに一致するセルだけを「ここを弾けば転調確定」として
      // 区別する（isPivotの青枠＝まだ確定していない将来の可能性とは別軸）。
      const confirmsPivot = pendingPivot ? pendingPivot.keys.some((k) => confirmsModulation(chord, k)) : false;
      const entry = { chord, score, category, isPivot, confirmsPivot };
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

/** 4レイヤー全ての{shiftHeld, ctrlHeld}組み合わせ。名前はchords.jsのlayersContainingSuffix()と対応。 */
const LAYER_MODS = [
  { name: 'normal', shiftHeld: false, ctrlHeld: false },
  { name: 'shift', shiftHeld: true, ctrlHeld: false },
  { name: 'ctrl', shiftHeld: false, ctrlHeld: true },
  { name: 'ctrlShift', shiftHeld: true, ctrlHeld: true },
];

/**
 * 進行テンプレートの「次の一手」探索専用に、4レイヤー全て（432セル）を横断して採点する。
 * 現在保持している修飾キーに関わらず候補を探すため、classifyAllCellsとは別に持つ
 * （line-cliche等、次の一手が現在のレイヤーに無いケースでもテンプレートを検出できるようにする）。
 * @returns {Array<{chord, score}>} 重複コード名は除去済み（同名は同じfamily/intervalsのためスコアも同一）
 */
function classifyAllLayers({ lastChord, key, tonicMidi }) {
  const byName = new Map();
  for (const { shiftHeld, ctrlHeld } of LAYER_MODS) {
    for (let semitone = MIN_SEMITONE; semitone <= MAX_SEMITONE; semitone++) {
      for (let row = 0; row < ROWS; row++) {
        const chord = chordFromSemitone(semitone, row, { tonicMidi, shiftHeld, ctrlHeld });
        if (byName.has(chord.name)) continue;
        const { score } = lastChord ? classifyProgression(lastChord, chord, key) : classifyInitial(chord, key);
        byName.set(chord.name, { chord, score });
      }
    }
  }
  return [...byName.values()];
}

/**
 * cellsの中から、進行テンプレートの「次の一手」stepに一致するもののうち最高スコアの1件を探す。
 * @param {Array<{chord, score}>} cells
 * @param {{degree: number, families?: string[], suffixes?: string[]}} step
 * @param {{tonicPc: number, mode: string}} key
 */
function findBestCellForStep(cells, step, key) {
  let best = null;
  for (const cell of cells) {
    const degree = mod12(cell.chord.rootPc - key.tonicPc);
    if (degree !== step.degree) continue;
    if (step.suffixes) {
      if (!step.suffixes.includes(cell.chord.suffix)) continue;
    } else if (!step.families.includes(normalizeFamily(cell.chord, key))) {
      continue;
    }
    if (!best || cell.score > best.score) best = cell;
  }
  return best;
}

/**
 * 候補を`cols`列×`rows`行のグリッドへ配置する。緑→黄→灰の順に1本の連続列として繋ぎ、
 * 先頭から`cols*rows`件を取り出してcol優先（列ごとにrows件）で敷き詰める。
 * 各セルの色は本来のcategory（GREEN/YELLOW/null）で決まるため、緑列の在庫が尽きて
 * 黄がはみ出した場合はそのセルは黄色で描かれる。
 *
 * progressionMatches（progressions.matchProgressions()の出力）が渡された場合、各マッチの
 * 「次の一手」を4レイヤー横断で探し、それが現在のレイヤー（shiftHeld/ctrlHeld）にも
 * 存在するときだけグリッドへprogressionHintsを付ける（無ければグリッドには何も足さない —
 * レイヤー切替が要る旨はcomputeProgressionLegend()側で案内する）。スコア順ではグリッド外に
 * 落ちるターゲットも、末尾（＝グリッド内で最もスコアの低いセル）と入れ替えてでも必ず表示する
 * （割り込み枠はcols*rows/2を上限にし、グリッドが提案で埋め尽くされないようにする）。
 *
 * progressionKey（省略時はkeyと同じ）は、progressionMatchesのnext.degreeを絶対ピッチクラスへ
 * 変換する際の基準キー。直前の1手がピボット転調を確定させた場合、表示上の現在キー(key)は
 * 転調後のものになるが、progressionMatches自体は「その手が選ばれた時点のキー」を基準に
 * 度数計算されている（chord-screen.jsのprogressionAnchorKey()参照）ため、ここでも同じ基準で
 * 度数→絶対ピッチクラスへ変換しないとズレる（実例: IV→IIIaugでAマイナーへの転調が確定した
 * 直後、next.degreeはCメジャー基準のままなのにkeyがAマイナーになり、ターゲットセルの
 * 取り違えが起きたバグの修正）。候補セル自体のスコアリング(classifyAllCells/classifyAllLayers)は
 * 表示上のkeyのままでよい（転調後の実際の響きを正しく採点するため）。
 * @returns {Array<{col: number, row: number, chord, score, category, isPivot, confirmsPivot, progressionHints: Array}>}
 */
export function computeCandidateGrid({
  lastChord,
  key,
  progressionKey = key,
  tonicMidi,
  shiftHeld,
  ctrlHeld,
  cols,
  rows,
  progressionMatches,
  pendingPivot,
}) {
  const { green, yellow, gray } = classifyAllCells({ lastChord, key, tonicMidi, shiftHeld, ctrlHeld, pendingPivot });
  const allCells = [...green, ...yellow, ...gray];
  const sequence = allCells.slice(0, cols * rows);

  const hintsByChordName = new Map();
  if (progressionMatches && progressionMatches.length > 0) {
    const allLayerCells = classifyAllLayers({ lastChord, key, tonicMidi });
    const maxInterrupts = Math.min(progressionMatches.length, Math.floor((cols * rows) / 2));
    let interruptsUsed = 0;
    progressionMatches.forEach((match, matchIndex) => {
      const bestOverall = findBestCellForStep(allLayerCells, match.next, progressionKey);
      if (!bestOverall) return;
      const target = allCells.find((c) => c.chord.name === bestOverall.chord.name);
      if (!target) return; // 現在のレイヤーには無い→凡例側でレイヤー切替を案内する（グリッドには足さない）
      if (!sequence.includes(target)) {
        if (interruptsUsed >= maxInterrupts) return;
        sequence[sequence.length - 1 - interruptsUsed] = target;
        interruptsUsed++;
      }
      const hints = hintsByChordName.get(target.chord.name) ?? [];
      hints.push({ id: match.id, name: match.name, badgeIndex: matchIndex + 1, position: match.position, total: match.total });
      hintsByChordName.set(target.chord.name, hints);
    });
  }

  return sequence.map((entry, i) => ({
    ...entry,
    col: Math.floor(i / rows),
    row: i % rows,
    progressionHints: hintsByChordName.get(entry.chord.name) ?? [],
  }));
}

/**
 * 進行テンプレートの凡例（画面上部のヒント文字列）用データを作る。4レイヤー横断で
 * 「次の一手」を探し、現在のレイヤーに無ければlayerName（chords.jsのlayersContainingSuffix()が
 * 返す名前の1つ）を添える。computeCandidateGrid()と別関数にしているのは、こちらは
 * グリッド外（現在のレイヤーに無いケース）も含めて全件を返す必要があるため。
 * @returns {Array<{badgeIndex: number, name: string, position: number, total: number, targetSuffix: string, inCurrentLayer: boolean}>}
 */
export function computeProgressionLegend({ lastChord, key, progressionKey = key, tonicMidi, shiftHeld, ctrlHeld, progressionMatches }) {
  if (!progressionMatches || progressionMatches.length === 0) return [];
  const allLayerCells = classifyAllLayers({ lastChord, key, tonicMidi });
  const currentLayerName = shiftHeld && ctrlHeld ? 'ctrlShift' : ctrlHeld ? 'ctrl' : shiftHeld ? 'shift' : 'normal';
  const results = [];
  progressionMatches.forEach((match, matchIndex) => {
    const target = findBestCellForStep(allLayerCells, match.next, progressionKey);
    if (!target) return;
    results.push({
      badgeIndex: matchIndex + 1,
      name: match.name,
      position: match.position,
      total: match.total,
      targetSuffix: target.chord.suffix,
      currentLayerName,
    });
  });
  return results;
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
 * @param {{chord, key: {tonicMidi, mode}, pendingPivot: object|null, velocity: number}} entry
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
