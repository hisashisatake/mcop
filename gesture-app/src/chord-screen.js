// コード画面：点線で区切った格子の各セルにコードを割り当て、クリックで発音する。
//
// 操作:
//   クリック   … 押している間だけ発音（ドラッグで隣のセルへ滑らせても切り替わらない）
//   セル内上下 … ベロシティ（上側ほど強い）
//   Shift     … 4和音中心のレイヤーへ切替（II-V-Iが縦2行以内に収まる）
//   Ctrl      … sus・付加音系のレイヤーへ切替
//   Ctrl+Shift… aug・オルタード系のレイヤーへ切替
//   Esc       … ピボットコード経由の転調を1つ取り消して直前の調へ戻す
//
// コード補助機能:
//   直前に鳴らしたコードを起点に、次に相性の良いセルを緑（定番）/黄（用例は少ないが理論的
//   裏付けあり）で光らせる（theory.js参照）。ピボットコード（近親調との共通コード）には
//   青を混ぜ、それを押した次に転調先固有のコードを押すと調が確定し、グリッド全体が
//   横スライドして新トニックが中心へ来る。

import { CHORD_CHANNEL, noteOn, noteOff, allNotesOff } from './midi.js';
import { applyTo as applyLfoTo } from './performance-lfo.js';
import { ROWS, DEFAULT_COLS, DEFAULT_TONIC_MIDI, NOTE_NAMES, chordAt, chordTypeAt, velocityFromCellY, homeRowIndex } from './chords.js';
import { classifyProgression, pivotKeysFor, confirmsModulation } from './theory.js';

const GRID_LINE = '#3a3a3a'; // 点線（黒よりの灰色）
const CENTER_LINE = '#5a5a5a'; // ホームセルの目印
const SLIDE_DURATION_MS = 250;
const MAX_SCORE_FOR_SHADING = 1.3; // だいたいの上限。alpha計算のクランプ用

let cols = DEFAULT_COLS;
let tonicMidi = DEFAULT_TONIC_MIDI;
let mode = 'major'; // 'major' | 'minor'
let showChordNames = false;
let assistEnabled = true;

let shiftHeld = false;
let ctrlHeld = false;
let hoverCell = null; // {col, row, yRatio}
let sounding = []; // 発音中のノート番号
let soundingCell = null; // 発音中のセル（描画用）
let pointerHeld = false; // マウスボタンを押している最中か（awaitを跨ぐ取りこぼし対策）

let lastChord = null; // 直前に鳴らしたコード（マウスを離しても保持。候補計算の起点）
let pendingPivot = null; // { keys: Key[] } 直前のコードがピボットだった場合の転調候補
let keyHistory = []; // Esc取り消し用スタック。{tonicMidi, mode}

let candidateCache = null; // { cacheKey, byCell: Map<'col,row', {score, category, isPivot}> }

let tonicSelectEl = null;
let modeSelectEl = null;

// 転調時の横スライド演出用（純粋に見た目だけの補間。ロジック上は瞬時に切り替わる）
let slideFromCols = 0;
let slideStart = 0;

/** 発音中チャンネル（performance-lfoが即時反映に使う）。 */
export function activeChannels() {
  return sounding.length > 0 ? [CHORD_CHANNEL] : [];
}

function currentKeyObj() {
  return { tonicPc: ((tonicMidi % 12) + 12) % 12, mode };
}

function invalidateCandidates() {
  candidateCache = null;
}

function resetTheoryState() {
  lastChord = null;
  pendingPivot = null;
  keyHistory = [];
  invalidateCandidates();
}

function shortestSemitoneDelta(toPc, fromPc) {
  let d = ((toPc - fromPc) % 12 + 12) % 12;
  if (d > 6) d -= 12;
  return d;
}

function startSlide(deltaCols) {
  slideFromCols = deltaCols;
  slideStart = performance.now();
}

function syncControlsFromState() {
  if (tonicSelectEl) tonicSelectEl.value = String(((tonicMidi % 12) + 12) % 12);
  if (modeSelectEl) modeSelectEl.value = mode;
}

/** 調を切り替える（ピボット転調・Esc取り消し共通）。演奏の連続性を保つためlastChord/履歴はそのまま。 */
function retune(newTonicMidi, newMode) {
  const fromPc = ((tonicMidi % 12) + 12) % 12;
  const toPc = ((newTonicMidi % 12) + 12) % 12;
  startSlide(shortestSemitoneDelta(toPc, fromPc));
  tonicMidi = newTonicMidi;
  mode = newMode;
  invalidateCandidates();
  syncControlsFromState();
}

function cellFromPoint(canvas, px, py) {
  const cw = canvas.width / cols;
  const ch = canvas.height / ROWS;
  const col = Math.floor(px / cw);
  const row = Math.floor(py / ch);
  if (col < 0 || col >= cols || row < 0 || row >= ROWS) return null;
  return { col, row, yRatio: (py - row * ch) / ch };
}

async function stopChord() {
  const notes = sounding;
  sounding = [];
  soundingCell = null;
  for (const note of notes) {
    await noteOff(CHORD_CHANNEL, note);
  }
}

/** ピボット経由の転調が確定したかを判定し、状態（lastChord/pendingPivot/調）を更新する。 */
function handleTheoryTransition(chord) {
  const key = currentKeyObj();
  let modulated = false;
  if (pendingPivot) {
    const confirmed = pendingPivot.keys.find((k) => confirmsModulation(chord, k, key));
    if (confirmed) {
      keyHistory.push({ tonicMidi, mode });
      retune(60 + confirmed.tonicPc, confirmed.mode);
      pendingPivot = null;
      modulated = true;
    }
  }
  if (!modulated) {
    const pivots = assistEnabled ? pivotKeysFor(chord, key) : [];
    pendingPivot = pivots.length > 0 ? { keys: pivots } : null;
  }
  lastChord = chord;
  invalidateCandidates();
}

async function playCell(cell) {
  const chord = chordAt(cell.col, cell.row, { cols, tonicMidi, shiftHeld, ctrlHeld });
  const velocity = velocityFromCellY(cell.yRatio);

  handleTheoryTransition(chord);

  await stopChord();
  await applyLfoTo(CHORD_CHANNEL);
  for (const note of chord.notes) {
    await noteOn(CHORD_CHANNEL, note, velocity);
  }
  sounding = chord.notes.slice();
  soundingCell = { ...cell, name: chord.name };
  return chord;
}

export function setupChordScreen(canvas, { onChordChange } = {}) {
  canvas.addEventListener('mousemove', (e) => {
    hoverCell = cellFromPoint(canvas, e.clientX, e.clientY);
  });

  canvas.addEventListener('mousedown', async (e) => {
    if (e.button !== 0) return;
    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    if (!cell) return;
    pointerHeld = true;
    const chord = await playCell(cell);
    // 発音のawaitを跨いでボタンが離されていたら、鳴らした音をここで回収する
    // （素早いクリックでmouseupが先に走ると鳴りっぱなしになるため）
    if (!pointerHeld) {
      await stopChord();
      onChordChange?.(null);
      return;
    }
    onChordChange?.(chord.name);
  });

  const release = async () => {
    pointerHeld = false;
    if (sounding.length === 0) return;
    await stopChord();
    onChordChange?.(null);
  };
  canvas.addEventListener('mouseup', release);
  canvas.addEventListener('mouseleave', release);

  window.addEventListener('keydown', (e) => {
    if (e.key === 'Shift') {
      shiftHeld = true;
      invalidateCandidates();
    } else if (e.key === 'Control') {
      ctrlHeld = true;
      invalidateCandidates();
    } else if (e.key === 'Escape') {
      if (keyHistory.length > 0) {
        const prev = keyHistory.pop();
        retune(prev.tonicMidi, prev.mode);
        pendingPivot = null;
      }
    }
  });
  window.addEventListener('keyup', (e) => {
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

  return { draw: (ctx) => draw(ctx, canvas) };
}

/** 調・列数・コードネーム表示・コード補助を切り替えるUIを配線する。 */
export function bindChordScreenControls({ tonicSelect, modeSelect, colsInput, namesToggle, assistToggle }) {
  tonicSelectEl = tonicSelect ?? null;
  modeSelectEl = modeSelect ?? null;

  if (tonicSelect) {
    NOTE_NAMES.forEach((name, i) => {
      const opt = document.createElement('option');
      opt.value = String(i);
      opt.textContent = name;
      tonicSelect.appendChild(opt);
    });
    tonicSelect.value = String(DEFAULT_TONIC_MIDI % 12);
    tonicSelect.addEventListener('change', async () => {
      // 調が変わると同じセルが別のコードを指すため、鳴りっぱなしを避けて止める
      await stopChord();
      await allNotesOff(CHORD_CHANNEL);
      const pitchClass = parseInt(tonicSelect.value, 10) || 0;
      tonicMidi = 60 + pitchClass;
      resetTheoryState();
    });
  }

  if (modeSelect) {
    modeSelect.value = mode;
    modeSelect.addEventListener('change', async () => {
      await stopChord();
      await allNotesOff(CHORD_CHANNEL);
      mode = modeSelect.value === 'minor' ? 'minor' : 'major';
      resetTheoryState();
    });
  }

  if (colsInput) {
    colsInput.value = String(cols);
    colsInput.addEventListener('input', () => {
      const raw = parseInt(colsInput.value, 10) || DEFAULT_COLS;
      // 中心セルを一意に決めるため奇数に丸める
      cols = Math.max(3, Math.min(25, raw % 2 === 0 ? raw + 1 : raw));
      invalidateCandidates();
    });
  }

  if (namesToggle) {
    namesToggle.checked = showChordNames;
    namesToggle.addEventListener('change', () => {
      showChordNames = namesToggle.checked;
    });
  }

  if (assistToggle) {
    assistToggle.checked = assistEnabled;
    assistToggle.addEventListener('change', () => {
      assistEnabled = assistToggle.checked;
      invalidateCandidates();
    });
  }
}

/** 117セル分の候補判定を、状態が変わったときだけ再計算してキャッシュする。 */
function computeCandidates() {
  const key = currentKeyObj();
  const cacheKey = JSON.stringify({
    from: lastChord ? { rootPc: lastChord.rootPc, family: lastChord.family } : null,
    key,
    shiftHeld,
    ctrlHeld,
    cols,
  });
  if (candidateCache && candidateCache.cacheKey === cacheKey) return candidateCache.byCell;

  const byCell = new Map();
  if (assistEnabled && lastChord) {
    for (let row = 0; row < ROWS; row++) {
      for (let col = 0; col < cols; col++) {
        const cellChord = chordAt(col, row, { cols, tonicMidi, shiftHeld, ctrlHeld });
        const { score, category } = classifyProgression(lastChord, cellChord, key);
        if (!category) continue;
        const isPivot = pivotKeysFor(cellChord, key).length > 0;
        byCell.set(`${col},${row}`, { score, category, isPivot });
      }
    }
  }
  candidateCache = { cacheKey, byCell };
  return byCell;
}

function currentSlideOffsetCols() {
  if (slideFromCols === 0) return 0;
  const elapsed = performance.now() - slideStart;
  const t = Math.min(1, elapsed / SLIDE_DURATION_MS);
  const eased = 1 - (1 - t) ** 2;
  const offset = slideFromCols * (1 - eased);
  if (t >= 1) slideFromCols = 0;
  return offset;
}

function draw(ctx, canvas) {
  const W = canvas.width;
  const H = canvas.height;
  const cw = W / cols;
  const ch = H / ROWS;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  ctx.save();
  const slideOffsetPx = currentSlideOffsetCols() * cw;
  if (slideOffsetPx !== 0) ctx.translate(slideOffsetPx, 0);

  drawCandidates(ctx, cw, ch);

  // 発音中のセル
  if (soundingCell) {
    ctx.fillStyle = 'rgba(120, 200, 255, 0.20)';
    ctx.fillRect(soundingCell.col * cw, soundingCell.row * ch, cw, ch);
  }

  // ホバー中のセル。塗りの濃さがそのままベロシティの目安になる
  if (hoverCell) {
    const velocity = velocityFromCellY(hoverCell.yRatio);
    const alpha = 0.05 + (velocity / 127) * 0.13;
    ctx.fillStyle = `rgba(150, 190, 255, ${alpha})`;
    ctx.fillRect(hoverCell.col * cw, hoverCell.row * ch, cw, ch);
  }

  // 格子（点線）
  ctx.save();
  ctx.setLineDash([2, 5]);
  ctx.strokeStyle = GRID_LINE;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let c = 1; c < cols; c++) {
    const x = Math.round(c * cw) + 0.5;
    ctx.moveTo(x, 0);
    ctx.lineTo(x, H);
  }
  for (let r = 1; r < ROWS; r++) {
    const y = Math.round(r * ch) + 0.5;
    ctx.moveTo(0, y);
    ctx.lineTo(W, y);
  }
  ctx.stroke();
  ctx.restore();

  // ホームセル（現在の調の主和音）の目印。モードによって行が変わる
  const centerCol = Math.floor(cols / 2);
  const homeRow = homeRowIndex(mode);
  ctx.strokeStyle = CENTER_LINE;
  ctx.lineWidth = 1;
  ctx.strokeRect(Math.round(centerCol * cw) + 0.5, Math.round(homeRow * ch) + 0.5, cw, ch);

  if (showChordNames) {
    drawChordNames(ctx, cw, ch);
  }

  ctx.restore();

  drawLayerHint(ctx, W, H);
}

function drawCandidates(ctx, cw, ch) {
  const candidates = computeCandidates();
  for (const [cellKey, { score, category, isPivot }] of candidates) {
    const [col, row] = cellKey.split(',').map(Number);
    const clampedScore = Math.max(0, Math.min(1, score / MAX_SCORE_FOR_SHADING));
    const alpha = 0.1 + clampedScore * 0.35;
    ctx.fillStyle = category === 'GREEN' ? `hsla(140, 65%, 45%, ${alpha})` : `hsla(48, 75%, 50%, ${alpha})`;
    ctx.fillRect(col * cw, row * ch, cw, ch);
    if (isPivot) {
      ctx.fillStyle = `hsla(210, 90%, 60%, ${0.18 + clampedScore * 0.12})`;
      ctx.fillRect(col * cw, row * ch, cw, ch);
    }
  }
}

function drawChordNames(ctx, cw, ch) {
  const size = Math.max(9, Math.min(15, Math.floor(cw / 5)));
  ctx.font = `${size}px monospace`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (let row = 0; row < ROWS; row++) {
    for (let col = 0; col < cols; col++) {
      const chord = chordAt(col, row, { cols, tonicMidi, shiftHeld, ctrlHeld });
      ctx.fillStyle = '#666';
      ctx.fillText(chord.name, col * cw + cw / 2, row * ch + ch / 2);
    }
  }
  ctx.textBaseline = 'alphabetic';
}

function drawLayerHint(ctx, W, H) {
  const center = chordTypeAt(ROWS >> 1, { shiftHeld, ctrlHeld });
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
  ctx.fillText(`${label}  中心 = ${NOTE_NAMES[tonicMidi % 12]}${center.suffix}　調 = ${keyLabel}`, W - 18, H - 22);
}
