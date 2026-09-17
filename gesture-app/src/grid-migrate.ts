// v1/v2/v3プロジェクトファイル（旧16ステップ/小節のリズム・旧8ステップ/小節のメロディ・
// リズム=1小節限定だったv3）を、現行の内部形式（1パルス=1/96小節、RHYTHM/MELODY共通の
// 8小節=768パルスタイムライン）へ展開する移行関数群。grid-units.tsのみに依存する
// （project-state.tsの`patternV1ToRows`等、後からこのモジュールへ委譲する側から
// 逆に依存されるため、rhythm-screen.ts等の.svelte.ts/Tauri呼び出しを含む
// モジュールには依存しない）。

import { PULSES_PER_BAR, SEQUENCE_BARS } from './grid-units.ts';
import type { RhythmRow, MelodyNote } from './types.ts';

const OLD_RHYTHM_STEPS = 16;
const OLD_MELODY_STEPS_PER_BAR = 8;
const PULSES_PER_OLD_RHYTHM_STEP = PULSES_PER_BAR / OLD_RHYTHM_STEPS; // 6
const PULSES_PER_OLD_MELODY_STEP = PULSES_PER_BAR / OLD_MELODY_STEPS_PER_BAR; // 12

// rhythm-screen.tsのDEFAULT_ROW_NOTES/LABELSと同じ内容をここに複製する
// （依存ゼロを保つため。v1形式はこの固定12行を前提にした形式のまま凍結されている）。
const DEFAULT_ROW_NOTES = [49, 51, 46, 42, 39, 37, 38, 40, 48, 45, 41, 36];
const DEFAULT_ROW_LABELS = ['Crash', 'Ride', 'OpenHH', 'ClosedHH', 'Clap', 'Rim', 'Snare', 'E.Snare', 'HiTom', 'MidTom', 'LoTom', 'Kick'];

/**
 * 旧16ステップ/小節のstepsを、1パルス=1/96小節の96要素配列へ展開する
 * （新パルス位置 = 旧index×6、他のパルスは0のまま）。
 *
 * 6パルス全体を同じレベルで埋めない理由: リズムの発音判定は1パルスごとに「非0なら
 * 発音」を行う（毎パルス走査、clock_loop参照）。6パルス全部を埋めると1個の16分音符が
 * 6回連続で再トリガーされてしまうため、旧データの各ヒットは必ず単一パルスとして
 * 配置する（描画側は`cellLevel()`が範囲内の最大値を読むため、単一パルスでも
 * 従来と同じ見た目になる）。
 */
export function expandRhythmSteps16To96(steps: number[]): number[] {
  const result = new Array(PULSES_PER_BAR).fill(0);
  for (let i = 0; i < OLD_RHYTHM_STEPS; i++) {
    const level = steps[i] ?? 0;
    if (level !== 0) result[i * PULSES_PER_OLD_RHYTHM_STEP] = level;
  }
  return result;
}

/** v2(8ステップ/小節)のMelodyNote[]を、startStep/lengthStepsを×12したパルス単位へ展開する。 */
export function expandMelodyNotesV2(notes: MelodyNote[]): MelodyNote[] {
  return notes.map((n) => ({
    ...n,
    startStep: n.startStep * PULSES_PER_OLD_MELODY_STEP,
    lengthSteps: n.lengthSteps * PULSES_PER_OLD_MELODY_STEP,
  }));
}

/**
 * 1小節分（96パルス）のリズムstepsを、RHYTHM/MELODY共通の8小節タイムライン（768パルス）へ
 * そのまま繰り返して展開する（v3までのリズムは1小節ループだったため、毎小節同じ内容を
 * 繰り返せば聞こえ方が変わらない）。
 */
export function repeatRhythmBarToSequence(steps96: number[]): number[] {
  const result: number[] = [];
  for (let bar = 0; bar < SEQUENCE_BARS; bar++) {
    for (let i = 0; i < PULSES_PER_BAR; i++) result.push(steps96[i] ?? 0);
  }
  return result;
}

/** v1(.gap505)の12×16固定パターンを、現行のRhythmRow[]（768パルス/行）へ直接展開する。 */
export function expandPatternV1(pattern: number[][]): RhythmRow[] {
  return DEFAULT_ROW_NOTES.map((note, i) => ({
    note,
    label: DEFAULT_ROW_LABELS[i],
    steps: repeatRhythmBarToSequence(expandRhythmSteps16To96(pattern[i] ?? [])),
  }));
}
