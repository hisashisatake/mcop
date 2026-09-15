// 独自プロジェクトファイル（.gap505）のOpen/Save/Save Asを呼び出す薄いラッパー。
// ファイルI/O自体（ダイアログ表示・読み書き）はRust側（src-tauri/src/project_file.rs）が
// 行い、ここではJSON文字列の受け渡しと「今どのパスを開いているか」の記憶だけを持つ。

import { serializeProject, deserializeAndApply } from './project-state.js';
import { pushUndo, resetUndoHistory } from './undo-manager.js';
import { getRows, setRows, STEPS as RHYTHM_STEPS_PER_BAR, DEFAULT_ROW_NOTES, DEFAULT_ROW_LABELS } from './rhythm-screen.js';
import { getNotes, setNotes, MIN_PITCH, MAX_PITCH, TOTAL_STEPS as MELODY_TOTAL_STEPS } from './melody-screen.js';
import { gm2DrumName } from './gm2-drums.ts';
import { getBpm, setBpm } from './tempo-state.js';
import { tapTempo } from './midi.js';
import { parseSmf, buildSmf, tempoMetaEvent, timeSignatureMetaEvent } from './smf.ts';
import {
  melodyNotesToEvents,
  midiEventsToMelodyNotes,
  rhythmRowsToEvents,
  midiEventsToRhythmRows,
  pickMelodyChannel,
  bpmFromEvents,
  microsecondsPerQuarterFromBpm,
} from './midi-convert.ts';

// フォールバックでブラウザ単体でも開ける（Tauri外では常にキャンセル扱い）
const invoke = window.__TAURI__?.core?.invoke ?? (async () => null);

// gesture-app独自のチャンネル割り当て（Rust側main.rs CHORD_CHANNEL等と対にはならない、
// standaloneへ送るMIDIチャンネルの規約。詳細はmemory
// `project_gesture_app_melody_screen_and_file_menu_plan.md`参照）。
const PPQ = 480;
const MELODY_CHANNEL = 1; // ch2（0-indexed）
const RHYTHM_CHANNEL = 9; // ch10（0-indexed）
const TICKS_PER_MELODY_STEP = PPQ / 2; // 8分音符単位
const TICKS_PER_RHYTHM_STEP = PPQ / 4; // 16分音符単位
const MELODY_BARS = 8;
const DEFAULT_EXPORT_BPM = 120; // タップテンポ未確定時のExport既定値

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

// ─────────────────────────────────────────────
// MIDI Import/Export（フェーズ3、標準MIDIファイル.mid）
//
// .gap505のOpen/Saveと違い、コードは対象外（リズム/メロディの2ch分のみ）。
// リズム/メロディは「丸ごと置換」だが、ファイル内に該当データが無い場合
// （例: メロディだけの.midをImportした際のリズム側）は既存の内容を残す
// （無関係な空データで上書きしてしまわないようにするため）。
// ─────────────────────────────────────────────

/**
 * @returns {Promise<boolean>} 実際に取り込めたか（キャンセル・パース失敗・
 *   有効なチャンネルが無かった場合はfalse）
 */
export async function importMidi() {
  const result = await invoke('import_midi');
  if (!result) return false;

  let division, events;
  try {
    ({ division, events } = parseSmf(new Uint8Array(result.bytes)));
  } catch {
    return false;
  }
  // divisionはppq前提（SMPTEはsmf.js側で例外化済み）。SMFのdivisionが480以外でも、
  // イベントのtickはそのdivision基準のまま渡ってくるため、量子化の単位もdivision基準で
  // スケールする必要がある（Exportは常にPPQ=480で書き出すが、Importは他ソフト製の
  // 任意divisionのファイルを受け付けるため）。
  const scale = division / PPQ;
  const ticksPerMelodyStep = TICKS_PER_MELODY_STEP * scale;
  const ticksPerRhythmStep = TICKS_PER_RHYTHM_STEP * scale;

  const melodyChannel = pickMelodyChannel(events);
  const newNotes = melodyChannel != null
    ? midiEventsToMelodyNotes(events, melodyChannel, ticksPerMelodyStep, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH)
    : null;
  // midiEventsToRhythmRowsは{note,steps}のみ返す（表示名の概念を持たない汎用ロジックのため）。
  // 既定12行に一致するノートは短縮ラベルを、それ以外はGM2名を付けてrows形式を完成させる
  // （行を動的に追加するというフェーズ3の設計方針、DEFAULT_ROW_NOTES参照）。
  const newRows = midiEventsToRhythmRows(events, RHYTHM_CHANNEL, ticksPerRhythmStep, RHYTHM_STEPS_PER_BAR).map((r) => {
    const idx = DEFAULT_ROW_NOTES.indexOf(r.note);
    return { note: r.note, steps: r.steps, label: idx >= 0 ? DEFAULT_ROW_LABELS[idx] : gm2DrumName(r.note) };
  });
  const bpm = bpmFromEvents(events);

  if (newNotes === null && newRows.length === 0 && bpm == null) return false;

  pushUndo();
  if (newNotes !== null) setNotes(newNotes);
  if (newRows.length > 0) setRows(newRows);
  if (bpm != null) {
    setBpm(bpm);
    tapTempo(bpm);
  }
  return true;
}

/** @returns {Promise<boolean>} 実際に保存したか（キャンセルならfalse） */
export async function exportMidi() {
  const bpm = getBpm() ?? DEFAULT_EXPORT_BPM;
  const melodyEvents = melodyNotesToEvents(getNotes(), MELODY_CHANNEL, TICKS_PER_MELODY_STEP);
  const rhythmEvents = rhythmRowsToEvents(getRows(), RHYTHM_CHANNEL, TICKS_PER_RHYTHM_STEP, RHYTHM_STEPS_PER_BAR, MELODY_BARS);
  const bytes = buildSmf({
    ppq: PPQ,
    tracks: [
      [tempoMetaEvent(0, microsecondsPerQuarterFromBpm(bpm)), timeSignatureMetaEvent(0)],
      melodyEvents,
      rhythmEvents,
    ],
  });
  const path = await invoke('export_midi', { bytes: Array.from(bytes) });
  return path != null;
}
