// ハンバーガーメニューのドロワー開閉状態。MenuBar（トグルボタン）・Drawer（表示）・
// screens.svelte.tsのonScreenChange購読（画面切り替え時に自動で閉じる）の3箇所から
// 触るため、Drawerコンポーネントローカルではなく共有モジュールに置く（段階C）。

import { onScreenChange } from './screens.svelte.ts';

export const drawerState: { open: boolean } = $state({ open: false });

export function toggleDrawer(): void {
  drawerState.open = !drawerState.open;
}

export function closeDrawer(): void {
  drawerState.open = false;
}

// 画面を切り替えたら、選んだ画面がすぐ見えるようドロワーを自動で閉じる。
onScreenChange(() => closeDrawer());
