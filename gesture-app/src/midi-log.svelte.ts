// 直近のMIDI送信内容を画面に表示する開発支援ロガー。
// 「本当に何が送られているか」を実機確認時に目視できるようにするためのもの。
// midi.tsの送信関数から呼ぶことで、今後シーケンサー/録音機能を追加しても
// 同じ経路を通る限り自動的に可視化される。
//
// 段階Cで$state化。MidiLogコンポーネントがmidiLogState.linesを直接読むため、
// 旧setupMidiLog()/render()のような要素参照・手動DOM同期は不要。

const MAX_ENTRIES = 16;

export const midiLogState: { lines: string[] } = $state({ lines: [] });

export function pushLog(text: string): void {
  const now = new Date();
  const time = now.toLocaleTimeString('ja-JP', { hour12: false }) + '.' + String(now.getMilliseconds()).padStart(3, '0');
  midiLogState.lines.push(`${time}  ${text}`);
  if (midiLogState.lines.length > MAX_ENTRIES) midiLogState.lines.shift();
}
