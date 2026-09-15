// リズム/メロディ共通の再生/停止フラグ（メニューバーの▶/■ボタンの有効/無効表示用）。
// 段階Cで$state化。実際の再生開始/停止（standaloneへのMIDI Clock送出等）は
// midi.tsのsetSequencerRunning()が行う、ここは表示専用。

export const sequencerState: { running: boolean } = $state({ running: false });
