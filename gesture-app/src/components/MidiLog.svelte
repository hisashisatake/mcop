<script lang="ts">
  // 直近のMIDI送信内容（実機確認用のデバッグ表示）。追記のたびに末尾までスクロールする。
  import { midiLogState } from '../midi-log.svelte.ts';

  let preEl: HTMLPreElement;

  $effect(() => {
    void midiLogState.lines.length; // 依存として読む
    if (preEl) preEl.scrollTop = preEl.scrollHeight;
  });
</script>

<div id="midi-log-panel">
  <div class="midi-log-title">MIDI LOG</div>
  <pre id="midi-log" bind:this={preEl}>{midiLogState.lines.join('\n')}</pre>
</div>
