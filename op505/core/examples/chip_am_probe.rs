//! CHIP LFO退役（memory `project_chip_lfo_retirement_investigation.md`）の聴感プローブ。
//!
//! CHIP LFOのAM経路をGain FGへ厳密変換した結果が、実際にどう聴こえるかをA/Bで確認する
//! ための使い捨てツール。②（オペレーターEGへの畳み込み、dB領域近似）は削除済みで、
//! 現行の変換は③（Gain FGのOP単位配線、線形領域どうしの厳密変換）のみ。
//!
//! 書き出す3グループ:
//! 1. `control_pitch_*` — ①ピッチ経路（CHIP LFO vs Pitch FG）。理論上ほぼ完全一致なので、
//!    **ここが聴き分けられないことを先に確認する**（比較手法そのものの対照実験。
//!    ここで差が聴こえるなら、以降のA/Bで聴こえた差も信用できない）。
//! 2. `am_carrier_*` — AM経路をキャリアに適用（Algorithm 7）。純粋な音量トレモロとして出る。
//! 3. `am_modulator_*` — AM経路をモジュレーターに適用（Algorithm 6）。変調指数の揺れ＝
//!    明るさのうねりとして出る。FMらしい使い方はこちら。
//!
//! ④AM位相オフセット（複数キャリアのAM位相をずらす）は、新設計ではAM源がGain FG 1本に
//! 統一されるため構造的に作れなくなった（AMが掛かる全OPが必ず同位相になる、実機の
//! 共有LFO1本と同じ構造）。クローズ済みの検討のためこのプローブでは扱わない
//! （詳細はmemory参照）。
//!
//! AM深さDは `ams_to_depth(ams) × chip_lfo_amd/255`。`ams=1`で `ams_to_depth≈0.936` なので、
//! `chip_lfo_amd` だけを振ればDをほぼ線形に制御できる（shallow≈0.20 / medium≈0.50 / deep≈0.94）。
//!
//! `ksr=0`固定（`Op505OperatorParams::default()`のまま）。キースケーリングでAMレートが
//! 音域依存に変わる交絡を避けるため。
//!
//! 実行: cargo run -p op505-core --example chip_am_probe -- <出力ディレクトリ>

use std::path::Path;

use op505_core::{
    chip_lfo_am_to_gain_fg, chip_lfo_pitch_to_pitch_fg, Op505ChannelParams, Op505Engine,
    Op505OperatorParams, Op505Patch,
};
use sound_core::{seconds_to_time, TimeEgParams, TimeStage, Vco, MAX_STAGES};

const SAMPLE_RATE: f32 = 44100.0;
const NOTE_FREQ: f32 = 220.0; // A3
const HOLD_SECS: f32 = 4.0;
const RELEASE_TAIL_SECS: f32 = 0.6;

/// 約5.7Hz。トレモロ/ビブラートとして自然に聴こえる速さ。
const LFO_FREQ: u8 = 50;
/// `ams_to_depth(1)≈0.936`。以降はchip_lfo_amdだけでAM深さを振る。
const AMS: u8 = 1;
/// サステインレベル（`apply_chip_lfo_am_to_eg`の適用条件＝保持区間終端が0でないこと）。
const SUSTAIN_LEVEL: u8 = 200;

/// (ラベル, chip_lfo_amd, おおよそのD, 1段近似の理論誤差)
const AM_DEPTHS: &[(&str, u8, &str)] = &[
    ("shallow", 54, "D≈0.20 誤差≈0.05dB"),
    ("medium", 136, "D≈0.50 誤差≈0.6dB"),
    ("deep", 255, "D≈0.94 誤差≈7.5dB"),
];

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| ".".to_string());
    let out_dir = Path::new(&out_dir);
    std::fs::create_dir_all(out_dir).expect("出力ディレクトリ作成失敗");

    write_control_pitch(out_dir);
    write_am_pair(out_dir, "am_carrier", false);
    write_am_pair(out_dir, "am_modulator", true);

    println!();
    println!("[chip_am_probe] 聴き方:");
    println!("  1. control_pitch_A/B が聴き分けられないことをまず確認（対照実験）");
    println!("  2. am_carrier_* / am_modulator_* を深さごとにA/B（B_gain_fgが新経路）");
}

// ---------------------------------------------------------------------------
// 1. 対照実験：①ピッチ経路（CHIP LFO vs Pitch FG）
// ---------------------------------------------------------------------------

/// pms=119は`pms_to_cents_range`で約±50セント（自然なビブラート幅）。
const PITCH_PMS: u8 = 119;

fn write_control_pitch(out_dir: &Path) {
    println!("[chip_am_probe] 1. 対照実験：ピッチ経路（差が聴こえなければ比較手法が健全）");

    // A: CHIP LFOのピッチ変調（移設前の経路）
    let mut a = single_carrier_patch();
    a.channel.chip_lfo_freq = LFO_FREQ;
    a.channel.pms = PITCH_PMS;
    a.channel.chip_lfo_pmd = 255;
    emit(out_dir, "control_pitch_A_chip", a, "CHIP LFOのピッチ変調");

    // B: Pitch FGへ移設したもの（①の変換結果、pms/pmdはクリア）
    let mut b = single_carrier_patch();
    b.channel.chip_lfo_freq = LFO_FREQ;
    b.channel.pitch_fg = chip_lfo_pitch_to_pitch_fg(PITCH_PMS, 255, LFO_FREQ, 0);
    emit(out_dir, "control_pitch_B_fg", b, "Pitch FGへ移設");
}

// ---------------------------------------------------------------------------
// 2/3. AM経路：キャリア（Alg 7）とモジュレーター（Alg 6）
// ---------------------------------------------------------------------------

/// `modulator`がtrueならAlgorithm 6でOP1(モジュレーター)にAMを乗せ、OP2をキャリアにする
/// （変調指数の揺れ＝明るさのうねり）。falseならAlgorithm 7でOP1キャリアに乗せる（音量トレモロ）。
fn write_am_pair(out_dir: &Path, prefix: &str, modulator: bool) {
    let kind = if modulator { "モジュレーター(明るさの揺れ)" } else { "キャリア(音量トレモロ)" };
    println!("[chip_am_probe] {prefix}: {kind}");

    for &(label, amd, note) in AM_DEPTHS {
        // A: CHIP LFOのAM（移設前の経路）
        let mut a = if modulator { fm_stack_patch() } else { single_carrier_patch() };
        a.channel.chip_lfo_freq = LFO_FREQ;
        a.channel.ams = AMS;
        a.channel.chip_lfo_amd = amd;
        a.operators[0].am_enable = true;
        emit(out_dir, &format!("{prefix}_{label}_A_chip"), a, note);

        // B: Gain FGへ厳密変換（現行の変換経路。am_enableは維持、ams/amdはクリア）
        let mut b = if modulator { fm_stack_patch() } else { single_carrier_patch() };
        b.channel.chip_lfo_freq = LFO_FREQ;
        let gain_fg = chip_lfo_am_to_gain_fg(AMS, amd, LFO_FREQ, 0).expect("depth>0のはず");
        b.channel.gain_fg = gain_fg;
        b.channel.gain_fg_to_master = false;
        b.channel.gain_fg_to_operators = true;
        b.operators[0].am_enable = true;
        emit(out_dir, &format!("{prefix}_{label}_B_gain_fg"), b, note);
    }
}

// ---------------------------------------------------------------------------
// パッチ組み立て
// ---------------------------------------------------------------------------

/// アタック→サステイン(200)で張り付く→リリース の3段EG。
/// `release_point=1`の段のレベルが0でないため`apply_chip_lfo_am_to_eg`の適用条件を満たす。
fn sustained_eg() -> TimeEgParams {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    stages[0] = TimeStage { time: seconds_to_time(0.02), level: 255, curve: 0 };
    stages[1] = TimeStage { time: seconds_to_time(0.15), level: SUSTAIN_LEVEL, curve: 0 };
    stages[2] = TimeStage { time: seconds_to_time(0.3), level: 0, curve: 0 };
    TimeEgParams {
        stages,
        stage_count: 3,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 1,
        ..TimeEgParams::default()
    }
}

/// Algorithm 7でOP1のみが鳴るパッチ（OP2〜4はtl=0で無音）。ノコギリ波でトレモロを聴き取りやすくする。
fn single_carrier_patch() -> Op505Patch {
    let mut patch = Op505Patch::default();
    patch.operators[0] = Op505OperatorParams {
        tl: 255,
        mul: 1,
        waveform: 8, // ノコギリ波
        eg: sustained_eg(),
        ..Op505OperatorParams::default()
    };
    patch.channel = Op505ChannelParams { algorithm: 7, filter_self_oscillation: false, ..Default::default() };
    patch
}

/// Algorithm 6（routes=[(0,1)]、carriers=[1,2,3]）でOP1→OP2の2opスタックを作る。
/// OP3/OP4はキャリアだがtl=0で無音。モジュレーターにAMを乗せると明るさが揺れる。
fn fm_stack_patch() -> Op505Patch {
    let mut patch = Op505Patch::default();
    // モジュレーター（AMをここに乗せる）。TLは最大にする——ここを下げると変調指数が
    // 足りずサイドバンドがほぼ出ず、AMを掛けても明るさが動かない（tl=200で検証したところ
    // ゼロ交差率が基音相当のまま変化しなかった）。
    patch.operators[0] = Op505OperatorParams {
        tl: 255,
        mul: 2,
        waveform: 0, // サイン波（古典的なFM）
        eg: sustained_eg(),
        ..Op505OperatorParams::default()
    };
    // キャリア
    patch.operators[1] = Op505OperatorParams {
        tl: 255,
        mul: 1,
        waveform: 0,
        eg: sustained_eg(),
        ..Op505OperatorParams::default()
    };
    patch.channel = Op505ChannelParams { algorithm: 6, filter_self_oscillation: false, ..Default::default() };
    patch
}

// ---------------------------------------------------------------------------
// レンダリング / WAV書き出し
// ---------------------------------------------------------------------------

fn emit(out_dir: &Path, name: &str, patch: Op505Patch, note: &str) {
    let path = out_dir.join(format!("{name}.wav"));
    write_wav(&path, &render(patch));
    println!("  wrote {}  ({note})", path.display());
}

fn render(patch: Op505Patch) -> Vec<f32> {
    let mut engine = Op505Engine::new(SAMPLE_RATE);
    engine.set_patch(patch);
    engine.note_on(0, NOTE_FREQ, 100);
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = vec![0.0f32; hold_samples + release_samples];
    engine.render(&mut out[..hold_samples], 1);
    engine.note_off(0);
    engine.render(&mut out[hold_samples..], 1);
    out
}

fn write_wav(path: &Path, samples: &[f32]) {
    let mut bytes = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("WAV書き込み失敗");
}
