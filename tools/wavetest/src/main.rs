//! wavetest — 非サイン波形（OPZ系 waveform 1〜7）をFMのキャリア/モジュレーターに
//! 使ったときの音色変化を試聴で確認するツール。
//!
//! 8系統の基本音色（ピアノ/E.ピアノ/シンセベース/リード/ブラス/ベル/オルガン/プラック）に
//! 対し、波形をオペレーター全体に適用したバリエーション（サイン/ハーフサイン/絶対値サイン/
//! ノコギリの4種）を作り、`<output_dir>/wav/NN_<音色>_<波形>.wav` と
//! `<output_dir>/b<bank>.38x6` を書き出す。
//!
//! 使い方:
//! ```text
//! wavetest <output_dir> [--bank <N>] [--note <C/D/...>] [--octave <N>] [--on <秒>]
//! ```
//! - 既定: bank=1 / C4 / on=1.2秒 + リリース2.0秒。
//!
//! 波形はモジュレーターにも適用される（FM下での倍音変化が本ツールの主目的のため、
//! キャリアだけでなく全オペレーターに同じ波形を割り当てる）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ym38x6_core::{
    ChannelParams, OperatorParams, PresetEntry, PresetFile, SoundEngine, Ym38x6Engine, Ym38x6Patch,
};

/// 試聴に使う波形（番号と表示名）。OPZ系8波形のうち代表4種。
/// 0=サイン（ベースライン）、1=ハーフサイン、2=絶対値サイン、4=ノコギリ。
const WAVE_VARIANTS: [(u8, &str); 4] = [
    (0, "sine"),
    (1, "halfsine"),
    (2, "abssine"),
    (4, "saw"),
];

/// オペレーター1個を簡潔に組み立てるヘルパー。
/// ksr/velocity_sensitivity/am_enable は 0/false、波形は後段で一括上書きするため 0 固定。
fn op(tl: u8, ar: u8, d1r: u8, d2r: u8, d1l: u8, rr: u8, mul: u8, dt1: u8) -> OperatorParams {
    OperatorParams {
        tl,
        ar,
        d1r,
        d2r,
        d1l,
        rr,
        mul,
        dt1,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: 128,
    }
}

/// チャンネル設定（アルゴリズム + フィードバック）だけ指定し、残りはデフォルト
/// （フィルター全開・音色LFO無効）。
fn channel(algorithm: u8, feedback: u8) -> ChannelParams {
    ChannelParams { algorithm, feedback, ..ChannelParams::default() }
}

/// 名前付きの基本音色（波形は全オペレーター 0=サイン。バリエーション生成のベース）。
struct BaseTimbre {
    name: &'static str,
    patch: Ym38x6Patch,
}

/// 8系統の基本音色を返す。各オペレーターの並びは O1=index0 〜 O4=index3。
/// アルゴリズムのトポロジーは algorithm.rs（OPN準拠）に従う。
fn base_timbres() -> Vec<BaseTimbre> {
    vec![
        // 1) Piano: Alg4 (O1→O2)+(O3→O4)。2つの倍音グループ、片方を微デチューン。
        BaseTimbre {
            name: "piano",
            patch: Ym38x6Patch {
                operators: [
                    op(200, 255, 180, 80, 100, 180, 1, 128), // O1 mod
                    op(255, 255, 60, 30, 200, 120, 1, 128),  // O2 car
                    op(150, 255, 200, 100, 60, 200, 2, 138),  // O3 mod
                    op(220, 255, 70, 35, 180, 130, 1, 118),   // O4 car
                ],
                channel: channel(4, 40),
            },
        },
        // 2) E.Piano: Alg4 + 強フィードバック。O1→O2をベル成分(MUL=14)、O3→O4をトーン。
        BaseTimbre {
            name: "epiano",
            patch: Ym38x6Patch {
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
            patch: Ym38x6Patch {
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
            patch: Ym38x6Patch {
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
            patch: Ym38x6Patch {
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
            patch: Ym38x6Patch {
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
            patch: Ym38x6Patch {
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
            patch: Ym38x6Patch {
                operators: [
                    op(150, 255, 200, 0, 0, 200, 3, 128), // O1 fb mod（速い倍音減衰）
                    op(140, 255, 200, 0, 0, 200, 1, 128), // O2 mod
                    op(160, 255, 200, 0, 0, 200, 2, 128), // O3 mod
                    op(255, 255, 115, 85, 0, 200, 1, 128), // O4 car（中程度の減衰）
                ],
                channel: channel(0, 30),
            },
        },
    ]
}

/// 音色ごとの試聴に適した基準音（CLIで音程を指定しなかった場合に使う）。
/// ベースは低音域(C2)で鳴らす。それ以外はC4。
fn timbre_pitch(name: &str) -> (&'static str, i32) {
    match name {
        "bass" => ("C", 2),
        _ => ("C", 4),
    }
}

/// 展開後の1音色（プログラム番号・名前・パッチ・試聴周波数）。
struct Voice {
    program: u8,
    name: String,
    patch: Ym38x6Patch,
    freq: f32,
}

/// 基本音色 × 波形バリエーションを [`Voice`] 列に展開する。
/// 同じ音色の波形違いが隣り合うようグループ化する。
/// `override_freq` が `Some` なら全音色をその周波数で、`None` なら音色ごとの基準音で鳴らす。
fn expand_voices(bases: &[BaseTimbre], override_freq: Option<f32>) -> Vec<Voice> {
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
            program += 1;
        }
    }
    out
}

/// フィルター入りデモ（ノコギリ波）。レゾナンス/カットオフ/フィルターEGで「凶悪」な音を
/// 試聴するための追加音色。指定した基本音色のオペレーター構成を流用し、全オペレーターを
/// ノコギリ波(waveform=4)にしてからローパスフィルターを設定する。
struct FilterDemo {
    name: &'static str,
    /// 流用する基本音色名（[`base_timbres`] の name）。
    base: &'static str,
    note: &'static str,
    octave: i32,
    /// ベースのカットオフ（0〜255）。低いほど暗い。
    cutoff: u8,
    /// レゾナンス（0〜255）。`self_osc`=trueなら最大Q≈1000で自己発振。
    resonance: u8,
    self_osc: bool,
    /// フィルターEGの深さ（カットオフへの加算量、0〜255）。0でEGスイープなし。
    eg_depth: u8,
    /// フィルターEG (attack, decay, sustain, release)。
    eg: (u8, u8, u8, u8),
}

fn filter_demos() -> Vec<FilterDemo> {
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
        // attackで明るくなっていく動きを聞かせる（decay/sustainを最大にして閉じない）。
        // 一度きりの開閉だと速くて聞き取りにくいため、単方向のゆっくりした掃引にする。
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
fn build_filter_demo(
    bases: &[BaseTimbre],
    demo: &FilterDemo,
    program: u8,
    override_freq: Option<f32>,
) -> Voice {
    let base = bases
        .iter()
        .find(|b| b.name == demo.base)
        .expect("FilterDemo.base は既知の基本音色名");
    let mut patch = base.patch;
    for op in patch.operators.iter_mut() {
        op.waveform = 4; // ノコギリ
    }
    let (a, d, s, r) = demo.eg;
    patch.channel.filter_type = 0; // LP
    patch.channel.filter_cutoff = demo.cutoff;
    patch.channel.filter_resonance = demo.resonance;
    patch.channel.filter_self_oscillation = demo.self_osc;
    patch.channel.filter_eg_depth = demo.eg_depth;
    patch.channel.filter_eg_attack = a;
    patch.channel.filter_eg_decay = d;
    patch.channel.filter_eg_sustain = s;
    patch.channel.filter_eg_release = r;

    let freq = override_freq
        .unwrap_or_else(|| note_to_freq(demo.octave, demo.note).expect("内蔵の基準音は常に有効"));
    Voice { program, name: format!("filt {}", demo.name), patch, freq }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("wavetest: {msg}");
            eprintln!("usage: wavetest <output_dir> [--bank <N>] [--note <C..B>] [--octave <N>] [--on <秒>]");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (output_dir, bank, override_freq, on_secs) = parse_args(args)?;

    let bases = base_timbres();
    let mut voices = expand_voices(&bases, override_freq);
    let grid_count = voices.len();

    // フィルター入りデモ（ノコギリ）をグリッドの後ろに連番で追加する。
    let demos = filter_demos();
    for (i, demo) in demos.iter().enumerate() {
        let program = (grid_count + i) as u8;
        voices.push(build_filter_demo(&bases, demo, program, override_freq));
    }

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {}: {e}", output_dir.display()))?;

    render_wavs(&voices, &output_dir, on_secs)?;

    let presets = voices
        .iter()
        .map(|v| PresetEntry {
            program: v.program,
            name: v.name.clone(),
            patch: v.patch,
        })
        .collect();
    let file = PresetFile::Presets { bank, presets };
    let path = output_dir.join(format!("b{bank}.38x6"));
    let json = file.to_json().map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("書き込みに失敗: {}: {e}", path.display()))?;

    println!("書き出し: {} ({} 音色)", path.display(), voices.len());
    println!(
        "完了: {} 系統 × {} 波形 ({}) + フィルターデモ {} = {} 音色",
        bases.len(),
        WAVE_VARIANTS.len(),
        grid_count,
        demos.len(),
        voices.len()
    );
    Ok(())
}

/// 戻り値の3つ目は試聴周波数の上書き指定（`--note`/`--octave` のどちらかが指定された場合のみ
/// `Some`。未指定なら `None` で、音色ごとの基準音 [`timbre_pitch`] を使う）。
fn parse_args(args: &[String]) -> Result<(PathBuf, u16, Option<f32>, f32), String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut bank: u16 = 1;
    let mut octave: Option<i32> = None;
    let mut note: Option<String> = None;
    let mut on_secs: f32 = 1.2;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
                i += 2;
            }
            "--octave" => {
                let v = args.get(i + 1).ok_or("--octave に値がありません")?;
                octave = Some(v.parse().map_err(|_| format!("--octave の値が不正: {v}"))?);
                i += 2;
            }
            "--note" => {
                let v = args.get(i + 1).ok_or("--note に値がありません")?;
                note_to_semitone(v)?; // バリデーション
                note = Some(v.to_string());
                i += 2;
            }
            "--on" => {
                let v = args.get(i + 1).ok_or("--on に値がありません")?;
                on_secs = v.parse().map_err(|_| format!("--on の値が不正: {v}"))?;
                if on_secs <= 0.0 {
                    return Err(format!("--on は正の値を指定してください: {v}"));
                }
                i += 2;
            }
            _ => {
                positional.push(&args[i]);
                i += 1;
            }
        }
    }
    if positional.len() != 1 {
        return Err("出力ディレクトリの1引数が必要です".to_string());
    }
    // --note / --octave のどちらかが指定されたら全音色をその音程で上書きする。
    let override_freq = match (note, octave) {
        (None, None) => None,
        (n, o) => Some(note_to_freq(o.unwrap_or(4), n.as_deref().unwrap_or("C"))?),
    };
    Ok((PathBuf::from(positional[0]), bank, override_freq, on_secs))
}

fn note_to_semitone(note: &str) -> Result<i32, String> {
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

fn note_to_freq(octave: i32, note: &str) -> Result<f32, String> {
    let semitone = note_to_semitone(note)?;
    let midi = (octave + 1) * 12 + semitone;
    Ok(440.0 * 2f32.powf((midi - 69) as f32 / 12.0))
}

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする。各音色は自身の試聴周波数
/// （[`Voice::freq`]）で鳴らす。`on_secs` キーオン後、2.0秒リリース。ノコギリ全Op等の
/// ホットな音色のクリップを抑えるため、書き出し時に控えめなマスターゲイン(0.7)を掛ける。
fn render_wavs(voices: &[Voice], output_dir: &Path, on_secs: f32) -> Result<(), String> {
    const SR: f32 = 44_100.0;
    const MASTER_GAIN: f32 = 0.7;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir).map_err(|e| format!("wavディレクトリ作成に失敗: {e}"))?;

    for (idx, v) in voices.iter().enumerate() {
        let mut engine = Ym38x6Engine::new(SR);
        engine.note_on_with_velocity(0, v.freq, 110, v.patch);

        let on = (SR * on_secs) as usize;
        let off = (SR * 2.0) as usize;
        let mut samples = vec![0.0f32; on];
        engine.render(&mut samples, 1);
        engine.note_off(0);
        let mut tail = vec![0.0f32; off];
        engine.render(&mut tail, 1);
        samples.extend_from_slice(&tail);

        let file_name = format!("{idx:02}_{}.wav", v.name.replace(' ', "_"));
        let path = wav_dir.join(&file_name);
        write_wav_mono16(&path, &samples, SR as u32, MASTER_GAIN)
            .map_err(|e| format!("WAV書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV書き出し: {} に {} 音色", wav_dir.display(), voices.len());
    Ok(())
}

/// mono / 16bit PCM の最小WAVライター。`gain` を掛けてから [-1,1] にクランプする。
fn write_wav_mono16(path: &Path, samples: &[f32], sr: u32, gain: f32) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = ((s * gain).clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out)
}
