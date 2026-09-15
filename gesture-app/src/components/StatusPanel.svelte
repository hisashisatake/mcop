<script lang="ts">
  // 右下の常時表示ステータス（波形メモリ切替・Bank/Program・Key）。演奏中も見えている
  // 必要があるためドロワーの外に置く。Key選択はコード画面専用。
  import { onMount } from 'svelte';
  import { screenState } from '../screens.svelte.ts';
  import { chordSettings } from '../chord-settings.svelte.ts';
  import { setTonicPitchClass, setChordMode } from '../chord-screen.ts';
  import { programState, setWaveformMemory, setBank, setProgramNumber, initProgramState } from '../program-state.svelte.ts';
  import { NOTE_NAMES } from '../chords.ts';
  import type { Mode } from '../types.ts';

  onMount(() => {
    initProgramState();
  });

  const tonicPitchClass = $derived(((chordSettings.tonicMidi % 12) + 12) % 12);
</script>

<div id="status-panel">
  <div class="program-row">
    <label>
      <input type="checkbox" id="waveform-memory-toggle" checked={programState.waveformMemory} onchange={(e) => setWaveformMemory((e.currentTarget as HTMLInputElement).checked)} />
      波形メモリ
    </label>
    <span id="program-label">{programState.label}</span>
  </div>
  <div class="program-row">
    Bank
    <input
      type="number"
      id="program-bank"
      min="0"
      max="16383"
      value={programState.bank}
      disabled={programState.waveformMemory}
      oninput={(e) => setBank(parseInt((e.currentTarget as HTMLInputElement).value, 10))}
    />
    Program
    <input
      type="number"
      id="program-num"
      min="0"
      max="127"
      value={programState.program}
      oninput={(e) => setProgramNumber(parseInt((e.currentTarget as HTMLInputElement).value, 10))}
    />
  </div>
  {#if screenState.active === 'chord'}
    <div id="status-key-row">
      <div class="program-row">
        Key
        <select id="key-tonic" value={tonicPitchClass} onchange={(e) => setTonicPitchClass(parseInt((e.currentTarget as HTMLSelectElement).value, 10))}>
          {#each NOTE_NAMES as name, i (i)}
            <option value={i}>{name}</option>
          {/each}
        </select>
        <select id="key-mode" value={chordSettings.mode} onchange={(e) => setChordMode((e.currentTarget as HTMLSelectElement).value as Mode)}>
          <option value="major">Major</option>
          <option value="minor">Minor</option>
        </select>
      </div>
    </div>
  {/if}
</div>
