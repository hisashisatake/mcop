// CHORD/RHYTHM/MELODYの編集を1本の時系列Undo/Redo履歴として扱う（統合型、Mementoパターン）。
// 各画面（chord-screen.js/rhythm-screen.js/melody-screen.js）は、値を書き換える直前に
// pushUndo()を呼ぶ。呼ばれるたびにプロジェクト全体（3画面ぶん）のスナップショットを
// スタックへ積むため、Ctrl+Zは「画面をまたいで直前の編集」を戻せる。
//
// 過去/未来コードクリックのような「再生位置の移動だけで値を書き換えない操作」は
// pushUndo()を呼ばない（編集操作ではないため、chord-screen.jsのコメント方針と同じ）。

import { captureProjectState, applyProjectState } from './project-state.js';

let undoStack = [];
let redoStack = [];

/** 値を書き換える直前に呼ぶ。redoStackは一般的なUndo/Redoの規約どおり破棄する。 */
export function pushUndo() {
  undoStack.push(captureProjectState());
  redoStack = [];
}

/** @returns {boolean} 実際に取り消しを行ったか（スタックが空なら何もせずfalse） */
export function undo() {
  if (undoStack.length === 0) return false;
  redoStack.push(captureProjectState());
  applyProjectState(undoStack.pop());
  return true;
}

/** @returns {boolean} 実際にやり直しを行ったか */
export function redo() {
  if (redoStack.length === 0) return false;
  undoStack.push(captureProjectState());
  applyProjectState(redoStack.pop());
  return true;
}

/** ファイルを新規に開いたときに呼ぶ。前のプロジェクトの編集履歴を引きずらないようにする。 */
export function resetUndoHistory() {
  undoStack = [];
  redoStack = [];
}
