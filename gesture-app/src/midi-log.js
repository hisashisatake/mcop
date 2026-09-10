// 直近のMIDI送信内容を画面に表示する開発支援ロガー。
// 「本当に何が送られているか」を実機確認時に目視できるようにするためのもの。
// midi.jsの送信関数から呼ぶことで、今後シーケンサー/録音機能を追加しても
// 同じ経路を通る限り自動的に可視化される。

const MAX_ENTRIES = 16;
const entries = [];
let listEl = null;

export function setupMidiLog(el) {
  listEl = el;
}

export function pushLog(text) {
  const now = new Date();
  const time = now.toLocaleTimeString('ja-JP', { hour12: false }) + '.' + String(now.getMilliseconds()).padStart(3, '0');
  entries.push(`${time}  ${text}`);
  if (entries.length > MAX_ENTRIES) entries.shift();
  render();
}

function render() {
  if (!listEl) return;
  listEl.textContent = entries.join('\n');
  listEl.scrollTop = listEl.scrollHeight;
}
