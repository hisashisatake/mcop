<script lang="ts">
  // ルートコンポーネント。画面共通のパネル類（メニューバー・ドロワー・ステータス・HUD・
  // リサイズグリップ）とcanvas（3画面共通）を並べるだけで、ロジックは持たない。
  import CanvasStage from './CanvasStage.svelte';
  import MenuBar from './MenuBar.svelte';
  import Drawer from './Drawer.svelte';
  import StatusPanel from './StatusPanel.svelte';
  import Hud from './Hud.svelte';
  import ResizeGrip from './ResizeGrip.svelte';
</script>

<CanvasStage />
<MenuBar />
<Drawer />
<StatusPanel />
<Hud />
<ResizeGrip />
