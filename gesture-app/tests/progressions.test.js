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

test('cyclicテンプレート(Axis進行)は末尾から先頭へ回り込む', () => {
  // Axis進行: 0:maj,7:dom/maj,9:min,5:maj (cyclic)
  const tail = [
    { degree: 7, normFamily: 'dom', suffix: '7' },
    { degree: 9, normFamily: 'min', suffix: 'm' },
    { degree: 5, normFamily: 'maj', suffix: '' },
  ];
  const result = matchProgressions(tail, 'major');
  const axis = result.find((m) => m.id === 'axis');
  assert.ok(axis, 'V→vi→IVの3手でAxis進行にマッチするはず');
  assert.equal(axis.position, 4);
  assert.deepEqual(axis.next, { degree: 0, families: ['maj'] }, 'cyclicなので先頭(I)へ回り込むはず');
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

test('maxResultsで結果件数が絞られる', () => {
  // 度数0(maj)の1手からI始まりの複数major系テンプレートが初手一致しうる状況で、
  // maxResultsが実際に効くことだけを確認する（minMatch=1で強制的に多くマッチさせる）
  const oneChord = [{ degree: 0, normFamily: 'maj', suffix: '' }];
  const result = matchProgressions(oneChord, 'major', { minMatch: 1, maxResults: 2 });
  assert.ok(result.length <= 2);
});
