// リズム/メロディ/コード共通の再生/停止フラグ（メニューバーの▶/■ボタンの有効/無効表示用）。
// 段階Cで$state化。実際の再生開始/停止（standaloneへのMIDI Clock送出等）は
// midi.tsのsetSequencerRunning()が行う、ここは表示専用。
//
// steppingはコード画面のコマ送り再生（⏭）モード中かどうか。runningとは独立
// （コマ送り中は1拍鳴っては自動的にrunning=falseへ戻ることを繰り返すため、
// 「コマ送りモードに入っているか」自体は別のフラグで持つ。詳細はtransport.ts参照）。

export const sequencerState: { running: boolean; stepping: boolean } = $state({ running: false, stepping: false });
