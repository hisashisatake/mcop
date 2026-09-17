// エントリポイント。App.svelteのマウントと、アクティブな画面を問わずグローバルに効く
// キー操作（Ctrl+Z/Y・E・DEL）の配線のみを持つ（段階C、画面ごとのDOM配線は各コンポーネントへ移した）。

import { mount } from 'svelte';
import App from './components/App.svelte';
import './app.css';
import { openEditor } from './midi.ts';
import { activeScreen } from './screens.svelte.ts';
import { activeProgramChannel } from './program-state.svelte.ts';
import { deleteSelectedMelodyNote, getNotes, setNotes } from './melody-screen.ts';
import { getRows, setRows } from './rhythm-screen.ts';
import { currentSelection, clearSelection, movePlayhead } from './timeline.ts';
import { deleteRhythmRange, deleteMelodyRange } from './timeline-edit.ts';
import { undo, redo, pushUndo } from './undo-manager.ts';

mount(App, { target: document.body });

/** ルーラーの選択範囲をリズム・メロディ両方から同時に取り除く（前に詰めるリップル削除）。 */
function deleteTimelineRange(start: number, end: number): void {
  pushUndo();
  setRows(getRows().map((r) => ({ ...r, steps: deleteRhythmRange(r.steps, start, end) })));
  setNotes(deleteMelodyRange(getNotes(), start, end));
  clearSelection();
  movePlayhead(start);
}

// Ctrl+Z/Ctrl+Yは統合Undo/Redo（undo-manager.ts）として、アクティブな画面を問わず
// グローバルに配線する。undo()/redo()はプロジェクト全体を復元するだけで、テンポ表示等の
// 追随はtempoState等の$stateが自動で行う（旧refreshTempoDisplay()の手動呼び出しは不要）。
window.addEventListener('keydown', async (e) => {
  if (e.key.toLowerCase() === 'e') {
    await openEditor(activeProgramChannel());
  } else if (e.key === 'Delete' || e.key === 'Backspace') {
    const screen = activeScreen();
    const sel = screen === 'rhythm' || screen === 'melody' ? currentSelection() : null;
    if (sel) {
      deleteTimelineRange(sel.start, sel.end);
    } else if (screen === 'melody') {
      deleteSelectedMelodyNote();
    }
  } else if (e.key.toLowerCase() === 'z' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    if (e.shiftKey) redo();
    else undo();
  } else if (e.key.toLowerCase() === 'y' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    redo();
  }
});
