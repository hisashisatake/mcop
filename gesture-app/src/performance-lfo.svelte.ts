// 演奏系モジュレーション（Vキーでビブラート⇔トレモロ切替）。
// ビブラート(Pitch)はPitch FGのループ、トレモロ(Volume)はGain FGのループへ配線される
// （質感LFO退役に伴い独立したLFOスロットは無くなり、選ばれなかった側のFGは音色プリセット
// 本来の値へ戻される。バックエンド側はsrc-tauri/src/main.rsのop505_set_performance_lfo参照）。
//
// modWheel/lfoDestination/lfoRateは段階Cで$state化した。ドロワー内の表示（LfoSectionコンポーネント）
// はこれを直接読むため、旧bindLfoIndicator()/updateIndicator()のような手動DOM同期は不要。

import { setPerformanceLfo } from './midi.ts';

const LFO_RATE_DEFAULT = 140; // 中程度の速さ
const LFO_RATE_STEP = 8;
const LFO_DELAY = 0;
// PerformanceLfoDestination（main.rs、旧sound-fm::FmLfoDestinationと同じ生値）:
// Unplugged=0/Pitch=1/Volume=2/TlCarrier=3/Cutoff=4
const LFO_DEST_PITCH = 1; // ビブラート
const LFO_DEST_VOLUME = 2; // トレモロ
const MOD_DEPTH_RANGE = 64; // RPN0,5デフォルト（約50セント相当）
const CC77_BASE = 0; // Depthベース値は0固定。深さはマウスホイール（CC1相当）のみで制御

export const performanceLfoState: { modWheel: number; destination: number; rate: number } = $state({
  modWheel: 0, // CC1相当。0〜255
  destination: LFO_DEST_PITCH,
  rate: LFO_RATE_DEFAULT,
});

/** 指定チャンネルへ現在のLFO設定を送る。発音直前に呼ぶ。 */
export function applyTo(channel: number): Promise<unknown> {
  return setPerformanceLfo({
    channel,
    rate: performanceLfoState.rate,
    delay: LFO_DELAY,
    destination: performanceLfoState.destination,
    cc77: CC77_BASE,
    cc1: performanceLfoState.modWheel,
    modDepthRange: MOD_DEPTH_RANGE,
  });
}

/**
 * ホイール（Depth）とV/C/Bキー（行き先・Rate）のハンドラを登録する。
 * @param target ホイールを拾う要素
 * @param activeChannels 変更を即時反映する発音中チャンネル
 */
export function setupPerformanceLfo(target: HTMLElement, activeChannels: () => number[]): void {
  const applyToActive = async () => {
    for (const ch of activeChannels()) {
      await applyTo(ch);
    }
  };

  target.addEventListener(
    'wheel',
    async (e: WheelEvent) => {
      e.preventDefault();
      performanceLfoState.modWheel = Math.max(0, Math.min(255, performanceLfoState.modWheel - Math.sign(e.deltaY) * 8));
      await applyToActive();
    },
    { passive: false },
  );

  window.addEventListener('keydown', async (e) => {
    const key = e.key.toLowerCase();
    if (key === 'v') {
      performanceLfoState.destination = performanceLfoState.destination === LFO_DEST_PITCH ? LFO_DEST_VOLUME : LFO_DEST_PITCH;
      await applyToActive();
    } else if (key === 'c') {
      performanceLfoState.rate = Math.max(0, performanceLfoState.rate - LFO_RATE_STEP);
      await applyToActive();
    } else if (key === 'b') {
      performanceLfoState.rate = Math.min(255, performanceLfoState.rate + LFO_RATE_STEP);
      await applyToActive();
    }
  });
}

/** LfoSectionコンポーネント向けの表示用ラベル・比率。 */
export function lfoDestinationLabel(): string {
  return performanceLfoState.destination === LFO_DEST_VOLUME ? 'Tremolo (Gain FG)' : 'Vibrato (Pitch FG)';
}
