// メロディ画面（フェーズ6：ピアノロール本体）。
//
// RHYTHM画面と同じ設計思想: 発音のタイミング自体はRust側`clock_loop`が持ち
// （JSタイマーは数十msの誤差が出るため使わない）、JSはノートをRustへミラーし、
// `melody-step`イベントで受け取った再生位置をカーソルとして描くだけ。
//
// RHYTHM画面が「固定16ステップ×12行のグリッド」なのに対し、メロディは音オブジェクト
// （可変の開始位置・長さ・音高を持つノート）のリストで表現する。1小節=8ステップ
// （8分音符単位、リズムの16分割より粗い）、ループ長は基本8小節（64ステップ、Rust側
// `MELODY_BARS`×`MELODY_STEPS_PER_BAR`と一致させること）。フルピアノ音域
// （A0=MIDI21〜C8=MIDI108、88鍵）を縦スクロールで、64ステップ全体を横スクロールで
// 見せるため、rhythmの「コンテナに合わせて等分割」ではなく「固定セルサイズ+
// スクロールオフセット」で描画する。
//
// ノートのID発行はJS側が担う（Rustが非同期でID発行するとJS側が往復待ちになるため、
// rhythmの`set_rhythm_step`と同じfire-and-forget方式に揃える）。

import { isActive } from './screens.js';
import { NOTE_NAMES } from './chords.js';
import { addMelodyNote, updateMelodyNote, deleteMelodyNote, onMelodyStepTick } from './midi.js';

const MIN_PITCH = 21; // A0
const MAX_PITCH = 108; // C8
const ROWS = MAX_PITCH - MIN_PITCH + 1; // 88

const STEPS_PER_BAR = 8;
const BARS = 8;
const TOTAL_STEPS = STEPS_PER_BAR * BARS; // 64

const CELL_W = 40;
const CELL_H = 18;
const LABEL_WIDTH = 42;
const TOP_MARGIN = 40; // メニューバー(高さ40px)の直下、他画面と揃える
const BOTTOM_MARGIN = 180; // 右下固定の#status-panelと最下段の行が重ならないための余白

const DRAG_THRESHOLD_PX = 4;
const EDGE_ZONE_PX = 6;

// [未使用, 通常, アクセント, 弱]。index0は「ノートが存在しない」ため使わない
// （rhythmのLEVEL_COLORSと違い、メロディは存在しないセルを描かないため）。
const LEVEL_COLORS = ['', 'hsl(200,55%,42%)', 'hsl(32,90%,55%)', 'hsl(200,35%,26%)'];

let notes = new Map(); // id -> { startStep, lengthSteps, pitch, level }
let selectedId = null;
let currentStep = -1;
let nextNoteId = 1;

let scrollStep = 0;
let scrollRow = 0;
let scrollInitialized = false;

let drag = null;

onMelodyStepTick((step) => {
  currentStep = step;
});

/** 再生/停止ボタンの停止側からmain.js経由で呼ばれる。カーソルのハイライトを
 * 即座に消す（rhythm-screen.jsの`resetRhythmCursor`と対）。 */
export function resetMelodyCursor() {
  currentStep = -1;
}

/** DELキー押下でmain.jsから呼ばれる。選択中ノートが無ければ何もしない。 */
export function deleteSelectedMelodyNote() {
  if (selectedId == null) return;
  notes.delete(selectedId);
  deleteMelodyNote(selectedId);
  selectedId = null;
}

function pitchName(pitch) {
  const name = NOTE_NAMES[((pitch % 12) + 12) % 12];
  const octave = Math.floor(pitch / 12) - 1;
  return `${name}${octave}`;
}

function isBlackKey(pitch) {
  return [1, 3, 6, 8, 10].includes(((pitch % 12) + 12) % 12);
}

function pitchToRowIndex(pitch) {
  return MAX_PITCH - pitch; // 0=最高音(画面上端)、ROWS-1=最低音(画面下端)
}

function rowIndexToPitch(rowIndex) {
  return MAX_PITCH - rowIndex;
}

function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v));
}

function gridMetrics(canvas) {
  const visibleCols = Math.max(1, Math.floor((canvas.width - LABEL_WIDTH) / CELL_W));
  const visibleRows = Math.max(1, Math.floor((canvas.height - TOP_MARGIN - BOTTOM_MARGIN) / CELL_H));
  return { visibleCols, visibleRows };
}

function ensureScrollInitialized(canvas) {
  if (scrollInitialized) return;
  const { visibleRows } = gridMetrics(canvas);
  scrollRow = clamp(pitchToRowIndex(60) - Math.floor(visibleRows / 2), 0, Math.max(0, ROWS - visibleRows));
  scrollInitialized = true;
}

function stepToX(step) {
  return LABEL_WIDTH + (step - scrollStep) * CELL_W;
}

function rowIndexToY(rowIndex) {
  return TOP_MARGIN + (rowIndex - scrollRow) * CELL_H;
}

/** 画面座標(px,py) → { step, pitch }。グリッド外ならnull。 */
function pointToCell(canvas, px, py) {
  const x = px - LABEL_WIDTH;
  const y = py - TOP_MARGIN;
  if (x < 0 || y < 0) return null;
  const step = Math.floor(x / CELL_W) + scrollStep;
  const rowIndex = Math.floor(y / CELL_H) + scrollRow;
  if (step < 0 || step >= TOTAL_STEPS || rowIndex < 0 || rowIndex >= ROWS) return null;
  return { step, pitch: rowIndexToPitch(rowIndex) };
}

function findNoteAt(step, pitch) {
  for (const [id, n] of notes) {
    if (n.pitch === pitch && step >= n.startStep && step < n.startStep + n.lengthSteps) {
      return { id, note: n };
    }
  }
  return null;
}

function commitNote(id) {
  const n = notes.get(id);
  if (!n) return;
  updateMelodyNote(id, n.startStep, n.lengthSteps, n.pitch, n.level);
}

export function setupMelodyScreen(canvas) {
  canvas.addEventListener('mousedown', (e) => {
    if (!isActive('melody') || e.button !== 0) return;
    ensureScrollInitialized(canvas);
    const cell = pointToCell(canvas, e.clientX, e.clientY);
    if (!cell) return;
    const hit = findNoteAt(cell.step, cell.pitch);

    if (hit) {
      const rightX = stepToX(hit.note.startStep + hit.note.lengthSteps);
      if (e.clientX >= rightX - EDGE_ZONE_PX && e.clientX < rightX) {
        drag = { mode: 'resize', id: hit.id, startClientX: e.clientX, startClientY: e.clientY, moved: false };
      } else if (hit.id === selectedId) {
        drag = { mode: 'pendingMoveOrCycle', id: hit.id, startClientX: e.clientX, startClientY: e.clientY, moved: false };
      } else {
        selectedId = hit.id;
        drag = { mode: 'pendingMoveOrClick', id: hit.id, startClientX: e.clientX, startClientY: e.clientY, moved: false };
      }
    } else {
      const id = nextNoteId++;
      notes.set(id, { startStep: cell.step, lengthSteps: 1, pitch: cell.pitch, level: 1 });
      selectedId = id;
      drag = { mode: 'create', id, startStep: cell.step };
    }
  });

  window.addEventListener('mousemove', (e) => {
    if (!drag) return;
    const n = notes.get(drag.id);
    if (!n) return;

    if (drag.mode === 'pendingMoveOrClick' || drag.mode === 'pendingMoveOrCycle') {
      const dx = e.clientX - drag.startClientX;
      const dy = e.clientY - drag.startClientY;
      if (!drag.moved && Math.hypot(dx, dy) >= DRAG_THRESHOLD_PX) {
        drag.moved = true;
        drag.mode = 'move';
      } else if (!drag.moved) {
        return;
      }
    }

    const cell = pointToCell(canvas, e.clientX, e.clientY);
    if (!cell) return;

    if (drag.mode === 'create' || drag.mode === 'resize') {
      n.lengthSteps = Math.max(1, cell.step - n.startStep + 1);
    } else if (drag.mode === 'move') {
      n.startStep = clamp(cell.step, 0, TOTAL_STEPS - n.lengthSteps);
      n.pitch = clamp(cell.pitch, MIN_PITCH, MAX_PITCH);
    }
  });

  window.addEventListener('mouseup', () => {
    if (!drag) return;
    const n = notes.get(drag.id);
    if (n) {
      if (drag.mode === 'create') {
        addMelodyNote(drag.id, n.startStep, n.lengthSteps, n.pitch, n.level);
      } else if (drag.mode === 'resize' || drag.mode === 'move') {
        commitNote(drag.id);
      } else if (drag.mode === 'pendingMoveOrCycle' && !drag.moved) {
        n.level = (n.level % 3) + 1; // 通常→アクセント→弱→通常
        commitNote(drag.id);
      }
      // 'pendingMoveOrClick'かつ未移動 = 選択のみ確定済み、コミット不要
    }
    drag = null;
  });

  canvas.addEventListener(
    'wheel',
    (e) => {
      if (!isActive('melody')) return;
      e.preventDefault();
      ensureScrollInitialized(canvas);
      const { visibleCols, visibleRows } = gridMetrics(canvas);
      const dir = Math.sign(e.shiftKey ? (e.deltaX || e.deltaY) : e.deltaY);
      if (e.shiftKey) {
        scrollStep = clamp(scrollStep + dir * 2, 0, Math.max(0, TOTAL_STEPS - visibleCols));
      } else {
        scrollRow = clamp(scrollRow + dir * 2, 0, Math.max(0, ROWS - visibleRows));
      }
    },
    { passive: false },
  );

  return { draw: (ctx) => draw(ctx, canvas) };
}

function draw(ctx, canvas) {
  if (!isActive('melody')) return;
  ensureScrollInitialized(canvas);

  const W = canvas.width;
  const H = canvas.height;
  const { visibleCols, visibleRows } = gridMetrics(canvas);
  const gridRight = LABEL_WIDTH + visibleCols * CELL_W;
  const gridBottom = TOP_MARGIN + visibleRows * CELL_H;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  // 行背景（黒鍵の行を薄く暗くして音域を把握しやすくする）
  for (let r = 0; r < visibleRows; r++) {
    const rowIndex = scrollRow + r;
    if (rowIndex >= ROWS) break;
    const pitch = rowIndexToPitch(rowIndex);
    ctx.fillStyle = isBlackKey(pitch) ? '#161616' : '#1c1c1c';
    ctx.fillRect(LABEL_WIDTH, TOP_MARGIN + r * CELL_H, gridRight - LABEL_WIDTH, CELL_H);
  }

  // ノート本体（可視範囲のみ）
  for (const [id, n] of notes) {
    const rowIndex = pitchToRowIndex(n.pitch);
    if (rowIndex < scrollRow || rowIndex >= scrollRow + visibleRows) continue;
    if (n.startStep + n.lengthSteps <= scrollStep || n.startStep >= scrollStep + visibleCols) continue;
    const x0 = Math.max(LABEL_WIDTH, stepToX(n.startStep));
    const x1 = Math.min(gridRight, stepToX(n.startStep + n.lengthSteps));
    const y = rowIndexToY(rowIndex);
    ctx.fillStyle = LEVEL_COLORS[n.level];
    ctx.fillRect(x0 + 1, y + 1, Math.max(1, x1 - x0 - 2), CELL_H - 2);
    if (id === selectedId) {
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 2;
      ctx.strokeRect(x0 + 1, y + 1, Math.max(1, x1 - x0 - 2), CELL_H - 2);
    }
  }

  // 再生カーソル
  if (currentStep >= scrollStep && currentStep < scrollStep + visibleCols) {
    ctx.fillStyle = 'rgba(255,255,255,0.14)';
    ctx.fillRect(stepToX(currentStep), TOP_MARGIN, CELL_W, gridBottom - TOP_MARGIN);
  }

  // 行ラベル（音名）
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = '10px monospace';
  for (let r = 0; r < visibleRows; r++) {
    const rowIndex = scrollRow + r;
    if (rowIndex >= ROWS) break;
    const pitch = rowIndexToPitch(rowIndex);
    ctx.fillStyle = isBlackKey(pitch) ? '#555' : '#999';
    ctx.fillText(pitchName(pitch), 4, TOP_MARGIN + r * CELL_H + CELL_H / 2);
  }
  ctx.textBaseline = 'alphabetic';

  // 格子線（1ステップごとに細く、1小節=8ステップごとに太く）
  ctx.strokeStyle = '#2a2a2a';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let c = 0; c <= visibleCols; c++) {
    const step = scrollStep + c;
    if (step > TOTAL_STEPS) break;
    const x = Math.round(stepToX(step)) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  for (let r = 0; r <= visibleRows; r++) {
    const y = Math.round(TOP_MARGIN + r * CELL_H) + 0.5;
    ctx.moveTo(LABEL_WIDTH, y);
    ctx.lineTo(gridRight, y);
  }
  ctx.stroke();

  ctx.strokeStyle = '#4a4a4a';
  ctx.beginPath();
  for (let step = 0; step <= TOTAL_STEPS; step += STEPS_PER_BAR) {
    if (step < scrollStep || step > scrollStep + visibleCols) continue;
    const x = Math.round(stepToX(step)) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  ctx.stroke();
}
