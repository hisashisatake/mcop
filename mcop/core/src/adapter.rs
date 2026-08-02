// ---------------------------------------------------------------------------
// Adapter: 既存.38x6（レート方式5段EG、`ym38x6_core::Ym38x6Patch`）を
// mcopのN点Time/Level方式（`McopPatch`）へ変換する。
//
// キーオン部（AR/D1R/D2R区間）はレートテーブルの秒数×レベル差から各セグメントの
// 所要時間を逆算する厳密変換。リリースはキーオフ時の開始レベルが実行時にしか
// 決まらないため厳密変換不可能で、フルスパン（1.0→0.0の所要秒数）を公称値として使う
// （memory `project_4point_tl_eg_decision.md`参照）。
// ---------------------------------------------------------------------------

use sound_core::{seconds_to_time, EgParams, TimeEgParams, TimeStage, MAX_STAGES};
use ym38x6_core::{OperatorParams, Ym38x6Patch};

use crate::{McopBipolarFg, McopChannelParams, McopOperatorParams, McopPatch};

fn seconds_for_ar(rate: u8) -> Option<f32> {
    if rate == 0 {
        None
    } else {
        Some(1.0 / sound_core::eg::ar_to_delta(rate, 1.0))
    }
}

fn seconds_for_decay(rate: u8) -> Option<f32> {
    if rate == 0 {
        None
    } else {
        Some(1.0 / sound_core::eg::decay_to_delta(rate, 1.0))
    }
}

/// RRはEgParams::rr=0でも284.9秒（reg=0相当）の有限値で、レート方式のフリーズ特殊値を持たない
/// （`Eg::rr_to_delta`のコメント参照）。
fn seconds_for_rr(rate: u8) -> f32 {
    1.0 / sound_core::eg::rr_to_delta(rate, 1.0)
}

/// 秒数→time値。30秒（TimeEgの上限）を超える場合は警告を積んでからクランプする
/// （`seconds_to_time`自体もクランプするが、ここでは変換の実害をユーザーに可視化する）。
fn time_for_seconds(seconds: f32, warnings: &mut Vec<String>, label: &str, what: &str) -> u8 {
    if seconds >= 30.0 {
        warnings.push(format!(
            "{label}: {what} {seconds:.1}秒はTimeEgの上限30秒を超えるためtime=255にクランプした"
        ));
    }
    seconds_to_time(seconds)
}

/// EGの5段形状(ar/d1r/d1l/d2r/rr + floor/loop/curve)をTimeEgParamsへ変換する共通ロジック。
/// オペレーターEGとPitch/Cutoff/Gain FGのeg部分（`EgParams`の`delay`以外の7フィールド）が
/// 同じ形なので、この1関数で両方をまかなう。
fn convert_eg_shape(
    ar: u8,
    d1r: u8,
    d1l: u8,
    d2r: u8,
    rr: u8,
    floor: u8,
    loop_enabled: u8,
    curve: u8,
    warnings: &mut Vec<String>,
    label: &str,
) -> TimeEgParams {
    let mut stages = [TimeStage::default(); MAX_STAGES];

    // ar=0（レート方式のフリーズ特殊値）＝常時無音。time=0（瞬時）と意味が真逆なので、
    // 「level=0の単一段で静止」として表現する（`time=0`のまま使うと「瞬時に0へ到達」になり
    // 意味は同じだが、フリーズ由来だと分かるよう警告を残す）。
    let Some(ar_sec) = seconds_for_ar(ar) else {
        warnings.push(format!("{label}: ar=0（フリーズ＝常時無音）を単一段(level=0)に変換した"));
        stages[0] = TimeStage { time: 0, level: 0, curve };
        let rr_time = time_for_seconds(seconds_for_rr(rr), warnings, label, "release(rr)");
        stages[1] = TimeStage { time: rr_time, level: 0, curve };
        return TimeEgParams {
            stages,
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 0,
            loop_end: 0,
            release_start: 1,
        };
    };
    let ar_time_full = time_for_seconds(ar_sec, warnings, label, "attack(ar)");

    if loop_enabled != 0 {
        if let Some(d1r_sec_full) = seconds_for_decay(d1r) {
            // Attack(0→peak, ar基準) → LoopDown(peak→floor, d1r基準) → LoopUp(floor→peak, ar基準)。
            // LoopDown/LoopUpの移動距離は(1.0-floor)倍のため、フルスパン秒数にその比率を掛ける。
            let frac = 1.0 - floor as f32 / 255.0;
            stages[0] = TimeStage { time: ar_time_full, level: 255, curve };
            let down_time = time_for_seconds(d1r_sec_full * frac, warnings, label, "loop down(d1r)");
            stages[1] = TimeStage { time: down_time, level: floor, curve };
            let up_time = time_for_seconds(ar_sec * frac, warnings, label, "loop up(ar)");
            stages[2] = TimeStage { time: up_time, level: 255, curve };
            let rr_time = time_for_seconds(seconds_for_rr(rr), warnings, label, "release(rr)");
            stages[3] = TimeStage { time: rr_time, level: 0, curve };
            return TimeEgParams {
                stages,
                stage_count: 4,
                loop_enabled: 1,
                loop_start: 1,
                loop_end: 2,
                release_start: 3,
            };
        }
        warnings.push(format!(
            "{label}: loop=1かつd1r=0（フリーズ）はループが成立しないため静止扱いに変換した"
        ));
        // フォールスルーしてd1r=0のワンショット扱いにする。
    }

    stages[0] = TimeStage { time: ar_time_full, level: 255, curve };
    let Some(d1r_sec) = seconds_for_decay(d1r) else {
        // d1r=0：D1Lへ降りずpeak(255)に張り付く（`Eg`のDecay1フリーズと同じ）。
        let rr_time = time_for_seconds(seconds_for_rr(rr), warnings, label, "release(rr)");
        stages[1] = TimeStage { time: rr_time, level: 0, curve };
        return TimeEgParams {
            stages,
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 0,
            loop_end: 0,
            release_start: 1,
        };
    };
    let d1_time = time_for_seconds(d1r_sec * (1.0 - d1l as f32 / 255.0), warnings, label, "decay1(d1r)");
    stages[1] = TimeStage { time: d1_time, level: d1l, curve };

    let Some(d2r_sec) = seconds_for_decay(d2r) else {
        // d2r=0：D1Lで張り付く。
        let rr_time = time_for_seconds(seconds_for_rr(rr), warnings, label, "release(rr)");
        stages[2] = TimeStage { time: rr_time, level: 0, curve };
        return TimeEgParams {
            stages,
            stage_count: 3,
            loop_enabled: 0,
            loop_start: 1,
            loop_end: 1,
            release_start: 2,
        };
    };
    let d2_time = time_for_seconds(d2r_sec * (d1l as f32 / 255.0), warnings, label, "decay2(d2r)");
    stages[2] = TimeStage { time: d2_time, level: 0, curve };
    let rr_time = time_for_seconds(seconds_for_rr(rr), warnings, label, "release(rr)");
    stages[3] = TimeStage { time: rr_time, level: 0, curve };
    TimeEgParams { stages, stage_count: 4, loop_enabled: 0, loop_start: 2, loop_end: 2, release_start: 3 }
}

/// 段の先頭にlevel=0のプラトー段を1つ挿入し、loop_start/loop_end/release_startを+1シフトする
/// （`EgParams::delay`＝キーオンからAR開始までの遅延をTimeEgで表現する。8段上限に収まる前提）。
fn prepend_delay_stage(params: &mut TimeEgParams, delay: u8, warnings: &mut Vec<String>, label: &str) {
    let delay_time = time_for_seconds(sound_core::delay_to_seconds(delay), warnings, label, "delay");
    let count = params.stage_count as usize;
    if count >= MAX_STAGES {
        warnings.push(format!("{label}: delay挿入で段数が上限{MAX_STAGES}を超えるため打ち切った"));
        return;
    }
    for i in (0..count).rev() {
        params.stages[i + 1] = params.stages[i];
    }
    params.stages[0] = TimeStage { time: delay_time, level: 0, curve: 0 };
    params.stage_count = (count + 1) as u8;
    params.loop_start += 1;
    params.loop_end += 1;
    params.release_start += 1;
}

/// オペレーターEG（`OperatorParams`のar/d1r/d1l/d2r/rr/floor/loop/curve）をTimeEgParamsへ変換する。
/// 警告が必要な場合は`convert_patch`経由で確認すること（この関数は静かにクランプする）。
pub fn convert_operator_eg(op: &OperatorParams) -> TimeEgParams {
    let mut warnings = Vec::new();
    convert_eg_shape(op.ar, op.d1r, op.d1l, op.d2r, op.rr, op.floor, op.loop_enabled, op.curve, &mut warnings, "op")
}

/// Pitch/Cutoff/Gain FGの`EgParams`（delay込み）をTimeEgParamsへ変換する。
/// Gain FGのrr=0特例（透過既定を維持する据え置き変換）は`convert_patch`側で追加適用する
/// （この関数はFG種別を区別しない汎用変換のみ行う）。
pub fn convert_fg_eg(eg: &EgParams) -> TimeEgParams {
    let mut warnings = Vec::new();
    let mut result = convert_eg_shape(
        eg.ar, eg.d1r, eg.d1l, eg.d2r, eg.rr, eg.floor, eg.loop_enabled, eg.curve, &mut warnings, "fg",
    );
    if eg.delay > 0 {
        prepend_delay_stage(&mut result, eg.delay, &mut warnings, "fg");
    }
    result
}

/// `.38x6`（`Ym38x6Patch`）をmcopの`McopPatch`へ変換する。クランプ・特殊ケース変換が
/// 発生した箇所は警告としてラベル付きで返す（例: `"op2: decay2(d2r) 45.3秒は..."`）。
pub fn convert_patch(src: &Ym38x6Patch) -> (McopPatch, Vec<String>) {
    let mut warnings = Vec::new();

    let operators = std::array::from_fn(|i| {
        let src_op = &src.operators[i];
        let eg = convert_eg_shape(
            src_op.ar,
            src_op.d1r,
            src_op.d1l,
            src_op.d2r,
            src_op.rr,
            src_op.floor,
            src_op.loop_enabled,
            src_op.curve,
            &mut warnings,
            &format!("op{i}"),
        );
        McopOperatorParams {
            tl: src_op.tl,
            eg,
            mul: src_op.mul,
            dt1: src_op.dt1,
            ksr: src_op.ksr,
            am_enable: src_op.am_enable,
            velocity_sensitivity: src_op.velocity_sensitivity,
            waveform: src_op.waveform,
            op_fine_tune: src_op.op_fine_tune,
            eg_shift: src_op.eg_shift,
            level_scale: src_op.level_scale,
            velocity_gain: src_op.velocity_gain,
        }
    });

    let ch = &src.channel;

    let mut pitch_fg_eg = convert_eg_shape(
        ch.pitch_fg.eg.ar,
        ch.pitch_fg.eg.d1r,
        ch.pitch_fg.eg.d1l,
        ch.pitch_fg.eg.d2r,
        ch.pitch_fg.eg.rr,
        ch.pitch_fg.eg.floor,
        ch.pitch_fg.eg.loop_enabled,
        ch.pitch_fg.eg.curve,
        &mut warnings,
        "pitch_fg",
    );
    if ch.pitch_fg.eg.delay > 0 {
        prepend_delay_stage(&mut pitch_fg_eg, ch.pitch_fg.eg.delay, &mut warnings, "pitch_fg");
    }

    let mut cutoff_fg_eg = convert_eg_shape(
        ch.cutoff_fg.eg.ar,
        ch.cutoff_fg.eg.d1r,
        ch.cutoff_fg.eg.d1l,
        ch.cutoff_fg.eg.d2r,
        ch.cutoff_fg.eg.rr,
        ch.cutoff_fg.eg.floor,
        ch.cutoff_fg.eg.loop_enabled,
        ch.cutoff_fg.eg.curve,
        &mut warnings,
        "cutoff_fg",
    );
    if ch.cutoff_fg.eg.delay > 0 {
        prepend_delay_stage(&mut cutoff_fg_eg, ch.cutoff_fg.eg.delay, &mut warnings, "cutoff_fg");
    }

    let mut gain_fg_eg = convert_eg_shape(
        ch.gain_fg.ar,
        ch.gain_fg.d1r,
        ch.gain_fg.d1l,
        ch.gain_fg.d2r,
        ch.gain_fg.rr,
        ch.gain_fg.floor,
        ch.gain_fg.loop_enabled,
        ch.gain_fg.curve,
        &mut warnings,
        "gain_fg",
    );
    if ch.gain_fg.delay > 0 {
        prepend_delay_stage(&mut gain_fg_eg, ch.gain_fg.delay, &mut warnings, "gain_fg");
    }
    // Gain FGのrr=0（透過既定、ゲートを閉じない）特例：リリース段を「現在の静止レベルのまま
    // 瞬時据え置き」に書き換える。フルスパン秒数で0へ向かわせると、離鍵後にゲインが本当に
    // 閉じてしまいキャリア本来のリリース尾を打ち消す（default_gain_fgの設計意図に反する）。
    if ch.gain_fg.rr == 0 && ch.gain_fg.loop_enabled == 0 {
        let release_idx = gain_fg_eg.release_start as usize;
        let settle_level = gain_fg_eg.stages[gain_fg_eg.loop_end as usize].level;
        gain_fg_eg.stages[release_idx] = TimeStage { time: 0, level: settle_level, curve: 0 };
        warnings.push(format!(
            "gain_fg: rr=0（透過既定）を維持するためリリース段をlevel={settle_level}で据え置きに変換した"
        ));
    }

    let channel = McopChannelParams {
        algorithm: ch.algorithm,
        feedback: ch.feedback,
        chip_lfo_freq: ch.chip_lfo_freq,
        chip_lfo_pmd: ch.chip_lfo_pmd,
        chip_lfo_amd: ch.chip_lfo_amd,
        chip_lfo_delay: ch.chip_lfo_delay,
        pms: ch.pms,
        ams: ch.ams,
        filter_cutoff: ch.filter_cutoff,
        filter_resonance: ch.filter_resonance,
        filter_type: ch.filter_type,
        filter_self_oscillation: ch.filter_self_oscillation,
        pitch_fg: McopBipolarFg { eg: pitch_fg_eg, depth: ch.pitch_fg.depth },
        cutoff_fg: McopBipolarFg { eg: cutoff_fg_eg, depth: ch.cutoff_fg.depth },
        gain_fg: gain_fg_eg,
        texture_lfo: ch.texture_lfo,
    };

    (McopPatch { operators, channel }, warnings)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sound_core::{Eg, TimeEg};

    /// classic条件（loop=0/curve=0/floor=0）のEG代表グリッドで、`Eg::tick`と変換後
    /// `TimeEg::tick`の(1)サステインレベル一致 (2)閾値到達サンプル時刻の一致を検証する。
    /// レート→時間変換が「別の描き方をしただけで実質同じ軌道」であることの実行可能な証明。
    #[test]
    fn adapter_matches_classic_eg_trajectory() {
        let sr = 44100.0;
        let cases: &[(u8, u8, u8, u8, u8)] = &[
            (200, 150, 180, 80, 150),
            (255, 200, 128, 40, 200),
            (100, 60, 220, 0, 100), // d2r=0（D1Lで張り付く）
            (150, 0, 255, 0, 90),   // d1r=0（peakで張り付く）
        ];
        for &(ar, d1r, d1l, d2r, rr) in cases {
            let eg_params = EgParams::classic(ar, d1r, d1l, d2r, rr);
            let time_params = convert_eg_shape(ar, d1r, d1l, d2r, rr, 0, 0, 0, &mut Vec::new(), "test");

            let mut classic = Eg::new();
            classic.note_on();
            let mut converted = TimeEg::new();
            converted.note_on();

            // 「targetにピタリ一致するサンプル」は1サンプルあたりのdeltaが粗いと存在しない
            // ことがある（線形ランプが1e-3幅の窓を跨いで通過してしまう）ため、target付近の
            // 「初めて上回った瞬間」を到達時刻とする（アタックが単調増加である前提に依存）。
            let target = d1l as f32 / 255.0;
            let mut classic_reach: Option<usize> = None;
            let mut converted_reach: Option<usize> = None;
            for i in 0..200_000 {
                let a = classic.tick(sr, eg_params, 1.0);
                let b = converted.tick(sr, time_params, 1.0);
                if classic_reach.is_none() && a >= target {
                    classic_reach = Some(i);
                }
                if converted_reach.is_none() && b >= target {
                    converted_reach = Some(i);
                }
                if classic_reach.is_some() && converted_reach.is_some() {
                    break;
                }
            }
            let (c, t) = (
                classic_reach.expect("classic should reach sustain"),
                converted_reach.expect("converted should reach sustain"),
            );
            let tolerance = ((c as f64) * 0.05).max(200.0) as usize;
            assert!(
                (c as i64 - t as i64).unsigned_abs() as usize <= tolerance,
                "case ar={ar} d1r={d1r} d1l={d1l} d2r={d2r}: classic reached at {c}, converted at {t}, tolerance {tolerance}"
            );

            // 十分後のサステインレベルも一致すること。
            let mut final_classic = 0.0;
            let mut final_converted = 0.0;
            for _ in 0..5000 {
                final_classic = classic.tick(sr, eg_params, 1.0);
                final_converted = converted.tick(sr, time_params, 1.0);
            }
            assert!(
                (final_classic - final_converted).abs() < 0.02,
                "case ar={ar} d1r={d1r} d1l={d1l} d2r={d2r}: sustain level mismatch classic={final_classic} converted={final_converted}"
            );
        }
    }

    #[test]
    fn ar_zero_converts_to_silent_single_stage_with_warning() {
        let mut warnings = Vec::new();
        let params = convert_eg_shape(0, 100, 128, 80, 150, 0, 0, 0, &mut warnings, "test");
        assert_eq!(params.stages[0].level, 0);
        assert!(warnings.iter().any(|w| w.contains("ar=0")));
    }

    #[test]
    fn very_slow_rate_triggers_clamp_warning() {
        let mut warnings = Vec::new();
        // decay_to_delta(1, 1.0)のフルスパンは284.9秒 → 30秒上限を超える。
        let _ = convert_eg_shape(200, 1, 128, 0, 200, 0, 0, 0, &mut warnings, "test");
        assert!(warnings.iter().any(|w| w.contains("30秒")), "expected a clamp warning, got {warnings:?}");
    }

    #[test]
    fn loop_conversion_oscillates_between_floor_and_peak() {
        let sr = 44100.0;
        let params = convert_eg_shape(200, 200, 0, 0, 200, 64, 1, 0, &mut Vec::new(), "test");
        let mut eg = TimeEg::new();
        eg.note_on();
        for _ in 0..40000 {
            if eg.tick(sr, params, 1.0) >= 0.99 {
                break;
            }
        }
        let mut max_seen = 0.0f32;
        let mut min_seen = 1.0f32;
        for _ in 0..40000 {
            let level = eg.tick(sr, params, 1.0);
            max_seen = max_seen.max(level);
            min_seen = min_seen.min(level);
        }
        assert!(max_seen > 0.99, "expected to reach peak, got {max_seen}");
        assert!((min_seen - 64.0 / 255.0).abs() < 0.02, "expected to reach floor, got {min_seen}");
        assert!(!eg.is_idle());
    }

    #[test]
    fn convert_patch_gain_fg_rr_zero_stays_transparent_after_release() {
        let src = Ym38x6Patch::default(); // channel.gain_fgは既定でrr=0（透過既定）
        let (patch, warnings) = convert_patch(&src);
        assert!(warnings.iter().any(|w| w.contains("gain_fg") && w.contains("透過")));

        let sr = 44100.0;
        let mut eg = TimeEg::new();
        eg.note_on();
        for _ in 0..1000 {
            eg.tick(sr, patch.channel.gain_fg, 1.0);
        }
        let before = eg.tick(sr, patch.channel.gain_fg, 1.0);
        eg.note_off();
        let mut after = before;
        for _ in 0..44100 {
            after = eg.tick(sr, patch.channel.gain_fg, 1.0);
        }
        assert!(
            (before - after).abs() < 1e-6,
            "gain_fg rr=0 should stay transparent (not close the gate) after note_off: before={before} after={after}"
        );
    }

    #[test]
    fn convert_patch_default_round_trips_through_serde() {
        let src = Ym38x6Patch::default();
        let (patch, _warnings) = convert_patch(&src);
        let json = serde_json::to_string(&patch).expect("serialize");
        let restored: McopPatch = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(patch, restored);
    }
}
