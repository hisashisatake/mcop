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
} from '../src/midi-convert.ts';
import { parseSmf, buildSmf } from '../src/smf.ts';
import type { SmfEvent, SmfRawEvent } from '../src/types.ts';

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
  const bpm = bpmFromEvents([{ tick: 0, kind: 'tempo', microsecondsPerQuarter: us }]);
  assert.ok(Math.abs(bpm! - 140) < 1);
});

test('bpmFromEvents: tempoイベントが無ければnull', () => {
  assert.equal(bpmFromEvents([{ tick: 0, kind: 'noteOff', channel: 0, note: 60 }]), null);
});

test('pickMelodyChannel: ch10(=9)以外でノートがある最小番号のチャンネルを選ぶ', () => {
  const events: SmfEvent[] = [
    { tick: 0, kind: 'noteOn', channel: 9, note: 36, velocity: 100 },
    { tick: 0, kind: 'noteOn', channel: 3, note: 60, velocity: 100 },
    { tick: 0, kind: 'noteOn', channel: 1, note: 64, velocity: 100 },
  ];
  assert.equal(pickMelodyChannel(events), 1);
});

test('pickMelodyChannel: ch10しか無ければnull', () => {
  assert.equal(pickMelodyChannel([{ tick: 0, kind: 'noteOn', channel: 9, note: 36, velocity: 100 }]), null);
});

// melodyNotesToEvents/rhythmRowsToEventsはsmf.buildSmf()向けの{tick,bytes}形式（生成専用）を
// 返すため、midiEventsToXxx（parseSmf()の出力である{tick,kind,channel,...}形式を期待する）へ
// 直接渡すことはできない。実際の使われ方どおりbuildSmf→parseSmfを経由させて往復させる。
function roundTripEvents(trackEventLists: SmfRawEvent[][]): SmfEvent[] {
  const bytes = buildSmf({ ppq: 480, tracks: trackEventLists });
  return parseSmf(bytes).events;
}

test('melodyNotesToEvents→midiEventsToMelodyNotesのラウンドトリップ', () => {
  const notes = [
    { startStep: 2, lengthSteps: 3, pitch: 68, level: 1 },
    { startStep: 8, lengthSteps: 3, pitch: 60, level: 2 },
  ];
  const events = roundTripEvents([melodyNotesToEvents(notes, 1, TICKS_PER_MELODY_STEP)]);
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH, 1);
  assert.equal(restored.length, 2);
  assert.deepEqual(
    restored.map(({ startStep, lengthSteps, pitch, level }) => ({ startStep, lengthSteps, pitch, level })),
    notes,
  );
});

test('midiEventsToMelodyNotes: 音域外(MIN_PITCH未満)のノートは捨てる', () => {
  const events = roundTripEvents([melodyNotesToEvents([{ startStep: 0, lengthSteps: 1, pitch: 10, level: 1 }], 1, TICKS_PER_MELODY_STEP)]);
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH, 1);
  assert.equal(restored.length, 0);
});

test('midiEventsToMelodyNotes: 8小節を超える位置のノートは捨てる', () => {
  const events = roundTripEvents([
    melodyNotesToEvents([{ startStep: MELODY_TOTAL_STEPS + 10, lengthSteps: 1, pitch: 60, level: 1 }], 1, TICKS_PER_MELODY_STEP),
  ]);
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH, 1);
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
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH, 1);
  assert.equal(restored.length, 2);
  assert.equal(restored[0].startStep, 0);
  assert.equal(restored[0].lengthSteps, 4); // 4で打ち切られる
  assert.equal(restored[1].startStep, 4);
  assert.equal(restored[1].lengthSteps, 8);
});

test('rhythmRowsToEvents→midiEventsToRhythmRowsのラウンドトリップ', () => {
  const rows = [
    { note: 36, steps: [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
    { note: 38, steps: [0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  ];
  const events = roundTripEvents([rhythmRowsToEvents(rows, 9, TICKS_PER_RHYTHM_STEP)]);
  const restored = midiEventsToRhythmRows(events, 9, TICKS_PER_RHYTHM_STEP, 16, 1);
  assert.deepEqual(restored, rows);
});

test('midiEventsToRhythmRows: 畳み込みをせずタイムライン全体をそのまま取り込む（8小節そのまま）', () => {
  // タイムライン共通化以降、リズムも「最多出現パターンへ畳む」処理は行わず、
  // 前半・後半で内容が異なっていてもそのまま保持する。
  const TOTAL = 32; // テスト用の短いタイムライン（16パルス×2ぶん）
  const introRow = [{ note: 49, steps: [1, ...new Array(15).fill(0)] }]; // 前半のみクラッシュ
  const mainRow = [{ note: 36, steps: [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0] }]; // 後半のキック
  const introEvents = rhythmRowsToEvents(introRow, 9, TICKS_PER_RHYTHM_STEP);
  const mainEvents = rhythmRowsToEvents(mainRow, 9, TICKS_PER_RHYTHM_STEP).map((e) => ({
    ...e,
    tick: e.tick + 16 * TICKS_PER_RHYTHM_STEP, // 後半へシフト
  }));
  const events = roundTripEvents([[...introEvents, ...mainEvents]]);
  const restored = midiEventsToRhythmRows(events, 9, TICKS_PER_RHYTHM_STEP, TOTAL, 1);
  assert.equal(restored.length, 2);
  const crash = restored.find((r) => r.note === 49)!;
  const kick = restored.find((r) => r.note === 36)!;
  assert.equal(crash.steps.length, TOTAL);
  assert.deepEqual(crash.steps.filter((v) => v !== 0), [1]); // 前半にのみ1ヒット
  assert.deepEqual(kick.steps.slice(16), mainRow[0].steps); // 後半はそのまま保持される（畳まれない）
});

test('midiEventsToRhythmRows: 対象チャンネルにヒットが無ければ空配列', () => {
  assert.deepEqual(midiEventsToRhythmRows([{ tick: 0, kind: 'noteOn', channel: 0, note: 36, velocity: 100 }], 9, TICKS_PER_RHYTHM_STEP, 16, 1), []);
});

test('Export→SMFバイト列→Import で完全往復する（統合テスト）', () => {
  const rows = [{ note: 36, steps: [1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0] }];
  const notes = [{ startStep: 0, lengthSteps: 4, pitch: 60, level: 1 }];

  const bytes = buildSmf({
    ppq: 480,
    tracks: [
      [],
      melodyNotesToEvents(notes, 1, TICKS_PER_MELODY_STEP),
      rhythmRowsToEvents(rows, 9, TICKS_PER_RHYTHM_STEP),
    ],
  });
  const { events } = parseSmf(bytes);

  const channel = pickMelodyChannel(events);
  assert.equal(channel, 1);
  const restoredNotes = midiEventsToMelodyNotes(events, channel!, TICKS_PER_MELODY_STEP, MELODY_TOTAL_STEPS, MIN_PITCH, MAX_PITCH, 1);
  assert.deepEqual(restoredNotes.map(({ startStep, lengthSteps, pitch, level }) => ({ startStep, lengthSteps, pitch, level })), notes);

  const restoredRows = midiEventsToRhythmRows(events, 9, TICKS_PER_RHYTHM_STEP, 16, 1);
  assert.deepEqual(restoredRows, rows);
});

test('midiEventsToMelodyNotes: quantizePulsesで開始/終了位置を指定グリッドへ丸める', () => {
  const TICKS_PER_PULSE = 20; // ppq480 / 24
  // 6パルス(16分)グリッドに対し1パルスずれた生の演奏データを想定
  const events: SmfEvent[] = [
    { tick: 7 * TICKS_PER_PULSE, kind: 'noteOn', channel: 1, note: 60, velocity: 100 },
    { tick: 13 * TICKS_PER_PULSE, kind: 'noteOff', channel: 1, note: 60 },
  ];
  const restored = midiEventsToMelodyNotes(events, 1, TICKS_PER_PULSE, 96, MIN_PITCH, MAX_PITCH, 6);
  assert.equal(restored.length, 1);
  assert.equal(restored[0].startStep, 6); // 7パルス→最寄りの6の倍数(6)へスナップ
  assert.equal(restored[0].lengthSteps, 6); // 13パルス→12へスナップ、長さ=12-6=6
});

test('midiEventsToRhythmRows: quantizePulsesでゆらぎを吸収する', () => {
  const TICKS_PER_PULSE = 20; // ppq480 / 24
  // 人間の演奏で16分グリッド(6パルス)から1パルスずれたキック
  const events: SmfEvent[] = [{ tick: 7 * TICKS_PER_PULSE, kind: 'noteOn', channel: 9, note: 36, velocity: 100 }];

  const restored = midiEventsToRhythmRows(events, 9, TICKS_PER_PULSE, 96);
  assert.equal(restored.length, 1);
  assert.equal(restored[0].note, 36);
  assert.equal(restored[0].steps.indexOf(1), 6); // 7パルス→最寄りの6の倍数(6)へスナップ
});
