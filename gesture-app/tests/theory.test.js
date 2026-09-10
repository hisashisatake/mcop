import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chordFromSemitone, NORMAL_LAYER, SHIFT_LAYER, CTRL_LAYER, CTRL_SHIFT_LAYER } from '../src/chords.js';
import {
  classifyProgression,
  isDiatonic,
  normalizeFamily,
  pivotKeysFor,
  confirmsModulation,
  dissonancePenalty,
  degreeName,
  chordFunction,
  isStrongResolution,
} from '../src/theory.js';

const TONIC_MIDI = 60; // C4

/** Cを基準にした半音オフセットと行インデックスから、実際のchordFromSemitone()と同じコード構築経路でコードを作る。 */
function chordFor(semitoneFromC, rowIndex, mods = {}) {
  return chordFromSemitone(semitoneFromC, rowIndex, {
    tonicMidi: TONIC_MIDI,
    shiftHeld: !!mods.shift,
    ctrlHeld: !!mods.ctrl,
  });
}

function rowIndexOf(layer, suffix) {
  const idx = layer.findIndex((e) => e.suffix === suffix);
  assert.notEqual(idx, -1, `suffix "${suffix}" not found in layer`);
  return idx;
}

const C_MAJOR = { tonicPc: 0, mode: 'major' };
const A_MINOR = { tonicPc: 9, mode: 'minor' };

// ダイアトニック度数のショートカット（Cメジャー基準の半音オフセット）
const C = 0, D = 2, E = 4, F = 5, G = 7, A = 9, B = 11;
const Db = 1, Eb = 3, Fs = 6, Ab = 8, Bb = 10;

test('II-V-I: Dm7 → G7 は GREEN、G7 → C は最高スコア帯', () => {
  const dm7 = chordFor(D, rowIndexOf(NORMAL_LAYER, 'm7'));
  const g7 = chordFor(G, rowIndexOf(NORMAL_LAYER, '7'));
  const c = chordFor(C, rowIndexOf(NORMAL_LAYER, ''));

  const dm7ToG7 = classifyProgression(dm7, g7, C_MAJOR);
  assert.equal(dm7ToG7.category, 'GREEN');
  assert.ok(dm7ToG7.score > 0.9, `score too low: ${dm7ToG7.score}`);

  const g7ToC = classifyProgression(g7, c, C_MAJOR);
  assert.equal(g7ToC.category, 'GREEN');
  assert.ok(g7ToC.score >= dm7ToG7.score, `G7→C should score at least as high as Dm7→G7: ${g7ToC.score}`);
});

test('セカンダリードミナント: C → A7 は GREEN', () => {
  const c = chordFor(C, rowIndexOf(NORMAL_LAYER, ''));
  const a7 = chordFor(A, rowIndexOf(NORMAL_LAYER, '7'));
  const result = classifyProgression(c, a7, C_MAJOR);
  assert.equal(result.category, 'GREEN');
});

test('裏コード（トライトーン代理）: C → Db7 は YELLOW', () => {
  const c = chordFor(C, rowIndexOf(NORMAL_LAYER, ''));
  const db7 = chordFor(Db, rowIndexOf(NORMAL_LAYER, '7'));
  const result = classifyProgression(c, db7, C_MAJOR);
  assert.equal(result.category, 'YELLOW');
});

test('モーダルインターチェンジ: C → Fm・C → Ab は YELLOW', () => {
  const c = chordFor(C, rowIndexOf(NORMAL_LAYER, ''));
  const fm = chordFor(F, rowIndexOf(NORMAL_LAYER, 'm'));
  const ab = chordFor(Ab, rowIndexOf(NORMAL_LAYER, ''));
  assert.equal(classifyProgression(c, fm, C_MAJOR).category, 'YELLOW');
  assert.equal(classifyProgression(c, ab, C_MAJOR).category, 'YELLOW');
});

test('逆行: G7 → F（D→S）は消灯（NONE）', () => {
  const g7 = chordFor(G, rowIndexOf(NORMAL_LAYER, '7'));
  const f = chordFor(F, rowIndexOf(NORMAL_LAYER, ''));
  const result = classifyProgression(g7, f, C_MAJOR);
  assert.equal(result.category, null);
});

test('susの正規化: Dm7 → G7sus4 は GREEN（Vのsus版）、G7sus4 → G7 も減点されず有効', () => {
  const dm7 = chordFor(D, rowIndexOf(NORMAL_LAYER, 'm7'));
  const g7sus4 = chordFor(G, rowIndexOf(CTRL_LAYER, '7sus4'), { ctrl: true });
  const g7 = chordFor(G, rowIndexOf(NORMAL_LAYER, '7'));

  assert.equal(classifyProgression(dm7, g7sus4, C_MAJOR).category, 'GREEN');

  const sameRootColorChange = classifyProgression(g7sus4, g7, C_MAJOR);
  assert.equal(sameRootColorChange.category, 'GREEN');
});

test('付加音の正規化: G7 → C6・G7 → Cadd9 は GREEN', () => {
  const g7 = chordFor(G, rowIndexOf(NORMAL_LAYER, '7'));
  const c6 = chordFor(C, rowIndexOf(CTRL_LAYER, '6'), { ctrl: true });
  const cadd9 = chordFor(C, rowIndexOf(CTRL_LAYER, 'add9'), { ctrl: true });
  assert.equal(classifyProgression(g7, c6, C_MAJOR).category, 'GREEN');
  assert.equal(classifyProgression(g7, cadd9, C_MAJOR).category, 'GREEN');
});

test('オルタードの度数依存: Dm7 → G7b9 は GREEN、C → Eb7#5 は YELLOW止まり', () => {
  const dm7 = chordFor(D, rowIndexOf(NORMAL_LAYER, 'm7'));
  const g7b9 = chordFor(G, rowIndexOf(CTRL_SHIFT_LAYER, '7b9'), { ctrl: true, shift: true });
  assert.equal(classifyProgression(dm7, g7b9, C_MAJOR).category, 'GREEN');

  const c = chordFor(C, rowIndexOf(NORMAL_LAYER, ''));
  const eb7sharp5 = chordFor(Eb, rowIndexOf(CTRL_SHIFT_LAYER, '7#5'), { ctrl: true, shift: true });
  assert.equal(classifyProgression(c, eb7sharp5, C_MAJOR).category, 'YELLOW');
});

test('濁り検出: 半音衝突・非許容の短9度は減点され、ドミナント機能上のb9は例外扱い', () => {
  assert.ok(dissonancePenalty({ rootPc: 0, family: 'maj', intervals: [0, 1, 7] }) > 0, '半音衝突は減点されるべき');
  assert.ok(
    dissonancePenalty({ rootPc: 0, family: 'maj', intervals: [0, 7, 13] }) > 0,
    'ドミナント系でない短9度は減点されるべき',
  );

  const g7b9 = chordFor(G, rowIndexOf(CTRL_SHIFT_LAYER, '7b9'), { ctrl: true, shift: true });
  assert.equal(dissonancePenalty(g7b9), 0, 'ドミナント上のb9は例外扱い');

  const g7sus4b9 = chordFor(G, rowIndexOf(CTRL_LAYER, '7sus4b9'), { ctrl: true });
  assert.equal(dissonancePenalty(g7sus4b9), 0, 'sus上のb9も例外扱い');
});

test('isStrongResolution: G7→C・vii゜→Cはドミナント→トニックの強進行、G7→Fや非ダイアトニック進行はfalse', () => {
  const g7 = chordFor(G, rowIndexOf(NORMAL_LAYER, '7'));
  const bdim = chordFor(B, rowIndexOf(SHIFT_LAYER, 'm7b5'), { shift: true }); // vii゜7相当
  const c = chordFor(C, rowIndexOf(NORMAL_LAYER, ''));
  const f = chordFor(F, rowIndexOf(NORMAL_LAYER, ''));
  const am = chordFor(A, rowIndexOf(NORMAL_LAYER, 'm')); // 偽終止先（Tのまま）

  assert.equal(isStrongResolution(g7, c, C_MAJOR), true);
  assert.equal(isStrongResolution(bdim, c, C_MAJOR), true);
  assert.equal(isStrongResolution(g7, am, C_MAJOR), true, '偽終止先もT機能なのでtrue');
  assert.equal(isStrongResolution(g7, f, C_MAJOR), false, 'D→Sは強進行ではない');
  assert.equal(isStrongResolution(c, g7, C_MAJOR), false, 'T→Dは対象外（fromがDである必要がある）');
});

test('ピボット: Cメジャーで Am は G メジャーへのピボットとして返る／G は返らない', () => {
  const am = chordFor(A, rowIndexOf(NORMAL_LAYER, 'm'));
  const pivotsForAm = pivotKeysFor(am, C_MAJOR);
  assert.ok(
    pivotsForAm.some((k) => k.tonicPc === 7 && k.mode === 'major'),
    'Am should pivot toward G major',
  );

  const g = chordFor(G, rowIndexOf(NORMAL_LAYER, ''));
  const pivotsForG = pivotKeysFor(g, C_MAJOR);
  assert.ok(
    !pivotsForG.some((k) => k.tonicPc === 7 && k.mode === 'major'),
    'G itself is the target key\'s tonic, so it must not be listed as a pivot toward G major',
  );
});

test('転調確定: D7はGメジャーへの転調を確定させるが、G自体はさせない', () => {
  const d7 = chordFor(D, rowIndexOf(NORMAL_LAYER, '7'));
  const gMajorKey = { tonicPc: 7, mode: 'major' };
  assert.equal(confirmsModulation(d7, gMajorKey, C_MAJOR), true);

  const g = chordFor(G, rowIndexOf(NORMAL_LAYER, ''));
  assert.equal(confirmsModulation(g, gMajorKey, C_MAJOR), false);
});

test('マイナーキー: Aマイナーで E7 → Am が最高スコア帯', () => {
  const e7 = chordFor(E, rowIndexOf(NORMAL_LAYER, '7'));
  const am = chordFor(A, rowIndexOf(NORMAL_LAYER, 'm'));
  const result = classifyProgression(e7, am, A_MINOR);
  assert.equal(result.category, 'GREEN');
  assert.ok(result.score > 1.0, `score too low: ${result.score}`);
});

test('normalizeFamily: susで7thを含まない場合はその度数のダイアトニックfamilyへ落ちる', () => {
  const csus4 = chordFor(C, rowIndexOf(CTRL_LAYER, 'sus4'), { ctrl: true });
  assert.equal(normalizeFamily(csus4, C_MAJOR), 'maj');
});

test('isDiatonic: Shiftレイヤーのmaj7/m7もダイアトニック判定できる', () => {
  const cmaj7 = chordFor(C, rowIndexOf(SHIFT_LAYER, 'maj7'), { shift: true });
  const bm7b5 = chordFor(B, rowIndexOf(SHIFT_LAYER, 'm7b5'), { shift: true });
  assert.equal(isDiatonic(cmaj7, C_MAJOR), true);
  assert.equal(isDiatonic(bm7b5, C_MAJOR), true);
});

// ─────────────────────────────────────────────
// ディグリーネーム・コード機能
// ─────────────────────────────────────────────

test('degreeName: Cメジャーで半音距離どおりの度数名が返る', () => {
  assert.equal(degreeName(chordFor(C, rowIndexOf(NORMAL_LAYER, '')), C_MAJOR), 'I');
  assert.equal(degreeName(chordFor(F, rowIndexOf(NORMAL_LAYER, '')), C_MAJOR), 'IV');
  assert.equal(degreeName(chordFor(Bb, rowIndexOf(NORMAL_LAYER, '')), C_MAJOR), 'bVII');
  assert.equal(degreeName(chordFor(Fs, rowIndexOf(NORMAL_LAYER, '')), C_MAJOR), '#IV');
  assert.equal(degreeName(chordFor(Db, rowIndexOf(NORMAL_LAYER, '')), C_MAJOR), 'bII');
});

test('degreeName: Aマイナーでも同じ半音距離基準（自然的短音階はbIII/bVI）', () => {
  assert.equal(degreeName(chordFor(C, rowIndexOf(NORMAL_LAYER, '')), A_MINOR), 'bIII');
  assert.equal(degreeName(chordFor(F, rowIndexOf(NORMAL_LAYER, '')), A_MINOR), 'bVI');
});

test('chordFunction: ダイアトニックコードはFUNCTION_LOOKUPどおりの機能（Amはトニック代理）', () => {
  const am = chordFor(A, rowIndexOf(NORMAL_LAYER, 'm'));
  assert.equal(chordFunction(am, C_MAJOR).kind, 'T');
});

test('chordFunction: A7はセカンダリードミナントとしてD、度数だけのAmとは区別される', () => {
  const a7 = chordFor(A, rowIndexOf(NORMAL_LAYER, '7'));
  const result = chordFunction(a7, C_MAJOR);
  assert.equal(result.kind, 'D');
  assert.equal(result.resolvesTo, 'II');
});

test('chordFunction: Abは同主調（Cマイナー）フォールバックでSD', () => {
  const ab = chordFor(Ab, rowIndexOf(NORMAL_LAYER, ''));
  assert.equal(chordFunction(ab, C_MAJOR).kind, 'SD');
});

test('chordFunction: C#mはどの段階にも当てはまらずkind=null', () => {
  const csharpm = chordFor(Db, rowIndexOf(NORMAL_LAYER, 'm'));
  assert.equal(chordFunction(csharpm, C_MAJOR).kind, null);
});

test('chordFunction: G7は本来のV7としてD→I', () => {
  const g7 = chordFor(G, rowIndexOf(NORMAL_LAYER, '7'));
  const result = chordFunction(g7, C_MAJOR);
  assert.equal(result.kind, 'D');
  assert.equal(result.resolvesTo, 'I');
});

test('chordFunction: Db7はG7の裏コードとして同じ解決先(I)を持つ', () => {
  const db7 = chordFor(Db, rowIndexOf(NORMAL_LAYER, '7'));
  const result = chordFunction(db7, C_MAJOR);
  assert.equal(result.kind, 'D');
  assert.equal(result.resolvesTo, 'I');
});

test('chordFunction: Eb7はV7の裏でもセカンダリードミナントでもなく解決先を持たない', () => {
  const eb7 = chordFor(Eb, rowIndexOf(NORMAL_LAYER, '7'));
  const result = chordFunction(eb7, C_MAJOR);
  assert.equal(result.kind, 'D');
  assert.equal(result.resolvesTo, null);
});

test('chordFunction: G7sus4もnormalizeFamily経由でdomへ正規化されD判定になる', () => {
  const g7sus4 = chordFor(G, rowIndexOf(CTRL_LAYER, '7sus4'), { ctrl: true });
  assert.equal(chordFunction(g7sus4, C_MAJOR).kind, 'D');
});

test('chordFunction: 同主調フォールバックはfamilyも検証する（Dmajは度数だけならCマイナーのiiと一致するが、familyがhalfdimと違うためkind=null）', () => {
  const dmaj = chordFor(D, rowIndexOf(NORMAL_LAYER, ''));
  assert.equal(chordFunction(dmaj, C_MAJOR).kind, null);
});

test('chordFunction: 非ダイアトニックなdimは半音上がダイアトニックなら経過和音としてP→◯を返す（Ebdim→Eへの経過音）', () => {
  const ebdim = chordFor(Eb, rowIndexOf(CTRL_SHIFT_LAYER, 'dim'), { ctrl: true, shift: true });
  const result = chordFunction(ebdim, C_MAJOR);
  assert.equal(result.kind, 'P');
  assert.equal(result.resolvesTo, 'III');
});

test('chordFunction: 半音上も非ダイアトニックなdimは経過和音にもならずkind=null（Fdim）', () => {
  const fdim = chordFor(F, rowIndexOf(CTRL_SHIFT_LAYER, 'dim'), { ctrl: true, shift: true });
  assert.equal(chordFunction(fdim, C_MAJOR).kind, null);
});

test('classifyProgression: Dm→Ebdimは機能的隣接チェックを免除され取りこぼさずYELLOW（直前コードの半音上という経過和音の形そのもの）', () => {
  const dm = chordFor(D, rowIndexOf(NORMAL_LAYER, 'm'));
  const ebdim = chordFor(Eb, rowIndexOf(CTRL_SHIFT_LAYER, 'dim'), { ctrl: true, shift: true });
  assert.equal(classifyProgression(dm, ebdim, C_MAJOR).category, 'YELLOW');
});
