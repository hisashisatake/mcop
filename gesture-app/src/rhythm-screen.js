// リズム画面（フェーズ3：3画面切り替えとシーケンサー土台）。
// ステップシーケンサーのグリッド本体はフェーズ4で実装する。フェーズ3ではメトロノームの
// ON/OFFと、Rust側`clock_loop`が刻む拍をそのまま可視化する拍インジケータだけを持つ
// （`clock_loop`のBEATS_PER_BAR=4と一致させる）。

import { isActive } from './screens.js';
import { setMetronomeEnabled, onSequencerTick } from './midi.js';

const BEATS_PER_BAR = 4;
const FLASH_DURATION_MS = 120;

let currentBeat = 0;
let flashUntil = 0; // performance.now()基準。拍が来た瞬間のフラッシュ演出用

onSequencerTick((beat) => {
  currentBeat = beat;
  flashUntil = performance.now() + FLASH_DURATION_MS;
});

export function setupRhythmScreen(canvas) {
  return { draw: (ctx) => draw(ctx, canvas) };
}

/** メトロノームON/OFFのチェックボックスを配線する。 */
export function bindRhythmScreenControls({ metronomeToggle }) {
  if (!metronomeToggle) return;
  metronomeToggle.checked = false;
  metronomeToggle.addEventListener('change', () => {
    setMetronomeEnabled(metronomeToggle.checked);
  });
}

function draw(ctx, canvas) {
  if (!isActive('rhythm')) return;
  const W = canvas.width;
  const H = canvas.height;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  ctx.textAlign = 'center';
  ctx.fillStyle = '#555';
  ctx.font = '20px monospace';
  ctx.fillText('リズム画面（ステップシーケンサーは準備中）', W / 2, H / 2 - 60);

  // 拍インジケータ：4つの丸。1拍目はベル（黄）、それ以外はクリック（水色）
  const radius = 18;
  const gap = 60;
  const cx0 = W / 2 - (gap * (BEATS_PER_BAR - 1)) / 2;
  const cy = H / 2;
  const flashing = performance.now() < flashUntil;
  for (let i = 0; i < BEATS_PER_BAR; i++) {
    const cx = cx0 + i * gap;
    ctx.beginPath();
    ctx.arc(cx, cy, radius, 0, Math.PI * 2);
    ctx.fillStyle = i === currentBeat && flashing ? (i === 0 ? '#fd6' : '#7cf') : '#2a2a2a';
    ctx.fill();
    ctx.strokeStyle = '#444';
    ctx.stroke();
  }
}
