//! op505-vstの一連のNRPN/RPN実装（NRPN(0,1) Channel Effect Route・NRPN(0,22)〜(0,24) Fixed
//! Note・RPN(0,1)/(0,2) Channel Fine/Coarse Tuning・NRPN(0,28)〜(0,33) FG Loop/Curve）向け
//! REAPER実機確認用SMFを書き出す使い捨てツール（`phase2_test_patches.rs`・
//! `channel_isolation_verify_smf.rs`と同じ「使い捨て診断用」の位置づけ）。
//!
//! 前提として`phase2_test_patches`（`cargo run -p op505-core --example phase2_test_patches`）で
//! bank=4に書き出したTest Lead(program=0)/Test Vibrato(program=2)/Test Cutoff FG(program=3)/
//! Test Gain FG(program=4)を使う。
//!
//! 実行: cargo run -p op505-core --example nrpn_verify_smf
//! 出力先: `op505/vst/private/nrpn_verify.mid`
//!
//! マーカー対応表（詳細は`op505/vst/private/nrpn_verify_checklist.md`参照）:
//! - 01〜04: NRPN(0,22)〜(0,24) Fixed Note Enable/Note/Fine
//! - 05〜09: RPN(0,1)/(0,2) Channel Fine/Coarse Tuning
//! - 10〜12: NRPN(0,1) Channel Effect Route + MasterEffectsマルチスロット
//! - 13〜16: NRPN(0,28)/(0,29) Pitch FG Loop/Curve
//! - 17〜19: NRPN(0,30)/(0,31) Cutoff FG Loop/Curve
//! - 20〜22: NRPN(0,32)/(0,33) Gain FG Loop/Curve
//! - 24〜26: NRPN(0,25) Pitch FG Depth絶対上書き
//! - 27〜29: NRPN(0,26) Cutoff FG Depth絶対上書き
//! - 30〜33: NRPN(0,27) Gain FG Depth絶対上書き + CC92(Tremolo Depth)加算

use std::path::PathBuf;

const TEST_BANK_MSB: u8 = 0; // bank=4 = msb*128+lsb → msb=0
const TEST_BANK_LSB: u8 = 4;
const PROGRAM_LEAD: u8 = 0;
const PROGRAM_VIBRATO: u8 = 2;
const PROGRAM_CUTOFF_FG: u8 = 3;
const PROGRAM_GAIN_FG: u8 = 4;
const NOTE_C3: u8 = 48;
const NOTE_C4: u8 = 60;
const NOTE_C5: u8 = 72;
const DIVISION: u16 = 480; // 1拍=480tick、既定120BPMで1拍=0.5秒
/// NRPN/RPNの「アドレス選択→値書き込み」の組ごとに空ける間隔（約0.2秒、120BPMで200tick）。
/// VST3ホストのパラメーター自動化は、同一処理ブロック内で同じCC番号(98/99/6/100/101)が
/// 複数回書き換わると最後の値だけを反映し、途中の書き込みが消える可能性がある
/// （オフラインレンダラーのsmf2op505はファイル順にそのまま処理するためこの問題が起きず、
/// VSTとの結果不一致の原因になり得る）。典型的なオーディオブロック長(数ms〜数十ms)より
/// 十分大きい間隔を空けることで、異なるNRPN/RPNアドレスへの書き込みが同一ブロックに
/// 混在しないようにする。
const GROUP_GAP: u32 = 200;

/// 可変長数値（delta-time）をエンコードする（`channel_isolation_verify_smf.rs`と同型）。
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

fn tempo_120bpm() -> Vec<u8> {
    // 500000 µs/四分音符 = 120BPM
    vec![0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]
}

/// Bank Select(CC0/CC32) + Program Change。**CC102（Program Change代替）を使う**——
/// 通常のMIDI Program Change（0xC0）はVST3プラグインへ届かない既知の制約があるため
/// （`channel_isolation_verify_smf.rs`と同じ罠、`op505/vst/src/lib.rs`のコメント参照）。
fn bank_select_and_program_change(ch: u8, program: u8) -> Vec<(u32, Vec<u8>)> {
    vec![
        (0, vec![0xB0 | ch, 0, TEST_BANK_MSB]),
        (0, vec![0xB0 | ch, 32, TEST_BANK_LSB]),
        (0, vec![0xB0 | ch, 102, program]),
    ]
}

/// アドレス選択の先頭イベントに`GROUP_GAP`を乗せる。直前のNRPN/RPN操作と同一ブロックに
/// 混在しないようにするため（上記`GROUP_GAP`のコメント参照）。
fn select_nrpn(ch: u8, msb: u8, lsb: u8) -> Vec<(u32, Vec<u8>)> {
    vec![(GROUP_GAP, vec![0xB0 | ch, 99, msb]), (0, vec![0xB0 | ch, 98, lsb])]
}

fn select_rpn(ch: u8, msb: u8, lsb: u8) -> Vec<(u32, Vec<u8>)> {
    vec![(GROUP_GAP, vec![0xB0 | ch, 101, msb]), (0, vec![0xB0 | ch, 100, lsb])]
}

fn data_entry(ch: u8, value: u8) -> (u32, Vec<u8>) {
    (0, vec![0xB0 | ch, 6, value])
}

fn data_entry_lsb(ch: u8, value: u8) -> (u32, Vec<u8>) {
    (0, vec![0xB0 | ch, 38, value])
}

fn cc(ch: u8, num: u8, value: u8) -> (u32, Vec<u8>) {
    (0, vec![0xB0 | ch, num, value])
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
    let ch2 = 1u8; // MIDI ch2（0-indexed、Effect Route確認専用）
    let mut ev: Vec<(u32, Vec<u8>)> = Vec::new();

    // --- 00: 初期化（両chともTest Leadをbank4/program0で選択） ---
    ev.push((0, marker("00")));
    ev.push((0, tempo_120bpm()));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_LEAD));
    ev.extend(bank_select_and_program_change(ch2, PROGRAM_LEAD));

    // =====================================================================
    // 01〜04: NRPN(0,22)〜(0,24) Fixed Note Enable/Note/Fine
    // =====================================================================

    // --- 01: ベースライン。C3とC5を鳴らし、異なる音程で鳴ることを確認 ---
    ev.push((960, marker("01")));
    ev.push(note_on(0, ch1, NOTE_C3, 100));
    ev.push(note_off(480, ch1, NOTE_C3));
    ev.push(note_on(240, ch1, NOTE_C5, 100));
    ev.push(note_off(480, ch1, NOTE_C5));

    // --- 02: Fixed Note Enable=ON、Fixed Note=C4、Fine=中心。C3/C5とも同じ音程(C4)になるはず ---
    ev.push((480, marker("02")));
    ev.extend(select_nrpn(ch1, 0, 22));
    ev.push(data_entry(ch1, 127)); // Fixed Note Enable ON（非0=true）
    ev.extend(select_nrpn(ch1, 0, 23));
    ev.push(data_entry(ch1, NOTE_C4)); // Fixed Note=C4(60)
    ev.extend(select_nrpn(ch1, 0, 24));
    ev.push(data_entry(ch1, 64)); // Fixed Note Fine=中心付近(cc_byte_to_u8(64)≈128)
    ev.push(note_on(0, ch1, NOTE_C3, 100));
    ev.push(note_off(480, ch1, NOTE_C3));
    ev.push(note_on(240, ch1, NOTE_C5, 100));
    ev.push(note_off(480, ch1, NOTE_C5));

    // --- 03: Fixed Note Fineを最大側へ。C5を鳴らすと02より約1半音シャープに聴こえるはず ---
    ev.push((480, marker("03")));
    ev.extend(select_nrpn(ch1, 0, 24));
    ev.push(data_entry(ch1, 127)); // Fine=127(最大、cc_byte_to_u8(127)=255→+100セント付近)
    ev.push(note_on(0, ch1, NOTE_C5, 100));
    ev.push(note_off(960, ch1, NOTE_C5));

    // --- 04: Fine=中心へ戻し、Enable=OFF。C3/C5が再び異なる音程に戻るはず（01と同じ聴こえ方） ---
    ev.push((480, marker("04")));
    ev.extend(select_nrpn(ch1, 0, 24));
    ev.push(data_entry(ch1, 64)); // Fine=中心へ戻す
    ev.extend(select_nrpn(ch1, 0, 22));
    ev.push(data_entry(ch1, 0)); // Fixed Note Enable OFF
    ev.push(note_on(0, ch1, NOTE_C3, 100));
    ev.push(note_off(480, ch1, NOTE_C3));
    ev.push(note_on(240, ch1, NOTE_C5, 100));
    ev.push(note_off(480, ch1, NOTE_C5));

    // =====================================================================
    // 05〜09: RPN(0,1)/(0,2) Channel Fine/Coarse Tuning
    // =====================================================================

    // --- 05: ベースライン。無補正のC4 ---
    ev.push((480, marker("05")));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(960, ch1, NOTE_C4));

    // --- 06: Coarse Tuning=70(+6半音)。C4がF#4相当まで上がって聴こえるはず ---
    ev.push((480, marker("06")));
    ev.extend(select_rpn(ch1, 0, 2));
    ev.push(data_entry(ch1, 70));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(960, ch1, NOTE_C4));

    // --- 07: Coarse Tuningを中心(64)へ戻す。05と同じ音程に戻るはず ---
    ev.push((480, marker("07")));
    ev.extend(select_rpn(ch1, 0, 2));
    ev.push(data_entry(ch1, 64));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(960, ch1, NOTE_C4));

    // --- 08: Fine Tuning=最大(16383、+100セント弱)。06(+6半音)とは異なる、半音弱の
    //     微妙な上ずりとして聴こえるはず（CoarseとFineが独立に効くことの確認） ---
    ev.push((480, marker("08")));
    ev.extend(select_rpn(ch1, 0, 1));
    ev.push(data_entry(ch1, 127)); // MSB
    ev.push(data_entry_lsb(ch1, 127)); // LSB → 14bit=16383
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(960, ch1, NOTE_C4));

    // --- 09: Fine Tuningを中心(8192)へ戻す。05と同じ音程に戻るはず ---
    ev.push((480, marker("09")));
    ev.extend(select_rpn(ch1, 0, 1));
    ev.push(data_entry(ch1, 64)); // MSB=64
    ev.push(data_entry_lsb(ch1, 0)); // LSB=0 → 14bit=8192(中心)
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(960, ch1, NOTE_C4));

    // =====================================================================
    // 10〜12: NRPN(0,1) Channel Effect Route + MasterEffectsマルチスロット
    // =====================================================================

    // --- 10: ch2もTest Leadへ。ch1(スロット0=既定)にリバーブを深くかける。
    //     ch1のノートは長い残響を伴って鳴るはず ---
    ev.push((480, marker("10")));
    ev.extend(bank_select_and_program_change(ch2, PROGRAM_LEAD));
    ev.extend(select_nrpn(ch1, 0, 2));
    ev.push(data_entry(ch1, 1)); // Reverb Type=1
    ev.extend(select_nrpn(ch1, 0, 4));
    ev.push(data_entry(ch1, 220)); // Reverb Time=長め
    ev.push(cc(ch1, 91, 127)); // Send To Reverb=最大
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(480, ch1, NOTE_C4));

    // --- 11: ch2をNRPN(0,1)でスロット1へルーティングし、スロット1はReverb Type=0(Room1、
    //     最も小さい空間)・Time=0(最短)に設定。ch2はch1と同じCC91=127を送っても
    //     ch1(Room2・長め)よりずっと小さく短い部屋鳴りに聴こえるはず
    //     （ReverbTypeを明示的に設定しないと既定のHall1のまま残り、Timeを短くしても
    //     Hall1固有の広い空間設計(predelay長め・delay_scale大)がch1より深く聴こえてしまう
    //     ことが判明したため、Typeも必ず明示する） ---
    ev.push((960, marker("11")));
    ev.extend(select_nrpn(ch2, 0, 1));
    ev.push(data_entry(ch2, 1)); // Channel Effect Route=スロット1
    ev.extend(select_nrpn(ch2, 0, 2));
    ev.push(data_entry(ch2, 0)); // スロット1のReverb Type=0(Room1、最小の空間)
    ev.extend(select_nrpn(ch2, 0, 4));
    ev.push(data_entry(ch2, 0)); // スロット1のReverb Time=0(最短)
    ev.push(cc(ch2, 91, 127)); // Send To Reverb=最大(ch1と同じ送り量)
    ev.push(note_on(0, ch2, NOTE_C4, 100));
    ev.push(note_off(480, ch2, NOTE_C4));

    // --- 12: ch1を新規コマンド無しで再度鳴らす。10で設定したスロット0の深い残響が
    //     ch2側の変更(11)に影響されず残っているはず（マルチスロットの独立性の核心） ---
    ev.push((960, marker("12")));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(480, ch1, NOTE_C4));

    // --- リセット: 13以降はFG(ピッチ/カットオフ/ゲイン)の聴き取りに集中できるよう、
    //     ch1(スロット0)・ch2(スロット1)双方のReverb Sendを0へ戻す。
    //     **Send=0は「これ以上新しい音を送り込まない」だけで、FDN内に既に溜まっている
    //     エネルギーの減衰速度は変えない**。Reverb Time=220(Room2、RT60≈1.26秒)のまま
    //     だと、Sendを止めてもテール自体は1秒以上鳴り続ける。`set_reverb_time`は
    //     フィードバックゲインをその場で更新する（次回tick以降に効く）ため、Timeも
    //     最短へ戻すと既にリング中のテールの減衰そのものが加速する。そのうえで
    //     RT60=最短(base_rt60=0.35秒)が十分減衰する余裕を見て2秒空ける ---
    ev.push((GROUP_GAP, cc(ch1, 91, 0).1));
    ev.extend(select_nrpn(ch1, 0, 4));
    ev.push(data_entry(ch1, 0)); // ch1(スロット0) Reverb Time最短へ→テールの減衰を加速
    ev.push(cc(ch2, 91, 0));
    ev.extend(select_nrpn(ch2, 0, 4));
    ev.push(data_entry(ch2, 0)); // ch2(スロット1) Reverb Timeも最短へ

    // =====================================================================
    // 13〜16: NRPN(0,28)/(0,29) Pitch FG Loop/Curve（Test Vibrato使用）
    // =====================================================================

    // --- 13: Test Vibratoへ切替。長い1音でビブラートが持続することを確認（既定でloop=1）。
    //     リセット直後からReverb Timeが最短になっているため、ここまでの間隔で
    //     テールは実用上無音まで減衰しているはず ---
    ev.push((1920, marker("13")));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_VIBRATO));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4)); // 約3秒(120BPMで2880tick)

    // --- 14: 長い1音の途中でPitch FG Loop=OFFにする。ビブラートが止まり
    //     音の後半は一定ピッチで持続するはず ---
    ev.push((480, marker("14")));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.extend(select_nrpn(ch1, 0, 28));
    ev.push((960 - GROUP_GAP, data_entry(ch1, 0).1)); // 1秒経過後にLoop OFF
    ev.push(note_off(1920, ch1, NOTE_C4)); // 残り2秒は一定ピッチのはず

    // --- 15: Loopを戻してから長い1音。ビブラートが最初から最後まで持続することを再確認 ---
    ev.push((480, marker("15")));
    ev.extend(select_nrpn(ch1, 0, 28));
    ev.push(data_entry(ch1, 1)); // Loop ON
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 16: Pitch FG Curveを切替（線形→サイン風、全段一括）。ビブラートの質感が
    //     わずかに変わる（聴き取りにくい可能性がある、参考程度の確認） ---
    ev.push((480, marker("16")));
    ev.extend(select_nrpn(ch1, 0, 29));
    ev.push(data_entry(ch1, 1)); // Curve=1(サイン風)
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));
    ev.extend(select_nrpn(ch1, 0, 29));
    ev.push(data_entry(ch1, 0)); // Curveを線形へ戻す(既定に戻す)

    // =====================================================================
    // 17〜19: NRPN(0,30)/(0,31) Cutoff FG Loop/Curve（Test Cutoff FG使用）
    // =====================================================================

    // --- 17: Test Cutoff FGへ切替。長い1音でオートワウ(フィルター掃引)が
    //     持続することを確認（既定でloop=1） ---
    ev.push((480, marker("17")));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_CUTOFF_FG));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 18: 長い1音の途中でCutoff FG Loop=OFF。掃引が止まり音の後半は
    //     一定の音色(フィルター開度)で持続するはず ---
    ev.push((480, marker("18")));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.extend(select_nrpn(ch1, 0, 30));
    ev.push((960 - GROUP_GAP, data_entry(ch1, 0).1));
    ev.push(note_off(1920, ch1, NOTE_C4));

    // --- 19: Loopを戻してから長い1音。掃引が最初から最後まで持続することを再確認 ---
    ev.push((480, marker("19")));
    ev.extend(select_nrpn(ch1, 0, 30));
    ev.push(data_entry(ch1, 1));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // =====================================================================
    // 20〜22: NRPN(0,32)/(0,33) Gain FG Loop/Curve（Test Gain FG使用）
    // =====================================================================

    // --- 20: Test Gain FGへ切替。長い1音でトレモロ(音量の往復)が
    //     持続することを確認（既定でloop=1） ---
    ev.push((480, marker("20")));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_GAIN_FG));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 21: 長い1音の途中でGain FG Loop=OFF。トレモロが止まり音の後半は
    //     一定音量(フル)で持続するはず ---
    ev.push((480, marker("21")));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.extend(select_nrpn(ch1, 0, 32));
    ev.push((960 - GROUP_GAP, data_entry(ch1, 0).1));
    ev.push(note_off(1920, ch1, NOTE_C4));

    // --- 22: Loopを戻してから長い1音。トレモロが最初から最後まで持続することを再確認 ---
    ev.push((480, marker("22")));
    ev.extend(select_nrpn(ch1, 0, 32));
    ev.push(data_entry(ch1, 1));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // =====================================================================
    // 24〜26: NRPN(0,25) Pitch FG Depth絶対上書き（Test Vibrato使用、
    // プリセット本来のdepth=183）
    // =====================================================================

    // --- 24: Test Vibratoへ切替（Program Changeで前段の上書きは全てクリアされる）。
    //     長い1音でプリセット本来の深さ(183)のビブラートを確認 ---
    ev.push((480, marker("24")));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_VIBRATO));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 25: NRPN(0,25) Pitch FG Depth=40(浅い、絶対上書き)。24より明確に浅いビブラートのはず ---
    ev.push((480, marker("25")));
    ev.extend(select_nrpn(ch1, 0, 25));
    ev.push(data_entry(ch1, 20)); // cc_byte_to_u8(20)=40
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 26: NRPN(0,25) Pitch FG Depth=255(最大、絶対上書き)。24(183)よりさらに深いビブラートのはず ---
    ev.push((480, marker("26")));
    ev.extend(select_nrpn(ch1, 0, 25));
    ev.push(data_entry(ch1, 127)); // cc_byte_to_u8(127)=255
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // =====================================================================
    // 27〜29: NRPN(0,26) Cutoff FG Depth絶対上書き（Test Cutoff FG使用、
    // プリセット本来のdepth=200）
    // =====================================================================

    // --- 27: Test Cutoff FGへ切替（Program Changeで25/26の上書きはクリアされる）。
    //     長い1音でプリセット本来の深さ(200)のオートワウを確認 ---
    ev.push((480, marker("27")));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_CUTOFF_FG));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 28: NRPN(0,26) Cutoff FG Depth=40(浅い)。27よりフィルター掃引が明確に狭いはず ---
    ev.push((480, marker("28")));
    ev.extend(select_nrpn(ch1, 0, 26));
    ev.push(data_entry(ch1, 20)); // cc_byte_to_u8(20)=40
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 29: NRPN(0,26) Cutoff FG Depth=255(最大)。27(200)よりさらに広い掃引のはず ---
    ev.push((480, marker("29")));
    ev.extend(select_nrpn(ch1, 0, 26));
    ev.push(data_entry(ch1, 127)); // cc_byte_to_u8(127)=255
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // =====================================================================
    // 30〜33: NRPN(0,27) Gain FG Depth絶対上書き + CC92(Tremolo Depth)加算
    // （Test Gain FG使用、プリセット本来のdepth=255＝旧仕様と同じ「EGの形をそのまま使う」）
    // =====================================================================

    // --- 30: Test Gain FGへ切替（Program Changeで28/29の上書きはクリアされる）。
    //     長い1音でプリセット本来の深さ(255=フル)のトレモロを確認 ---
    ev.push((480, marker("30")));
    ev.extend(bank_select_and_program_change(ch1, PROGRAM_GAIN_FG));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 31: NRPN(0,27) Gain FG Depth=40(浅い、絶対上書き)。30よりトレモロがほぼ聴こえない
    //     程度に浅くなるはず ---
    ev.push((480, marker("31")));
    ev.extend(select_nrpn(ch1, 0, 27));
    ev.push(data_entry(ch1, 20)); // cc_byte_to_u8(20)=40
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 32: 31のNRPN上書き(depth=40)を保持したままCC92=120を加算(depth=40+120=160)。
    //     31より深く、30(255)より浅いトレモロになるはず——CC92がNRPN上書き後の値へ
    //     さらに加算されることの確認 ---
    ev.push((480, marker("32")));
    ev.push(cc(ch1, 92, 60)); // cc_byte_to_u8(60)=120
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    // --- 33: CC92=0へ戻す（depth=40のまま、31と同じ浅さに戻るはず）。
    //     CC92の加算が可逆であることの確認 ---
    ev.push((480, marker("33")));
    ev.push(cc(ch1, 92, 0));
    ev.push(note_on(0, ch1, NOTE_C4, 100));
    ev.push(note_off(2880, ch1, NOTE_C4));

    ev.push((480, marker("34 (end)")));

    // デバッグ用：各マーカーの絶対tick/秒数を出力する（波形解析での区間切り出しに使う。
    // GROUP_GAP等の変更で全体のタイミングがずれても手計算し直さずに済むようにするため）。
    let mut cumulative_tick: u64 = 0;
    for (delta, bytes) in &ev {
        cumulative_tick += *delta as u64;
        if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0x06 {
            let text = String::from_utf8_lossy(&bytes[3..]);
            let seconds = cumulative_tick as f64 / (DIVISION as f64 * 2.0); // 960tick/秒(120BPM)
            println!("marker {text}: tick={cumulative_tick} t={seconds:.3}s");
        }
    }

    let smf = build_smf(&ev);

    let out_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("vst").join("private");
    std::fs::create_dir_all(&out_dir).expect("create op505/vst/private");
    let out_path = out_dir.join("nrpn_verify.mid");
    std::fs::write(&out_path, &smf).expect("write nrpn_verify.mid");
    println!("Wrote {} ({} bytes)", out_path.display(), smf.len());
}
