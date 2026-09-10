// フロー方式コード画面：過去スロットの幾何計算。Canvas/DOM非依存の純粋関数のみを置く
// （chord-flow.js/theory.jsと同じ方針）。
//
// 過去コードは現在スロットの左へ、遠い（indexが大きい）ほどサイズも間隔も指数的に縮む
// （遠近法的な配置）。描画側（chord-screen.jsのdraw）と当たり判定側（cellFromPoint）が
// 必ず同じ配列を参照することで、フロー方式刷新時に踏んだ「当たり判定と描画位置がズレる」
// バグ（[[project_gesture_app_3screen_minidaw_redesign]]参照）を構造的に防ぐ。

/**
 * 現在スロットの左に並ぶ過去スロットの幾何（中心座標とサイズ）を返す。
 * index=0が現在の直前（cursor-1相当）。左端（leftMargin）に収まらなくなった時点、
 * またはcount個に達した時点で打ち切るため、固定の上限個数は持たない。
 *
 * @returns {Array<{index: number, x: number, y: number, size: number}>}
 */
export function computePastSlotGeoms({
  currentX,
  currentSize,
  slotY,
  baseSize,
  count,
  minSize = 20,
  shrink = 0.8,
  gapRatio = 0.125,
  leftMargin = 16,
}) {
  const geoms = [];
  let rightEdge = currentX - currentSize / 2;
  for (let i = 0; i < count; i++) {
    const size = Math.max(minSize, baseSize * shrink ** i);
    const gap = size * gapRatio;
    const centerX = rightEdge - gap - size / 2;
    const leftEdge = centerX - size / 2;
    if (leftEdge < leftMargin) break;
    geoms.push({ index: i, x: centerX, y: slotY, size });
    rightEdge = leftEdge;
  }
  return geoms;
}
