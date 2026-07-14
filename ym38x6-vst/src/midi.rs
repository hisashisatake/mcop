use std::collections::HashMap;
use ym38x6_core::algorithm::ALGORITHMS;
use ym38x6_core::Ym38x6Patch;

/// MIDI CC値（0.0〜1.0正規化）を本プロジェクトの内部表現（0〜255）に変換
pub(crate) fn cc_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// MIDI CC値（0.0〜1.0正規化）をGM2準拠の7bit値（0〜127）に変換
pub(crate) fn cc_to_u7(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 127.0).round() as u8
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

/// Channel Pressure / Poly Key Pressureの加算先（NRPN(0,16)/(0,17)、spec.md AT Destination参照）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum AtDestination {
    #[default]
    LfoPmd,
    LfoAmd,
    FilterCutoff,
    FilterResonance,
    TlAllOps,
    TlCarriers,
}

impl AtDestination {
    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            0 => AtDestination::LfoPmd,
            1 => AtDestination::LfoAmd,
            2 => AtDestination::FilterCutoff,
            3 => AtDestination::FilterResonance,
            4 => AtDestination::TlAllOps,
            _ => AtDestination::TlCarriers,
        }
    }
}

/// Channel Pressure / Poly Key Pressureの加算モデルを指定ノートのパッチへ適用する。
/// `実効値 = clamp(ベース値 + プレッシャー値, 0, 255)`（spec.md AT Destination参照）。
/// `note_channels`の借用と`engine`への可変アクセスを同じループ内で行うため、
/// `&self`を取らないフリー関数にしている。
pub(crate) fn apply_at_modulation(
    note: u8,
    at_destination: AtDestination,
    poly_at_destination: AtDestination,
    channel_pressure: u8,
    poly_pressure: &HashMap<u8, u8>,
    patch: &mut Ym38x6Patch,
) {
    let pressure_for = |destination: AtDestination| -> u8 {
        let mut total: u16 = 0;
        if at_destination == destination {
            total += channel_pressure as u16;
        }
        if poly_at_destination == destination {
            total += *poly_pressure.get(&note).unwrap_or(&0) as u16;
        }
        total.min(255) as u8
    };
    let add = |base: u8, pressure: u8| (base as u16 + pressure as u16).min(255) as u8;

    let pmd = pressure_for(AtDestination::LfoPmd);
    if pmd > 0 {
        patch.channel.chip_lfo_pmd = add(patch.channel.chip_lfo_pmd, pmd);
    }
    let amd = pressure_for(AtDestination::LfoAmd);
    if amd > 0 {
        patch.channel.chip_lfo_amd = add(patch.channel.chip_lfo_amd, amd);
    }
    let cutoff = pressure_for(AtDestination::FilterCutoff);
    if cutoff > 0 {
        patch.channel.filter_cutoff = add(patch.channel.filter_cutoff, cutoff);
    }
    let resonance = pressure_for(AtDestination::FilterResonance);
    if resonance > 0 {
        patch.channel.filter_resonance = add(patch.channel.filter_resonance, resonance);
    }
    let tl_all = pressure_for(AtDestination::TlAllOps);
    if tl_all > 0 {
        for op in patch.operators.iter_mut() {
            op.tl = add(op.tl, tl_all);
        }
    }
    let tl_carriers = pressure_for(AtDestination::TlCarriers);
    if tl_carriers > 0 {
        for &i in ALGORITHMS[patch.channel.algorithm as usize].carriers {
            patch.operators[i].tl = add(patch.operators[i].tl, tl_carriers);
        }
    }
}
