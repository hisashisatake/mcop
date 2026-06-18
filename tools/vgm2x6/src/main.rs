//! vgm2x6 — YM2151 VGM/VGZ から音色（.38x6）と SMF（.mid）を抽出する。
//!
//! 使い方:
//! ```text
//! vgm2x6 <input.vgz|.vgm> [--out-bank output.38x6] [--out-midi output.mid] [--bank N]
//! ```

mod opm;
mod patch;
mod smf;
mod vgm;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use opm::{kc_to_midi_note, kf_to_pitch_bend, OpmState};
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
                "usage: vgm2x6 <input.vgz|.vgm> \
                 [--out-bank output.38x6] [--out-midi output.mid] [--bank N]"
            );
            ExitCode::FAILURE
        }
    }
}

struct Args {
    input: PathBuf,
    out_bank: Option<PathBuf>,
    out_midi: Option<PathBuf>,
    bank: u16,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_bank: Option<PathBuf> = None;
    let mut out_midi: Option<PathBuf> = None;
    let mut bank: u16 = 1;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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

    // デフォルト出力パス: 入力と同じディレクトリ、同じステム
    let stem = input.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out");
    let dir = input.parent().unwrap_or(Path::new("."));

    let out_bank = Some(out_bank.unwrap_or_else(|| dir.join(format!("{stem}.38x6"))));
    let out_midi = Some(out_midi.unwrap_or_else(|| dir.join(format!("{stem}.mid"))));

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
    let mut active_note:    [Option<u8>;   8] = [None; 8]; // 現在鳴らしているMIDIノート
    let mut active_patch:   [Option<usize>; 8] = [None; 8]; // 現在のパッチインデックス
    let mut last_pb:        [i16; 8]           = [8192; 8]; // 直前のピッチベンド値

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

                            // KF → ピッチベンド
                            let pb = kf_to_pitch_bend(opm.kf(ch));
                            if pb != last_pb[ch] {
                                smf.add_pitch_bend(tr, tick, ch as u8, pb);
                                last_pb[ch] = pb;
                            }

                            // ノートオン
                            let note = kc_to_midi_note(opm.kc(ch));
                            smf.add_note_on(tr, tick, ch as u8, note, 100);
                            active_note[ch] = Some(note);
                        } else {
                            // キーオフ
                            if let Some(prev_note) = active_note[ch].take() {
                                smf.add_note_off(tr, tick, ch as u8, prev_note);
                            }
                        }
                    }

                    // KC 変更（音程変化 = ノートオフ→オン）
                    0x28..=0x2F => {
                        let ch = (reg - 0x28) as usize;
                        opm.write(reg, val);
                        if let Some(prev_note) = active_note[ch].take() {
                            let tr = ch + 1;
                            smf.add_note_off(tr, tick, ch as u8, prev_note);
                            let pb = kf_to_pitch_bend(opm.kf(ch));
                            if pb != last_pb[ch] {
                                smf.add_pitch_bend(tr, tick, ch as u8, pb);
                                last_pb[ch] = pb;
                            }
                            let note = kc_to_midi_note(val);
                            smf.add_note_on(tr, tick, ch as u8, note, 100);
                            active_note[ch] = Some(note);
                        }
                    }

                    // KF 変更 → ピッチベンド更新
                    0x30..=0x37 => {
                        let ch = (reg - 0x30) as usize;
                        opm.write(reg, val);
                        if active_note[ch].is_some() {
                            let pb = kf_to_pitch_bend(val);
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
    if let Some(ref bp) = args.out_bank {
        bank.write(bp, args.bank)?;
        println!("音色バンク: {} ({} 音色)", bp.display(), bank.len());
    }
    if let Some(ref mp) = args.out_midi {
        smf.write(mp).map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
        println!("MIDI: {}", mp.display());
    }

    Ok(())
}
