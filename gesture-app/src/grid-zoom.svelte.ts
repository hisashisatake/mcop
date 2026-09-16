// RHYTHM/MELODY画面共通のグリッド倍率（表示ズーム兼スナップ単位）。screens.svelte.tsと
// 同じ`export const xState = $state({...})`形式。倍率は表示設定のためUndo対象外・
// プロジェクトファイルにも保存しない（project-state.ts参照）。
//
// 値は`grid-units.ts`の`SNAP_PULSES`の添字（0=4分・7=32分3連）。既定はRHYTHM=4（16分、
// 6パルス/マス）・MELODY=2（8分、12パルス/マス）で、倍率UI導入前の固定スナップと
// 同じ見た目を保つ。

import { SNAP_PULSES } from './grid-units.ts';

export const gridZoomState = $state({
  rhythm: 4,
  melody: 2,
});

export function rhythmSnap(): number {
  return SNAP_PULSES[gridZoomState.rhythm];
}

export function melodySnap(): number {
  return SNAP_PULSES[gridZoomState.melody];
}

/** スライダー・−/+ボタン・Ctrl+ホイールから呼ぶ。0〜(SNAP_PULSES.length-1)にクランプする。 */
export function zoomStep(screen: 'rhythm' | 'melody', delta: number): void {
  const next = gridZoomState[screen] + delta;
  gridZoomState[screen] = Math.max(0, Math.min(SNAP_PULSES.length - 1, next));
}
