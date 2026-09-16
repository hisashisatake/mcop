<script lang="ts">
  // RHYTHM/MELODY画面共通のグリッド倍率スライダー（右下、#status-panelの真上）。
  // スライダー自体はDOM（Svelte）にする——キーボード操作・フォーカスが無料で付き、
  // canvasに独自ヒットテストを足さずに済む（既存の「chromeはDOM・グリッドだけcanvas」
  // という分担と一致）。App.svelteが{#if screenState.active !== 'chord'}で出し入れする
  // ため、ここでは常にrhythm/melodyのどちらかがアクティブという前提で良い。
  import { screenState } from '../screens.svelte.ts';
  import { gridZoomState, zoomStep } from '../grid-zoom.svelte.ts';
  import { SNAP_LABELS } from '../grid-units.ts';

  const screen = $derived(screenState.active === 'melody' ? 'melody' : 'rhythm');
  const value = $derived(gridZoomState[screen]);
  const label = $derived(SNAP_LABELS[value]);

  function onInput(e: Event): void {
    gridZoomState[screen] = parseInt((e.currentTarget as HTMLInputElement).value, 10);
  }
</script>

<div id="grid-zoom">
  <button type="button" title="粗く" disabled={value === 0} onclick={() => zoomStep(screen, -1)}>−</button>
  <input type="range" min="0" max={SNAP_LABELS.length - 1} step="1" value={value} oninput={onInput} />
  <button type="button" title="細かく" disabled={value === SNAP_LABELS.length - 1} onclick={() => zoomStep(screen, 1)}>+</button>
  <span id="grid-zoom-label">{label}</span>
</div>
