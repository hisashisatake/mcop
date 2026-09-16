// リズム画面（フェーズ4：ステップシーケンサー本体、フェーズ3で行の動的化に対応）。
//
// 内部データは1パルス=1/96小節（`grid-units.ts`参照）で持つ。このステップではまだ
// 「マス幅を変えられる」倍率UIを持たないため、既定の16分音符(FIXED_SNAP=6パルス/マス)に
// 固定した見た目（従来と同じ16マス/行）で描画・クリック判定する（倍率スライダー自体は
// 次のステップで追加する）。読み取りは`cellLevel()`でマスが覆う6パルスの範囲内の
// 最大値を見る（そのマスが今どう聞こえているかの代表値）。書き込みはマスの先頭パルス
// 1個だけに行う（移行データ・新規クリックとも常に6パルスの先頭にのみ非0値が立つ運用の
// ため、範囲全体への一括書き込みが要るのは倍率を粗くしてから、という次のステップの話）。
//
// セルをクリックするたびに消音→通常→アクセント→弱→消音…の順にベロシティが巡回する
// （3段階＋消音）。発音のタイミング自体はRust側`clock_loop`が持つ（JSタイマーは数十msの誤差が
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
import { onRhythmStepTick, setRhythmStep, setRhythmRowsBulk } from './midi.ts';
import { pushUndo } from './undo-manager.ts';
import { PULSES_PER_BAR, SNAP_PULSES, cellStartPulse, cellLevel } from './grid-units.ts';
import { expandPatternV1 } from './grid-migrate.ts';
import type { RhythmRow } from './types.ts';

// MIDI Import/Export（project-file.js）が1小節のパルス数として参照するためexportする。
export const STEPS = PULSES_PER_BAR;
// 倍率UI導入までの固定スナップ（16分音符=6パルス/マス、従来の見た目と一致させる）。
const FIXED_SNAP = SNAP_PULSES[4];
const VISUAL_CELLS = STEPS / FIXED_SNAP; // 16（従来と同じマス数）
const STEP_GROUP = 4; // 4マス（1拍）ごとに区切り線を太くする
const LABEL_WIDTH = 74;
const TOP_MARGIN = 40; // 上部の余白（画面タブ等はハンバーガーメニューのドロワーへ移動済みのため最小限でよい）
const BOTTOM_MARGIN = 180; // 右下固定の#status-panel（波形メモリ/Bank・Program/Key/TAPテンポ）と最下段の行が重ならないための余白
const MIN_ROW_H = 18; // これを下回る行高になる場合は固定してスクロールに切り替える

// 既定12行のノート番号と短縮ラベル（見た目は従来のまま）。Rust側は行の概念を持たず
// ノート番号で直接引くため、この対応表はJS側だけが持つ。project-state.jsのv1(.gap505)
// 互換読込（`patternV1ToRows`）でも使う。
export const DEFAULT_ROW_NOTES = [49, 51, 46, 42, 39, 37, 38, 40, 48, 45, 41, 36];
export const DEFAULT_ROW_LABELS = ['Crash', 'Ride', 'OpenHH', 'ClosedHH', 'Clap', 'Rim', 'Snare', 'E.Snare', 'HiTom', 'MidTom', 'LoTom', 'Kick'];

// [消音, 通常, アクセント, 弱]。Rust側の0〜3と対応。
const LEVEL_COLORS = ['#1c1c1c', 'hsl(200,55%,42%)', 'hsl(32,90%,55%)', 'hsl(200,35%,26%)'];

let rows: RhythmRow[] = DEFAULT_ROW_NOTES.map((note, i) => ({ note, label: DEFAULT_ROW_LABELS[i], steps: new Array(STEPS).fill(0) }));
let scrollRow = 0;
let currentStep = -1;

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

onRhythmStepTick((step) => {
  currentStep = step;
});

export function setupRhythmScreen(canvas: HTMLCanvasElement): { draw: (ctx: CanvasRenderingContext2D) => void } {
  canvas.addEventListener('mousedown', (e) => {
    if (!isActive('rhythm') || e.button !== 0) return;
    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    if (!cell) return;
    const row = rows[cell.rowIndex];
    const current = cellLevel(row.steps, cell.step, FIXED_SNAP);
    const next = (current + 1) % 4;
    pushUndo();
    row.steps[cell.step] = next; // マスの先頭パルスにのみ書く（FIXED_SNAP=6の残り5パルスは0のまま）
    setRhythmStep(row.note, cell.step, next);
  });

  canvas.addEventListener(
    'wheel',
    (e) => {
      if (!isActive('rhythm')) return;
      const { visibleRows } = gridMetrics(canvas);
      if (visibleRows >= rows.length) return; // 全行表示できているならスクロール不要
      e.preventDefault();
      scrollRow = clamp(scrollRow + Math.sign(e.deltaY), 0, Math.max(0, rows.length - visibleRows));
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
}

function gridMetrics(canvas: HTMLCanvasElement): GridMetrics {
  const cw = (canvas.width - LABEL_WIDTH) / VISUAL_CELLS;
  const bodyH = canvas.height - TOP_MARGIN - BOTTOM_MARGIN;
  const naturalCh = rows.length > 0 ? bodyH / rows.length : bodyH;
  const ch = Math.max(MIN_ROW_H, naturalCh);
  const visibleRows = ch <= naturalCh ? rows.length : Math.max(1, Math.floor(bodyH / ch));
  return { cw, ch, bodyH, visibleRows };
}

/** 戻り値の`step`はマスの先頭パルス（`FIXED_SNAP`刻み）。 */
function cellFromPoint(canvas: HTMLCanvasElement, px: number, py: number): { rowIndex: number; step: number } | null {
  const { cw, ch, visibleRows } = gridMetrics(canvas);
  const x = px - LABEL_WIDTH;
  const y = py - TOP_MARGIN;
  if (x < 0 || y < 0) return null;
  const cellIndex = Math.floor(x / cw);
  const r = Math.floor(y / ch);
  if (cellIndex < 0 || cellIndex >= VISUAL_CELLS || r < 0 || r >= visibleRows) return null;
  const rowIndex = scrollRow + r;
  if (rowIndex >= rows.length) return null;
  return { rowIndex, step: cellStartPulse(cellIndex, FIXED_SNAP) };
}

function draw(ctx: CanvasRenderingContext2D, canvas: HTMLCanvasElement): void {
  if (!isActive('rhythm')) return;
  const W = canvas.width;
  const H = canvas.height;
  const { cw, ch, visibleRows } = gridMetrics(canvas);
  const gridBottom = TOP_MARGIN + visibleRows * ch;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  // セル本体（可視範囲のみ）。マスが覆うFIXED_SNAP個のパルスのうち最大レベルを代表値として描く。
  for (let r = 0; r < visibleRows; r++) {
    const rowIndex = scrollRow + r;
    if (rowIndex >= rows.length) break;
    const row = rows[rowIndex];
    for (let cellIndex = 0; cellIndex < VISUAL_CELLS; cellIndex++) {
      const level = cellLevel(row.steps, cellStartPulse(cellIndex, FIXED_SNAP), FIXED_SNAP);
      ctx.fillStyle = LEVEL_COLORS[level];
      ctx.fillRect(LABEL_WIDTH + cellIndex * cw + 1, TOP_MARGIN + r * ch + 1, cw - 2, ch - 2);
    }
  }

  // 再生カーソル（現在鳴っているステップの列を薄くハイライト）。セル本体より後に
  // 描かないと、不透明なセルの塗りで上書きされて見えなくなる。
  if (currentStep >= 0) {
    ctx.fillStyle = 'rgba(255,255,255,0.14)';
    ctx.fillRect(LABEL_WIDTH + currentStep * cw, TOP_MARGIN, cw, gridBottom - TOP_MARGIN);
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

  // 格子線（4マス＝1拍ごとに明るく）
  ctx.strokeStyle = '#2a2a2a';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let cellIndex = 0; cellIndex <= VISUAL_CELLS; cellIndex++) {
    const x = Math.round(LABEL_WIDTH + cellIndex * cw) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  for (let r = 0; r <= visibleRows; r++) {
    const y = Math.round(TOP_MARGIN + r * ch) + 0.5;
    ctx.moveTo(LABEL_WIDTH, y);
    ctx.lineTo(W, y);
  }
  ctx.stroke();

  ctx.strokeStyle = '#4a4a4a';
  ctx.beginPath();
  for (let cellIndex = 0; cellIndex <= VISUAL_CELLS; cellIndex += STEP_GROUP) {
    const x = Math.round(LABEL_WIDTH + cellIndex * cw) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  ctx.stroke();
}
