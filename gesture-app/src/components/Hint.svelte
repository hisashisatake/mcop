<script lang="ts">
  // 画面ごとのキーボード操作ヒント。screenState.activeを読むだけで自動的に切り替わる
  // （旧main.jsではonScreenChangeでinnerHTMLを書き換えていた）。
  import { screenState } from '../screens.svelte.ts';
</script>

<div id="hint">
  <div class="drawer-section-title">操作</div>
  {#if screenState.active === 'chord'}
    候補クリック: 発音して選択（セル上側ほど強く） <br />過去コードクリック: そこへ戻る<br />
    Ctrl+Z: 1つ戻す &nbsp;|&nbsp; Ctrl+Y: やり直す<br />
    Shift: 4和音 &nbsp;|&nbsp; Ctrl: sus/付加音 &nbsp;|&nbsp; Ctrl+Shift: aug/オルタード<br />
    Alt: 自動転回 一時切替<br />
    ↑/↓: 候補行数 &nbsp;|&nbsp; ←/→: 候補列数<br />
    ホイール: LFO Depth &nbsp;|&nbsp; V: Vibrato/Tremolo切替<br />
    C/B: LFO Rate down/up &nbsp;|&nbsp; E: 音色エディタ
  {:else if screenState.active === 'rhythm'}
    クリック: セルのベロシティを巡回（消音→通常→アクセント→弱）<br />
    再生/停止は左上メニューバーのアイコンから（再生にはTAPでテンポ確定が必要）<br />
    メトロノームON/OFFは下の音源パネルから<br />
    E: 音色エディタ
  {:else}
    空セルをクリック+ドラッグ: ノート作成（長さ調整）<br />
    ノートをクリック: 選択<br />
    選択中ノートをクリック: 音量を巡回（通常→アクセント→弱）<br />
    選択中ノートをドラッグ: 移動<br />
    ノート右端をドラッグ: 長さ変更<br />
    DEL: 選択中ノートを削除<br />
    ホイール: 音域スクロール　Shift+ホイール: 小節スクロール<br />
    再生/停止はリズムと共通（左上メニューバー）<br />
    E: 音色エディタ
  {/if}
</div>
