// gesture-app全体で共有する型定義。旧JS版のJSDocコメントで表現していた形をTSの型として起こす。

export type Mode = 'major' | 'minor';

export type ChordFamily = 'maj' | 'dom' | 'min' | 'halfdim' | 'dim' | 'sus' | 'six' | 'msix' | 'aug' | 'mmaj';

/** normalizeFamily()が返す、ダイアトニック判定・機能判定で使う基本5種。 */
export type NormalizedFamily = 'maj' | 'dom' | 'min' | 'halfdim' | 'dim';

/** chords.jsのレイヤー定義1件分。 */
export interface ChordTypeDef {
  row: number;
  suffix: string;
  intervals: number[];
  family: ChordFamily;
}

export type LayerName = 'normal' | 'shift' | 'ctrl' | 'ctrlShift';

export type ScreenName = 'chord' | 'rhythm' | 'melody';

export interface Mods {
  shiftHeld?: boolean;
  ctrlHeld?: boolean;
}

/** chordFromSemitone()が返すコード。 */
export interface Chord {
  name: string;
  suffix: string;
  rootMidi: number;
  rootPc: number;
  family: ChordFamily;
  intervals: number[];
}

/** theory.js/voicing.jsが受け取るコード形（rootMidiを持たない、rootPc基準のみ）。Chordはこれを満たす。 */
export interface ChordLike {
  rootPc: number;
  family: ChordFamily;
  intervals: number[];
}

/** {tonicMidi, mode}形式のキー（chord-flow.js/chord-screen.jsのhistory系）。 */
export interface Key {
  tonicMidi: number;
  mode: Mode;
}

/** {tonicPc, mode}形式のキー（theory.js/chord-flow.jsのスコアリング系）。 */
export interface KeyObj {
  tonicPc: number;
  mode: Mode;
}

export type ProgressionCategory = 'GREEN' | 'YELLOW' | null;

export interface ClassifyResult {
  score: number;
  category: ProgressionCategory;
}

export type ChordFunctionKind = 'T' | 'SD' | 'D' | 'P' | null;

export interface ChordFunctionResult {
  kind: ChordFunctionKind;
  resolvesTo: string | null;
}

export interface PendingPivot {
  keys: KeyObj[];
}

export type RelatedKeyRole = 'dominant' | 'subdominant' | 'relative' | 'parallel';

export interface RelatedKey extends KeyObj {
  role: RelatedKeyRole;
}

/** progressions.jsのテンプレート定義1ステップ分。 */
export interface ProgressionStep {
  degree: number;
  families?: ChordFamily[];
  suffixes?: string[];
}

export interface Progression {
  id: string;
  name: string;
  mode: Mode;
  cyclic: boolean;
  steps: ProgressionStep[];
  minMatch?: number;
}

/** matchProgressions()が受け取る、直近1手の要約。 */
export interface PlayedChordSummary {
  degree: number;
  normFamily: NormalizedFamily | null;
  suffix: string;
}

export interface ProgressionMatch {
  id: string;
  name: string;
  matchedLength: number;
  position: number;
  total: number;
  next: ProgressionStep;
}

export interface ProgressionLegendEntry {
  badgeIndex: number;
  name: string;
  position: number;
  total: number;
  targetSuffix: string;
  currentLayerName: LayerName;
}

/** chord-flow.jsの履歴1エントリ。 */
export interface HistoryEntry {
  chord: Chord;
  key: Key;
  pendingPivot: PendingPivot | null;
  velocity: number;
  voicing: number[];
}

export interface ChordHistory {
  entries: HistoryEntry[];
  cursor: number;
  initialKey: Key;
}

export interface CandidateCell {
  chord: Chord;
  score: number;
  category: ProgressionCategory;
  isPivot: boolean;
  confirmsPivot: boolean;
}

export interface ProgressionHint {
  id: string;
  name: string;
  badgeIndex: number;
  position: number;
  total: number;
}

export interface CandidateGridCell extends CandidateCell {
  col: number;
  row: number;
  progressionHints: ProgressionHint[];
}

export interface PastSlotGeom {
  index: number;
  x: number;
  y: number;
  size: number;
}

// ─────────────────────────────────────────────
// SMF (smf.ts)
// ─────────────────────────────────────────────

export interface SmfNoteOnEvent {
  tick: number;
  kind: 'noteOn';
  channel: number;
  note: number;
  velocity: number;
}
export interface SmfNoteOffEvent {
  tick: number;
  kind: 'noteOff';
  channel: number;
  note: number;
}
export interface SmfProgramEvent {
  tick: number;
  kind: 'program';
  channel: number;
  program: number;
}
export interface SmfControlChangeEvent {
  tick: number;
  kind: 'controlChange';
  channel: number;
  cc: number;
  value: number;
}
export interface SmfPitchBendEvent {
  tick: number;
  kind: 'pitchBend';
  channel: number;
  value: number;
}
export interface SmfTempoEvent {
  tick: number;
  kind: 'tempo';
  microsecondsPerQuarter: number;
}

export type SmfEvent =
  | SmfNoteOnEvent
  | SmfNoteOffEvent
  | SmfProgramEvent
  | SmfControlChangeEvent
  | SmfPitchBendEvent
  | SmfTempoEvent;

export interface SmfParseResult {
  division: number;
  events: SmfEvent[];
}

/** buildSmf()向けの生成専用イベント形（tick+バイト列のみ、種別を持たない）。 */
export interface SmfRawEvent {
  tick: number;
  bytes: number[];
}

// ─────────────────────────────────────────────
// リズム/メロディ（rhythm-screen.ts/melody-screen.ts/project-state.ts）
// ─────────────────────────────────────────────

export interface RhythmRow {
  note: number;
  label: string;
  steps: number[];
}

/** midi-convert.jsのImport/Export向け（labelを持たない）。 */
export interface RhythmRowData {
  note: number;
  steps: number[];
}

export interface MelodyNote {
  id: number;
  startStep: number;
  lengthSteps: number;
  pitch: number;
  level: number;
}

// ─────────────────────────────────────────────
// プロジェクトファイル（project-state.ts）
// ─────────────────────────────────────────────

// v2→v3: リズム/メロディの内部単位を「1パルス=1/96小節」へ統一した
// （グリッド解像度細分化、RhythmRow.steps/MelodyNote.startStep・lengthStepsの意味が
// 変わるだけで、型の形自体はv2と同じ）。
// v3→v4: RhythmRow.stepsの長さをMELODY画面と共通の8小節=768パルスへ拡張した
// （タイムライン共通化、旧v3は1小節=96パルスのみだった。型の形自体はv3と同じ）。

export interface ProjectStateV4 {
  version: 4;
  bpm: number | null;
  chord: ChordHistory;
  rhythm: { rows: RhythmRow[] };
  melody: { notes: MelodyNote[] };
}

export interface ProjectStateV3 {
  version: 3;
  bpm: number | null;
  chord: ChordHistory;
  rhythm: { rows: RhythmRow[] };
  melody: { notes: MelodyNote[] };
}

export interface ProjectStateV2 {
  version: 2;
  bpm: number | null;
  chord: ChordHistory;
  rhythm: { rows: RhythmRow[] };
  melody: { notes: MelodyNote[] };
}

export interface ProjectStateV1 {
  version: 1;
  bpm: number | null;
  chord: ChordHistory;
  rhythm: { pattern: number[][] };
  melody: { notes: MelodyNote[] };
}

export type ProjectState = ProjectStateV1 | ProjectStateV2 | ProjectStateV3 | ProjectStateV4;

// ─────────────────────────────────────────────
// MIDI（midi.ts）
// ─────────────────────────────────────────────

export type ProgramInfoStatus = 'disconnected' | 'resolved' | 'not_found' | 'rhythm' | 'editing';

export interface ProgramInfo {
  bank: number;
  program: number;
  name?: string;
  status: ProgramInfoStatus;
}

/** op505_set_performance_lfoコマンドへ渡す引数（performance-lfo.ts参照）。 */
export interface PerformanceLfoArgs {
  channel: number;
  rate: number;
  delay: number;
  destination: number;
  cc77: number;
  cc1: number;
  modDepthRange: number;
}
