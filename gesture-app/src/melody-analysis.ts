// メロディ画面に登録されたノートを解析し、コード画面の候補絞り込み（chord-flow.tsの
// 加点方式スコアリング）に使うピッチクラス重みテーブルを作る。
//
// 解析は「小節の四分割（4分音符）」単位で行う。8小節×4分音符=32スロットに区切り、
// 各スロットへ重なっているメロディノートのピッチクラスを集計する。ノートがそのスロットで
// 鳴り始めた（オンセット）場合はフル重み、前のスロットから鳴り続けているだけ（ロングトーン
// の跨り）の場合は重みを下げる（ユーザー確認済み: 「そのスロットで新しく選ばれた音」ほど
// コード選びへの影響を強くしたいが、鳴りっぱなしの音まで毎スロット同じ強さで効くと
// 実際にはあまり動いていないメロディでも絞り込みが強く効きすぎるため）。
//
// 解析はメロディノートを編集するたびに自動実行するのではなく、明示的なタイミング
// （.gap505/.midのOpen/Save、メニューバーの更新アイコン）でのみ行う（ユーザー確認済み、
// project-file.ts/MenuBar.svelte参照）。chord-screen.tsは再生位置（timeline.tsのdisplayPulse）
// からスロットを逆算して毎フレーム参照するため、再生中はスロットが切り替わるたびに
// コード候補がリアルタイムで絞り込まれる。

import { PULSES_PER_BEAT, SEQUENCE_TOTAL_PULSES } from './grid-units.ts';
import type { MelodyNote } from './types.ts';

/** 1スロット=4分音符(24パルス)。 */
export const SLOT_PULSES = PULSES_PER_BEAT;
/** 8小節×4拍=32スロット。 */
export const SLOT_COUNT = SEQUENCE_TOTAL_PULSES / SLOT_PULSES;

/** そのスロットでノートが鳴り始めた（オンセット）場合の重み。 */
const ONSET_WEIGHT = 1.0;
/** 前のスロットから鳴り続けている（ロングトーンの跨り）場合の重み。 */
const SUSTAIN_WEIGHT = 0.5;

const mod12 = (n: number): number => ((n % 12) + 12) % 12;

// slot index -> (pitch class -> weight)。空スロットは空Mapのまま（絞り込み無し）。
let slotPitchWeights: Map<number, number>[] = Array.from({ length: SLOT_COUNT }, () => new Map());

/** 解析結果のバージョン。呼び出し側（chord-screen.tsのキャッシュキー）が変化検知に使う。 */
let version = 0;

/**
 * メロディノート配列から、4分音符スロットごとのピッチクラス重みテーブルを作る
 * （DOM・Tauri・.svelte.tsのいずれにも触れない純粋関数、chord-flow.ts/theory.tsと同じ方針。
 * node --testから直接検証できるよう、状態を持つ`analyzeMelodyForChordHints`から分離してある）。
 */
export function computeMelodySlotWeights(notes: MelodyNote[]): Map<number, number>[] {
  const table: Map<number, number>[] = Array.from({ length: SLOT_COUNT }, () => new Map());
  for (const note of notes) {
    const start = Math.max(0, note.startStep);
    const end = start + Math.max(1, note.lengthSteps); // 排他的終端
    const firstSlot = Math.floor(start / SLOT_PULSES);
    const lastSlot = Math.floor((end - 1) / SLOT_PULSES);
    const pc = mod12(note.pitch);
    for (let slot = firstSlot; slot <= lastSlot && slot < SLOT_COUNT; slot++) {
      const weight = slot === firstSlot ? ONSET_WEIGHT : SUSTAIN_WEIGHT;
      const map = table[slot];
      map.set(pc, Math.max(map.get(pc) ?? 0, weight));
    }
  }
  return table;
}

/**
 * メロディノートを再解析する。.gap505/.midのOpen/Save、メニューバーの更新アイコンから、
 * 呼び出し側が`melody-screen.ts`の`getNotes()`を渡して呼ぶ（ユーザー確認済み、リアルタイム
 * 自動解析はしない）。このモジュール自体は`melody-screen.ts`（`.svelte.ts`経由でSvelteの
 * コンパイラマクロに依存する）をimportしない——node --testから直接検証できるようにするため
 * （chord-flow.ts/theory.tsと同じ「DOM・Tauri・Svelteに触れない純粋関数」の方針）。
 */
export function analyzeMelodyForChordHints(notes: MelodyNote[]): void {
  slotPitchWeights = computeMelodySlotWeights(notes);
  version++;
}

/** 解析結果のバージョン番号（再解析のたびに増える）。 */
export function melodyAnalysisVersion(): number {
  return version;
}

/** pulse位置（ループ長でラップする）が属する4分音符スロット番号（0〜31）。 */
export function melodySlotAtPulse(pulse: number): number {
  const wrapped = ((pulse % SEQUENCE_TOTAL_PULSES) + SEQUENCE_TOTAL_PULSES) % SEQUENCE_TOTAL_PULSES;
  return Math.floor(wrapped / SLOT_PULSES);
}

/** 指定スロットのピッチクラス重み（無ければ空Map＝絞り込み無し）。 */
export function melodyWeightsForSlot(slot: number): Map<number, number> {
  return slotPitchWeights[slot] ?? new Map();
}
