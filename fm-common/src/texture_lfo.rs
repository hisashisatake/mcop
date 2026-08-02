// ---------------------------------------------------------------------------
// 質感LFO（TextureLfo）+ LFO適用先（FmLfoDestination）
//
// `ym38x6-core`/`op505-core`共通で使う、EG非依存のFM合成汎用部品
// （元は`ym38x6-core/src/lib.rs`直下に定義されていたものをfm-commonへ切り出した）。
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};
use sound_core::{lfo_fade_mode_from_index, lfo_offset_from_param, LfoWaveform, PerformanceLfoShape};

/// 質感LFO（spec-sound.md「質感LFO（5波形専用・焼き込み）」節）。旧「チャンネルLFO」を再編し、
/// FGのループ（Floor⇄peak）では表せない5波形（矩形/台形/S&H/Random/Chaos）だけを担う、
/// 焼き込み専用（演奏CCによる補正を受けない）の1基。全項目を`ChannelParams`が所有する。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureLfo {
    /// 0=矩形波/1=台形波/2=S&H/3=Random/4=Chaos。
    pub waveform: u8,
    /// 0=Pitch/1=Volume/2=TL（キャリア一括）/3=Cutoff/4=未接続（`FmLfoDestination`と同じ並び）。
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

/// 質感LFOの5波形インデックス(0〜4)を、内部で再利用する`sound_core::PerformanceLfo`の
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

/// 質感LFOの波形/Fade/Offset設定を、`sound_core::PerformanceLfo`が受け取る
/// `PerformanceLfoShape`へ変換する（rate/delay/destination/depthは別途セッターで設定する）。
pub fn texture_lfo_to_shape(texture_lfo: TextureLfo) -> PerformanceLfoShape {
    PerformanceLfoShape {
        waveform: texture_lfo_waveform_to_engine(texture_lfo.waveform),
        fade_mode: lfo_fade_mode_from_index(texture_lfo.fade_mode),
        fade_time: texture_lfo.fade_time,
        offset: lfo_offset_from_param(texture_lfo.offset),
    }
}

/// パフォーマンスLFOの適用先。共通Destination（Pitch/Volume）に加え、
/// FM合成チップ共通の拡張Destination（TLキャリア一括、Cutoff）を持つ。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum FmLfoDestination {
    #[default]
    Pitch,
    Volume,
    TlCarrier,
    /// フィルターCutoffへの持続的な変調（オートワウ）。Filter EG Depth（キーオン一発の変調）
    /// とは独立に積み重なる（`Channel::tick`でLFOがシフトした基準Cutoffを、Filter EGがさらに変調する）。
    Cutoff,
    /// どこにも接続されていない（質感LFOパッチベイでケーブルをTEXTURE LFOパネル自身へ
    /// ドロップした状態）。LFOは`tick`し続けるが、いずれの変調ターゲットへも出力しない。
    Unplugged,
}

impl FmLfoDestination {
    /// 0〜255からの変換（質感LFOの`Destination`フィールド用、`FilterType::from_u8`と同じ慣習）。
    /// 0=Pitch/1=Volume/2=TL（キャリア一括）/3=Cutoff/4=未接続/5以上=Pitchへフォールバック。
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Volume,
            2 => Self::TlCarrier,
            3 => Self::Cutoff,
            4 => Self::Unplugged,
            _ => Self::Pitch,
        }
    }
}
