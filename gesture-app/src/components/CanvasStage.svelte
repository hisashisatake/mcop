<script lang="ts">
  // コード/リズム/メロディ3画面共通のcanvas。各画面モジュールはcanvas/windowへ直接
  // イベントリスナーを貼る命令的な実装のままなので、ここではonMountでの配線と
  // requestAnimationFrameの描画ループだけを持つ（画面自体のリアクティブ化はしない設計、
  // [[project_gesture_app_svelte_migration_consult]]参照）。

  import { onMount } from 'svelte';
  import { setupChordScreen, activeChannels } from '../chord-screen.ts';
  import { setupRhythmScreen } from '../rhythm-screen.ts';
  import { setupMelodyScreen } from '../melody-screen.ts';
  import { setupPerformanceLfo } from '../performance-lfo.svelte.ts';
  import { activeScreen, screenState } from '../screens.svelte.ts';
  import { setHudChordInfo } from '../hud-state.svelte.ts';

  let canvasEl: HTMLCanvasElement;

  onMount(() => {
    const ctx = canvasEl.getContext('2d')!;

    function resize(): void {
      canvasEl.width = window.innerWidth;
      canvasEl.height = window.innerHeight;
    }
    window.addEventListener('resize', resize);
    resize();

    const chordScreen = setupChordScreen(canvasEl, { onChordChange: setHudChordInfo });
    const rhythmScreen = setupRhythmScreen(canvasEl);
    const melodyScreen = setupMelodyScreen(canvasEl);
    setupPerformanceLfo(canvasEl, activeChannels);

    let raf = 0;
    function tick(): void {
      const screen = activeScreen();
      if (screen === 'rhythm') rhythmScreen.draw(ctx);
      else if (screen === 'melody') melodyScreen.draw(ctx);
      else chordScreen.draw(ctx);
      raf = requestAnimationFrame(tick);
    }
    tick();

    return () => {
      window.removeEventListener('resize', resize);
      cancelAnimationFrame(raf);
    };
  });
</script>

<canvas id="canvas" bind:this={canvasEl} class:default-cursor={screenState.active !== 'chord'}></canvas>
