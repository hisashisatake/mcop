// コード画面の調・候補設定（表示用の小さな状態）。Svelteコンポーネントから直接読める
// ようにするための置き場（段階C）。実際の値の変更・副作用（発音停止・履歴リセット等）は
// chord-screen.tsが持つ関数（setTonicPitchClass等）経由で行う。history（コード履歴、
// 大きな配列を含む）はここには置かない（[[project_gesture_app_svelte_migration_consult]]
// 参照、Undo/Redoの不変スナップショット前提とSvelteの深いプロキシが食い違うため）。

import type { Mode } from './types.ts';

export interface ChordSettings {
  tonicMidi: number;
  mode: Mode;
  rows: number;
  cols: number;
  autoVoicing: boolean;
  baseOctave: number;
  altHeld: boolean; // 押している間だけ自動転回ON/OFFを反転する一時トグルの実際の押下状態
}

export const chordSettings: ChordSettings = $state({
  tonicMidi: 60,
  mode: 'major',
  rows: 7,
  cols: 3,
  autoVoicing: false,
  baseOctave: 0,
  altHeld: false,
});
