// 画面切り替えの骨格（フェーズ3、段階Cで$state化）。コード/リズム/メロディの3画面を切り替える。
//
// 各画面モジュール（chord-screen.ts等）はcanvas/windowへ直接イベントリスナーを貼るため、
// 非アクティブな画面が裏で操作を拾ってしまわないよう、各ハンドラの先頭で`isActive(name)`を
// ガードとして使う（リスナー自体の付け外しはしない。既存のsetupChordScreen()の構造を
// 変えずに済むための最小変更）。
//
// activeはSvelteコンポーネントが直接読める$stateにした。onScreenChangeは引き続き、DOM表示
// 同期ではない副作用（画面切り替え時に鳴りっぱなしの音を止める等）のための購読口として残す
// （DOM表示の同期自体はコンポーネントがscreenState.activeを読むだけで自動的に行われる）。
import type { ScreenName } from './types.ts';

const SCREENS: ScreenName[] = ['chord', 'rhythm', 'melody'];

export const screenState: { active: ScreenName } = $state({ active: 'chord' });
const changeListeners = new Set<(next: ScreenName, previous: ScreenName) => void>();

export function isActive(name: ScreenName): boolean {
  return screenState.active === name;
}

export function activeScreen(): ScreenName {
  return screenState.active;
}

export function setActiveScreen(name: ScreenName): void {
  if (!SCREENS.includes(name) || name === screenState.active) return;
  const previous = screenState.active;
  screenState.active = name;
  changeListeners.forEach((fn) => fn(name, previous));
}

/** 画面が切り替わるたびに呼ばれる（DOM表示同期以外の副作用向け）。引数は(次の画面名, 直前の画面名)。 */
export function onScreenChange(fn: (next: ScreenName, previous: ScreenName) => void): void {
  changeListeners.add(fn);
}
