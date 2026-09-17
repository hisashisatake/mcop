import { test } from 'node:test';
import assert from 'node:assert/strict';
import { expandRhythmSteps16To96, expandMelodyNotesV2, expandPatternV1, repeatRhythmBarToSequence } from '../src/grid-migrate.ts';
import { PULSES_PER_BAR, SEQUENCE_BARS, SEQUENCE_TOTAL_PULSES } from '../src/grid-units.ts';
import type { MelodyNote } from '../src/types.ts';

test('expandRhythmSteps16To96は各ステップの値を新index×6の単一パルスへ配置する（複製しない）', () => {
  // 発音判定が1パルス単位（非0なら発音）になるため、6パルス全部を埋めると
  // 1ヒットが6回連続で再トリガーされてしまう。単一パルス配置ならその心配が無い。
  const steps = new Array(16).fill(0);
  steps[0] = 1;
  steps[1] = 2;
  steps[15] = 3;

  const expanded = expandRhythmSteps16To96(steps);
  assert.equal(expanded.length, PULSES_PER_BAR);
  assert.equal(expanded[0], 1);
  assert.equal(expanded[6], 2);
  assert.equal(expanded[90], 3);
  // それ以外のパルスは全て消音のまま
  const nonZeroCount = expanded.filter((v) => v !== 0).length;
  assert.equal(nonZeroCount, 3);
});

test('expandRhythmSteps16To96は16要素に満たない入力を0で補完する', () => {
  const expanded = expandRhythmSteps16To96([1]);
  assert.equal(expanded.length, PULSES_PER_BAR);
  assert.equal(expanded[0], 1);
  assert.equal(expanded.filter((v) => v !== 0).length, 1);
});

test('expandMelodyNotesV2はstartStep/lengthStepsを×12する', () => {
  const notes: MelodyNote[] = [
    { id: 1, startStep: 0, lengthSteps: 1, pitch: 60, level: 2 },
    { id: 2, startStep: 4, lengthSteps: 2, pitch: 64, level: 3 },
  ];
  const expanded = expandMelodyNotesV2(notes);

  assert.deepEqual(expanded, [
    { id: 1, startStep: 0, lengthSteps: 12, pitch: 60, level: 2 },
    { id: 2, startStep: 48, lengthSteps: 24, pitch: 64, level: 3 },
  ]);
  // 元の配列は書き換わらない
  assert.equal(notes[0].startStep, 0);
  assert.equal(notes[0].lengthSteps, 1);
});

test('repeatRhythmBarToSequenceは1小節分(96パルス)をSEQUENCE_BARS回そのまま繰り返す', () => {
  const bar = new Array(PULSES_PER_BAR).fill(0);
  bar[0] = 1;
  bar[6] = 2;

  const sequence = repeatRhythmBarToSequence(bar);
  assert.equal(sequence.length, SEQUENCE_TOTAL_PULSES);
  for (let b = 0; b < SEQUENCE_BARS; b++) {
    assert.equal(sequence[b * PULSES_PER_BAR + 0], 1);
    assert.equal(sequence[b * PULSES_PER_BAR + 6], 2);
  }
});

test('expandPatternV1は12行の固定パターンをRhythmRow[]（768パルス/行、8小節分繰り返し）へ変換する', () => {
  const pattern: number[][] = [];
  for (let i = 0; i < 12; i++) {
    const row = new Array(16).fill(0);
    row[0] = i % 4; // 行ごとに違うレベルで区別できるようにする
    pattern.push(row);
  }

  const rows = expandPatternV1(pattern);
  assert.equal(rows.length, 12);
  for (let i = 0; i < 12; i++) {
    assert.equal(rows[i].steps.length, SEQUENCE_TOTAL_PULSES);
    assert.equal(rows[i].steps[0], i % 4);
    // 1小節ループだったv1のパターンは、8小節とも同じ内容で繰り返される。
    assert.equal(rows[i].steps[PULSES_PER_BAR], i % 4);
    assert.equal(rows[i].steps[PULSES_PER_BAR * (SEQUENCE_BARS - 1)], i % 4);
    assert.ok(typeof rows[i].note === 'number');
    assert.ok(typeof rows[i].label === 'string');
  }
});

test('expandPatternV1は行数が12に満たない入力を消音の行で補完する', () => {
  const rows = expandPatternV1([[1, 2, 3]]);
  assert.equal(rows.length, 12);
  assert.equal(rows[0].steps[0], 1);
  assert.deepEqual(rows[1].steps, new Array(SEQUENCE_TOTAL_PULSES).fill(0));
});
