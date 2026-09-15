<script lang="ts">
  // 右下の手動リサイズグリップ。Tauri/WebView2側の既知の不具合（tauri-apps/tauri#9023、
  // 装飾ウィンドウの右端・下端(East/South)のドラッグがNCHITTESTを素通りしリサイズできない。
  // 左端・上端(West/North)は正常）の回避策として、startResizeDragging()を明示的に呼び出す
  // ハンドルを自前で描く。単一方向（'East'/'South'単体）は実機検証でstartResizeDragging自体が
  // 反応しないと判明したため使わず、'SouthEast'（斜め）のみ使用する——横だけ・縦だけ動かせば
  // 事実上その方向だけのリサイズとして機能する。
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { isTauri } from '@tauri-apps/api/core';

  async function onMouseDown(e: MouseEvent): Promise<void> {
    e.preventDefault();
    if (!isTauri()) return;
    await getCurrentWindow().startResizeDragging('SouthEast');
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div id="resize-grip" title="ドラッグで幅・高さをリサイズ（横だけ/縦だけの移動でその方向のみ変更）" onmousedown={onMouseDown}></div>
