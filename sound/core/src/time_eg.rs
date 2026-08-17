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
    TABLE.get_or_init(|| build_time_seconds_table(0.001, 300.0))
}

/// time(0〜255)→秒数。0.001秒（1ms）〜300秒の指数マッピング
/// （T_MAX=300はレート方式EGの理論最遅値284.9秒(D1R/D2R rate=1・RR rate=0のフルスパン)を
/// 余裕を持ってカバーする値。旧T_MAX=30秒はOPZ等実機の遅いレートを変換する際に約9.5倍も
/// クロップしていた。memory `project_timeeg_time_field_16bit_consideration.md`参照）。
/// `time=0`は「瞬時（0秒）」という、レート方式の`rate=0`＝フリーズとは意味が真逆の特殊値
/// （混同すると事故るので明記）。
pub fn time_to_seconds(time: u8) -> f32 {
    if time == 0 {
        return 0.0;
    }
    time_seconds_table()[time as usize]
}

/// `time_to_seconds`の逆写像。秒数からtime値(0〜255)を逆算する
/// （`op505-core`のAdapter、レート方式EGパラメーターからの変換で使用）。
/// 0秒以下は`time=0`（瞬時）、300秒以上は`time=255`にクランプする。
pub fn seconds_to_time(seconds: f32) -> u8 {
    const T_MIN: f32 = 0.001;
    const T_MAX: f32 = 300.0;
    if seconds <= 0.0 {
        return 0;
    }
    if seconds <= T_MIN {
        return 1;
    }
    if seconds >= T_MAX {
        return 255;
    }
    let t = 1.0 + 254.0 * (seconds / T_MIN).ln() / (T_MAX / T_MIN).ln();
    t.round().clamp(1.0, 255.0) as u8
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
///
/// 段リストは`release_point`ただ1つで「保持区間」と「リリース区間」へ過不足なく分割される。
/// 旧`loop_end`+`release_start`の2フィールド方式は、1つの境界を2つの独立した数で表していたため
/// 重複（同じ段が両区間に属しグラフに二重描画される）と隙間（どちらにも属さない到達不能段）を
/// 表現できてしまった。1つに統合したことでその矛盾は表現不能になっている。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeEgParams {
    pub stages: [TimeStage; MAX_STAGES],
    /// 使用する段数(1〜8)。0は1として扱う。
    pub stage_count: u8,
    /// 0=ワンショット（`release_point`で静止＝サステイン点）／1=`loop_start`〜`release_point`を周回。
    pub loop_enabled: u8,
    pub loop_start: u8,
    /// 保持区間の最終段。キーオン中は`0..=release_point`を辿り、ここで静止（ループ有効なら
    /// `loop_start`へ戻る）。リリース区間は`release_point+1..stage_count`で、中間段を飛ばさず
    /// 順に辿る多段リリースになる。
    ///
    /// `release_point == stage_count-1`のときリリース区間は空で、note-offは何もしない
    /// （現在レベルのまま静止し続ける）。Gain FGの透過既定＝ゲートを一切閉じない用途に使う。
    ///
    /// serdeのaliasは旧`loop_end`からの移行用。値の意味が同じなので変換は不要
    /// （旧`release_start`フィールドは`deny_unknown_fields`未使用のため自動的に無視される）。
    #[serde(alias = "loop_end")]
    pub release_point: u8,
    /// テンポ同期の有効/無効。0=無効（`time`の絶対秒数をそのまま使う）／1=有効
    /// （`tempo_speed_scale()`で同期対象区間を`sync_note`の音価ちょうどへ伸縮する）。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で0（無効）にする。
    #[serde(default)]
    pub sync_enabled: u8,
    /// 同期先の音価（`sync_note_beats()`のindex、0〜19）。所要時間の昇順に並んだテーブルで、
    /// index 10 = 1/4（1拍）が既定。`sync_enabled=0`のときは無視される。
    #[serde(default = "default_sync_note")]
    pub sync_note: u8,
    /// retrigger()時（ボイス使い回しの再キーオン）のFGレベルの扱い。
    /// 0=Continue（既定・現在レベルを保ったまま段0へ向かう、`TimeEg::retrigger()`相当）／
    /// 1=Reset（`TimeEg::note_on()`相当、常に0から）。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で0（Continue）にする
    /// （実バンク調査の結果、Pitch FGは324プリセット中0件が非平坦、影響があるのはGain FG 1件
    /// （継承で改善）とCutoff FG 1件（継承で再アタックが弱まる、パッチ側でResetを選べる）のみ
    /// だった。詳細はmemory `project_timeeg_tempo_sync_and_retrigger_mode.md`参照）。
    #[serde(default)]
    pub retrigger_mode: u8,
}

/// `sync_note`の`#[serde(default)]`用。フィールド欠落時（旧バンク）は`sync_enabled=0`なので
/// この値自体は無視されるが、UIで初めてSYNCをONにしたときに1/4から始まるよう10にしておく。
fn default_sync_note() -> u8 {
    10
}

/// retrigger_modeの意味を表す定数（生のu8のまま`TimeEgParams`に持たせているため、
/// 呼び出し側の可読性のためにここへ集約する）。
pub const RETRIGGER_MODE_CONTINUE: u8 = 0;
pub const RETRIGGER_MODE_RESET: u8 = 1;

/// 既定は「保持1段＋リリース1段」の2段（全段time=0/level=0＝無音）。
///
/// `#[derive(Default)]`の全ゼロ（`stage_count=0`→1段扱い）にしないのは、1段だとリリース区間
/// （`release_point+1..stage_count`）が空になり**note-offが何も起きなくなる**ため。
/// オペレーターEGでそれをやるとEGが永久にIdleにならず、ボイスが解放されないまま溜まる
/// （op505のボイス解放条件は「全4オペレーターが`is_idle()`」）。
/// リリース区間を持たせてよいのはGain FG（出力への乗算でボイス解放に関与しない）だけで、
/// そちらは`op505_core::default_gain_fg`が明示的に1段を組み立てる。
impl Default for TimeEgParams {
    fn default() -> Self {
        Self {
            stages: [TimeStage::default(); MAX_STAGES],
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
            sync_enabled: 0,
            sync_note: default_sync_note(),
            retrigger_mode: RETRIGGER_MODE_CONTINUE,
        }
    }
}

fn level_of(stage: &TimeStage) -> f32 {
    stage.level as f32 / 255.0
}

fn clamp_stage_count(stage_count: u8) -> usize {
    (stage_count as usize).clamp(1, MAX_STAGES)
}

// ---------------------------------------------------------------------------
// テンポ同期
//
// LFOのテンポ同期（1周＝指定音価）と同じ考え方をTimeEgへ適用する（方式A）。
// ループ有効なら`loop_start..=release_point`の1周、無効なら`0..=release_point`の
// 保持区間全体を対象に、その素の所要時間を指定音価ちょうどへ伸縮する`speed_scale`倍率を返す。
// アタックやリリースも同じ比率で一緒に伸縮する（対象区間より外は同期の管轄外）ため、
// 「ビブラートだけ同期してアタックは固定秒数」という表現はできない
// （区間ごとに速度を分けるにはtick()自体の再設計が要る、次善は将来課題）。
// ---------------------------------------------------------------------------

/// 同期音価テーブルの段数。付点・3連を含み、所要時間の昇順（index 0が最短）。
pub const SYNC_NOTE_COUNT: usize = 20;

/// 同期音価テーブル。値は拍数（4分音符=1.0）。所要時間の昇順に並べてあるため、
/// ノブ/セレクタでindexを増やすと単調に長くなる。index 10 = 1/4（1拍、既定）。
///
/// | index | 音価 | 拍数 |
/// |---|---|---|
/// | 0 | 1/32T | 0.0833 |
/// | 1 | 1/32 | 0.125 |
/// | 2 | 1/16T | 0.1667 |
/// | 3 | 1/32D | 0.1875 |
/// | 4 | 1/16 | 0.25 |
/// | 5 | 1/8T | 0.3333 |
/// | 6 | 1/16D | 0.375 |
/// | 7 | 1/8 | 0.5 |
/// | 8 | 1/4T | 0.6667 |
/// | 9 | 1/8D | 0.75 |
/// | 10 | 1/4（既定） | 1.0 |
/// | 11 | 1/2T | 1.3333 |
/// | 12 | 1/4D | 1.5 |
/// | 13 | 1/2 | 2.0 |
/// | 14 | 1/1T | 2.6667 |
/// | 15 | 1/2D | 3.0 |
/// | 16 | 1/1（1小節） | 4.0 |
/// | 17 | 1/1D | 6.0 |
/// | 18 | 2/1（2小節） | 8.0 |
/// | 19 | 4/1（4小節） | 16.0 |
const SYNC_NOTE_BEATS: [f32; SYNC_NOTE_COUNT] = [
    1.0 / 32.0 * (2.0 / 3.0),
    1.0 / 32.0,
    1.0 / 16.0 * (2.0 / 3.0),
    1.0 / 32.0 * 1.5,
    1.0 / 16.0,
    1.0 / 8.0 * (2.0 / 3.0),
    1.0 / 16.0 * 1.5,
    1.0 / 8.0,
    1.0 / 4.0 * (2.0 / 3.0),
    1.0 / 8.0 * 1.5,
    1.0 / 4.0,
    1.0 / 2.0 * (2.0 / 3.0),
    1.0 / 4.0 * 1.5,
    1.0 / 2.0,
    1.0 * (2.0 / 3.0),
    1.0 / 2.0 * 1.5,
    1.0,
    1.0 * 1.5,
    2.0,
    4.0,
];

/// 同期音価テーブルのindex(0〜19)→拍数（4分音符=1.0）。範囲外はクランプする。
pub fn sync_note_beats(index: u8) -> f32 {
    SYNC_NOTE_BEATS[(index as usize).min(SYNC_NOTE_COUNT - 1)]
}

/// テンポ同期の対象区間（ループ有効なら`loop_start..=release_point`のループ1周、
/// 無効なら`0..=release_point`の保持区間全体）の、同期前（素の`time`値どおり）の合計秒数。
pub fn sync_region_seconds(params: &TimeEgParams) -> f32 {
    let stage_count = clamp_stage_count(params.stage_count);
    let release_point = (params.release_point as usize).min(stage_count - 1);
    let start = if params.loop_enabled != 0 {
        (params.loop_start as usize).min(release_point)
    } else {
        0
    };
    (start..=release_point)
        .map(|i| time_to_seconds(params.stages[i].time))
        .sum()
}

/// テンポ同期が有効なときに`tick()`の`speed_scale`へ乗算する倍率。
/// `対象区間の素の秒数 ÷ 指定音価の秒数`で、対象区間ちょうどが音価どおりの長さになるよう
/// 時間軸を伸縮する。同期無効・対象区間が実質0秒・bpmが0以下のいずれかなら1.0（無補正）を返す
/// （既存のCC76由来速度補正等、他の`speed_scale`要因と乗算で共存できる設計）。
pub fn tempo_speed_scale(params: &TimeEgParams, bpm: f32) -> f32 {
    if params.sync_enabled == 0 || bpm <= 0.0 {
        return 1.0;
    }
    let region = sync_region_seconds(params);
    if region <= f32::EPSILON {
        return 1.0;
    }
    let target_seconds = sync_note_beats(params.sync_note) * 60.0 / bpm;
    region / target_seconds
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
    /// 現在の段に入ってからの経過秒数。T_MAX=300秒の長い段でも毎サンプル加算の丸め誤差が
    /// 蓄積しにくいよう`f64`で保持する（他フィールドは`f32`のまま、ここだけ精度を上げる）。
    elapsed: f64,
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
            let release_point = (params.release_point as usize).min(stage_count - 1);
            // リリース区間は`release_point+1..stage_count`。空（release_pointが最終段）のときは
            // note-offを何もせず現在レベルのまま静止し続ける（Gain FGの透過既定＝ゲートを閉じない）。
            if release_point + 1 < stage_count {
                self.stage_index = release_point + 1;
                self.segment_start = self.level;
                self.segment_end = level_of(&params.stages[release_point + 1]);
                self.elapsed = 0.0;
                self.releasing = true;
                self.idle = false;
            }
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
        let seconds = time_to_seconds(stage.time) as f64;

        self.elapsed += (1.0 / sample_rate as f64) * speed_scale as f64;

        let (progress, overflow): (f64, f64) = if seconds <= f64::EPSILON {
            (1.0, 0.0)
        } else {
            let p = self.elapsed / seconds;
            if p >= 1.0 {
                (1.0, self.elapsed - seconds)
            } else {
                (p, 0.0)
            }
        };

        self.level = self.segment_start + (self.segment_end - self.segment_start) * progress as f32;
        let out = self.shaped_output(curve);

        if progress >= 1.0 {
            self.level = self.segment_end;
            self.advance(params, cur, stage_count, overflow.max(0.0));
        }

        out
    }

    fn advance(&mut self, params: &TimeEgParams, cur: usize, stage_count: usize, overflow: f64) {
        if self.releasing {
            if cur + 1 < stage_count {
                self.enter_stage(params, cur + 1, overflow);
            } else {
                self.idle = true;
                self.stage_index = cur;
            }
            return;
        }

        let release_point = (params.release_point as usize).min(stage_count - 1);
        let loop_start = (params.loop_start as usize).min(release_point);

        if cur == release_point {
            // 保持区間の終端。ループ有効なら戻り、無効ならサステイン点として静止する
            // （`eg::Eg`のD2R=0フリーズに相当）。ここから先はnote-offでしか進まない。
            if params.loop_enabled != 0 {
                // 1段ループ（`loop_start == release_point`）だけは、その段の**入口レベル**
                // （直前段の終端レベル、段0なら0.0）へ跳ね戻してからやり直す。
                // レベル連続のままだと「自分の終端レベルから自分の終端レベルへ」向かうことになり
                // 完全に平坦で音として何も起きない。跳ね戻すことで1段だけでノコギリ波を表現できる。
                // 多段ループ（`loop_start < release_point`）は従来どおりレベル連続で周回する
                // （区間の両端を行き来する三角波的な動きになる）。
                if loop_start == release_point {
                    self.level = if loop_start == 0 { 0.0 } else { level_of(&params.stages[loop_start - 1]) };
                }
                self.enter_stage(params, loop_start, overflow);
            } else {
                self.settle_at_current_level(cur);
            }
        } else if cur + 1 < stage_count {
            self.enter_stage(params, cur + 1, overflow);
        } else {
            // stage_countの終端に達したがrelease_pointに届いていない設定不整合。そこで静止する。
            self.settle_at_current_level(cur);
        }
    }

    fn enter_stage(&mut self, params: &TimeEgParams, next: usize, overflow: f64) {
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
    fn seconds_to_time_round_trips_within_one_step() {
        // time=0(瞬時)は特殊値、他は256要素テーブルの丸め誤差1ステップ以内で往復することを確認
        assert_eq!(seconds_to_time(time_to_seconds(0)), 0);
        for t in 1u8..=255 {
            let seconds = time_to_seconds(t);
            let back = seconds_to_time(seconds);
            let diff = (back as i32 - t as i32).abs();
            assert!(diff <= 1, "time={t} seconds={seconds} back={back} diff={diff}");
        }
    }

    #[test]
    fn seconds_to_time_clamps_out_of_range() {
        assert_eq!(seconds_to_time(0.0), 0);
        assert_eq!(seconds_to_time(-1.0), 0);
        assert_eq!(seconds_to_time(0.0005), 1);
        assert_eq!(seconds_to_time(300.0), 255);
        assert_eq!(seconds_to_time(1000.0), 255);
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
                release_point: 0,
             ..Default::default()};
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
    fn loop_cycles_between_loop_start_and_release_point_without_idle() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(80, 255, 0), (70, 80, 0), (70, 200, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
         ..Default::default()};
        // time値ごとの実秒数から、attack＋複数ループ分のサンプル予算を実測ベースで組み立てる
        // （time=200のような大きな生値は数秒スケールになりうるため、固定回数の決め打ちは避ける）。
        let attack_secs = time_to_seconds(80) as f64;
        let cycle_secs = (time_to_seconds(70) as f64) * 2.0;
        // attack直後の最初の下降脚（1.0→loop_start）はrelease_pointの上限(0.784)を一時的に超えて通過する
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
    fn no_loop_settles_at_release_point_level() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(100, 255, 0), (100, 128, 0), (100, 0, 0)]),
            stage_count: 3,
            loop_enabled: 0,
            loop_start: 1,
            release_point: 1,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();
        let mut level = 0.0;
        for _ in 0..20_000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 128.0 / 255.0).abs() < 1e-3, "expected to settle at release_point level, got {level}");
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
            release_point: 1,
         ..Default::default()};
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
            release_point: 0,
         ..Default::default()};
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
            release_point: 1,
         ..Default::default()};

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

    /// リリース区間が空（`release_point == stage_count-1`）のときnote-offが何もしないこと。
    /// Gain FGの透過既定（`op505_core::default_gain_fg`＝ゲートを一切閉じない）が依存する挙動で、
    /// この形はGain FG専用。OP EG/Pitch FG/Cutoff FGはUI側でSTAGE>=2かつ最終段level=0を強制するため
    /// 必ずリリース区間を持つ。
    #[test]
    fn empty_release_region_makes_note_off_a_noop() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(120, 255, 0)]),
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();
        let mut level = 0.0;
        for _ in 0..20_000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 1.0).abs() < 1e-3);
        assert!(!eg.is_idle(), "single stage without loop should hold (sustain), not free-run to idle");

        eg.note_off();
        for _ in 0..20_000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 1.0).abs() < 1e-3, "gate must stay open after note_off, got {level}");
        assert!(!eg.is_idle(), "empty release region must never reach Idle (transparent gate)");
    }

    /// リリースが`release_point+1`から最終段まで中間段を飛ばさず順に辿ること。
    /// 段2をサステインレベルより「上」に置くことで、段3へ直行した場合と区別できるようにしている
    /// （直行なら100→0の単調降下でサステインレベルを超えないため）。
    #[test]
    fn release_traverses_intermediate_stages_without_skipping() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(100, 255, 0), (100, 100, 0), (100, 220, 0), (100, 0, 0)]),
            stage_count: 4,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 1,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();
        let mut level = 0.0;
        for _ in 0..40_000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 100.0 / 255.0).abs() < 1e-2, "expected sustain at stage1 level, got {level}");

        eg.note_off();
        let mut peak_during_release = 0.0f32;
        let mut became_idle = false;
        for _ in 0..80_000 {
            let out = eg.tick(sr, params, 1.0);
            peak_during_release = peak_during_release.max(out);
            if eg.is_idle() {
                became_idle = true;
                break;
            }
        }
        assert!(became_idle, "expected release to reach Idle");
        assert!(
            peak_during_release > 200.0 / 255.0,
            "release must climb through stage2 (220), not jump straight to stage3; peak={peak_during_release}"
        );
    }

    /// 保持区間とリリース区間が段リストを過不足なく分割すること（重複も隙間も無い）。
    /// 旧`loop_end`+`release_start`の2フィールド方式ではこの不変条件を破れたが、
    /// `release_point`1本になったため型のレベルで破れない。ここでは全ての`release_point`について
    /// 「保持側の最後の段＝release_point」「リリース側の最初の段＝release_point+1」を確認する。
    #[test]
    fn hold_and_release_regions_partition_the_stage_list() {
        let sr = 44100.0;
        let levels = [255u8, 200, 150, 100, 0];
        for release_point in 0u8..4 {
            let params = TimeEgParams {
                stages: stages_with(&[
                    (60, levels[0], 0),
                    (60, levels[1], 0),
                    (60, levels[2], 0),
                    (60, levels[3], 0),
                    (60, levels[4], 0),
                ]),
                stage_count: 5,
                loop_enabled: 0,
                loop_start: 0,
                release_point,
             ..Default::default()};
            let mut eg = TimeEg::new();
            eg.note_on();
            let mut level = 0.0;
            for _ in 0..60_000 {
                level = eg.tick(sr, params, 1.0);
            }
            let expected_sustain = levels[release_point as usize] as f32 / 255.0;
            assert!(
                (level - expected_sustain).abs() < 1e-2,
                "release_point={release_point}: expected sustain at stage{release_point} level, got {level}"
            );

            eg.note_off();
            let mut became_idle = false;
            let mut final_level = -1.0;
            for _ in 0..200_000 {
                let out = eg.tick(sr, params, 1.0);
                if eg.is_idle() {
                    became_idle = true;
                    final_level = out;
                    break;
                }
            }
            assert!(became_idle, "release_point={release_point}: expected release to reach Idle");
            assert!(
                final_level.abs() < 1e-6,
                "release_point={release_point}: expected final level 0.0, got {final_level}"
            );
        }
    }

    /// 1段ループ（`loop_start == release_point`）はその段の入口レベルへ跳ね戻してやり直すため、
    /// 1段だけでノコギリ波になる。段0で255まで上がり、段1で120へ下る形をループさせると、
    /// 「255へ跳ね上がって120へ下る」を繰り返す。
    #[test]
    fn single_stage_loop_sawtooths_from_stage_entry_level() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(80, 255, 0), (80, 120, 0), (80, 0, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 1,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();
        // アタック(段0)を抜けて定常のノコギリに入るまで進める。
        let settle = (time_to_seconds(80) as f64 * 2.5 * sr as f64) as usize;
        for _ in 0..settle {
            eg.tick(sr, params, 1.0);
        }
        let mut min_seen = 1.0f32;
        let mut max_seen = 0.0f32;
        for _ in 0..((time_to_seconds(80) as f64 * 6.0 * sr as f64) as usize) {
            let out = eg.tick(sr, params, 1.0);
            min_seen = min_seen.min(out);
            max_seen = max_seen.max(out);
        }
        assert!((max_seen - 1.0).abs() < 0.02, "入口レベル(255)へ跳ね戻るはず: max={max_seen}");
        assert!((min_seen - 120.0 / 255.0).abs() < 0.02, "段1のtarget(120)まで下るはず: min={min_seen}");
        assert!(!eg.is_idle(), "ループ中は自然にIdleにならないはず");
    }

    /// `loop_start == 0`の1段ループは、段0の入口＝レベル0へ跳ね戻す（note_onの開始レベルと同じ）。
    #[test]
    fn single_stage_loop_at_stage_zero_falls_back_to_silence() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(80, 255, 0), (80, 0, 0)]),
            stage_count: 2,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 0,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();
        let cycle = (time_to_seconds(80) as f64 * sr as f64) as usize;
        for _ in 0..(cycle * 2) {
            eg.tick(sr, params, 1.0);
        }
        let mut min_seen = 1.0f32;
        let mut max_seen = 0.0f32;
        for _ in 0..(cycle * 4) {
            let out = eg.tick(sr, params, 1.0);
            min_seen = min_seen.min(out);
            max_seen = max_seen.max(out);
        }
        assert!((max_seen - 1.0).abs() < 0.02, "段0のtarget(255)まで上るはず: max={max_seen}");
        assert!(min_seen < 0.05, "入口レベル0へ跳ね戻るはず: min={min_seen}");
    }

    /// 旧`.op505`バンク（`loop_end`+`release_start`の2フィールド）がそのまま読めること。
    /// `release_point`は旧`loop_end`と値の意味が同じなので変換不要、余分な`release_start`は
    /// `deny_unknown_fields`未使用のため自動的に無視される。
    #[test]
    fn deserializes_legacy_loop_end_and_ignores_release_start() {
        let legacy = serde_json::json!({
            "stages": vec![serde_json::json!({ "time": 10, "level": 200, "curve": 0 }); MAX_STAGES],
            "stage_count": 4,
            "loop_enabled": 0,
            "loop_start": 2,
            "loop_end": 2,
            "release_start": 3,
        });
        let params: TimeEgParams = serde_json::from_value(legacy).expect("legacy JSON should deserialize");
        assert_eq!(params.stage_count, 4);
        assert_eq!(params.loop_start, 2);
        assert_eq!(params.release_point, 2, "release_point should adopt the legacy loop_end value");
    }
}
