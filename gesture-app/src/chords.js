// コード画面（フロー方式）に割り当てるコード定義と、音楽的な計算。
//
// 行（縦）= コードの種類。中心行から外側へ行くほど緊張が強い。
//            上方向がメジャー系→ドミナント系、下方向がマイナー系→ディミニッシュ系。
// 半音オフセット（chordFromSemitone引数）= トニックからの音程。負値は下方向。
//
// 修飾キーで4レイヤーを切り替える:
//   なし        … トライアド中心（上=ドミナント系テンション、下=ディミニッシュ系）
//   Shift       … 4和音中心（中心行自体がmaj7になり、V7が中心の真上へ来る。ジャズ進行用）
//   Ctrl        … sus・付加音系（3度が曖昧な和音。上=解決したがる緊張、下=甘く静か）
//   Ctrl+Shift  … aug・オルタード系（5度の変化／オルタードテンション。上=ドミナント側、下=マイナー・減系）
//
// sus4/augは3度・5度そのものを書き換える和音なので、7th/9thを積む既存2レイヤーの縦軸には
// 構造的に乗らない（詳細はplan「gesture-app フェーズ2」参照）。
export const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];

/** 通常レイヤー（トライアド中心）。配列順=画面の上から下。 */
export const NORMAL_LAYER = [
  { row: 4, suffix: '13', intervals: [0, 4, 7, 10, 14, 21], family: 'dom' },
  { row: 3, suffix: '7', intervals: [0, 4, 7, 10], family: 'dom' },
  { row: 2, suffix: 'maj9', intervals: [0, 4, 7, 11, 14], family: 'maj' },
  { row: 1, suffix: 'maj7', intervals: [0, 4, 7, 11], family: 'maj' },
  { row: 0, suffix: '', intervals: [0, 4, 7], family: 'maj' },
  { row: -1, suffix: 'm', intervals: [0, 3, 7], family: 'min' },
  { row: -2, suffix: 'm7', intervals: [0, 3, 7, 10], family: 'min' },
  { row: -3, suffix: 'm9', intervals: [0, 3, 7, 10, 14], family: 'min' },
  { row: -4, suffix: 'm7b5', intervals: [0, 3, 6, 10], family: 'halfdim' },
];

/** Shiftレイヤー（4和音中心）。中心がmaj7、その真上がドミナント7th。 */
export const SHIFT_LAYER = [
  { row: 4, suffix: '7#9', intervals: [0, 4, 7, 10, 15], family: 'dom' },
  { row: 3, suffix: '13', intervals: [0, 4, 7, 10, 14, 21], family: 'dom' },
  { row: 2, suffix: '9', intervals: [0, 4, 7, 10, 14], family: 'dom' },
  { row: 1, suffix: '7', intervals: [0, 4, 7, 10], family: 'dom' },
  { row: 0, suffix: 'maj7', intervals: [0, 4, 7, 11], family: 'maj' },
  { row: -1, suffix: 'm7', intervals: [0, 3, 7, 10], family: 'min' },
  { row: -2, suffix: 'm9', intervals: [0, 3, 7, 10, 14], family: 'min' },
  { row: -3, suffix: 'm7b5', intervals: [0, 3, 6, 10], family: 'halfdim' },
  { row: -4, suffix: 'dim7', intervals: [0, 3, 6, 9], family: 'dim' },
];

/** Ctrlレイヤー（sus・付加音系）。中心がsus4で、上=解決したがる緊張／下=甘く静か。 */
export const CTRL_LAYER = [
  { row: 4, suffix: '7sus4b9', intervals: [0, 5, 7, 10, 13], family: 'sus' },
  { row: 3, suffix: '13sus4', intervals: [0, 5, 7, 10, 14, 21], family: 'sus' },
  { row: 2, suffix: '9sus4', intervals: [0, 5, 7, 10, 14], family: 'sus' },
  { row: 1, suffix: '7sus4', intervals: [0, 5, 7, 10], family: 'sus' },
  { row: 0, suffix: 'sus4', intervals: [0, 5, 7], family: 'sus' },
  { row: -1, suffix: 'sus2', intervals: [0, 2, 7], family: 'sus' },
  { row: -2, suffix: 'add9', intervals: [0, 4, 7, 14], family: 'six' },
  { row: -3, suffix: '6', intervals: [0, 4, 7, 9], family: 'six' },
  { row: -4, suffix: 'm6', intervals: [0, 3, 7, 9], family: 'msix' },
];

/** Ctrl+Shiftレイヤー（aug・オルタード系）。中心がaug、上=ドミナント側／下=マイナー・減系。 */
export const CTRL_SHIFT_LAYER = [
  { row: 4, suffix: '7#9#5', intervals: [0, 4, 8, 10, 15], family: 'aug' },
  { row: 3, suffix: '7#5', intervals: [0, 4, 8, 10], family: 'aug' },
  { row: 2, suffix: '7b9', intervals: [0, 4, 7, 10, 13], family: 'aug' },
  { row: 1, suffix: '7b5', intervals: [0, 4, 6, 10], family: 'aug' },
  { row: 0, suffix: 'aug', intervals: [0, 4, 8], family: 'aug' },
  { row: -1, suffix: 'maj7#5', intervals: [0, 4, 8, 11], family: 'aug' },
  { row: -2, suffix: 'mMaj7', intervals: [0, 3, 7, 11], family: 'mmaj' },
  { row: -3, suffix: 'dim', intervals: [0, 3, 6], family: 'dim' },
  { row: -4, suffix: 'dim7', intervals: [0, 3, 6, 9], family: 'dim' },
];

export const ROWS = NORMAL_LAYER.length; // 9

/** 起点となるルート音（MIDIノート番号）。調を変えるとこの値が動く。 */
export const DEFAULT_TONIC_MIDI = 60;

/** 修飾キーの状態からレイヤー配列を選ぶ。 */
export function layerFor(mods) {
  const ctrlHeld = mods?.ctrlHeld ?? false;
  const shiftHeld = mods?.shiftHeld ?? false;
  if (ctrlHeld && shiftHeld) return CTRL_SHIFT_LAYER;
  if (ctrlHeld) return CTRL_LAYER;
  if (shiftHeld) return SHIFT_LAYER;
  return NORMAL_LAYER;
}

/**
 * 行インデックス（0=最上段）と修飾キー状態から、コード種類を引く。
 */
export function chordTypeAt(rowIndex, mods) {
  const layer = layerFor(mods);
  return layer[Math.max(0, Math.min(layer.length - 1, rowIndex))];
}

/**
 * トニックからの半音オフセットと行インデックスからコードを組み立てる。
 * @returns {{name: string, rootMidi: number, rootPc: number, family: string, intervals: number[], notes: number[]}}
 */
export function chordFromSemitone(semitone, rowIndex, { tonicMidi, shiftHeld, ctrlHeld }) {
  const type = chordTypeAt(rowIndex, { shiftHeld, ctrlHeld });
  const rootMidi = tonicMidi + semitone;
  const rootPc = ((rootMidi % 12) + 12) % 12;
  const rootName = NOTE_NAMES[rootPc];
  return {
    name: rootName + type.suffix,
    rootMidi,
    rootPc,
    family: type.family,
    intervals: type.intervals,
    notes: type.intervals.map((i) => rootMidi + i),
  };
}

// velocityFromCellYが返しうる範囲。過去/未来スロットのベロシティバー表示（chord-screen.js）が
// 同じ範囲で正規化するため、ここでエクスポートして共有する。
export const VELOCITY_MIN = 40;
export const VELOCITY_MAX = 127;

/**
 * セル内縦位置（0=上端, 1=下端）をベロシティへ変換する。
 * 上下いっぱいにすると無音になってしまうため上限を設ける。
 */
export function velocityFromCellY(ratio) {
  const t = 1 - Math.max(0, Math.min(1, ratio));
  return Math.round(VELOCITY_MIN + (VELOCITY_MAX - VELOCITY_MIN) * t);
}
