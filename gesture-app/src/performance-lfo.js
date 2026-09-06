// 演奏系モジュレーション（Vキーでビブラート⇔トレモロ切替）。
// ビブラート(Pitch)はPitch FGのループ、トレモロ(Volume)はGain FGのループへ配線される
// （質感LFO退役に伴い独立したLFOスロットは無くなり、選ばれなかった側のFGは音色プリセット
// 本来の値へ戻される。バックエンド側はsrc-tauri/src/main.rsのop505_set_performance_lfo参照）。

import { setPerformanceLfo } from './midi.js';

const LFO_RATE_DEFAULT = 140; // 中程度の速さ
const LFO_RATE_STEP = 8;
const LFO_DELAY = 0;
// PerformanceLfoDestination（main.rs、旧sound-fm::FmLfoDestinationと同じ生値）:
// Unplugged=0/Pitch=1/Volume=2/TlCarrier=3/Cutoff=4
const LFO_DEST_PITCH = 1; // ビブラート
const LFO_DEST_VOLUME = 2; // トレモロ
const MOD_DEPTH_RANGE = 64; // RPN0,5デフォルト（約50セント相当）
const CC77_BASE = 0; // Depthベース値は0固定。深さはマウスホイール（CC1相当）のみで制御

let modWheel = 0; // CC1相当。0〜255
let lfoDestination = LFO_DEST_PITCH;
let lfoRate = LFO_RATE_DEFAULT;

/** 指定チャンネルへ現在のLFO設定を送る。発音直前に呼ぶ。 */
export function applyTo(channel) {
  return setPerformanceLfo({
    channel,
    rate: lfoRate,
    delay: LFO_DELAY,
    destination: lfoDestination,
    cc77: CC77_BASE,
    cc1: modWheel,
    modDepthRange: MOD_DEPTH_RANGE,
  });
}

/**
 * ホイール（Depth）とV/C/Bキー（行き先・Rate）のハンドラを登録する。
 * @param {HTMLElement} target ホイールを拾う要素
 * @param {() => number[]} activeChannels 変更を即時反映する発音中チャンネル
 */
export function setupPerformanceLfo(target, activeChannels) {
  const applyToActive = async () => {
    for (const ch of activeChannels()) {
      await applyTo(ch);
    }
  };

  target.addEventListener(
    'wheel',
    async (e) => {
      e.preventDefault();
      modWheel = Math.max(0, Math.min(255, modWheel - Math.sign(e.deltaY) * 8));
      await applyToActive();
    },
    { passive: false },
  );

  window.addEventListener('keydown', async (e) => {
    const key = e.key.toLowerCase();
    if (key === 'v') {
      lfoDestination = lfoDestination === LFO_DEST_PITCH ? LFO_DEST_VOLUME : LFO_DEST_PITCH;
      await applyToActive();
    } else if (key === 'c') {
      lfoRate = Math.max(0, lfoRate - LFO_RATE_STEP);
      await applyToActive();
    } else if (key === 'b') {
      lfoRate = Math.min(255, lfoRate + LFO_RATE_STEP);
      await applyToActive();
    }
  });
}

/** 画面左上のインジケーターを描く。 */
export function drawLfoIndicator(ctx) {
  const label = lfoDestination === LFO_DEST_VOLUME ? 'Tremolo (Gain FG)' : 'Vibrato (Pitch FG)';
  const x = 16;
  const barW = 100;
  const barH = 6;

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
