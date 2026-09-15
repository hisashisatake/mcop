// 左下HUD（コード名表示）の表示用状態。chord-screen.tsのonChordChangeコールバックから
// 書き込まれる（発音・選択・Undo/Redo・ファイル読込のたびに呼ばれる）。段階Cで$state化し、
// main.tsが直接textContentを書いていた箇所をHudコンポーネントの読み取りへ置き換えた。

export const hudState: { degreeLabel: string; func: string; noteName: string } = $state({
  degreeLabel: '—',
  func: '',
  noteName: '',
});

export function setHudChordInfo(info: { degreeLabel: string; func: string; noteName: string } | null): void {
  hudState.degreeLabel = info?.degreeLabel ?? '—';
  hudState.func = info?.func ?? '';
  hudState.noteName = info?.noteName ?? '';
}
