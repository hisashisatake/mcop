// タップテンポで確定したBPMを、複数モジュール（main.js/project-state.js）から参照できる
// ようにするための小さな置き場。main.jsの`tapTempo`ボタンハンドラが確定値をここへ書き、
// project-state.jsはプロジェクトの保存/復元（Open/Save/Undo/Redo）でこの値を読み書きする。

let currentBpm: number | null = null; // nullは「まだタップされておらずBPM未確定」を表す

export function getBpm(): number | null {
  return currentBpm;
}

export function setBpm(bpm: number | null): void {
  currentBpm = bpm;
}
