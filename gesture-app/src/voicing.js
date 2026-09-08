// コード（ピッチクラス＋intervals）を、実際に鳴らすMIDIノート配列へ変換する（ボイシング計算）。
// theory.js/chords.js/chord-flow.jsと同じく、DOM・MIDI・Canvasのいずれにも触れない純粋関数のみを置く。
//
// コアトーン（ルート・3度・5度・7度など、interval<12）だけを転回（回転）の対象にし、
// テンション（9th/13th等、interval>=12）は常に「基準ルートの上」に固定した上部構造として扱う。
// テンションまで回転させると、回転のたびにコアの上へ押し出されて極端な高音になり、
// 音域が際限なく広がってしまうため。

export const MIN_MIDI = 0;
export const MAX_MIDI = 127;

const MIN_OCTAVE = 1; // baseRoot探索の下限（chord.rootPc + 12*1 = 12〜23付近）
const MAX_OCTAVE = 8; // baseRoot探索の上限（chord.rootPc + 12*8 = 96〜107付近）
const REGISTER_WEIGHT = 0.15; // 音域アンカー（centerMidiへの近さ）の重み。声部移動コストに対する相対的な強さ
const ROTATION_PENALTY = 0.01; // 同コストなら基本形（回転なし）を優先するタイブレーク

/** 直前のボイシングとの声部移動コスト（新しい各音を、直前ボイシング中の最も近い音へ寄せた距離の平均）。 */
function voiceLeadingCost(notes, previousNotes) {
  if (!previousNotes || previousNotes.length === 0) return 0;
  let total = 0;
  for (const n of notes) {
    let best = Infinity;
    for (const p of previousNotes) {
      const d = Math.abs(n - p);
      if (d < best) best = d;
    }
    total += best;
  }
  return total / notes.length;
}

function centroid(notes) {
  return notes.reduce((a, b) => a + b, 0) / notes.length;
}

/**
 * コードの構成音を、直前のボイシングに一番近い転回形・オクターブで実際に鳴らすMIDIノート配列へ変換する。
 * @param {{rootPc: number, intervals: number[]}} chord
 * @param {{previousNotes?: number[], centerMidi?: number, requireRootInBass?: boolean}} [opts]
 *   previousNotes: 直前に鳴らしたボイシング（無ければ音域アンカーのみで決まる）
 *   centerMidi: 音域アンカー（基準オクターブ設定から算出、既定は中央ド=60）
 *   requireRootInBass: trueなら転回（k>0）を候補から外し、根音だけをバスへ強制する
 *     （ドミナント→トニック等の強進行で、移動量最小化のあまりバスが根音へ着地しない
 *     のを防ぐ。theory.jsのisStrongResolution参照）
 * @returns {number[]} 昇順ソート済みのMIDIノート番号配列
 */
export function voiceChord(chord, { previousNotes = [], centerMidi = 60, requireRootInBass = false } = {}) {
  const core = chord.intervals.filter((i) => i < 12);
  const tensions = chord.intervals.filter((i) => i >= 12);

  let bestNotes = null;
  let bestCost = Infinity;

  const maxK = requireRootInBass ? 1 : core.length;
  for (let k = 0; k < maxK; k++) {
    // 先頭からk個（=音程が低い側からk個）を1オクターブ上げる＝k回目の転回形
    const rotatedCore = core.map((c, idx) => (idx < k ? c + 12 : c));
    for (let octave = MIN_OCTAVE; octave <= MAX_OCTAVE; octave++) {
      const baseRoot = chord.rootPc + 12 * octave;
      const notes = [...rotatedCore, ...tensions].map((o) => baseRoot + o).sort((a, b) => a - b);
      if (notes[0] < MIN_MIDI || notes[notes.length - 1] > MAX_MIDI) continue;

      const cost =
        voiceLeadingCost(notes, previousNotes) +
        REGISTER_WEIGHT * Math.abs(centroid(notes) - centerMidi) +
        ROTATION_PENALTY * k;

      if (cost < bestCost) {
        bestCost = cost;
        bestNotes = notes;
      }
    }
  }

  return bestNotes;
}

/**
 * 自動転回OFF時（従来方式）のボイシング：ルートの上にintervalsをそのまま積む。
 * baseOctaveは基準オクターブの手動±調整（1=+12半音、-1=-12半音）。
 * @param {{rootMidi: number, intervals: number[]}} chord
 * @param {number} [baseOctave]
 * @returns {number[]}
 */
export function rawVoicing(chord, baseOctave = 0) {
  return chord.intervals.map((i) => chord.rootMidi + 12 * baseOctave + i).sort((a, b) => a - b);
}
