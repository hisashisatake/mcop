// コード画面のコマ送り再生（ステップ・トランスポート）を含む、3画面共通のトランスポート
// （▶/■/⏭）の状態遷移をここへ集約する。実際の再生/停止/一時停止はRust側`clock_loop`が
// 権威を持つ（コマ送り中の自動一時停止はRust発、詳細はsrc-tauri/src/midi_out.rsの
// STEP_MODE_ACTIVE/SUPPRESS_JUMP_ON_RESUME参照）。ここは(1)ボタン押下→Tauriコマンド発行、
// (2)Rustからの一時停止通知（sequencer-paused）→sequencerState反映、の橋渡しのみ持つ。
//
// 停止（■）は常に「このトランスポートセッションを開始した位置」へ戻る。コマ送りで何拍
// 進めていても、Rust側PLAYBACK_START_PULSEはコマ送りの各ステップでは一切書き換えないため、
// この「セッション開始位置へ戻る」は追加のJS側状態を持たずRust側の挙動として自然に実現される
// （ユーザー確認済み: コマ送り中の停止は「コマ送りを始めた位置」へ戻る）。

import { sequencerState } from './sequencer-state.svelte.ts';
import { setSequencerRunning, enterStepMode as invokeEnterStepMode, stepAdvance as invokeStepAdvance, onSequencerPaused } from './midi.ts';
import { resetRhythmCursor } from './rhythm-screen.ts';
import { resetMelodyCursor } from './melody-screen.ts';

/** ▶ボタン（3画面共通）。コマ送り中でも通常再生中でも、コマ送りを抜けて通常再生する
 * （コマ送りの一時停止中に押した場合は今の位置から続けて通常再生になる、
 * Rust側`set_sequencer_running`のdoc参照）。 */
export function play(): void {
  sequencerState.running = true;
  sequencerState.stepping = false;
  setSequencerRunning(true);
}

/** ■ボタン（3画面共通）。コマ送り・通常再生とも抜け、このセッションを開始した位置へ戻る。 */
export function stop(): void {
  sequencerState.running = false;
  sequencerState.stepping = false;
  resetRhythmCursor();
  resetMelodyCursor();
  setSequencerRunning(false);
}

/** ⏭ボタン（コード画面専用）。コマ送りモードへ入る。既に通常再生中なら次の拍の頭まで
 * 進んで自動的に一時停止する（`sequencer-paused`購読側でsequencerState.running=falseへ
 * 反映される）。停止中なら位置はそのまま、`stepAdvance()`が呼ばれるまで待機する。 */
export function enterStepMode(): void {
  sequencerState.stepping = true;
  invokeEnterStepMode();
}

/** コマ送り中、候補コード・過去/未来コード・現在スロットのクリックやSpaceで1拍分進める。
 * コマ送りモードでなければ何もしない（通常再生・完全停止中はコード画面の発音操作を
 * トランスポートに影響させない）。既に再生中（前回の1拍がまだ鳴っている最中）の場合は
 * Rust側が無視する（＝どのみち次の拍の頭で自動的に一時停止する）。 */
export function stepAdvance(): void {
  if (!sequencerState.stepping) return;
  sequencerState.running = true;
  invokeStepAdvance();
}

// `sequencer-paused`は起動時に一度だけ購読する（複数箇所から呼ばれても二重登録しない）。
onSequencerPaused(() => {
  sequencerState.running = false;
});
