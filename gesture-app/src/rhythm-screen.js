// リズム画面（フェーズ4：ステップシーケンサー本体）。
//
// 16ステップ×12打楽器行のグリッド。セルをクリックするたびに
// 消音→通常→アクセント→弱→消音…の順にベロシティが巡回する（3段階＋消音）。
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

import { isActive } from './screens.js';
import { setMetronomeEnabled, onRhythmStepTick, setRhythmStep } from './midi.js';
import { pushUndo } from './undo-manager.js';

const ROWS = 12;
const STEPS = 16;
const STEP_GROUP = 4; // 4ステップ（1拍）ごとに区切り線を太くする
const LABEL_WIDTH = 74;
const TOP_MARGIN = 40; // 上部の余白（画面タブ等はハンバーガーメニューのドロワーへ移動済みのため最小限でよい）
const BOTTOM_MARGIN = 180; // 右下固定の#status-panel（波形メモリ/Bank・Program/Key/TAPテンポ）と最下段の行が重ならないための余白

// Rust側`DRUM_NOTES`（midi_out.rs）と行の並びを一致させること。JSは行番号だけを
// やり取りし、実際のGM2ノート番号はRust側が持つ。
const ROW_LABELS = [
  'Crash', 'Ride', 'OpenHH', 'ClosedHH', 'Clap', 'Rim',
  'Snare', 'E.Snare', 'HiTom', 'MidTom', 'LoTom', 'Kick',
];

// [消音, 通常, アクセント, 弱]。Rust側の0〜3と対応。
const LEVEL_COLORS = ['#1c1c1c', 'hsl(200,55%,42%)', 'hsl(32,90%,55%)', 'hsl(200,35%,26%)'];

let pattern = Array.from({ length: ROWS }, () => new Array(STEPS).fill(0));
let currentStep = -1;

onRhythmStepTick((step) => {
  currentStep = step;
});

export function setupRhythmScreen(canvas) {
  canvas.addEventListener('mousedown', (e) => {
    if (!isActive('rhythm') || e.button !== 0) return;
    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    if (!cell) return;
    const next = (pattern[cell.row][cell.step] + 1) % 4;
    pushUndo();
    pattern[cell.row][cell.step] = next;
    setRhythmStep(cell.row, cell.step, next);
  });

  return { draw: (ctx) => draw(ctx, canvas) };
}

/**
 * このリズム画面が持つ状態（パターン）を取得する。project-state.jsがプロジェクト全体の
 * スナップショットを組み立てる際に呼ぶ。`pattern`は以後この画面側でセルごとに書き換えられて
 * いく実体なので、スナップショットには複製を返す（統合Undo/Redoのスタックへ積んだ後で
 * 元の配列が書き換わり、過去のスナップショットまで壊れてしまう事故を防ぐため）。
 */
export function getPattern() {
  return pattern.map((row) => row.slice());
}

/**
 * 統合Undo/Redo・ファイル読込による復元用。セルごとに現在値と比較し、変化した位置だけ
 * Rust側の共有パターンへ`setRhythmStep`でミラーする（192セル全部を無条件で送ると、
 * リズムを一切編集していないUndo/Redoでも毎回192回のinvokeが走ってしまうため）。
 */
export function setPattern(newPattern) {
  for (let row = 0; row < ROWS; row++) {
    for (let step = 0; step < STEPS; step++) {
      const level = newPattern[row][step];
      if (pattern[row][step] !== level) {
        pattern[row][step] = level;
        setRhythmStep(row, step, level);
      }
    }
  }
}

/** メトロノームON/OFFのチェックボックスを配線する。リズム/メロディ共通の再生/停止
 * ボタンはmain.js側でまとめて配線する（停止時に両画面のカーソルを揃えてリセットする
 * 必要があるため、両画面を知っているmain.jsが持つのが自然。詳細はmidi_out.rs
 * `SEQUENCER_RUNNING`のコメント参照）。 */
export function bindRhythmScreenControls({ metronomeToggle }) {
  if (!metronomeToggle) return;
  metronomeToggle.checked = false;
  metronomeToggle.addEventListener('change', () => {
    setMetronomeEnabled(metronomeToggle.checked);
  });
}

/** 再生/停止ボタンの停止側からmain.js経由で呼ばれる。再生カーソルのハイライトを
 * 即座に消す（Rust側は`rhythm-step`イベントの送出自体を止めるだけで、直前のカーソル
 * 位置を明示的に片付けてはくれないため）。 */
export function resetRhythmCursor() {
  currentStep = -1;
}

function gridMetrics(canvas) {
  const cw = (canvas.width - LABEL_WIDTH) / STEPS;
  const ch = (canvas.height - TOP_MARGIN - BOTTOM_MARGIN) / ROWS;
  return { cw, ch };
}

function cellFromPoint(canvas, px, py) {
  const { cw, ch } = gridMetrics(canvas);
  const x = px - LABEL_WIDTH;
  const y = py - TOP_MARGIN;
  if (x < 0 || y < 0) return null;
  const step = Math.floor(x / cw);
  const row = Math.floor(y / ch);
  if (step < 0 || step >= STEPS || row < 0 || row >= ROWS) return null;
  return { row, step };
}

function draw(ctx, canvas) {
  if (!isActive('rhythm')) return;
  const W = canvas.width;
  const H = canvas.height;
  const { cw, ch } = gridMetrics(canvas);
  const gridBottom = TOP_MARGIN + ROWS * ch;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  // セル本体
  for (let row = 0; row < ROWS; row++) {
    for (let step = 0; step < STEPS; step++) {
      ctx.fillStyle = LEVEL_COLORS[pattern[row][step]];
      ctx.fillRect(LABEL_WIDTH + step * cw + 1, TOP_MARGIN + row * ch + 1, cw - 2, ch - 2);
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
  for (let row = 0; row < ROWS; row++) {
    ctx.fillText(ROW_LABELS[row], 8, TOP_MARGIN + row * ch + ch / 2);
  }
  ctx.textBaseline = 'alphabetic';

  // 格子線（4ステップ＝1拍ごとに明るく）
  ctx.strokeStyle = '#2a2a2a';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let step = 0; step <= STEPS; step++) {
    const x = Math.round(LABEL_WIDTH + step * cw) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  for (let row = 0; row <= ROWS; row++) {
    const y = Math.round(TOP_MARGIN + row * ch) + 0.5;
    ctx.moveTo(LABEL_WIDTH, y);
    ctx.lineTo(W, y);
  }
  ctx.stroke();

  ctx.strokeStyle = '#4a4a4a';
  ctx.beginPath();
  for (let step = 0; step <= STEPS; step += STEP_GROUP) {
    const x = Math.round(LABEL_WIDTH + step * cw) + 0.5;
    ctx.moveTo(x, TOP_MARGIN);
    ctx.lineTo(x, gridBottom);
  }
  ctx.stroke();
}
