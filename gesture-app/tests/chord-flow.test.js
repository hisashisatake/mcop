import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeCandidateGrid, createHistory, keyAt, currentEntry, pendingPivotAt, selectChord, jumpTo } from '../src/chord-flow.js';
import { chordFromSemitone, NORMAL_LAYER } from '../src/chords.js';

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

test('computeCandidateGrid: progressionMatchesのターゲットがグリッド外でも末尾と入れ替えて必ず表示される', () => {
  const c = chordFor(0, rowIndexOf(NORMAL_LAYER, ''));
  // 度数6(F#)・halfdim(m7b5)はCから見て理論スコアが低く(0.45程度)、
  // cols=1,rows=3の3枠には通常入らないgray候補になる
  const progressionMatches = [{ id: 'test-progression', name: 'テスト進行', position: 1, total: 2, next: { degree: 6, families: ['halfdim'] } }];
  const grid = computeCandidateGrid({
    lastChord: c,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 1,
    rows: 3,
    progressionMatches,
  });
  assert.equal(grid.length, 3);
  const hinted = grid.find((cell) => cell.progressionHints.some((h) => h.id === 'test-progression'));
  assert.ok(hinted, 'ターゲットが強制的にグリッドへ割り込んでいるはず');
  const degree = (((hinted.chord.rootPc - C_MAJOR_KEY.tonicPc) % 12) + 12) % 12;
  assert.equal(degree, 6);
  assert.equal(hinted.row, 2, '末尾（最もスコアの低いセル）と入れ替わっているはず');
});

test('computeCandidateGrid: 割り込み数の上限はfloor(cols*rows/2)', () => {
  const c = chordFor(0, rowIndexOf(NORMAL_LAYER, ''));
  // グリッド外に落ちる3件のターゲットを渡すが、cols*rows=4なら上限floor(4/2)=2件までしか割り込まない
  const progressionMatches = [
    { id: 'p1', name: 'p1', position: 1, total: 2, next: { degree: 6, families: ['halfdim'] } },
    { id: 'p2', name: 'p2', position: 1, total: 2, next: { degree: 1, families: ['halfdim'] } },
    { id: 'p3', name: 'p3', position: 1, total: 2, next: { degree: 11, families: ['halfdim'] } },
  ];
  const grid = computeCandidateGrid({
    lastChord: c,
    key: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 2,
    rows: 2,
    progressionMatches,
  });
  const hintedCells = grid.filter((cell) => cell.progressionHints.length > 0);
  assert.ok(hintedCells.length <= Math.floor((2 * 2) / 2), '割り込み数はcols*rows/2以下のはず');
});

test('computeCandidateGrid: progressionKeyがkeyと異なる場合、next.degreeはprogressionKey基準で解決される', () => {
  // ピボット転調が確定した直後を模したケース: 表示上のkeyはAマイナー(転調後)だが、
  // progressionMatches自体は転調前のCメジャーを基準に度数計算されている前提
  // （chord-screen.jsのprogressionAnchorKey()参照）。次の一手(degree9:min)はCメジャー基準なら
  // 絶対ピッチクラス9(A)を指すはずで、もしkey(Aマイナー)基準のまま解決すると別の音
  // (絶対ピッチクラス6=F#)を指してしまう。IV→IIIaugで実際に踏んだ回帰。
  const lastChord = chordFor(4, rowIndexOf(NORMAL_LAYER, '')); // 便宜上の直前コード(スコアリングに影響するのみ)
  const A_MINOR_KEY = { tonicPc: 9, mode: 'minor' };
  // familiesではなくsuffixes指定にして、4レイヤー横断探索が'm6'/'mMaj7'等の別layer専用
  // バリエーションを最高スコアとして選んでしまう可能性を排除し、NORMAL_LAYERの'm'に固定する
  // （このテストの主眼はprogressionKey basisの検証であり、レイヤー横断選択自体は別テストの範囲）
  const progressionMatches = [{ id: 'test-anchor', name: 'テスト', position: 1, total: 2, next: { degree: 9, suffixes: ['m'] } }];
  const grid = computeCandidateGrid({
    lastChord,
    key: A_MINOR_KEY,
    progressionKey: C_MAJOR_KEY,
    tonicMidi: TONIC_MIDI,
    shiftHeld: false,
    ctrlHeld: false,
    cols: 1,
    rows: 3,
    progressionMatches,
  });
  const hinted = grid.find((cell) => cell.progressionHints.some((h) => h.id === 'test-anchor'));
  assert.ok(hinted, 'ターゲットが見つかるはず');
  const degreeFromAnchor = (((hinted.chord.rootPc - C_MAJOR_KEY.tonicPc) % 12) + 12) % 12;
  assert.equal(degreeFromAnchor, 9, 'progressionKey(Cメジャー)基準で度数9(A)に解決されるはず');
  const degreeFromDisplayKey = (((hinted.chord.rootPc - A_MINOR_KEY.tonicPc) % 12) + 12) % 12;
  assert.notEqual(degreeFromDisplayKey, 9, '表示key(Aマイナー)基準の度数9(F#)ではないはず');
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

test('history: 戻ってから新しいコードを選ぶと、その先の履歴は破棄される（上書き）', () => {
  let h = createHistory(INITIAL_KEY);
  h = selectChord(h, entryFor('C'));
  h = selectChord(h, entryFor('F'));
  h = selectChord(h, entryFor('G'));
  h = jumpTo(h, 1); // cursor=1 (F)
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
