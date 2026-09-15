<script lang="ts">
  // 左下: 現在コードの大きな表示。chord-screen.tsのonChordChangeがhud-state.svelte.tsへ
  // 書き込み、ここはそれを読むだけ。
  import { hudState } from '../hud-state.svelte.ts';
</script>

<div id="hud">
  <div id="chord-display-row">
    <div id="chord-display">{hudState.degreeLabel}</div>
    <span id="chord-note-name">{hudState.noteName}</span>
  </div>
  <div id="chord-sub"><span id="chord-function">{hudState.func}</span></div>
</div>
