// ---------------------------------------------------------------------------
// TimeEg — ループ付きN点Time/Level形式エンベロープ（プロトタイプ）
//
// 5段OPM形式（`eg::Eg`）が「傾き」を指定するのに対し、こちらは「所要時間」を指定する。
// 段(Stage)は最大8つ（CZ-101の8段に準拠）、うち任意区間をループでき、キーオフ後は
// `release_start`から残りの段を順に辿る（多段リリース）。既存の`Eg`・`EgParams`・
// `ym38x6-core`側は一切変更しない、独立した実験用の型（memory
// `project_4point_tl_eg_decision.md`参照）。
//
// `Eg`とAPI形状を揃える（note_on/note_off/retrigger/is_idle/level/tick）ことで、
// 将来enumで共存させる際にそのまま噛み合うようにしてある。
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 段の最大数。CZ-101の8段に準拠。4点T/Lは`stage_count=4`で表現する。
pub const MAX_STAGES: usize = 8;

// ---------------------------------------------------------------------------
// 時間テーブル（`eg::build_rate_seconds_table`と同じOnceLockテーブル化パターン）
//
// `time`(0〜255)は1ノート中不変のパッチ値だが、毎サンプル`powf()`を呼ぶのは避けたいため
// 初回アクセス時に1回だけ256要素テーブルを構築する。
// ---------------------------------------------------------------------------

/// time(1〜255)→秒数の256要素テーブルを構築する（index 0は未使用、`time_to_seconds`で
/// 別途0.0秒として特別扱いする）。
fn build_time_seconds_table(t_min: f32, t_max: f32) -> [f32; 256] {
    let mut table = [0.0f32; 256];
    for (t, slot) in table.iter_mut().enumerate().skip(1) {
        *slot = t_min * (t_max / t_min).powf((t as f32 - 1.0) / 254.0);
    }
    table
}

fn time_seconds_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| build_time_seconds_table(0.001, 30.0))
}

/// time(0〜255)→秒数。0.001秒（1ms）〜30秒の指数マッピング。
/// `time=0`は「瞬時（0秒）」という、レート方式の`rate=0`＝フリーズとは意味が真逆の特殊値
/// （混同すると事故るので明記）。
pub fn time_to_seconds(time: u8) -> f32 {
    if time == 0 {
        return 0.0;
    }
    time_seconds_table()[time as usize]
}

// ---------------------------------------------------------------------------
// パラメーター
// ---------------------------------------------------------------------------

/// 1段分のパラメーター。「現在レベルから`level`へ`time`かけて向かう」を表す。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeStage {
    pub time: u8,
    pub level: u8,
    /// 0=線形／1=レイズドコサイン整形。段ごとに指定できる。
    pub curve: u8,
}

/// TimeEgのパラメーター一式。`stage_count`本だけ`stages`を使う（残りは無視）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeEgParams {
    pub stages: [TimeStage; MAX_STAGES],
    /// 使用する段数(1〜8)。0は1として扱う。
    pub stage_count: u8,
    /// 0=ワンショット（`loop_end`で静止＝サステイン点）／1=`loop_start`〜`loop_end`を周回。
    pub loop_enabled: u8,
    pub loop_start: u8,
    pub loop_end: u8,
    /// キーオフ後に辿り始める段。以降`stage_count-1`まで順に辿り、完了でIdleへ（多段リリース）。
    pub release_start: u8,
}

fn level_of(stage: &TimeStage) -> f32 {
    stage.level as f32 / 255.0
}

fn clamp_stage_count(stage_count: u8) -> usize {
    (stage_count as usize).clamp(1, MAX_STAGES)
}

// ---------------------------------------------------------------------------
// 状態機械
// ---------------------------------------------------------------------------

/// note_on/retriggerされた直後、まだ`tick`が一度も呼ばれていない状態（段0のセグメント境界を
/// `params`から解決できるのが`tick`内だけのため、`eg::Eg`のDelay→Attackフォールスルーと同じ
/// 遅延初期化パターンを踏む）。Freshは0.0から、Retriggerは現在レベルを保持したまま段0へ。
#[derive(Clone, Copy, PartialEq, Debug)]
enum PendingStart {
    None,
    Fresh,
    Retrigger,
}

pub struct TimeEg {
    stage_index: usize,
    level: f32,
    segment_start: f32,
    segment_end: f32,
    /// 現在の段に入ってからの経過秒数。
    elapsed: f32,
    releasing: bool,
    pending_start: PendingStart,
    /// note_off直後、まだ`tick`が呼ばれておらず`release_start`段のセグメント境界を
    /// `params`から解決できていない状態（pending_startと同じ遅延初期化パターン）。
    release_pending: bool,
    idle: bool,
}

impl TimeEg {
    pub fn new() -> Self {
        Self {
            stage_index: 0,
            level: 0.0,
            segment_start: 0.0,
            segment_end: 0.0,
            elapsed: 0.0,
            releasing: false,
            pending_start: PendingStart::None,
            release_pending: false,
            idle: true,
        }
    }

    pub fn note_on(&mut self) {
        self.level = 0.0;
        self.stage_index = 0;
        self.segment_start = 0.0;
        self.segment_end = 0.0;
        self.elapsed = 0.0;
        self.releasing = false;
        self.release_pending = false;
        self.pending_start = PendingStart::Fresh;
        self.idle = false;
    }

    /// 残響レベルを保持したまま段0へ再突入する（`eg::Eg::retrigger`と同じ思想）。
    pub fn retrigger(&mut self) {
        self.releasing = false;
        self.release_pending = false;
        self.pending_start = PendingStart::Retrigger;
        self.elapsed = 0.0;
        self.idle = false;
    }

    pub fn note_off(&mut self) {
        if self.idle {
            return;
        }
        self.release_pending = true;
    }

    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// 現在のエンベロープレベル(0.0〜1.0、Curve整形前の生値)。
    pub fn level(&self) -> f32 {
        self.level
    }

    fn shaped_output(&self, curve: u8) -> f32 {
        if curve == 0 {
            return self.level;
        }
        let span = self.segment_end - self.segment_start;
        if span.abs() < 1e-9 {
            return self.level;
        }
        let progress = ((self.level - self.segment_start) / span).clamp(0.0, 1.0);
        let shaped = 0.5 - 0.5 * (std::f32::consts::PI * progress).cos();
        self.segment_start + shaped * span
    }

    /// 1サンプル分エンベロープを進め、現在のレベル(0.0〜1.0、Curve整形適用後)を返す。
    /// `speed_scale`は時間軸への乗算（大きいほど速い。`eg::Eg::tick`の`rate_scale`と向きを揃えた。
    /// テンポ同期はこの引数に「1周の合計時間÷目標時間」を渡すことで実現できる）。
    pub fn tick(&mut self, sample_rate: f32, params: TimeEgParams, speed_scale: f32) -> f32 {
        let params = &params;
        let stage_count = clamp_stage_count(params.stage_count);

        if self.release_pending {
            self.release_pending = false;
            let release_start = (params.release_start as usize).min(stage_count - 1);
            self.stage_index = release_start;
            self.segment_start = self.level;
            self.segment_end = level_of(&params.stages[release_start]);
            self.elapsed = 0.0;
            self.releasing = true;
            self.idle = false;
        } else if self.pending_start != PendingStart::None {
            let start_level = match self.pending_start {
                PendingStart::Fresh => 0.0,
                PendingStart::Retrigger => self.level,
                PendingStart::None => unreachable!(),
            };
            self.pending_start = PendingStart::None;
            self.stage_index = 0;
            self.level = start_level;
            self.segment_start = start_level;
            self.segment_end = level_of(&params.stages[0]);
            self.elapsed = 0.0;
        }

        if self.idle {
            return self.level;
        }

        let cur = self.stage_index.min(stage_count - 1);
        let stage = &params.stages[cur];
        let curve = stage.curve;
        let seconds = time_to_seconds(stage.time);

        self.elapsed += (1.0 / sample_rate) * speed_scale;

        let (progress, overflow) = if seconds <= f32::EPSILON {
            (1.0, 0.0)
        } else {
            let p = self.elapsed / seconds;
            if p >= 1.0 {
                (1.0, self.elapsed - seconds)
            } else {
                (p, 0.0)
            }
        };

        self.level = self.segment_start + (self.segment_end - self.segment_start) * progress;
        let out = self.shaped_output(curve);

        if progress >= 1.0 {
            self.level = self.segment_end;
            self.advance(params, cur, stage_count, overflow.max(0.0));
        }

        out
    }

    fn advance(&mut self, params: &TimeEgParams, cur: usize, stage_count: usize, overflow: f32) {
        if self.releasing {
            if cur + 1 < stage_count {
                self.enter_stage(params, cur + 1, overflow);
            } else {
                self.idle = true;
                self.stage_index = cur;
            }
            return;
        }

        let loop_start = (params.loop_start as usize).min(stage_count - 1);
        let loop_end = (params.loop_end as usize).min(stage_count - 1);

        if cur == loop_end {
            if params.loop_enabled != 0 {
                self.enter_stage(params, loop_start, overflow);
            } else {
                // サステイン点で静止（`eg::Eg`のD2R=0フリーズに相当）。
                self.settle_at_current_level(cur);
            }
        } else if cur + 1 < stage_count {
            self.enter_stage(params, cur + 1, overflow);
        } else {
            // stage_countの終端に達したがloop_endに届いていない設定不整合。そこで静止する。
            self.settle_at_current_level(cur);
        }
    }

    fn enter_stage(&mut self, params: &TimeEgParams, next: usize, overflow: f32) {
        self.stage_index = next;
        self.segment_start = self.level;
        self.segment_end = level_of(&params.stages[next]);
        self.elapsed = overflow;
    }

    fn settle_at_current_level(&mut self, cur: usize) {
        self.stage_index = cur;
        self.segment_start = self.level;
        self.segment_end = self.level;
        self.elapsed = 0.0;
    }
}

impl Default for TimeEg {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn stages_with(entries: &[(u8, u8, u8)]) -> [TimeStage; MAX_STAGES] {
        let mut stages = [TimeStage::default(); MAX_STAGES];
        for (i, &(time, level, curve)) in entries.iter().enumerate() {
            stages[i] = TimeStage { time, level, curve };
        }
        stages
    }

    /// サンプル単位の一致比較。`elapsed`はサンプルごとの浮動小数点加算（`eg::Eg`のDelayフェーズと
    /// 同じ方式）のため、長い区間ほど累積誤差で数サンプル早まる／遅れる。相対0.2%＋余裕2サンプルを許容する。
    fn assert_close_samples(actual: i64, expected: i64) {
        let tolerance = ((expected as f64) * 0.002).ceil() as i64 + 2;
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual} expected={expected} tolerance={tolerance}"
        );
    }

    #[test]
    fn stage_duration_independent_of_level_delta() {
        let sr = 44100.0;
        let seconds = time_to_seconds(150);
        let expected_samples = (seconds * sr).round() as i64;

        for &lvl in &[255u8, 40u8] {
            let params = TimeEgParams {
                stages: stages_with(&[(150, lvl, 0)]),
                stage_count: 1,
                loop_enabled: 0,
                loop_start: 0,
                loop_end: 0,
                release_start: 0,
            };
            let mut eg = TimeEg::new();
            eg.note_on();
            let target = lvl as f32 / 255.0;
            let mut reached_at: Option<i64> = None;
            for i in 0..(expected_samples + 50) {
                let out = eg.tick(sr, params, 1.0);
                if reached_at.is_none() && (out - target).abs() < 1e-4 {
                    reached_at = Some(i);
                }
            }
            let reached = reached_at.expect("should reach target level");
            assert_close_samples(reached, expected_samples);
        }
    }

    #[test]
    fn loop_cycles_between_loop_start_and_loop_end_without_idle() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(80, 255, 0), (70, 80, 0), (70, 200, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            loop_end: 2,
            release_start: 2,
        };
        // time値ごとの実秒数から、attack＋複数ループ分のサンプル予算を実測ベースで組み立てる
        // （time=200のような大きな生値は数秒スケールになりうるため、固定回数の決め打ちは避ける）。
        let attack_secs = time_to_seconds(80) as f64;
        let cycle_secs = (time_to_seconds(70) as f64) * 2.0;
        // attack直後の最初の下降脚（1.0→loop_start）はloop_endの上限(0.784)を一時的に超えて通過する
        // ため、定常ループに入るまで（attack＋1周期分）はレベル一致判定に使わずスキップする。
        let skip_samples = ((attack_secs + cycle_secs) * sr as f64) as usize + 200;
        let observe_samples = (cycle_secs * 8.0 * sr as f64) as usize + 2000;

        let mut eg = TimeEg::new();
        eg.note_on();
        for _ in 0..skip_samples {
            eg.tick(sr, params, 1.0);
        }
        let mut max_seen = 0.0f32;
        let mut min_seen = 1.0f32;
        for _ in 0..observe_samples {
            let level = eg.tick(sr, params, 1.0);
            max_seen = max_seen.max(level);
            min_seen = min_seen.min(level);
        }
        assert!((max_seen - 200.0 / 255.0).abs() < 0.01, "max_seen={max_seen}");
        assert!((min_seen - 80.0 / 255.0).abs() < 0.01, "min_seen={min_seen}");
        assert!(!eg.is_idle(), "looping should never become idle on its own");
    }

    #[test]
    fn no_loop_settles_at_loop_end_level() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(100, 255, 0), (100, 128, 0), (100, 0, 0)]),
            stage_count: 3,
            loop_enabled: 0,
            loop_start: 1,
            loop_end: 1,
            release_start: 2,
        };
        let mut eg = TimeEg::new();
        eg.note_on();
        let mut level = 0.0;
        for _ in 0..20_000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 128.0 / 255.0).abs() < 1e-3, "expected to settle at loop_end level, got {level}");
        assert!(!eg.is_idle());
    }

    #[test]
    fn note_off_walks_release_stages_to_idle() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(100, 255, 0), (100, 128, 0), (80, 40, 0), (80, 0, 0)]),
            stage_count: 4,
            loop_enabled: 0,
            loop_start: 1,
            loop_end: 1,
            release_start: 2,
        };
        let mut eg = TimeEg::new();
        eg.note_on();
        for _ in 0..10_000 {
            eg.tick(sr, params, 1.0);
        }
        assert!(!eg.is_idle());

        eg.note_off();
        let mut became_idle = false;
        let mut final_level = -1.0;
        for _ in 0..30_000 {
            let level = eg.tick(sr, params, 1.0);
            if eg.is_idle() {
                became_idle = true;
                final_level = level;
                break;
            }
        }
        assert!(became_idle, "expected note_off to walk release stages to Idle");
        assert!((final_level - 0.0).abs() < 1e-6, "expected final release level 0.0, got {final_level}");
    }

    #[test]
    fn speed_scale_halves_stage_duration() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(180, 255, 0)]),
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            loop_end: 0,
            release_start: 0,
        };
        let seconds = time_to_seconds(180);
        let expected_1x = (seconds * sr).round() as i64;
        let expected_2x = (seconds * sr / 2.0).round() as i64;

        let reach_sample = |scale: f32| -> i64 {
            let mut eg = TimeEg::new();
            eg.note_on();
            for i in 0..(expected_1x + 100) {
                let out = eg.tick(sr, params, scale);
                if (out - 1.0).abs() < 1e-4 {
                    return i;
                }
            }
            panic!("did not reach target level");
        };

        let at_1x = reach_sample(1.0);
        let at_2x = reach_sample(2.0);
        assert_close_samples(at_1x, expected_1x);
        assert_close_samples(at_2x, expected_2x);
    }

    #[test]
    fn curve_does_not_change_stage_transition_timing() {
        let sr = 44100.0;
        let build = |curve: u8| TimeEgParams {
            stages: stages_with(&[(150, 200, curve), (100, 128, 0)]),
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 1,
            loop_end: 1,
            release_start: 1,
        };

        let ticks_to_sustain = |params: TimeEgParams| -> i64 {
            let mut eg = TimeEg::new();
            eg.note_on();
            for i in 0..200_000 {
                eg.tick(sr, params, 1.0);
                if (eg.level() - 128.0 / 255.0).abs() < 1e-6 {
                    return i;
                }
            }
            panic!("did not reach sustain in time");
        };

        let linear = ticks_to_sustain(build(0));
        let curved = ticks_to_sustain(build(1));
        assert_eq!(linear, curved, "curve should not affect stage transition timing");
    }

    #[test]
    fn single_stage_is_simple_one_shot() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(120, 255, 0)]),
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            loop_end: 0,
            release_start: 0,
        };
        let mut eg = TimeEg::new();
        eg.note_on();
        let mut level = 0.0;
        for _ in 0..20_000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 1.0).abs() < 1e-3);
        assert!(!eg.is_idle(), "single stage without loop should hold (sustain), not free-run to idle");

        eg.note_off();
        let mut became_idle = false;
        for _ in 0..20_000 {
            eg.tick(sr, params, 1.0);
            if eg.is_idle() {
                became_idle = true;
                break;
            }
        }
        assert!(became_idle, "note_off should eventually release to idle even in the single-stage case");
    }
}
