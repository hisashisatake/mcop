// コード画面（フロー方式）：現在のコードを中心に、左へ過去コードの履歴、右へ次の候補を並べる。
//
// 操作:
//   候補セルをクリック … 押している間だけ発音し、離しても選択は確定して現在コードになる
//                        （セル内上下でベロシティ、上側ほど強い）
//   過去コードをクリック … 押している間だけ発音し、その地点まで巻き戻す
//   Ctrl+Z             … 1つ戻す（発音はしない）
//   Ctrl+Y/Ctrl+Shift+Z … 1つ進める（発音はしない）
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
// 履歴は線形（cursor+entries配列）。戻ってから新しいコードを選ぶと、その先の履歴は破棄される
// （分岐は保持しない設計判断。詳細はplan「gesture-app コード画面をグリッド方式からフロー方式へ刷新」）。

import { CHORD_CHANNEL, noteOn, noteOff, allNotesOff } from './midi.js';
import { applyTo as applyLfoTo } from './performance-lfo.js';
import { DEFAULT_TONIC_MIDI, NOTE_NAMES, velocityFromCellY } from './chords.js';
import { pivotKeysFor, confirmsModulation } from './theory.js';
import { computeCandidateGrid, createHistory, keyAt, currentEntry, pendingPivotAt, selectChord, undo, redo, jumpTo } from './chord-flow.js';
import { isActive, onScreenChange } from './screens.js';

const TOP_MARGIN = 74; // 左上の画面切り替えタブと重ならないよう本体を下げる（rhythm-screen.jsと同じ手当）
const BOTTOM_MARGIN = 190; // 右下固定の#program-panel（chord-controls表示時の高さ）と候補ブロックが重ならないための余白
const CANDIDATE_TOP_MARGIN = 340; // 右上固定の#midi-log-panel（top:92px,max-height:240px）+#hintと候補ブロックが重ならないための開始位置。過去/現在スロットは画面左寄りで重ならないためTOP_MARGINのまま
const MAX_SCORE_FOR_SHADING = 1.3; // だいたいの上限。alpha計算のクランプ用
const SLIDE_DURATION_MS = 220;
const MAX_PAST_SLOTS = 4;

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
let hoverPast = null; // { index, yRatio }
let sounding = []; // 発音中のノート番号
let pointerHeld = false; // マウスボタンを押している最中か（awaitを跨ぐ取りこぼし対策）

// { entries: [{chord, key:{tonicMidi,mode}, pendingPivot}], cursor, initialKey }
let history = createHistory({ tonicMidi, mode });
let candidateCache = null; // { cacheKey, grid: [...] }

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
    const pastCount = Math.min(MAX_PAST_SLOTS, history.cursor);
    for (let i = 0; i < pastCount; i++) {
      const slotX = layout.currentX - layout.pastGap * (i + 1);
      if (x >= slotX - layout.pastW / 2 && x < slotX + layout.pastW / 2) {
        return { kind: 'past', index: history.cursor - 1 - i, yRatio: 0.5 };
      }
    }
    return null;
  }

  return null;
}

function computeLayout(canvas) {
  const W = canvas.width;
  const H = canvas.height;
  const bodyH = Math.max(1, H - TOP_MARGIN - BOTTOM_MARGIN);
  // 候補ブロックは右上のMIDIログパネルと重なるため、過去/現在スロットとは別の（より下から始まる）
  // 縦領域を使う。過去/現在スロットは画面左寄りでパネルと重ならないためbodyHのままでよい。
  const candidateBodyH = Math.max(1, H - CANDIDATE_TOP_MARGIN - BOTTOM_MARGIN);
  const currentX = W * 0.3;
  const candidateX = W * 0.52;
  const cellW = (W - candidateX - 16) / assistCols;
  const cellH = Math.min(84, candidateBodyH / assistRows);
  const pastGap = Math.min(110, currentX / (MAX_PAST_SLOTS + 1));
  const pastW = Math.min(84, pastGap - 8);
  // 候補ブロックは縦方向中央揃えで描く（draw()・cellFromPoint()の両方がここを基準にする）
  const candidateOriginY = CANDIDATE_TOP_MARGIN + (candidateBodyH - assistRows * cellH) / 2;
  return { W, H, bodyH, currentX, candidateX, cellW, cellH, pastGap, pastW, candidateOriginY };
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

/** 候補コードの選択を即座に確定する（発音は呼び出し側が行う）。選択は保持時間に関わらず確定する。 */
function commitSelection(chord) {
  const { key, pendingPivot } = evaluateTheoryTransition(chord);
  history = selectChord(history, { chord, key, pendingPivot });
  applyKey(key);
  invalidateCandidates();
  startSlide(1);
}

/** 過去コードの地点へ即座に巻き戻す（発音は呼び出し側が行う）。 */
function jumpToIndex(index) {
  const direction = index < history.cursor ? -1 : 1;
  history = jumpTo(history, index);
  applyKey(keyAt(history));
  invalidateCandidates();
  startSlide(direction);
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
    hoverPast = cell?.kind === 'past' ? cell : null;
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
    } else if (cell.kind === 'past') {
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
      const direction = e.shiftKey ? 1 : -1;
      history = e.shiftKey ? redo(history) : undo(history);
      applyKey(keyAt(history));
      invalidateCandidates();
      startSlide(direction);
      onChordChange?.(currentEntry(history)?.chord.name ?? null);
    } else if (e.key.toLowerCase() === 'y' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      history = redo(history);
      applyKey(keyAt(history));
      invalidateCandidates();
      startSlide(1);
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
  const startOffset = slideDirection > 0 ? layout.pastGap : -layout.pastGap;
  const offsetPx = startOffset * (1 - eased);
  if (t >= 1) slideDirection = 0;
  return { offsetPx, candidateAlpha: eased };
}

function draw(ctx, canvas) {
  const layout = computeLayout(canvas);
  const { W, H, bodyH, currentX, candidateX, cellW, cellH, pastGap, pastW, candidateOriginY } = layout;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  const { offsetPx, candidateAlpha } = currentSlideState(layout);
  const entry = currentEntry(history);

  ctx.save();
  if (offsetPx !== 0) ctx.translate(offsetPx, 0);

  // 過去コード（現在スロットの左）。cursor-1, cursor-2, ... と遡る
  const pastCount = Math.min(MAX_PAST_SLOTS, history.cursor);
  for (let i = 0; i < pastCount; i++) {
    const idx = history.cursor - 1 - i;
    const past = history.entries[idx];
    if (!past) continue;
    const slotX = currentX - pastGap * (i + 1);
    const alpha = 0.75 - i * 0.18;
    const isHover = hoverPast?.index === idx;
    ctx.fillStyle = isHover ? `rgba(150,190,255,${alpha + 0.15})` : `rgba(200,200,200,${alpha * 0.15})`;
    ctx.fillRect(slotX - pastW / 2, TOP_MARGIN + bodyH / 2 - pastW / 2, pastW, pastW);
    ctx.strokeStyle = `rgba(180,180,180,${alpha})`;
    ctx.strokeRect(slotX - pastW / 2 + 0.5, TOP_MARGIN + bodyH / 2 - pastW / 2 + 0.5, pastW, pastW);
    ctx.fillStyle = `rgba(220,220,220,${alpha})`;
    ctx.font = '16px monospace';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(past.chord.name, slotX, TOP_MARGIN + bodyH / 2);
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
        ? `hsla(140, 65%, 45%, ${alpha})`
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
