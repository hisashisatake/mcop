// リズムグリッド/メロディノートとSMFイベント列の相互変換。純粋関数のみ（smf.jsと同じ方針）。
// gesture-appのグリッドは固定分割（メロディ=8分音符単位、リズム=16分音符単位）のため、
// Export（グリッド→SMF）は情報を失わないが、Import（SMF→グリッド）は必ず量子化・丸めを伴う。
// 詳細な設計判断（取込チャンネルの選び方・複数小節の畳み方等）はmemory
// `project_gesture_app_melody_screen_and_file_menu_plan.md`「フェーズ3」節参照。

import { noteOnEvent, noteOffEvent } from './smf.ts';
import { snapRound } from './grid-units.ts';
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
 * @param quantizePulses 開始/終了位置を丸めるグリッド幅（パルス単位、既定6＝16分音符）。
 *   人間の演奏はグリッドぴったりに乗らないため、ticksPerStep基準の生パルスをそのまま使うと
 *   startStep/lengthStepsが1パルス単位でばらつく（見た目のスナップと一致しない）。
 */
export function midiEventsToMelodyNotes(
  events: SmfEvent[],
  channel: number,
  ticksPerStep: number,
  totalSteps: number,
  minPitch: number,
  maxPitch: number,
  quantizePulses: number = 6,
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
      const startStep = Math.max(0, snapRound(r.startTick / ticksPerStep, quantizePulses));
      const endStep = Math.max(startStep + quantizePulses, snapRound(r.endTick / ticksPerStep, quantizePulses));
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
// リズム: グリッド → SMFイベント（Export、情報を失わない）
// ─────────────────────────────────────────────

/** `row.steps`の長さぶん（RHYTHM/MELODY共通の8小節タイムライン全体）をそのまま書き出す。 */
export function rhythmRowsToEvents(rows: RhythmRowData[], channel: number, ticksPerStep: number): SmfRawEvent[] {
  const events: SmfRawEvent[] = [];
  for (const row of rows) {
    for (let step = 0; step < row.steps.length; step++) {
      const level = row.steps[step];
      if (!level) continue;
      const tick = step * ticksPerStep;
      events.push(noteOnEvent(tick, channel, row.note, levelToVelocity(level)));
      events.push(noteOffEvent(tick + ticksPerStep, channel, row.note));
    }
  }
  return events;
}

// ─────────────────────────────────────────────
// リズム: SMFイベント → グリッド（Import、8小節タイムラインへそのまま取り込む）
// ─────────────────────────────────────────────

/**
 * @param events parseSmf()の出力
 * @param totalSteps タイムライン全体のパルス数（RHYTHM/MELODY共通の8小節=768）
 * @param quantizePulses ヒット位置を丸めるグリッド幅（パルス単位、既定6＝16分音符）。
 *   人間の演奏はグリッドぴったりに乗らないため量子化する（melodyと同じ理由）。
 * @returns 該当ヒットが無ければ空配列
 */
export function midiEventsToRhythmRows(events: SmfEvent[], channel: number, ticksPerStep: number, totalSteps: number, quantizePulses: number = 6): RhythmRowData[] {
  const hits = events.filter((e): e is Extract<SmfEvent, { kind: 'noteOn' }> => e.kind === 'noteOn' && e.channel === channel);
  if (hits.length === 0) return [];

  const byNote = new Map<number, Map<number, number>>();
  for (const e of hits) {
    const step = snapRound(e.tick / ticksPerStep, quantizePulses);
    if (step < 0 || step >= totalSteps) continue;
    if (!byNote.has(e.note)) byNote.set(e.note, new Map());
    byNote.get(e.note)!.set(step, e.velocity);
  }
  if (byNote.size === 0) return [];

  const notes = [...byNote.keys()].sort((a, b) => a - b);
  return notes.map((note) => {
    const steps = new Array(totalSteps).fill(0);
    for (const [step, velocity] of byNote.get(note)!) {
      steps[step] = velocityToLevel(velocity);
    }
    return { note, steps };
  });
}
