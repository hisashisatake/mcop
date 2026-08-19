// ---------------------------------------------------------------------------
// EG変換: レート方式5段EG（`sound_core::EgParams`が表す ar/d1r/d1l/d2r/rr + floor/loop/curve）
// をop505のN点Time/Level方式（`sound_core::TimeEgParams`）へ変換する。
//
// このモジュールはym38x6-coreに一切依存しない（op505/デフォーク計画Phase 4）。
// キーオン部（AR/D1R/D2R区間）はレートテーブルの秒数×レベル差から各セグメントの
// 所要時間を逆算する厳密変換。リリースはキーオフ時の開始レベルが実行時にしか
// 決まらないため厳密変換不可能で、フルスパン（1.0→0.0の所要秒数）を公称値として使う
// （memory `project_4point_tl_eg_decision.md`参照）。
//
// `opz2op505`/`psr2op505`/`mucom2op505`/`opm2op505`が実機レートを本モジュールの
// レートスケールへ写像した後`convert_eg_shape`に通すことで、段割り当て・特殊ケース・
// 300秒クランプ警告を共有する（旧`.38x6`経由の2段変換と同じ変換ロジックを直接呼ぶ形）。
// ---------------------------------------------------------------------------

use sound_core::{seconds_to_time, EgParams, TimeEgParams, TimeStage, MAX_STAGES};
use sound_fm::chip_lfo::{ams_to_depth, chip_lfo_freq_to_hz};
use sound_fm::mapping::eg_shift_to_db_range;

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

/// 秒数→time値。300秒（TimeEgの上限）を超える場合は警告を積んでからクランプする
/// （`seconds_to_time`自体もクランプするが、ここでは変換の実害をユーザーに可視化する）。
/// レート方式EGの理論最遅値は284.9秒（D1R/D2R rate=1・RR rate=0のフルスパン）なので、
/// `convert_eg_shape`経由の通常変換ではこのクランプは実質発生しない
/// （実発生するのはレート方式の範囲を超えた入力を渡した場合のみ）。
fn time_for_seconds(seconds: f32, warnings: &mut Vec<String>, label: &str, what: &str) -> u8 {
    if seconds >= 300.0 {
        warnings.push(format!(
            "{label}: {what} {seconds:.1}秒はTimeEgの上限300秒を超えるためtime=255にクランプした"
        ));
    }
    seconds_to_time(seconds)
}

/// EGの5段形状(ar/d1r/d1l/d2r/rr + floor/loop/curve)をTimeEgParamsへ変換する共通ロジック。
/// オペレーターEGとPitch/Cutoff/Gain FGのeg部分（`EgParams`の`delay`以外の7フィールド）が
/// 同じ形なので、この1関数で両方をまかなう。
/// `opz2op505`等の直接変換ツールも、実機レートをこのモジュールのレートスケール相当へ
/// 写像した後この関数に通すことで、段割り当て・特殊ケース・300秒クランプ警告を共有する。
pub fn convert_eg_shape(
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
            release_point: 0,
         ..Default::default()};
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
                release_point: 2,
             ..Default::default()};
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
            release_point: 0,
         ..Default::default()};
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
            release_point: 1,
         ..Default::default()};
    };
    let d2_time = time_for_seconds(d2r_sec * (d1l as f32 / 255.0), warnings, label, "decay2(d2r)");
    stages[2] = TimeStage { time: d2_time, level: 0, curve };
    let rr_time = time_for_seconds(seconds_for_rr(rr), warnings, label, "release(rr)");
    stages[3] = TimeStage { time: rr_time, level: 0, curve };
    TimeEgParams { stages, stage_count: 4, loop_enabled: 0, loop_start: 2, release_point: 2 , ..Default::default()}
}

/// 段の先頭にlevel=0のプラトー段を1つ挿入し、loop_start/release_pointを+1シフトする
/// （`EgParams::delay`＝キーオンからAR開始までの遅延をTimeEgで表現する。8段上限に収まる前提）。
pub(crate) fn prepend_delay_stage(params: &mut TimeEgParams, delay: u8, warnings: &mut Vec<String>, label: &str) {
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
    params.release_point += 1;
}

/// Pitch/Cutoff/Gain FGの`EgParams`（delay込み）をTimeEgParamsへ変換する。`label`は
/// 警告メッセージの先頭に付く識別子（例: `"pitch_fg"`）。Gain FGのrr=0特例（透過既定を
/// 維持する据え置き変換）は`apply_transparent_gain_release`を追加で適用すること
/// （この関数はFG種別を区別しない汎用変換のみ行う）。
pub fn convert_fg_eg(eg: &EgParams, warnings: &mut Vec<String>, label: &str) -> TimeEgParams {
    let mut result =
        convert_eg_shape(eg.ar, eg.d1r, eg.d1l, eg.d2r, eg.rr, eg.floor, eg.loop_enabled, eg.curve, warnings, label);
    if eg.delay > 0 {
        prepend_delay_stage(&mut result, eg.delay, warnings, label);
    }
    result
}

/// Gain FGのrr=0（透過既定、ゲートを閉じない）特例を適用する。リリース段を「現在の静止
/// レベルのまま瞬時据え置き」に書き換える。フルスパン秒数で0へ向かわせると、離鍵後に
/// ゲインが本当に閉じてしまいキャリア本来のリリース尾を打ち消す（default_gain_fgの設計
/// 意図に反する）。`src_rr`/`src_loop_enabled`は変換元`EgParams`のrr/loop_enabled値。
pub fn apply_transparent_gain_release(
    gain_fg_eg: &mut TimeEgParams,
    src_rr: u8,
    src_loop_enabled: u8,
    warnings: &mut Vec<String>,
) {
    if src_rr == 0 && src_loop_enabled == 0 {
        // リリース区間は`release_point+1..stage_count`。その先頭段を据え置きへ書き換える
        // （区間が空＝リリース無しならすでに透過なので何もしない）。
        let release_idx = gain_fg_eg.release_point as usize + 1;
        if release_idx >= gain_fg_eg.stage_count as usize {
            return;
        }
        let settle_level = gain_fg_eg.stages[gain_fg_eg.release_point as usize].level;
        gain_fg_eg.stages[release_idx] = TimeStage { time: 0, level: settle_level, curve: 0 };
        warnings.push(format!(
            "gain_fg: rr=0（透過既定）を維持するためリリース段をlevel={settle_level}で据え置きに変換した"
        ));
    }
}

/// CHIP LFOのAM(振幅変調)経路を1オペレーターのEGへ畳み込む
/// （CHIP LFO退役の第二段階、memory `project_chip_lfo_retirement_investigation.md`参照）。
///
/// `env_amp`(dB領域のEG)と`amp_factor`(線形領域のchip AM乗算)が`operator.rs`で完全に独立した
/// 乗算項である以上、AMをEGそのものへ畳み込む選択は「dBレンジへの変換」という近似を伴う
/// （振幅リニアの三角波はdBレンジでは直線にならない）。実データ走査でAM可聴パッチのAM深さは
/// 中央値0.27・7割が1段近似の誤差0.3dB未満に収まると確認済み。
///
/// 対象は「保持区間(`0..=release_point`)がループしておらず(`loop_enabled==0`)、かつ保持区間の
/// 終端レベルが0でない」EGのみ。実データ走査でAM可聴パッチのオペレーターEGはこの条件を
/// 全て満たすか、終端レベル0（無音まで減衰しきる）のいずれかで、後者にAMをかけても
/// 聴こえないため素直にNoneを返す（ループ中のEGT系EGも同様、二重ループは表現できないため対象外）。
/// 呼び出し側はNoneのとき、そのオペレーターの`am_enable`をtrueのまま維持しCHIP LFO側のAMを使う。
///
/// 段構成：保持区間終端(`release_point`)の後に「平坦待機(半周期+delay) → 下降(1/4周期) →
/// 上昇(1/4周期)」の3段を追加しループさせる（`loop_start=release_point+1`,
/// `release_point'=release_point+3`）。CHIP LFOのAMは`clamp(1-triangle*D, 0, 1)`で上側が
/// 常にクランプされるため、実波形は「半周期平坦、半周期で谷へ往復」というこの3段と一致する。
/// 谷レベルは`ΔL = |20·log10(1-D)| / db_range × 255`をサステインレベルから引いた値
/// （0未満はクランプ、`amp_factor`のclampと同じ挙動）。
pub fn apply_chip_lfo_am_to_eg(
    eg: &TimeEgParams,
    ams: u8,
    chip_lfo_amd: u8,
    chip_lfo_freq: u8,
    chip_lfo_delay: u8,
    eg_shift: u8,
) -> Option<TimeEgParams> {
    let depth = ams_to_depth(ams) * (chip_lfo_amd as f32 / 255.0);
    if depth <= 0.0 || eg.loop_enabled != 0 {
        return None;
    }
    let stage_count = eg.stage_count as usize;
    if stage_count + 3 > MAX_STAGES {
        return None;
    }
    let r = eg.release_point as usize;
    let sustain_level = eg.stages[r].level;
    if sustain_level == 0 {
        return None;
    }

    let db_range = eg_shift_to_db_range(eg_shift);
    let delta_db = -20.0 * (1.0 - depth).max(1e-6).log10();
    let delta_level = (delta_db / db_range * 255.0).round();
    let floor_level = (sustain_level as f32 - delta_level).max(0.0) as u8;

    let half_period_seconds = 0.5 / chip_lfo_freq_to_hz(chip_lfo_freq);
    let delay_seconds = chip_lfo_delay as f32 / 255.0 * 10.0;
    let flat_time = seconds_to_time(half_period_seconds + delay_seconds);
    let ramp_time = seconds_to_time(half_period_seconds / 2.0);

    let mut stages = eg.stages;
    for i in (r + 1..stage_count).rev() {
        stages[i + 3] = stages[i];
    }
    stages[r + 1] = TimeStage { time: flat_time, level: sustain_level, curve: 0 };
    stages[r + 2] = TimeStage { time: ramp_time, level: floor_level, curve: 0 };
    stages[r + 3] = TimeStage { time: ramp_time, level: sustain_level, curve: 0 };

    Some(TimeEgParams {
        stages,
        stage_count: (stage_count + 3) as u8,
        loop_enabled: 1,
        loop_start: (r + 1) as u8,
        release_point: (r + 3) as u8,
        ..*eg
    })
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
    fn slowest_rate_no_longer_clamps_against_300sec_ceiling() {
        // decay_to_delta(1, 1.0)のフルスパンは284.9秒（レート方式EGの理論最遅値）。
        // T_MAX=30秒だった頃はここで約9.5倍クロップされていたが、T_MAX=300秒への拡張後は
        // 理論最遅値がT_MAX以内に収まるため、クランプ警告は出ない
        // （memory `project_timeeg_time_field_16bit_consideration.md`参照）。
        let mut warnings = Vec::new();
        let _ = convert_eg_shape(200, 1, 128, 0, 200, 0, 0, 0, &mut warnings, "test");
        assert!(
            !warnings.iter().any(|w| w.contains("クランプ")),
            "expected no clamp warning, got {warnings:?}"
        );
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
    fn apply_transparent_gain_release_keeps_level_after_note_off() {
        let mut warnings = Vec::new();
        let mut params = convert_eg_shape(255, 0, 255, 0, 0, 0, 0, 0, &mut warnings, "test");
        apply_transparent_gain_release(&mut params, 0, 0, &mut warnings);
        assert!(warnings.iter().any(|w| w.contains("gain_fg") && w.contains("透過")));

        let sr = 44100.0;
        let mut eg = TimeEg::new();
        eg.note_on();
        for _ in 0..1000 {
            eg.tick(sr, params, 1.0);
        }
        let before = eg.tick(sr, params, 1.0);
        eg.note_off();
        let mut after = before;
        for _ in 0..44100 {
            after = eg.tick(sr, params, 1.0);
        }
        assert!(
            (before - after).abs() < 1e-6,
            "gain_fg rr=0 should stay transparent (not close the gate) after note_off: before={before} after={after}"
        );
    }

    // -----------------------------------------------------------------------
    // apply_chip_lfo_am_to_eg（CHIP LFO AM経路→オペレーターEG畳み込み）
    // -----------------------------------------------------------------------

    #[test]
    fn apply_chip_lfo_am_to_eg_none_when_depth_zero() {
        let eg = convert_eg_shape(200, 0, 200, 0, 150, 0, 0, 0, &mut Vec::new(), "test");
        assert!(apply_chip_lfo_am_to_eg(&eg, 0, 200, 128, 0, 0).is_none());
        assert!(apply_chip_lfo_am_to_eg(&eg, 200, 0, 128, 0, 0).is_none());
    }

    #[test]
    fn apply_chip_lfo_am_to_eg_none_when_sustain_is_silent() {
        // d1r=0=0でなくとも通常4段でstages[release_point].level==0（D2Rが0まで減衰しきる形）はNone。
        let eg = convert_eg_shape(200, 150, 180, 100, 150, 0, 0, 0, &mut Vec::new(), "test");
        assert_eq!(eg.stages[eg.release_point as usize].level, 0);
        assert!(apply_chip_lfo_am_to_eg(&eg, 200, 200, 128, 0, 0).is_none());
    }

    #[test]
    fn apply_chip_lfo_am_to_eg_none_when_already_looping() {
        // loop_enabled!=0（EGT系）で変換されたEGは対象外。
        let eg = convert_eg_shape(200, 200, 0, 0, 200, 64, 1, 0, &mut Vec::new(), "test");
        assert_eq!(eg.loop_enabled, 1);
        assert!(apply_chip_lfo_am_to_eg(&eg, 200, 200, 128, 0, 0).is_none());
    }

    #[test]
    fn apply_chip_lfo_am_to_eg_builds_loop_around_sustain() {
        // d1r=0（255で張り付く）: stage_count=2, release_point=0, sustain=255。
        let eg = convert_eg_shape(200, 0, 255, 0, 150, 0, 0, 0, &mut Vec::new(), "test");
        assert_eq!(eg.release_point, 0);
        assert_eq!(eg.stages[0].level, 255);

        let result = apply_chip_lfo_am_to_eg(&eg, 255, 255, 128, 0, 0).expect("should migrate");
        assert_eq!(result.stage_count, 5); // 元2段 + AM3段
        assert_eq!(result.loop_enabled, 1);
        assert_eq!(result.loop_start, 1);
        assert_eq!(result.release_point, 3);
        assert_eq!(result.stages[1].level, 255, "平坦段はサステインレベルのまま");
        assert!(result.stages[2].level < 255, "下降段は谷へ向かう");
        assert_eq!(result.stages[3].level, 255, "上昇段はサステインレベルへ戻る");
        assert_eq!(result.stages[2].time, result.stages[3].time, "下降/上昇は対称");
        // 元のリリース段(index1)はindex4へシフトしている。
        assert_eq!(result.stages[4], eg.stages[1]);
    }

    #[test]
    fn apply_chip_lfo_am_to_eg_floor_never_goes_negative() {
        // 深いAM(ほぼ100%)でも谷は0未満にならない。d2r=0（d1rでd1lへ降りて張り付く）で
        // sustain=d1l=40の低いサステインを使う（d1r=0だとd1lが無視されsustain=255になるため）。
        let eg = convert_eg_shape(200, 100, 40, 0, 150, 0, 0, 0, &mut Vec::new(), "test");
        assert_eq!(eg.stages[eg.release_point as usize].level, 40);
        let result = apply_chip_lfo_am_to_eg(&eg, 255, 255, 128, 0, 0).expect("should migrate");
        assert_eq!(result.stages[result.loop_start as usize + 1].level, 0);
    }

    /// 畳み込み後のEGを実際にTimeEgで鳴らし、山谷がsustain/floorへ正しく到達し
    /// ボイス解放（idle化）も従来どおり機能することを確認する統合テスト。
    #[test]
    fn apply_chip_lfo_am_to_eg_oscillates_and_still_releases() {
        let sr = 44100.0;
        // d2r=0（d1rでd1l=200へ降りて張り付く）: release_point段のsustain=200。
        let eg = convert_eg_shape(255, 100, 200, 0, 200, 0, 0, 0, &mut Vec::new(), "test");
        assert_eq!(eg.stages[eg.release_point as usize].level, 200);
        let result = apply_chip_lfo_am_to_eg(&eg, 255, 255, 200, 0, 0).expect("should migrate");

        let mut tick_eg = TimeEg::new();
        tick_eg.note_on();
        // アタック(255到達)＋d1r減衰(200へ、実測で約1秒かかる)の通過中は255〜200の中間値を
        // 経由するため、定常ループに入るまで十分ウォームアップしてから観測する。
        for _ in 0..(sr as usize * 2) {
            tick_eg.tick(sr, result, 1.0);
        }
        let mut max_seen = 0.0f32;
        let mut min_seen = 1.0f32;
        for _ in 0..44100 {
            let level = tick_eg.tick(sr, result, 1.0);
            max_seen = max_seen.max(level);
            min_seen = min_seen.min(level);
        }
        assert!((max_seen - 200.0 / 255.0).abs() < 0.02, "max_seen={max_seen}");
        assert!(min_seen < max_seen - 0.05, "expected audible swing, min={min_seen} max={max_seen}");
        assert!(!tick_eg.is_idle(), "looping AM should never idle on its own");

        tick_eg.note_off();
        let mut became_idle = false;
        for _ in 0..200_000 {
            tick_eg.tick(sr, result, 1.0);
            if tick_eg.is_idle() {
                became_idle = true;
                break;
            }
        }
        assert!(became_idle, "note_off should still walk the release stage to Idle");
    }
}
