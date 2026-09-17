import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deleteRhythmRange, deleteMelodyRange } from '../src/timeline-edit.ts';

test('deleteRhythmRangeは[start,end)を取り除き後続を前へ詰め、末尾を0で埋める', () => {
  const steps = [1, 2, 3, 4, 5, 6, 7, 8];
  const result = deleteRhythmRange(steps, 2, 4); // [3,4)を除去
  assert.deepEqual(result, [1, 2, 5, 6, 7, 8, 0, 0]);
  assert.equal(result.length, steps.length);
});

test('deleteMelodyRangeは範囲内に完全に収まるノートを削除する', () => {
  const notes = [{ id: 1, startStep: 10, lengthSteps: 5, pitch: 60, level: 1 }];
  const result = deleteMelodyRange(notes, 0, 20);
  assert.deepEqual(result, []);
});

test('deleteMelodyRangeは範囲の前方にはみ出すノートを重なった分だけ短くする', () => {
  // ノート: 5〜15、削除範囲: [10,20) → 生き残るのは5〜10
  const notes = [{ id: 1, startStep: 5, lengthSteps: 10, pitch: 60, level: 1 }];
  const result = deleteMelodyRange(notes, 10, 20);
  assert.deepEqual(result, [{ id: 1, startStep: 5, lengthSteps: 5, pitch: 60, level: 1 }]);
});

test('deleteMelodyRangeは範囲の後方にはみ出すノートを重なった分だけ短くし、範囲先頭へ詰める', () => {
  // ノート: 15〜25、削除範囲: [10,20) → 生き残るのは20〜25が10〜15へ詰まる
  const notes = [{ id: 1, startStep: 15, lengthSteps: 10, pitch: 60, level: 1 }];
  const result = deleteMelodyRange(notes, 10, 20);
  assert.deepEqual(result, [{ id: 1, startStep: 10, lengthSteps: 5, pitch: 60, level: 1 }]);
});

test('deleteMelodyRangeは範囲を跨いで内包するノートを重なった分だけ短くする', () => {
  // ノート: 0〜30、削除範囲: [10,20) → 前後がつながり0〜20(長さ20)になる
  const notes = [{ id: 1, startStep: 0, lengthSteps: 30, pitch: 60, level: 1 }];
  const result = deleteMelodyRange(notes, 10, 20);
  assert.deepEqual(result, [{ id: 1, startStep: 0, lengthSteps: 20, pitch: 60, level: 1 }]);
});

test('deleteMelodyRangeは範囲より後ろのノートを削除幅ぶん前へ詰める', () => {
  const notes = [{ id: 1, startStep: 30, lengthSteps: 10, pitch: 60, level: 1 }];
  const result = deleteMelodyRange(notes, 10, 20); // 削除幅10
  assert.deepEqual(result, [{ id: 1, startStep: 20, lengthSteps: 10, pitch: 60, level: 1 }]);
});

test('deleteMelodyRangeは範囲より前のノートに触れない', () => {
  const notes = [{ id: 1, startStep: 0, lengthSteps: 5, pitch: 60, level: 1 }];
  const result = deleteMelodyRange(notes, 10, 20);
  assert.deepEqual(result, notes);
});
