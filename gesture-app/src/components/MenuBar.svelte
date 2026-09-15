<script lang="ts">
  // 常時表示のメニューバー。☰でドロワー開閉、RHYTHM/MELODY画面では再生/停止アイコンを表示、
  // 右端にテンポ表示とタップテンポボタン（MC-505のタップボタン相当）。

  import { toggleDrawer } from '../drawer-state.svelte.ts';
  import { screenState } from '../screens.svelte.ts';
  import { tempoState, setBpm } from '../tempo-state.svelte.ts';
  import { sequencerState } from '../sequencer-state.svelte.ts';
  import { setSequencerRunning, tapTempo } from '../midi.ts';
  import { resetRhythmCursor } from '../rhythm-screen.ts';
  import { resetMelodyCursor } from '../melody-screen.ts';

  const MIN_BPM = 40;
  const MAX_BPM = 300;
  const RESET_GAP_MS = 2000;
  const MOVING_AVERAGE_WINDOW = 4; // 直近何区間を平均するか

  let tapTimestamps: number[] = [];

  const showSequencerButtons = $derived(screenState.active === 'rhythm' || screenState.active === 'melody');
  const tempoLabel = $derived(tempoState.bpm == null ? '— BPM' : `${Math.round(tempoState.bpm)} BPM`);

  function startSequencer(): void {
    sequencerState.running = true;
    setSequencerRunning(true);
  }

  function stopSequencer(): void {
    sequencerState.running = false;
    resetRhythmCursor();
    resetMelodyCursor();
    setSequencerRunning(false);
  }

  async function onTapTempo(): Promise<void> {
    const now = performance.now();
    if (tapTimestamps.length > 0 && now - tapTimestamps[tapTimestamps.length - 1] > RESET_GAP_MS) {
      tapTimestamps = [];
    }
    tapTimestamps.push(now);
    if (tapTimestamps.length > MOVING_AVERAGE_WINDOW + 1) {
      tapTimestamps.shift();
    }
    if (tapTimestamps.length < 3) {
      return; // 3タップ（2区間）が揃うまではBPMを確定しない
    }
    const intervals: number[] = [];
    for (let i = 1; i < tapTimestamps.length; i++) {
      intervals.push(tapTimestamps[i] - tapTimestamps[i - 1]);
    }
    const avgIntervalMs = intervals.reduce((a, b) => a + b, 0) / intervals.length;
    const bpm = Math.max(MIN_BPM, Math.min(MAX_BPM, 60000 / avgIntervalMs));
    setBpm(bpm);
    await tapTempo(bpm);
  }
</script>

<div id="menu-bar">
  <button type="button" id="menu-toggle" title="メニュー" onclick={toggleDrawer}>☰</button>
  {#if showSequencerButtons}
    <button type="button" id="sequencer-play-btn" title="再生" disabled={sequencerState.running} onclick={startSequencer}>▶</button>
    <button type="button" id="sequencer-stop-btn" title="停止" disabled={!sequencerState.running} onclick={stopSequencer}>■</button>
  {/if}
  <span id="tempo-display" class="menu-bar-label">{tempoLabel}</span>
  <button id="tap-tempo-btn" type="button" title="タップテンポ" onclick={onTapTempo}>TAP</button>
</div>
