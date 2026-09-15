// エントリポイント。App.svelteのマウントと、アクティブな画面を問わずグローバルに効く
// キー操作（Ctrl+Z/Y・E・DEL）の配線のみを持つ（段階C、画面ごとのDOM配線は各コンポーネントへ移した）。

import { mount } from 'svelte';
import App from './components/App.svelte';
import './app.css';
import { openEditor } from './midi.ts';
import { activeScreen } from './screens.svelte.ts';
import { deleteSelectedMelodyNote } from './melody-screen.ts';
import { undo, redo } from './undo-manager.ts';

mount(App, { target: document.body });

// Ctrl+Z/Ctrl+Yは統合Undo/Redo（undo-manager.ts）として、アクティブな画面を問わず
// グローバルに配線する。undo()/redo()はプロジェクト全体を復元するだけで、テンポ表示等の
// 追随はtempoState等の$stateが自動で行う（旧refreshTempoDisplay()の手動呼び出しは不要）。
window.addEventListener('keydown', async (e) => {
  if (e.key.toLowerCase() === 'e') {
    await openEditor();
  } else if ((e.key === 'Delete' || e.key === 'Backspace') && activeScreen() === 'melody') {
    deleteSelectedMelodyNote();
  } else if (e.key.toLowerCase() === 'z' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    if (e.shiftKey) redo();
    else undo();
  } else if (e.key.toLowerCase() === 'y' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    redo();
  }
});
