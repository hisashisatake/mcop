import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeCandidateGrid, createHistory, keyAt, currentEntry, pendingPivotAt, selectChord, undo, redo, jumpTo } from './chord-flow.js';
import { chordFromSemitone, NORMAL_LAYER } from './chords.js';

const TONIC_MIDI = 60; // C4
const C_MAJOR_KEY = { tonicPc: 0, mode: 'major' };

function rowIndexOf(layer, suffix) {
  const idx = layer.findIndex((e) => e.suffix === suffix);
  assert.notEqual(idx, -1, `suffix "${suffix}" not found in layer`);
  return idx;
}

function chordFor(semitoneFromC, rowIndex) {
  return chordFromSemitone(semitoneFromC, rowIndex, { tonicMidi: TONIC_MIDI, shiftHeld: false, ctrlHeld: false });
}

test('computeCandidateGrid: 各列はスコア降順', () => {
  const c = chordFor(0, rowIndexOf(NORMAL_LAYER, ''));
  const grid = computeCandidateGrid({
    lastChord: c,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 3,
    rows: 5,
  });
  for (let col = 0; col < 3; col++) {
    const cellsInCol = grid.filter((g) => g.col === col).sort((a, b) => a.row - b.row);
    for (let i = 1; i < cellsInCol.length; i++) {
      assert.ok(
        cellsInCol[i - 1].score >= cellsInCol[i].score,
        `col ${col} row ${i - 1}->${i} not descending: ${cellsInCol[i - 1].score} < ${cellsInCol[i].score}`,
      );
    }
  }
});

test('computeCandidateGrid: cols=1なら緑のみ、GREEN以外は現れない', () => {
  const c = chordFor(0, rowIndexOf(NORMAL_LAYER, ''));
  const grid = computeCandidateGrid({
    lastChord: c,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 1,
    rows: 5,
  });
  assert.ok(grid.length > 0);
  assert.ok(grid.every((g) => g.category === 'GREEN'));
  assert.ok(grid.every((g) => g.col === 0));
});

test('computeCandidateGrid: cols=3で行数を増やすと灰（category=null）が現れる', () => {
  // Cメジャートライアド起点だとGREEN+YELLOWだけで数十件あるため、灰まで届かせるには
  // rowsを大きくする必要がある（実際にゲーム内で↑キーを連打した状況に相当）。
  const c = chordFor(0, rowIndexOf(NORMAL_LAYER, ''));
  const grid = computeCandidateGrid({
    lastChord: c,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 3,
    rows: 20,
  });
  assert.ok(grid.some((g) => g.category === null), '十分な行数でも灰が出ない');
});

test('computeCandidateGrid: 緑の在庫が尽きたら黄が同じ列内で続く（カテゴリはcolではなく連続シーケンスで決まる）', () => {
  const c = chordFor(0, rowIndexOf(NORMAL_LAYER, ''));
  // rowsを大きくして緑の在庫（Cメジャーのダイアトニック7種前後）を使い切らせる
  const grid = computeCandidateGrid({
    lastChord: c,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 1,
    rows: 12,
  });
  // cols=1の12行取得で、GREEN在庫が12件未満ならYELLOWで埋まっているはず
  const greenCount = grid.filter((g) => g.category === 'GREEN').length;
  if (greenCount < grid.length) {
    assert.ok(grid.some((g) => g.category === 'YELLOW'), '緑の在庫切れ後に黄で埋まっていない');
  }
});

test('computeCandidateGrid: コード名は重複しない', () => {
  const c = chordFor(0, rowIndexOf(NORMAL_LAYER, ''));
  const grid = computeCandidateGrid({
    lastChord: c,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 3,
    rows: 12,
  });
  const names = grid.map((g) => g.chord.name);
  assert.equal(new Set(names).size, names.length);
});

test('computeCandidateGrid: 直前コードが無い1手目でも候補が出る', () => {
  const grid = computeCandidateGrid({
    lastChord: null,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 3,
    rows: 5,
  });
  assert.ok(grid.length > 0);
  assert.ok(grid.some((g) => g.category === 'GREEN'));
});

// ─────────────────────────────────────────────
// 履歴モデル
// ─────────────────────────────────────────────

const INITIAL_KEY = { tonicMidi: 60, mode: 'major' };

function entryFor(name) {
  return { chord: { name }, key: { tonicMidi: 60, mode: 'major' }, pendingPivot: null };
}

test('history: 初期状態はcursor=-1でinitialKeyを返す', () => {
  const h = createHistory(INITIAL_KEY);
  assert.equal(h.cursor, -1);
  assert.deepEqual(keyAt(h), INITIAL_KEY);
  assert.equal(currentEntry(h), null);
  assert.equal(pendingPivotAt(h), null);
});

test('history: selectChordで積み上がりcursorが進む', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, entryFor('C'));
  h = selectChord(h, entryFor('F'));
  assert.equal(h.cursor, 1);
  assert.equal(currentEntry(h).chord.name, 'F');
});

test('history: undo/redoでcursorが前後する', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, entryFor('C'));
  h = selectChord(h, entryFor('F'));
  h = undo(h);
  assert.equal(currentEntry(h).chord.name, 'C');
  h = redo(h);
  assert.equal(currentEntry(h).chord.name, 'F');
});

test('history: undoは先頭で止まり、redoは末尾で止まる', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, entryFor('C'));
  h = undo(h);
  h = undo(h); // 既に先頭
  assert.equal(h.cursor, -1);
  h = redo(h);
  h = redo(h); // 既に末尾
  assert.equal(h.cursor, 0);
});

test('history: 戻ってから新しいコードを選ぶと、その先の履歴は破棄される（上書き）', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, entryFor('C'));
  h = selectChord(h, entryFor('F'));
  h = selectChord(h, entryFor('G'));
  h = undo(h); // cursor=1 (F)
  h = selectChord(h, entryFor('Am')); // Gの先を破棄してAmを積む
  assert.equal(h.entries.length, 3);
  assert.deepEqual(h.entries.map((e) => e.chord.name), ['C', 'F', 'Am']);
  assert.equal(h.cursor, 2);
});

test('history: jumpToで任意の地点へ移動でき、キー状態も復元される', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, { ...entryFor('C'), key: { tonicMidi: 60, mode: 'major' } });
  h = selectChord(h, { ...entryFor('D7'), key: { tonicMidi: 60, mode: 'major' } });
  h = selectChord(h, { ...entryFor('G'), key: { tonicMidi: 67, mode: 'major' } }); // 転調したとする
  h = jumpTo(h, 0);
  assert.equal(currentEntry(h).chord.name, 'C');
  assert.deepEqual(keyAt(h), { tonicMidi: 60, mode: 'major' });
  h = jumpTo(h, 2);
  assert.deepEqual(keyAt(h), { tonicMidi: 67, mode: 'major' });
});

test('history: jumpToで-1（初期状態）へ戻せる', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, entryFor('C'));
  h = jumpTo(h, -1);
  assert.equal(h.cursor, -1);
  assert.equal(currentEntry(h), null);
});

test('history: jumpToは範囲外なら変化しない', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, entryFor('C'));
  const before = h;
  h = jumpTo(h, 5);
  assert.equal(h, before);
  h = jumpTo(h, -2);
  assert.equal(h, before);
});
