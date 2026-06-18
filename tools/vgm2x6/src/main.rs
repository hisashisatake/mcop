//! vgm2x6 — YM2151 VGM/VGZ から音色（.38x6）と SMF（.mid）を抽出する。
//!
//! 使い方:
//! ```text
//! vgm2x6 <input.vgz|.vgm> [--out <dir>] [--out-bank <file>] [--out-midi <file>] [--bank N]
//! ```
//! - `--out <dir>`: 出力ディレクトリを指定（ファイル名は入力ステム名を使用）
//! - `--out-bank` / `--out-midi`: 個別ファイルパスを明示指定（--out より優先）
//! - 何も指定しない場合: カレントディレクトリに <stem>.38x6 / <stem>.mid を出力

mod opm;
mod patch;
mod smf;
mod vgm;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use opm::{
    compute_pitch_bend, kc_kf_to_ref_midi, kc_to_midi_note, midi_note_name, pb_to_semitones,
    OpmState, PB_SENSITIVITY,
};
use patch::PatchBank;
use smf::SmfBuilder;
use vgm::{VgmCmd, VgmIter};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vgm2x6: {msg}");
            eprintln!(
                "usage: vgm2x6 <input.vgz|.vgm> [--out <dir>] \
                 [--out-bank <file>] [--out-midi <file>] [--bank N] [--dump-pitch] [--wav <file>] [--gain <factor>]"
            );
            ExitCode::FAILURE
        }
    }
}

struct Args {
    input: PathBuf,
    out_bank: PathBuf,
    out_midi: PathBuf,
    bank: u16,
    dump_pitch: bool,
    wav: Option<PathBuf>,
    gain: f32,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut out_bank: Option<PathBuf> = None;
    let mut out_midi: Option<PathBuf> = None;
    let mut bank: u16 = 1;
    let mut dump_pitch = false;
    let mut wav: Option<PathBuf> = None;
    let mut gain: f32 = 1.0;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump-pitch" => {
                dump_pitch = true;
                i += 1;
            }
            "--wav" => {
                wav = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--wav に値がありません")?,
                ));
                i += 2;
            }
            "--gain" => {
                let v = args.get(i + 1).ok_or("--gain に値がありません")?;
                gain = v.parse().map_err(|_| format!("--gain の値が不正: {v}"))?;
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
    // --out-bank / --out-midi が明示されていない場合は --out（またはカレント）を使う
    let base_dir = out_dir.as_deref().unwrap_or(Path::new("."));
    let out_bank = out_bank.unwrap_or_else(|| base_dir.join(format!("{stem}.38x6")));
    let out_midi = out_midi.unwrap_or_else(|| base_dir.join(format!("{stem}.mid")));

    Ok(Args { input, out_bank, out_midi, bank, dump_pitch, wav, gain })
}

/// ピッチ二重変換ダンプ（--dump-pitch）。
///
/// VGMを走査し、音程イベント（ノートオン・KC変更・KF変更）ごとに、
/// 現パイプライン（kc_to_midi_note + ピッチベンド）が実際にエンジンへ送る再現ピッチと、
/// 実機YM2151 canonical変換による真値を比較し、CSVを標準出力へ出す。
///
/// 列: time_s, ch, event, kc, kf, base_kc, pb, repro_midi, repro_note, ref_midi, ref_note, error_cents
/// - repro_* : 現パイプラインがエンジンに鳴らさせるピッチ（音痴の実体）
/// - ref_*   : 実機YM2151が鳴らすはずの真値
/// - error_cents : (repro - ref) をセント換算。±100の倍数で並べばノート表の欠番ズレが原因
fn dump_pitch(data: &[u8], data_start: usize) -> Result<(), String> {
    const SAMPLE_RATE: f64 = 44_100.0;

    let mut opm = OpmState::new();
    let mut samples: u64 = 0;

    let mut active:   [bool; 8] = [false; 8];
    let mut base_kc:  [u8;   8] = [0;     8];

    println!("time_s,ch,event,kc,kf,base_kc,pb,repro_midi,repro_note,ref_midi,ref_note,error_cents");

    // 1行出力するクロージャ的処理（borrow制約のためマクロ風に手書き）
    let emit = |samples: u64, ch: usize, event: &str, kc: u8, kf: u8, base: u8, pb: i16| {
        let repro = kc_to_midi_note(base) as f32 + pb_to_semitones(pb);
        let refm = kc_kf_to_ref_midi(kc, kf);
        let err = (repro - refm) * 100.0;
        println!(
            "{:.4},{},{},0x{:02X},{},0x{:02X},{},{:.3},{},{:.3},{},{:.1}",
            samples as f64 / SAMPLE_RATE,
            ch,
            event,
            kc,
            (kf >> 2) & 0x3F,
            base,
            pb,
            repro,
            midi_note_name(repro),
            refm,
            midi_note_name(refm),
            err,
        );
    };

    for cmd in VgmIter::new(data, data_start) {
        match cmd {
            VgmCmd::Wait(n) => samples += n as u64,
            VgmCmd::Ym2151Write { reg, val } => match reg {
                0x08 => {
                    let ch = (val & 0x07) as usize;
                    let slots = val & 0x78;
                    if slots != 0 {
                        opm.write(reg, val);
                        let kc = opm.kc(ch);
                        let kf = opm.kf(ch);
                        base_kc[ch] = kc;
                        let (pb, _) = compute_pitch_bend(kc, kc, kf);
                        active[ch] = true;
                        emit(samples, ch, "on", kc, kf, kc, pb);
                    } else {
                        active[ch] = false;
                        opm.write(reg, val);
                    }
                }
                0x28..=0x2F => {
                    let ch = (reg - 0x28) as usize;
                    opm.write(reg, val);
                    if active[ch] {
                        let kf = opm.kf(ch);
                        let (pb, oor) = compute_pitch_bend(val, base_kc[ch], kf);
                        if oor {
                            base_kc[ch] = val;
                            let (pb2, _) = compute_pitch_bend(val, val, kf);
                            emit(samples, ch, "kc-reon", val, kf, val, pb2);
                        } else {
                            emit(samples, ch, "kc", val, kf, base_kc[ch], pb);
                        }
                    }
                }
                0x30..=0x37 => {
                    let ch = (reg - 0x30) as usize;
                    opm.write(reg, val);
                    if active[ch] {
                        let kc = opm.kc(ch);
                        let (pb, _) = compute_pitch_bend(kc, base_kc[ch], val);
                        emit(samples, ch, "kf", kc, val, base_kc[ch], pb);
                    }
                }
                _ => opm.write(reg, val),
            },
            VgmCmd::End => break,
        }
    }

    Ok(())
}

/// 直接WAVレンダリング（--wav）。
///
/// VGMを走査し、run()と同じ音程ロジック（ノートオン/オフ・KC/KF→ピッチベンド・
/// プログラムチェンジ）で ym38x6-core を駆動し、演奏をそのままWAV（mono 44.1kHz 16bit）へ
/// 書き出す。DAW/VST/SMFを経由しないため、ピッチベンド感度RPNやホストのMIDI処理に依存しない
/// 「変換器＋エンジンが本来出すべき音」が得られる。
///
/// `gain`は書き出し前に全サンプルへ掛ける出力レベル倍率（クリップ対策。1.0で素通し）。
fn render_wav(data: &[u8], data_start: usize, out: &Path, gain: f32) -> Result<(), String> {
    use ym38x6_core::SoundEngine;
    const SR: f32 = 44_100.0;

    let mut opm = OpmState::new();
    let mut bank = PatchBank::new();
    let mut engine = ym38x6_core::Ym38x6Engine::new(SR);
    let mut audio: Vec<f32> = Vec::new();

    let mut active:   [bool; 8] = [false; 8];
    let mut base_kc:  [u8;   8] = [0;     8];
    let mut patch_idx: [Option<usize>; 8] = [None; 8];

    // pb(0-16383) → セント。VSTが感度±PB_SENSITIVITY半音で行う変換と同一
    // （pb_to_semitones は PB_SENSITIVITY を使う）。RPNが正しく効いた理想状態を再現する。
    let pb_cents = |pb: i16| pb_to_semitones(pb) * 100.0;
    let midi_to_freq = |note: u8| 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);

    for cmd in VgmIter::new(data, data_start) {
        match cmd {
            VgmCmd::Wait(n) => {
                let mut buf = vec![0.0f32; n as usize];
                engine.render(&mut buf, 1);
                audio.extend_from_slice(&buf);
            }
            VgmCmd::Ym2151Write { reg, val } => match reg {
                0x08 => {
                    let ch = (val & 0x07) as usize;
                    let slots = val & 0x78;
                    if slots != 0 {
                        opm.write(reg, val);
                        let voice = opm.build_voice(ch, slots);
                        let idx = bank.find_or_insert(voice);
                        patch_idx[ch] = Some(idx);
                        let patch = bank.patch_at(idx);
                        let kc = opm.kc(ch);
                        let kf = opm.kf(ch);
                        base_kc[ch] = kc;
                        let note = kc_to_midi_note(kc);
                        let (pb, _) = compute_pitch_bend(kc, kc, kf);
                        engine.note_on_with_velocity(ch, midi_to_freq(note), 100, patch);
                        engine.set_pitch_bend(ch, pb_cents(pb));
                        active[ch] = true;
                    } else {
                        if active[ch] {
                            engine.note_off(ch);
                        }
                        active[ch] = false;
                        opm.write(reg, val);
                    }
                }
                0x28..=0x2F => {
                    let ch = (reg - 0x28) as usize;
                    opm.write(reg, val);
                    if active[ch] {
                        let kf = opm.kf(ch);
                        let (pb, oor) = compute_pitch_bend(val, base_kc[ch], kf);
                        if oor {
                            // 感度超過 → 基準を取り直して再キーオン（SMFパスと同じ挙動）
                            base_kc[ch] = val;
                            let note = kc_to_midi_note(val);
                            let (pb2, _) = compute_pitch_bend(val, val, kf);
                            let patch = patch_idx[ch].map(|i| bank.patch_at(i)).unwrap_or_default();
                            engine.note_on_with_velocity(ch, midi_to_freq(note), 100, patch);
                            engine.set_pitch_bend(ch, pb_cents(pb2));
                        } else {
                            engine.set_pitch_bend(ch, pb_cents(pb));
                        }
                    }
                }
                0x30..=0x37 => {
                    let ch = (reg - 0x30) as usize;
                    opm.write(reg, val);
                    if active[ch] {
                        let kc = opm.kc(ch);
                        let (pb, _) = compute_pitch_bend(kc, base_kc[ch], val);
                        engine.set_pitch_bend(ch, pb_cents(pb));
                    }
                }
                _ => opm.write(reg, val),
            },
            VgmCmd::End => break,
        }
    }

    // リリースの尾を1秒分レンダリング
    let mut tail = vec![0.0f32; SR as usize];
    engine.render(&mut tail, 1);
    audio.extend_from_slice(&tail);

    // ゲイン適用前のピークを測り、クリップ状況を報告する
    let raw_peak = audio.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    if gain != 1.0 {
        for s in audio.iter_mut() {
            *s *= gain;
        }
    }
    let out_peak = raw_peak * gain;
    let clipped = audio.iter().filter(|&&s| s.abs() > 1.0).count();

    write_wav_mono16(out, &audio, SR as u32)
        .map_err(|e| format!("WAV書き込み失敗: {}: {e}", out.display()))?;
    println!(
        "WAV: {} ({:.1}秒, gain={:.2}, 素ピーク={:.3}, 出力ピーク={:.3}{})",
        out.display(),
        audio.len() as f32 / SR,
        gain,
        raw_peak,
        out_peak,
        if clipped > 0 {
            format!(", クリップ{clipped}サンプル→--gain {:.2}推奨", 1.0 / raw_peak.max(1e-6))
        } else {
            String::new()
        },
    );
    Ok(())
}

/// mono / 16bit PCM の最小 WAV ライター。
fn write_wav_mono16(path: &Path, samples: &[f32], sr: u32) -> std::io::Result<()> {
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
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out)
}

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    // 1. VGZ/VGM 読み込み
    let data = vgm::load(&args.input)?;
    let header = vgm::parse_header(&data)?;

    if header.ym2151_clock == 0 {
        return Err("YM2151 が見つかりません（YM2151 以外のVGMです）".into());
    }
    eprintln!(
        "YM2151 clock: {} Hz, {} samples",
        header.ym2151_clock, header.total_samples
    );

    // --dump-pitch: SMF/バンクを書かず、ピッチ二重変換の比較CSVを標準出力へ出す
    if args.dump_pitch {
        return dump_pitch(&data, header.data_start);
    }

    // --wav: DAW/VST/SMFを一切経由せず、ym38x6-coreで直接演奏をWAVへ書き出す。
    // 原曲VGZと聴き比べることで、変換器＋エンジンが正しい音程を出せるかを切り分ける。
    if let Some(wav_path) = &args.wav {
        return render_wav(&data, header.data_start, wav_path, args.gain);
    }

    // 2. 状態初期化
    let mut opm = OpmState::new();
    let mut bank = PatchBank::new();
    let mut smf = SmfBuilder::new();
    let mut samples: u64 = 0;

    // OPMチャンネルごとの演奏状態
    // track インデックス = ch + 1 (track 0 はテンポ専用)
    let mut active_note:  [Option<u8>;    8] = [None; 8]; // 現在鳴らしているMIDIノート
    let mut base_kc:      [u8;            8] = [0;    8]; // ノートオン時の基準KC
    let mut active_patch: [Option<usize>; 8] = [None; 8]; // 現在のパッチインデックス
    let mut last_pb:      [i16;           8] = [8192; 8]; // 直前のピッチベンド値

    // 各チャンネルのピッチベンド感度を ±PB_SENSITIVITY 半音に設定
    for ch in 0..8usize {
        smf.set_pitch_bend_sensitivity(ch + 1, ch as u8, PB_SENSITIVITY);
    }

    // 3. VGM コマンドループ
    for cmd in VgmIter::new(&data, header.data_start) {
        match cmd {
            VgmCmd::Wait(n) => {
                samples += n as u64;
            }

            VgmCmd::Ym2151Write { reg, val } => {
                let tick = SmfBuilder::samples_to_ticks(samples);

                match reg {
                    // キーオン/オフ
                    0x08 => {
                        let ch    = (val & 0x07) as usize;
                        let slots = val & 0x78; // bits[6:3]
                        let tr    = ch + 1;     // SMF トラック番号

                        if slots != 0 {
                            // キーオン: 既存ノートがあれば先に解放
                            if let Some(prev_note) = active_note[ch].take() {
                                smf.add_note_off(tr, tick, ch as u8, prev_note);
                            }

                            // 音色スナップショット
                            let voice = opm.build_voice(ch, slots);
                            let patch_idx = bank.find_or_insert(voice);

                            // プログラムチェンジ（音色が変わったとき）
                            // Program Change (0xCn) + CC92 の両方を送ることで
                            // CLAP（Program Change）と VST3（CC92）の両方に対応する。
                            if active_patch[ch] != Some(patch_idx) {
                                let prog = (patch_idx % 128) as u8;
                                smf.add_program_change(tr, tick, ch as u8, prog);
                                smf.add_cc(tr, tick, ch as u8, 92, prog);
                                active_patch[ch] = Some(patch_idx);
                            }

                            // 基準KC を更新してピッチベンドをリセット
                            let kc = opm.kc(ch);
                            base_kc[ch] = kc;
                            let (pb, _) = compute_pitch_bend(kc, kc, opm.kf(ch));
                            if pb != last_pb[ch] {
                                smf.add_pitch_bend(tr, tick, ch as u8, pb);
                                last_pb[ch] = pb;
                            }

                            // ノートオン（基準KCのMIDIノート）
                            let note = kc_to_midi_note(kc);
                            smf.add_note_on(tr, tick, ch as u8, note, 100);
                            active_note[ch] = Some(note);
                        } else {
                            // キーオフ
                            if let Some(prev_note) = active_note[ch].take() {
                                smf.add_note_off(tr, tick, ch as u8, prev_note);
                            }
                        }
                    }

                    // KC 変更（ビブラート・ポルタメント → ピッチベンドで表現）
                    0x28..=0x2F => {
                        let ch = (reg - 0x28) as usize;
                        opm.write(reg, val);
                        if active_note[ch].is_some() {
                            let tr = ch + 1;
                            let (pb, out_of_range) =
                                compute_pitch_bend(val, base_kc[ch], opm.kf(ch));
                            if out_of_range {
                                // 感度を超える大きな音程変化 → Note Off + On
                                if let Some(prev_note) = active_note[ch].take() {
                                    smf.add_note_off(tr, tick, ch as u8, prev_note);
                                }
                                base_kc[ch] = val;
                                let (pb2, _) = compute_pitch_bend(val, val, opm.kf(ch));
                                if pb2 != last_pb[ch] {
                                    smf.add_pitch_bend(tr, tick, ch as u8, pb2);
                                    last_pb[ch] = pb2;
                                }
                                let note = kc_to_midi_note(val);
                                smf.add_note_on(tr, tick, ch as u8, note, 100);
                                active_note[ch] = Some(note);
                            } else if pb != last_pb[ch] {
                                smf.add_pitch_bend(tr, tick, ch as u8, pb);
                                last_pb[ch] = pb;
                            }
                        }
                    }

                    // KF 変更 → ピッチベンド更新（KC delta と合算）
                    0x30..=0x37 => {
                        let ch = (reg - 0x30) as usize;
                        opm.write(reg, val);
                        if active_note[ch].is_some() {
                            let (pb, _) = compute_pitch_bend(opm.kc(ch), base_kc[ch], val);
                            if pb != last_pb[ch] {
                                smf.add_pitch_bend(ch + 1, tick, ch as u8, pb);
                                last_pb[ch] = pb;
                            }
                        }
                    }

                    _ => {
                        opm.write(reg, val);
                    }
                }
            }

            VgmCmd::End => break,
        }
    }

    // 残っているノートをすべて解放
    let end_tick = SmfBuilder::samples_to_ticks(samples);
    for ch in 0..8usize {
        if let Some(note) = active_note[ch].take() {
            smf.add_note_off(ch + 1, end_tick, ch as u8, note);
        }
    }

    // 4. ファイル出力
    bank.write(&args.out_bank, args.bank)?;
    println!("音色バンク: {} ({} 音色)", args.out_bank.display(), bank.len());

    smf.write(&args.out_midi).map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
    println!("MIDI: {}", args.out_midi.display());

    Ok(())
}
