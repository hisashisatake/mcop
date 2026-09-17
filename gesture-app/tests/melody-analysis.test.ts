import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeMelodySlotWeights, melodySlotAtPulse, SLOT_PULSES, SLOT_COUNT } from '../src/melody-analysis.ts';
import type { MelodyNote } from '../src/types.ts';

test('melodySlotAtPulse: 4分音符(24パルス)ごとにスロットが変わる', () => {
  assert.equal(melodySlotAtPulse(0), 0);
  assert.equal(melodySlotAtPulse(23), 0);
  assert.equal(melodySlotAtPulse(24), 1);
  assert.equal(melodySlotAtPulse(SLOT_PULSES * SLOT_COUNT - 1), SLOT_COUNT - 1);
});

test('melodySlotAtPulse: ループ長(768パルス)でラップする', () => {
  const total = SLOT_PULSES * SLOT_COUNT;
  assert.equal(melodySlotAtPulse(total), 0);
  assert.equal(melodySlotAtPulse(total + 24), 1);
});

test('computeMelodySlotWeights: オンセットスロットはフル重み、跨りスロットは減衰重み', () => {
  const notes: MelodyNote[] = [
    { id: 1, startStep: 0, lengthSteps: 48, pitch: 60, level: 1 }, // C4。スロット0で開始しスロット1へロングトーンで跨る
  ];
  const table = computeMelodySlotWeights(notes);
  assert.equal(table[0].get(0), 1.0, 'オンセットスロットはフル重み');
  assert.equal(table[1].get(0), 0.5, '跨りスロットは減衰重み');
  assert.equal(table[2].size, 0, '音が無いスロットは空のはず');
});

test('computeMelodySlotWeights: 同じピッチクラスが重なる場合は最大値を採用する（加算しない）', () => {
  const notes: MelodyNote[] = [
    { id: 1, startStep: 0, lengthSteps: 48, pitch: 60, level: 1 }, // C4。スロット0で開始しスロット1へ跨る(重み0.5)
    { id: 2, startStep: 24, lengthSteps: 24, pitch: 72, level: 1 }, // C5。スロット1でオンセット(重み1.0、同じピッチクラス0)
  ];
  const table = computeMelodySlotWeights(notes);
  assert.equal(table[1].get(0), 1.0, '加算(1.5)ではなく最大値(1.0)のはず');
});

test('computeMelodySlotWeights: 異なるピッチクラスのノートはそれぞれ別エントリーとして残る', () => {
  const notes: MelodyNote[] = [
    { id: 1, startStep: 0, lengthSteps: 12, pitch: 60, level: 1 }, // C (pc=0)
    { id: 2, startStep: 0, lengthSteps: 12, pitch: 64, level: 1 }, // E (pc=4)
    { id: 3, startStep: 0, lengthSteps: 12, pitch: 67, level: 1 }, // G (pc=7)
  ];
  const table = computeMelodySlotWeights(notes);
  assert.equal(table[0].size, 3);
  assert.equal(table[0].get(0), 1.0);
  assert.equal(table[0].get(4), 1.0);
  assert.equal(table[0].get(7), 1.0);
});

test('computeMelodySlotWeights: SLOT_COUNTを超える位置は無視する', () => {
  const total = SLOT_PULSES * SLOT_COUNT;
  const notes: MelodyNote[] = [{ id: 1, startStep: total - 12, lengthSteps: 24, pitch: 60, level: 1 }];
  const table = computeMelodySlotWeights(notes);
  assert.equal(table.length, SLOT_COUNT, '範囲外スロットへは書き込まないはず');
  assert.equal(table[SLOT_COUNT - 1].get(0), 1.0, '最後のスロットまでは反映されるはず');
});
