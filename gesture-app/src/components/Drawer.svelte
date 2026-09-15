<script lang="ts">
  // ハンバーガーメニューのドロワー本体。画面タブ・File・演奏LFO表示・詳細設定・
  // ヒント・MIDIログを収める。開閉はdrawer-state.svelte.ts、画面切り替えは
  // screens.svelte.tsのsetActiveScreen()を直接呼ぶ。
  import { drawerState } from '../drawer-state.svelte.ts';
  import { screenState, setActiveScreen } from '../screens.svelte.ts';
  import FileSection from './FileSection.svelte';
  import LfoSection from './LfoSection.svelte';
  import ProgramPanel from './ProgramPanel.svelte';
  import Hint from './Hint.svelte';
  import MidiLog from './MidiLog.svelte';
</script>

<div id="drawer" class:open={drawerState.open}>
  <div id="drawer-tabs">
    <button type="button" id="tab-chord" class:active={screenState.active === 'chord'} onclick={() => setActiveScreen('chord')}>CHORD</button>
    <button type="button" id="tab-rhythm" class:active={screenState.active === 'rhythm'} onclick={() => setActiveScreen('rhythm')}>RHYTHM</button>
    <button type="button" id="tab-melody" class:active={screenState.active === 'melody'} onclick={() => setActiveScreen('melody')}>MELODY</button>
  </div>

  <FileSection />
  <LfoSection />
  <ProgramPanel />
  <Hint />
  <MidiLog />
</div>
