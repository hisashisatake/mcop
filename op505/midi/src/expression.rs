use op505_core::Op505Patch;
use sound_fm::algorithm::ALGORITHMS;

/// 表情コントローラー（AT/CC2/CC4）共通の加算先（spec-sound.md Expression Destination参照）。
///
/// `LfoAmd`（旧CHIP LFO AMDへの加算）はCHIP LFO完全退役に伴い削除済み。Gain FGは
/// スカラーの「深さ」フィールドを持たないため、CHIP LFO時代と同じ意味の代替先が存在しない
/// （memory `project_chip_lfo_retirement_investigation.md`参照）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum ExpressionDestination {
    /// 旧`LfoPmd`（CHIP LFO PMDへの加算）の後継。Pitch FGの`depth`（振れ幅倍率）へ加算する。
    #[default]
    PitchFgDepth,
    FilterCutoff,
    FilterResonance,
    TlAllOps,
    TlCarriers,
}

impl ExpressionDestination {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => ExpressionDestination::PitchFgDepth,
            1 => ExpressionDestination::FilterCutoff,
            2 => ExpressionDestination::FilterResonance,
            3 => ExpressionDestination::TlAllOps,
            _ => ExpressionDestination::TlCarriers,
        }
    }
}

/// 表情コントローラー（Channel Pressure / Poly Key Pressure / CC2 / CC4）の加算モデルを
/// 指定ノートのパッチへ適用する。`実効値 = clamp(ベース値 + Σソース値, 0, 255)`
/// （spec-sound.md Expression Destination参照）。`sources`は`(値, 行先)`のリスト（CC2/CC4等、
/// MIDIチャンネル全体で1系統ずつ）で、Poly Key Pressureのみノート単位のため別引数。
/// `poly_pressure`はオーディオスレッドでのアロケーションを避けるため`HashMap`ではなく
/// ノート番号(0〜127)で直接引ける固定長配列にしている。
pub fn apply_expression_modulation(
    note: u8,
    sources: &[(u8, ExpressionDestination)],
    poly_at_destination: ExpressionDestination,
    poly_pressure: &[u8; 128],
    patch: &mut Op505Patch,
) {
    let pressure_for = |destination: ExpressionDestination| -> u8 {
        let mut total: u16 = 0;
        for &(value, dest) in sources {
            if dest == destination {
                total += value as u16;
            }
        }
        if poly_at_destination == destination {
            total += poly_pressure[note as usize] as u16;
        }
        total.min(255) as u8
    };
    let add = |base: u8, pressure: u8| (base as u16 + pressure as u16).min(255) as u8;

    let pmd = pressure_for(ExpressionDestination::PitchFgDepth);
    if pmd > 0 {
        patch.channel.pitch_fg.depth = add(patch.channel.pitch_fg.depth, pmd);
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
pub fn apply_soft_pedal(patch: &mut Op505Patch, depth: u8) {
    if depth == 0 {
        return;
    }
    for &i in ALGORITHMS[patch.channel.algorithm as usize].carriers {
        patch.operators[i].tl = patch.operators[i].tl.saturating_sub(depth);
    }
    patch.channel.filter_cutoff = patch.channel.filter_cutoff.saturating_sub(depth);
}
