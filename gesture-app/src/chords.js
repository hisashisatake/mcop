// コード画面のグリッドに割り当てるコード定義と、音楽的な計算。
//
// グリッドの軸:
//   横（列） = 現在の調のトニックを中心とした半音単位の音程。右へ行くほど高い。
//   縦（行） = コードの種類。中心0行から外側へ行くほど緊張が強い。
//              上方向はメジャー系→ドミナント系、下方向はマイナー系→ディミニッシュ系。
//
// Shiftキーを押している間は「トライアドの世界」から「4和音の世界」へ切り替わる
// （中心行自体がmaj7になり、V7が中心の真上へ来る）。ドミナント7thを上方外側へ
// 置いた結果II-V-Iが縦に5行往復してしまう問題を、Shiftで距離を縮めて解消する
// ためのレイヤー。ジャズ進行はShiftを押しっぱなしで弾く運用になる。

export const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];

/** 通常レイヤー（トライアド中心）。配列順は画面の上から下。 */
export const NORMAL_LAYER = [
  { row: 4, suffix: '13', intervals: [0, 4, 7, 10, 14, 21] },
  { row: 3, suffix: '7', intervals: [0, 4, 7, 10] },
  { row: 2, suffix: 'maj9', intervals: [0, 4, 7, 11, 14] },
  { row: 1, suffix: 'maj7', intervals: [0, 4, 7, 11] },
  { row: 0, suffix: '', intervals: [0, 4, 7] },
  { row: -1, suffix: 'm', intervals: [0, 3, 7] },
  { row: -2, suffix: 'm7', intervals: [0, 3, 7, 10] },
  { row: -3, suffix: 'm9', intervals: [0, 3, 7, 10, 14] },
  { row: -4, suffix: 'm7b5', intervals: [0, 3, 6, 10] },
];

/** Shiftレイヤー（4和音中心）。中心がmaj7、その真上がドミナント7th。 */
export const SHIFT_LAYER = [
  { row: 4, suffix: '7#9', intervals: [0, 4, 7, 10, 15] },
  { row: 3, suffix: '13', intervals: [0, 4, 7, 10, 14, 21] },
  { row: 2, suffix: '9', intervals: [0, 4, 7, 10, 14] },
  { row: 1, suffix: '7', intervals: [0, 4, 7, 10] },
  { row: 0, suffix: 'maj7', intervals: [0, 4, 7, 11] },
  { row: -1, suffix: 'm7', intervals: [0, 3, 7, 10] },
  { row: -2, suffix: 'm9', intervals: [0, 3, 7, 10, 14] },
  { row: -3, suffix: 'm7b5', intervals: [0, 3, 6, 10] },
  { row: -4, suffix: 'dim7', intervals: [0, 3, 6, 9] },
];

export const ROWS = NORMAL_LAYER.length; // 9

/** 既定の列数（±6半音＝1オクターブ分）。可変にするためここは初期値でしかない。 */
export const DEFAULT_COLS = 13;

/** 中心列のルート音（C4）。調を変えるとこの値が動く。 */
export const DEFAULT_TONIC_MIDI = 60;

/**
 * 行インデックス（0が最上段）とShift状態から、コード種類を引く。
 */
export function chordTypeAt(rowIndex, shiftHeld) {
  const layer = shiftHeld ? SHIFT_LAYER : NORMAL_LAYER;
  return layer[Math.max(0, Math.min(layer.length - 1, rowIndex))];
}

/**
 * 列インデックス（0が最左）から、中心を0とする半音オフセットへ変換する。
 * 列数が偶数のときは中心が半端になるため、左寄りの列を中心とみなす。
 */
export function colToSemitone(colIndex, cols) {
  return colIndex - Math.floor(cols / 2);
}

/**
 * セル（列・行）からコードを組み立てる。
 * @returns {{name: string, rootMidi: number, notes: number[]}}
 */
export function chordAt(colIndex, rowIndex, { cols, tonicMidi, shiftHeld }) {
  const type = chordTypeAt(rowIndex, shiftHeld);
  const rootMidi = tonicMidi + colToSemitone(colIndex, cols);
  const rootName = NOTE_NAMES[((rootMidi % 12) + 12) % 12];
  return {
    name: rootName + type.suffix,
    rootMidi,
    notes: type.intervals.map((i) => rootMidi + i),
  };
}

/**
 * セル内の縦位置（0=上端, 1=下端）をベロシティへ変換する。
 * 上側ほど強い。0にすると無音になってしまうため下限を設ける。
 */
export function velocityFromCellY(ratio) {
  const MIN = 40;
  const MAX = 127;
  const t = 1 - Math.max(0, Math.min(1, ratio));
  return Math.round(MIN + (MAX - MIN) * t);
}
