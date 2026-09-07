// コード画面（フロー方式）：現在のコードを中心に、左へ過去コードの履歴、右へ「記録済みの続き
// （未来）」と「次の候補」を並べる。
//
// 過去/未来のナビゲーションと、Ctrl+Z/Ctrl+Yの取り消しは別の概念として扱う:
//   - 過去/未来コードのクリックは「再生位置の移動」（cursorを動かすだけ、entriesは不変）
//   - Ctrl+Z/Ctrl+Yは「新しいコードを選択した」という編集操作そのものの一般的なUndo/Redo
//     （過去へ戻ってから新しいコードを選んで先の履歴を上書きした場合、Ctrl+Zはその上書き
//     操作自体を取り消し、上書き前のentries全体（上書きされる前に見えていた続きも含む）へ
//     丸ごと復元する。例: A-B-C-Dと選んでからBまで戻りEを選ぶとA-B-Eになるが、Ctrl+Zを押すと
//     A-B-C-D（カーソルはB）に戻る）。編集操作でないただのカーソル移動（過去/未来クリックや
//     Ctrl+Z/Y自体）はこのUndo/Redoスタックに積まない
//
// 操作:
//   候補セルをクリック … 押している間だけ発音し、離しても選択は確定して現在コードになる
//                        （セル内上下でベロシティ、上側ほど強い）。cursorより先の履歴が
//                        あれば、この時点で破棄される（Ctrl+Zで丸ごと復元可能）
//   過去/未来コードをクリック … 押している間だけ発音し、その地点まで再生位置を移動する
//                        （過去=戻る、未来=記録済みの続きへ進む。どちらも即座に確定）
//   Ctrl+Z             … 直前のコード選択（上書きを含む）を取り消す（発音はしない）
//   Ctrl+Y/Ctrl+Shift+Z … Ctrl+Zで取り消した選択をやり直す（発音はしない）
//   Shift               … 4和音中心のレイヤーへ切替（II-V-Iが縦2行以内に収まる）
//   Ctrl                … sus・付加音系のレイヤーへ切替
//   Ctrl+Shift          … aug・オルタード系のレイヤーへ切替
//   ↓/↑                … 候補の行数（3〜12）を増減
//   ←/→                … 候補の列数（1〜3）を増減
//
// コード補助機能:
//   直前に鳴らしたコードを起点に、次に相性の良いコードを緑（定番）/黄（用例は少ないが理論的
//   裏付けあり）/灰（理論スコア低めの自由枠）の3列で提示する（chord-flow.js/theory.js参照）。
//   ピボットコード（近親調との共通コード）には青を混ぜ、それを押した次に転調先固有のコードを
//   押すと調が確定する。
//
// 履歴は線形（cursor+entries配列）。過去/未来への移動はentriesを一切変更しないが、戻った
// 状態で新しいコードを選ぶと、その先の履歴（未来スロットに見えていた続き）は破棄される
// （分岐は保持しない設計判断。詳細はplan「gesture-app コード画面をグリッド方式からフロー方式へ刷新」）。

import { CHORD_CHANNEL, noteOn, noteOff, allNotesOff } from './midi.js';
import { applyTo as applyLfoTo } from './performance-lfo.js';
import { DEFAULT_TONIC_MIDI, NOTE_NAMES, velocityFromCellY } from './chords.js';
import { pivotKeysFor, confirmsModulation } from './theory.js';
import { computeCandidateGrid, createHistory, keyAt, currentEntry, pendingPivotAt, selectChord, jumpTo } from './chord-flow.js';
import { isActive, onScreenChange } from './screens.js';

const TOP_MARGIN = 40; // 上部の余白（画面タブ・ヒント・ログ等はハンバーガーメニューのドロワーへ移動済みのため最小限でよい）
const BOTTOM_MARGIN = 180; // 左下固定の#hud（コード名の大きな表示）・右下固定の#status-panel（波形メモリ/Bank・Program/Key/TAPテンポ）と過去/現在/未来スロット・候補ブロックが重ならないための余白
const MAX_SCORE_FOR_SHADING = 1.3; // だいたいの上限。alpha計算のクランプ用
const SLIDE_DURATION_MS = 220;
const MAX_HISTORY_SLOTS = 4; // 過去・未来共通の最大表示数（対称レイアウト）

const MIN_ROWS = 3;
const MAX_ROWS = 12;
const MIN_COLS = 1;
const MAX_COLS = 3;
const DEFAULT_ROWS = 5;
const DEFAULT_COLS = 3;

let tonicMidi = DEFAULT_TONIC_MIDI;
let mode = 'major'; // 'major' | 'minor'
let assistRows = DEFAULT_ROWS;
let assistCols = DEFAULT_COLS;

let shiftHeld = false;
let ctrlHeld = false;
let hoverCandidate = null; // {col, row, yRatio}
let hoverSlot = null; // { kind: 'past'|'future', index, yRatio }（過去・未来のホバー共通）
let sounding = []; // 発音中のノート番号
let pointerHeld = false; // マウスボタンを押している最中か（awaitを跨ぐ取りこぼし対策）

// { entries: [{chord, key:{tonicMidi,mode}, pendingPivot}], cursor, initialKey }
let history = createHistory({ tonicMidi, mode });
let candidateCache = null; // { cacheKey, grid: [...] }

// Ctrl+Z/Ctrl+Yの編集Undo/Redo用スタック（Mementoパターン）。要素はcommitSelection直前のhistory
// スナップショットそのもの（historyは常に新しいオブジェクトを返す設計のため、参照を保持するだけで
// 安全に巻き戻せる）。過去/未来クリックによるcursor移動はここへ積まない（編集操作ではないため）。
let undoStack = [];
let redoStack = [];

let tonicSelectEl = null;
let modeSelectEl = null;

// 選択・Undo/Redo時の横スライド演出用（純粋に見た目だけの補間。ロジック上は瞬時に切り替わる）
let slideDirection = 0; // +1 = 前進（右→左へ流れる）, -1 = 後退
let slideStart = 0;

/** 発音中チャンネル（performance-lfoが即時反映に使う）。 */
export function activeChannels() {
  return sounding.length > 0 ? [CHORD_CHANNEL] : [];
}

function currentKeyObj() {
  const k = keyAt(history);
  return { tonicPc: ((k.tonicMidi % 12) + 12) % 12, mode: k.mode };
}

function invalidateCandidates() {
  candidateCache = null;
}

function syncControlsFromState() {
  const k = keyAt(history);
  if (tonicSelectEl) tonicSelectEl.value = String(((k.tonicMidi % 12) + 12) % 12);
  if (modeSelectEl) modeSelectEl.value = k.mode;
}

function resetHistory() {
  history = createHistory({ tonicMidi, mode });
  undoStack = [];
  redoStack = [];
  invalidateCandidates();
  syncControlsFromState();
}

function startSlide(direction) {
  slideDirection = direction;
  slideStart = performance.now();
}

function cellFromPoint(canvas, px, py) {
  const layout = computeLayout(canvas);
  const x = px;
  const y = py - TOP_MARGIN;
  if (y < 0) return null;

  // 候補ブロック（縦方向は中央揃えなので、y=0(TOP_MARGIN)ではなくcandidateOriginYを基準にする）
  if (x >= layout.candidateX) {
    const localX = x - layout.candidateX;
    const localY = py - layout.candidateOriginY;
    const col = Math.floor(localX / layout.cellW);
    const row = Math.floor(localY / layout.cellH);
    if (col < 0 || col >= assistCols || row < 0 || row >= assistRows) return null;
    return { kind: 'candidate', col, row, yRatio: (localY - row * layout.cellH) / layout.cellH };
  }

  // 過去コード列（現在スロットより左）。draw()と同じ並び: i=0が現在の直前(cursor-1)。
  if (x < layout.currentX) {
    const pastCount = Math.min(MAX_HISTORY_SLOTS, history.cursor);
    for (let i = 0; i < pastCount; i++) {
      const slotX = layout.currentX - layout.slotGap * (i + 1);
      if (x >= slotX - layout.slotW / 2 && x < slotX + layout.slotW / 2) {
        return { kind: 'past', index: history.cursor - 1 - i, yRatio: 0.5 };
      }
    }
    return null;
  }

  // 未来コード列（現在スロットより右、候補ブロックより左）。過去と対称の並び:
  // i=0が記録済みの直後(cursor+1)。まだ選択していない先には何も無い（自然に0件になる）。
  const futureCount = Math.min(MAX_HISTORY_SLOTS, history.entries.length - 1 - history.cursor);
  for (let i = 0; i < futureCount; i++) {
    const slotX = layout.currentX + layout.slotGap * (i + 1);
    if (x >= slotX - layout.slotW / 2 && x < slotX + layout.slotW / 2) {
      return { kind: 'future', index: history.cursor + 1 + i, yRatio: 0.5 };
    }
  }
  return null;
}

function computeLayout(canvas) {
  const W = canvas.width;
  const H = canvas.height;
  // 過去/現在/未来スロットと候補ブロックは、ハンバーガーメニュー化で常時表示のUIが
  // canvas上から無くなったため、同じ縦領域（TOP_MARGIN〜H-BOTTOM_MARGIN）を共有する。
  const bodyH = Math.max(1, H - TOP_MARGIN - BOTTOM_MARGIN);
  const currentX = W * 0.3;
  const slotGap = Math.min(110, currentX / (MAX_HISTORY_SLOTS + 1));
  const slotW = Math.min(84, slotGap - 8);
  // 未来スロットの表示幅は過去（0〜currentX）と対称にする。過去にさかのぼる操作と、
  // 記録済みの続きへクリックで進む操作が同じ見た目の「再生位置の移動」になるように
  const candidateX = currentX * 2;
  // 候補セルは正方形（横長だとセル内上下の位置＝ベロシティの変化が実感しにくいため）。
  // 縦方向（行数から決まる高さ）と横方向（列数から決まる幅、はみ出し防止）の両方で頭打ちにする。
  const cellSize = Math.min(84, bodyH / assistRows, (W - candidateX - 16) / assistCols);
  const cellW = cellSize;
  const cellH = cellSize;
  // 候補ブロックは縦方向中央揃えで描く（draw()・cellFromPoint()の両方がここを基準にする）
  const candidateOriginY = TOP_MARGIN + (bodyH - assistRows * cellH) / 2;
  return { W, H, bodyH, currentX, candidateX, cellW, cellH, slotGap, slotW, candidateOriginY };
}

async function stopChord() {
  const notes = sounding;
  sounding = [];
  for (const note of notes) {
    await noteOff(CHORD_CHANNEL, note);
  }
}

/** ピボット経由の転調が確定したかを判定し、新しいkey/pendingPivotを返す（historyへは反映しない）。 */
function evaluateTheoryTransition(chord) {
  const key = currentKeyObj();
  const prevPivot = pendingPivotAt(history);
  let newKey = { tonicMidi, mode };
  if (prevPivot) {
    const confirmed = prevPivot.keys.find((k) => confirmsModulation(chord, k, key));
    if (confirmed) {
      newKey = { tonicMidi: 60 + confirmed.tonicPc, mode: confirmed.mode };
    }
  }
  const newKeyObj = { tonicPc: ((newKey.tonicMidi % 12) + 12) % 12, mode: newKey.mode };
  const pivots = pivotKeysFor(chord, newKeyObj);
  const newPendingPivot = pivots.length > 0 ? { keys: pivots } : null;
  return { key: newKey, pendingPivot: newPendingPivot };
}

function applyKey(newKey) {
  tonicMidi = newKey.tonicMidi;
  mode = newKey.mode;
  syncControlsFromState();
}

/**
 * 候補コードの選択を即座に確定する（発音は呼び出し側が行う）。選択は保持時間に関わらず確定する。
 * 編集操作なのでundoStackへ直前のhistoryを積み、redoStackは破棄する（一般的なUndo/Redoの規約）。
 */
function commitSelection(chord) {
  const { key, pendingPivot } = evaluateTheoryTransition(chord);
  undoStack.push(history);
  redoStack = [];
  history = selectChord(history, { chord, key, pendingPivot });
  applyKey(key);
  invalidateCandidates();
  startSlide(1);
}

/** 過去/未来の地点へ即座に再生位置を移動する（発音は呼び出し側が行う）。編集操作ではないためundo/redoスタックには積まない。 */
function jumpToIndex(index) {
  const direction = index < history.cursor ? -1 : 1;
  history = jumpTo(history, index);
  applyKey(keyAt(history));
  invalidateCandidates();
  startSlide(direction);
}

/** 直前のコード選択（過去へ戻った上での上書きも含む）を取り消し、その操作の直前のhistoryへ丸ごと復元する。 */
function undoEdit() {
  if (undoStack.length === 0) return;
  redoStack.push(history);
  history = undoStack.pop();
  applyKey(keyAt(history));
  invalidateCandidates();
  startSlide(-1);
}

/** undoEditで取り消したコード選択をやり直す。 */
function redoEdit() {
  if (redoStack.length === 0) return;
  undoStack.push(history);
  history = redoStack.pop();
  applyKey(keyAt(history));
  invalidateCandidates();
  startSlide(1);
}

async function playChord(chord, velocity) {
  await stopChord();
  await applyLfoTo(CHORD_CHANNEL);
  for (const note of chord.notes) {
    await noteOn(CHORD_CHANNEL, note, velocity);
  }
  sounding = chord.notes.slice();
}

export function setupChordScreen(canvas, { onChordChange } = {}) {
  canvas.addEventListener('mousemove', (e) => {
    if (!isActive('chord')) return;
    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    hoverCandidate = cell?.kind === 'candidate' ? cell : null;
    hoverSlot = cell?.kind === 'past' || cell?.kind === 'future' ? cell : null;
  });

  canvas.addEventListener('mousedown', async (e) => {
    if (!isActive('chord') || e.button !== 0) return;
    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    if (!cell) return;
    pointerHeld = true;

    if (cell.kind === 'candidate') {
      const grid = computeCandidates();
      const found = grid.find((c) => c.col === cell.col && c.row === cell.row);
      if (!found) {
        pointerHeld = false;
        return;
      }
      // 選択は保持時間に関わらず確定する（離しても現在コードとして残る）
      commitSelection(found.chord);
      await playChord(found.chord, velocityFromCellY(cell.yRatio));
    } else if (cell.kind === 'past' || cell.kind === 'future') {
      // 過去・未来どちらも「その地点へ再生位置を移動する」操作として対称に扱う
      jumpToIndex(cell.index);
      const entry = currentEntry(history);
      if (!entry) {
        pointerHeld = false;
        return;
      }
      await playChord(entry.chord, velocityFromCellY(cell.yRatio));
    }

    if (!pointerHeld) {
      await stopChord();
    }
    onChordChange?.(currentEntry(history)?.chord.name ?? null);
  });

  const release = async () => {
    pointerHeld = false;
    if (sounding.length === 0) return;
    await stopChord();
    onChordChange?.(currentEntry(history)?.chord.name ?? null);
  };
  canvas.addEventListener('mouseup', release);
  canvas.addEventListener('mouseleave', release);
  // 他の画面へ切り替えたときも、鳴りっぱなしを残さず止める（画面を跨いだ音の取りこぼし対策）
  onScreenChange((next) => {
    if (next !== 'chord') release();
  });

  window.addEventListener('keydown', (e) => {
    if (!isActive('chord')) return;
    if (e.key === 'Shift') {
      shiftHeld = true;
      invalidateCandidates();
    } else if (e.key === 'Control') {
      ctrlHeld = true;
      invalidateCandidates();
    } else if (e.key.toLowerCase() === 'z' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      if (e.shiftKey) redoEdit();
      else undoEdit();
      onChordChange?.(currentEntry(history)?.chord.name ?? null);
    } else if (e.key.toLowerCase() === 'y' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      redoEdit();
      onChordChange?.(currentEntry(history)?.chord.name ?? null);
    } else if (e.key === 'ArrowDown') {
      assistRows = Math.max(MIN_ROWS, assistRows - 1);
      invalidateCandidates();
      syncRowsCols();
    } else if (e.key === 'ArrowUp') {
      assistRows = Math.min(MAX_ROWS, assistRows + 1);
      invalidateCandidates();
      syncRowsCols();
    } else if (e.key === 'ArrowLeft') {
      assistCols = Math.max(MIN_COLS, assistCols - 1);
      invalidateCandidates();
      syncRowsCols();
    } else if (e.key === 'ArrowRight') {
      assistCols = Math.min(MAX_COLS, assistCols + 1);
      invalidateCandidates();
      syncRowsCols();
    }
  });
  window.addEventListener('keyup', (e) => {
    if (!isActive('chord')) return;
    if (e.key === 'Shift') {
      shiftHeld = false;
      invalidateCandidates();
    } else if (e.key === 'Control') {
      ctrlHeld = false;
      invalidateCandidates();
    }
  });
  // ウィンドウがフォーカスを失うとkeyupを取りこぼすため、押しっぱなし状態を解除する
  window.addEventListener('blur', () => {
    shiftHeld = false;
    ctrlHeld = false;
    invalidateCandidates();
  });

  return { draw: (ctx) => isActive('chord') && draw(ctx, canvas) };
}

let rowsInputEl = null;
let colsInputEl = null;
function syncRowsCols() {
  if (rowsInputEl) rowsInputEl.value = String(assistRows);
  if (colsInputEl) colsInputEl.value = String(assistCols);
}

/** 調・候補の行数/列数を切り替えるUIを配線する。 */
export function bindChordScreenControls({ tonicSelect, modeSelect, rowsInput, colsInput }) {
  tonicSelectEl = tonicSelect ?? null;
  modeSelectEl = modeSelect ?? null;
  rowsInputEl = rowsInput ?? null;
  colsInputEl = colsInput ?? null;

  if (tonicSelect) {
    NOTE_NAMES.forEach((name, i) => {
      const opt = document.createElement('option');
      opt.value = String(i);
      opt.textContent = name;
      tonicSelect.appendChild(opt);
    });
    tonicSelect.value = String(DEFAULT_TONIC_MIDI % 12);
    tonicSelect.addEventListener('change', async () => {
      // 調が変わると同じ候補が別のコードを指すため、鳴りっぱなしを避けて止める
      await stopChord();
      await allNotesOff(CHORD_CHANNEL);
      const pitchClass = parseInt(tonicSelect.value, 10) || 0;
      tonicMidi = 60 + pitchClass;
      resetHistory();
    });
  }

  if (modeSelect) {
    modeSelect.value = mode;
    modeSelect.addEventListener('change', async () => {
      await stopChord();
      await allNotesOff(CHORD_CHANNEL);
      mode = modeSelect.value === 'minor' ? 'minor' : 'major';
      resetHistory();
    });
  }

  if (rowsInput) {
    rowsInput.value = String(assistRows);
    rowsInput.addEventListener('input', () => {
      const raw = parseInt(rowsInput.value, 10) || DEFAULT_ROWS;
      assistRows = Math.max(MIN_ROWS, Math.min(MAX_ROWS, raw));
      invalidateCandidates();
    });
  }

  if (colsInput) {
    colsInput.value = String(assistCols);
    colsInput.addEventListener('input', () => {
      const raw = parseInt(colsInput.value, 10) || DEFAULT_COLS;
      assistCols = Math.max(MIN_COLS, Math.min(MAX_COLS, raw));
      invalidateCandidates();
    });
  }
}

/** 候補グリッドを、状態が変わったときだけ再計算してキャッシュする。 */
function computeCandidates() {
  const key = currentKeyObj();
  const entry = currentEntry(history);
  const cacheKey = JSON.stringify({
    from: entry ? { rootPc: entry.chord.rootPc, family: entry.chord.family } : null,
    key,
    shiftHeld,
    ctrlHeld,
    rows: assistRows,
    cols: assistCols,
  });
  if (candidateCache && candidateCache.cacheKey === cacheKey) return candidateCache.grid;

  const grid = computeCandidateGrid({
    lastChord: entry?.chord ?? null,
    key,
    tonicMidi,
    shiftHeld,
    ctrlHeld,
    cols: assistCols,
    rows: assistRows,
  });
  candidateCache = { cacheKey, grid };
  return grid;
}

/**
 * 選択/Undo/Redoの直後だけ発生する演出用の状態を返す。
 * offsetPx: 過去コード＋現在スロットの一時的な水平オフセット（0へ収束）。
 *   選択（前進）なら+1スロット分右から、Undo（後退）なら-1スロット分左から現在位置へ戻る。
 * candidateAlpha: 候補ブロックのフェードイン係数（0→1）。
 */
function currentSlideState(layout) {
  if (slideDirection === 0) return { offsetPx: 0, candidateAlpha: 1 };
  const elapsed = performance.now() - slideStart;
  const t = Math.min(1, elapsed / SLIDE_DURATION_MS);
  const eased = 1 - (1 - t) ** 2; // ease-out
  const startOffset = slideDirection > 0 ? layout.slotGap : -layout.slotGap;
  const offsetPx = startOffset * (1 - eased);
  if (t >= 1) slideDirection = 0;
  return { offsetPx, candidateAlpha: eased };
}

/** 過去/未来スロット1個分の描画（対称デザインなので共通化）。 */
function drawHistorySlot(ctx, chordName, slotX, slotY, slotW, alpha, isHover) {
  ctx.fillStyle = isHover ? `rgba(150,190,255,${alpha + 0.15})` : `rgba(200,200,200,${alpha * 0.15})`;
  ctx.fillRect(slotX - slotW / 2, slotY - slotW / 2, slotW, slotW);
  ctx.strokeStyle = `rgba(180,180,180,${alpha})`;
  ctx.strokeRect(slotX - slotW / 2 + 0.5, slotY - slotW / 2 + 0.5, slotW, slotW);
  ctx.fillStyle = `rgba(220,220,220,${alpha})`;
  ctx.font = '16px monospace';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(chordName, slotX, slotY);
}

function draw(ctx, canvas) {
  const layout = computeLayout(canvas);
  const { W, H, bodyH, currentX, candidateX, cellW, cellH, slotGap, slotW, candidateOriginY } = layout;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  const { offsetPx, candidateAlpha } = currentSlideState(layout);
  const entry = currentEntry(history);
  const slotY = TOP_MARGIN + bodyH / 2;

  ctx.save();
  if (offsetPx !== 0) ctx.translate(offsetPx, 0);

  // 過去コード（現在スロットの左）。cursor-1, cursor-2, ... と遡る
  const pastCount = Math.min(MAX_HISTORY_SLOTS, history.cursor);
  for (let i = 0; i < pastCount; i++) {
    const idx = history.cursor - 1 - i;
    const past = history.entries[idx];
    if (!past) continue;
    const slotX = currentX - slotGap * (i + 1);
    const alpha = 0.75 - i * 0.18;
    const isHover = hoverSlot?.kind === 'past' && hoverSlot.index === idx;
    drawHistorySlot(ctx, past.chord.name, slotX, slotY, slotW, alpha, isHover);
  }

  // 未来コード（現在スロットの右）。過去と対称。cursor+1, cursor+2, ... と記録済みの続きを辿る
  const futureCount = Math.min(MAX_HISTORY_SLOTS, history.entries.length - 1 - history.cursor);
  for (let i = 0; i < futureCount; i++) {
    const idx = history.cursor + 1 + i;
    const future = history.entries[idx];
    if (!future) continue;
    const slotX = currentX + slotGap * (i + 1);
    const alpha = 0.75 - i * 0.18;
    const isHover = hoverSlot?.kind === 'future' && hoverSlot.index === idx;
    drawHistorySlot(ctx, future.chord.name, slotX, slotY, slotW, alpha, isHover);
  }

  // 現在コードのスロット
  {
    const size = 96;
    const x = currentX;
    const y = TOP_MARGIN + bodyH / 2;
    ctx.fillStyle = sounding.length > 0 ? 'rgba(120,200,255,0.12)' : 'rgba(255,255,255,0.04)';
    ctx.fillRect(x - size / 2, y - size / 2, size, size);
    ctx.strokeStyle = '#eee';
    ctx.lineWidth = 2;
    ctx.strokeRect(x - size / 2 + 1, y - size / 2 + 1, size - 2, size - 2);
    ctx.fillStyle = '#fff';
    ctx.font = 'bold 22px monospace';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(entry ? entry.chord.name : '—', x, y);
    ctx.lineWidth = 1;
  }

  ctx.restore();

  // 候補ブロック（現在スロットの右）。過去コード＋現在スロットの移動には追従させず、
  // 新しい状態へのフェードインだけを演出する。
  ctx.save();
  ctx.globalAlpha = candidateAlpha;
  const grid = computeCandidates();
  for (const cell of grid) {
    const x = candidateX + cell.col * cellW;
    const y = candidateOriginY + cell.row * cellH;
    const clampedScore = Math.max(0, Math.min(1, cell.score / MAX_SCORE_FOR_SHADING));
    const alpha = 0.15 + clampedScore * 0.45;
    ctx.fillStyle =
      cell.category === 'GREEN'
        ? `hsla(120, 100%, 50%, ${alpha})` // 00FF00（最も明るい緑）と同じhue/sat/lightness
        : cell.category === 'YELLOW'
          ? `hsla(48, 75%, 50%, ${alpha})`
          : `hsla(0, 0%, 50%, ${alpha * 0.6})`;
    ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
    if (cell.isPivot) {
      ctx.fillStyle = `hsla(210, 90%, 60%, ${0.18 + clampedScore * 0.12})`;
      ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
    }
    ctx.strokeStyle = '#3a3a3a';
    ctx.strokeRect(Math.round(x) + 0.5, Math.round(y) + 0.5, cellW, cellH);
  }

  // ホバー中の候補セル。塗りの濃さがそのままベロシティの目安になる（候補セル本体より後に描く）
  if (hoverCandidate) {
    const velocity = velocityFromCellY(hoverCandidate.yRatio);
    const alpha = 0.08 + (velocity / 127) * 0.18;
    const x = candidateX + hoverCandidate.col * cellW;
    const y = candidateOriginY + hoverCandidate.row * cellH;
    ctx.fillStyle = `rgba(150, 190, 255, ${alpha})`;
    ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
  }

  // 候補セル名
  ctx.font = `${Math.max(9, Math.min(14, Math.floor(cellW / 6)))}px monospace`;
  ctx.fillStyle = '#ddd';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (const cell of grid) {
    const x = candidateX + cell.col * cellW + cellW / 2;
    const y = candidateOriginY + cell.row * cellH + cellH / 2;
    ctx.fillText(cell.chord.name, x, y);
  }
  ctx.textBaseline = 'alphabetic';
  ctx.restore(); // globalAlphaを戻す

  drawLayerHint(ctx, W, H);
}

function drawLayerHint(ctx, W, H) {
  const label =
    ctrlHeld && shiftHeld
      ? 'aug/オルタードレイヤー (Ctrl+Shift)'
      : ctrlHeld
        ? 'sus/付加音レイヤー (Ctrl)'
        : shiftHeld
          ? '4和音レイヤー (Shift)'
          : 'トライアドレイヤー';
  const color = ctrlHeld || shiftHeld ? '#4af' : '#555';
  ctx.textAlign = 'right';
  ctx.font = '13px monospace';
  ctx.fillStyle = color;
  const keyLabel = `${NOTE_NAMES[((tonicMidi % 12) + 12) % 12]} ${mode === 'minor' ? 'Minor' : 'Major'}`;
  ctx.fillText(`${label}　調 = ${keyLabel}`, W - 18, H - 22);
}
