import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chordFromSemitone, NORMAL_LAYER } from '../src/chords.js';
import { voiceChord, rawVoicing, MIN_MIDI, MAX_MIDI } from '../src/voicing.js';

const TONIC_MIDI = 60; // C4

function chordFor(semitoneFromC, suffix) {
  const rowIndex = NORMAL_LAYER.findIndex((e) => e.suffix === suffix);
  assert.notEqual(rowIndex, -1, `suffix "${suffix}" not found in NORMAL_LAYER`);
  return chordFromSemitone(semitoneFromC, rowIndex, { tonicMidi: TONIC_MIDI, shiftHeld: false, ctrlHeld: false });
}

function pitchClassSet(notes) {
  return new Set(notes.map((n) => ((n % 12) + 12) % 12));
}

function avgMovement(notes, previousNotes) {
  let total = 0;
  for (const n of notes) {
    let best = Infinity;
    for (const p of previousNotes) best = Math.min(best, Math.abs(n - p));
    total += best;
  }
  return total / notes.length;
}

test('C→E→A（3度ずつ上行）を連続適用しても音域が想定バンド内に収まる', () => {
  const c = chordFor(0, '');
  const e = chordFor(4, '');
  const a = chordFor(9, '');

  const centerMidi = 60;
  let notes = voiceChord(c, { previousNotes: [], centerMidi });
  const maxima = [Math.max(...notes)];
  for (const chord of [e, a]) {
    notes = voiceChord(chord, { previousNotes: notes, centerMidi });
    maxima.push(Math.max(...notes));
  }

  for (const m of maxima) {
    assert.ok(m <= 84, `最高音が想定バンドを超えている: ${m}`);
  }
});

test('自動転回は従来方式（rawVoicing）より声部移動量が小さくなる', () => {
  const c = chordFor(0, '');
  const e = chordFor(4, '');

  const cVoicing = voiceChord(c, { previousNotes: [], centerMidi: 60 });
  const eVoicingAuto = voiceChord(e, { previousNotes: cVoicing, centerMidi: 60 });
  const eVoicingRaw = rawVoicing(e, 0);

  const autoCost = avgMovement(eVoicingAuto, cVoicing);
  const rawCost = avgMovement(eVoicingRaw, cVoicing);
  assert.ok(autoCost <= rawCost, `自動転回のほうが移動量が大きい: auto=${autoCost} raw=${rawCost}`);
});

test('13thコードのボイシングは構成音（ピッチクラス集合）を保つ', () => {
  const chord = chordFor(0, '13'); // intervals: [0,4,7,10,14,21]
  const notes = voiceChord(chord, { previousNotes: [], centerMidi: 60 });
  const expected = pitchClassSet(chord.intervals.map((i) => chord.rootMidi + i));
  assert.deepEqual(pitchClassSet(notes), expected);
});

test('1手目（previousNotesが空）は音域アンカーcenterMidi近傍に来る', () => {
  const chord = chordFor(0, '');
  const notes = voiceChord(chord, { previousNotes: [], centerMidi: 60 });
  const centroid = notes.reduce((a, b) => a + b, 0) / notes.length;
  assert.ok(Math.abs(centroid - 60) < 12, `重心がcenterMidiから離れすぎている: ${centroid}`);
});

test('基準オクターブ(baseOctave)を-1にすると結果が概ね12半音下がる', () => {
  const chord = chordFor(0, '');
  const centerMidi0 = 60;
  const centerMidiDown = 60 - 12;
  const notes0 = voiceChord(chord, { previousNotes: [], centerMidi: centerMidi0 });
  const notesDown = voiceChord(chord, { previousNotes: [], centerMidi: centerMidiDown });

  const centroid0 = notes0.reduce((a, b) => a + b, 0) / notes0.length;
  const centroidDown = notesDown.reduce((a, b) => a + b, 0) / notesDown.length;
  assert.ok(Math.abs(centroid0 - centroidDown - 12) < 6, `1オクターブ分下がっていない: ${centroid0} -> ${centroidDown}`);
});

test('rawVoicing(chord, 0)は変更前のrootMidi+intervalマッピングと一致する', () => {
  const chord = chordFor(4, '7'); // E7
  const legacy = chord.intervals.map((i) => chord.rootMidi + i);
  assert.deepEqual(rawVoicing(chord, 0), legacy);
});

test('rawVoicingのbaseOctaveは12半音単位でシフトする', () => {
  const chord = chordFor(0, '');
  const base = rawVoicing(chord, 0);
  const up = rawVoicing(chord, 1);
  const down = rawVoicing(chord, -1);
  assert.deepEqual(up, base.map((n) => n + 12));
  assert.deepEqual(down, base.map((n) => n - 12));
});

test('requireRootInBass: 移動量最小化だけだとバスが根音に着地しないケースで根音着地を強制する', () => {
  const g7 = chordFor(7, '7'); // G7
  const c = chordFor(0, ''); // C

  const g7Voicing = voiceChord(g7, { previousNotes: [], centerMidi: 60 });
  const cVoicingFree = voiceChord(c, { previousNotes: g7Voicing, centerMidi: 60 });
  assert.notEqual(cVoicingFree[0] % 12, c.rootPc, '前提: 自由選択だと第二転回形（バス=G）になる');

  const cVoicingForced = voiceChord(c, { previousNotes: g7Voicing, centerMidi: 60, requireRootInBass: true });
  assert.equal(((cVoicingForced[0] % 12) + 12) % 12, c.rootPc, 'requireRootInBass指定時はバスが根音になるべき');
});

test('voiceChordは短2度(半音)で密集する配置を選ばない（テンションがコアトーンの転回に埋もれて濁る問題の回帰テスト）', () => {
  // maj9のコアトーン(0,4,7,11)が転回で1オクターブ上がると、11(長7度)と12(オクターブ上の
  // ルート)が半音で隣接し、さらに9th(14)がその間に埋もれる形になっていた
  const maj9Def = NORMAL_LAYER.find((d) => d.suffix === 'maj9');
  for (let rootPc = 0; rootPc < 12; rootPc++) {
    const notes = voiceChord({ rootPc, intervals: maj9Def.intervals }, { previousNotes: [], centerMidi: 60 });
    for (let i = 1; i < notes.length; i++) {
      assert.notEqual(notes[i] - notes[i - 1], 1, `半音で密集: root=${rootPc} notes=[${notes.join(',')}]`);
    }
  }
});

test('voiceChordはMIDI範囲[0,127]を超えるボイシングを選ばない', () => {
  const chord = chordFor(0, '13');
  const notes = voiceChord(chord, { previousNotes: [], centerMidi: 60 });
  for (const n of notes) {
    assert.ok(n >= MIN_MIDI && n <= MAX_MIDI, `MIDI範囲外の音: ${n}`);
  }
});
