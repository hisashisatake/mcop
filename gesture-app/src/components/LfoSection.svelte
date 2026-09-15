<script lang="ts">
  // 演奏系LFO（ホイールでDepth、V/C/Bキーで行き先・Rate）の表示。実際の値変更は
  // performance-lfo.svelte.tsのsetupPerformanceLfo()がcanvas/window側で配線する
  // （CanvasStage.svelte参照）ため、ここは表示専用。
  import { performanceLfoState, lfoDestinationLabel } from '../performance-lfo.svelte.ts';

  const depthPercent = $derived((performanceLfoState.modWheel / 255) * 100);
  const ratePercent = $derived((performanceLfoState.rate / 255) * 100);
</script>

<div id="lfo-section">
  <div class="drawer-section-title">PERFORMANCE LFO</div>
  <div class="lfo-row" id="lfo-label" style:color={performanceLfoState.modWheel > 0 ? '#4af' : '#666'}>
    LFO: {lfoDestinationLabel()} (V)
  </div>
  <div class="lfo-bar-track"><div class="lfo-bar-fill" id="lfo-depth-bar" style:width="{depthPercent}%"></div></div>
  <div class="lfo-row" id="lfo-rate-label">Rate: {performanceLfoState.rate} (C/B)</div>
  <div class="lfo-bar-track"><div class="lfo-bar-fill" id="lfo-rate-bar" style:width="{ratePercent}%"></div></div>
</div>
