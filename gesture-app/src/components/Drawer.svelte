<script lang="ts">
  // ハンバーガーメニューのドロワー本体。画面タブ・File・演奏LFO表示・詳細設定・
  // ヒント・MIDIログを収める。開閉はdrawer-state.svelte.ts、画面切り替えは
  // screens.svelte.tsのsetActiveScreen()を直接呼ぶ。
  //
  // 閉じるタイミングは3つ: (1)ハンバーガー再クリック(MenuBar側のtoggleDrawer)、
  // (2)画面タブクリック（下記selectScreen、既にアクティブな画面を再クリックした場合も含めて
  // 明示的に閉じる——setActiveScreen()は画面が実際に変わらないとonScreenChangeを発火せず
  // 閉じないため）、(3)ドロワー外クリック（下記onWindowMouseDown、ユーザー要望で追加）。
  import { onMount } from 'svelte';
  import { drawerState, closeDrawer } from '../drawer-state.svelte.ts';
  import { screenState, setActiveScreen } from '../screens.svelte.ts';
  import FileSection from './FileSection.svelte';
  import LfoSection from './LfoSection.svelte';
  import ProgramPanel from './ProgramPanel.svelte';
  import Hint from './Hint.svelte';
  import MidiLog from './MidiLog.svelte';

  let drawerEl: HTMLDivElement;

  function selectScreen(name: 'chord' | 'rhythm' | 'melody'): void {
    setActiveScreen(name);
    closeDrawer();
  }

  onMount(() => {
    // ドロワー外クリックで閉じる。ハンバーガーボタン(#menu-toggle)自体のクリックは除外する
    // ——除外しないと、閉じている状態からハンバーガーを押した瞬間にtoggleDrawer()で開いた
    // 直後、同じクリックをこのリスナーが「外側クリック」と見なして即座に閉じ直してしまう
    // （両ハンドラともmousedownで、開く処理の方が先に走るため）。
    function onWindowMouseDown(e: MouseEvent): void {
      if (!drawerState.open) return;
      const target = e.target as Node;
      if (drawerEl.contains(target)) return;
      if ((e.target as HTMLElement).closest?.('#menu-toggle')) return;
      closeDrawer();
    }
    window.addEventListener('mousedown', onWindowMouseDown);
    return () => window.removeEventListener('mousedown', onWindowMouseDown);
  });
</script>

<div id="drawer" bind:this={drawerEl} class:open={drawerState.open}>
  <div id="drawer-tabs">
    <button type="button" id="tab-chord" class:active={screenState.active === 'chord'} onclick={() => selectScreen('chord')}>CHORD</button>
    <button type="button" id="tab-rhythm" class:active={screenState.active === 'rhythm'} onclick={() => selectScreen('rhythm')}>RHYTHM</button>
    <button type="button" id="tab-melody" class:active={screenState.active === 'melody'} onclick={() => selectScreen('melody')}>MELODY</button>
  </div>

  <FileSection />
  <LfoSection />
  <ProgramPanel />
  <Hint />
  <MidiLog />
</div>
