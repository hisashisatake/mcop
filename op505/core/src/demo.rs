//! OP505のデモパッチ（TimeEg固有の表現力を示す組み込み音色）。
//!
//! `examples/op505_probe.rs`のm1/m2/m3として試聴・確定した形を正式APIへ昇格したもの。
//! Adapterで`.38x6`を変換したパッチは「ym38x6と同じ軌道の折れ線」にしかならないため、
//! レート方式では書けない形（静止を挟んだループ・多段プラトー）はこのデモパッチだけが示せる。
//! gesture-appのエンジン切替UI（OP505選択時のデモ音色リスト）や、将来のプリセットバンクの
//! 種として使う。
//!
//! 各パッチの出自はTimeEgプローブ（`sound-core/examples/time_eg_probe.rs`）の試聴実験：
//! - m1 = e3（静止を挟んだ音量2値スイッチ。D/E群試聴で最も刺さった本命ユースケース）
//! - m2 = 多段ループをモジュレーターEGへ適用（レート方式では原理的に書けない形）
//! - m3 = d1（階段状プラトー）をPitch FGへ適用

use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};

use crate::{Op505BipolarFg, Op505OperatorParams, Op505Patch};

/// `demo_patch`のインデックスに対応する表示名（UI用）。
pub const DEMO_NAMES: [&str; 3] = [
    "Gain Switch (e3)",
    "Modulator Multi-stage",
    "Pitch Steps",
];

/// デモパッチを返す（0=Gain Switch / 1=Modulator Multi-stage / 2=Pitch Steps）。
/// 範囲外は`None`。
pub fn demo_patch(index: u8) -> Option<Op505Patch> {
    match index {
        0 => Some(gain_switch_patch()),
        1 => Some(modulator_multistage_patch()),
        2 => Some(pitch_steps_patch()),
        _ => None,
    }
}

fn stages(entries: &[(u8, u8, u8)]) -> [TimeStage; MAX_STAGES] {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    for (i, &(time, level, curve)) in entries.iter().enumerate() {
        stages[i] = TimeStage { time, level, curve };
    }
    stages
}

/// 瞬時に満レベルへ到達しそのまま無限サスティンするEG（通常のオペレーターEG用の既定形）。
fn instant_sustain_eg() -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[(0, 255, 0)]),
        stage_count: 1,
        loop_enabled: 0,
        loop_start: 0,
        loop_end: 0,
        release_start: 0,
    }
}

fn plain_operator(tl: u8, mul: u8, dt1: u8) -> Op505OperatorParams {
    Op505OperatorParams {
        tl,
        eg: instant_sustain_eg(),
        mul,
        dt1,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: 128,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    }
}

/// m1: e3（TimeEgプローブ）と全く同じ形の音量2値スイッチをGain FGに設定した、
/// アルゴリズム4（(O1→O2)+(O3→O4)の2系統FMペア）のパッチ。
fn gain_switch_patch() -> Op505Patch {
    let mut patch = Op505Patch::default();
    patch.operators = [
        plain_operator(190, 2, 128),
        plain_operator(255, 1, 128),
        plain_operator(190, 2, 140), // 2系統目はわずかにデチューンして厚みを出す
        plain_operator(255, 1, 140),
    ];
    patch.channel.algorithm = 4;
    patch.channel.gain_fg = TimeEgParams {
        stages: stages(&[
            (15, 230, 0), // 高い位置へ
            (40, 230, 0), // 静止(高)
            (15, 40, 0),  // 低い位置へ
            (40, 40, 0),  // 静止(低)
            (100, 0, 0),  // リリース
        ]),
        stage_count: 5,
        loop_enabled: 1,
        loop_start: 0,
        loop_end: 3,
        release_start: 4,
    };
    patch
}

/// m2: アルゴリズム0（O1→O2→O3→O4、キャリアはO4）。キャリアを直接変調するO3(index 2)の
/// オペレーターEGを多段ループ化し、変調指数（＝明るさ）が折れ線状に往復する様子を聴く
/// （レート方式のEGでは「速いアタック＋任意形状のループ」が原理的に書けなかった）。
fn modulator_multistage_patch() -> Op505Patch {
    let mut patch = Op505Patch::default();
    let silent = plain_operator(0, 1, 128);
    let mut modulator = plain_operator(220, 1, 128);
    modulator.eg = TimeEgParams {
        stages: stages(&[
            (20, 220, 0), // 明るい位置へ
            (50, 220, 0), // 静止(明)
            (20, 80, 0),  // 暗い位置へ
            (50, 80, 0),  // 静止(暗)
        ]),
        stage_count: 4,
        loop_enabled: 1,
        loop_start: 0,
        loop_end: 3,
        release_start: 3,
    };
    let carrier = plain_operator(255, 1, 128);
    patch.operators = [silent, silent, modulator, carrier];
    patch.channel.algorithm = 0;
    patch
}

/// m3: アルゴリズム7（単一キャリアop0のみ有効）。Pitch FGにTimeEgプローブd1と同じ形の
/// 階段状プラトーを設定し、アルペジオ的なピッチステップを聴く。
fn pitch_steps_patch() -> Op505Patch {
    let mut patch = Op505Patch::default();
    patch.operators = [
        plain_operator(255, 1, 128),
        plain_operator(0, 1, 128),
        plain_operator(0, 1, 128),
        plain_operator(0, 1, 128),
    ];
    patch.channel.algorithm = 7;
    patch.channel.pitch_fg = Op505BipolarFg {
        eg: TimeEgParams {
            stages: stages(&[
                (15, 90, 0),  // 第1段への上昇(速い)
                (45, 90, 0),  // 静止(プラトー)
                (15, 170, 0), // 第2段への上昇
                (45, 170, 0), // 静止(プラトー)
                (15, 255, 0), // 第3段への上昇
                (45, 255, 0), // 静止(プラトー)
                (25, 20, 0),  // 速いリセット
                (110, 0, 0),  // リリース
            ]),
            stage_count: 8,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 6,
            release_start: 7,
        },
        depth: 200, // バイポーラ、中心128超で+方向（最大約+675セント）
    };
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_patch_returns_some_for_all_names_and_none_outside() {
        for i in 0..DEMO_NAMES.len() as u8 {
            assert!(demo_patch(i).is_some(), "demo_patch({i}) はSomeのはず");
        }
        assert!(demo_patch(DEMO_NAMES.len() as u8).is_none());
        assert!(demo_patch(u8::MAX).is_none());
    }

    #[test]
    fn gain_switch_loops_gain_fg_with_release_tail() {
        // m1の核＝Gain FGが「高→静止→低→静止」の4段ループ+専用リリース段であること。
        let p = demo_patch(0).unwrap();
        let fg = &p.channel.gain_fg;
        assert_eq!(fg.loop_enabled, 1);
        assert_eq!((fg.loop_start, fg.loop_end), (0, 3));
        assert_eq!(fg.release_start, 4);
        assert_eq!(fg.stage_count, 5);
        // 静止段=目標レベルが直前と同値（level 230→230、40→40）。
        assert_eq!(fg.stages[0].level, fg.stages[1].level);
        assert_eq!(fg.stages[2].level, fg.stages[3].level);
    }

    #[test]
    fn modulator_multistage_puts_loop_on_op3_eg() {
        let p = demo_patch(1).unwrap();
        assert_eq!(p.channel.algorithm, 0);
        let eg = &p.operators[2].eg;
        assert_eq!(eg.loop_enabled, 1);
        assert_eq!(eg.stage_count, 4);
    }

    #[test]
    fn pitch_steps_uses_full_eight_stages() {
        let p = demo_patch(2).unwrap();
        let fg = &p.channel.pitch_fg;
        assert_eq!(fg.eg.stage_count, 8);
        assert_eq!(fg.eg.loop_enabled, 1);
        assert!(fg.depth > 128, "中心128超の+方向デチューンのはず");
    }
}
