import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computePastSlotGeoms } from './chord-layout.js';

const BASE_ARGS = {
  currentX: 556,
  currentSize: 64,
  slotY: 400,
  baseSize: 64,
  count: 999,
};

test('computePastSlotGeoms: サイズは単調減少し、minSizeを下回らない', () => {
  const geoms = computePastSlotGeoms(BASE_ARGS);
  assert.ok(geoms.length > 1);
  for (let i = 1; i < geoms.length; i++) {
    assert.ok(geoms[i].size <= geoms[i - 1].size, `size not decreasing at index ${i}`);
    assert.ok(geoms[i].size >= 20, `size below minSize at index ${i}`);
  }
});

test('computePastSlotGeoms: どのスロットも左端(leftMargin)を超えず、隣接スロットは重ならない', () => {
  const geoms = computePastSlotGeoms(BASE_ARGS);
  for (const g of geoms) {
    assert.ok(g.x - g.size / 2 >= 16, `slot ${g.index} left edge below leftMargin`);
  }
  for (let i = 1; i < geoms.length; i++) {
    const prevLeftEdge = geoms[i - 1].x - geoms[i - 1].size / 2;
    const curRightEdge = geoms[i].x + geoms[i].size / 2;
    assert.ok(curRightEdge <= prevLeftEdge + 0.001, `slot ${i} overlaps slot ${i - 1}`);
  }
});

test('computePastSlotGeoms: countより多くは返さない', () => {
  const geoms = computePastSlotGeoms({ ...BASE_ARGS, count: 3 });
  assert.equal(geoms.length, 3);
});

test('computePastSlotGeoms: 横幅を広げる(currentXを大きくする)と個数が増える', () => {
  const narrow = computePastSlotGeoms({ ...BASE_ARGS, currentX: 200 });
  const wide = computePastSlotGeoms({ ...BASE_ARGS, currentX: 800 });
  assert.ok(wide.length > narrow.length);
});

test('computePastSlotGeoms: 既定パラメーターで過去領域524pxのとき15個以上返る', () => {
  // currentX=556, currentSize=64 → currentX - currentSize/2 = 524（過去領域の右端）
  const geoms = computePastSlotGeoms(BASE_ARGS);
  assert.ok(geoms.length >= 15, `expected >=15 slots, got ${geoms.length}`);
});

test('computePastSlotGeoms: index=0が現在に最も近い(中心xが最大)', () => {
  const geoms = computePastSlotGeoms(BASE_ARGS);
  for (let i = 1; i < geoms.length; i++) {
    assert.ok(geoms[i].x < geoms[i - 1].x, `slot ${i} not further left than slot ${i - 1}`);
  }
});
