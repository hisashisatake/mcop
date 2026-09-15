import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseSmf, buildSmf, noteOnEvent, noteOffEvent, tempoMetaEvent, timeSignatureMetaEvent } from '../src/smf.ts';

test('buildSmf→parseSmfのラウンドトリップ: NoteOn/NoteOffが正しく復元される', () => {
  const bytes = buildSmf({
    ppq: 480,
    tracks: [
      [tempoMetaEvent(0, 500_000), timeSignatureMetaEvent(0)],
      [noteOnEvent(0, 1, 60, 100), noteOffEvent(240, 1, 60), noteOnEvent(240, 1, 64, 90), noteOffEvent(480, 1, 64)],
    ],
  });
  const { division, events } = parseSmf(bytes);
  assert.equal(division, 480);

  const noteOns = events.filter((e) => e.kind === 'noteOn');
  assert.equal(noteOns.length, 2);
  assert.deepEqual(noteOns[0], { tick: 0, kind: 'noteOn', channel: 1, note: 60, velocity: 100 });
  assert.deepEqual(noteOns[1], { tick: 240, kind: 'noteOn', channel: 1, note: 64, velocity: 90 });

  const noteOffs = events.filter((e) => e.kind === 'noteOff');
  assert.equal(noteOffs.length, 2);
  assert.deepEqual(noteOffs[0], { tick: 240, kind: 'noteOff', channel: 1, note: 60 });
  assert.deepEqual(noteOffs[1], { tick: 480, kind: 'noteOff', channel: 1, note: 64 });

  const tempo = events.find((e) => e.kind === 'tempo');
  assert.deepEqual(tempo, { tick: 0, kind: 'tempo', microsecondsPerQuarter: 500_000 });
});

test('velocity=0のNoteOnはNoteOffとして解釈される（MIDI仕様どおり）', () => {
  const bytes = buildSmf({ ppq: 480, tracks: [[{ tick: 0, bytes: [0x90, 60, 0] }]] });
  const { events } = parseSmf(bytes);
  assert.deepEqual(events[0], { tick: 0, kind: 'noteOff', channel: 0, note: 60 });
});

test('parseSmf: MThdヘッダが無いデータはエラーになる', () => {
  assert.throws(() => parseSmf(new Uint8Array([1, 2, 3])), /MThd/);
});

test('parseSmf: 複数トラック(Format1)のイベントがtick昇順にマージされる', () => {
  const bytes = buildSmf({
    ppq: 480,
    tracks: [
      [noteOnEvent(480, 0, 60, 100)],
      [noteOnEvent(0, 1, 40, 90)],
    ],
  });
  const { events } = parseSmf(bytes);
  const noteOns = events.filter((e) => e.kind === 'noteOn');
  assert.equal(noteOns[0].tick, 0);
  assert.equal(noteOns[1].tick, 480);
});

test('parseSmf: ControlChange/Programイベントも解釈される', () => {
  const bytes = buildSmf({
    ppq: 480,
    tracks: [[{ tick: 0, bytes: [0xb0, 7, 100] }, { tick: 0, bytes: [0xc0, 5] }]],
  });
  const { events } = parseSmf(bytes);
  assert.deepEqual(events[0], { tick: 0, kind: 'controlChange', channel: 0, cc: 7, value: 100 });
  assert.deepEqual(events[1], { tick: 0, kind: 'program', channel: 0, program: 5 });
});

test('parseSmf: SysExイベントは読み飛ばされ後続イベントの解釈がずれない', () => {
  const bytes = buildSmf({
    ppq: 480,
    tracks: [[{ tick: 0, bytes: [0xf0, 0x03, 0x7f, 0x7f, 0xf7] }, noteOnEvent(0, 0, 60, 100)]],
  });
  const { events } = parseSmf(bytes);
  assert.equal(events.length, 1);
  assert.equal(events[0].kind, 'noteOn');
});

test('writeVlq/readVlqのラウンドトリップ: 大きめのtick値でも壊れない(4バイトVLQ境界)', () => {
  const bigTick = 0x0fffffff; // VLQ最大4バイト
  const bytes = buildSmf({ ppq: 480, tracks: [[noteOnEvent(bigTick, 0, 60, 100)]] });
  const { events } = parseSmf(bytes);
  assert.equal(events[0].tick, bigTick);
});
