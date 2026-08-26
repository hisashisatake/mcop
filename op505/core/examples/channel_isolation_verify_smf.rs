//! op505-vstのMIDIチャンネル別化（`channels: [op505_midi::ChannelState; 16]`全面移行、
//! 2026-08-26）向けREAPER実機確認用SMFを書き出す使い捨てツール
//! （`phase2_test_patches.rs`と同じ「使い捨て診断用」の位置づけ）。
//!
//! `phase2_verify.mid`（マーカー00〜20）とは独立した新規ファイル。ch1(MIDI ch1)/ch2(MIDI ch2)
//! の2チャンネルだけを使い、「他chへのNRPN/RPN/Program Changeが対象chへ漏れないこと」を
//! 音の違いとして聴き取れるように構成する。前提として`phase2_test_patches`
//! （`cargo run -p op505-core --example phase2_test_patches`）で書き出したbank=4の
//! Test Lead(program=0)/Test Pluck(program=1)を使う。
//!
//! 実行: cargo run -p op505-core --example channel_isolation_verify_smf
//! 出力先: `op505/vst/private/channel_isolation_verify.mid`
//!
//! マーカー対応表（詳細は`op505/vst/private/phase2_verify_checklist.md`「新規追加項目」節）:
//! - 21: NRPN(0,9) Algorithm。ch1だけ音色が変わりch2は不変
//! - 22: ch1↔ch2でNRPN選択(CC98/99)を交互に送る→ch1のCC38/CC6がch2の選択に横取りされない
//! - 23: ch2へのProgram Changeがch1のNRPN上書き(Algorithm)を消さない
//! - 24: RPN(0,0) Pitch Bend Range。ch1だけベンド量が大きくなりch2は既定のまま

use std::path::PathBuf;

const TEST_BANK_MSB: u8 = 0; // bank=4 = msb*128+lsb → msb=0
const TEST_BANK_LSB: u8 = 4;
const PROGRAM_LEAD: u8 = 0;
const PROGRAM_PLUCK: u8 = 1;
const NOTE_C4: u8 = 60;
const NOTE_E4: u8 = 64;
const DIVISION: u16 = 480;

/// 可変長数値（delta-time）をエンコードする（`smf2op505`のテストヘルパーと同型）。
fn vlq(mut v: u32) -> Vec<u8> {
    let mut buf = vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 {
        buf.insert(0, ((v & 0x7F) as u8) | 0x80);
        v >>= 7;
    }
    buf
}

fn marker(text: &str) -> Vec<u8> {
    let mut ev = vec![0xFF, 0x06, text.len() as u8];
    ev.extend_from_slice(text.as_bytes());
    ev
}

/// Bank Select(CC0/CC32) + Program Change。**CC102（Program Change代替）を使う**——
/// 通常のMIDI Program Change（0xC0）はVST3プラグインへ届かない既知の制約があるため
/// （`op505/vst/src/lib.rs`のコメント参照、前セッションで誤診断しかけた罠）。
fn bank_select_and_program_change(ch: u8, program: u8) -> Vec<(u32, Vec<u8>)> {
    vec![
        (0, vec![0xB0 | ch, 0, TEST_BANK_MSB]),
        (0, vec![0xB0 | ch, 32, TEST_BANK_LSB]),
        (0, vec![0xB0 | ch, 102, program]),
    ]
}

fn select_nrpn(ch: u8, msb: u8, lsb: u8) -> Vec<(u32, Vec<u8>)> {
    vec![(0, vec![0xB0 | ch, 99, msb]), (0, vec![0xB0 | ch, 98, lsb])]
}

fn select_rpn(ch: u8, msb: u8, lsb: u8) -> Vec<(u32, Vec<u8>)> {
    vec![(0, vec![0xB0 | ch, 101, msb]), (0, vec![0xB0 | ch, 100, lsb])]
}

fn data_entry(ch: u8, value: u8) -> (u32, Vec<u8>) {
    (0, vec![0xB0 | ch, 6, value])
}

fn note_on(delta: u32, ch: u8, note: u8, vel: u8) -> (u32, Vec<u8>) {
    (delta, vec![0x90 | ch, note, vel])
}

fn note_off(delta: u32, ch: u8, note: u8) -> (u32, Vec<u8>) {
    (delta, vec![0x80 | ch, note, 0])
}

fn build_smf(events: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut track: Vec<u8> = Vec::new();
    for (delta, bytes) in events {
        track.extend(vlq(*delta));
        track.extend_from_slice(bytes);
    }
    track.extend(vlq(0));
    track.extend_from_slice(&[0xFF, 0x2F, 0x00]); // End of Track

    let mut smf: Vec<u8> = Vec::new();
    smf.extend_from_slice(b"MThd");
    smf.extend_from_slice(&6u32.to_be_bytes());
    smf.extend_from_slice(&0u16.to_be_bytes()); // format 0
    smf.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    smf.extend_from_slice(&DIVISION.to_be_bytes());
    smf.extend_from_slice(b"MTrk");
    smf.extend_from_slice(&(track.len() as u32).to_be_bytes());
    smf.extend_from_slice(&track);
    smf
}

fn main() {
    let ch1 = 0u8; // MIDI ch1（0-indexed）
    let ch2 = 1u8; // MIDI ch2（0-indexed）
    let mut ev: Vec<(u32, Vec<u8>)> = Vec::new();

    // --- 00: 両ch共通の初期化（Test Leadをbank4/program0で選択） ---
    ev.push((0, marker("00")));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_LEAD));
    ev.extend(bank_select_and_program_change(ch2, PROGRAM_LEAD));

    // --- 21: NRPN(0,9) Algorithm。ch1だけ音色が変わりch2は不変 ---
    ev.push((960, marker("21")));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_on(0, ch2, NOTE_E4, 100));
    ev.extend(select_nrpn(ch1, 0, 9));
    ev.push(data_entry(ch1, 7)); // Algorithm=4(既定)→7、ch1だけ切り替わるはず
    ev.push(note_off(960, ch1, NOTE_C4));
    ev.push(note_off(0, ch2, NOTE_E4));

    // --- 22: ch1↔ch2でNRPN選択を交互に送ってもch1のCC38/CC6はch1自身の選択にのみ従う ---
    ev.push((480, marker("22")));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_on(0, ch2, NOTE_E4, 100));
    ev.extend(select_nrpn(ch1, 0, 18)); // ch1: Op1 F-Number選択
    ev.extend(select_nrpn(ch2, 0, 14)); // ch2: Filter Typeを選択（ch1の選択を横取りしないか）
    ev.push((0, vec![0xB0 | ch1, 38, 0])); // CC38(LSB)=0
    ev.push(data_entry(ch1, 16)); // combined=16*128=2048=F_NUMBER_CENTER(4096)の半分→1オクターブ下
    ev.push(note_off(960, ch1, NOTE_C4));
    ev.push(note_off(0, ch2, NOTE_E4));

    // --- 23: ch2へのProgram Changeがch1のNRPN上書き(Algorithm)を消さない ---
    ev.push((480, marker("23")));
    ev.extend(select_nrpn(ch1, 0, 9));
    ev.push(data_entry(ch1, 7)); // ch1: Algorithm override ON
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.extend(bank_select_and_program_change(ch2, PROGRAM_PLUCK)); // ch2だけPC（ch1のoverridesは無関係のはず）
    ev.push(note_off(960, ch1, NOTE_C4));

    // --- 24: RPN(0,0) Pitch Bend Range。ch1だけベンドが大きく開きch2は既定(±2半音)のまま ---
    ev.push((480, marker("24")));
    ev.extend(bank_select_and_program_change(ch2, PROGRAM_LEAD)); // ch2をTest Leadへ戻す
    ev.extend(select_rpn(ch1, 0, 0));
    ev.push(data_entry(ch1, 12)); // ch1: Pitch Bend Range=12半音
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_on(0, ch2, NOTE_E4, 100));
    ev.push((240, vec![0xE0 | ch1, 0x7F, 0x7F])); // ch1: 最大ピッチベンド
    ev.push((0, vec![0xE0 | ch2, 0x7F, 0x7F])); // ch2: 同じ最大ピッチベンド（既定range±2半音のまま）
    ev.push(note_off(720, ch1, NOTE_C4));
    ev.push(note_off(0, ch2, NOTE_E4));

    ev.push((480, marker("25 (end)")));

    let smf = build_smf(&ev);

    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("vst")
        .join("private")
        .join("channel_isolation_verify.mid");
    std::fs::write(&out_path, &smf).expect("write channel_isolation_verify.mid");
    println!("Wrote {} ({} bytes)", out_path.display(), smf.len());
}
