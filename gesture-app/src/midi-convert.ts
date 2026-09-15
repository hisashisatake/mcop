// リズムグリッド/メロディノートとSMFイベント列の相互変換。純粋関数のみ（smf.jsと同じ方針）。
// gesture-appのグリッドは固定分割（メロディ=8分音符単位、リズム=16分音符単位）のため、
// Export（グリッド→SMF）は情報を失わないが、Import（SMF→グリッド）は必ず量子化・丸めを伴う。
// 詳細な設計判断（取込チャンネルの選び方・複数小節の畳み方等）はmemory
// `project_gesture_app_melody_screen_and_file_menu_plan.md`「フェーズ3」節参照。

import { noteOnEvent, noteOffEvent } from './smf.ts';
import type { MelodyNote, RhythmRowData, SmfEvent, SmfRawEvent } from './types.ts';

const RHYTHM_CHANNEL = 9; // ch10（0-indexed）。standalone側のGM2リズムチャンネルと一致。

// [消音, 通常, アクセント, 弱] に対応するベロシティ。Rust側`midi_out.rs`の
// RHYTHM_VELOCITY_NORMAL/ACCENT/WEAKと一致させること。
const LEVEL_VELOCITY: Record<number, number> = { 1: 95, 2: 127, 3: 55 };

export function levelToVelocity(level: number): number {
  return LEVEL_VELOCITY[level] ?? LEVEL_VELOCITY[1];
}

/** ベロシティを最も近いレベル(1/2/3)へ丸める。 */
export function velocityToLevel(velocity: number): number {
  let best = 1;
  let bestDist = Infinity;
  for (const [level, v] of Object.entries(LEVEL_VELOCITY)) {
    const dist = Math.abs(v - velocity);
    if (dist < bestDist) {
      bestDist = dist;
      best = Number(level);
    }
  }
  return best;
}

export function bpmFromEvents(events: SmfEvent[]): number | null {
  const tempo = events.find((e) => e.kind === 'tempo');
  return tempo ? 60_000_000 / tempo.microsecondsPerQuarter : null;
}

export function microsecondsPerQuarterFromBpm(bpm: number): number {
  return Math.round(60_000_000 / bpm);
}

/**
 * ch10以外でnoteOnが存在する最小番号のチャンネルを返す（メロディの取込元と判断する）。
 * 該当が無ければnull。
 */
export function pickMelodyChannel(events: SmfEvent[]): number | null {
  let best: number | null = null;
  for (const e of events) {
    if (e.kind !== 'noteOn' || e.channel === RHYTHM_CHANNEL) continue;
    if (best === null || e.channel < best) best = e.channel;
  }
  return best;
}

// ─────────────────────────────────────────────
// メロディ: グリッド → SMFイベント（Export、情報を失わない）
// ─────────────────────────────────────────────

export function melodyNotesToEvents(
  notes: Array<Pick<MelodyNote, 'startStep' | 'lengthSteps' | 'pitch' | 'level'>>,
  channel: number,
  ticksPerStep: number,
): SmfRawEvent[] {
  const events: SmfRawEvent[] = [];
  for (const n of notes) {
    const startTick = n.startStep * ticksPerStep;
    const endTick = (n.startStep + n.lengthSteps) * ticksPerStep;
    events.push(noteOnEvent(startTick, channel, n.pitch, levelToVelocity(n.level)));
    events.push(noteOffEvent(endTick, channel, n.pitch));
  }
  return events;
}

// ─────────────────────────────────────────────
// メロディ: SMFイベント → グリッド（Import、量子化を伴う）
// ─────────────────────────────────────────────

/**
 * @param events parseSmf()の出力
 * @param channel pickMelodyChannel()で決めた取込元チャンネル
 */
export function midiEventsToMelodyNotes(
  events: SmfEvent[],
  channel: number,
  ticksPerStep: number,
  totalSteps: number,
  minPitch: number,
  maxPitch: number,
): MelodyNote[] {
  // 同じ音高のノートが（legatoの重なり等で）連続するSMFでは、noteOn/noteOffの対応関係が
  // 単純な「ピッチごとに1個」のスロットでは壊れる（後発のnoteOnが先発の対応情報を上書きし、
  // 先発ノートを取りこぼす）。ピッチごとにスタックを持ち、noteOffは先に鳴り始めたノートから
  // 順に閉じる（FIFO）。ノート長を無視した完全な入れ子（重なり）は元々MIDI表現として一意に
  // 復元不可能なため対象外（そこまで奇妙な入力は想定しない）。
  const activeStacks = new Map<number, Array<{ startTick: number; velocity: number }>>();
  const raw: Array<{ startTick: number; endTick: number; pitch: number; velocity: number }> = [];
  for (const e of events) {
    if (e.kind !== 'noteOn' && e.kind !== 'noteOff') continue;
    if (e.channel !== channel) continue;
    if (e.kind === 'noteOn') {
      if (!activeStacks.has(e.note)) activeStacks.set(e.note, []);
      activeStacks.get(e.note)!.push({ startTick: e.tick, velocity: e.velocity });
    } else if (e.kind === 'noteOff') {
      const stack = activeStacks.get(e.note);
      if (!stack || stack.length === 0) continue;
      const start = stack.shift()!;
      raw.push({ startTick: start.startTick, endTick: e.tick, pitch: e.note, velocity: start.velocity });
    }
  }

  const quantized = raw
    .map((r) => {
      const startStep = Math.max(0, Math.round(r.startTick / ticksPerStep));
      const endStep = Math.max(startStep + 1, Math.round(r.endTick / ticksPerStep));
      return { startStep, lengthSteps: endStep - startStep, pitch: r.pitch, velocity: r.velocity };
    })
    .filter((n) => n.startStep < totalSteps && n.pitch >= minPitch && n.pitch <= maxPitch)
    .map((n) => ({ ...n, lengthSteps: Math.min(n.lengthSteps, totalSteps - n.startStep) }))
    .filter((n) => n.lengthSteps > 0);

  // 同じ音高の重なりを解消（先のノートを次の開始位置で打ち切る）
  const byPitch = new Map<number, typeof quantized>();
  for (const n of quantized) {
    if (!byPitch.has(n.pitch)) byPitch.set(n.pitch, []);
    byPitch.get(n.pitch)!.push(n);
  }
  const resolved: typeof quantized = [];
  for (const list of byPitch.values()) {
    list.sort((a, b) => a.startStep - b.startStep);
    for (let i = 0; i < list.length; i++) {
      const cur = list[i];
      const next = list[i + 1];
      const maxEnd = next ? next.startStep : Infinity;
      resolved.push({ ...cur, lengthSteps: Math.max(1, Math.min(cur.lengthSteps, maxEnd - cur.startStep)) });
    }
  }
  resolved.sort((a, b) => a.startStep - b.startStep || a.pitch - b.pitch);

  return resolved.map((n, i) => ({
    id: i + 1,
    startStep: n.startStep,
    lengthSteps: n.lengthSteps,
    pitch: n.pitch,
    level: velocityToLevel(n.velocity),
  }));
}

// ─────────────────────────────────────────────
// リズム: グリッド → SMFイベント（Export、1小節パターンをbars回繰り返す）
// ─────────────────────────────────────────────

export function rhythmRowsToEvents(
  rows: RhythmRowData[],
  channel: number,
  ticksPerStep: number,
  stepsPerBar: number,
  bars: number,
): SmfRawEvent[] {
  const events: SmfRawEvent[] = [];
  const ticksPerBar = ticksPerStep * stepsPerBar;
  for (let bar = 0; bar < bars; bar++) {
    for (const row of rows) {
      for (let step = 0; step < stepsPerBar; step++) {
        const level = row.steps[step];
        if (!level) continue;
        const tick = bar * ticksPerBar + step * ticksPerStep;
        events.push(noteOnEvent(tick, channel, row.note, levelToVelocity(level)));
        events.push(noteOffEvent(tick + ticksPerStep, channel, row.note));
      }
    }
  }
  return events;
}

// ─────────────────────────────────────────────
// リズム: SMFイベント → グリッド（Import、最多出現の1小節パターンへ畳む）
// ─────────────────────────────────────────────

function barPatternKey(stepsByNote: Map<number, Map<number, number>>): string {
  return [...stepsByNote.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([note, stepVel]) => {
      const steps = [...stepVel.entries()].sort((a, b) => a[0] - b[0]).map(([s, v]) => `${s}:${v}`).join(',');
      return `${note}=[${steps}]`;
    })
    .join('|');
}

/**
 * @param events parseSmf()の出力
 * @returns 該当ヒットが無ければ空配列
 */
export function midiEventsToRhythmRows(events: SmfEvent[], channel: number, ticksPerStep: number, stepsPerBar: number): RhythmRowData[] {
  const hits = events.filter((e): e is Extract<SmfEvent, { kind: 'noteOn' }> => e.kind === 'noteOn' && e.channel === channel);
  if (hits.length === 0) return [];

  const ticksPerBar = ticksPerStep * stepsPerBar;
  const maxTick = Math.max(...hits.map((e) => e.tick));
  const barCount = Math.floor(maxTick / ticksPerBar) + 1;

  const barPatterns: Array<Map<number, Map<number, number>>> = Array.from({ length: barCount }, () => new Map());
  for (const e of hits) {
    const bar = Math.floor(e.tick / ticksPerBar);
    const localTick = e.tick - bar * ticksPerBar;
    const step = Math.round(localTick / ticksPerStep) % stepsPerBar;
    if (!barPatterns[bar].has(e.note)) barPatterns[bar].set(e.note, new Map());
    barPatterns[bar].get(e.note)!.set(step, e.velocity);
  }

  // 最多出現パターンを選ぶ（空小節は除外、同数なら先に出現した小節を採用）
  const counts = new Map<string, { count: number; firstIdx: number; pattern: Map<number, Map<number, number>> }>();
  barPatterns.forEach((pattern, idx) => {
    if (pattern.size === 0) return;
    const key = barPatternKey(pattern);
    if (!counts.has(key)) counts.set(key, { count: 0, firstIdx: idx, pattern });
    counts.get(key)!.count += 1;
  });
  if (counts.size === 0) return [];

  let best: { count: number; firstIdx: number; pattern: Map<number, Map<number, number>> } | null = null;
  for (const entry of counts.values()) {
    if (!best || entry.count > best.count || (entry.count === best.count && entry.firstIdx < best.firstIdx)) {
      best = entry;
    }
  }

  const notes = [...best!.pattern.keys()].sort((a, b) => a - b);
  return notes.map((note) => {
    const steps = new Array(stepsPerBar).fill(0);
    for (const [step, velocity] of best!.pattern.get(note)!) {
      steps[step] = velocityToLevel(velocity);
    }
    return { note, steps };
  });
}
