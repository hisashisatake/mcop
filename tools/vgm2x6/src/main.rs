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

use opm::{compute_pitch_bend, kc_to_midi_note, OpmState, PB_SENSITIVITY};
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
                 [--out-bank <file>] [--out-midi <file>] [--bank N]"
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
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut out_bank: Option<PathBuf> = None;
    let mut out_midi: Option<PathBuf> = None;
    let mut bank: u16 = 1;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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

    Ok(Args { input, out_bank, out_midi, bank })
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
                            if active_patch[ch] != Some(patch_idx) {
                                smf.add_program_change(
                                    tr, tick, ch as u8, (patch_idx % 128) as u8,
                                );
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
