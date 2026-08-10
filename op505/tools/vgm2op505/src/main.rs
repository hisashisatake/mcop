//! vgm2op505 — YM2151(OPM) / OPN系(YM2612/YM2203/YM2608) の VGM/VGZ から
//! 音色（.op505）と SMF（.mid）または WAV を **直接** 抽出する（中間の`.38x6`ファイルを
//! 経由しない）。演奏ロジック（VGM逐次デコード・ピッチベンド・SSGコアレス処理等）は
//! vgm2x6のlibクレートを再利用し、音色変換だけをOP505直接変換版（opm2op505/mucom2op505の
//! `voice_to_op505_patch`）に差し替える。SSG合成パッチ（psg/noise/mix_patch）はx6ネイティブ
//! 定義の人工パッチなので、`op505_core::adapter::convert_patch`（.38x6→.op505の2段変換）で
//! 変換する。
//!
//! 使い方:
//! ```text
//! vgm2op505 <input.vgz|.vgm> [--out <dir>] [--out-bank <file>] [--out-midi <file>] [--bank N]
//!           [--out-wav <file>] [--wav] [--gain <factor>] [--fm-gain <factor>] [--ssg-gain <factor>]
//!           [--attack <bias|none|curve>] [--only-ch <n>]
//! ```
//! vgm2x6と異なり `--dump-pitch`（ピッチロジック共通で新情報が無い）と
//! `--fb-max`/`--fb-2sample`/`--fb-1sample`（op505-coreはフィードバック実験フラグを
//! 搭載しない設計）は持たない。`--attack`は他の直接変換ツール群と同じ
//! （既定`none`。詳細は`opm2op505::conv::AttackMode`）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use op505_core::{Op505Patch, Op505PresetEntry, Op505PresetFile};
use vgm2op505::convert::{parse_attack_mode, AttackMode, Engine505, Op505Converter};
use vgm2x6::patch::{PatchBank, PatchConverter};
use vgm2x6::play::{self, OpnInfo, SmfSink, SourceChip, WavEngine, WavSink};
use vgm2x6::vgm;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vgm2op505: {msg}");
            eprintln!(
                "usage: vgm2op505 <input.vgz|.vgm> [--out <dir>] \
                 [--out-bank <file>] [--out-midi <file>] [--bank N] [--out-wav <file>] [--wav] \
                 [--gain <factor>] [--fm-gain <factor>] [--ssg-gain <factor>] \
                 [--attack <bias|none|curve>] [--only-ch <n>]"
            );
            ExitCode::FAILURE
        }
    }
}

// ===========================================================================
// CLI引数
// ===========================================================================

struct Args {
    input: PathBuf,
    out_bank: PathBuf,
    out_midi: PathBuf,
    bank: u16,
    wav: Option<PathBuf>,
    gain: f32,
    fm_gain: f32,
    ssg_gain: f32,
    attack: AttackMode,
    only_ch: Option<usize>,
}

/// 音量ゲイン係数(線形)を基準velocity 100 に適用する。vgm2x6::scaled_velocityと同一式。
fn scaled_velocity(gain: f32) -> u8 {
    (100.0 * gain).round().clamp(1.0, 127.0) as u8
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut out_bank: Option<PathBuf> = None;
    let mut out_midi: Option<PathBuf> = None;
    let mut bank: u16 = 0;
    let mut fm_gain: f32 = 1.0;
    let mut ssg_gain: f32 = 1.0;
    let mut out_wav: Option<PathBuf> = None;
    let mut wav_flag = false;
    let mut gain: f32 = 1.0;
    let mut attack = AttackMode::None;
    let mut only_ch: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--only-ch" => {
                let v = args.get(i + 1).ok_or("--only-ch に値がありません")?;
                only_ch = Some(v.parse().map_err(|_| format!("--only-ch の値が不正: {v}"))?);
                i += 2;
            }
            "--out-wav" => {
                out_wav = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out-wav に値がありません")?,
                ));
                i += 2;
            }
            "--wav" => {
                wav_flag = true;
                i += 1;
            }
            "--gain" => {
                let v = args.get(i + 1).ok_or("--gain に値がありません")?;
                gain = v.parse().map_err(|_| format!("--gain の値が不正: {v}"))?;
                i += 2;
            }
            "--fm-gain" => {
                let v = args.get(i + 1).ok_or("--fm-gain に値がありません")?;
                fm_gain = v.parse().map_err(|_| format!("--fm-gain の値が不正: {v}"))?;
                i += 2;
            }
            "--ssg-gain" => {
                let v = args.get(i + 1).ok_or("--ssg-gain に値がありません")?;
                ssg_gain = v.parse().map_err(|_| format!("--ssg-gain の値が不正: {v}"))?;
                i += 2;
            }
            "--attack" => {
                let v = args.get(i + 1).ok_or("--attack に値がありません")?;
                attack = parse_attack_mode(v)?;
                i += 2;
            }
            "--out" => {
                out_dir = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out に値がありません")?,
                ));
                i += 2;
            }
            "--out-bank" => {
                out_bank = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out-bank に値がありません")?,
                ));
                i += 2;
            }
            "--out-midi" => {
                out_midi = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out-midi に値がありません")?,
                ));
                i += 2;
            }
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
                i += 2;
            }
            s => {
                input = Some(PathBuf::from(s));
                i += 1;
            }
        }
    }
    let input = input.ok_or("入力ファイルのパスが必要です")?;
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    let base_dir = out_dir.as_deref().unwrap_or(Path::new("."));
    let out_bank = out_bank.unwrap_or_else(|| base_dir.join(format!("{stem}.op505")));
    let out_midi = out_midi.unwrap_or_else(|| base_dir.join(format!("{stem}.mid")));
    let wav = out_wav.or_else(|| wav_flag.then(|| base_dir.join(format!("{stem}.wav"))));

    Ok(Args { input, out_bank, out_midi, bank, wav, gain, fm_gain, ssg_gain, attack, only_ch })
}

// ===========================================================================
// バンク出力・警告レポート（PatchBank<X6Converter>専用のwrite()相当をOp505向けに実装）
// ===========================================================================

fn write_bank<C>(bank: &PatchBank<C>, path: &Path, bank_no: u16) -> Result<(), String>
where
    C: PatchConverter<Patch = Op505Patch>,
{
    if bank.len() == 0 {
        eprintln!("警告: 音色が抽出されませんでした");
        return Ok(());
    }
    let presets: Vec<Op505PresetEntry> = bank
        .entries()
        .map(|(program, name, patch)| Op505PresetEntry {
            program,
            name: name.to_string(),
            patch: *patch,
        })
        .collect();
    let file = Op505PresetFile::Presets { bank: bank_no, presets };
    let json = file.to_json().map_err(|e| format!(".op505 シリアライズに失敗: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

fn report_warnings<C: PatchConverter>(bank: &PatchBank<C>) {
    let warnings = bank.warnings();
    if warnings.is_empty() {
        return;
    }
    println!("変換警告 ({} 音色):", warnings.len());
    for (name, ws) in warnings {
        println!("  [{name}]");
        for w in ws {
            println!("    - {w}");
        }
    }
}

// ===========================================================================
// メイン処理
// ===========================================================================

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    let data = vgm::load(&args.input)?;
    let header = vgm::parse_header(&data)?;

    let chip = play::detect_chip(&header).ok_or(
        "YM2151 も OPN系(YM2612/YM2203/YM2608) も見つかりません（対応外のVGMです）",
    )?;

    match chip {
        SourceChip::Opm => {
            eprintln!(
                "YM2151 clock: {} Hz, {} samples",
                header.ym2151_clock, header.total_samples
            );
            if let Some(wav_path) = &args.wav {
                return render_wav(&data, header.data_start, wav_path, args.gain, args.attack);
            }
            run_opm_smf(&data, &header, &args)
        }
        SourceChip::Opn(info) => {
            eprintln!(
                "{} clock: {} Hz, {} samples (FM {}ch{})",
                info.label, info.clock, header.total_samples, info.fm_channels,
                if info.has_ssg { " + SSG 3ch" } else { "" },
            );
            run_opn(&data, header.data_start, info, &args)
        }
    }
}

/// YM2151(OPM) の直接WAVレンダリング（--wav）。
fn render_wav(data: &[u8], data_start: usize, out: &Path, gain: f32, attack: AttackMode) -> Result<(), String> {
    const SR: f32 = 44_100.0;
    let mut engine = Engine505::new(SR);
    let audio = play::opm_to_audio(data, data_start, Op505Converter { attack }, &mut engine);
    play::finish_wav(&mut engine, audio, SR, out, gain)
}

/// YM2151(OPM) の SMF + .op505バンクを出力する。
fn run_opm_smf(data: &[u8], header: &vgm::VgmHeader, args: &Args) -> Result<(), String> {
    let mut bank = PatchBank::new(Op505Converter { attack: args.attack });
    let mut smf = play::opm_to_smf(data, header.data_start, &mut bank);

    write_bank(&bank, &args.out_bank, args.bank)?;
    println!("音色バンク: {} ({} 音色)", args.out_bank.display(), bank.len());
    report_warnings(&bank);

    smf.write(&args.out_midi)
        .map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
    println!("MIDI: {}", args.out_midi.display());

    Ok(())
}

/// OPN系の出力（SMF+.op505バンク または WAV）。
fn run_opn(data: &[u8], data_start: usize, info: OpnInfo, args: &Args) -> Result<(), String> {
    let total_ch = info.fm_channels + if info.has_ssg { 3 } else { 0 };
    let mut bank = PatchBank::new(Op505Converter { attack: args.attack });

    let fm_vel = scaled_velocity(args.fm_gain);
    let ssg_vel = scaled_velocity(args.ssg_gain);

    if let Some(c) = args.only_ch {
        eprintln!("実験: エンジンチャンネル {c} のみ発音");
    }

    if let Some(wav_path) = &args.wav {
        let mut sink = WavSink::<Engine505>::new(44_100.0, total_ch);
        play::process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);
        return sink.finish_and_write(wav_path, args.gain);
    }

    let mut sink = SmfSink::new(total_ch);
    play::process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);

    write_bank(&bank, &args.out_bank, args.bank)?;
    println!("音色バンク: {} ({} 音色)", args.out_bank.display(), bank.len());
    report_warnings(&bank);
    sink.smf
        .write(&args.out_midi)
        .map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
    println!("MIDI: {}", args.out_midi.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mucom2x6::conv::{OpnOperator, OpnVoice};
    use opm2x6::parse::{OpmOpReg, OpmVoice, OperatorOrder};
    use vgm2x6::patch::X6Converter;
    use vgm2x6::play::OpnWriteKind;

    // -----------------------------------------------------------------------
    // (b) vgm2x6(X6Converter)とvgm2op505(Op505Converter)のSMFバイト一致
    //
    // 同一VGMコマンド列を両変換器で走査し、SmfBuilder::to_bytes()（プログラム番号・
    // ノートイベント等）が完全一致することを検証する。SMFにはパッチの中身が
    // 現れないため、これは「非OPMエントリーの重複判定キーをYm38x6Patchへ統一した」
    // 設計（PatchBank::find_or_insert_opn/find_or_insert_fixed）がバンク番号の
    // ずれを起こしていないことの回帰テストになる。
    // -----------------------------------------------------------------------

    fn push_opm(data: &mut Vec<u8>, reg: u8, val: u8) {
        data.push(0x54);
        data.push(reg);
        data.push(val);
    }
    fn push_opn2203(data: &mut Vec<u8>, reg: u8, val: u8) {
        data.push(0x55);
        data.push(reg);
        data.push(val);
    }
    fn push_wait(data: &mut Vec<u8>, n: u16) {
        data.push(0x61);
        data.extend_from_slice(&n.to_le_bytes());
    }

    /// ch0のみを使うOPM VGMコマンド列: 音色レジスタ設定→KC/KF→キーオン→wait→キーオフ→End。
    fn opm_test_stream() -> Vec<u8> {
        let mut d = Vec::new();
        push_opm(&mut d, 0x20, 0xC7); // RL/FB/CON ch0
        push_opm(&mut d, 0x38, 0x00); // PMS/AMS ch0
        for op in 0..4u8 {
            let i = op * 8; // ch0（i = op_reg_idx*8 + ch）
            push_opm(&mut d, 0x40 + i, 0x11);
            push_opm(&mut d, 0x60 + i, 0x18);
            push_opm(&mut d, 0x80 + i, 0x1F);
            push_opm(&mut d, 0xA0 + i, 0x0A);
            push_opm(&mut d, 0xC0 + i, 0x08);
            push_opm(&mut d, 0xE0 + i, 0x8F);
        }
        push_opm(&mut d, 0x28, 0x4A); // KC ch0
        push_opm(&mut d, 0x30, 0x00); // KF ch0
        push_opm(&mut d, 0x08, 0x78); // keyon ch0 全slot
        push_wait(&mut d, 4410);
        push_opm(&mut d, 0x08, 0x00); // keyoff ch0
        push_wait(&mut d, 100);
        d.push(0x66);
        d
    }

    #[test]
    fn opm_smf_matches_vgm2x6_x6_converter() {
        let data = opm_test_stream();

        let mut x6_bank = PatchBank::new(X6Converter);
        let mut x6_smf = play::opm_to_smf(&data, 0, &mut x6_bank);

        let mut op505_bank = PatchBank::new(Op505Converter { attack: AttackMode::None });
        let mut op505_smf = play::opm_to_smf(&data, 0, &mut op505_bank);

        assert_eq!(x6_smf.to_bytes(), op505_smf.to_bytes());

        let x6_names: Vec<(u8, String)> =
            x6_bank.entries().map(|(p, n, _)| (p, n.to_string())).collect();
        let op505_names: Vec<(u8, String)> =
            op505_bank.entries().map(|(p, n, _)| (p, n.to_string())).collect();
        assert_eq!(x6_names, op505_names);
    }

    /// YM2203(OPN)のch0 FM音声 + SSG(A)トーンを含むコマンド列。
    fn opn_test_stream() -> Vec<u8> {
        let mut d = Vec::new();
        for slot in 0..4u8 {
            let i = slot * 4; // ch0（i = slot*4 + cip、cip=0）
            push_opn2203(&mut d, 0x30 + i, 0x11);
            push_opn2203(&mut d, 0x40 + i, 0x18);
            push_opn2203(&mut d, 0x50 + i, 0x1F);
            push_opn2203(&mut d, 0x60 + i, 0x0A);
            push_opn2203(&mut d, 0x70 + i, 0x08);
            push_opn2203(&mut d, 0x80 + i, 0x8F);
        }
        push_opn2203(&mut d, 0xB0, 0x34); // fb/alg ch0
        push_opn2203(&mut d, 0xA4, 0x22); // block/fnum hi ch0
        push_opn2203(&mut d, 0xA0, 0x00); // fnum lo ch0（確定）
        push_opn2203(&mut d, 0x28, 0xF0); // keyon ch0 全op

        // SSG(A)トーン
        push_opn2203(&mut d, 0x00, 0x34);
        push_opn2203(&mut d, 0x01, 0x02);
        push_opn2203(&mut d, 0x07, 0b111_110); // トーンch0のみ有効
        push_opn2203(&mut d, 0x08, 0x0F);

        push_wait(&mut d, 4410);
        d.push(0x66);
        d
    }

    #[test]
    fn opn_smf_matches_vgm2x6_x6_converter() {
        let data = opn_test_stream();
        let info = OpnInfo {
            label: "YM2203(OPN)",
            write_kind: OpnWriteKind::Ym2203,
            clock: 4_000_000,
            fm_channels: 3,
            fm_divisor: vgm2x6::opn::FM_DIVISOR_3CH,
            ssg_clock_div: 2,
            has_ssg: true,
        };
        let total_ch = info.fm_channels + 3;

        let mut x6_bank = PatchBank::new(X6Converter);
        let mut x6_sink = SmfSink::new(total_ch);
        play::process_opn(&data, 0, info, &mut x6_bank, &mut x6_sink, 100, 100, None);

        let mut op505_bank = PatchBank::new(Op505Converter { attack: AttackMode::None });
        let mut op505_sink = SmfSink::new(total_ch);
        play::process_opn(&data, 0, info, &mut op505_bank, &mut op505_sink, 100, 100, None);

        assert_eq!(x6_sink.smf.to_bytes(), op505_sink.smf.to_bytes());

        let x6_names: Vec<(u8, String)> =
            x6_bank.entries().map(|(p, n, _)| (p, n.to_string())).collect();
        let op505_names: Vec<(u8, String)> =
            op505_bank.entries().map(|(p, n, _)| (p, n.to_string())).collect();
        assert_eq!(x6_names, op505_names);
    }

    // -----------------------------------------------------------------------
    // (c) --attack bias 時、直接変換 == 「旧2段変換(X6Converter → adapter::convert_patch)」
    //
    // FG(pitch_fg/cutoff_fg/gain_fg)は直接変換ツールがOP505ネイティブ既定値を使う仕様
    // （opm2op505/mucom2op505の既存テストが保証済み）で、convert_patchのFG変換とは
    // ビット表現が異なるため意図的に比較対象から除外する。
    // -----------------------------------------------------------------------

    fn make_opm_op(ar: u8) -> OpmOpReg {
        OpmOpReg { ar, d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 20, mul: 1, dt1: 0, dt2: 0, ks: 1, ams_en: false }
    }

    fn make_opm_voice() -> OpmVoice {
        OpmVoice {
            number: 0,
            name: "test".to_string(),
            lfrq: 0,
            pmd: 0,
            amd: 0,
            lfo_wf: 2,
            fl: 3,
            con: 4,
            pms: 0,
            ams: 0,
            slot: 120,
            m1: make_opm_op(20),
            c1: make_opm_op(18),
            m2: make_opm_op(22),
            c2: make_opm_op(24),
        }
    }

    fn assert_non_fg_channel_eq(a: &Op505Patch, b: &Op505Patch) {
        assert_eq!(a.channel.algorithm, b.channel.algorithm);
        assert_eq!(a.channel.feedback, b.channel.feedback);
        assert_eq!(a.channel.chip_lfo_freq, b.channel.chip_lfo_freq);
        assert_eq!(a.channel.chip_lfo_pmd, b.channel.chip_lfo_pmd);
        assert_eq!(a.channel.chip_lfo_amd, b.channel.chip_lfo_amd);
        assert_eq!(a.channel.chip_lfo_delay, b.channel.chip_lfo_delay);
        assert_eq!(a.channel.pms, b.channel.pms);
        assert_eq!(a.channel.ams, b.channel.ams);
        assert_eq!(a.channel.filter_cutoff, b.channel.filter_cutoff);
        assert_eq!(a.channel.filter_resonance, b.channel.filter_resonance);
        assert_eq!(a.channel.filter_type, b.channel.filter_type);
        assert_eq!(a.channel.filter_self_oscillation, b.channel.filter_self_oscillation);
        assert_eq!(a.channel.texture_lfo, b.channel.texture_lfo);
    }

    #[test]
    fn bias_attack_opm_matches_two_stage_convert_patch() {
        let voice = make_opm_voice();

        let mut x6 = X6Converter;
        let (x6_patch, _) = x6.from_opm(&voice);
        let (two_stage, _) = op505_core::adapter::convert_patch(&x6_patch);

        let mut op505 = Op505Converter { attack: AttackMode::Bias };
        let (direct, warnings) = op505.from_opm(&voice);
        assert!(warnings.is_empty());

        for i in 0..4 {
            assert_eq!(direct.operators[i], two_stage.operators[i], "op{i} mismatch");
        }
        assert_non_fg_channel_eq(&direct, &two_stage);

        // OperatorOrderは同じ値を使っている（Op505Converter::from_opmの既定=Direct）ことの確認。
        let _ = OperatorOrder::Direct;
    }

    fn make_opn_op(ar: u8) -> OpnOperator {
        OpnOperator { ar, d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 20, mul: 1, dt1: 0, ks: 1, am_enable: false }
    }

    fn make_opn_voice() -> OpnVoice {
        let mut v = OpnVoice { algorithm: 4, feedback: 3, ..OpnVoice::default() };
        v.operators = [make_opn_op(20), make_opn_op(18), make_opn_op(22), make_opn_op(24)];
        v
    }

    #[test]
    fn bias_attack_opn_matches_two_stage_convert_patch() {
        let voice = make_opn_voice();

        let mut x6 = X6Converter;
        let (x6_patch, _) = x6.from_opn(&voice);
        let (two_stage, _) = op505_core::adapter::convert_patch(&x6_patch);

        let mut op505 = Op505Converter { attack: AttackMode::Bias };
        let (direct, warnings) = op505.from_opn(&voice);
        assert!(warnings.is_empty());

        for i in 0..4 {
            assert_eq!(direct.operators[i], two_stage.operators[i], "op{i} mismatch");
        }
        assert_non_fg_channel_eq(&direct, &two_stage);
    }
}
