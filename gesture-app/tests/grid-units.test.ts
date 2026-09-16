import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  PULSES_PER_BEAT,
  PULSES_PER_BAR,
  SNAP_PULSES,
  SNAP_LABELS,
  snapFloor,
  snapRound,
  snapCeil,
  cellIndexOf,
  cellStartPulse,
  cellLevel,
  isBeatHead,
  isBarHead,
} from '../src/grid-units.ts';

test('PULSES_PER_BEAT/PULSES_PER_BARは24 PPQN×4拍の関係を保つ', () => {
  assert.equal(PULSES_PER_BEAT, 24);
  assert.equal(PULSES_PER_BAR, 96);
  assert.equal(PULSES_PER_BAR, PULSES_PER_BEAT * 4);
});

test('SNAP_PULSES/SNAP_LABELSは8段階で対応する長さを持つ', () => {
  assert.equal(SNAP_PULSES.length, 8);
  assert.equal(SNAP_LABELS.length, 8);
  // 全て1小節(96パルス)を割り切れる整数であること
  for (const snap of SNAP_PULSES) {
    assert.equal(PULSES_PER_BAR % snap, 0, `${snap}は96を割り切れない`);
  }
});

test('snapFloor/snapCeil/snapRoundはマス境界へ丸める', () => {
  const snap = 6; // 16分
  assert.equal(snapFloor(0, snap), 0);
  assert.equal(snapFloor(5, snap), 0);
  assert.equal(snapFloor(6, snap), 6);
  assert.equal(snapFloor(11, snap), 6);

  assert.equal(snapCeil(0, snap), 0);
  assert.equal(snapCeil(1, snap), 6);
  assert.equal(snapCeil(6, snap), 6);
  assert.equal(snapCeil(7, snap), 12);

  assert.equal(snapRound(2, snap), 0);
  assert.equal(snapRound(3, snap), 6); // 中間はMath.round方式で切り上げ側
  assert.equal(snapRound(4, snap), 6);
});

test('cellIndexOf/cellStartPulseは往復する', () => {
  const snap = 12; // 8分
  for (const index of [0, 1, 5, 7]) {
    const start = cellStartPulse(index, snap);
    assert.equal(cellIndexOf(start, snap), index);
    assert.equal(cellIndexOf(start + snap - 1, snap), index); // マス内のどのパルスでも同じマスに属する
  }
});

test('cellLevelは範囲内の最大レベルを返す（見たまま＝鳴る）', () => {
  const snap = 6;
  const steps = new Array(96).fill(0);
  steps[0] = 1;
  steps[3] = 2; // 同じマス内のより強いレベル
  assert.equal(cellLevel(steps, 0, snap), 2);
  assert.equal(cellLevel(steps, 6, snap), 0); // 打ち込みが無いマスは0
});

test('cellLevelは配列末尾を超えないよう範囲をクランプする', () => {
  const steps = [0, 0, 3];
  assert.equal(cellLevel(steps, 0, 6), 3);
});

test('isBeatHead/isBarHeadは拍・小節の境界を判定する', () => {
  assert.equal(isBeatHead(0), true);
  assert.equal(isBeatHead(24), true);
  assert.equal(isBeatHead(23), false);
  assert.equal(isBeatHead(48), true);

  assert.equal(isBarHead(0), true);
  assert.equal(isBarHead(96), true);
  assert.equal(isBarHead(24), false);
});
