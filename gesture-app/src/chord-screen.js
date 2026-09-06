// コード画面：点線で区切った格子の各セルにコードを割り当て、クリックで発音する。
//
// 操作:
//   クリック   … 押している間だけ発音（ドラッグで隣のセルへ滑らせても切り替わらない）
//   セル内上下 … ベロシティ（上側ほど強い）
//   Shift     … 4和音中心のレイヤーへ切替（II-V-Iが縦2行以内に収まる）

import { CHORD_CHANNEL, noteOn, noteOff, allNotesOff } from './midi.js';
import { applyTo as applyLfoTo } from './performance-lfo.js';
import { ROWS, DEFAULT_COLS, DEFAULT_TONIC_MIDI, NOTE_NAMES, chordAt, chordTypeAt, velocityFromCellY } from './chords.js';

const GRID_LINE = '#3a3a3a'; // 点線（黒よりの灰色）
const CENTER_LINE = '#5a5a5a'; // 中心セルの目印

let cols = DEFAULT_COLS;
let tonicMidi = DEFAULT_TONIC_MIDI;
let showChordNames = false;

let shiftHeld = false;
let hoverCell = null; // {col, row, yRatio}
let sounding = []; // 発音中のノート番号
let soundingCell = null; // 発音中のセル（描画用）
let pointerHeld = false; // マウスボタンを押している最中か（awaitを跨ぐ取りこぼし対策）

/** 発音中チャンネル（performance-lfoが即時反映に使う）。 */
export function activeChannels() {
  return sounding.length > 0 ? [CHORD_CHANNEL] : [];
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

async function playCell(cell) {
  const chord = chordAt(cell.col, cell.row, { cols, tonicMidi, shiftHeld });
  const velocity = velocityFromCellY(cell.yRatio);

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
    if (e.key === 'Shift') shiftHeld = true;
  });
  window.addEventListener('keyup', (e) => {
    if (e.key === 'Shift') shiftHeld = false;
  });
  // ウィンドウがフォーカスを失うとkeyupを取りこぼすため、押しっぱなし状態を解除する
  window.addEventListener('blur', () => {
    shiftHeld = false;
  });

  return { draw: (ctx) => draw(ctx, canvas) };
}

/** 調・列数・コードネーム表示を切り替えるUIを配線する。 */
export function bindChordScreenControls({ tonicSelect, colsInput, namesToggle }) {
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
    });
  }

  if (colsInput) {
    colsInput.value = String(cols);
    colsInput.addEventListener('input', () => {
      const raw = parseInt(colsInput.value, 10) || DEFAULT_COLS;
      // 中心セルを一意に決めるため奇数に丸める
      cols = Math.max(3, Math.min(25, raw % 2 === 0 ? raw + 1 : raw));
    });
  }

  if (namesToggle) {
    namesToggle.checked = showChordNames;
    namesToggle.addEventListener('change', () => {
      showChordNames = namesToggle.checked;
    });
  }
}

function draw(ctx, canvas) {
  const W = canvas.width;
  const H = canvas.height;
  const cw = W / cols;
  const ch = H / ROWS;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

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

  // 中心セル（現在の調のトニック × トライアド）の目印。基準が見えないと弾けないため
  // 枠だけ控えめに明るくする
  const centerCol = Math.floor(cols / 2);
  const centerRow = ROWS >> 1;
  ctx.strokeStyle = CENTER_LINE;
  ctx.lineWidth = 1;
  ctx.strokeRect(Math.round(centerCol * cw) + 0.5, Math.round(centerRow * ch) + 0.5, cw, ch);

  if (showChordNames) {
    drawChordNames(ctx, cw, ch);
  }

  drawLayerHint(ctx, W, H);
}

function drawChordNames(ctx, cw, ch) {
  const size = Math.max(9, Math.min(15, Math.floor(cw / 5)));
  ctx.font = `${size}px monospace`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (let row = 0; row < ROWS; row++) {
    for (let col = 0; col < cols; col++) {
      const chord = chordAt(col, row, { cols, tonicMidi, shiftHeld });
      ctx.fillStyle = '#666';
      ctx.fillText(chord.name, col * cw + cw / 2, row * ch + ch / 2);
    }
  }
  ctx.textBaseline = 'alphabetic';
}

function drawLayerHint(ctx, W, H) {
  const center = chordTypeAt(ROWS >> 1, shiftHeld);
  const label = shiftHeld ? '4和音レイヤー (Shift)' : 'トライアドレイヤー';
  ctx.textAlign = 'right';
  ctx.font = '13px monospace';
  ctx.fillStyle = shiftHeld ? '#4af' : '#555';
  ctx.fillText(`${label}  中心 = ${NOTE_NAMES[tonicMidi % 12]}${center.suffix}`, W - 18, H - 22);
}
