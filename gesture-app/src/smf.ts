// 標準MIDIファイル（SMF）のパース・生成。外部ライブラリ不使用、純粋関数のみ。
//
// Rust版（op505/tools/smf2op505/src/smf.rs のパーサ、op505/tools/vgm2op505/src/smf.rs の
// ライター）と同じ設計をJSへ移植した。Format 0/1のメトリカルタイミング(division>0)のみ対応
// （SMPTEタイミングは非対応）。gesture-appのImport/Exportに必要な最小限のイベント種別
// （NoteOn/NoteOff/Program/ControlChange/PitchBend/Tempoメタ）のみ解釈し、それ以外
// （SysEx・他のメタイベント等）は読み飛ばす。
import type { SmfEvent, SmfParseResult, SmfRawEvent } from './types.ts';

function readVlq(data: Uint8Array, i: number): [number, number] {
  // 32bitのビット演算だと4バイト目でオーバーフローしうるため、乗算で桁上げする
  // （Rust版はu64蓄積だがJSではNumberの安全な整数範囲で十分）。
  let val = 0;
  for (;;) {
    const b = data[i] ?? 0;
    i += 1;
    val = val * 128 + (b & 0x7f);
    if ((b & 0x80) === 0) break;
  }
  return [val, i];
}

function bytesToAscii(data: Uint8Array, start: number, end: number): string {
  let s = '';
  for (let i = start; i < end; i++) s += String.fromCharCode(data[i] ?? 0);
  return s;
}

/**
 * SMFバイト列をパースする。
 * @returns eventsはtick昇順（同tick内はファイル出現順）。
 */
export function parseSmf(data: Uint8Array): SmfParseResult {
  if (data.length < 14 || bytesToAscii(data, 0, 4) !== 'MThd') {
    throw new Error('MThdヘッダがありません');
  }
  const ntrk = (data[10] << 8) | data[11];
  const division = (data[12] << 8) | data[13];
  if (division & 0x8000) {
    throw new Error('SMPTEタイミング(division<0)は未対応です');
  }

  const events: SmfEvent[] = [];
  let i = 14;
  for (let t = 0; t < ntrk; t++) {
    if (i + 8 > data.length || bytesToAscii(data, i, i + 4) !== 'MTrk') break;
    const tlen = ((data[i + 4] << 24) | (data[i + 5] << 16) | (data[i + 6] << 8) | data[i + 7]) >>> 0;
    i += 8;
    const end = Math.min(i + tlen, data.length);
    let tick = 0;
    let status = 0;
    let j = i;
    while (j < end) {
      const [delta, nj] = readVlq(data, j);
      j = nj;
      tick += delta;
      let b = data[j] ?? 0;
      if (b & 0x80) {
        status = b;
        j += 1;
        b = data[j] ?? 0;
      }
      const ev = status & 0xf0;
      const channel = status & 0x0f;
      if (ev === 0x90) {
        const note = data[j] ?? 0;
        const vel = data[j + 1] ?? 0;
        j += 2;
        events.push(vel > 0 ? { tick, kind: 'noteOn', channel, note, velocity: vel } : { tick, kind: 'noteOff', channel, note });
      } else if (ev === 0x80) {
        const note = data[j] ?? 0;
        j += 2;
        events.push({ tick, kind: 'noteOff', channel, note });
      } else if (ev === 0xc0) {
        const program = data[j] ?? 0;
        j += 1;
        events.push({ tick, kind: 'program', channel, program });
      } else if (ev === 0xd0) {
        j += 1; // channel pressure、Import/Exportで使わないため読み飛ばし
      } else if (ev === 0xa0) {
        j += 2; // poly pressure、読み飛ばし
      } else if (ev === 0xb0) {
        const cc = data[j] ?? 0;
        const value = data[j + 1] ?? 0;
        j += 2;
        events.push({ tick, kind: 'controlChange', channel, cc, value });
      } else if (ev === 0xe0) {
        const lsb = data[j] ?? 0;
        const msb = data[j + 1] ?? 0;
        j += 2;
        events.push({ tick, kind: 'pitchBend', channel, value: ((msb << 7) | lsb) - 8192 });
      } else if (status === 0xff) {
        const meta = data[j] ?? 0;
        j += 1;
        const [mlen, nj2] = readVlq(data, j);
        j = nj2;
        if (meta === 0x51 && mlen === 3) {
          const us = ((data[j] ?? 0) << 16) | ((data[j + 1] ?? 0) << 8) | (data[j + 2] ?? 0);
          events.push({ tick, kind: 'tempo', microsecondsPerQuarter: us });
        }
        j += mlen;
      } else if (status === 0xf0 || status === 0xf7) {
        const [slen, nj2] = readVlq(data, j);
        j = nj2;
        j += slen; // SysEx本体、読み飛ばし
      } else {
        // 未知のステータス（壊れたファイル等）。これ以上安全に読み進められないため
        // このトラックを打ち切る（後続トラックのパースは続行する）。
        break;
      }
    }
    i = end;
  }

  events.sort((a, b) => a.tick - b.tick);
  return { division, events };
}

// ─────────────────────────────────────────────
// 生成（Format 1、複数トラック）
// ─────────────────────────────────────────────

export function noteOnEvent(tick: number, channel: number, note: number, velocity: number): SmfRawEvent {
  return { tick, bytes: [0x90 | (channel & 0x0f), note & 0x7f, velocity & 0x7f] };
}

export function noteOffEvent(tick: number, channel: number, note: number): SmfRawEvent {
  return { tick, bytes: [0x80 | (channel & 0x0f), note & 0x7f, 0] };
}

export function tempoMetaEvent(tick: number, microsecondsPerQuarter: number): SmfRawEvent {
  const us = microsecondsPerQuarter & 0xffffff;
  return { tick, bytes: [0xff, 0x51, 0x03, (us >> 16) & 0xff, (us >> 8) & 0xff, us & 0xff] };
}

/** 4/4拍子固定（gesture-appは他の拍子を扱わないため）。 */
export function timeSignatureMetaEvent(tick: number): SmfRawEvent {
  return { tick, bytes: [0xff, 0x58, 0x04, 4, 2, 24, 8] };
}

function writeVlq(buf: number[], value: number): void {
  const stack = [value & 0x7f];
  value = Math.floor(value / 128);
  while (value > 0) {
    stack.push((value & 0x7f) | 0x80);
    value = Math.floor(value / 128);
  }
  for (let i = stack.length - 1; i >= 0; i--) buf.push(stack[i]);
}

function pushString(buf: number[], s: string): void {
  for (let i = 0; i < s.length; i++) buf.push(s.charCodeAt(i));
}

function pushU32(buf: number[], v: number): void {
  buf.push((v >>> 24) & 0xff, (v >>> 16) & 0xff, (v >>> 8) & 0xff, v & 0xff);
}

function pushU16(buf: number[], v: number): void {
  buf.push((v >>> 8) & 0xff, v & 0xff);
}

function serializeTrack(events: SmfRawEvent[]): number[] {
  const sorted = [...events].sort((a, b) => a.tick - b.tick);
  const buf: number[] = [];
  let prevTick = 0;
  for (const { tick, bytes } of sorted) {
    writeVlq(buf, Math.max(0, tick - prevTick));
    buf.push(...bytes);
    prevTick = tick;
  }
  buf.push(0x00, 0xff, 0x2f, 0x00); // End of Track（delta=0）
  return buf;
}

export interface BuildSmfSpec {
  ppq: number;
  tracks: SmfRawEvent[][];
}

/**
 * SMF Format 1のバイト列を組み立てる。各トラックの末尾にEnd of Trackを自動付与する。
 */
export function buildSmf({ ppq, tracks }: BuildSmfSpec): Uint8Array {
  const buf: number[] = [];
  pushString(buf, 'MThd');
  pushU32(buf, 6);
  pushU16(buf, 1); // format 1
  pushU16(buf, tracks.length);
  pushU16(buf, ppq);

  for (const trackEvents of tracks) {
    const trackBytes = serializeTrack(trackEvents);
    pushString(buf, 'MTrk');
    pushU32(buf, trackBytes.length);
    buf.push(...trackBytes);
  }
  return new Uint8Array(buf);
}
