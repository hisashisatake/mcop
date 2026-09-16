// キャンバス描画のスクロールバー共通ロジック（トラック/つまみ計算・ヒットテスト・
// ドラッグ時の位置逆算）。RHYTHM/MELODY画面はどちらも「固定セル＋スクロール」方式だが、
// スクロール量の単位がそれぞれ行数（整数）・パルス数（snapの倍数）と異なるため、
// このモジュールは単位を持たない「トラック長・コンテンツ量・可視量・現在位置」の
// 4値だけを扱う（単位変換は呼び出し側が担う）。

export const SB_THICKNESS = 10;
const MIN_THUMB_LEN = 24;

export interface ThumbLayout {
  thumbStart: number; // トラック座標系でのつまみ開始px
  thumbLen: number;
}

/** 1本のスクロールバーのジオメトリ（呼び出し側が画面のレイアウトから組み立てて保持する）。 */
export interface ScrollbarGeom {
  trackStart: number; // トラック開始px（縦=TOP_MARGIN、横=LABEL_WIDTH）
  trackLen: number;
  barStart: number; // オーバーレイ描画位置（縦=gridRight-SB_THICKNESS、横=gridBottom-SB_THICKNESS）
  thumb: ThumbLayout | null; // コンテンツが可視範囲に収まっているならnull（バー自体非表示）
  contentUnits: number;
  viewportUnits: number;
}

export function computeThumb(trackStart: number, trackLen: number, contentUnits: number, viewportUnits: number, scrollUnits: number): ThumbLayout {
  const thumbLen = Math.max(MIN_THUMB_LEN, Math.min(trackLen, (trackLen * viewportUnits) / contentUnits));
  const maxScroll = Math.max(0, contentUnits - viewportUnits);
  const range = trackLen - thumbLen;
  const thumbStart = trackStart + (maxScroll > 0 ? range * clamp01(scrollUnits / maxScroll) : 0);
  return { thumbStart, thumbLen };
}

export function isInThumb(pos: number, thumb: ThumbLayout): boolean {
  return pos >= thumb.thumbStart && pos < thumb.thumbStart + thumb.thumbLen;
}

/** つまみドラッグ中: つまみ先頭の新しいトラック内px位置から、コンテンツ単位での
 * スクロール量を逆算する（0〜maxScrollへクランプ済み）。 */
export function scrollFromThumbStart(trackStart: number, trackLen: number, thumb: ThumbLayout, contentUnits: number, viewportUnits: number, newThumbStartPx: number): number {
  const maxScroll = Math.max(0, contentUnits - viewportUnits);
  const range = trackLen - thumb.thumbLen;
  const ratio = range > 0 ? clamp01((newThumbStartPx - trackStart) / range) : 0;
  return ratio * maxScroll;
}

/** トラック上（つまみ外）クリック時、つまみをどちら向きへ1画面分動かすか。 */
export function pageJumpDirection(pos: number, thumb: ThumbLayout): 1 | -1 {
  return pos < thumb.thumbStart ? -1 : 1;
}

function clamp01(v: number): number {
  return Math.max(0, Math.min(1, v));
}
