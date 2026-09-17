// グリッド解像度の共通定数・純粋関数（RHYTHM/MELODY画面共通）。
// tests/*.test.tsがnode --testで`src/*.ts`を直接importするため、依存ゼロを保つ
// （.svelte.tsやTauri呼び出しを含むモジュールに依存するとテストから読めなくなる）。
//
// 内部保持は全て1パルス=1/96小節に統一する（MIDI Clockは規格上24 PPQN固定、
// 1小節=24×4拍=96パルス）。スライダーの8段階はすべて整数パルスで、24が3で
// 割り切れるため3連符も正確に表現できる。

export const PULSES_PER_BEAT = 24;
export const PULSES_PER_BAR = 96;

/** RHYTHM/MELODY共通のシーケンス長（小節数）。両画面とも同じ長さのタイムラインを持つ。 */
export const SEQUENCE_BARS = 8;
export const SEQUENCE_TOTAL_PULSES = PULSES_PER_BAR * SEQUENCE_BARS; // 768

/** スライダー位置(0〜7、添字がそのままgridZoomStateの値)ごとの1マスのパルス数。 */
export const SNAP_PULSES = [24, 16, 12, 8, 6, 4, 3, 2];
export const SNAP_LABELS = ['1/4', '1/4T', '1/8', '1/8T', '1/16', '1/16T', '1/32', '1/32T'];

/** pulseを含むマスの開始パルスへ切り下げる。 */
export function snapFloor(pulse: number, snapPulses: number): number {
  return Math.floor(pulse / snapPulses) * snapPulses;
}

/** pulseに最も近いマス境界へ丸める。 */
export function snapRound(pulse: number, snapPulses: number): number {
  return Math.round(pulse / snapPulses) * snapPulses;
}

/** pulseを含むマスの次のマス境界へ切り上げる（pulseがちょうどマス境界のときはその位置自身）。 */
export function snapCeil(pulse: number, snapPulses: number): number {
  return Math.ceil(pulse / snapPulses) * snapPulses;
}

/** pulseが何番目のマスに属するかを返す（0起点）。 */
export function cellIndexOf(pulse: number, snapPulses: number): number {
  return Math.floor(pulse / snapPulses);
}

/** マス番号(0起点)の開始パルスを返す。 */
export function cellStartPulse(index: number, snapPulses: number): number {
  return index * snapPulses;
}

/**
 * 「見たまま＝鳴る」の読み取り: startから続くsnapPulses個ぶんのパルスのうち、
 * 最大のレベル値を返す（範囲内に打ち込みが1つも無ければ0）。RHYTHM画面が粗い倍率で
 * マスをクリックしたとき、そのマスが現在どう聞こえているかを1個の代表値として読む。
 */
export function cellLevel(steps: number[], start: number, snapPulses: number): number {
  let max = 0;
  for (let i = start; i < start + snapPulses && i < steps.length; i++) {
    if (steps[i] > max) max = steps[i];
  }
  return max;
}

/** pulseが拍頭（4分音符境界）かどうか。 */
export function isBeatHead(pulse: number): boolean {
  return pulse % PULSES_PER_BEAT === 0;
}

/** pulseが小節頭かどうか。 */
export function isBarHead(pulse: number): boolean {
  return pulse % PULSES_PER_BAR === 0;
}
