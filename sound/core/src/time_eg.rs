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

/// バイポーラ解釈するEGの「無変調」を表す生レベル値。DT1・op_fine_tune等このプロジェクトの
/// 他のバイポーラパラメーターと同じ中心128（`(v-128)/128`慣例）に揃えてある。
pub const BIPOLAR_NEUTRAL_RAW: u8 = 128;

/// `BIPOLAR_NEUTRAL_RAW`を`tick()`の出力空間（`level/255`の0.0〜1.0）で表したもの。
pub const BIPOLAR_NEUTRAL_LEVEL: f32 = BIPOLAR_NEUTRAL_RAW as f32 / 255.0;

/// `tick()`の出力（0.0〜1.0）を、中心128を0とするバイポーラ値（-1.0〜+127/128）へ写す。
///
/// Pitch FG／Cutoff FGはこの値にDepth（符号を持たない強度0〜255）を掛けて変調量にする。
/// 「符号はレベル側の波形が持ち、Depthは振れ幅の倍率」という役割分担のため、1音の中で
/// 上下対称に振れるサイクル（三角波・パルス等）をFGの形だけで表現できる
/// （旧方式は`level(0〜1) × (depth-128)/128`で、符号が常にDepth1個に固定されていたため
/// 谷が必ずベース値へ張り付き、片側にしか振れなかった）。
///
/// 正側の最大が`127/128`で頭打ちになるのは`(v-128)/128`慣例の帰結で、DT1等と同じ
/// （`lfo.rs`の`lfo_offset_from_param`にも同じ非対称性がある）。
pub fn bipolar_level(tick_output: f32) -> f32 {
    (tick_output * 255.0 - BIPOLAR_NEUTRAL_RAW as f32) / BIPOLAR_NEUTRAL_RAW as f32
}

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

/// `stages`（固定長配列）の**要素数を緩めた**デシリアライザ。
///
/// serdeの`[T; N]`既定実装はデシリアライズ時に要素数を厳密に照合するため、`MAX_STAGES`を
/// 変えた瞬間に**既存の`.op505`プリセット・op505-vstがDAWプロジェクトへ書いたpersist状態・
/// gesture-appのIPC JSONが全滅する**（しかも読み込み失敗は両方とも無言で握り潰される）。
/// ここで「足りなければ`TimeStage::default()`でパディング、多ければ切り詰め」に緩めることで、
/// `MAX_STAGES`の増減がデータ互換性を壊さなくなる（増やす方向も減らす方向も安全＝
/// 変更のロールバックも可能になる）。
///
/// **serialize側は敢えて上書きしない**（既定の`[T; MAX_STAGES]`実装＝`serialize_tuple`が
/// そのままJSON配列を出すので出力形式は完全に不変）。片側だけ差し替えることで
/// 「読みは緩い／書きは正準」という非対称を明示する。
///
/// 構造体全体のカスタム`Deserialize`にしないのは、`TimeEgParams`が既に`#[serde(alias)]`・
/// `#[serde(default)]`・`#[serde(default = "...")]`を多用しており、ヘルパー構造体へ
/// 全部書き写すとフィールド追加時の同期漏れが必ず起きるため。`deserialize_with`は
/// 兄弟フィールドの属性に一切干渉しない。
mod stages_serde {
    use super::{TimeStage, MAX_STAGES};
    use serde::de::{SeqAccess, Visitor};
    use serde::Deserializer;
    use std::fmt;

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<[TimeStage; MAX_STAGES], D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(StagesVisitor)
    }

    struct StagesVisitor;

    impl<'de> Visitor<'de> for StagesVisitor {
        type Value = [TimeStage; MAX_STAGES];

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "a sequence of TimeStage (padded/truncated to {MAX_STAGES})")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // パディング値は`TimeStage::default()`（level=0）。Pitch/Cutoff FGはレベルを
            // バイポーラ解釈する（128が無変調の中心）ため、level=0は「全開マイナス」を意味する。
            // それでも実害が無いのは、`stage_count`を超える段はエンジンもエディタも参照せず、
            // **段を増やす唯一の経路（`ui_core`の`TimeEgFieldHandle::set(StageCount)`と
            // `insert_stage_after`）が直前段を複製する**ため。この複製ロジックが唯一の安全弁で、
            // 「配列の値を直接有効化する」コードを将来足すとこの罠を踏む。
            let mut out = [TimeStage::default(); MAX_STAGES];
            let mut i = 0usize;
            // 余剰要素も必ず最後まで読み切る（途中で止めるとserde_json側が
            // trailing elementsで失敗する）。
            while let Some(stage) = seq.next_element::<TimeStage>()? {
                if i < MAX_STAGES {
                    out[i] = stage;
                }
                i += 1;
            }
            Ok(out)
        }
    }
}

/// TimeEgのパラメーター一式。`stage_count`本だけ`stages`を使う（残りは無視）。
///
/// 段リストは`release_point`ただ1つで「保持区間」と「リリース区間」へ過不足なく分割される。
/// 旧`loop_end`+`release_start`の2フィールド方式は、1つの境界を2つの独立した数で表していたため
/// 重複（同じ段が両区間に属しグラフに二重描画される）と隙間（どちらにも属さない到達不能段）を
/// 表現できてしまった。1つに統合したことでその矛盾は表現不能になっている。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeEgParams {
    /// 段データ。要素数はデシリアライズ時のみ緩めてある（`MAX_STAGES`変更時の
    /// データ互換保証はすべて`stages_serde`が担う。そちらのdocコメント参照）。
    #[serde(deserialize_with = "stages_serde::deserialize")]
    pub stages: [TimeStage; MAX_STAGES],
    /// 使用する段数(1〜`MAX_STAGES`)。0は1として扱う。
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
    /// （`tempo_speed_scale()`で同期対象区間を`sync_rate`の長さちょうどへ伸縮する）。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で0（無効）にする。
    #[serde(default)]
    pub sync_enabled: u8,
    /// 同期先の長さを表す連続レート（0〜255、`sync_rate_beats()`で拍数へ写す）。
    /// 1/32T〜4/1の192倍レンジを1バイトで無段階に刻む。20個の音価は
    /// `sync_note_anchor()`のアンカー値へ**厳密に**乗るため、UIのドロップダウンで音価を
    /// 選べば同期は正確なまま、ノブで回せばその間を連続的に動かせる。既定134＝1/4（1拍）。
    /// `sync_enabled=0`のときは無視される。
    ///
    /// 旧フィールド`sync_note`（0〜19のindex）からの意味変更。`deny_unknown_fields`未使用のため
    /// 旧キーは自動的に無視され、旧バンクは既定の1/4へフォールバックする（移行はしない方針。
    /// テンポ同期は1日前に入ったばかりの機能で、同時に入れた4倍バグ修正でどのみち選び直しに
    /// なるため。詳細はmemory `project_timeeg_sync_rate_knob.md`参照）。
    #[serde(default = "default_sync_rate")]
    pub sync_rate: u8,
    /// retrigger()時（ボイス使い回しの再キーオン）のFGレベルの扱い。
    /// 0=Continue（既定・現在レベルを保ったまま段0へ向かう、`TimeEg::retrigger()`相当）／
    /// 1=Reset（`TimeEg::note_on()`相当、常に0から）。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で0（Continue）にする
    /// （実バンク調査の結果、Pitch FGは324プリセット中0件が非平坦、影響があるのはGain FG 1件
    /// （継承で改善）とCutoff FG 1件（継承で再アタックが弱まる、パッチ側でResetを選べる）のみ
    /// だった。詳細はmemory `project_timeeg_tempo_sync_and_retrigger_mode.md`参照）。
    #[serde(default)]
    pub retrigger_mode: u8,
    /// ループ1周ごとに中心（振れ幅の中点）へ加算するレベル量。128=無効（中心固定）。
    /// バイポーラ（128未満は下降方向、128超は上昇方向）。ループが同じレベルへ戻り続ける
    /// 制約を崩し、「減衰する音の上で揺れ続ける」形を表現する（旧`.op505`バンクには存在しない
    /// フィールドのため`#[serde(default)]`で128=無効にする。詳細はmemory
    /// `project_chip_lfo_retirement_investigation.md`参照）。
    #[serde(default = "default_drift")]
    pub level_drift: u8,
    /// ループ1周ごとに振れ幅へ掛ける率。128=等倍（無効）。0側は縮小、255側は拡大。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で128（無効）にする。
    #[serde(default = "default_drift")]
    pub depth_drift: u8,
    /// ループ区間の各段を、その区間のレベル最小〜最大の間で乱数抽選した値へ置き換える「質感」。
    /// 0=OFF／1=S&H（即ジャンプしてホールド）／2=Random（現在値から補間して滑らかに動く）／
    /// 3=Chaos（ロジスティック写像`x=3.9x(1-x)`による決定論的カオス、段内はS&Hと同じくホールド）。
    /// `loop_enabled=0`（ワンショット）のときは効かない（周回自体がないため）。
    /// 段のtime（アンカー）が拍を、段のlevel範囲が振れ幅を決め、textureはその範囲内の
    /// 「どの値へ向かうか」だけを乱数化する（旧質感LFOのS&H/Random/Chaos波形の後継、
    /// memory `project_texture_lfo_retirement.md`参照）。
    /// 旧`.op505`バンクには存在しないフィールドのため`#[serde(default)]`で0（OFF）にする。
    #[serde(default)]
    pub texture: u8,
}

/// `texture`の意味を表す定数（生のu8のまま`TimeEgParams`に持たせているため、
/// 呼び出し側の可読性のためにここへ集約する）。
pub const TEXTURE_OFF: u8 = 0;
pub const TEXTURE_SAMPLE_HOLD: u8 = 1;
pub const TEXTURE_RANDOM: u8 = 2;
pub const TEXTURE_CHAOS: u8 = 3;

/// `sync_rate`の`#[serde(default)]`用。フィールド欠落時（旧バンク）は`sync_enabled=0`なので
/// この値自体は無視されるが、UIで初めてSYNCをONにしたときに1/4から始まるよう
/// index 10（1/4）のアンカー値にしておく。
fn default_sync_rate() -> u8 {
    SYNC_NOTE_ANCHORS[10]
}

/// `level_drift`/`depth_drift`の`#[serde(default)]`用。128＝無効（中心128慣例に揃える）。
fn default_drift() -> u8 {
    BIPOLAR_NEUTRAL_RAW
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
            sync_rate: default_sync_rate(),
            retrigger_mode: RETRIGGER_MODE_CONTINUE,
            level_drift: default_drift(),
            depth_drift: default_drift(),
            texture: TEXTURE_OFF,
        }
    }
}

impl TimeEgParams {
    /// `level_drift`/`depth_drift`のどちらかが無効値(128)でない＝ループドリフトが有効か。
    /// 中立時（両方128）はこのフラグでガードし、ドリフト計算を一切実行しないことで
    /// 既存パッチの出力をビット単位で不変に保つ。
    pub fn has_drift(&self) -> bool {
        self.level_drift != BIPOLAR_NEUTRAL_RAW || self.depth_drift != BIPOLAR_NEUTRAL_RAW
    }
}

fn level_of(stage: &TimeStage) -> f32 {
    stage.level as f32 / 255.0
}

fn clamp_stage_count(stage_count: u8) -> usize {
    (stage_count as usize).clamp(1, MAX_STAGES)
}

/// `level_drift`(0〜255、128=無効)→ループ1周あたりレベル空間(0.0〜1.0)への加算量。
/// 128で厳密に0.0。バイポーラ（128未満は負方向=下降、128超は正方向=上昇）、
/// 両側とも距離1〜127を0.0010〜1.0の指数カーブへ写す（`pms_to_cents_range`と同じ
/// OnceLockテーブル化パターン）。MINは初期案0.0005→0.0015→0.0010と聴感プローブ
/// （`op505/core/examples/loop_drift_probe.rs`、BPF+ノコギリ波比較）で調整済み。
/// 詳細はmemory `project_timeeg_loop_drift.md`参照。
pub fn level_drift_per_cycle(raw: u8) -> f32 {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const MIN: f32 = 0.0010;
        const MAX: f32 = 1.0;
        let mut table = [0.0f32; 256];
        for (raw, slot) in table.iter_mut().enumerate() {
            let raw = raw as i32;
            let center = BIPOLAR_NEUTRAL_RAW as i32;
            if raw == center {
                continue;
            }
            let (sign, dist) = if raw < center {
                (-1.0, (center - raw) as f32)
            } else {
                (1.0, (raw - center) as f32)
            };
            *slot = sign * MIN * (MAX / MIN).powf(((dist - 1.0) / 126.0).clamp(0.0, 1.0));
        }
        table
    });
    table[raw as usize]
}

/// `depth_drift`(0〜255、128=等倍)→ループ1周あたり振れ幅への乗率。
/// 128で厳密に1.0、0≈0.5倍、255≈1.99倍（`2^((raw-128)/128)`、初期案）。
pub fn depth_drift_per_cycle(raw: u8) -> f32 {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0.0f32; 256];
        for (raw, slot) in table.iter_mut().enumerate() {
            let exponent = (raw as f32 - BIPOLAR_NEUTRAL_RAW as f32) / BIPOLAR_NEUTRAL_RAW as f32;
            *slot = 2f32.powf(exponent);
        }
        table
    });
    table[raw as usize]
}

/// ループ区間（`loop_start..=release_point`）のレベルレンジの中点。ドリフトは
/// この点を中心に振れ幅を伸縮する（`depth_drift`）。1段ループは跳ね戻し先レベルと
/// 段自身のレベルの中点を使う。`neutral_level`はキーオン起点（`TimeEg::neutral_level`と
/// 一致させること。`TimeEg`のランタイム計算（`advance`/`enter_stage`）と、`TimeEg`を
/// 持たないプレビュー描画（ui-core）の両方から呼ばれるため`pub`にしてある）。
pub fn loop_pivot_level(params: &TimeEgParams, neutral_level: f32, loop_start: usize, release_point: usize) -> f32 {
    if loop_start == release_point {
        let bounce =
            if loop_start == 0 { neutral_level } else { level_of(&params.stages[loop_start - 1]) };
        let stage_level = level_of(&params.stages[loop_start]);
        (bounce + stage_level) * 0.5
    } else {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for stage in &params.stages[loop_start..=release_point] {
            let l = level_of(stage);
            lo = lo.min(l);
            hi = hi.max(l);
        }
        (lo + hi) * 0.5
    }
}

/// `pivot`を中心に`level_offset`（加算）・`depth_gain`（乗算）のドリフトを`raw_level`へ適用し、
/// 0.0〜1.0へクランプする。
pub fn apply_loop_drift(pivot: f32, raw_level: f32, level_offset: f32, depth_gain: f32) -> f32 {
    (pivot + (raw_level - pivot) * depth_gain + level_offset).clamp(0.0, 1.0)
}

/// ループ区間（`loop_start..=release_point`）のレベルレンジ`(最小, 最大)`。`texture`が
/// 乱数抽選する値域を決めるのに使う（`loop_pivot_level`の中点だけでなく両端が要るため別関数）。
/// 1段ループは跳ね戻し先レベルと段自身のレベルの2値、多段ループは区間内の最小/最大。
pub fn loop_level_range(params: &TimeEgParams, neutral_level: f32, loop_start: usize, release_point: usize) -> (f32, f32) {
    if loop_start == release_point {
        let bounce =
            if loop_start == 0 { neutral_level } else { level_of(&params.stages[loop_start - 1]) };
        let stage_level = level_of(&params.stages[loop_start]);
        (bounce.min(stage_level), bounce.max(stage_level))
    } else {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for stage in &params.stages[loop_start..=release_point] {
            let l = level_of(stage);
            lo = lo.min(l);
            hi = hi.max(l);
        }
        (lo, hi)
    }
}

// ---------------------------------------------------------------------------
// 質感（texture）の乱数源
//
// S&H/Randomは一様乱数、Chaosはロジスティック写像。どちらも`TimeEg`内に持つ状態から
// 決定論的に進む（`note_on`/`retrigger`で固定初期値へリセットするため、同じMIDI入力からは
// 同じ乱数列が再現する。ボイス割当が`BTreeMap`で決定論化済みのため、レンダリング結果の
// ビット一致検証（golden/perf-bench）が壊れない）。
// ---------------------------------------------------------------------------

/// xorshift32。0は不動点のため`0`が渡ってきたら固定の非ゼロ値へ差し替える。
fn xorshift32(state: u32) -> u32 {
    let mut x = if state == 0 { 0x9E3779B9 } else { state };
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

/// `rng_state`を1歩進め、0.0〜1.0の一様乱数を返す（S&H/Random用）。
fn next_uniform(rng_state: &mut u32) -> f32 {
    *rng_state = xorshift32(*rng_state);
    (*rng_state >> 8) as f32 / (1u32 << 24) as f32
}

/// `chaos_state`（0.0〜1.0）をロジスティック写像`x=3.9x(1-x)`で1歩進め、その値を返す（Chaos用）。
/// 不動点0.0/1.0付近に収束しないよう、極端に0や1へ寄った値は中央寄りへ引き戻す。
fn next_chaos(chaos_state: &mut f32) -> f32 {
    let x = chaos_state.clamp(0.001, 0.999);
    *chaos_state = (3.9 * x * (1.0 - x)).clamp(0.001, 0.999);
    *chaos_state
}

/// `texture`が乱数抽選した値を`(lo, hi)`へ線形マップする。
fn texture_target_level(texture: u8, rng_state: &mut u32, chaos_state: &mut f32, lo: f32, hi: f32) -> f32 {
    let r = if texture == TEXTURE_CHAOS { next_chaos(chaos_state) } else { next_uniform(rng_state) };
    lo + r * (hi - lo)
}

/// ループを`cycles`周ぶん折り返した後の累積`(level_offset, depth_gain)`を返す
/// （0周＝まだ一度も折り返していない中立値`(0.0, 1.0)`）。`TimeEg`自身の累積とは独立で、
/// プレビュー描画（ui-core）がランタイム状態を持たずに「N周先の見た目」を計算するために使う。
pub fn drift_accumulated_after_cycles(params: &TimeEgParams, cycles: usize) -> (f32, f32) {
    if !params.has_drift() || cycles == 0 {
        return (0.0, 1.0);
    }
    let per_level = level_drift_per_cycle(params.level_drift);
    let per_depth = depth_drift_per_cycle(params.depth_drift);
    (per_level * cycles as f32, per_depth.powi(cycles as i32))
}

// ---------------------------------------------------------------------------
// テンポ同期
//
// LFOのテンポ同期（1周＝指定音価）と同じ考え方をTimeEgへ適用する（方式A）。
// ループ有効なら`loop_start..=release_point`の1周、無効なら`0..=release_point`の
// 保持区間全体を対象に、その素の所要時間を指定レートちょうどへ伸縮する`speed_scale`倍率を返す。
// 同期先は`sync_rate`(0〜255)の連続値で、20音価がアンカーとして厳密に踏める（下記参照）。
// アタックやリリースも同じ比率で一緒に伸縮する（対象区間より外は同期の管轄外）ため、
// 「ビブラートだけ同期してアタックは固定秒数」という表現はできない
// （区間ごとに速度を分けるにはtick()自体の再設計が要る、次善は将来課題）。
// ---------------------------------------------------------------------------

/// 同期音価テーブルの段数。付点・3連を含み、所要時間の昇順（index 0が最短）。
pub const SYNC_NOTE_COUNT: usize = 20;

/// 同期音価テーブル。値は拍数（4分音符=1.0）。所要時間の昇順に並べてあるため、
/// ノブ/セレクタでindexを増やすと単調に長くなる。index 10 = 1/4（1拍、既定）。
///
/// **注意（2026-08-17修正）**: 初版は音符の分数そのもの（1/4→`1.0/4.0`、1/1→`1.0`）を
/// 入れており、拍数の1/4しかなかった。`tempo_speed_scale()`が`* 60.0 / bpm`（＝1拍の秒数）を
/// 掛けるため、全音価が**4倍速い**状態だった（BPM120の「1/4」が0.125秒）。
/// テーブル名・関数名・下の表がいずれも「拍数」と言っているのに配列だけが分数だった、
/// という意味論の不一致。`sync_note_beats_matches_documented_table`テストで再発を防ぐ。
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
    4.0 / 32.0 * (2.0 / 3.0),
    4.0 / 32.0,
    4.0 / 16.0 * (2.0 / 3.0),
    4.0 / 32.0 * 1.5,
    4.0 / 16.0,
    4.0 / 8.0 * (2.0 / 3.0),
    4.0 / 16.0 * 1.5,
    4.0 / 8.0,
    4.0 / 4.0 * (2.0 / 3.0),
    4.0 / 8.0 * 1.5,
    4.0 / 4.0,
    4.0 / 2.0 * (2.0 / 3.0),
    4.0 / 4.0 * 1.5,
    4.0 / 2.0,
    4.0 * (2.0 / 3.0),
    4.0 / 2.0 * 1.5,
    4.0,
    4.0 * 1.5,
    8.0,
    16.0,
];

/// 同期音価テーブルのindex(0〜19)→拍数（4分音符=1.0）。範囲外はクランプする。
pub fn sync_note_beats(index: u8) -> f32 {
    SYNC_NOTE_BEATS[(index as usize).min(SYNC_NOTE_COUNT - 1)]
}

// ---------------------------------------------------------------------------
// 連続レート（音価アンカー＋指数補間）
//
// `sync_rate`(0〜255)は、上の20音価を「アンカー」として正確に踏みながら、その間を
// 幾何補間で無段階に埋める。単純な指数マッピング（min〜maxを255等分）だと1目盛り約2.1%で、
// ドロップダウンから「1/4」を選んでも最寄りの目盛りが1%程度ズレ、同期がじわじわ狂ってしまう。
// アンカー方式ならドロップダウン選択は厳密に音価どおり、ノブは連続、という両立ができる。
// （このリポジトリで実績のある「理論値アンカー＋指数補間」パターン。AR/D1R/FB/KSRでも採用）
// ---------------------------------------------------------------------------

/// 音価index(0〜19)→`sync_rate`のアンカー値。`round(i * 255 / 19)`を展開したもの
/// （constで`round()`が使えないため定数畳み込み済みの実値を置く。両端が0/255になり、
/// 間隔は13〜14で概ね均等）。
const SYNC_NOTE_ANCHORS: [u8; SYNC_NOTE_COUNT] = [
    0, 13, 27, 40, 54, 67, 81, 94, 107, 121, 134, 148, 161, 175, 188, 201, 215, 228, 242, 255,
];

/// 音価index(0〜19)→その音価にぴったり乗る`sync_rate`値。範囲外はクランプする。
/// UIのドロップダウンで音価を選んだときに、この値をノブへ書き込む。
pub fn sync_note_anchor(index: u8) -> u8 {
    SYNC_NOTE_ANCHORS[(index as usize).min(SYNC_NOTE_COUNT - 1)]
}

/// `sync_rate`(0〜255)→拍数の256要素テーブルを構築する。
/// アンカー上では`SYNC_NOTE_BEATS`と厳密に一致し、アンカー間は幾何補間（比が指数的に動く）。
fn build_sync_rate_beats_table() -> [f32; 256] {
    let mut table = [0.0f32; 256];
    for seg in 0..SYNC_NOTE_COUNT - 1 {
        let (lo, hi) = (SYNC_NOTE_ANCHORS[seg] as usize, SYNC_NOTE_ANCHORS[seg + 1] as usize);
        let (b_lo, b_hi) = (SYNC_NOTE_BEATS[seg], SYNC_NOTE_BEATS[seg + 1]);
        for (rate, slot) in table.iter_mut().enumerate().take(hi + 1).skip(lo) {
            // 端点はアンカー値をそのまま代入し、powf()の丸め誤差が乗らないようにする
            // （ドロップダウン選択が「厳密に音価どおり」であることをテストで保証するため）。
            *slot = if rate == lo {
                b_lo
            } else if rate == hi {
                b_hi
            } else {
                let t = (rate - lo) as f32 / (hi - lo) as f32;
                b_lo * (b_hi / b_lo).powf(t)
            };
        }
    }
    table
}

fn sync_rate_beats_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(build_sync_rate_beats_table)
}

/// `sync_rate`(0〜255)→拍数（4分音符=1.0）。`tempo_speed_scale()`が毎サンプル呼ばれるため
/// `powf()`は初回のテーブル構築時のみ実行する（`time_to_seconds`と同じOnceLockパターン）。
pub fn sync_rate_beats(rate: u8) -> f32 {
    sync_rate_beats_table()[rate as usize]
}

/// `sync_rate`に最も近い音価indexと、それがアンカーへ厳密に乗っているか否かを返す。
/// UIのドロップダウンが「今どのあたりか」を表示するのに使う（乗っていなければ`~1/8`のように
/// 近似表示する）。
pub fn nearest_sync_note(rate: u8) -> (u8, bool) {
    let mut best = 0usize;
    let mut best_dist = u16::MAX;
    for (i, anchor) in SYNC_NOTE_ANCHORS.iter().enumerate() {
        let dist = (*anchor as i16 - rate as i16).unsigned_abs();
        if dist < best_dist {
            best_dist = dist;
            best = i;
        }
    }
    (best as u8, best_dist == 0)
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
/// `対象区間の素の秒数 ÷ 指定レートの秒数`で、対象区間ちょうどが`sync_rate`どおりの長さに
/// なるよう時間軸を伸縮する。同期無効・対象区間が実質0秒・bpmが0以下のいずれかなら
/// 1.0（無補正）を返す
/// （既存のCC76由来速度補正等、他の`speed_scale`要因と乗算で共存できる設計）。
pub fn tempo_speed_scale(params: &TimeEgParams, bpm: f32) -> f32 {
    if params.sync_enabled == 0 || bpm <= 0.0 {
        return 1.0;
    }
    let region = sync_region_seconds(params);
    if region <= f32::EPSILON {
        return 1.0;
    }
    let target_seconds = sync_rate_beats(params.sync_rate) * 60.0 / bpm;
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
    /// キーオン起点および1段ループの跳ね戻し先に使う「無変調」レベル。
    ///
    /// 振幅系（OP EG／Gain FG）は0.0＝無音が無変調なので既定のまま。Pitch FG／Cutoff FGの
    /// ようにレベルをバイポーラ解釈する用途では、0.0は「無変調」ではなく「全開マイナス」を
    /// 意味してしまうため、`new_bipolar()`で`BIPOLAR_NEUTRAL_LEVEL`を入れる
    /// （さもないと段0のtimeが0でない限り、キーオン直後に全開マイナスからの
    /// スイープが必ず入ってしまう）。
    neutral_level: f32,
    /// ループドリフト（`level_drift`）の累積オフセット。ループが1周するたびに加算される。
    /// note_on/retriggerで0.0へリセットする（累積は「今のノートで何周したか」を表すため）。
    level_offset: f32,
    /// ループドリフト（`depth_drift`）の累積ゲイン。ループが1周するたびに乗算される。
    /// note_on/retriggerで1.0へリセットする。
    depth_gain: f32,
    /// `texture`（S&H/Random）用の乱数状態。ループ区間の段へ入るたびに1歩進む。
    /// note_on/retriggerで固定初期値へリセットし、同じ入力からは同じ乱数列を再現する
    /// （golden/perf-benchのビット一致検証が壊れないようにするため）。
    texture_rng: u32,
    /// `texture`（Chaos）用のロジスティック写像状態(0.0〜1.0)。同じくnote_on/retriggerでリセット。
    texture_chaos: f32,
}

/// `texture_rng`のnote_on/retrigger既定シード（0だと不動点のため非ゼロの固定値）。
const TEXTURE_RNG_SEED: u32 = 0x9E3779B9;
/// `texture_chaos`のnote_on/retrigger既定値（0.0/1.0付近だと収束が遅いため中央寄りの固定値）。
const TEXTURE_CHAOS_SEED: f32 = 0.37;

impl TimeEg {
    pub fn new() -> Self {
        Self::with_neutral_level(0.0)
    }

    /// レベルをバイポーラ（中心128＝無変調）として扱う用途向けのコンストラクタ。
    /// Pitch FG／Cutoff FGが使う。振幅系は`new()`のまま。
    pub fn new_bipolar() -> Self {
        Self::with_neutral_level(BIPOLAR_NEUTRAL_LEVEL)
    }

    fn with_neutral_level(neutral_level: f32) -> Self {
        Self {
            stage_index: 0,
            level: neutral_level,
            segment_start: neutral_level,
            segment_end: neutral_level,
            elapsed: 0.0,
            releasing: false,
            pending_start: PendingStart::None,
            release_pending: false,
            idle: true,
            neutral_level,
            level_offset: 0.0,
            depth_gain: 1.0,
            texture_rng: TEXTURE_RNG_SEED,
            texture_chaos: TEXTURE_CHAOS_SEED,
        }
    }

    pub fn note_on(&mut self) {
        self.level = self.neutral_level;
        self.stage_index = 0;
        self.segment_start = self.neutral_level;
        self.segment_end = self.neutral_level;
        self.elapsed = 0.0;
        self.releasing = false;
        self.release_pending = false;
        self.pending_start = PendingStart::Fresh;
        self.idle = false;
        self.level_offset = 0.0;
        self.depth_gain = 1.0;
        self.texture_rng = TEXTURE_RNG_SEED;
        self.texture_chaos = TEXTURE_CHAOS_SEED;
    }

    /// 残響レベルを保持したまま段0へ再突入する（`eg::Eg::retrigger`と同じ思想）。
    pub fn retrigger(&mut self) {
        self.releasing = false;
        self.release_pending = false;
        self.pending_start = PendingStart::Retrigger;
        self.elapsed = 0.0;
        self.idle = false;
        self.level_offset = 0.0;
        self.depth_gain = 1.0;
        self.texture_rng = TEXTURE_RNG_SEED;
        self.texture_chaos = TEXTURE_CHAOS_SEED;
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
                PendingStart::Fresh => self.neutral_level,
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
                self.enter_stage(params, stage_count, cur + 1, overflow);
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
                // ループが1周完了した瞬間。ドリフト（level_drift/depth_drift）の累積を
                // ここで更新する（`params.has_drift()`で中立時は0演算のままスキップし、
                // 既存パッチの出力をビット単位で不変に保つ）。
                if params.has_drift() {
                    self.level_offset += level_drift_per_cycle(params.level_drift);
                    self.depth_gain *= depth_drift_per_cycle(params.depth_drift);
                }
                // 1段ループ（`loop_start == release_point`）だけは、その段の**入口レベル**
                // （直前段の終端レベル、段0なら`neutral_level`）へ跳ね戻してからやり直す。
                // レベル連続のままだと「自分の終端レベルから自分の終端レベルへ」向かうことになり
                // 完全に平坦で音として何も起きない。跳ね戻すことで1段だけでノコギリ波を表現できる。
                // 多段ループ（`loop_start < release_point`）は従来どおりレベル連続で周回する
                // （区間の両端を行き来する三角波的な動きになる）。
                // `texture`が有効なときは跳ね戻し自体を行わない。ノコギリ波の「同じ動きの
                // 繰り返し」を作るための仕掛けだが、textureは`enter_stage`が毎回新しい乱数
                // ターゲットへ向かうため、現在レベルから素直に続けた方が意図に合う。
                if loop_start == release_point && params.texture == TEXTURE_OFF {
                    let bounce_raw = if loop_start == 0 {
                        self.neutral_level
                    } else {
                        level_of(&params.stages[loop_start - 1])
                    };
                    self.level = if params.has_drift() {
                        let pivot = loop_pivot_level(params, self.neutral_level, loop_start, release_point);
                        apply_loop_drift(pivot, bounce_raw, self.level_offset, self.depth_gain)
                    } else {
                        bounce_raw
                    };
                }
                self.enter_stage(params, stage_count, loop_start, overflow);
            } else {
                self.settle_at_current_level(cur);
            }
        } else if cur + 1 < stage_count {
            self.enter_stage(params, stage_count, cur + 1, overflow);
        } else {
            // stage_countの終端に達したがrelease_pointに届いていない設定不整合。そこで静止する。
            self.settle_at_current_level(cur);
        }
    }

    /// `idx`段がループ区間（`loop_start..=release_point`、ループ無効なら常にfalse）に
    /// 属するか。属する段だけがドリフト（累積オフセット/ゲイン）の対象になる。
    fn stage_in_loop_region(params: &TimeEgParams, stage_count: usize, idx: usize) -> bool {
        if params.loop_enabled == 0 {
            return false;
        }
        let release_point = (params.release_point as usize).min(stage_count - 1);
        let loop_start = (params.loop_start as usize).min(release_point);
        idx >= loop_start && idx <= release_point
    }

    fn enter_stage(&mut self, params: &TimeEgParams, stage_count: usize, next: usize, overflow: f64) {
        self.stage_index = next;
        self.segment_start = self.level;
        let raw_end = level_of(&params.stages[next]);
        let in_loop = Self::stage_in_loop_region(params, stage_count, next);
        self.segment_end = if params.texture != TEXTURE_OFF && in_loop {
            let release_point = (params.release_point as usize).min(stage_count - 1);
            let loop_start = (params.loop_start as usize).min(release_point);
            let (lo, hi) = loop_level_range(params, self.neutral_level, loop_start, release_point);
            let target =
                texture_target_level(params.texture, &mut self.texture_rng, &mut self.texture_chaos, lo, hi);
            if params.texture != TEXTURE_RANDOM {
                // S&H/Chaos: 段の時間に関わらず即座にジャンプしてホールドする
                // （Randomだけは`segment_start`を現在レベルのまま残し、時間をかけて補間する）。
                self.segment_start = target;
            }
            target
        } else if params.has_drift() && in_loop {
            let release_point = (params.release_point as usize).min(stage_count - 1);
            let loop_start = (params.loop_start as usize).min(release_point);
            let pivot = loop_pivot_level(params, self.neutral_level, loop_start, release_point);
            apply_loop_drift(pivot, raw_end, self.level_offset, self.depth_gain)
        } else {
            raw_end
        };
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

    /// バイポーラ写像の要： 生値128がぴったり0（無変調）になること。
    /// ここがずれると「DEPTHを上げただけで音がずれる」という形で表面化する。
    #[test]
    fn bipolar_level_maps_raw_128_to_zero() {
        assert_eq!(bipolar_level(BIPOLAR_NEUTRAL_LEVEL), 0.0);
        assert_eq!(bipolar_level(0.0), -1.0);
        // 正側は`(v-128)/128`慣例により127/128で頭打ち（DT1等と同じ非対称性）。
        assert!((bipolar_level(1.0) - 127.0 / 128.0).abs() < 1e-6, "{}", bipolar_level(1.0));
    }

    /// `new_bipolar()`のキーオン起点が中央（無変調）であること。
    /// `new()`の0.0起点のままだと、段0のtimeが0でない限りキーオン直後に
    /// 全開マイナスからのスイープが必ず入ってしまう。
    #[test]
    fn bipolar_time_eg_starts_from_neutral_not_zero() {
        let sr = 44100.0;
        // 段0: 中央(128)へ向かって時間をかけて進む形。起点が中央なら終始無変調のまま。
        let params = TimeEgParams {
            stages: stages_with(&[(200, BIPOLAR_NEUTRAL_RAW, 0), (0, BIPOLAR_NEUTRAL_RAW, 0)]),
            stage_count: 2,
            release_point: 0,
            ..Default::default()
        };

        let mut bipolar = TimeEg::new_bipolar();
        bipolar.note_on();
        for _ in 0..64 {
            let out = bipolar.tick(sr, params, 1.0);
            assert!(
                bipolar_level(out).abs() < 1e-6,
                "バイポーラEGはキーオン直後から無変調であるべき: {}",
                bipolar_level(out)
            );
        }

        // 従来の`new()`は0.0起点のまま（振幅系の意味論は変えない）。
        let mut unipolar = TimeEg::new();
        unipolar.note_on();
        let first = unipolar.tick(sr, params, 1.0);
        assert!(first < BIPOLAR_NEUTRAL_LEVEL, "振幅系EGは従来どおり0.0から立ち上がる: {first}");
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

    // -----------------------------------------------------------------------
    // ループドリフト（level_drift / depth_drift）
    // -----------------------------------------------------------------------

    fn drift_test_cycle_samples(sr: f32, stage_time: u8) -> usize {
        (time_to_seconds(stage_time) as f64 * 2.0 * sr as f64) as usize
    }

    fn drift_test_attack_samples(sr: f32, stage_time: u8) -> usize {
        (time_to_seconds(stage_time) as f64 * sr as f64) as usize + 50
    }

    #[test]
    fn level_drift_makes_loop_center_descend_over_cycles() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(30, 255, 0), (30, 200, 0), (30, 100, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
            level_drift: 40, // 128未満=下降方向
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        let cycle_samples = drift_test_cycle_samples(sr, 30);
        for _ in 0..drift_test_attack_samples(sr, 30) {
            eg.tick(sr, params, 1.0);
        }

        let mut first_cycle_max = 0.0f32;
        for _ in 0..cycle_samples {
            first_cycle_max = first_cycle_max.max(eg.tick(sr, params, 1.0));
        }
        for _ in 0..(cycle_samples * 8) {
            eg.tick(sr, params, 1.0);
        }
        let mut later_cycle_max = 0.0f32;
        for _ in 0..cycle_samples {
            later_cycle_max = later_cycle_max.max(eg.tick(sr, params, 1.0));
        }

        assert!(
            later_cycle_max < first_cycle_max - 0.05,
            "level_drift should make the loop descend over cycles: first={first_cycle_max} later={later_cycle_max}"
        );
        assert!(!eg.is_idle(), "drifting loop should never become idle on its own");
    }

    #[test]
    fn depth_drift_shrinks_swing_over_cycles() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(30, 255, 0), (30, 200, 0), (30, 50, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
            depth_drift: 40, // 128未満=縮小方向
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        let cycle_samples = drift_test_cycle_samples(sr, 30);
        for _ in 0..drift_test_attack_samples(sr, 30) {
            eg.tick(sr, params, 1.0);
        }

        let swing_over = |eg: &mut TimeEg, n: usize| -> f32 {
            let (mut lo, mut hi) = (1.0f32, 0.0f32);
            for _ in 0..n {
                let out = eg.tick(sr, params, 1.0);
                lo = lo.min(out);
                hi = hi.max(out);
            }
            hi - lo
        };

        let first_swing = swing_over(&mut eg, cycle_samples);
        for _ in 0..(cycle_samples * 8) {
            eg.tick(sr, params, 1.0);
        }
        let later_swing = swing_over(&mut eg, cycle_samples);

        assert!(
            later_swing < first_swing * 0.7,
            "depth_drift should shrink the swing over cycles: first={first_swing} later={later_swing}"
        );
        assert!(!eg.is_idle());
    }

    /// リリース区間はループ区間の外なので、ドリフトが蓄積していてもrawレベルへ着地し、
    /// idleになる（ボイス解放条件「全4オペレーターがidle」を壊さないための必須要件）。
    #[test]
    fn release_settles_at_raw_level_ignoring_accumulated_drift() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(30, 255, 0), (30, 200, 0), (30, 100, 0), (30, 0, 0)]),
            stage_count: 4,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
            level_drift: 220, // 上昇方向、大きめに蓄積させる
            depth_drift: 220, // 拡大方向、大きめに蓄積させる
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        // 複数周ループさせてドリフトを蓄積させる。
        for _ in 0..(drift_test_cycle_samples(sr, 30) * 6) {
            eg.tick(sr, params, 1.0);
        }
        assert!(!eg.is_idle());

        eg.note_off();
        let mut became_idle = false;
        let mut final_level = -1.0;
        for _ in 0..200_000 {
            let level = eg.tick(sr, params, 1.0);
            if eg.is_idle() {
                became_idle = true;
                final_level = level;
                break;
            }
        }
        assert!(became_idle, "expected note_off to walk to Idle even with drift accumulated");
        assert!(
            (final_level - 0.0).abs() < 1e-6,
            "release stage should settle at raw level 0.0, unaffected by drift: {final_level}"
        );
    }

    #[test]
    fn note_on_resets_drift_accumulators() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(30, 255, 0), (30, 200, 0), (30, 100, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
            level_drift: 40,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        let cycle_samples = drift_test_cycle_samples(sr, 30);
        let attack_samples = drift_test_attack_samples(sr, 30);

        for _ in 0..attack_samples {
            eg.tick(sr, params, 1.0);
        }
        let mut first_run_max = 0.0f32;
        for _ in 0..cycle_samples {
            first_run_max = first_run_max.max(eg.tick(sr, params, 1.0));
        }

        // さらにループさせてドリフトを蓄積させておく。
        for _ in 0..(cycle_samples * 5) {
            eg.tick(sr, params, 1.0);
        }

        // note_onで再スタート。累積がリセットされていれば1回目と同じ最初の周のmaxになる。
        eg.note_on();
        for _ in 0..attack_samples {
            eg.tick(sr, params, 1.0);
        }
        let mut second_run_max = 0.0f32;
        for _ in 0..cycle_samples {
            second_run_max = second_run_max.max(eg.tick(sr, params, 1.0));
        }

        assert!(
            (second_run_max - first_run_max).abs() < 0.01,
            "note_on should reset drift accumulators: first={first_run_max} second={second_run_max}"
        );
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
        // level_drift/depth_driftは旧バンクに存在しない。derive(Deserialize)の`#[serde(default)]`が
        // 素の0を入れるとバイポーラ値としては「最大マイナス」になり全パッチが壊れる
        // （過去にDT1/pitch_fg.depthで実際に踏んだバグ。memory `project_bipolar_default_center_bugfix`）。
        assert_eq!(params.level_drift, BIPOLAR_NEUTRAL_RAW, "missing level_drift should default to neutral(128), not 0");
        assert_eq!(params.depth_drift, BIPOLAR_NEUTRAL_RAW, "missing depth_drift should default to neutral(128), not 0");
        assert!(!params.has_drift());
    }

    // -----------------------------------------------------------------------
    // テンポ同期
    // -----------------------------------------------------------------------

    /// テーブルの値が「拍数」というdoc/関数名の宣言どおりであることを固定する。
    /// 初版は音符の分数（1/4→0.25）が入っており、全音価が4倍速かった。
    #[test]
    fn sync_note_beats_matches_documented_table() {
        assert_eq!(sync_note_beats(10), 1.0, "1/4は1拍");
        assert_eq!(sync_note_beats(13), 2.0, "1/2は2拍");
        assert_eq!(sync_note_beats(16), 4.0, "1/1（1小節）は4拍");
        assert_eq!(sync_note_beats(19), 16.0, "4/1（4小節）は16拍");
        assert_eq!(sync_note_beats(1), 0.125, "1/32は0.125拍");
        // 昇順に並んでいること（UIでindexを増やすと単調に長くなる前提）
        for i in 1..SYNC_NOTE_COUNT {
            assert!(
                sync_note_beats(i as u8) > sync_note_beats(i as u8 - 1),
                "index {i} が単調増加していない"
            );
        }
    }

    /// 同期対象区間が、指定した音価ちょうどの実時間になること。
    /// BPM120・1/4（＝0.5秒）に対し素の区間が2.0秒なら、時間軸を4倍速で回せばよい。
    #[test]
    fn tempo_speed_scale_makes_region_take_exactly_one_note() {
        let params = TimeEgParams {
            stages: stages_with(&[(seconds_to_time(2.0), 255, 0), (0, 0, 0)]),
            stage_count: 2,
            release_point: 0,
            sync_enabled: 1,
            sync_rate: sync_note_anchor(10), // 1/4
            ..Default::default()
        };
        let region = sync_region_seconds(&params);
        assert!((region - 2.0).abs() < 0.02, "region={region}");

        let scale = tempo_speed_scale(&params, 120.0);
        // 区間の実時間 = region / scale が、1拍＝0.5秒になること
        let actual_seconds = region / scale;
        assert!((actual_seconds - 0.5).abs() < 0.005, "actual={actual_seconds}");

        // BPMが半分なら1拍は倍の長さになる
        let slow = region / tempo_speed_scale(&params, 60.0);
        assert!((slow - 1.0).abs() < 0.01, "slow={slow}");
    }

    /// ドロップダウンで音価を選んだとき（＝アンカー値をノブへ書いたとき）、
    /// 連続レートが音価テーブルの値と**厳密に**一致すること。ここがズレると同期が狂う。
    #[test]
    fn sync_rate_anchors_hit_note_values_exactly() {
        for i in 0..SYNC_NOTE_COUNT as u8 {
            let rate = sync_note_anchor(i);
            assert_eq!(
                sync_rate_beats(rate),
                sync_note_beats(i),
                "index {i} (rate={rate}) がアンカー上で一致しない"
            );
        }
        // 両端がテーブルの端に対応していること
        assert_eq!(sync_note_anchor(0), 0);
        assert_eq!(sync_note_anchor(SYNC_NOTE_COUNT as u8 - 1), 255);
    }

    /// ノブを右に回すと必ず長くなること（アンカー間の補間も含めて単調）。
    #[test]
    fn sync_rate_is_monotonic() {
        for rate in 1..=255u8 {
            assert!(
                sync_rate_beats(rate) > sync_rate_beats(rate - 1),
                "rate {rate} で単調増加していない: {} <= {}",
                sync_rate_beats(rate),
                sync_rate_beats(rate - 1)
            );
        }
    }

    /// ドロップダウン表示用の逆引き。アンカー上では厳密一致フラグが立ち、
    /// 途中の値では最寄りの音価が返る。
    #[test]
    fn nearest_sync_note_round_trips_anchors() {
        for i in 0..SYNC_NOTE_COUNT as u8 {
            let (index, exact) = nearest_sync_note(sync_note_anchor(i));
            assert_eq!(index, i, "index {i} のアンカーが逆引きできない");
            assert!(exact, "index {i} のアンカーが厳密一致と判定されない");
        }
        // アンカー(134=1/4)から1目盛りずらすと、最寄りは1/4のままだが厳密一致ではなくなる
        let (index, exact) = nearest_sync_note(135);
        assert_eq!(index, 10);
        assert!(!exact);
    }

    /// 同期が無効／BPM未確定のときは補正しない（他の`speed_scale`要因を素通しする）。
    #[test]
    fn tempo_speed_scale_is_neutral_when_disabled() {
        let mut params = TimeEgParams {
            stages: stages_with(&[(seconds_to_time(2.0), 255, 0), (0, 0, 0)]),
            stage_count: 2,
            release_point: 0,
            sync_enabled: 0,
            sync_rate: sync_note_anchor(10),
            ..Default::default()
        };
        assert_eq!(tempo_speed_scale(&params, 120.0), 1.0, "sync_enabled=0なら無補正");

        params.sync_enabled = 1;
        assert_eq!(tempo_speed_scale(&params, 0.0), 1.0, "bpm=0なら無補正");
        assert_eq!(tempo_speed_scale(&params, -1.0), 1.0, "bpm<0なら無補正");

        // 対象区間が実質0秒（全段time=0）でも0除算せず無補正
        let empty = TimeEgParams { sync_enabled: 1, ..Default::default() };
        assert_eq!(tempo_speed_scale(&empty, 120.0), 1.0);
    }

    // -----------------------------------------------------------------------
    // 質感（texture: S&H / Random / Chaos）
    // -----------------------------------------------------------------------

    /// `texture=OFF`が既定であること（旧`.op505`バンクのserde欠落時フォールバックと同じ値）。
    #[test]
    fn texture_defaults_to_off() {
        assert_eq!(TimeEgParams::default().texture, TEXTURE_OFF);
    }

    /// `texture=OFF`のときは既存のドリフトテスト群がそのまま通ることが暗黙の回帰保証だが、
    /// ここでは明示的に「1段ループのbounce」がtexture=OFF時は従来どおり作動することを確認する
    /// （`advance()`の`loop_start == release_point && params.texture == TEXTURE_OFF`分岐）。
    #[test]
    fn texture_off_preserves_single_stage_loop_sawtooth() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(30, 255, 0)]),
            stage_count: 1,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 0,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();
        let cycle_samples = drift_test_cycle_samples(sr, 30) / 2 + 10;
        // ノコギリ波: 毎周期0.0から255/255へ直線的に登る。
        let mut min_seen = 1.0f32;
        for _ in 0..(cycle_samples * 4) {
            min_seen = min_seen.min(eg.tick(sr, params, 1.0));
        }
        assert!(min_seen < 0.05, "texture=OFFなら毎周期0.0へ跳ね戻るはず: min_seen={min_seen}");
    }

    /// S&H: ループ区間の段内はレベルが一定（階段状）で、段境界でジャンプすること。
    #[test]
    fn sample_hold_holds_level_within_stage() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(30, 200, 0), (30, 200, 0)]),
            stage_count: 2,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 1,
            texture: TEXTURE_SAMPLE_HOLD,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        let stage_samples = (time_to_seconds(30) as f64 * sr as f64) as usize;
        // 最初の段（アタック相当）を抜けて、ループ内の1段ぶんを観測する。
        for _ in 0..(stage_samples + 5) {
            eg.tick(sr, params, 1.0);
        }
        let held = eg.tick(sr, params, 1.0);
        for _ in 0..(stage_samples - 10) {
            let level = eg.tick(sr, params, 1.0);
            assert!((level - held).abs() < 1e-6, "S&H中は段内で一定のはず: held={held} level={level}");
        }
    }

    /// S&Hが実際にランダム値へ飛ぶこと（決め打ちの1値へ張り付かないこと）を、
    /// 十分な段数を観測して値のばらつきで確認する。
    #[test]
    fn sample_hold_visits_multiple_distinct_levels() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(20, 255, 0), (10, 200, 0), (10, 0, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
            texture: TEXTURE_SAMPLE_HOLD,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        let stage_samples = (time_to_seconds(10) as f64 * sr as f64) as usize + 5;
        let attack_samples = (time_to_seconds(20) as f64 * sr as f64) as usize + 5;
        for _ in 0..attack_samples {
            eg.tick(sr, params, 1.0);
        }

        let mut distinct = std::collections::BTreeSet::new();
        for _ in 0..40 {
            let level = eg.tick(sr, params, 1.0);
            for _ in 0..(stage_samples - 1) {
                eg.tick(sr, params, 1.0);
            }
            distinct.insert((level * 1000.0).round() as i32);
        }
        assert!(distinct.len() > 3, "S&Hは複数の異なるレベルを訪れるはず: distinct={}", distinct.len());
    }

    /// Random: S&Hと違い、段の間はレベルが動き続ける（一定に留まらない）こと。
    /// 段のレベルをあえて異ならせ（lo/hiに幅を持たせ）、乱数ターゲットが現在値と
    /// 一致してしまう確率を下げた上で、複数周期にわたり観測する。
    #[test]
    fn random_interpolates_within_stage_unlike_sample_hold() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(30, 255, 0), (30, 20, 0)]),
            stage_count: 2,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 1,
            texture: TEXTURE_RANDOM,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        // 最初の段0到達（`pending_start`経由、texture未適用）を抜ける。
        let stage_samples = (time_to_seconds(30) as f64 * sr as f64) as usize;
        for _ in 0..(stage_samples + 5) {
            eg.tick(sr, params, 1.0);
        }

        let mut saw_change = false;
        for _ in 0..8 {
            let start = eg.tick(sr, params, 1.0);
            for _ in 0..(stage_samples - 10) {
                let level = eg.tick(sr, params, 1.0);
                if (level - start).abs() > 1e-4 {
                    saw_change = true;
                }
            }
            // 次の段へ（ループ境界をまたぐ）。
            for _ in 0..10 {
                eg.tick(sr, params, 1.0);
            }
        }
        assert!(saw_change, "Randomは段の途中でレベルが動き続けるはず（複数周期観測）");
    }

    /// texture有効時、乱数ターゲットはループ区間のレベル最小〜最大の範囲内に収まること。
    /// note_on直後の段0到達（`pending_start`経由、アタック相当）はtexture未適用の生値を
    /// 辿るため、最初の1周ぶんはスキップしてから観測する（drift系テストの
    /// `drift_test_attack_samples`と同じ考え方）。
    #[test]
    fn texture_targets_stay_within_loop_level_range() {
        let sr = 44100.0;
        let (lo, hi) = (50u8, 220u8);
        let params = TimeEgParams {
            stages: stages_with(&[(10, hi, 0), (10, lo, 0), (10, hi, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 2,
            texture: TEXTURE_RANDOM,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        // 段0→段1→段2（アタック相当、3段ぶん）をスキップしてループ折返し後だけ観測する。
        let stage_samples = (time_to_seconds(10) as f64 * sr as f64) as usize;
        for _ in 0..(stage_samples * 3 + 15) {
            eg.tick(sr, params, 1.0);
        }

        let (lo_f, hi_f) = (lo as f32 / 255.0, hi as f32 / 255.0);
        for _ in 0..200_000 {
            let level = eg.tick(sr, params, 1.0);
            assert!(
                level >= lo_f - 1e-3 && level <= hi_f + 1e-3,
                "texture targetはループ区間のレベル範囲内のはず: level={level} range=[{lo_f},{hi_f}]"
            );
        }
    }

    /// Chaos: ロジスティック写像なので決定論的。同じnote_onからは同じレベル列が再現する。
    #[test]
    fn chaos_is_deterministic_across_note_on() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(10, 255, 0), (10, 200, 0), (10, 0, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
            texture: TEXTURE_CHAOS,
         ..Default::default()};

        let run = || {
            let mut eg = TimeEg::new();
            eg.note_on();
            let mut samples = Vec::new();
            for _ in 0..20_000 {
                samples.push(eg.tick(sr, params, 1.0));
            }
            samples
        };

        assert_eq!(run(), run(), "Chaosは決定論的で、同じnote_onからは同じ列を再現するはず");
    }

    /// note_on/retriggerで乱数状態がリセットされ、1回目と同じ列が再現すること
    /// （`level_drift`累積のリセットテストと同型）。
    #[test]
    fn note_on_resets_texture_rng() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(10, 255, 0), (10, 200, 0), (10, 50, 0)]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 1,
            release_point: 2,
            texture: TEXTURE_SAMPLE_HOLD,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();

        let mut first_run = Vec::new();
        for _ in 0..5_000 {
            first_run.push(eg.tick(sr, params, 1.0));
        }

        // 大きく進めて乱数状態をnote_on時と違う位置まで動かしておく。
        for _ in 0..20_000 {
            eg.tick(sr, params, 1.0);
        }

        eg.note_on();
        let mut second_run = Vec::new();
        for _ in 0..5_000 {
            second_run.push(eg.tick(sr, params, 1.0));
        }

        assert_eq!(first_run, second_run, "note_onは乱数状態を固定初期値へリセットするはず");
    }

    /// ワンショット（`loop_enabled=0`）ではtextureが効かない（周回自体がないため）。
    #[test]
    fn texture_has_no_effect_without_loop() {
        let sr = 44100.0;
        let params = TimeEgParams {
            stages: stages_with(&[(20, 255, 0), (20, 0, 0)]),
            stage_count: 2,
            loop_enabled: 0,
            release_point: 1,
            texture: TEXTURE_SAMPLE_HOLD,
         ..Default::default()};
        let mut eg = TimeEg::new();
        eg.note_on();
        let mut level = 0.0;
        for _ in 0..20_000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 0.0).abs() < 1e-3, "ワンショットではrelease_pointの生レベルへ着地するはず: {level}");
    }
}
