// リズム画面（フェーズ4：ステップシーケンサー本体、フェーズ3で行の動的化に対応）。
//
// 内部データは1パルス=1/96小節（`grid-units.ts`参照）で持つ。マス幅（スナップ単位）は
// `grid-zoom.svelte.ts`の`gridZoomState`（`GridZoomSlider.svelte`のスライダー・
// −/+ボタン、またはCtrl+ホイールで変更）が持ち、`gridSnap()`で毎回読む。倍率は
// MELODY画面と共有する（ユーザー要望、両画面で同じスナップ単位になる）。既定値
// （16分音符=6パルス/マス、16マス/行）は倍率UI導入前と同じ見た目を保つ。
//
// セル幅はmelody-screen.tsと同じ「固定セル＋スクロール」方式: 1小節がウィンドウ幅に
// 収まるならウィンドウ幅へ引き伸ばし（既定の16分音符では従来とほぼ同じ見た目）、
// 収まらなくなったら最小幅(MIN_CELL_W)に張り付いて横スクロールする（縦のMIN_ROW_H/
// scrollRowと同じ考え方）。
//
// クリック処理は`cellLevel()`でマスが覆う範囲内の最大レベルを読む（そのマスが今どう
// 聞こえているかの代表値）→ (level+1)%4 → 範囲全体を新レベルで上書き（`setRhythmRange`
// を1回）。粗い倍率のマスをクリックすると、そのマスが覆う範囲内の細かい打ち込みも
// まとめて新しい値へ統一される（「見たまま＝鳴る」、CLAUDE.mdグリッド解像度細分化参照）。
//
// 格子線は3階層（マス線=snap刻み・拍線=24パルスごと・小節線=96パルスごと）。3連符系の
// スナップでは拍頭がマス境界に乗らない（例: 1/4T=16パルスは24を割らない）ため、拍線・
// 小節線はマス格子とは独立にパルス単位で描く。
//
// 発音のタイミング自体はRust側`clock_loop`が持つ（JSタイマーは数十msの誤差が
// 出るため使わない）。JSはパターンをRustへミラーし、`rhythm-step`イベントで
// 受け取った再生位置をカーソルとして描くだけ。パターンの発音は再生/停止ボタンの
// ON/OFF（Rust側`SEQUENCER_RUNNING`、メロディ画面と共通）でゲートされ、テンポ
// （タップテンポ）が設定されクロックが走っている間は、画面を切り替えても再生中の
// パターンは裏で鳴り続ける（「ループ再生の上に各パートを重ねる」というミニDAWの
// 設計意図どおり）。
//
// メトロノームのON/OFF・MIDI Clock自体（TimeEgテンポ同期が使う）は再生/停止ボタンとは
// 独立で、停止中も動き続ける。
//
// フェーズ3（MIDI Import）で「行=固定12個」の制約を撤廃した。行はGM2ノート番号を
// キーとする可変長リスト（`rows: [{note, label, steps[96]}]`）で持ち、Rust側
// （midi_out.rs）もノート番号で直接引く128行分のパターンへ変更済み。既定は従来と同じ
// 12行（DEFAULT_ROW_NOTES/DEFAULT_ROW_LABELS）。行数が増えて1行18px未満になる場合は
// 18px固定にしてホイールで縦スクロールする（メロディ画面と同じ方式）。

import { isActive } from './screens.svelte.ts';
import { onRhythmStepTick, setRhythmRange, setRhythmRowsBulk } from './midi.ts';
import { pushUndo } from './undo-manager.ts';
import { PULSES_PER_BAR, PULSES_PER_BEAT, cellStartPulse, cellLevel, snapFloor } from './grid-units.ts';
import { expandPatternV1 } from './grid-migrate.ts';
import { gridSnap, zoomStep } from './grid-zoom.svelte.ts';
import { SB_THICKNESS, computeThumb, isInThumb, scrollFromThumbStart, pageJumpDirection, type ScrollbarGeom } from './scrollbar.ts';
import type { RhythmRow } from './types.ts';

// MIDI Import/Export（project-file.js）が1小節のパルス数として参照するためexportする。
export const STEPS = PULSES_PER_BAR;
const LABEL_WIDTH = 74;
const TOP_MARGIN = 40; // 上部の余白（画面タブ等はハンバーガーメニューのドロワーへ移動済みのため最小限でよい）
const BOTTOM_MARGIN = 180; // 右下固定の#status-panel（波形メモリ/Bank・Program/Key/TAPテンポ）と最下段の行が重ならないための余白
const MIN_ROW_H = 18; // これを下回る行高になる場合は固定してスクロールに切り替える
const MIN_CELL_W = 24; // これを下回るセル幅になる場合は固定して横スクロールに切り替える

// 既定12行のノート番号と短縮ラベル（見た目は従来のまま）。Rust側は行の概念を持たず
// ノート番号で直接引くため、この対応表はJS側だけが持つ。project-state.jsのv1(.gap505)
// 互換読込（`patternV1ToRows`）でも使う。
export const DEFAULT_ROW_NOTES = [49, 51, 46, 42, 39, 37, 38, 40, 48, 45, 41, 36];
export const DEFAULT_ROW_LABELS = ['Crash', 'Ride', 'OpenHH', 'ClosedHH', 'Clap', 'Rim', 'Snare', 'E.Snare', 'HiTom', 'MidTom', 'LoTom', 'Kick'];

// [消音, 通常, アクセント, 弱]。Rust側の0〜3と対応。
const LEVEL_COLORS = ['#1c1c1c', 'hsl(200,55%,42%)', 'hsl(32,90%,55%)', 'hsl(200,35%,26%)'];

let rows: RhythmRow[] = DEFAULT_ROW_NOTES.map((note, i) => ({ note, label: DEFAULT_ROW_LABELS[i], steps: new Array(STEPS).fill(0) }));
let scrollRow = 0;
let scrollStep = 0; // パルス単位、常に現在のスナップ(gridSnap())の倍数
let currentStep = -1;

interface ScrollDragState {
  axis: 'v' | 'h';
  geom: ScrollbarGeom;
  startClientPos: number;
  snap?: number; // h軸のみ。スクロール量(セル単位)をパルスへ戻すのに要る
}

let scrollDrag: ScrollDragState | null = null;

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/** 縦横スクロールバーのジオメトリ（トラック位置・つまみ位置）をまとめて計算する。
 * 両方とも表示される場合はコーナーで重ならないよう互いのトラック長を1本分縮める。 */
function scrollbarGeometries(canvas: HTMLCanvasElement): { v: ScrollbarGeom; h: ScrollbarGeom } {
  const snap = gridSnap();
  const visualCells = STEPS / snap;
  const { cw, ch, visibleRows, visibleCols } = gridMetrics(canvas);
  const gridRight = LABEL_WIDTH + visibleCols * cw;
  const gridBottom = TOP_MARGIN + visibleRows * ch;
  const needsV = rows.length > visibleRows;
  const needsH = visualCells > visibleCols;

  const vTrackLen = gridBottom - TOP_MARGIN - (needsH ? SB_THICKNESS : 0);
  const v: ScrollbarGeom = {
    trackStart: TOP_MARGIN,
    trackLen: vTrackLen,
    barStart: gridRight - SB_THICKNESS,
    thumb: needsV ? computeThumb(TOP_MARGIN, vTrackLen, rows.length, visibleRows, scrollRow) : null,
    contentUnits: rows.length,
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

onRhythmStepTick((step) => {
  currentStep = step;
});

export function setupRhythmScreen(canvas: HTMLCanvasElement): { draw: (ctx: CanvasRenderingContext2D) => void } {
  canvas.addEventListener('mousedown', (e) => {
    if (!isActive('rhythm') || e.button !== 0) return;

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

    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    if (!cell) return;
    const snap = gridSnap();
    const row = rows[cell.rowIndex];
    const current = cellLevel(row.steps, cell.step, snap);
    const next = (current + 1) % 4;
    pushUndo();
    // マスが覆うsnap個のパルス全体を新レベルで上書きする（「見たまま＝鳴る」、
    // 範囲内にあった細かい打ち込みも消音を含めまとめて統一される）。
    for (let i = 0; i < snap; i++) row.steps[cell.step + i] = next;
    setRhythmRange(row.note, cell.step, snap, next);
  });

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

  canvas.addEventListener(
    'wheel',
    (e) => {
      if (!isActive('rhythm')) return;
      if (e.ctrlKey) {
        e.preventDefault();
        // ズームイン(細かく)=スクロール上、ズームアウト(粗く)=スクロール下（DAW慣習）。
        zoomStep(Math.sign(e.deltaY) < 0 ? 1 : -1);
        const snap = gridSnap();
        const visualCells = STEPS / snap;
        // 左端の可視パルスを保持しつつ新しいマス境界へ丸める（視点を飛ばさない）。
        scrollStep = snapFloor(scrollStep, snap);
        const { visibleCols } = gridMetrics(canvas);
        scrollStep = clamp(scrollStep, 0, Math.max(0, (visualCells - visibleCols) * snap));
        return;
      }
      const snap = gridSnap();
      const visualCells = STEPS / snap;
      const { visibleRows, visibleCols } = gridMetrics(canvas);
      const needsVScroll = visibleRows < rows.length;
      const needsHScroll = visibleCols < visualCells;
      if (!needsVScroll && !needsHScroll) return;
      const horizontal = e.shiftKey && needsHScroll;
      if (!horizontal && !needsVScroll) return;
      e.preventDefault();
      const dir = Math.sign(horizontal ? (e.deltaX || e.deltaY) : e.deltaY);
      if (horizontal) {
        scrollStep = clamp(scrollStep + dir * snap, 0, Math.max(0, (visualCells - visibleCols) * snap));
      } else {
        scrollRow = clamp(scrollRow + dir, 0, Math.max(0, rows.length - visibleRows));
      }
    },
    { passive: false },
  );

  return { draw: (ctx: CanvasRenderingContext2D) => draw(ctx, canvas) };
}

/**
 * このリズム画面が持つ状態（行構成込みのパターン）を取得する。project-state.jsが
 * プロジェクト全体のスナップショットを組み立てる際に呼ぶ。`rows`は以後この画面側で
 * セルごとに書き換えられていく実体なので、スナップショットには複製を返す
 * （統合Undo/Redoのスタックへ積んだ後で元の配列が書き換わり、過去のスナップショットまで
 * 壊れてしまう事故を防ぐため）。
 */
export function getRows(): RhythmRow[] {
  return rows.map((r) => ({ note: r.note, label: r.label, steps: r.steps.slice() }));
}

/**
 * 統合Undo/Redo・ファイル読込による復元用。行の集合自体が変わりうる
 * （Importで未知のノート番号の行が増減する）ため丸ごと置換する。96パルス化で
 * 差分invokeループだと1回あたり最大1000回超のinvokeが飛びうるため、Rust側の
 * `set_rhythm_rows`（全置換）を1回だけ呼ぶバルク方式にする。
 */
export function setRows(newRows: RhythmRow[]): void {
  rows = newRows.map((r) => ({ note: r.note, label: r.label, steps: r.steps.slice() }));
  setRhythmRowsBulk(rows.map((r) => ({ note: r.note, steps: r.steps })));
  scrollRow = clamp(scrollRow, 0, Math.max(0, rows.length - 1));
}

/** v1(.gap505)形式の12×16固定パターンを、現行のrows形式（96パルス/行）へ変換する
 * （project-state.js参照）。実体は依存ゼロの`grid-migrate.ts`へ委譲する。 */
export function patternV1ToRows(pattern: number[][]): RhythmRow[] {
  return expandPatternV1(pattern);
}

/** 再生/停止ボタンの停止側からmain.ts経由で呼ばれる。再生カーソルのハイライトを
 * 即座に消す（Rust側は`rhythm-step`イベントの送出自体を止めるだけで、直前のカーソル
 * 位置を明示的に片付けてはくれないため）。 */
export function resetRhythmCursor(): void {
  currentStep = -1;
}

interface GridMetrics {
  cw: number;
  ch: number;
  bodyH: number;
  visibleRows: number;
  visibleCols: number;
}

function gridMetrics(canvas: HTMLCanvasElement): GridMetrics {
  const visualCells = STEPS / gridSnap();
  const availW = canvas.width - LABEL_WIDTH;
  const naturalCw = availW / visualCells;
  const cw = Math.max(MIN_CELL_W, naturalCw);
  const visibleCols = cw <= naturalCw ? visualCells : Math.max(1, Math.floor(availW / cw));
  const bodyH = canvas.height - TOP_MARGIN - BOTTOM_MARGIN;
  const naturalCh = rows.length > 0 ? bodyH / rows.length : bodyH;
  const ch = Math.max(MIN_ROW_H, naturalCh);
  const visibleRows = ch <= naturalCh ? rows.length : Math.max(1, Math.floor(bodyH / ch));
  return { cw, ch, bodyH, visibleRows, visibleCols };
}

/** `pulse`をキャンバスX座標へ変換する（`scrollStep`はパルス単位、常に`snap`の倍数）。 */
function pulseToX(pulse: number, scrollStep: number, cw: number, snap: number): number {
  return LABEL_WIDTH + ((pulse - scrollStep) / snap) * cw;
}

/** 戻り値の`step`はマスの先頭パルス（現在のスナップ刻み）。 */
function cellFromPoint(canvas: HTMLCanvasElement, px: number, py: number): { rowIndex: number; step: number } | null {
  const { cw, ch, visibleRows, visibleCols } = gridMetrics(canvas);
  const x = px - LABEL_WIDTH;
  const y = py - TOP_MARGIN;
  if (x < 0 || y < 0) return null;
  const col = Math.floor(x / cw);
  const r = Math.floor(y / ch);
  if (col < 0 || col >= visibleCols || r < 0 || r >= visibleRows) return null;
  const rowIndex = scrollRow + r;
  if (rowIndex >= rows.length) return null;
  const snap = gridSnap();
  const cellIndex = col + scrollStep / snap;
  return { rowIndex, step: cellStartPulse(cellIndex, snap) };
}

function draw(ctx: CanvasRenderingContext2D, canvas: HTMLCanvasElement): void {
  if (!isActive('rhythm')) return;
  const snap = gridSnap();
  const W = canvas.width;
  const H = canvas.height;
  const { cw, ch, visibleRows, visibleCols } = gridMetrics(canvas);
  const gridRight = LABEL_WIDTH + visibleCols * cw;
  const gridBottom = TOP_MARGIN + visibleRows * ch;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  // セル本体（可視範囲のみ）。マスが覆うsnap個のパルスのうち最大レベルを代表値として描く。
  for (let r = 0; r < visibleRows; r++) {
    const rowIndex = scrollRow + r;
    if (rowIndex >= rows.length) break;
    const row = rows[rowIndex];
    for (let c = 0; c < visibleCols; c++) {
      const pulse = scrollStep + c * snap;
      const level = cellLevel(row.steps, pulse, snap);
      ctx.fillStyle = LEVEL_COLORS[level];
      ctx.fillRect(LABEL_WIDTH + c * cw + 1, TOP_MARGIN + r * ch + 1, cw - 2, ch - 2);
    }
  }

  // 再生カーソル（現在鳴っているステップの列を薄くハイライト）。セル本体より後に
  // 描かないと、不透明なセルの塗りで上書きされて見えなくなる。`currentStep`は
  // Rust側`rhythm-step`から届く旧スケール(0〜15、16分音符単位)のままなのでパルスへ
  // 変換する（イベント頻度自体は倍率に関わらず一定、CLAUDE.mdグリッド解像度細分化参照）。
  const currentPulse = currentStep * 6;
  if (currentStep >= 0 && currentPulse >= scrollStep && currentPulse < scrollStep + visibleCols * snap) {
    ctx.fillStyle = 'rgba(255,255,255,0.14)';
    ctx.fillRect(pulseToX(currentPulse, scrollStep, cw, snap), TOP_MARGIN, cw, gridBottom - TOP_MARGIN);
  }

  // 行ラベル
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = '12px monospace';
  ctx.fillStyle = '#888';
  for (let r = 0; r < visibleRows; r++) {
    const rowIndex = scrollRow + r;
    if (rowIndex >= rows.length) break;
    ctx.fillText(rows[rowIndex].label, 8, TOP_MARGIN + r * ch + ch / 2);
  }
  ctx.textBaseline = 'alphabetic';

  // マス線（可視セルの境界、細）
  ctx.strokeStyle = '#2a2a2a';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let c = 0; c <= visibleCols; c++) {
    const x = Math.round(LABEL_WIDTH + c * cw) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  for (let r = 0; r <= visibleRows; r++) {
    const y = Math.round(TOP_MARGIN + r * ch) + 0.5;
    ctx.moveTo(LABEL_WIDTH, y);
    ctx.lineTo(gridRight, y);
  }
  ctx.stroke();

  // 拍線（24パルスごと、中太）。3連符系のスナップだと拍頭がマス境界に乗らないため、
  // マス格子とは独立にパルス単位で描く。
  const visibleEndPulse = scrollStep + visibleCols * snap;
  ctx.strokeStyle = '#4a4a4a';
  ctx.beginPath();
  for (let pulse = 0; pulse <= STEPS; pulse += PULSES_PER_BEAT) {
    if (pulse < scrollStep || pulse > visibleEndPulse) continue;
    const x = Math.round(pulseToX(pulse, scrollStep, cw, snap)) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  ctx.stroke();

  // 小節線（96パルスごと、さらに太く）。
  ctx.strokeStyle = '#6a6a6a';
  ctx.beginPath();
  for (let pulse = 0; pulse <= STEPS; pulse += PULSES_PER_BAR) {
    if (pulse < scrollStep || pulse > visibleEndPulse) continue;
    const x = Math.round(pulseToX(pulse, scrollStep, cw, snap)) + 0.5;
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
