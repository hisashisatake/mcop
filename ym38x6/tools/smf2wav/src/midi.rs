//! MIDI CC/RPN/NRPN 解釈のための純ロジック。
//!
//! ここに置く型・関数（`RpnSelection`/`ExpressionDestination`/`apply_expression_modulation`/
//! `cc_to_u8`/`cc_to_u7`/`channel_gain`）は `ym38x6-vst/src/midi.rs` および `lib.rs` の実装を
//! **そのまま複製**したものである。VST はプラグイン(cdylib)クレートで他クレートから
//! 参照できず、`sound-core`/`ym38x6-core` は「MIDI解釈はコア外」（CLAUDE.md）のため
//! 置き場所にできない。ロードマップ（フェーズ7ステップ9/10）の方針に従い smf2wav 側に
//! 最小複製する。**VST側の該当ロジックを変更したらここも追従させること。**
//! （将来 vst/smf2wav 共有の小 midi-interp クレートへ括り出す選択肢はあるが本ステップ外。）

use std::collections::HashMap;

use ym38x6_core::algorithm::ALGORITHMS;
use ym38x6_core::Ym38x6Patch;

/// MIDI CC値（0〜127の7bit生値）を内部表現（0〜255）へ変換する（`min(cc × 2, 255)`）。
/// VST の `cc_to_u8`（正規化0.0〜1.0入力）と等価な整数版。
pub(crate) fn cc_to_u8(value: u8) -> u8 {
    ((value as u16) * 2).min(255) as u8
}

/// MIDI CC値（0〜127）をそのまま7bit値として使う。VST の `cc_to_u7` と等価。
pub(crate) fn cc_to_u7(value: u8) -> u8 {
    value.min(127)
}

/// GM2 の実効音量カーブ = (cc/127)^2。CC7 と CC11 をそれぞれ二乗して積を取る。
/// VST の `channel_gain` の複製。
#[inline]
pub(crate) fn channel_gain(cc7: u8, cc11: u8) -> f32 {
    let v7 = cc7 as f32 / 127.0;
    let v11 = cc11 as f32 / 127.0;
    v7 * v7 * v11 * v11
}

/// CC99/98(NRPN)・CC101/100(RPN) で選択中のパラメーター番号。
/// CC6(Data Entry MSB)はこの選択状態に応じて値を適用する。
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum RpnSelection {
    #[default]
    None,
    Rpn(u8, u8),
    Nrpn(u8, u8),
}

/// 表情コントローラー（AT/CC2/CC4）共通の加算先（NRPN(0,16)/(0,17)/(0,34)/(0,35)、
/// spec.md Expression Destination参照）。元はAT専用（`AtDestination`）だったが、
/// CC2(ブレス)/CC4(フット)も同じ加算モデルへ載せるため全ソース共通の名前へ改称した。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum ExpressionDestination {
    #[default]
    LfoPmd,
    LfoAmd,
    FilterCutoff,
    FilterResonance,
    TlAllOps,
    TlCarriers,
}

impl ExpressionDestination {
    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            0 => ExpressionDestination::LfoPmd,
            1 => ExpressionDestination::LfoAmd,
            2 => ExpressionDestination::FilterCutoff,
            3 => ExpressionDestination::FilterResonance,
            4 => ExpressionDestination::TlAllOps,
            _ => ExpressionDestination::TlCarriers,
        }
    }
}

/// 表情コントローラー（Channel Pressure / Poly Key Pressure / CC2 / CC4）の加算モデルを
/// 指定ノートのパッチへ適用する。`実効値 = clamp(ベース値 + Σソース値, 0, 255)`
/// （spec.md Expression Destination参照）。`sources`は`(値, 行先)`のリスト（CC2/CC4等、
/// チャンネル全体で1系統ずつ）で、Poly Key Pressureのみノート単位のため別引数。
/// VST `apply_expression_modulation` の複製。
pub(crate) fn apply_expression_modulation(
    note: u8,
    sources: &[(u8, ExpressionDestination)],
    poly_at_destination: ExpressionDestination,
    poly_pressure: &HashMap<u8, u8>,
    patch: &mut Ym38x6Patch,
) {
    let pressure_for = |destination: ExpressionDestination| -> u8 {
        let mut total: u16 = 0;
        for &(value, dest) in sources {
            if dest == destination {
                total += value as u16;
            }
        }
        if poly_at_destination == destination {
            total += *poly_pressure.get(&note).unwrap_or(&0) as u16;
        }
        total.min(255) as u8
    };
    let add = |base: u8, pressure: u8| (base as u16 + pressure as u16).min(255) as u8;

    let pmd = pressure_for(ExpressionDestination::LfoPmd);
    if pmd > 0 {
        patch.channel.chip_lfo_pmd = add(patch.channel.chip_lfo_pmd, pmd);
    }
    let amd = pressure_for(ExpressionDestination::LfoAmd);
    if amd > 0 {
        patch.channel.chip_lfo_amd = add(patch.channel.chip_lfo_amd, amd);
    }
    let cutoff = pressure_for(ExpressionDestination::FilterCutoff);
    if cutoff > 0 {
        patch.channel.filter_cutoff = add(patch.channel.filter_cutoff, cutoff);
    }
    let resonance = pressure_for(ExpressionDestination::FilterResonance);
    if resonance > 0 {
        patch.channel.filter_resonance = add(patch.channel.filter_resonance, resonance);
    }
    let tl_all = pressure_for(ExpressionDestination::TlAllOps);
    if tl_all > 0 {
        for op in patch.operators.iter_mut() {
            op.tl = add(op.tl, tl_all);
        }
    }
    let tl_carriers = pressure_for(ExpressionDestination::TlCarriers);
    if tl_carriers > 0 {
        for &i in ALGORITHMS[patch.channel.algorithm as usize].carriers {
            patch.operators[i].tl = add(patch.operators[i].tl, tl_carriers);
        }
    }
}

/// Soft Pedal（CC67）：ON中に新規キーオンしたノートに対して、実効TL（キャリアのみ）と
/// Filter Cutoffを`depth`だけ減算する（spec-sound.md「Soft Pedal（CC67）」参照）。
/// VST `apply_soft_pedal` の複製。
pub(crate) fn apply_soft_pedal(patch: &mut Ym38x6Patch, depth: u8) {
    if depth == 0 {
        return;
    }
    for &i in ALGORITHMS[patch.channel.algorithm as usize].carriers {
        patch.operators[i].tl = patch.operators[i].tl.saturating_sub(depth);
    }
    patch.channel.filter_cutoff = patch.channel.filter_cutoff.saturating_sub(depth);
}
