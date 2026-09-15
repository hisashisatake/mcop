// 独自プロジェクトファイル（.gap505）のOpen/Save/Save Asを呼び出す薄いラッパー。
// ファイルI/O自体（ダイアログ表示・読み書き）はRust側（src-tauri/src/project_file.rs）が
// 行い、ここではJSON文字列の受け渡しと「今どのパスを開いているか」の記憶だけを持つ。

import { serializeProject, deserializeAndApply } from './project-state.js';
import { resetUndoHistory } from './undo-manager.js';

// フォールバックでブラウザ単体でも開ける（Tauri外では常にキャンセル扱い）
const invoke = window.__TAURI__?.core?.invoke ?? (async () => null);

let currentPath = null;

export function currentProjectPath() {
  return currentPath;
}

/** @returns {Promise<boolean>} 実際に開けたか（キャンセル・失敗ならfalse） */
export async function openProject() {
  const result = await invoke('open_project');
  if (!result) return false;
  deserializeAndApply(result.json);
  resetUndoHistory();
  currentPath = result.path;
  return true;
}

/** 既知のパスがあれば上書き保存、無ければSave Asと同じ動作にフォールバックする。 */
export async function saveProject() {
  if (!currentPath) return saveProjectAs();
  const json = serializeProject();
  return invoke('save_project_to', { path: currentPath, json });
}

/** @returns {Promise<boolean>} 実際に保存したか（キャンセルならfalse） */
export async function saveProjectAs() {
  const json = serializeProject();
  const path = await invoke('save_project_as', { json });
  if (!path) return false;
  currentPath = path;
  return true;
}
