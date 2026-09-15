// CHORD/RHYTHM/MELODYの編集を1本の時系列Undo/Redo履歴として扱う（統合型、Mementoパターン）。
// 各画面（chord-screen.js/rhythm-screen.js/melody-screen.js）は、値を書き換える直前に
// pushUndo()を呼ぶ。呼ばれるたびにプロジェクト全体（3画面ぶん）のスナップショットを
// スタックへ積むため、Ctrl+Zは「画面をまたいで直前の編集」を戻せる。
//
// 過去/未来コードクリックのような「再生位置の移動だけで値を書き換えない操作」は
// pushUndo()を呼ばない（編集操作ではないため、chord-screen.jsのコメント方針と同じ）。

import { captureProjectState, applyProjectState } from './project-state.ts';
import type { ProjectState } from './types.ts';

let undoStack: ProjectState[] = [];
let redoStack: ProjectState[] = [];

/** 値を書き換える直前に呼ぶ。redoStackは一般的なUndo/Redoの規約どおり破棄する。 */
export function pushUndo(): void {
  undoStack.push(captureProjectState());
  redoStack = [];
}

/** @returns 実際に取り消しを行ったか（スタックが空なら何もせずfalse） */
export function undo(): boolean {
  if (undoStack.length === 0) return false;
  redoStack.push(captureProjectState());
  applyProjectState(undoStack.pop()!);
  return true;
}

/** @returns 実際にやり直しを行ったか */
export function redo(): boolean {
  if (redoStack.length === 0) return false;
  undoStack.push(captureProjectState());
  applyProjectState(redoStack.pop()!);
  return true;
}

/** ファイルを新規に開いたときに呼ぶ。前のプロジェクトの編集履歴を引きずらないようにする。 */
export function resetUndoHistory(): void {
  undoStack = [];
  redoStack = [];
}
