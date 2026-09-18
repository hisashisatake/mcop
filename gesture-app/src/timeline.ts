// RHYTHM/MELODY画面共通のタイムライン・ルーラー行（再生位置移動・範囲選択）。
// 横スクロールバーの直上に常時表示する帯で、両画面が同じ状態を共有する
// （画面を切り替えても同じ再生位置・選択範囲が見える）。倍率と同じく表示/編集用の
// 状態でUndo対象外・プロジェクトファイル非保存（詳細はmemory
// project_gesture_app_timeline_ruler_plan参照）。
//
// 再生位置は「停止中に編集できる次回開始位置(startPulse)」と「再生中に受信する実際の
// 位置(livePulse、表示専用)」の2つを持つ。停止中はstartPulseを、再生中はlivePulseを
// 表示することで、「停止すると再生を開始した位置へ戻る」という挙動を無理なく表現する
// （再開のたびにRust側`clock_loop`がclock_totalをstartPulseへジャンプさせる実体は
// `set_playback_start_pulse`、`midi_out.rs`参照）。ライブseekはしない設計のため、
// 再生中はstartPulseを書き換えない。
//
// コード画面のコマ送り再生（`sequencerState.stepping`）中は、1拍ごとにrunningがtrue/false
// を行き来する（拍の頭で自動的に一時停止するため）。この「一時停止中」はstartPulseではなく
// livePulseの最後の値を見せ続ける必要がある——さもないと一時停止のたびに表示上の再生位置が
// 0（またはstartPulse）へ瞬間的に戻って見えてしまう（実機確認で発見・修正、2026-09-18）。
// stepping中はrunning/stepping問わずlivePulseを表示し、コマ送りを完全に抜けた
// （stepping=false、■停止）後だけstartPulseへ戻す。

import { PULSES_PER_BEAT, PULSES_PER_BAR, SEQUENCE_TOTAL_PULSES } from './grid-units.ts';
import { setPlaybackStartPulse } from './midi.ts';
import { sequencerState } from './sequencer-state.svelte.ts';

export const RULER_H = 18;
/** ルーラー上でドラッグと単純クリックを区別するしきい値（px、他画面のドラッグ判定と揃える）。 */
export const RULER_DRAG_THRESHOLD_PX = 4;

interface Selection {
  start: number;
  end: number;
}

let startPulse = 0;
let livePulse = 0;
let selection: Selection | null = null;

/** 今描画すべき再生位置（通常は停止中=startPulse・再生中=livePulse。コマ送り再生中は
 * 1拍ごとの自動一時停止でrunningがfalseに戻ってもlivePulseを見せ続ける、上記コメント参照）。 */
export function displayPulse(): number {
  return sequencerState.running || sequencerState.stepping ? livePulse : startPulse;
}

/** `rhythm-step`/`melody-step`イベント受信時に呼ぶ（表示専用、値の書き込みはしない）。 */
export function updateLivePulse(pulse: number): void {
  livePulse = pulse;
}

/** ルーラーのクリックで再生位置を移動する。再生中は無視する（ライブseekはしない設計）。 */
export function movePlayhead(pulse: number): void {
  if (sequencerState.running) return;
  startPulse = Math.max(0, Math.min(SEQUENCE_TOTAL_PULSES - 1, pulse));
  setPlaybackStartPulse(startPulse);
}

export function currentSelection(): Selection | null {
  return selection;
}

export function setSelection(start: number, end: number): void {
  selection = { start, end };
}

export function clearSelection(): void {
  selection = null;
}

export interface RulerLayout {
  left: number; // LABEL_WIDTH
  right: number; // gridRight
  top: number; // ルーラー帯の上端
  gridTop: number; // グリッド本体の上端（縦線を伸ばす範囲）
  gridBottom: number; // グリッド本体の下端（縦線を伸ばす範囲）
  pulseToX: (pulse: number) => number;
  visibleStartPulse: number;
  visibleEndPulse: number;
  /** 範囲選択のハイライトを描くか（既定true）。コード画面は範囲選択・削除を持たないため、
   * RHYTHM/MELODY画面で選択中の範囲（モジュール共有のselection状態）が紛れ込んで
   * 見えてしまわないようfalseを渡す。 */
  showSelection?: boolean;
}

/**
 * ルーラー帯（背景・選択範囲・小節番号・拍目盛り・再生位置マーカー）を描画する。
 * 座標変換は呼び出し側（各画面）が既存の`pulseToX`/`stepToX`をそのまま渡す
 * （描画と当たり判定で式を分けるとズレる、という既存の教訓に従う）。
 */
export function drawRuler(ctx: CanvasRenderingContext2D, layout: RulerLayout): void {
  const { left, right, top, gridTop, gridBottom, pulseToX, visibleStartPulse, visibleEndPulse, showSelection = true } = layout;

  const sel = showSelection ? selection : null;
  if (sel) {
    const s = Math.max(sel.start, visibleStartPulse);
    const e = Math.min(sel.end, visibleEndPulse);
    if (e > s) {
      const x0 = pulseToX(s);
      const x1 = pulseToX(e);
      // グリッド本体は薄い縦帯、ルーラー帯は濃いめでハイライトする（どの範囲が対象か
      // 一目で分かるように、CLAUDE.mdタイムライン・ルーラー計画の確定仕様）。
      ctx.fillStyle = 'rgba(120,170,255,0.12)';
      ctx.fillRect(x0, gridTop, x1 - x0, gridBottom - gridTop);
    }
  }

  ctx.fillStyle = '#1c1c1c';
  ctx.fillRect(left, top, right - left, RULER_H);

  if (sel) {
    const s = Math.max(sel.start, visibleStartPulse);
    const e = Math.min(sel.end, visibleEndPulse);
    if (e > s) {
      const x0 = pulseToX(s);
      const x1 = pulseToX(e);
      ctx.fillStyle = 'rgba(120,170,255,0.3)';
      ctx.fillRect(x0, top, x1 - x0, RULER_H);
      ctx.fillStyle = 'rgba(120,170,255,0.55)';
      ctx.fillRect(x0, top, x1 - x0, 3);
    }
  }

  // 拍目盛り（細）・小節目盛り（太め）
  ctx.strokeStyle = '#4a4a4a';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let pulse = 0; pulse <= SEQUENCE_TOTAL_PULSES; pulse += PULSES_PER_BEAT) {
    if (pulse < visibleStartPulse || pulse > visibleEndPulse) continue;
    const isBar = pulse % PULSES_PER_BAR === 0;
    const x = Math.round(pulseToX(pulse)) + 0.5;
    ctx.moveTo(x, top + RULER_H - (isBar ? 11 : 5));
    ctx.lineTo(x, top + RULER_H);
  }
  ctx.stroke();

  ctx.fillStyle = '#999';
  ctx.font = '10px monospace';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'top';
  for (let pulse = 0; pulse < SEQUENCE_TOTAL_PULSES; pulse += PULSES_PER_BAR) {
    if (pulse < visibleStartPulse || pulse > visibleEndPulse) continue;
    const bar = pulse / PULSES_PER_BAR + 1;
    ctx.fillText(String(bar), pulseToX(pulse) + 2, top + 1);
  }

  // 再生位置マーカー（三角＋縦線、グリッド本体を貫通させる）
  const playPulse = displayPulse();
  if (playPulse >= visibleStartPulse && playPulse <= visibleEndPulse) {
    const x = pulseToX(playPulse);
    ctx.strokeStyle = 'rgba(255,255,255,0.5)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(Math.round(x) + 0.5, gridTop);
    ctx.lineTo(Math.round(x) + 0.5, gridBottom);
    ctx.stroke();

    ctx.fillStyle = '#fff';
    ctx.beginPath();
    ctx.moveTo(x - 5, top);
    ctx.lineTo(x + 5, top);
    ctx.lineTo(x, top + 7);
    ctx.closePath();
    ctx.fill();
  }
}
