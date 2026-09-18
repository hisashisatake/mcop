<script lang="ts">
  // 画面ごとの詳細設定パネル。CHORD画面: 候補行数/列数・自動転回・基準オクターブ・コマ送り単位
  // （chord-settings.svelte.tsの$stateを読み、変更はchord-screen.ts/transport.tsの関数経由）。
  // RHYTHM画面: メトロノームON/OFF（他モジュールから読まれないため、ここではローカル状態）。
  import { screenState } from '../screens.svelte.ts';
  import { chordSettings } from '../chord-settings.svelte.ts';
  import { setAssistRows, setAssistCols, setAutoVoicing, setBaseOctave, effectiveAutoVoicing } from '../chord-screen.ts';
  import { setStepUnit } from '../transport.ts';
  import { setMetronomeEnabled } from '../midi.ts';
  import { STEP_UNIT_LABELS } from '../grid-units.ts';

  let metronomeOn = $state(false);

  function onMetronomeChange(e: Event): void {
    metronomeOn = (e.currentTarget as HTMLInputElement).checked;
    setMetronomeEnabled(metronomeOn);
  }
</script>

<div id="program-panel">
  <div class="drawer-section-title">詳細設定</div>
  {#if screenState.active === 'chord'}
    <div id="chord-controls">
      <div class="program-row">
        候補 行数
        <input
          type="number"
          id="candidate-rows"
          min="3"
          max="12"
          value={chordSettings.rows}
          oninput={(e) => setAssistRows(parseInt((e.currentTarget as HTMLInputElement).value, 10))}
        />
        列数
        <input
          type="number"
          id="candidate-cols"
          min="1"
          max="3"
          value={chordSettings.cols}
          oninput={(e) => setAssistCols(parseInt((e.currentTarget as HTMLInputElement).value, 10))}
        />
      </div>
      <div class="program-row">
        <label>
          <input
            type="checkbox"
            id="auto-voicing-toggle"
            checked={effectiveAutoVoicing()}
            onchange={(e) => setAutoVoicing((e.currentTarget as HTMLInputElement).checked)}
          />
          自動転回
        </label>
        基準Oct
        <input
          type="number"
          id="base-octave"
          min="-2"
          max="2"
          value={chordSettings.baseOctave}
          oninput={(e) => setBaseOctave(parseInt((e.currentTarget as HTMLInputElement).value, 10))}
        />
      </div>
      <div class="program-row">
        コマ送り単位
        <select id="step-unit" value={chordSettings.stepUnitIndex} onchange={(e) => setStepUnit(parseInt((e.currentTarget as HTMLSelectElement).value, 10))}>
          {#each STEP_UNIT_LABELS as label, i (label)}
            <option value={i}>{label}</option>
          {/each}
        </select>
      </div>
    </div>
  {:else if screenState.active === 'rhythm'}
    <div id="rhythm-controls">
      <div class="program-row">
        <label><input type="checkbox" id="metronome-toggle" checked={metronomeOn} onchange={onMetronomeChange} /> メトロノーム</label>
      </div>
    </div>
  {/if}
</div>
