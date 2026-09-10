// 定番コード進行（王道進行・ジャズスタンダード等）のテンプレート集と、演奏履歴とのマッチング判定。
// theory.js/chords.js/chord-flow.jsと同じく、DOM・MIDI・Canvasのいずれにも触れない純粋関数のみを置く。
//
// スコアリング(theory.js)には一切手を入れず、別レイヤーの「進行テンプレート」として実装する
// 方針にした経緯: セカンダリードミナント判定の拡張＋到達性フィルタ緩和でも王道進行等の
// 全リンクをGREEN化できることは実測で確認したが、緑の密度が20%→33%へ増え「緑=良い進行」の
// 選別としての意味が薄まるため、ユーザー判断で不採用。度数列の一致検出という形にした。
//
// 度数(degree)はキーのトニックからの半音数(0-11)。familiesはtheory.jsのnormalizeFamily()後の
// 値('maj'|'dom'|'min'|'halfdim'|'dim')の配列で、いずれかに一致すればそのステップにマッチする。
// line-cliche（半音下降ラインクリシェ）だけはfamiliesで区別が付かない(全ステップ度数0・min系)ため、
// familiesの代わりにsuffixes（chord.suffixの完全一致）を使う。

/** @typedef {{degree: number, families?: string[], suffixes?: string[]}} ProgressionStep */
/**
 * @typedef {{id: string, name: string, mode: 'major'|'minor', cyclic: boolean, steps: ProgressionStep[], minMatch?: number}} Progression
 *   minMatch: このテンプレートだけに適用するmatchProgressions()の最小一致手数の上書き
 *   （省略時はmatchProgressions()呼び出し側の既定値=2を使う）。
 */

/** @type {Progression[]} */
export const PROGRESSIONS = [
  // ─────────────────────────────────────────────
  // ジャズ寄り(12)
  // ─────────────────────────────────────────────
  {
    id: 'ii-v-i',
    name: 'ii-V-I',
    mode: 'major',
    cyclic: false,
    steps: [
      { degree: 2, families: ['min'] },
      { degree: 7, families: ['dom'] },
      { degree: 0, families: ['maj'] },
    ],
  },
  {
    id: 'minor-ii-v-i',
    name: 'マイナーii-V-i',
    mode: 'minor',
    cyclic: false,
    steps: [
      { degree: 2, families: ['halfdim'] },
      { degree: 7, families: ['dom'] },
      { degree: 0, families: ['min'] },
    ],
  },
  {
    id: 'backdoor',
    name: 'Backdoor進行',
    mode: 'major',
    cyclic: false,
    steps: [
      { degree: 5, families: ['min'] },
      { degree: 10, families: ['dom'] },
      { degree: 0, families: ['maj'] },
    ],
  },
  {
    id: 'circle',
    name: 'Circle進行',
    mode: 'major',
    cyclic: false,
    steps: [
      { degree: 4, families: ['min'] },
      { degree: 9, families: ['min'] },
      { degree: 2, families: ['min'] },
      { degree: 7, families: ['dom', 'maj'] },
      { degree: 0, families: ['maj'] },
    ],
  },
  {
    id: 'just-two-of-us',
    name: 'Just the Two of Us進行',
    mode: 'major',
    cyclic: false,
    // 1手目(IVmaj7)自体はダイアトニックで見つけやすいが、2手目(III7)がまさに元々の
    // 「見えにくい」問題の張本人。既定のminMatch=2だと1→2手目のリンクだけは
    // 事前案内できない(1手だけでは一致長が閾値に届かないため)ので、このテンプレートに
    // 限りminMatch=1にして1手目から案内できるようにする。
    minMatch: 1,
    steps: [
      { degree: 5, families: ['maj'] },
      { degree: 4, families: ['dom'] },
      { degree: 9, families: ['min'] },
      { degree: 2, families: ['min'] },
      { degree: 7, families: ['dom'] },
    ],
  },
  {
    id: '1625',
    name: '1625（イチロクニーゴー）',
    mode: 'major',
    cyclic: true,
    // 度数9・2はセカンダリードミナント化(I-VI7-II7-V)も同じ枠のバリエーションとして許容する
    steps: [
      { degree: 0, families: ['maj'] },
      { degree: 9, families: ['min', 'dom', 'maj'] },
      { degree: 2, families: ['min', 'dom', 'maj'] },
      { degree: 7, families: ['dom', 'maj'] },
    ],
  },
  {
    id: 'blues12',
    name: '12小節ブルース',
    mode: 'major',
    cyclic: true,
    steps: [0, 0, 0, 0, 5, 5, 0, 0, 7, 5, 0, 7].map((degree) => ({ degree, families: ['dom'] })),
  },
  {
    id: 'rhythm-bridge',
    name: 'Rhythm Changesのブリッジ',
    mode: 'major',
    cyclic: false,
    steps: [
      { degree: 4, families: ['dom'] },
      { degree: 9, families: ['dom'] },
      { degree: 2, families: ['dom'] },
      { degree: 7, families: ['dom'] },
    ],
  },
  {
    id: 'coltrane',
    name: 'Coltrane Changes',
    mode: 'major',
    cyclic: true,
    // Giant Stepsの1サイクル分（長3度ずつ移調する3トニックシステム）。
    // 各トニックへドミナント7thで進入する: 0(maj)→3(dom,V/8)→8(maj)→11(dom,V/4)→4(maj)→7(dom,V/0)→(cyclic)0
    steps: [
      { degree: 0, families: ['maj'] },
      { degree: 3, families: ['dom'] },
      { degree: 8, families: ['maj'] },
      { degree: 11, families: ['dom'] },
      { degree: 4, families: ['maj'] },
      { degree: 7, families: ['dom'] },
    ],
  },
  {
    id: 'ladybird',
    name: 'Ladybird changes(トライトーン代理)',
    mode: 'major',
    cyclic: true,
    steps: [
      { degree: 0, families: ['maj'] },
      { degree: 3, families: ['dom'] },
      { degree: 8, families: ['dom'] },
      { degree: 1, families: ['dom'] },
    ],
  },
  {
    id: 'honeysuckle-bridge',
    name: 'Honeysuckle Roseブリッジ',
    mode: 'major',
    cyclic: false,
    steps: [
      { degree: 0, families: ['dom'] },
      { degree: 5, families: ['dom'] },
      { degree: 2, families: ['min'] },
      { degree: 7, families: ['dom'] },
    ],
  },
  {
    id: 'passing-dim',
    name: 'パッシングディミニッシュ・ターンアラウンド',
    mode: 'major',
    cyclic: false,
    steps: [
      { degree: 0, families: ['maj'] },
      { degree: 1, families: ['dim'] },
      { degree: 2, families: ['min'] },
      { degree: 7, families: ['dom'] },
    ],
  },

  // ─────────────────────────────────────────────
  // それ以外(8)
  // ─────────────────────────────────────────────
  {
    id: 'oudou-4536',
    name: '王道進行(4536)',
    mode: 'major',
    cyclic: true,
    steps: [
      { degree: 5, families: ['maj'] },
      { degree: 7, families: ['dom', 'maj'] },
      { degree: 4, families: ['min'] },
      { degree: 9, families: ['min'] },
    ],
  },
  {
    id: 'canon',
    name: 'カノン進行',
    mode: 'major',
    cyclic: false,
    steps: [
      { degree: 0, families: ['maj'] },
      { degree: 7, families: ['dom', 'maj'] },
      { degree: 9, families: ['min'] },
      { degree: 4, families: ['min'] },
      { degree: 5, families: ['maj'] },
      { degree: 0, families: ['maj'] },
      { degree: 5, families: ['maj'] },
      { degree: 7, families: ['dom', 'maj'] },
    ],
  },
  {
    id: 'komuro',
    name: '小室進行',
    mode: 'major',
    cyclic: true,
    steps: [
      { degree: 9, families: ['min'] },
      { degree: 5, families: ['maj'] },
      { degree: 7, families: ['dom', 'maj'] },
      { degree: 0, families: ['maj'] },
    ],
  },
  {
    id: '50s',
    name: '50sプログレッション',
    mode: 'major',
    cyclic: true,
    steps: [
      { degree: 0, families: ['maj'] },
      { degree: 9, families: ['min'] },
      { degree: 5, families: ['maj'] },
      { degree: 7, families: ['dom', 'maj'] },
    ],
  },
  {
    id: 'axis',
    name: 'Axis進行',
    mode: 'major',
    cyclic: true,
    steps: [
      { degree: 0, families: ['maj'] },
      { degree: 7, families: ['dom', 'maj'] },
      { degree: 9, families: ['min'] },
      { degree: 5, families: ['maj'] },
    ],
  },
  {
    id: 'andalusian',
    name: 'アンダルシア終止',
    mode: 'minor',
    cyclic: false,
    steps: [
      { degree: 0, families: ['min'] },
      { degree: 10, families: ['maj'] },
      { degree: 8, families: ['maj'] },
      { degree: 7, families: ['dom', 'min'] },
    ],
  },
  {
    id: 'aeolian-vamp',
    name: 'Aeolianヴァンプ',
    mode: 'minor',
    cyclic: true,
    steps: [
      { degree: 0, families: ['min'] },
      { degree: 10, families: ['maj'] },
      { degree: 8, families: ['maj'] },
    ],
  },
  {
    id: 'line-cliche',
    name: '半音下降ラインクリシェ',
    mode: 'minor',
    cyclic: false,
    steps: [
      { degree: 0, suffixes: ['m'] },
      { degree: 0, suffixes: ['mMaj7'] },
      { degree: 0, suffixes: ['m7'] },
      { degree: 0, suffixes: ['m6'] },
    ],
  },
];

/** @param {ProgressionStep} step @param {{degree: number, normFamily: string, suffix: string}} played */
function stepMatches(step, played) {
  if (step.degree !== played.degree) return false;
  if (step.suffixes) return step.suffixes.includes(played.suffix);
  return step.families.includes(played.normFamily);
}

/** recentTail(長さn、古い順)が、steps[endIndex-n+1..endIndex]と連続一致するか。 */
function windowMatches(recentTail, steps, endIndex, n) {
  const startIndex = endIndex - n + 1;
  if (startIndex < 0) return false; // テンプレート先頭より前へは巻き戻らない(cyclicでも同様)
  for (let i = 0; i < n; i++) {
    if (!stepMatches(steps[startIndex + i], recentTail[i])) return false;
  }
  return true;
}

/**
 * 直近の演奏履歴が、登録済みテンプレートのどれかと連続一致しているかを判定する。
 * @param {Array<{degree: number, normFamily: string, suffix: string}>} recent 古い順（末尾が直前のコード）
 * @param {'major'|'minor'} mode 現在のキーのモード（一致するテンプレートのみ対象）
 * @param {{minMatch?: number, maxResults?: number}} [opts] minMatchは既定値。テンプレート側にminMatchが
 *   指定されていればそちらを優先する（just-two-of-us等、詳細はPROGRESSIONS内のコメント参照）
 * @returns {Array<{id: string, name: string, matchedLength: number, position: number, total: number, next: ProgressionStep}>}
 *   一致した手数の長い順、最大maxResults件（次の一手が無い＝非cyclicで末尾到達したものは含まない）
 */
export function matchProgressions(recent, mode, { minMatch = 2, maxResults = Infinity } = {}) {
  const results = [];
  for (const prog of PROGRESSIONS) {
    if (prog.mode !== mode) continue;
    const steps = prog.steps;
    const progMinMatch = prog.minMatch ?? minMatch;
    let best = null; // { n, endIndex }
    for (let endIndex = 0; endIndex < steps.length; endIndex++) {
      const maxN = Math.min(recent.length, endIndex + 1);
      for (let n = maxN; n >= progMinMatch; n--) {
        const tail = recent.slice(recent.length - n);
        if (windowMatches(tail, steps, endIndex, n)) {
          if (!best || n > best.n) best = { n, endIndex };
          break; // このendIndexでの最長一致が見つかったので次のendIndexへ
        }
      }
    }
    if (!best) continue;

    let nextIndex = best.endIndex + 1;
    if (nextIndex >= steps.length) {
      if (!prog.cyclic) continue; // 末尾到達かつ非cyclicなら次の一手が無い
      nextIndex = 0;
    }

    results.push({
      id: prog.id,
      name: prog.name,
      matchedLength: best.n,
      position: best.endIndex + 1,
      total: steps.length,
      next: steps[nextIndex],
    });
  }
  results.sort((a, b) => b.matchedLength - a.matchedLength);
  return results.slice(0, maxResults);
}
