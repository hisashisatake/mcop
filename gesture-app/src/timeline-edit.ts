// タイムライン範囲削除（リップル削除）の純粋関数（RHYTHM/MELODY共通、依存ゼロ）。
// ルーラーで選択した[start,end)を取り除き、後続を前へ詰める（timeline.ts参照）。
// tests/timeline-edit.test.tsがnode --testで直接importするため、依存ゼロを保つ。

/** [start,end)を取り除き後続を前へ詰め、末尾を0で埋めて元の長さを保つ。 */
export function deleteRhythmRange(steps: number[], start: number, end: number): number[] {
  const removed = end - start;
  return [...steps.slice(0, start), ...steps.slice(end), ...new Array(removed).fill(0)];
}

/**
 * [start,end)より前の位置はそのまま、範囲内に収まる位置は`start`へ、範囲より後ろの
 * 位置は削除幅ぶん前へ詰める（[start,end)を取り除いた後の座標系への写像）。
 * ノートの始点・終点それぞれにこれを適用するだけで、完全内包(削除)・前方/後方はみ出し
 * (重なった分だけ短縮)・範囲を跨いで内包(重なった分だけ短縮)・範囲より後ろ(平行移動)の
 * 全ケースが一様に導ける。
 */
function mapPosAfterDelete(pulse: number, start: number, end: number): number {
  if (pulse <= start) return pulse;
  if (pulse >= end) return pulse - (end - start);
  return start;
}

/** 範囲内に完全に収まるノートは削除し、範囲と重なるノートは重なった分だけ短くする。
 * 範囲より後ろのノートは削除幅ぶん前へ詰める。 */
export function deleteMelodyRange<T extends { startStep: number; lengthSteps: number }>(notes: T[], start: number, end: number): T[] {
  const result: T[] = [];
  for (const n of notes) {
    const newStart = mapPosAfterDelete(n.startStep, start, end);
    const newEnd = mapPosAfterDelete(n.startStep + n.lengthSteps, start, end);
    const newLength = newEnd - newStart;
    if (newLength <= 0) continue;
    result.push({ ...n, startStep: newStart, lengthSteps: newLength });
  }
  return result;
}
