// 画面切り替えの骨格（フェーズ3）。コード/リズム/メロディの3画面を切り替える。
//
// 各画面モジュール（chord-screen.js等）はcanvas/windowへ直接イベントリスナーを貼るため、
// 非アクティブな画面が裏で操作を拾ってしまわないよう、各ハンドラの先頭で`isActive(name)`を
// ガードとして使う（リスナー自体の付け外しはしない。既存のsetupChordScreen()の構造を
// 変えずに済むための最小変更）。

const SCREENS = ['chord', 'rhythm', 'melody'];

let active = 'chord';
const changeListeners = new Set();

export function isActive(name) {
  return active === name;
}

export function activeScreen() {
  return active;
}

export function setActiveScreen(name) {
  if (!SCREENS.includes(name) || name === active) return;
  const previous = active;
  active = name;
  changeListeners.forEach((fn) => fn(name, previous));
}

/** 画面が切り替わるたびに呼ばれる。引数は(次の画面名, 直前の画面名)。 */
export function onScreenChange(fn) {
  changeListeners.add(fn);
}

/** 画面切り替えタブを配線する。`buttons`は{chord, rhythm, melody}のボタン要素。 */
export function bindScreenTabs(buttons) {
  for (const name of SCREENS) {
    const btn = buttons[name];
    if (!btn) continue;
    btn.addEventListener('click', () => setActiveScreen(name));
  }
  const syncActiveClass = () => {
    for (const name of SCREENS) {
      buttons[name]?.classList.toggle('active', name === active);
    }
  };
  onScreenChange(syncActiveClass);
  syncActiveClass();
}
