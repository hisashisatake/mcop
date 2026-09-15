// タップテンポで確定したBPMを、複数モジュール（main.ts/project-state.ts）から参照できる
// ようにするための小さな置き場。main.tsのMenuBarコンポーネント（タップテンポ操作）が
// 確定値をここへ書き、project-state.tsはプロジェクトの保存/復元（Open/Save/Undo/Redo）で
// この値を読み書きする。$state化（段階C）によりテンポ表示は自動で追随し、
// 旧refreshTempoDisplay()のような手動DOM同期呼び出しは不要になった。

export const tempoState: { bpm: number | null } = $state({ bpm: null }); // nullは「まだタップされておらずBPM未確定」を表す

export function getBpm(): number | null {
  return tempoState.bpm;
}

export function setBpm(bpm: number | null): void {
  tempoState.bpm = bpm;
}
