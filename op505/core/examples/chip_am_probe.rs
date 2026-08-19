//! CHIP LFO退役（memory `project_chip_lfo_retirement_investigation.md`）の聴感プローブ。
//!
//! CHIP LFOの変調経路をFG/オペレーターEGへ移設した結果が、実際にどう聴こえるかを
//! A/Bで確認するための使い捨てツール。数値上の誤差（dB領域とリニア領域の曲線差など）が
//! 聴感でどう出るか、あるいは「アナログ的なニュアンス」として歓迎できるかを耳で判定する。
//!
//! 書き出す4グループ:
//! 1. `control_pitch_*` — ①ピッチ経路（CHIP LFO vs Pitch FG）。理論上ほぼ完全一致なので、
//!    **ここが聴き分けられないことを先に確認する**（比較手法そのものの対照実験。
//!    ここで差が聴こえるなら、以降のA/Bで聴こえた差も信用できない）。
//! 2. `am_carrier_*` — ②AM経路をキャリアに適用（Algorithm 7）。純粋な音量トレモロとして出る。
//! 3. `am_modulator_*` — ②AM経路をモジュレーターに適用（Algorithm 6）。変調指数の揺れ＝
//!    明るさのうねりとして出る。FMらしい使い方はこちら。
//! 4. `phase_*` — ④AM位相オフセットの先取り。4キャリア（MUL 1/2/3/4）のAM位相を
//!    揃えた場合と90°ずつ回した場合を比較する。CHIP LFOはLFO 1本を全OPで共有するため
//!    構造的に「揃った」状態しか作れず、回した側は退役後にしか存在しえない音。
//!
//! AM深さDは `ams_to_depth(ams) × chip_lfo_amd/255`。`ams=1`で `ams_to_depth≈0.936` なので、
//! `chip_lfo_amd` だけを振ればDをほぼ線形に制御できる（shallow≈0.20 / medium≈0.50 / deep≈0.94）。
//! 1段近似の理論誤差はそれぞれ約0.05dB / 0.6dB / 7.5dB。
//!
//! `ksr=0`固定（`Op505OperatorParams::default()`のまま）。キースケーリングでAMレートが
//! 音域依存に変わる交絡を避けるため。
//!
//! 実行: cargo run -p op505-core --example chip_am_probe -- <出力ディレクトリ>

use std::path::Path;

use op505_core::eg_convert::apply_chip_lfo_am_to_eg;
use op505_core::{
    chip_lfo_pitch_to_pitch_fg, Op505ChannelParams, Op505Engine, Op505OperatorParams, Op505Patch,
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
    write_phase_demo(out_dir);

    println!();
    println!("[chip_am_probe] 聴き方:");
    println!("  1. control_pitch_A/B が聴き分けられないことをまず確認（対照実験）");
    println!("  2. am_carrier_* / am_modulator_* を深さごとにA/B");
    println!("  3. phase_aligned と phase_rotated を比較（④の判断材料）");
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

        // B: オペレーターEGへ畳み込み（②の変換結果。am_enable/ams/amdはクリア）
        let mut b = if modulator { fm_stack_patch() } else { single_carrier_patch() };
        b.channel.chip_lfo_freq = LFO_FREQ;
        let folded = apply_chip_lfo_am_to_eg(&b.operators[0].eg, AMS, amd, LFO_FREQ, 0, 0)
            .expect("サステインが可聴なEGなので畳み込めるはず");
        b.operators[0].eg = folded;
        emit(out_dir, &format!("{prefix}_{label}_B_folded"), b, note);
    }
}

// ---------------------------------------------------------------------------
// 4. ④AM位相オフセットの先取り
// ---------------------------------------------------------------------------

fn write_phase_demo(out_dir: &Path) {
    println!("[chip_am_probe] 4. AM位相オフセット（④の判断材料）");
    let period = 1.0 / sound_fm_chip_lfo_hz(LFO_FREQ);

    // 揃えた場合＝CHIP LFO時代と同じ挙動（LFO 1本を全OPで共有）。
    emit(out_dir, "phase_aligned", phase_patch(&[0.0, 0.0, 0.0, 0.0], period), "4キャリアのAM位相を揃える");
    // 90°ずつ回した場合＝退役後にしか作れない音（倍音が順番に膨らむ）。
    emit(out_dir, "phase_rotated", phase_patch(&[0.0, 0.25, 0.5, 0.75], period), "90°ずつ回す");
}

/// 4キャリア（MUL 1/2/3/4＝4本の倍音）にAMを畳み込み、指定の位相オフセットを与える。
fn phase_patch(offsets: &[f32; 4], period_seconds: f32) -> Op505Patch {
    let mut patch = Op505Patch::default();
    patch.channel = Op505ChannelParams { algorithm: 7, filter_self_oscillation: false, ..Default::default() };
    patch.channel.chip_lfo_freq = LFO_FREQ;

    for (i, &offset) in offsets.iter().enumerate() {
        let base = sustained_eg();
        let mut eg = apply_chip_lfo_am_to_eg(&base, AMS, 200, LFO_FREQ, 0, 0).expect("畳み込めるはず");
        offset_am_phase(&mut eg, offset, period_seconds);
        patch.operators[i] = Op505OperatorParams {
            // 4本を均等に混ぜると飽和するのでTLを下げる。230は他グループとおおよそ同じ
            // 再生レベルになる値（tl=200では約12dB小さく、A/B時に音量差が邪魔になった）。
            tl: 230,
            mul: (i + 1) as u8,
            waveform: 0,
            eg,
            ..Op505OperatorParams::default()
        };
    }
    patch
}

/// AMループへ入る直前に平坦段を1つ挿し込み、ループ突入を`fraction`周期ぶん遅らせる
/// （＝相対的な位相オフセット）。ループ内の段長を変えると周期そのものが変わってしまうため、
/// ループ外に専用の待機段を置く必要がある。
fn offset_am_phase(eg: &mut TimeEgParams, fraction: f32, period_seconds: f32) {
    if fraction <= 0.0 {
        return;
    }
    let count = eg.stage_count as usize;
    let loop_start = eg.loop_start as usize;
    if count + 1 > MAX_STAGES || loop_start == 0 {
        return;
    }
    for i in (loop_start..count).rev() {
        eg.stages[i + 1] = eg.stages[i];
    }
    // 直前段の到達レベル（＝サステインレベル）を保ったまま待機する。
    let hold_level = eg.stages[loop_start - 1].level;
    eg.stages[loop_start] =
        TimeStage { time: seconds_to_time(period_seconds * fraction), level: hold_level, curve: 0 };
    eg.stage_count = (count + 1) as u8;
    eg.loop_start += 1;
    eg.release_point += 1;
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

/// `sound_fm::chip_lfo::chip_lfo_freq_to_hz`と同じ式（op505-coreからは再エクスポートされて
/// いないためプローブ内で再計算する）。
fn sound_fm_chip_lfo_hz(freq: u8) -> f32 {
    const F_MIN: f32 = 3.0;
    const F_MAX: f32 = 80.0;
    F_MIN * (F_MAX / F_MIN).powf(freq as f32 / 255.0)
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
