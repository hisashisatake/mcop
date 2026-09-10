import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PROGRESSIONS, matchProgressions } from '../src/progressions.js';

const KNOWN_FAMILIES = new Set(['maj', 'dom', 'min', 'halfdim', 'dim']);

test('PROGRESSIONS: 20個のテンプレートが登録されている', () => {
  assert.equal(PROGRESSIONS.length, 20);
});

test('PROGRESSIONS: 各テンプレートのステップが健全（degree0-11、既知family、2ステップ以上）', () => {
  for (const prog of PROGRESSIONS) {
    assert.ok(prog.steps.length >= 2, `${prog.id}: steps too short`);
    assert.ok(prog.mode === 'major' || prog.mode === 'minor', `${prog.id}: invalid mode`);
    for (const step of prog.steps) {
      assert.ok(step.degree >= 0 && step.degree <= 11, `${prog.id}: degree out of range`);
      if (step.suffixes) {
        assert.ok(step.suffixes.length > 0, `${prog.id}: empty suffixes`);
      } else {
        assert.ok(step.families && step.families.length > 0, `${prog.id}: missing families`);
        for (const f of step.families) {
          assert.ok(KNOWN_FAMILIES.has(f), `${prog.id}: unknown family "${f}"`);
        }
      }
    }
  }
});

test('PROGRESSIONS: idに重複が無い', () => {
  const ids = PROGRESSIONS.map((p) => p.id);
  assert.equal(new Set(ids).size, ids.length);
});

test('12小節ブルース: minMatch=4により、反復構造による途中からの誤検出が起きない', () => {
  // steps=[0,0,0,0,5,5,0,0,7,5,0,7]。既定のminMatch=2だと「I→I」のような
  // ありふれた2手が本来の位置(1〜2小節目)以外(6〜7小節目等)でも一致してしまうため、
  // 4手に引き上げている（詳細はprogressions.js内のコメント参照）
  const twoI = [
    { degree: 0, normFamily: 'dom', suffix: '7' },
    { degree: 0, normFamily: 'dom', suffix: '7' },
  ];
  assert.ok(!matchProgressions(twoI, 'major').some((m) => m.id === 'blues12'), '2手だけではマッチしないはず');

  // 5〜8小節目相当(IV,IV,I,I)を単独で弾いても、先頭(1〜4小節目)と誤認してマッチしないはず
  const midFour = [
    { degree: 5, normFamily: 'dom', suffix: '7' },
    { degree: 5, normFamily: 'dom', suffix: '7' },
    { degree: 0, normFamily: 'dom', suffix: '7' },
    { degree: 0, normFamily: 'dom', suffix: '7' },
  ];
  assert.ok(!matchProgressions(midFour, 'major').some((m) => m.id === 'blues12'), '曲中盤の4手を単独で弾いてもマッチしないはず');

  const firstFour = [
    { degree: 0, normFamily: 'dom', suffix: '7' },
    { degree: 0, normFamily: 'dom', suffix: '7' },
    { degree: 0, normFamily: 'dom', suffix: '7' },
    { degree: 0, normFamily: 'dom', suffix: '7' },
  ];
  const result = matchProgressions(firstFour, 'major');
  const blues = result.find((m) => m.id === 'blues12');
  assert.ok(blues, '先頭から4手(I-I-I-I)ならマッチするはず');
  assert.equal(blues.position, 4);
});

test('カノン進行: 2手一致で次の一手が返り、1手だけでは返らない(minMatch=2)', () => {
  // カノン進行: 0:maj,7:dom/maj,9:min,4:min,5:maj,0:maj,5:maj,7:dom/maj
  const oneChord = [{ degree: 0, normFamily: 'maj', suffix: '' }];
  const oneMatch = matchProgressions(oneChord, 'major');
  assert.ok(!oneMatch.some((m) => m.id === 'canon'), '1手だけではマッチしないはず');

  const twoChords = [
    { degree: 0, normFamily: 'maj', suffix: '' },
    { degree: 7, normFamily: 'dom', suffix: '7' },
  ];
  const twoMatch = matchProgressions(twoChords, 'major');
  const canon = twoMatch.find((m) => m.id === 'canon');
  assert.ok(canon, 'I→V(7)の2手でカノン進行にマッチするはず');
  assert.equal(canon.position, 2);
  assert.equal(canon.total, 8);
  assert.deepEqual(canon.next, { degree: 9, families: ['min'] });
});

test('カノン進行: 末尾(V)まで一致し、非cyclicなので次の一手は無い', () => {
  const full = [
    { degree: 0, normFamily: 'maj', suffix: '' },
    { degree: 7, normFamily: 'dom', suffix: '7' },
    { degree: 9, normFamily: 'min', suffix: 'm' },
    { degree: 4, normFamily: 'min', suffix: 'm' },
    { degree: 5, normFamily: 'maj', suffix: '' },
    { degree: 0, normFamily: 'maj', suffix: '' },
    { degree: 5, normFamily: 'maj', suffix: '' },
    { degree: 7, normFamily: 'dom', suffix: '7' },
  ];
  const result = matchProgressions(full, 'major');
  assert.ok(!result.some((m) => m.id === 'canon'), '非cyclicで末尾到達後は結果に含まれないはず');
});

test('cyclicテンプレート(Axis進行)は先頭(I)から一周弾くと末尾から先頭へ回り込む', () => {
  // Axis進行: 0:maj,7:dom/maj,9:min,5:maj (cyclic)
  const full = [
    { degree: 0, normFamily: 'maj', suffix: '' },
    { degree: 7, normFamily: 'dom', suffix: '7' },
    { degree: 9, normFamily: 'min', suffix: 'm' },
    { degree: 5, normFamily: 'maj', suffix: '' },
  ];
  const result = matchProgressions(full, 'major');
  const axis = result.find((m) => m.id === 'axis');
  assert.ok(axis, 'I→V→vi→IVの4手(先頭から1周)でAxis進行にマッチするはず');
  assert.equal(axis.position, 4);
  assert.deepEqual(axis.next, { degree: 0, families: ['maj'] }, 'cyclicなので先頭(I)へ回り込むはず');
});

test('先頭(I)を経由しないAxis進行の部分列(V→vi→IV)はマッチしない（途中からの誤検出防止）', () => {
  // 6451進行(V→Iの2手だけでComplete扱いされていたバグ)と同種の問題を全テンプレートで
  // 遮断するため、一致はテンプレートの先頭からの連続のみを認める設計にした
  // （詳細はprogressions.js matchProgressions()のコメント参照）
  const tail = [
    { degree: 7, normFamily: 'dom', suffix: '7' },
    { degree: 9, normFamily: 'min', suffix: 'm' },
    { degree: 5, normFamily: 'maj', suffix: '' },
  ];
  const result = matchProgressions(tail, 'major');
  assert.ok(!result.some((m) => m.id === 'axis'), 'Iを経由していないのでAxis進行にはマッチしないはず');
});

test('line-cliche: suffix完全一致で判定される(familiesでは区別できないケース)', () => {
  // 半音下降ラインクリシェ: 0:m → 0:mMaj7 → 0:m7 → 0:m6 (全て度数0・minor系)
  const tail = [
    { degree: 0, normFamily: 'min', suffix: 'm' },
    { degree: 0, normFamily: 'min', suffix: 'mMaj7' },
  ];
  const result = matchProgressions(tail, 'minor');
  const lineCliche = result.find((m) => m.id === 'line-cliche');
  assert.ok(lineCliche, 'm→mMaj7の2手でline-clicheにマッチするはず');
  assert.deepEqual(lineCliche.next, { degree: 0, suffixes: ['m7'] });
});

test('Just the Two of Us進行はminMatch=1のため1手目だけでもマッチする', () => {
  // IVmaj7(1手目)自体はダイアトニックで見つけやすいが、2手目のIII7が元々の「見えにくい」問題の
  // 張本人だったため、このテンプレートだけ1手目から次(III7)を案内できるようにした
  const oneChord = [{ degree: 5, normFamily: 'maj', suffix: 'maj7' }];
  const result = matchProgressions(oneChord, 'major');
  const jtt = result.find((m) => m.id === 'just-two-of-us');
  assert.ok(jtt, 'IVmaj7の1手だけでJust the Two of Us進行にマッチするはず');
  assert.equal(jtt.position, 1);
  assert.deepEqual(jtt.next, { degree: 4, families: ['dom'] });
});

test('他のテンプレートは既定どおりminMatch=2のまま(1手では出ない)', () => {
  const oneChord = [{ degree: 0, normFamily: 'maj', suffix: '' }];
  const result = matchProgressions(oneChord, 'major');
  assert.ok(!result.some((m) => m.id === '1625'), '1625は1手だけではマッチしないはず');
});

test('Just the Two of Us進行のminMatch=1は先頭ステップ限定。途中のii(度数2)を単独/2手目で弾いても誤って進捗表示されない', () => {
  // IVmaj7(1〜3手目)を一切経由せず、いきなりIIm9(度数2、テンプレートの4番目のステップ)だけを
  // 弾いた場合に「4/5まで進んだ」ように誤表示されないことの回帰テスト
  const iim9Only = [{ degree: 2, normFamily: 'min', suffix: 'm9' }];
  const resultSingle = matchProgressions(iim9Only, 'major');
  assert.ok(
    !resultSingle.some((m) => m.id === 'just-two-of-us'),
    'IIm9を単独で弾いただけではJust the Two of Us進行にマッチしないはず',
  );

  // Imaj9→IIm9の2手でも、IVmaj7を経由していないので同様にマッチしないはず
  const imaj9ThenIim9 = [
    { degree: 0, normFamily: 'maj', suffix: 'maj9' },
    { degree: 2, normFamily: 'min', suffix: 'm9' },
  ];
  const resultTwo = matchProgressions(imaj9ThenIim9, 'major');
  assert.ok(
    !resultTwo.some((m) => m.id === 'just-two-of-us'),
    'Imaj9→IIm9はJust the Two of Us進行の並びではないのでマッチしないはず',
  );
});

test('maxResultsで結果件数が絞られる', () => {
  // I→V→vi→IV(度数0,7,9,5)はAxis進行(4/4)・6451進行(2/4)・Just the Two of Us進行(1/5)の
  // 3件が自然に同時マッチする状況。maxResultsで件数が絞られることを確認する
  const fourChords = [
    { degree: 0, normFamily: 'maj', suffix: '' },
    { degree: 7, normFamily: 'maj', suffix: '' },
    { degree: 9, normFamily: 'min', suffix: '' },
    { degree: 5, normFamily: 'maj', suffix: '' },
  ];
  const unlimited = matchProgressions(fourChords, 'major', { maxResults: Infinity });
  assert.equal(unlimited.length, 3, '前提: 3件マッチするはず');

  const limited = matchProgressions(fourChords, 'major', { maxResults: 2 });
  assert.equal(limited.length, 2);
});
