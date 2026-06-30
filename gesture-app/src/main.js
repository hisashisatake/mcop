'use strict';

// Tauri invoke — フォールバックでブラウザ単体でも開ける
const invoke = window.__TAURI__?.core?.invoke ?? (async (_cmd, _args) => 0);

// ─────────────────────────────────────────────
// State
// ─────────────────────────────────────────────
const S = { CAL_C: 0, CAL_F: 1, CAL_G: 2, PLAYING: 3 };
let state = S.CAL_C;

// キャリブレーション点（画面座標）
const cal = { C: null, F: null, G: null };

// 変換済み座標系
let cs = null; // { origin, rdx, rdy, cdx, cdy, pixPerSemi, chordPixPerStep }

// 再生状態
let activeChannels = [];
let lastChordKey   = null;
let isUpdating     = false;
let pendingPos     = null;   // {x,y} — mousemove から animation tick へ橋渡し
let mouseHeld      = false;
let mousePos       = { x: 0, y: 0 };
let currentProgram = 0;

// ym38x6ビルトイン波形（スロット0〜7、waveform.rs参照）。波形メモリ音色のProgram番号に対応。
const WAVE_NAMES = ['sine', 'half-sine', 'abs-sine', 'square', 'saw', 'quantized', 'pulse', 'octave'];

// Bank=0（GM2 Bank0）で手動チューニング済みのProgram名（preset.rsのgm2_bank0_patch参照）。
// 未掲載のProgramはplaceholder_patchへフォールバックする。
const FM_PROGRAM_NAMES = { 0: 'Acoustic Grand Piano', 4: 'Electric Piano 1', 80: 'Lead 1 (Square)' };

// 波形メモリ音色専用のBank Select番号（ym38x6-coreのWAVEFORM_MEMORY_BANKと一致させる）。
// Program 0〜7=ビルトイン波形+ピアノ風ADSR、8〜15=ビルトイン波形+リード風ADSR、
// 16〜127=ユーザー波形スロット（preset.rsのwaveform_memory_params_for_program参照）。
const WAVEFORM_MEMORY_BANK = 16383;

// ─────────────────────────────────────────────
// パフォーマンスLFO（ビブラート/トレモロ）
// ─────────────────────────────────────────────
const LFO_RATE_DEFAULT = 140; // 中程度の速さ
const LFO_RATE_STEP    = 8;
const LFO_DELAY = 0;
const LFO_WAVEFORM_TRIANGLE = 0;
const LFO_DEST_PITCH  = 0; // ビブラート
const LFO_DEST_VOLUME = 1; // トレモロ
const MOD_DEPTH_RANGE = 64; // RPN0,5デフォルト（約50セント相当）
const CC77_BASE = 0;        // Depthベース値は0固定。深さはマウスホイール（CC1相当）のみで制御

let modWheel       = 0; // CC1相当。0〜255
let lfoDestination = LFO_DEST_PITCH;
let lfoRate        = LFO_RATE_DEFAULT;

async function applyPerformanceLfo(channel) {
  await invoke('ym38x6_set_performance_lfo', {
    channel,
    rate: lfoRate,
    delay: LFO_DELAY,
    waveform: LFO_WAVEFORM_TRIANGLE,
    destination: lfoDestination,
    cc77: CC77_BASE,
    cc1: modWheel,
    modDepthRange: MOD_DEPTH_RANGE,
  });
}

async function applyPerformanceLfoToActiveChannels() {
  for (const ch of activeChannels) {
    await applyPerformanceLfo(ch);
  }
}

// ─────────────────────────────────────────────
// 音源モード切替（波形メモリ ⇔ Bank/Program手動指定）とProgram切り替え
// （動作確認用の簡易UI）。デフォルトはFM音源（チェックOFF）。チェックを切り替えるたびに
// 切り替え前のBank/Programをそのモード用に退避し、切り替え後のモードで前回使っていた
// Bank/Programを復元する（FM⇔波形メモリのどちら向きの切替でも双方向に復元される）。
// 「波形メモリ」チェックON時はBank欄をWAVEFORM_MEMORY_BANKに固定して編集不可にする。
// ─────────────────────────────────────────────
(() => {
  const wmToggle = document.getElementById('waveform-memory-toggle');
  const bankEl   = document.getElementById('program-bank');
  const numEl    = document.getElementById('program-num');
  const labelEl  = document.getElementById('program-label');

  // 各モードで最後に使っていたBank/Program（モード切替時の復元先）
  let savedFmBank    = parseInt(bankEl.value, 10) || 0;
  let savedFmProgram = parseInt(numEl.value, 10) || 0;
  let savedWmProgram = 0;

  function programName(bank, program) {
    if (bank === WAVEFORM_MEMORY_BANK) {
      if (program < 8)  return `${WAVE_NAMES[program]} (piano)`;
      if (program < 16) return `${WAVE_NAMES[program - 8]} (lead)`;
      return `波形 #${program} (user)`;
    }
    if (bank === 0) return FM_PROGRAM_NAMES[program] ?? `FM #${program}（placeholder）`;
    return `Bank ${bank} / Program ${program}`;
  }

  function syncBankField() {
    if (wmToggle.checked) {
      // FM → 波形メモリ：現在のFM Bank/Programを退避し、波形メモリ側の前回Programを復元
      savedFmBank    = parseInt(bankEl.value, 10) || 0;
      savedFmProgram = parseInt(numEl.value, 10) || 0;
      bankEl.value = WAVEFORM_MEMORY_BANK;
      numEl.value  = savedWmProgram;
    } else {
      // 波形メモリ → FM：現在のProgramを退避し、FM側のBank/Programを復元
      savedWmProgram = parseInt(numEl.value, 10) || 0;
      bankEl.value = savedFmBank;
      numEl.value  = savedFmProgram;
    }
    bankEl.disabled = wmToggle.checked;
  }

  async function applyProgram() {
    const bank = wmToggle.checked
      ? WAVEFORM_MEMORY_BANK
      : Math.max(0, Math.min(16383, parseInt(bankEl.value, 10) || 0));
    const program = Math.max(0, Math.min(127, parseInt(numEl.value, 10) || 0));
    currentProgram = program;
    labelEl.textContent = programName(bank, program);
    await invoke('ym38x6_set_program', { bank, program });
    lastChordKey = null; // 同じコードでも即座に音色変更させる
  }

  wmToggle.addEventListener('change', () => { syncBankField(); applyProgram(); });
  bankEl.addEventListener('input', applyProgram);
  numEl.addEventListener('input', applyProgram);

  syncBankField();
  applyProgram(); // 起動時に既定の音色（FM音源 Bank0/Program0）を反映
})();

// ─────────────────────────────────────────────
// Canvas
// ─────────────────────────────────────────────
const canvas  = document.getElementById('canvas');
const ctx     = canvas.getContext('2d');
const chordEl = document.getElementById('chord-display');
const hintEl  = document.getElementById('chord-type-hint');

function resize() {
  canvas.width  = window.innerWidth;
  canvas.height = window.innerHeight;
}
window.addEventListener('resize', resize);
resize();

// ─────────────────────────────────────────────
// 座標系の構築
// ─────────────────────────────────────────────
function buildCoords(C, F, G) {
  // C→F（5半音）と C→G（7半音）の両ベクトルを半音数で重み付けした加重平均で
  // ルート音軸を推定 — 1点だけ使うより方向・スケール両方が安定する
  const cfDx = F.x - C.x, cfDy = F.y - C.y;
  const cgDx = G.x - C.x, cgDy = G.y - C.y;
  const cfDist = Math.hypot(cfDx, cfDy);
  const cgDist = Math.hypot(cgDx, cgDy);
  if (cfDist < 5 || cgDist < 10) return null;

  // 単位ベクトルを半音数で重み付け
  const wx = (cfDx / cfDist) * 5 + (cgDx / cgDist) * 7;
  const wy = (cfDy / cfDist) * 5 + (cgDy / cgDist) * 7;
  const wLen = Math.hypot(wx, wy);
  const rdx = wx / wLen, rdy = wy / wLen;   // ルート音方向（単位ベクトル）

  // ピクセル/半音 = 両推定の平均
  const pixPerSemi = (cfDist / 5 + cgDist / 7) / 2;

  // コード種類軸 = ルート音軸を 90° 時計回りに回転
  // （右へ動く → dom7/maj7, 左 → m/dim）
  const cdx = rdy, cdy = -rdx;
  const chordPixPerStep = pixPerSemi * 2;    // 1ステップあたりのピクセル（調整可）

  return { origin: { ...C }, rdx, rdy, cdx, cdy, pixPerSemi, chordPixPerStep };
}

// 画面座標 → 音楽座標
function toMusicCoords(px, py) {
  const { origin, rdx, rdy, cdx, cdy, pixPerSemi, chordPixPerStep } = cs;
  const dx = px - origin.x, dy = py - origin.y;
  return {
    semitones:  (dx * rdx + dy * rdy) / pixPerSemi,
    chordParam: (dx * cdx + dy * cdy) / chordPixPerStep,
  };
}

// ─────────────────────────────────────────────
// 音楽ロジック
// ─────────────────────────────────────────────
const NOTE_NAMES = ['C','C#','D','D#','E','F','F#','G','G#','A','A#','B'];

function midiFreq(midi) { return 440 * Math.pow(2, (midi - 69) / 12); }

function semitoneToRoot(semi) {
  const n = Math.round(semi);
  const clamped = Math.max(-24, Math.min(24, n));   // C2 〜 C6 程度
  const name = NOTE_NAMES[((clamped % 12) + 12) % 12];
  return { midi: 60 + clamped, name };
}

// コード種類テーブル（chordParam の範囲 → インターバル）
const CHORD_TYPES = [
  { suffix: 'dim',  intervals: [0, 3, 6],        maxParam: -1.5 },
  { suffix: 'm',    intervals: [0, 3, 7],         maxParam: -0.5 },
  { suffix: '',     intervals: [0, 4, 7],         maxParam:  0.5 },
  { suffix: '7',    intervals: [0, 4, 7, 10],     maxParam:  1.5 },
  { suffix: 'maj7', intervals: [0, 4, 7, 11],     maxParam:  Infinity },
];

function chordFromParam(param) {
  return CHORD_TYPES.find(c => param < c.maxParam) ?? CHORD_TYPES[2];
}

function chordParamLabel(param) {
  if (param < -1.5) return '← dim';
  if (param < -0.5) return '← m';
  if (param <  0.5) return 'maj';
  if (param <  1.5) return '7 →';
  return 'maj7 →';
}

// ─────────────────────────────────────────────
// 発音制御
// ─────────────────────────────────────────────
async function stopChord() {
  const chs = activeChannels.splice(0);
  for (const ch of chs) {
    await invoke('note_off', { channel: ch });
  }
}

async function updateChord(px, py) {
  if (!cs || isUpdating) { pendingPos = { x: px, y: py }; return; }

  const { semitones, chordParam } = toMusicCoords(px, py);
  const root  = semitoneToRoot(semitones);
  const chord = chordFromParam(chordParam);
  const key   = `${root.midi}:${chord.suffix}`;
  if (key === lastChordKey) return;

  isUpdating = true;
  try {
    const frequencies = chord.intervals.map(interval => midiFreq(root.midi + interval));
    const prevCount = activeChannels.length;
    const nextChannels = [];

    // 各声部は固定スロット（声部インデックス i）をチャンネルIDとして使う。
    // 押し直し（前のコードを離した後の再発音）でも同じスロットIDへnote_onするため、
    // エンジン側で直前のリリーステールが即座にカット&再アタックされる（同音チョーク）。
    for (let i = 0; i < frequencies.length; i++) {
      await invoke('note_on', { channel: i, waveSlot: currentProgram, frequency: frequencies[i] });
      nextChannels.push(i);
      await applyPerformanceLfo(i);
    }
    // 旧コードの方が声部が多い場合、余ったスロットをキーオフ
    for (let i = frequencies.length; i < prevCount; i++) {
      await invoke('note_off', { channel: i });
    }

    activeChannels = nextChannels;

    // マウスが await 中に離された場合は発音しない
    if (!mouseHeld) {
      await stopChord();
      return;
    }

    lastChordKey = key;
    chordEl.textContent = root.name + chord.suffix;
    hintEl.textContent  = chordParamLabel(chordParam);
  } finally {
    isUpdating = false;
    if (pendingPos) {
      const p = pendingPos; pendingPos = null;
      updateChord(p.x, p.y);
    }
  }
}

// ─────────────────────────────────────────────
// 入力ハンドラ
// ─────────────────────────────────────────────
canvas.addEventListener('mousedown', async (e) => {
  if (e.button !== 0) return;
  if (state !== S.PLAYING) { handleCalibClick(e.clientX, e.clientY); return; }
  mouseHeld = true;
  await updateChord(e.clientX, e.clientY);
});

canvas.addEventListener('mousemove', (e) => {
  mousePos = { x: e.clientX, y: e.clientY };
  if (state === S.PLAYING && mouseHeld) pendingPos = { x: e.clientX, y: e.clientY };
});

async function releaseChord() {
  if (!mouseHeld) return;
  mouseHeld    = false;
  lastChordKey = null;
  await stopChord();
  chordEl.textContent = '—';
  hintEl.textContent  = '';
}
canvas.addEventListener('mouseup',    releaseChord);
canvas.addEventListener('mouseleave', releaseChord);

canvas.addEventListener('wheel', async (e) => {
  if (state !== S.PLAYING) return;
  e.preventDefault();
  modWheel = Math.max(0, Math.min(255, modWheel - Math.sign(e.deltaY) * 8));
  await applyPerformanceLfoToActiveChannels();
}, { passive: false });

window.addEventListener('keydown', async (e) => {
  if (e.key.toLowerCase() === 'r') {
    await stopChord();
    cal.C = cal.F = cal.G = null;
    cs = null; state = S.CAL_C; lastChordKey = null;
    chordEl.textContent = '—'; hintEl.textContent = '';
  }
  if (e.key.toLowerCase() === 'v') {
    lfoDestination = lfoDestination === LFO_DEST_PITCH ? LFO_DEST_VOLUME : LFO_DEST_PITCH;
    await applyPerformanceLfoToActiveChannels();
  }
  if (e.key.toLowerCase() === 'c') {
    lfoRate = Math.max(0, lfoRate - LFO_RATE_STEP);
    await applyPerformanceLfoToActiveChannels();
  }
  if (e.key.toLowerCase() === 'b') {
    lfoRate = Math.min(255, lfoRate + LFO_RATE_STEP);
    await applyPerformanceLfoToActiveChannels();
  }
});

// ─────────────────────────────────────────────
// 音色エディタ（egui-wasm、editor-wasmクレート）
// Eキーでオーバーレイ表示/非表示をトグル。初回表示時にwasmモジュールを遅延ロードする。
// パラメーター変更はeditor-wasm内部でTauri IPC(ym38x6_set_patch/set_master_effects)へ
// 直接送られ、main.js側のチャンネル発音ロジックとは独立して動く。
//
// Eキーのリスナーはキャプチャフェーズで登録する。エディタ表示中はeguiのcanvasに
// フォーカスが移り、eguiが内部でkeydownを処理してbubbleフェーズまで伝播させない
// （stopPropagation相当）ため、bubbleフェーズのリスナーだとエディタ起動後に
// Eキーが届かず閉じられなくなる。キャプチャフェーズはcanvasより先に発火するため、
// これを回避できる。
// ─────────────────────────────────────────────
const editorOverlay = document.getElementById('editor-overlay');
let editorHandle  = null;
let editorModule  = null; // import()したモジュール参照。notify_shift呼び出しに使う
let editorVisible = false;

async function toggleEditor() {
  editorVisible = !editorVisible;
  editorOverlay.style.display = editorVisible ? 'block' : 'none';
  if (editorVisible && !editorHandle) {
    editorModule = await import('./editor-wasm/editor_wasm.js');
    await editorModule.default();
    editorHandle = new editorModule.EditorHandle();
    await editorHandle.start('editor-canvas');
  }
}

window.addEventListener('keydown', async (e) => {
  if (e.key.toLowerCase() === 'e') {
    e.stopPropagation();
    e.preventDefault();
    await toggleEditor();
  }
}, true);

// 鍵盤UIのオクターブ移動（左Ctrl/右Ctrl）。
// event.key（'Control'）は左右を区別できないため、物理キーを区別できるevent.code
// （'ControlLeft'/'ControlRight'）を使う。event.location（1=左/2=右）も合わせてチェックし、
// どちらか一方しか取れない環境でも動くようにする。
// egui標準のキー入力経路では左右Ctrlを判別できない（editor-wasm/src/shift_keys.rs参照）ため、
// ここで直接editor-wasmのnotify_shift()をwasm-bindgen経由で呼び出す。
window.addEventListener('keydown', (e) => {
  if (!editorVisible || !editorModule) return;
  if (e.key !== 'Control') return;
  const isLeft  = e.code === 'ControlLeft'  || e.location === 1;
  const isRight = e.code === 'ControlRight' || e.location === 2;
  if (isLeft) {
    e.preventDefault();
    editorModule.notify_shift('left');
  } else if (isRight) {
    e.preventDefault();
    editorModule.notify_shift('right');
  }
}, true);

// ─────────────────────────────────────────────
// キャリブレーション
// ─────────────────────────────────────────────
function handleCalibClick(x, y) {
  if      (state === S.CAL_C) { cal.C = { x, y }; state = S.CAL_F; }
  else if (state === S.CAL_F) { cal.F = { x, y }; state = S.CAL_G; }
  else if (state === S.CAL_G) {
    cal.G = { x, y };
    const built = buildCoords(cal.C, cal.F, cal.G);
    if (built) { cs = built; state = S.PLAYING; }
    else { cal.G = null; } // C と G が近すぎた — やり直し
  }
}

// ─────────────────────────────────────────────
// 描画
// ─────────────────────────────────────────────
const CAL_INFO = [
  { label: 'C', color: '#4af', prompt: 'Cメジャーの位置でクリック', sub: 'ド・ミ・ソ（I）' },
  { label: 'F', color: '#fa4', prompt: 'Fメジャーの位置でクリック', sub: 'ファ・ラ・ド（IV）' },
  { label: 'G', color: '#4f8', prompt: 'Gメジャーの位置でクリック', sub: 'ソ・シ・レ（V）' },
];

function draw() {
  const W = canvas.width, H = canvas.height;
  ctx.clearRect(0, 0, W, H);

  // 背景
  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  if (state === S.PLAYING) {
    drawPlayingLayer(W, H);
  } else {
    drawCalibLayer(W, H);
  }
  drawLfoIndicator(W, H);
}

function drawCalibLayer(W, H) {
  const info = CAL_INFO[state];

  // 配置済み点
  const placed = [
    state > S.CAL_C && cal.C && { p: cal.C, ...CAL_INFO[0] },
    state > S.CAL_F && cal.F && { p: cal.F, ...CAL_INFO[1] },
  ].filter(Boolean);
  for (const { p, label, color } of placed) {
    ctx.beginPath();
    ctx.arc(p.x, p.y, 12, 0, Math.PI * 2);
    ctx.fillStyle = color + '44';
    ctx.fill();
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.stroke();
    ctx.fillStyle = color;
    ctx.font = 'bold 14px monospace';
    ctx.textAlign = 'left';
    ctx.fillText(label, p.x + 16, p.y + 5);
  }

  // 中央の指示
  ctx.textAlign = 'center';
  ctx.fillStyle = info.color;
  ctx.font = 'bold 30px monospace';
  ctx.fillText(info.prompt, W / 2, H / 2 - 18);

  ctx.fillStyle = '#888';
  ctx.font = '18px monospace';
  ctx.fillText(info.sub, W / 2, H / 2 + 18);

  ctx.fillStyle = '#444';
  ctx.font = '13px monospace';
  ctx.fillText('どこでも OK — 自分の感覚で自然な位置をクリック', W / 2, H / 2 + 52);
  ctx.fillText('後から R キーで再キャリブレーション', W / 2, H / 2 + 72);
}

function drawPlayingLayer(W, H) {
  // マウス周囲に光彩（コード種類で色調変化）
  if (mouseHeld && cs) {
    const { chordParam } = toMusicCoords(mousePos.x, mousePos.y);
    const t = Math.max(-1, Math.min(1, chordParam / 2));
    // 左（マイナー系）→青, 右（メジャー/7th系）→緑
    const rr = t < 0 ? Math.round(40 + (-t) * 80) : 20;
    const gg = t > 0 ? Math.round(40 + t * 80) : 20;
    const bb = Math.round(80 + (1 - Math.abs(t)) * 60);
    const grd = ctx.createRadialGradient(mousePos.x, mousePos.y, 0, mousePos.x, mousePos.y, 350);
    grd.addColorStop(0, `rgba(${rr},${gg},${bb},0.22)`);
    grd.addColorStop(1, 'rgba(0,0,0,0)');
    ctx.fillStyle = grd;
    ctx.fillRect(0, 0, W, H);
  }

  // ルート音軸をうっすら表示
  if (cs) {
    const { origin, rdx, rdy } = cs;
    const len = Math.max(W, H) * 1.5;
    ctx.save();
    ctx.strokeStyle = 'rgba(100,180,255,0.12)';
    ctx.lineWidth = 1;
    ctx.setLineDash([5, 10]);
    ctx.beginPath();
    ctx.moveTo(origin.x - rdx * len, origin.y - rdy * len);
    ctx.lineTo(origin.x + rdx * len, origin.y + rdy * len);
    ctx.stroke();
    ctx.restore();
  }

  // キャリブレーション点（小さめ）
  for (const [key, info] of [['C', CAL_INFO[0]], ['F', CAL_INFO[1]], ['G', CAL_INFO[2]]]) {
    const p = cal[key]; if (!p) continue;
    ctx.beginPath();
    ctx.arc(p.x, p.y, 4, 0, Math.PI * 2);
    ctx.fillStyle = info.color + '55';
    ctx.fill();
  }

  // コード種類スケール（画面下部中央）
  drawChordScale(W, H);
}

function drawChordScale(W, H) {
  const labels = ['dim', 'm', 'maj', '7', 'maj7'];
  const total = labels.length;
  const step = 72, startX = W / 2 - step * (total - 1) / 2;
  const y = H - 68;

  // 現在のコード種類をハイライト
  let activeIdx = 2; // default: maj
  if (cs && mouseHeld) {
    const { chordParam } = toMusicCoords(mousePos.x, mousePos.y);
    activeIdx = CHORD_TYPES.findIndex(c => chordParam < c.maxParam);
    if (activeIdx < 0) activeIdx = CHORD_TYPES.length - 1;
  }

  for (let i = 0; i < total; i++) {
    const x = startX + i * step;
    const isActive = i === activeIdx;
    ctx.fillStyle = isActive ? '#fff' : '#444';
    ctx.font = isActive ? 'bold 14px monospace' : '13px monospace';
    ctx.textAlign = 'center';
    ctx.fillText(labels[i], x, y);
    if (isActive) {
      ctx.beginPath();
      ctx.arc(x, y + 10, 3, 0, Math.PI * 2);
      ctx.fillStyle = '#4af';
      ctx.fill();
    }
  }

  // ← → ガイド
  ctx.fillStyle = '#333';
  ctx.font = '12px monospace';
  ctx.textAlign = 'left';
  ctx.fillText('← 暗い', startX - 60, y);
  ctx.textAlign = 'right';
  ctx.fillText('明るい →', startX + step * (total - 1) + 60, y);

}

function drawLfoIndicator() {
  const label = lfoDestination === LFO_DEST_VOLUME ? 'Tremolo' : 'Vibrato';
  const x = 16, barW = 100, barH = 6;

  ctx.textAlign = 'left';
  ctx.font = '13px monospace';
  ctx.fillStyle = modWheel > 0 ? '#4af' : '#444';
  ctx.fillText(`LFO: ${label} (V)`, x, 28);

  ctx.strokeStyle = '#444';
  ctx.lineWidth = 1;
  ctx.strokeRect(x, 38, barW, barH);
  ctx.fillStyle = '#4af';
  ctx.fillRect(x, 38, barW * (modWheel / 255), barH);

  ctx.fillStyle = '#666';
  ctx.fillText(`Rate: ${lfoRate} (C/B)`, x, 64);
  ctx.strokeStyle = '#444';
  ctx.strokeRect(x, 70, barW, barH);
  ctx.fillStyle = '#888';
  ctx.fillRect(x, 70, barW * (lfoRate / 255), barH);
}

// ─────────────────────────────────────────────
// アニメーションループ
// ─────────────────────────────────────────────
function tick() {
  // mousemove で溜まった pending を消化
  if (state === S.PLAYING && mouseHeld && pendingPos && !isUpdating) {
    const p = pendingPos; pendingPos = null;
    updateChord(p.x, p.y);
  }
  draw();
  requestAnimationFrame(tick);
}

tick();
