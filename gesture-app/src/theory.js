// コード補助機能の音楽理論モジュール。egui・MIDI・DOMのいずれにも触れない純粋関数のみを置く。
//
// 型（JSDocのみ、TSは使わない）:
//   key   = { tonicPc: 0..11, mode: 'major' | 'minor' }
//   chord = { rootPc: 0..11, family: string, intervals: number[] }
//     family: 'maj' | 'dom' | 'min' | 'halfdim' | 'dim' | 'sus' | 'six' | 'msix' | 'aug' | 'mmaj'
//     intervals はルートからの半音オフセット（chords.jsのintervalsと同じ生の値。9th=14/13th=21等、
//     オクターブを跨いでも畳み込まない）。dissonancePenaltyがこの生の値の差を見る。

const MIN_SCORE = 0.6;

/** ダイアトニック表: 度数（トニックからの半音数）→ 許容されるfamily（正規化後）。 */
const DIATONIC_DEGREES = {
  major: { 0: ['maj'], 2: ['min'], 4: ['min'], 5: ['maj'], 7: ['dom', 'maj'], 9: ['min'], 11: ['halfdim'] },
  // 自然的短音階と和声的短音階を併用する（V7と v、bVIIと vii゜の両方を許容）
  minor: { 0: ['min'], 2: ['halfdim'], 3: ['maj'], 5: ['min'], 7: ['dom', 'min'], 8: ['maj'], 10: ['maj'], 11: ['dim'] },
};

/** 機能表: トニック(T)／サブドミナント(S)／ドミナント(D)の度数。 */
const FUNCTION_DEGREES = {
  major: { T: [0, 4, 9], S: [2, 5], D: [7, 11] },
  minor: { T: [0, 3], S: [2, 5, 8], D: [7, 10, 11] },
};

const TRANSITION_TABLE = {
  D: { T: 1.0, S: 0.3, D: 0.5 },
  S: { D: 0.92, T: 0.62, S: 0.5 },
  T: { S: 0.85, D: 0.8, T: 0.55 },
};
const DEFAULT_TRANSITION = 0.45; // 半音進行・借用・セカンダリードミナント等、度数がT/S/Dに乗らない場合

/** 根音進行（半音差、0-11）ごとのボーナス。表に無い差は0。 */
const ROOT_MOTION_BONUS = { 5: 0.15, 2: 0.06, 10: 0.06, 1: 0.05, 11: 0.05, 3: 0.04, 9: 0.04, 7: 0.02, 6: -0.05 };

/** ドミナント機能上のb9/短9度は許容されるテンションとして濁り減点を免除するfamily。 */
const DOMINANT_ISH_FAMILIES = new Set(['dom', 'aug', 'sus']);

const mod12 = (n) => ((n % 12) + 12) % 12;

function buildFunctionLookup(mode) {
  const table = {};
  for (const [fn, degrees] of Object.entries(FUNCTION_DEGREES[mode])) {
    for (const d of degrees) table[d] = fn;
  }
  return table;
}
const FUNCTION_LOOKUP = { major: buildFunctionLookup('major'), minor: buildFunctionLookup('minor') };

function hasSeventh(intervals) {
  return intervals.some((iv) => {
    const m = mod12(iv);
    return m === 10 || m === 11;
  });
}

/**
 * sus/6/add9/aug等の装飾的familyを、ダイアトニック判定・機能判定で使う
 * 基本5種（maj/dom/min/halfdim/dim）へ正規化する。
 * 新レイヤーの和音は大半が既存ダイアトニックコードの色替えなので、正規化しないと
 * 全て非ダイアトニック＝暗くなってしまう。
 */
export function normalizeFamily(chord, key) {
  const { family, intervals, rootPc } = chord;
  switch (family) {
    case 'six':
      return 'maj';
    case 'msix':
    case 'mmaj':
      return 'min';
    case 'aug':
      return 'dom'; // オルタードドミナント扱い
    case 'dim':
      return 'dim';
    case 'sus': {
      if (hasSeventh(intervals)) return 'dom';
      // 7thを含まないsus2/sus4は、その度数のダイアトニックfamilyへフォールバックする
      const degree = mod12(rootPc - key.tonicPc);
      const fams = DIATONIC_DEGREES[key.mode][degree];
      return fams ? fams[0] : null;
    }
    default:
      return family; // maj, dom, min, halfdim はそのまま機能クラス
  }
}

/** コードが現在の調にダイアトニックに属するか。 */
export function isDiatonic(chord, key) {
  const degree = mod12(chord.rootPc - key.tonicPc);
  const allowed = DIATONIC_DEGREES[key.mode][degree];
  if (!allowed) return false;
  const nf = normalizeFamily(chord, key);
  return nf != null && allowed.includes(nf);
}

export function isDiatonicInOppositeMode(chord, key) {
  const opposite = { tonicPc: key.tonicPc, mode: key.mode === 'major' ? 'minor' : 'major' };
  return isDiatonic(chord, opposite);
}

function diatonicRootDegrees(key) {
  return Object.keys(DIATONIC_DEGREES[key.mode]).map(Number);
}

function transitionBase(fromFn, toFn) {
  if (fromFn && toFn) {
    const row = TRANSITION_TABLE[fromFn];
    if (row && row[toFn] != null) return row[toFn];
  }
  return DEFAULT_TRANSITION;
}

/** 根音進行ボーナス。同根音でfamilyだけ変わる色替え（G7sus4→G7等）は常に自然な進行として加点する。 */
export function rootMotionBonus(fromChord, toChord) {
  const diff = mod12(toChord.rootPc - fromChord.rootPc);
  if (diff === 0) return fromChord.family === toChord.family ? -0.1 : 0.15;
  return ROOT_MOTION_BONUS[diff] ?? 0;
}

function pitchClassSet(chord) {
  return new Set(chord.intervals.map((iv) => mod12(chord.rootPc + iv)));
}

/** 起点コードとの共通ピッチクラス比によるボーナス。 */
export function commonToneBonus(fromChord, toChord) {
  const fromSet = pitchClassSet(fromChord);
  const toSet = pitchClassSet(toChord);
  let shared = 0;
  for (const pc of toSet) {
    if (fromSet.has(pc)) shared++;
  }
  return (shared / toSet.size) * 0.1;
}

function isAvoidException(chord, a, b, diff) {
  // ドミナント機能上のb9（ルートと短9度）は理論上許容されるテンションなので減点しない
  return diff === 13 && a === 0 && DOMINANT_ISH_FAMILIES.has(chord.family);
}

/**
 * コード構成音の濁り（半音衝突・短9度）を検出し減点する。avoid noteテーブルの
 * 例外（ドミナント上のb9等）は免除する。intervalsは生の値のまま（畳み込まない）。
 */
export function dissonancePenalty(chord) {
  const ivs = chord.intervals;
  let penalty = 0;
  for (let i = 0; i < ivs.length; i++) {
    for (let j = i + 1; j < ivs.length; j++) {
      const a = Math.min(ivs[i], ivs[j]);
      const b = Math.max(ivs[i], ivs[j]);
      const diff = b - a;
      if (diff === 1 && !isAvoidException(chord, a, b, diff)) penalty += 0.35;
      else if (diff === 13 && !isAvoidException(chord, a, b, diff)) penalty += 0.2;
    }
  }
  return penalty;
}

/**
 * fromChord→toChordがドミナント→トニックの強進行（V7→I等）かどうか。
 * voicing.jsの自動転回は移動量最小化しか見ておらず、この種の強進行でもバスが根音へ
 * 着地しない（例: G7→Cがバスg のままの第二転回形になる）ことがあるため、該当時は
 * 呼び出し側（chord-screen.js）でrequireRootInBassを立てて着地を強制する。
 */
export function isStrongResolution(fromChord, toChord, key) {
  const fromDegree = mod12(fromChord.rootPc - key.tonicPc);
  const toDegree = mod12(toChord.rootPc - key.tonicPc);
  const fnLookup = FUNCTION_LOOKUP[key.mode];
  const fromFn = fnLookup[fromDegree] ?? null;
  const toFn = fnLookup[toDegree] ?? null;
  return fromFn === 'D' && toFn === 'T';
}

/**
 * fromChord→toChordの進行を採点し、緑/黄/消灯を判定する。
 * @returns {{score: number, category: 'GREEN' | 'YELLOW' | null}}
 */
export function classifyProgression(fromChord, toChord, key) {
  const fromDegree = mod12(fromChord.rootPc - key.tonicPc);
  const toDegree = mod12(toChord.rootPc - key.tonicPc);
  const fnLookup = FUNCTION_LOOKUP[key.mode];
  const fromFn = fnLookup[fromDegree] ?? null;
  const toFn = fnLookup[toDegree] ?? null;

  let score = transitionBase(fromFn, toFn);
  score += rootMotionBonus(fromChord, toChord);
  score += commonToneBonus(fromChord, toChord);
  score -= dissonancePenalty(toChord);

  const toNormFamily = normalizeFamily(toChord, key);
  const rootDegrees = diatonicRootDegrees(key);
  const vDegree = 7; // 属音は長短どちらの調でもトニックの7半音上
  const toIsDiatonic = isDiatonic(toChord, key);

  // 「機能的に隣接」の判定: 解決先の度数が現在の調のトニックそのもの、または
  // fromChordから見て強い機能移動（T→S/T→D/S→D/D→T、しきい値0.80）で届く度数のときだけ、
  // ドミナント系の代理コード（セカンダリードミナント・裏コード）を隣接候補として認める。
  // これが無いと「根音+5(または-1)がダイアトニック根音」という条件だけで調内の
  // ほぼ全ての半音位置がヒットしてしまう（実測で117セル中53%が点灯した）。
  const STRONG_MOVE_THRESHOLD = 0.8;
  function isReachableDegree(targetDegree) {
    if (targetDegree === 0) return true; // トニックへの解決は常に強い
    const targetFn = fnLookup[targetDegree] ?? null;
    return transitionBase(fromFn, targetFn) >= STRONG_MOVE_THRESHOLD;
  }

  const secondaryDominantTarget = mod12(toDegree + 5);
  const isSecondaryDominant =
    toNormFamily === 'dom' &&
    toDegree !== vDegree &&
    rootDegrees.includes(secondaryDominantTarget) &&
    isReachableDegree(secondaryDominantTarget);

  const tritoneSubTarget = mod12(toDegree - 1);
  const isTritoneSub = toNormFamily === 'dom' && rootDegrees.includes(tritoneSubTarget) && isReachableDegree(tritoneSubTarget);

  const isModalBorrow = !toIsDiatonic && isDiatonicInOppositeMode(toChord, key);

  // パッシングディミニッシュは「半音上がダイアトニックコードへ解決し、かつその解決先が
  // 機能的に隣接している」場合に限定する。ただし「直前のコードの半音上」という
  // 典型的な経過和音の形そのものであれば、isReachableDegreeの機能的隣接チェックを
  // 免除する（例: Dm→D#dim→Emはfrom=S/to=Tの遷移が0.62でしきい値0.80に届かず
  // 取りこぼしていたが、形そのものがパッシングディミニッシュとして自明なため）
  const passingDiminishedTarget = mod12(toDegree + 1);
  const isChromaticApproachFromCurrent = mod12(toDegree - fromDegree) === 1;
  const isPassingDiminished =
    toNormFamily === 'dim' &&
    rootDegrees.includes(passingDiminishedTarget) &&
    (isReachableDegree(passingDiminishedTarget) || isChromaticApproachFromCurrent);

  const isDominantDegree = toDegree === vDegree || isSecondaryDominant;
  // augレイヤー由来（7#9#5/7#5/7b9/7b5/aug/maj7#5）は、ドミナント機能の度数に
  // 乗るときだけGREEN、それ以外はYELLOW止まりにする
  const isAlteredOnDominantDegree = toChord.family === 'aug' && isDominantDegree;
  const isAlteredOffDominantDegree = toChord.family === 'aug' && !isDominantDegree;

  let category = null;
  if (toIsDiatonic || isSecondaryDominant || isAlteredOnDominantDegree) {
    category = 'GREEN';
  } else if (isModalBorrow || isTritoneSub || isPassingDiminished || isAlteredOffDominantDegree) {
    category = 'YELLOW';
  }

  // 特定カテゴリに該当するコードへは、ベーススコアだけでは基準を満たせないことがあるため
  // ボーナスを与える（複数該当してもmaxのみ採用し、積み上げで暴走させない）
  const bonusCandidates = [0];
  if (isSecondaryDominant) bonusCandidates.push(0.08);
  if (isTritoneSub) bonusCandidates.push(0.15);
  if (isModalBorrow) bonusCandidates.push(0.18);
  if (isAlteredOnDominantDegree) bonusCandidates.push(0.12);
  if (isAlteredOffDominantDegree) bonusCandidates.push(0.12);
  if (isPassingDiminished) bonusCandidates.push(0.15);
  score += Math.max(...bonusCandidates);

  if (score < MIN_SCORE) category = null;

  return { score, category };
}

/** 直前コードが無い1手目用の度数重要度（トニック＞属＞下属＞…）。GREENの並び順に使う。 */
const INITIAL_DEGREE_SCORE = {
  major: { 0: 1.0, 7: 0.9, 5: 0.85, 9: 0.6, 2: 0.55, 4: 0.5, 11: 0.4 },
  minor: { 0: 1.0, 7: 0.9, 5: 0.85, 3: 0.6, 8: 0.55, 10: 0.5, 2: 0.45, 11: 0.4 },
};
const INITIAL_DEFAULT_SCORE = 0.5;
const INITIAL_BORROW_SCORE = 0.5;

/**
 * 直前に鳴らしたコードが無い1手目用の採点。classifyProgressionと違い遷移元が無いため、
 * ダイアトニックなら度数の重要度で、同主調からの借用ならYELLOW固定スコアで評価する。
 * @returns {{score: number, category: 'GREEN' | 'YELLOW' | null}}
 */
export function classifyInitial(chord, key) {
  const degree = mod12(chord.rootPc - key.tonicPc);
  let category = null;
  let score = INITIAL_DEFAULT_SCORE;
  if (isDiatonic(chord, key)) {
    category = 'GREEN';
    score = INITIAL_DEGREE_SCORE[key.mode][degree] ?? INITIAL_DEFAULT_SCORE;
  } else if (isDiatonicInOppositeMode(chord, key)) {
    category = 'YELLOW';
    score = INITIAL_BORROW_SCORE;
  }
  score -= dissonancePenalty(chord);
  return { score, category };
}

/** 現在の調から見た近親調4種（属調・下属調・平行調・同主調）。 */
export function relatedKeys(key) {
  const { tonicPc, mode } = key;
  if (mode === 'major') {
    return [
      { tonicPc: mod12(tonicPc + 7), mode: 'major', role: 'dominant' },
      { tonicPc: mod12(tonicPc + 5), mode: 'major', role: 'subdominant' },
      { tonicPc: mod12(tonicPc + 9), mode: 'minor', role: 'relative' },
      { tonicPc, mode: 'minor', role: 'parallel' },
    ];
  }
  return [
    { tonicPc: mod12(tonicPc + 7), mode: 'minor', role: 'dominant' },
    { tonicPc: mod12(tonicPc + 5), mode: 'minor', role: 'subdominant' },
    { tonicPc: mod12(tonicPc + 3), mode: 'major', role: 'relative' },
    { tonicPc, mode: 'major', role: 'parallel' },
  ];
}

/**
 * chordが現在の調のピボットコード（近親調との共通コード）として機能する
 * 近親調の一覧を返す。転調先でI/iにあたる場合はその調を除外する
 * （それ自体が転調先の主和音なので「きっかけ」の予告にならないため）。
 */
export function pivotKeysFor(chord, currentKey) {
  if (!isDiatonic(chord, currentKey)) return [];
  return relatedKeys(currentKey).filter((k) => {
    if (!isDiatonic(chord, k)) return false;
    const targetDegree = mod12(chord.rootPc - k.tonicPc);
    return targetDegree !== 0;
  });
}

/** nextChordがcandidateKeyへの転調を確定させるか（現在の調には無く候補調にはあるダイアトニックコード）。 */
export function confirmsModulation(nextChord, candidateKey, currentKey) {
  return isDiatonic(nextChord, candidateKey) && !isDiatonic(nextChord, currentKey);
}

// ─────────────────────────────────────────────
// ディグリーネーム（ローマ数字度数）とコード機能の表示用ラベル
// ─────────────────────────────────────────────

/** トニックからの半音距離ベースの度数名。メジャー/マイナーどちらのキーでも同じテーブルを使う
 * （マイナーキーの自然的短音階は I bIII IV V bVI bVII と表示される）。 */
const DEGREE_NAMES = ['I', 'bII', 'II', 'bIII', 'III', 'IV', '#IV', 'V', 'bVI', 'VI', 'bVII', 'VII'];

/** chordの度数名（'I' | 'bII' | ... | 'VII'）。 */
export function degreeName(chord, key) {
  return DEGREE_NAMES[mod12(chord.rootPc - key.tonicPc)];
}

/**
 * コード機能。{ kind: 'T'|'SD'|'D'|'P'|null, resolvesTo: 度数名|null }
 * resolvesToはkind==='D'または'P'のときだけ非nullになりうる（表示側で 'D→II' 等を組み立てる）。
 *
 * 判定順（コード単体で決まる必要があるため、直前コードに依存するclassifyProgressionの
 * isReachableDegreeとは別ロジック）:
 *   1. normalizeFamily===dom なら常にD。解決先は次の2通りで探す:
 *      a. 完全4度上がダイアトニック根音なら、そこへ解決（本来のV7・セカンダリードミナント）
 *      b. そうでなくトライトーン先（+6）がダイアトニックのV度数（7）と一致するなら、
 *         V7本来の解決先（I）へ向かう裏コードとして扱う（例: Db7→I、G7の裏）
 *      どちらにも当たらなければresolvesTo=null（例: Eb7 — 半音下のD自体はダイアトニックだが、
 *      Eb7はV7の裏でもセカンダリードミナントでもなく、単体では明確な解決先を持たない）
 *   2. familyも含めて現在のキーにダイアトニックに属する場合のみ、FUNCTION_LOOKUPで
 *      機能を引く（度数だけの一致では判定しない — 例えばDmaj(度数2)は「メジャーキーの
 *      度数2=ii(min)」と度数が一致するだけで、family(maj)がiiとは別物なのでここには乗らない）
 *   3. 同主調（メジャー⇔マイナー）でfamilyも含めてダイアトニックに属する場合のみ、
 *      そちらのFUNCTION_LOOKUPで機能を引く（借用和音。こちらも度数だけの一致では判定しない —
 *      例えばD#dim(度数3)は「Cマイナーの度数3=Ebmaj」と度数が一致するだけで、
 *      family(dim)がCマイナーのbIII(maj)とは別物なのでフォールバックしない）
 *   4. normalizeFamily===dimなら、半音上がダイアトニック根音のときに限り経過和音(P)として
 *      その解決先を返す（パッシングディミニッシュ）
 *   5. 該当なしはkind=null
 */
export function chordFunction(chord, key) {
  const degree = mod12(chord.rootPc - key.tonicPc);
  const nf = normalizeFamily(chord, key);

  if (nf === 'dom') {
    const rootDegrees = diatonicRootDegrees(key);
    const V_DEGREE = 7;
    const fourthUpTarget = mod12(degree + 5);
    let resolvesTo = null;
    if (rootDegrees.includes(fourthUpTarget)) {
      resolvesTo = DEGREE_NAMES[fourthUpTarget];
    } else if (mod12(degree + 6) === V_DEGREE) {
      resolvesTo = DEGREE_NAMES[mod12(V_DEGREE + 5)];
    }
    return { kind: 'D', resolvesTo };
  }

  if (isDiatonic(chord, key)) {
    const fn = FUNCTION_LOOKUP[key.mode][degree];
    return { kind: fn === 'S' ? 'SD' : fn, resolvesTo: null };
  }

  if (isDiatonicInOppositeMode(chord, key)) {
    const oppositeMode = key.mode === 'major' ? 'minor' : 'major';
    const oppositeFn = FUNCTION_LOOKUP[oppositeMode][degree];
    return { kind: oppositeFn === 'S' ? 'SD' : oppositeFn, resolvesTo: null };
  }

  if (nf === 'dim') {
    const rootDegrees = diatonicRootDegrees(key);
    const passingTarget = mod12(degree + 1);
    if (rootDegrees.includes(passingTarget)) {
      return { kind: 'P', resolvesTo: DEGREE_NAMES[passingTarget] };
    }
  }

  return { kind: null, resolvesTo: null };
}
