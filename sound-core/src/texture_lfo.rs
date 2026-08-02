// ---------------------------------------------------------------------------
// 質感LFO（TextureLfo）
//
// VCO実装（FM/PCM/減算合成等）に依存しない、モジュレーション層の汎用部品
// （元は`ym38x6-core/src/lib.rs`直下→`fm-common`を経てsound-coreへ移設）。
// 適用先（Destination）の解釈はチップ固有の関心事のため、この型には含めない
// （FM合成チップの場合は`fm-common::FmLfoDestination`が担う）。
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

use crate::{lfo_fade_mode_from_index, lfo_offset_from_param, LfoWaveform, PerformanceLfoShape};

/// 質感LFO（spec-sound.md「質感LFO（5波形専用・焼き込み）」節）。旧「チャンネルLFO」を再編し、
/// FGのループ（Floor⇄peak）では表せない5波形（矩形/台形/S&H/Random/Chaos）だけを担う、
/// 焼き込み専用（演奏CCによる補正を受けない）の1基。全項目を`ChannelParams`が所有する。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureLfo {
    /// 0=矩形波/1=台形波/2=S&H/3=Random/4=Chaos。
    pub waveform: u8,
    /// 適用先（0〜255）。解釈はチップ固有（FM合成チップの場合は`FmLfoDestination`と同じ並び）。
    pub destination: u8,
    pub rate: u8,
    pub depth: u8,
    pub delay: u8,
    /// 0=ON-IN/1=ON-OUT/2=OFF-IN/3=OFF-OUT。
    pub fade_mode: u8,
    pub fade_time: u8,
    /// 波形の中心シフト(0〜255、中心128＝オフセットなし)。
    pub offset: u8,
}

impl Default for TextureLfo {
    /// 既定Depth=0で鳴らない（旧パッチ挙動と互換の「無効」状態）。
    fn default() -> Self {
        Self { waveform: 0, destination: 0, rate: 0, depth: 0, delay: 0, fade_mode: 0, fade_time: 0, offset: 128 }
    }
}

/// 質感LFOの5波形インデックス(0〜4)を、内部で再利用する`PerformanceLfo`の
/// 8波形`LfoWaveform`へ写像する（三角/サイン/のこぎりには写像しない＝新エンコーディングの範囲外）。
fn texture_lfo_waveform_to_engine(waveform: u8) -> LfoWaveform {
    match waveform {
        1 => LfoWaveform::Trapezoid,
        2 => LfoWaveform::SampleHold,
        3 => LfoWaveform::Random,
        4 => LfoWaveform::Chaos,
        _ => LfoWaveform::Square,
    }
}

/// 質感LFOの波形/Fade/Offset設定を、`PerformanceLfo`が受け取る
/// `PerformanceLfoShape`へ変換する（rate/delay/destination/depthは別途セッターで設定する）。
pub fn texture_lfo_to_shape(texture_lfo: TextureLfo) -> PerformanceLfoShape {
    PerformanceLfoShape {
        waveform: texture_lfo_waveform_to_engine(texture_lfo.waveform),
        fade_mode: lfo_fade_mode_from_index(texture_lfo.fade_mode),
        fade_time: texture_lfo.fade_time,
        offset: lfo_offset_from_param(texture_lfo.offset),
    }
}
