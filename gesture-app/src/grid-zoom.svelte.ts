// RHYTHM/MELODY画面共通のグリッド倍率（表示ズーム兼スナップ単位）。screens.svelte.tsと
// 同じ`export const xState = $state({...})`形式。倍率は表示設定のためUndo対象外・
// プロジェクトファイルにも保存しない（project-state.ts参照）。
//
// 値は`grid-units.ts`の`SNAP_PULSES`の添字（0=4分・7=32分3連）。RHYTHM/MELODYで
// 独立していた倍率を1本化し、両画面が同じ倍率を共有する（タイムラインの長さも
// SEQUENCE_TOTAL_PULSESで共通化済み、ユーザー要望）。既定は4（16分、6パルス/マス、
// 旧RHYTHM既定と同じ）。

import { SNAP_PULSES } from './grid-units.ts';

export const gridZoomState = $state({
  index: 4,
});

export function gridSnap(): number {
  return SNAP_PULSES[gridZoomState.index];
}

/** スライダー・−/+ボタン・Ctrl+ホイールから呼ぶ。0〜(SNAP_PULSES.length-1)にクランプする。 */
export function zoomStep(delta: number): void {
  const next = gridZoomState.index + delta;
  gridZoomState.index = Math.max(0, Math.min(SNAP_PULSES.length - 1, next));
}
