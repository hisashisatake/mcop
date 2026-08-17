//! 音色定義（波形バリエーション展開・フィルターデモ・TimeEgネイティブデモ）。
//! `main.rs`のバイナリ本体から分離し、ゴールデンテストから参照できるようにする。
//!
//! 由来: ym38x6/tools/wavetest4x6/src/main.rs（コミット ef3d309 時点の複製、2026-08-13）。
//! デフォーク後のop505ツール群向け複製（fork-on-write）。

use op505_core::{Op505ChannelParams, Op505OperatorParams, Op505Patch};
use op505_core::eg_convert::convert_eg_shape;
use sound_core::{seconds_to_time, TimeEgParams, TimeStage, MAX_STAGES};

/// 試聴に使う波形（番号と表示名）。ビルトイン32波形（4基本波 × 8変換）を全て出力する。
/// 0-7=サイン系(OPZ由来) / 8-15=ノコギリ系 / 16-23=矩形系(PWM) / 24-31=三角系。
pub const WAVE_VARIANTS: [(u8, &str); 32] = [
    // 0-7: サイン系（OPZ由来）
    (0, "sine"),
    (1, "sin2"),
    (2, "halfsine"),
    (3, "halfsin2"),
    (4, "sine2x_h"),
    (5, "sin2_2x_h"),
    (6, "abssine2x_h"),
    (7, "possin2_2x_h"),
    // 8-15: ノコギリ系（saw × OPZ8変換）
    (8, "saw"),
    (9, "saw_sq"),
    (10, "saw_half"),
    (11, "saw_halfsq"),
    (12, "saw_2xh"),
    (13, "saw_sq2xh"),
    (14, "saw_abs2xh"),
    (15, "saw_possq2xh"),
    // 16-23: 矩形系（PWMファミリー）
    (16, "sq50"),
    (17, "sq33"),
    (18, "sq25"),
    (19, "sq16"),
    (20, "sq12"),
    (21, "sq6"),
    (22, "sq_half"),
    (23, "sq_2xh"),
    // 24-31: 三角系（triangle × OPZ8変換）
    (24, "tri"),
    (25, "tri_sq"),
    (26, "tri_half"),
    (27, "tri_halfsq"),
    (28, "tri_2xh"),
    (29, "tri_sq2xh"),
    (30, "tri_abs2xh"),
    (31, "tri_possq2xh"),
];

/// TimeStage配列を`(秒, level, curve)`のリストから組み立てる。
fn stages(entries: &[(f32, u8, u8)]) -> [TimeStage; MAX_STAGES] {
    let mut out = [TimeStage::default(); MAX_STAGES];
    for (i, &(secs, level, curve)) in entries.iter().enumerate() {
        out[i] = TimeStage { time: seconds_to_time(secs), level, curve };
    }
    out
}

/// レート方式5段EG(ar/d1r/d2r/d1l/rr)の音色数値からオペレーターを組み立てるヘルパー。
/// ym38x6版wavetestで手で追い込んだ聴感資産の数値をそのまま渡せるよう、引数順を維持する。
///
/// 注意: [`convert_eg_shape`]の引数順は`(ar, d1r, d1l, d2r, rr, ...)`で、この関数の
/// 引数順(d1r, d2r, d1l, rr)とはd1l/d2rの位置が入れ替わる。
fn op(tl: u8, ar: u8, d1r: u8, d2r: u8, d1l: u8, rr: u8, mul: u8, dt1: u8) -> Op505OperatorParams {
    let mut warnings = Vec::new();
    let eg = convert_eg_shape(ar, d1r, d1l, d2r, rr, 0, 0, 0, &mut warnings, "wavetest");
    Op505OperatorParams { tl, eg, mul, dt1, ..Op505OperatorParams::default() }
}

/// TimeEgを直接指定してオペレーターを組み立てるヘルパー（TimeEgネイティブデモ用。
/// `convert_eg_shape`では表現できない形＝多段リリース・非単調EG等に使う）。
fn native_op(tl: u8, eg: TimeEgParams, mul: u8, dt1: u8) -> Op505OperatorParams {
    Op505OperatorParams { tl, eg, mul, dt1, ..Op505OperatorParams::default() }
}

/// チャンネル設定（アルゴリズム + フィードバック）だけ指定し、残りはデフォルト
/// （フィルター全開・音色LFO無効）。
fn channel(algorithm: u8, feedback: u8) -> Op505ChannelParams {
    Op505ChannelParams { algorithm, feedback, filter_self_oscillation: false, ..Op505ChannelParams::default() }
}

/// 名前付きの基本音色（波形は全オペレーター 0=サイン。バリエーション生成のベース）。
pub struct BaseTimbre {
    pub name: &'static str,
    pub patch: Op505Patch,
}

/// 9系統の基本音色を返す。各オペレーターの並びは O1=index0 〜 O4=index3。
/// アルゴリズムのトポロジーは`sound_fm::algorithm`（OPN準拠）に従う。
/// EG数値はym38x6版wavetestと同一（`convert_eg_shape`経由でTimeEgParamsへ変換するのみ）。
pub fn base_timbres() -> Vec<BaseTimbre> {
    vec![
        // 0) Pure: Alg7（全並列4キャリア）。AR=255・無限サスティン・MUL=1。
        //    FMモジュレーション効果なし・波形そのものの音色を確認するための基準音色。
        BaseTimbre {
            name: "pure",
            patch: Op505Patch {
                operators: [
                    op(240, 255, 0, 0, 255, 100, 1, 128),
                    op(240, 255, 0, 0, 255, 100, 1, 128),
                    op(240, 255, 0, 0, 255, 100, 1, 128),
                    op(240, 255, 0, 0, 255, 100, 1, 128),
                ],
                channel: channel(7, 0),
            },
        },
        // 1) Piano: Alg4 (O1→O2)+(O3→O4)。2つの倍音グループ、片方を微デチューン。
        BaseTimbre {
            name: "piano",
            patch: Op505Patch {
                operators: [
                    op(200, 255, 180, 80, 100, 180, 1, 128), // O1 mod
                    op(255, 255, 60, 30, 200, 120, 1, 128),  // O2 car
                    op(150, 255, 200, 100, 60, 200, 2, 138), // O3 mod
                    op(220, 255, 70, 35, 180, 130, 1, 118),  // O4 car
                ],
                channel: channel(4, 40),
            },
        },
        // 2) E.Piano: Alg4 + 強フィードバック。O1→O2をベル成分(MUL=14)、O3→O4をトーン。
        BaseTimbre {
            name: "epiano",
            patch: Op505Patch {
                operators: [
                    op(230, 255, 220, 150, 40, 200, 14, 128), // O1 ベルmod
                    op(160, 255, 200, 120, 30, 180, 1, 128),  // O2 ベルcar
                    op(180, 255, 100, 50, 150, 150, 1, 128),  // O3 トーンmod
                    op(255, 255, 50, 25, 220, 120, 1, 128),   // O4 トーンcar
                ],
                channel: channel(4, 180),
            },
        },
        // 3) Synth Bass: Alg0 4直列。全MUL=1で太く、中速減衰のパンチ。
        BaseTimbre {
            name: "bass",
            patch: Op505Patch {
                operators: [
                    op(160, 255, 200, 0, 0, 150, 3, 128),    // O1 fb mod
                    op(150, 255, 180, 0, 0, 150, 1, 128),    // O2 mod
                    op(180, 255, 160, 80, 80, 150, 1, 128),  // O3 mod
                    op(255, 255, 120, 60, 120, 150, 1, 128), // O4 car
                ],
                channel: channel(0, 80),
            },
        },
        // 4) Lead: Alg7 全並列。3本デチューンユニゾン + 1オクターブ上で厚み、無限サスティン。
        BaseTimbre {
            name: "lead",
            patch: Op505Patch {
                operators: [
                    op(255, 255, 0, 0, 255, 80, 1, 128), // O1
                    op(220, 255, 0, 0, 255, 80, 1, 138), // O2 +デチューン
                    op(220, 255, 0, 0, 255, 80, 1, 118), // O3 -デチューン
                    op(160, 255, 0, 0, 255, 80, 2, 128), // O4 1oct上
                ],
                channel: channel(7, 0),
            },
        },
        // 5) Brass: Alg4。アタックを弱く（モジュレーターAR=120でゆっくり倍音が立ち上がる
        // 柔らかいスウェル、キャリアAR=150）。MUCOM/OPNブラス寄り。
        BaseTimbre {
            name: "brass",
            patch: Op505Patch {
                operators: [
                    op(180, 120, 80, 40, 180, 120, 1, 128), // O1 mod（ゆっくり立ち上がる倍音）
                    op(255, 150, 60, 30, 200, 120, 1, 128), // O2 car
                    op(170, 120, 80, 40, 170, 120, 2, 133), // O3 mod
                    op(230, 150, 60, 30, 200, 120, 1, 123), // O4 car
                ],
                channel: channel(4, 60),
            },
        },
        // 6) Bell: Alg5 (O1→O2,O3,O4)。インハーモニックMUL(7)のモジュレーターで金属感、長い余韻。
        BaseTimbre {
            name: "bell",
            patch: Op505Patch {
                operators: [
                    op(190, 255, 120, 60, 80, 80, 7, 128), // O1 mod(全キャリアへ)
                    op(255, 255, 90, 40, 40, 60, 1, 128),  // O2 car
                    op(220, 255, 90, 40, 40, 60, 3, 134),  // O3 car
                    op(200, 255, 90, 40, 40, 60, 5, 124),  // O4 car
                ],
                channel: channel(5, 40),
            },
        },
        // 7) Organ: Alg7 全並列の加算合成。MUL=1/2/4/6 のドローバー風、無限サスティン。
        BaseTimbre {
            name: "organ",
            patch: Op505Patch {
                operators: [
                    op(255, 255, 0, 0, 255, 60, 1, 128), // O1 基音
                    op(210, 255, 0, 0, 255, 60, 2, 128), // O2 +1oct
                    op(180, 255, 0, 0, 255, 60, 4, 128), // O3 +2oct
                    op(150, 255, 0, 0, 255, 60, 6, 128), // O4 +5度上(2oct)
                ],
                channel: channel(7, 0),
            },
        },
        // 8) Pluck: Alg0 4直列。d1l=0でサスティンせず減衰しきる撥弦風。モジュレーターは
        // 速く減衰させて打弦直後だけ倍音を立て、キャリアはゆっくり減衰させて余韻を残す。
        BaseTimbre {
            name: "pluck",
            patch: Op505Patch {
                operators: [
                    op(150, 255, 200, 0, 0, 200, 3, 128),  // O1 fb mod（速い倍音減衰）
                    op(140, 255, 200, 0, 0, 200, 1, 128),  // O2 mod
                    op(160, 255, 200, 0, 0, 200, 2, 128),  // O3 mod
                    op(255, 255, 115, 85, 0, 200, 1, 128), // O4 car（中程度の減衰）
                ],
                channel: channel(0, 30),
            },
        },
        // 9) TimeEg Multi-Release: convert_eg_shapeでは表現できない「多段リリース」
        // （キーオフ後、まず素早く中間レベルへ落ち、その後ゆっくり長いテールで消える）。
        // Alg0 4直列で全オペレーターに同じ形のEGを与え、素の減衰カーブの違いを聴きやすくする。
        BaseTimbre {
            name: "timeeg multirelease",
            patch: Op505Patch {
                operators: [
                    native_op(180, multirelease_eg(), 3, 128),
                    native_op(170, multirelease_eg(), 1, 128),
                    native_op(190, multirelease_eg(), 2, 128),
                    native_op(255, multirelease_eg(), 1, 128),
                ],
                channel: channel(0, 40),
            },
        },
        // 10) TimeEg Gain FG Gate: オペレーターEGは通常のオルガン風無限サスティンのまま、
        // チャンネルのGain FGをループ有効にして開閉させ、トレモロ/ゲート効果を作る
        // （convert_eg_shapeが変換するのはオペレーターEGのみで、Gain FGのループ機能は
        // ym38x6のレート方式5段EGには存在しなかったop505固有の機能）。
        BaseTimbre {
            name: "timeeg gainfg gate",
            patch: Op505Patch {
                operators: [
                    op(255, 255, 0, 0, 255, 60, 1, 128),
                    op(210, 255, 0, 0, 255, 60, 2, 128),
                    op(180, 255, 0, 0, 255, 60, 4, 128),
                    op(150, 255, 0, 0, 255, 60, 6, 128),
                ],
                channel: Op505ChannelParams {
                    gain_fg: gate_gain_fg(),
                    filter_self_oscillation: false,
                    ..channel(7, 0)
                },
            },
        },
        // 11) TimeEg Non-Monotonic: いったんピークへ達した後わずかに沈み、再び持ち上げてから
        // サステインする「スウェル→ディップ→スウェル」の非単調EG。レート方式5段EGは
        // ピーク後は単調減衰しかできないため、TimeEgでしか作れない形。Alg7（全並列・
        // モジュレーション連鎖なし）にして、FM由来の位相相互作用でEG形状の聴こえ方が
        // 濁らないようにする（Alg0の3段モジュレーション連鎖で試したところ大幅な音量低下・
        // ほぼ無音化が起きたため、EGの形をそのまま聴かせる構成に変更した）。
        BaseTimbre {
            name: "timeeg nonmonotonic",
            patch: Op505Patch {
                operators: [
                    native_op(255, nonmonotonic_eg(), 1, 128),
                    native_op(220, nonmonotonic_eg(), 1, 138), // +デチューン
                    native_op(220, nonmonotonic_eg(), 1, 118), // -デチューン
                    native_op(160, nonmonotonic_eg(), 2, 128), // 1oct上
                ],
                channel: channel(7, 0),
            },
        },
    ]
}

/// 多段リリース: attack→decay→sustain(loop_endで静止)→[キーオフ]→急速な中間レベルへの
/// 一次リリース→ゆっくり長いテールの二次リリース、の5段。
fn multirelease_eg() -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[
            (0.01, 255, 0), // attack
            (0.25, 190, 0), // decay -> サステインレベル
            (0.05, 90, 0),  // release1: 素早く中間レベルへ
            (2.2, 0, 1),    // release2: ゆっくり長いテール
        ]),
        stage_count: 4,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 1, // stage1(decay到達点)で静止=サステイン、段2〜3がリリース
        ..Default::default()
    }
}

/// Gain FGのループゲート: 開(255)を保持→素早く閉(30)→閉を保持、を周回する。
fn gate_gain_fg() -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[
            (0.0, 255, 0),  // 即座に全開
            (0.35, 255, 0), // 開を保持
            (0.03, 30, 0),  // 素早く閉じる
            (0.35, 30, 0),  // 閉を保持
        ]),
        stage_count: 4,
        loop_enabled: 1,
        loop_start: 0,
        // Gain FGなのでリリース区間が空（release_pointが最終段）でよい＝ゲートは閉じたまま終わる。
        release_point: 3,
     ..Default::default()}
}

/// 非単調EG: attack→わずかなディップ→再上昇→sustain(loop_endで静止)→release。
fn nonmonotonic_eg() -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[
            (0.02, 255, 0), // attack: ピークへ
            (0.15, 140, 0), // dip: 一旦沈む
            (0.25, 220, 0), // swell: 再上昇
            (0.3, 180, 0),  // sustainレベルへ収束(loop_endで静止)
            (1.0, 0, 1),    // release
        ]),
        stage_count: 5,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 3,
     ..Default::default()}
}

/// 音色ごとの試聴に適した基準音（CLIで音程を指定しなかった場合に使う）。
/// ベースは低音域(C2)で鳴らす。それ以外はC4。
pub fn timbre_pitch(name: &str) -> (&'static str, i32) {
    match name {
        "bass" => ("C", 2),
        _ => ("C", 4),
    }
}

/// 展開後の1音色（プログラム番号・名前・パッチ・試聴周波数）。
pub struct Voice {
    pub program: u8,
    pub name: String,
    pub patch: Op505Patch,
    pub freq: f32,
}

/// 基本音色 × 波形バリエーションを [`Voice`] 列に展開する。
/// 同じ音色の波形違いが隣り合うようグループ化する。
/// `override_freq` が `Some` なら全音色をその周波数で、`None` なら音色ごとの基準音で鳴らす。
pub fn expand_voices(bases: &[BaseTimbre], override_freq: Option<f32>) -> Vec<Voice> {
    let mut out = Vec::new();
    let mut program: u8 = 0;
    for base in bases {
        let freq = override_freq.unwrap_or_else(|| {
            let (note, octave) = timbre_pitch(base.name);
            note_to_freq(octave, note).expect("内蔵の基準音は常に有効")
        });
        for (wave, wave_name) in WAVE_VARIANTS {
            let mut patch = base.patch;
            for op in patch.operators.iter_mut() {
                op.waveform = wave;
            }
            out.push(Voice {
                program,
                name: format!("{} {}", base.name, wave_name),
                patch,
                freq,
            });
            program = program.wrapping_add(1);
        }
    }
    out
}

/// フィルター入りデモ（2倍速ハーフサイン）。レゾナンス/カットオフ/フィルターEGで「凶悪」な音を
/// 試聴するための追加音色。指定した基本音色のオペレーター構成を流用し、全オペレーターを
/// 2倍速ハーフサイン(waveform=4)にしてからローパスフィルターを設定する。
pub struct FilterDemo {
    pub name: &'static str,
    /// 流用する基本音色名（[`base_timbres`] の name）。
    pub base: &'static str,
    pub note: &'static str,
    pub octave: i32,
    /// ベースのカットオフ（0〜255）。低いほど暗い。
    pub cutoff: u8,
    /// レゾナンス（0〜255）。`self_osc`=trueなら最大Q≈1000で自己発振。
    pub resonance: u8,
    pub self_osc: bool,
    /// フィルターEG(Cutoff FG)の深さ（カットオフへの加算量、0〜255）。0でEGスイープなし。
    pub eg_depth: u8,
    /// フィルターEG (attack, decay, sustain, release)。レート方式の値をそのままCutoff FGへ変換する。
    pub eg: (u8, u8, u8, u8),
}

pub fn filter_demos() -> Vec<FilterDemo> {
    vec![
        // 静的レゾナンス: カットオフ低め + 強レゾナンスで太く唸るノコギリ。
        FilterDemo {
            name: "leadsaw lp-res",
            base: "lead",
            note: "C",
            octave: 4,
            cutoff: 95,
            resonance: 200,
            self_osc: false,
            eg_depth: 0,
            eg: (255, 0, 255, 80),
        },
        // EGスイープ: 暗い状態から時間をかけてゆっくり開き、開いたまま保持する上昇スイープ。
        FilterDemo {
            name: "leadsaw eg-sweep",
            base: "lead",
            note: "C",
            octave: 4,
            cutoff: 35,
            resonance: 190,
            self_osc: false,
            eg_depth: 200,
            eg: (90, 255, 255, 120),
        },
        // ベース: C2でカットオフ低め、軽いEGスイープでパンチのある攻撃的ベース。
        FilterDemo {
            name: "bass saw lp",
            base: "bass",
            note: "C",
            octave: 2,
            cutoff: 70,
            resonance: 180,
            self_osc: false,
            eg_depth: 120,
            eg: (255, 90, 30, 120),
        },
        // スクリーム: 自己発振ONの高レゾナンスでフィルターが鳴く凶悪リード。
        FilterDemo {
            name: "leadsaw scream",
            base: "lead",
            note: "C",
            octave: 4,
            cutoff: 80,
            resonance: 235,
            self_osc: true,
            eg_depth: 120,
            eg: (255, 140, 50, 120),
        },
    ]
}

/// [`FilterDemo`] を1つの [`Voice`] に展開する。基本音色のオペレーターを流用し、
/// 全オペレーターをノコギリ波にしてLPフィルターを設定する。
pub fn build_filter_demo(
    bases: &[BaseTimbre],
    demo: &FilterDemo,
    program: u8,
    override_freq: Option<f32>,
) -> Voice {
    let base = bases.iter().find(|b| b.name == demo.base).expect("FilterDemo.base は既知の基本音色名");
    let mut patch = base.patch;
    for op in patch.operators.iter_mut() {
        op.waveform = 4; // 2倍速ハーフサイン
    }
    let (a, d, s, r) = demo.eg;
    let mut warnings = Vec::new();
    patch.channel.filter_type = 0; // LP
    patch.channel.filter_cutoff = demo.cutoff;
    patch.channel.filter_resonance = demo.resonance;
    patch.channel.filter_self_oscillation = demo.self_osc;
    // 旧unipolar Filter EG Depth(0〜255) → 新bipolar Cutoff FG Depth(中心128)への変換
    // （常に開く方向として保つ）。
    patch.channel.cutoff_fg.depth = (128.0 + demo.eg_depth as f32 * 128.0 / 255.0).clamp(0.0, 255.0) as u8;
    patch.channel.cutoff_fg.eg = convert_eg_shape(a, d, s, 0, r, 0, 0, 0, &mut warnings, "wavetest-filter");

    let freq =
        override_freq.unwrap_or_else(|| note_to_freq(demo.octave, demo.note).expect("内蔵の基準音は常に有効"));
    Voice { program, name: format!("filt {}", demo.name), patch, freq }
}

pub fn note_to_semitone(note: &str) -> Result<i32, String> {
    match note.to_uppercase().as_str() {
        "C" => Ok(0),
        "D" => Ok(2),
        "E" => Ok(4),
        "F" => Ok(5),
        "G" => Ok(7),
        "A" => Ok(9),
        "B" => Ok(11),
        _ => Err(format!("不正な音階: {note}（C/D/E/F/G/A/B）")),
    }
}

pub fn note_to_freq(octave: i32, note: &str) -> Result<f32, String> {
    let semitone = note_to_semitone(note)?;
    let midi = (octave + 1) * 12 + semitone;
    Ok(440.0 * 2f32.powf((midi - 69) as f32 / 12.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_argument_order_not_swapped() {
        // d1l=255(フルサステイン)・d2r=0の音色でstages[1].level(=decay到達点=サステインレベル)
        // がd1lの値になっていることを確認する（op()引数順とconvert_eg_shape引数順のd1l/d2r
        // 入れ替えを取り違えていないかの検知）。
        let params = op(200, 255, 180, 0, 255, 100, 1, 128);
        assert_eq!(params.eg.stages[1].level, 255, "d1l=255はstages[1].levelに反映されるはず");
    }

    #[test]
    fn base_timbres_are_not_silent_by_construction() {
        for base in base_timbres() {
            let has_audible_carrier = base.patch.operators.iter().any(|op| op.tl > 0);
            assert!(has_audible_carrier, "{}: 全オペレーターtl=0（無音パッチの疑い）", base.name);
            for op in &base.patch.operators {
                assert!(op.eg.stage_count > 0, "{}: eg.stage_count=0（EG未設定の疑い）", base.name);
            }
        }
    }

    #[test]
    fn native_eg_demos_have_expected_stage_shape() {
        let mr = multirelease_eg();
        assert_eq!(mr.stage_count, 4);
        assert_eq!(mr.release_point, 1, "保持は段0〜1、リリースは段2(急速な中間レベル)から始まるはず");

        let gate = gate_gain_fg();
        assert_eq!(gate.loop_enabled, 1, "Gain FGゲートはループ有効であるはず");

        let nm = nonmonotonic_eg();
        // 非単調性: stage1(dip)のlevelがstage0(attack)より低く、stage2(swell)で再び上がる。
        assert!(nm.stages[1].level < nm.stages[0].level, "dipはattackより低いはず");
        assert!(nm.stages[2].level > nm.stages[1].level, "swellはdipより高いはず");
    }

    #[test]
    fn expand_voices_covers_all_wave_variants() {
        let bases = base_timbres();
        let base_count = bases.len();
        let voices = expand_voices(&bases, None);
        assert_eq!(voices.len(), base_count * WAVE_VARIANTS.len());
    }
}
