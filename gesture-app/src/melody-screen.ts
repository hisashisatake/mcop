// メロディ画面（フェーズ6：ピアノロール本体）。
//
// RHYTHM画面と同じ設計思想: 発音のタイミング自体はRust側`clock_loop`が持ち
// （JSタイマーは数十msの誤差が出るため使わない）、JSはノートをRustへミラーし、
// `melody-step`イベントで受け取った再生位置をカーソルとして描くだけ。
//
// 内部データ（`startStep`/`lengthSteps`）は1パルス=1/96小節（`grid-units.ts`参照）で
// 持つ。マス幅（スナップ単位）は`grid-zoom.svelte.ts`の`gridZoomState`
// （`GridZoomSlider.svelte`のスライダー・−/+ボタン、またはCtrl+ホイールで変更）が持ち、
// `gridSnap()`で毎回読む。倍率はRHYTHM画面と共有する（ユーザー要望）。既定値
// （16分音符=6パルス/マス）は倍率UI導入前のRHYTHM既定と同じ。
//
// RHYTHM画面が「固定マス×12行のグリッド」なのに対し、メロディは音オブジェクト
// （可変の開始位置・長さ・音高を持つノート）のリストで表現する。ループ長はRHYTHM画面と
// 共通の8小節=768パルス（`grid-units.ts`の`SEQUENCE_TOTAL_PULSES`、タイムライン共通化。
// Rust側`SEQUENCE_TOTAL_PULSES`と一致させること）。
// フルピアノ音域（A0=MIDI21〜C8=MIDI108、88鍵）を縦スクロールで、768パルス全体を
// 横スクロールで見せるため、rhythmの「コンテナに合わせて等分割」ではなく「固定セル
// サイズ+スクロールオフセット」で描画する。
//
// ノートのID発行はJS側が担う（Rustが非同期でID発行するとJS側が往復待ちになるため、
// rhythmの`set_rhythm_range`と同じfire-and-forget方式に揃える）。

import { isActive } from './screens.svelte.ts';
import { NOTE_NAMES } from './chords.ts';
import { addMelodyNote, updateMelodyNote, deleteMelodyNote, onMelodyStepTick, setMelodyNotesBulk } from './midi.ts';
import { pushUndo } from './undo-manager.ts';
import { PULSES_PER_BAR, PULSES_PER_BEAT, SEQUENCE_TOTAL_PULSES, snapFloor, snapCeil } from './grid-units.ts';
import { gridSnap, zoomStep } from './grid-zoom.svelte.ts';
import { SB_THICKNESS, computeThumb, isInThumb, scrollFromThumbStart, pageJumpDirection, type ScrollbarGeom } from './scrollbar.ts';
import type { MelodyNote } from './types.ts';

// MIDI Import/Export（project-file.js）が量子化の範囲・単位として参照するため、
// この3定数はexportする。
export const MIN_PITCH = 21; // A0
export const MAX_PITCH = 108; // C8
const ROWS = MAX_PITCH - MIN_PITCH + 1; // 88

export const TOTAL_STEPS = SEQUENCE_TOTAL_PULSES; // 768

const CELL_W = 40;
const CELL_H = 18;
const LABEL_WIDTH = 42;
const TOP_MARGIN = 40; // メニューバー(高さ40px)の直下、他画面と揃える
const BOTTOM_MARGIN = 180; // 右下固定の#status-panelと最下段の行が重ならないための余白

const DRAG_THRESHOLD_PX = 4;
const EDGE_ZONE_PX = Math.min(6, CELL_W / 3);

// [未使用, 通常, アクセント, 弱]。index0は「ノートが存在しない」ため使わない
// （rhythmのLEVEL_COLORSと違い、メロディは存在しないセルを描かないため）。
const LEVEL_COLORS = ['', 'hsl(200,55%,42%)', 'hsl(32,90%,55%)', 'hsl(200,35%,26%)'];

type NoteData = Omit<MelodyNote, 'id'>;

let notes = new Map<number, NoteData>(); // id -> { startStep, lengthSteps, pitch, level }
let selectedId: number | null = null;
let currentStep = -1;
let nextNoteId = 1;

let scrollStep = 0;
let scrollRow = 0;
let scrollInitialized = false;

interface DragState {
  mode: 'resize' | 'pendingMoveOrCycle' | 'pendingMoveOrClick' | 'move' | 'create';
  id: number;
  startClientX?: number;
  startClientY?: number;
  moved?: boolean;
  startStep?: number;
}

let drag: DragState | null = null;

interface ScrollDragState {
  axis: 'v' | 'h';
  geom: ScrollbarGeom;
  startClientPos: number;
  snap?: number; // h軸のみ。スクロール量(セル単位)をパルスへ戻すのに要る
}

let scrollDrag: ScrollDragState | null = null;

onMelodyStepTick((step) => {
  currentStep = step;
});

/** 再生/停止ボタンの停止側からmain.js経由で呼ばれる。カーソルのハイライトを
 * 即座に消す（rhythm-screen.jsの`resetRhythmCursor`と対）。 */
export function resetMelodyCursor(): void {
  currentStep = -1;
}

/** DELキー押下でmain.jsから呼ばれる。選択中ノートが無ければ何もしない。 */
export function deleteSelectedMelodyNote(): void {
  if (selectedId == null) return;
  pushUndo();
  notes.delete(selectedId);
  deleteMelodyNote(selectedId);
  selectedId = null;
}

/**
 * このメロディ画面が持つ状態（ノート一覧）を取得する。project-state.jsがプロジェクト全体の
 * スナップショットを組み立てる際に呼ぶ。`notes`はMapのままだとJSON化できないため配列形式
 * （{id, startStep, lengthSteps, pitch, level}[]）へ変換して返す。
 */
export function getNotes(): MelodyNote[] {
  return [...notes.entries()].map(([id, n]) => ({ id, ...n }));
}

/**
 * 統合Undo/Redo・ファイル読込による復元用。ノート集合を丸ごと置換する。768パルス化で
 * 差分invokeループが編集していないUndo/Redoでも大量に飛びうるため、Rust側の
 * `set_melody_notes`（全置換）を1回だけ呼ぶバルク方式にする。
 */
export function setNotes(notesArray: MelodyNote[]): void {
  notes = new Map<number, NoteData>(
    notesArray.map((n) => [n.id, { startStep: n.startStep, lengthSteps: n.lengthSteps, pitch: n.pitch, level: n.level }]),
  );
  setMelodyNotesBulk(notesArray);
  nextNoteId = notesArray.reduce((max, n) => Math.max(max, n.id), 0) + 1;
  if (selectedId != null && !notes.has(selectedId)) selectedId = null;
}

function pitchName(pitch: number): string {
  const name = NOTE_NAMES[((pitch % 12) + 12) % 12];
  const octave = Math.floor(pitch / 12) - 1;
  return `${name}${octave}`;
}

function isBlackKey(pitch: number): boolean {
  return [1, 3, 6, 8, 10].includes(((pitch % 12) + 12) % 12);
}

function pitchToRowIndex(pitch: number): number {
  return MAX_PITCH - pitch; // 0=最高音(画面上端)、ROWS-1=最低音(画面下端)
}

function rowIndexToPitch(rowIndex: number): number {
  return MAX_PITCH - rowIndex;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

interface GridMetrics {
  visibleCols: number;
  visibleRows: number;
}

function gridMetrics(canvas: HTMLCanvasElement): GridMetrics {
  const visibleCols = Math.max(1, Math.floor((canvas.width - LABEL_WIDTH) / CELL_W));
  const visibleRows = Math.max(1, Math.floor((canvas.height - TOP_MARGIN - BOTTOM_MARGIN) / CELL_H));
  return { visibleCols, visibleRows };
}

function ensureScrollInitialized(canvas: HTMLCanvasElement): void {
  if (scrollInitialized) return;
  const { visibleRows } = gridMetrics(canvas);
  scrollRow = clamp(pitchToRowIndex(60) - Math.floor(visibleRows / 2), 0, Math.max(0, ROWS - visibleRows));
  scrollInitialized = true;
}

/** `step`はパルス単位（`scrollStep`もパルス単位、常に`snap`の倍数）。 */
function stepToX(step: number, snap: number): number {
  return LABEL_WIDTH + ((step - scrollStep) / snap) * CELL_W;
}

function rowIndexToY(rowIndex: number): number {
  return TOP_MARGIN + (rowIndex - scrollRow) * CELL_H;
}

/** 画面座標(px,py) → { pulse, rawPulse, pitch }。`pulse`はマスの先頭パルス（スナップ済み）、
 * `rawPulse`はスナップ前の連続位置（リサイズの伸縮量計算に使う）。グリッド外ならnull。 */
function pointToCell(canvas: HTMLCanvasElement, px: number, py: number): { pulse: number; rawPulse: number; pitch: number } | null {
  const x = px - LABEL_WIDTH;
  const y = py - TOP_MARGIN;
  if (x < 0 || y < 0) return null;
  const snap = gridSnap();
  const rawPulse = scrollStep + (x / CELL_W) * snap;
  const pulse = snapFloor(rawPulse, snap);
  const rowIndex = Math.floor(y / CELL_H) + scrollRow;
  if (pulse < 0 || pulse >= TOTAL_STEPS || rowIndex < 0 || rowIndex >= ROWS) return null;
  return { pulse, rawPulse, pitch: rowIndexToPitch(rowIndex) };
}

/** 縦横スクロールバーのジオメトリ（トラック位置・つまみ位置）をまとめて計算する。
 * 両方とも表示される場合はコーナーで重ならないよう互いのトラック長を1本分縮める。 */
function scrollbarGeometries(canvas: HTMLCanvasElement): { v: ScrollbarGeom; h: ScrollbarGeom } {
  const snap = gridSnap();
  const visualCells = TOTAL_STEPS / snap;
  const { visibleRows, visibleCols } = gridMetrics(canvas);
  const gridRight = LABEL_WIDTH + visibleCols * CELL_W;
  const gridBottom = TOP_MARGIN + visibleRows * CELL_H;
  const needsV = ROWS > visibleRows;
  const needsH = visualCells > visibleCols;

  const vTrackLen = gridBottom - TOP_MARGIN - (needsH ? SB_THICKNESS : 0);
  const v: ScrollbarGeom = {
    trackStart: TOP_MARGIN,
    trackLen: vTrackLen,
    barStart: gridRight - SB_THICKNESS,
    thumb: needsV ? computeThumb(TOP_MARGIN, vTrackLen, ROWS, visibleRows, scrollRow) : null,
    contentUnits: ROWS,
    viewportUnits: visibleRows,
  };

  const hTrackLen = gridRight - LABEL_WIDTH - (needsV ? SB_THICKNESS : 0);
  const h: ScrollbarGeom = {
    trackStart: LABEL_WIDTH,
    trackLen: hTrackLen,
    barStart: gridBottom - SB_THICKNESS,
    thumb: needsH ? computeThumb(LABEL_WIDTH, hTrackLen, visualCells, visibleCols, scrollStep / snap) : null,
    contentUnits: visualCells,
    viewportUnits: visibleCols,
  };

  return { v, h };
}

function findNoteAt(step: number, pitch: number): { id: number; note: NoteData } | null {
  for (const [id, n] of notes) {
    if (n.pitch === pitch && step >= n.startStep && step < n.startStep + n.lengthSteps) {
      return { id, note: n };
    }
  }
  return null;
}

function commitNote(id: number): void {
  const n = notes.get(id);
  if (!n) return;
  updateMelodyNote(id, n.startStep, n.lengthSteps, n.pitch, n.level);
}

export function setupMelodyScreen(canvas: HTMLCanvasElement): { draw: (ctx: CanvasRenderingContext2D) => void } {
  canvas.addEventListener('mousedown', (e) => {
    if (!isActive('melody') || e.button !== 0) return;
    ensureScrollInitialized(canvas);

    const { v, h } = scrollbarGeometries(canvas);
    if (v.thumb && e.clientX >= v.barStart && e.clientX < v.barStart + SB_THICKNESS && e.clientY >= v.trackStart && e.clientY < v.trackStart + v.trackLen) {
      if (isInThumb(e.clientY, v.thumb)) {
        scrollDrag = { axis: 'v', geom: v, startClientPos: e.clientY };
      } else {
        const dir = pageJumpDirection(e.clientY, v.thumb);
        scrollRow = clamp(scrollRow + dir * v.viewportUnits, 0, Math.max(0, v.contentUnits - v.viewportUnits));
      }
      return;
    }
    if (h.thumb && e.clientY >= h.barStart && e.clientY < h.barStart + SB_THICKNESS && e.clientX >= h.trackStart && e.clientX < h.trackStart + h.trackLen) {
      const snap = gridSnap();
      if (isInThumb(e.clientX, h.thumb)) {
        scrollDrag = { axis: 'h', geom: h, startClientPos: e.clientX, snap };
      } else {
        const dir = pageJumpDirection(e.clientX, h.thumb);
        scrollStep = clamp(scrollStep + dir * h.viewportUnits * snap, 0, Math.max(0, (h.contentUnits - h.viewportUnits) * snap));
      }
      return;
    }

    const cell = pointToCell(canvas, e.clientX, e.clientY);
    if (!cell) return;
    const hit = findNoteAt(cell.pulse, cell.pitch);

    if (hit) {
      const rightX = stepToX(hit.note.startStep + hit.note.lengthSteps, gridSnap());
      if (e.clientX >= rightX - EDGE_ZONE_PX && e.clientX < rightX) {
        pushUndo(); // リサイズは掴んだ時点で編集開始とみなす（離すまで実際に長さが変わるかは未確定だが、掴み直しての微調整も含め1操作として扱う）
        drag = { mode: 'resize', id: hit.id, startClientX: e.clientX, startClientY: e.clientY, moved: false };
      } else if (hit.id === selectedId) {
        drag = { mode: 'pendingMoveOrCycle', id: hit.id, startClientX: e.clientX, startClientY: e.clientY, moved: false };
      } else {
        selectedId = hit.id;
        drag = { mode: 'pendingMoveOrClick', id: hit.id, startClientX: e.clientX, startClientY: e.clientY, moved: false };
      }
    } else {
      pushUndo();
      const id = nextNoteId++;
      notes.set(id, { startStep: cell.pulse, lengthSteps: gridSnap(), pitch: cell.pitch, level: 1 });
      selectedId = id;
      drag = { mode: 'create', id, startStep: cell.pulse };
    }
  });

  window.addEventListener('mousemove', (e) => {
    if (!drag) return;
    const n = notes.get(drag.id);
    if (!n) return;

    if (drag.mode === 'pendingMoveOrClick' || drag.mode === 'pendingMoveOrCycle') {
      const dx = e.clientX - drag.startClientX!;
      const dy = e.clientY - drag.startClientY!;
      if (!drag.moved && Math.hypot(dx, dy) >= DRAG_THRESHOLD_PX) {
        pushUndo(); // 実際に動かし始めた瞬間（=編集開始）を捉える。単なる選択クリックは積まない
        drag.moved = true;
        drag.mode = 'move';
      } else if (!drag.moved) {
        return;
      }
    }

    const cell = pointToCell(canvas, e.clientX, e.clientY);
    if (!cell) return;

    if (drag.mode === 'create' || drag.mode === 'resize') {
      // カーソルの連続位置(rawPulse)が属するマスの末尾まで伸ばす。
      const snap = gridSnap();
      n.lengthSteps = Math.max(snap, snapCeil(cell.rawPulse + 1, snap) - n.startStep);
    } else if (drag.mode === 'move') {
      // 開始位置だけスナップし、長さは保持する。
      n.startStep = clamp(cell.pulse, 0, TOTAL_STEPS - n.lengthSteps);
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
        pushUndo();
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
      if (e.ctrlKey) {
        // ズームイン(細かく)=スクロール上、ズームアウト(粗く)=スクロール下（DAW慣習）。
        zoomStep(Math.sign(e.deltaY) < 0 ? 1 : -1);
        const snap = gridSnap();
        // 左端の可視パルスを保持しつつ新しいマス境界へ丸める（視点を飛ばさない）。
        scrollStep = snapFloor(scrollStep, snap);
        const { visibleCols } = gridMetrics(canvas);
        scrollStep = clamp(scrollStep, 0, Math.max(0, TOTAL_STEPS - visibleCols * snap));
        return;
      }
      const { visibleCols, visibleRows } = gridMetrics(canvas);
      const dir = Math.sign(e.shiftKey ? (e.deltaX || e.deltaY) : e.deltaY);
      if (e.shiftKey) {
        const snap = gridSnap();
        scrollStep = clamp(scrollStep + dir * 2 * snap, 0, Math.max(0, TOTAL_STEPS - visibleCols * snap));
      } else {
        scrollRow = clamp(scrollRow + dir * 2, 0, Math.max(0, ROWS - visibleRows));
      }
    },
    { passive: false },
  );

  window.addEventListener('mousemove', (e) => {
    if (!scrollDrag) return;
    const thumb = scrollDrag.geom.thumb!;
    const clientPos = scrollDrag.axis === 'v' ? e.clientY : e.clientX;
    const deltaPx = clientPos - scrollDrag.startClientPos;
    const newThumbStart = thumb.thumbStart + deltaPx;
    const newScrollUnits = scrollFromThumbStart(scrollDrag.geom.trackStart, scrollDrag.geom.trackLen, thumb, scrollDrag.geom.contentUnits, scrollDrag.geom.viewportUnits, newThumbStart);
    if (scrollDrag.axis === 'v') {
      scrollRow = clamp(Math.round(newScrollUnits), 0, Math.max(0, scrollDrag.geom.contentUnits - scrollDrag.geom.viewportUnits));
    } else {
      const snap = scrollDrag.snap!;
      scrollStep = clamp(Math.round(newScrollUnits) * snap, 0, Math.max(0, (scrollDrag.geom.contentUnits - scrollDrag.geom.viewportUnits) * snap));
    }
  });

  window.addEventListener('mouseup', () => {
    scrollDrag = null;
  });

  return { draw: (ctx: CanvasRenderingContext2D) => draw(ctx, canvas) };
}

function draw(ctx: CanvasRenderingContext2D, canvas: HTMLCanvasElement): void {
  if (!isActive('melody')) return;
  ensureScrollInitialized(canvas);
  const snap = gridSnap();

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
    if (n.startStep + n.lengthSteps <= scrollStep || n.startStep >= scrollStep + visibleCols * snap) continue;
    const x0 = Math.max(LABEL_WIDTH, stepToX(n.startStep, snap));
    const x1 = Math.min(gridRight, stepToX(n.startStep + n.lengthSteps, snap));
    const y = rowIndexToY(rowIndex);
    ctx.fillStyle = LEVEL_COLORS[n.level];
    ctx.fillRect(x0 + 1, y + 1, Math.max(1, x1 - x0 - 2), CELL_H - 2);
    if (id === selectedId) {
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 2;
      ctx.strokeRect(x0 + 1, y + 1, Math.max(1, x1 - x0 - 2), CELL_H - 2);
    }
  }

  // 再生カーソル。`currentStep`はRust側`melody-step`の旧スケール(0〜63、8分音符=12パルス
  // 単位)のままイベント頻度を維持している（倍率に関わらず一定、CLAUDE.mdグリッド解像度
  // 細分化参照）ため、パルス単位へ変換してから比較・描画する。
  const currentPulse = currentStep * 12;
  if (currentStep >= 0 && currentPulse >= scrollStep && currentPulse < scrollStep + visibleCols * snap) {
    ctx.fillStyle = 'rgba(255,255,255,0.14)';
    ctx.fillRect(stepToX(currentPulse, snap), TOP_MARGIN, CELL_W, gridBottom - TOP_MARGIN);
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

  // マス線（可視セルの境界、細）
  ctx.strokeStyle = '#2a2a2a';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let c = 0; c <= visibleCols; c++) {
    const step = scrollStep + c * snap;
    if (step > TOTAL_STEPS) break;
    const x = Math.round(stepToX(step, snap)) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  for (let r = 0; r <= visibleRows; r++) {
    const y = Math.round(TOP_MARGIN + r * CELL_H) + 0.5;
    ctx.moveTo(LABEL_WIDTH, y);
    ctx.lineTo(gridRight, y);
  }
  ctx.stroke();

  // 拍線（24パルスごと、中太）。3連符系のスナップだと拍頭がマス境界に乗らないため、
  // マス格子とは独立にパルス単位で描く。
  const visibleEndPulse = scrollStep + visibleCols * snap;
  ctx.strokeStyle = '#4a4a4a';
  ctx.beginPath();
  for (let pulse = 0; pulse <= TOTAL_STEPS; pulse += PULSES_PER_BEAT) {
    if (pulse < scrollStep || pulse > visibleEndPulse) continue;
    const x = Math.round(stepToX(pulse, snap)) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  ctx.stroke();

  // 小節線（96パルスごと、さらに太く）。
  ctx.strokeStyle = '#6a6a6a';
  ctx.beginPath();
  for (let pulse = 0; pulse <= TOTAL_STEPS; pulse += PULSES_PER_BAR) {
    if (pulse < scrollStep || pulse > visibleEndPulse) continue;
    const x = Math.round(stepToX(pulse, snap)) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  ctx.stroke();

  drawScrollbars(ctx, canvas);
}

function drawScrollbars(ctx: CanvasRenderingContext2D, canvas: HTMLCanvasElement): void {
  const { v, h } = scrollbarGeometries(canvas);
  if (v.thumb) {
    ctx.fillStyle = 'rgba(255,255,255,0.06)';
    ctx.fillRect(v.barStart, v.trackStart, SB_THICKNESS, v.trackLen);
    ctx.fillStyle = 'rgba(255,255,255,0.28)';
    ctx.fillRect(v.barStart + 2, v.thumb.thumbStart, SB_THICKNESS - 4, v.thumb.thumbLen);
  }
  if (h.thumb) {
    ctx.fillStyle = 'rgba(255,255,255,0.06)';
    ctx.fillRect(h.trackStart, h.barStart, h.trackLen, SB_THICKNESS);
    ctx.fillStyle = 'rgba(255,255,255,0.28)';
    ctx.fillRect(h.thumb.thumbStart, h.barStart + 2, h.thumb.thumbLen, SB_THICKNESS - 4);
  }
}
