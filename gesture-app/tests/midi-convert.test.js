import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  levelToVelocity,
  velocityToLevel,
  bpmFromEvents,
  microsecondsPerQuarterFromBpm,
  pickMelodyChannel,
  melodyNotesToEvents,
  midiEventsToMelodyNotes,
  rhythmRowsToEvents,
  midiEventsToRhythmRows,
} from '../src/midi-convert.js';
import { parseSmf, buildSmf } from '../src/smf.js';

const TICKS_PER_MELODY_STEP = 240; // ppq480 / 2（8分音符）
const TICKS_PER_RHYTHM_STEP = 120; // ppq480 / 4（16分音符）
const MELODY_TOTAL_STEPS = 64;
const MIN_PITCH = 21;
const MAX_PITCH = 108;

test('levelToVelocity/velocityToLevelは3段階を正しく往復する', () => {
  for (const level of [1, 2, 3]) {
    assert.equal(velocityToLevel(levelToVelocity(level)), level);
  }
  assert.equal(velocityToLevel(100), 1); // 95に一番近い
  assert.equal(velocityToLevel(120), 2); // 127に一番近い
  assert.equal(velocityToLevel(60), 3); // 55に一番近い
});

test('bpmFromEvents/microsecondsPerQuarterFromBpmは往復する（丸め誤差1bpm以内）', () => {
  const us = microsecondsPerQuarterFromBpm(140);
  const bpm = bpmFromEvents([{ kind: 'tempo', microsecondsPerQuarter: us }]);
  assert.ok(Math.abs(bpm - 140) < 1);
});

test('bpmFromEvents: tempoイベントが無ければnull', () => {
  assert.equal(bpmFromEvents([{ kind: 'noteOn' }]), null);
});

test('pickMelodyChannel: ch10(=9)以外でノートがある最小番号のチャンネルを選ぶ', () => {
  const events = [
    { kind: 'noteOn', channel: 9, note: 36 },
    { kind: 'noteOn', channel: 3, note: 60 },
    { kind: 'noteOn', channel: 1, note: 64 },
  ];
  assert.equal(pickMelodyChannel(events), 1);
});

test('pickMelodyChannel: ch10しか無ければnull', () => {
  assert.equal(pickMelodyChannel([{ kind: 'noteOn', channel: 9, note: 36 }]), null);
});

// melodyNotesToEvents/rhythmRowsToEventsはsmf.buildSmf()向けの{tick,bytes}形式（生成専用）を
// 返すため、midiEventsToXxx（parseSmf()の出力である{tick,kind,channel,...}形式を期待する）へ
// 直接渡すことはできない。実際の使われ方どおりbuildSmf→parseSmfを経由させて往復させる。
function roundTripEvents(trackEventLists) {
  const bytes = buildSmf({ ppq: 480, tracks: trackEventLists });
  return parseSmf(bytes).events;
}

test('melodyNotesToEvents→midiEventsToMelodyNotesのラウンドトリップ', () => {
  const notes = [
    { startStep: 2, lengthSteps: 3, pitch: 68, level: 1 },
    { startStep: 8, lengthSteps: 3, pitch: 60, level: 2 },
  ];
  const events = roundTripEvents([melodyNotesToEvents(notes, 1, TICKS_PER_MELODY_STEP)]);
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH);
  assert.equal(restored.length, 2);
  assert.deepEqual(
    restored.map(({ startStep, lengthSteps, pitch, level }) => ({ startStep, lengthSteps, pitch, level })),
    notes,
  );
});

test('midiEventsToMelodyNotes: 音域外(MIN_PITCH未満)のノートは捨てる', () => {
  const events = roundTripEvents([melodyNotesToEvents([{ startStep: 0, lengthSteps: 1, pitch: 10, level: 1 }], 1, TICKS_PER_MELODY_STEP)]);
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH);
  assert.equal(restored.length, 0);
});

test('midiEventsToMelodyNotes: 8小節を超える位置のノートは捨てる', () => {
  const events = roundTripEvents([
    melodyNotesToEvents([{ startStep: MELODY_TOTAL_STEPS + 10, lengthSteps: 1, pitch: 60, level: 1 }], 1, TICKS_PER_MELODY_STEP),
  ]);
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH);
  assert.equal(restored.length, 0);
});

test('midiEventsToMelodyNotes: 同じ音高がlegato気味に重なる場合は先のノートを次の開始位置で打ち切る', () => {
  // 完全な入れ子（前のノートが後のノートを内包する）は、そもそもnoteOn/noteOffの並びだけでは
  // 一意に復元できないMIDI表現として対象外（テストしない）。ここでは「前のノートの終わりが
  // 次のノートの始まりより後」という現実的なlegato重なりのみ扱う。
  const events = roundTripEvents([
    melodyNotesToEvents(
      [
        { startStep: 0, lengthSteps: 6, pitch: 60, level: 1 }, // 0〜6だが次のノートで4に打ち切られるはず
        { startStep: 4, lengthSteps: 8, pitch: 60, level: 2 },
      ],
      1,
      TICKS_PER_MELODY_STEP,
    ),
  ]);
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH);
  assert.equal(restored.length, 2);
  assert.equal(restored[0].startStep, 0);
  assert.equal(restored[0].lengthSteps, 4); // 4で打ち切られる
  assert.equal(restored[1].startStep, 4);
  assert.equal(restored[1].lengthSteps, 8);
});

test('rhythmRowsToEvents→midiEventsToRhythmRowsのラウンドトリップ（1小節）', () => {
  const rows = [
    { note: 36, steps: [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
    { note: 38, steps: [0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  ];
  const events = roundTripEvents([rhythmRowsToEvents(rows, 9, TICKS_PER_RHYTHM_STEP, 16, 1)]);
  const restored = midiEventsToRhythmRows(events, 9, TICKS_PER_RHYTHM_STEP, 16);
  assert.deepEqual(restored, rows);
});

test('midiEventsToRhythmRows: 複数小節では最多出現パターンを採用する（イントロ等の単発小節は無視）', () => {
  const introRow = [{ note: 49, steps: [1, ...new Array(15).fill(0)] }]; // 1小節目だけクラッシュ
  const mainRow = [{ note: 36, steps: [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0] }];
  const introEvents = rhythmRowsToEvents(introRow, 9, TICKS_PER_RHYTHM_STEP, 16, 1);
  const mainEvents = rhythmRowsToEvents(mainRow, 9, TICKS_PER_RHYTHM_STEP, 16, 3).map((e) => ({
    ...e,
    tick: e.tick + 16 * TICKS_PER_RHYTHM_STEP, // 2小節目以降へシフト
  }));
  const events = roundTripEvents([[...introEvents, ...mainEvents]]);
  const restored = midiEventsToRhythmRows(events, 9, TICKS_PER_RHYTHM_STEP, 16);
  assert.deepEqual(restored, mainRow);
});

test('midiEventsToRhythmRows: 対象チャンネルにヒットが無ければ空配列', () => {
  assert.deepEqual(midiEventsToRhythmRows([{ kind: 'noteOn', channel: 0, tick: 0, note: 36, velocity: 100 }], 9, TICKS_PER_RHYTHM_STEP, 16), []);
});

test('Export→SMFバイト列→Import で完全往復する（統合テスト）', () => {
  const rows = [{ note: 36, steps: [1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0] }];
  const notes = [{ startStep: 0, lengthSteps: 4, pitch: 60, level: 1 }];

  const bytes = buildSmf({
    ppq: 480,
    tracks: [
      [],
      melodyNotesToEvents(notes, 1, TICKS_PER_MELODY_STEP),
      rhythmRowsToEvents(rows, 9, TICKS_PER_RHYTHM_STEP, 16, 8),
    ],
  });
  const { events } = parseSmf(bytes);

  const channel = pickMelodyChannel(events);
  assert.equal(channel, 1);
  const restoredNotes = midiEventsToMelodyNotes(events, channel, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH);
  assert.deepEqual(restoredNotes.map(({ startStep, lengthSteps, pitch, level }) => ({ startStep, lengthSteps, pitch, level })), notes);

  const restoredRows = midiEventsToRhythmRows(events, 9, TICKS_PER_RHYTHM_STEP, 16);
  assert.deepEqual(restoredRows, rows);
});
