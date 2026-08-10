//! vgm2x6 — YM2151(OPM) / OPN系(YM2612/YM2203/YM2608) の VGM/VGZ から
//! 音色（.38x6）と SMF（.mid）または WAV を抽出する。
//!
//! 対応チップは入力VGMヘッダーのクロック欄から自動判定する（YM2151を最優先）。
//! OPN系では FM チャンネルに加え、YM2203/YM2608 内蔵の SSG(PSG) 3ch を矩形波音色で
//! 常に出力する（SSGの音量レジスタはSMFではCC11、WAVではキャリアTLへ反映）。
//! OPN音色→38x6変換は mucom2x6 の OPN変換を再利用する。
//!
//! 使い方:
//! ```text
//! vgm2x6 <input.vgz|.vgm> [--out <dir>] [--out-bank <file>] [--out-midi <file>] [--bank N]
//!        [--out-wav <file>] [--wav] [--gain <factor>] [--fm-gain <factor>] [--ssg-gain <factor>]
//!        [--dump-pitch]
//! ```
//! - `--out <dir>`: 出力ディレクトリを指定（ファイル名は入力ステム名を使用）
//! - `--out-bank` / `--out-midi` / `--out-wav`: 個別ファイルパスを明示指定（--out より優先）
//! - WAV非指定時のみ: カレント（または--out）ディレクトリに <stem>.38x6 / <stem>.mid を出力
//! - `--wav`: <stem>.wav のみを出力する（.mid/.38x6は出さない排他モード。
//!   DAW/VST/SMFを経由せず ym38x6-core で直接レンダリング）
//! - `--out-wav <file>`: WAV出力パスを明示指定（同じく.wavのみ排他出力）
//! - `--gain <factor>`: WAV書き出し前の出力レベル倍率（クリップ対策、既定1.0）
//! - `--fm-gain <factor>` / `--ssg-gain <factor>`: 【OPN系専用】FM/SSG各パートの音量倍率
//!   （既定1.0）。FMとSSGの音量バランス調整用。0.5で半分、>1は上げ（velocity頭打ちで約1.27倍が上限）。
//!   WAV/SMF両出力に反映する。
//! - `--dump-pitch`: ピッチ診断CSVを標準出力へ出す（OPM/OPN共通スキーマ。音程デバッグ用）。
//!   freq_hz/midi_f/note/pb_cents/alg/carrier_muls を両チップ共通で出し、OPMは kc/kf/ref_midi/error_cents、
//!   OPNは fnum/block を追加列として埋める。
//! - `--fb-max <f>`【実験用】: feedback_to_scale の上限を上書き（既定1.8）。高FB音色の音程破綻検証用。
//! - `--fb-2sample`: feedback帰還を2サンプル平均に明示指定（既定なので通常不要）。
//! - `--fb-1sample`【診断用】: feedback帰還を旧1サンプルに戻す（既定=2サンプル平均とのA/B比較用）。
//! - `--only-ch <n>`【実験用】: 指定エンジンチャンネルのみ発音（FM 0..fm、SSG fm..fm+3）。音程ズレの単一ch分離用。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vgm2x6::opm::{
    compute_pitch_bend, kc_kf_to_ref_midi, kc_to_midi_note, pb_to_semitones,
    OpmState, PB_SENSITIVITY,
};
use vgm2x6::opn::{self, OpnState};
use vgm2x6::patch::PatchBank;
use vgm2x6::play::{
    self, detect_chip, freq_pitch_bend, OpnInfo, OpnWriteKind, SmfSink, SourceChip, WavSink,
};
use vgm2x6::ssg::{self, SsgState};
use vgm2x6::vgm::{self, VgmCmd, VgmIter};
use ym38x6_core::{algorithm, Ym38x6Patch};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vgm2x6: {msg}");
            eprintln!(
                "usage: vgm2x6 <input.vgz|.vgm> [--out <dir>] \
                 [--out-bank <file>] [--out-midi <file>] [--bank N] [--dump-pitch] [--out-wav <file>] [--wav] [--gain <factor>] [--fm-gain <factor>] [--ssg-gain <factor>] [--fb-max <f>] [--fb-2sample] [--fb-1sample] [--only-ch <n>]"
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
    /// FMパートの音量ゲイン（線形、1.0=従来）。OPNのFM/SSGバランス調整用。
    fm_gain: f32,
    /// SSGパートの音量ゲイン（線形、1.0=従来）。OPNのFM/SSGバランス調整用。
    ssg_gain: f32,
    /// 【実験用】feedback_to_scale の最大値を上書き（None=既定1.8）。高FB音色の音程破綻検証用。
    fb_max: Option<f32>,
    /// 2サンプル平均帰還を明示指定（既定で有効なため通常は不要、A/B比較用）。
    fb_2sample: bool,
    /// 【診断用】旧1サンプル帰還に戻す（既定=2サンプル平均とのA/B比較用）。
    fb_1sample: bool,
    /// 【実験用】指定したエンジンチャンネルのみ発音する（FM 0..fm、SSG fm..fm+3）。
    /// 音程ズレを単一チャンネルに分離して測定するためのデバッグフラグ。
    only_ch: Option<usize>,
}

/// 音量ゲイン係数(線形)を基準velocity 100 に適用する。0.5→50、2.0→127(頭打ち)。
/// FM音色は velocity_sensitivity=0 のため、velocityスケールは明るさを変えず音量のみに効く。
fn scaled_velocity(gain: f32) -> u8 {
    (100.0 * gain).round().clamp(1.0, 127.0) as u8
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut out_bank: Option<PathBuf> = None;
    let mut out_midi: Option<PathBuf> = None;
    let mut bank: u16 = 0;
    let mut dump_pitch = false;
    let mut fm_gain: f32 = 1.0;
    let mut ssg_gain: f32 = 1.0;
    let mut out_wav: Option<PathBuf> = None;
    let mut wav_flag = false;
    let mut gain: f32 = 1.0;
    let mut fb_max: Option<f32> = None;
    let mut fb_2sample = false;
    let mut fb_1sample = false;
    let mut only_ch: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump-pitch" => {
                dump_pitch = true;
                i += 1;
            }
            "--fb-max" => {
                let v = args.get(i + 1).ok_or("--fb-max に値がありません")?;
                fb_max = Some(v.parse().map_err(|_| format!("--fb-max の値が不正: {v}"))?);
                i += 2;
            }
            "--fb-2sample" => {
                fb_2sample = true;
                i += 1;
            }
            "--fb-1sample" => {
                fb_1sample = true;
                i += 1;
            }
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
    // WAVは要求時のみ出力する。--out-wav（明示パス）が最優先、なければ--wavフラグで
    // base_dir/<stem>.wav。どちらも無ければNone（WAV出力なし）。
    let wav = out_wav.or_else(|| {
        wav_flag.then(|| base_dir.join(format!("{stem}.wav")))
    });

    Ok(Args { input, out_bank, out_midi, bank, dump_pitch, wav, gain, fm_gain, ssg_gain, fb_max, fb_2sample, fb_1sample, only_ch })
}

/// `--dump-pitch` の1行分。OPM/OPN共通スキーマ。
///
/// チップ非依存の「実際に鳴るピッチ」列（freq_hz/midi_f/note/pb_cents/alg/carrier_muls）を
/// 主役に、OPM固有（kc/kf/ref_midi/error_cents）とOPN固有（fnum/block）を Option で持つ。
/// 空欄はCSVで空文字になる（pandas等でそのまま読める superset スキーマ）。
struct PitchRow {
    t: f64,
    chip: &'static str,
    ch: usize,
    event: &'static str,
    /// OPM: KCレジスタ値（0-127）。OPNでは None。
    kc: Option<u8>,
    /// OPM: KF（0-63、上位6bit）。OPNでは None。
    kf: Option<u8>,
    /// OPN: F-Number（11bit）。OPMでは None。
    fnum: Option<u16>,
    /// OPN: Block（0-7）。OPMでは None。
    block: Option<u8>,
    /// 実際に鳴る周波数（Hz）。両チップ共通。
    freq_hz: f32,
    /// 浮動小数MIDIノート（A4=69.0）。両チップ共通。
    midi_f: f32,
    /// エンジンへ送るMIDIノート番号（整数）。両チップ共通。
    note: u8,
    /// エンジンへ送るピッチベンド量（セント）。両チップ共通。
    pb_cents: f32,
    /// 変換後パッチのアルゴリズム（0-7）。両チップ共通。
    alg: u8,
    /// 変換後パッチのキャリアMULを "+" 連結（残差ピッチ/オクターブ知覚の診断用）。両チップ共通。
    carrier_muls: String,
    /// OPM: 実機YM2151 canonical変換による真値MIDI。OPNでは None（直接式のため比較対象なし）。
    ref_midi: Option<f32>,
    /// OPM: (実際に鳴るmidi_f - ref_midi) のセント差。±100の倍数ならノート表欠番ズレ。OPNでは None。
    error_cents: Option<f32>,
}

impl PitchRow {
    fn header() -> &'static str {
        "time_s,chip,ch,event,kc,kf,fnum,block,freq_hz,midi_f,note,pb_cents,alg,carrier_muls,ref_midi,error_cents"
    }
    fn print(&self) {
        fn os<T: std::fmt::Display>(v: &Option<T>) -> String {
            v.as_ref().map(|x| x.to_string()).unwrap_or_default()
        }
        fn of3(v: &Option<f32>) -> String {
            v.map(|x| format!("{x:.3}")).unwrap_or_default()
        }
        fn of1(v: &Option<f32>) -> String {
            v.map(|x| format!("{x:.1}")).unwrap_or_default()
        }
        println!(
            "{:.4},{},{},{},{},{},{},{},{:.3},{:.3},{},{:.1},{},{},{},{}",
            self.t, self.chip, self.ch, self.event,
            os(&self.kc), os(&self.kf), os(&self.fnum), os(&self.block),
            self.freq_hz, self.midi_f, self.note, self.pb_cents,
            self.alg, self.carrier_muls,
            of3(&self.ref_midi), of1(&self.error_cents),
        );
    }
}

/// 変換後パッチからアルゴリズムとキャリアMUL列（"2+1"等）を取り出す。OPM/OPN共通。
/// 残差ピッチ・オクターブ知覚（キャリアMUL≠1で基音が省略され得る）の診断に使う。
fn alg_carrier_muls(patch: &Ym38x6Patch) -> (u8, String) {
    let alg = patch.channel.algorithm.min(7);
    let carriers = algorithm::ALGORITHMS[alg as usize].carriers;
    let s = carriers
        .iter()
        .map(|&i| patch.operators[i].mul.to_string())
        .collect::<Vec<_>>()
        .join("+");
    (alg, s)
}

#[inline]
fn midi_to_freq_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

/// ピッチ診断ダンプ（--dump-pitch、OPM/OPN共通）。
///
/// VGMを走査し、音程イベントごとに「変換器＋エンジンが実際に鳴らすピッチ」を
/// [PitchRow] の共通スキーマでCSV出力する。チップ判定は呼び出し側（[run]）で済んでいる。
fn dump_pitch(data: &[u8], data_start: usize, chip: &SourceChip) -> Result<(), String> {
    println!("{}", PitchRow::header());
    match chip {
        SourceChip::Opm => dump_pitch_opm(data, data_start),
        SourceChip::Opn(info) => dump_pitch_opn(data, data_start, *info),
    }
    Ok(())
}

/// OPM（YM2151）のピッチイベントを [PitchRow] 共通スキーマで出力する。
fn dump_pitch_opm(data: &[u8], data_start: usize) {
    const SAMPLE_RATE: f64 = 44_100.0;
    let mut opm = OpmState::new();
    let mut samples: u64 = 0;
    let mut active: [bool; 8] = [false; 8];
    let mut base_kc: [u8; 8] = [0; 8];

    // 1イベント分の PitchRow を組み立てて出力する。
    // `slot_byte` は carrier_muls 用（algorithm/MUL のみ参照、スロットマスクには非依存）。
    let emit = |opm: &OpmState, samples: u64, ch: usize, event: &'static str, kc: u8, kf: u8, base: u8, pb: i16| {
        let midi_f = kc_to_midi_note(base) as f32 + pb_to_semitones(pb);
        let refm = kc_kf_to_ref_midi(kc, kf);
        let (alg, cmuls) = alg_carrier_muls(&opm2x6::conv::voice_to_patch(
            &opm.build_voice(ch, 0x78),
            opm2x6::parse::OperatorOrder::Direct,
        ));
        PitchRow {
            t: samples as f64 / SAMPLE_RATE,
            chip: "OPM",
            ch,
            event,
            kc: Some(kc),
            kf: Some((kf >> 2) & 0x3F),
            fnum: None,
            block: None,
            freq_hz: midi_to_freq_hz(midi_f),
            midi_f,
            note: kc_to_midi_note(base),
            pb_cents: pb_to_semitones(pb) * 100.0,
            alg,
            carrier_muls: cmuls,
            ref_midi: Some(refm),
            error_cents: Some((midi_f - refm) * 100.0),
        }
        .print();
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
                        emit(&opm, samples, ch, "on", kc, kf, kc, pb);
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
                            emit(&opm, samples, ch, "kc-reon", val, kf, val, pb2);
                        } else {
                            emit(&opm, samples, ch, "kc", val, kf, base_kc[ch], pb);
                        }
                    }
                }
                0x30..=0x37 => {
                    let ch = (reg - 0x30) as usize;
                    opm.write(reg, val);
                    if active[ch] {
                        let kc = opm.kc(ch);
                        let (pb, _) = compute_pitch_bend(kc, base_kc[ch], val);
                        emit(&opm, samples, ch, "kf", kc, val, base_kc[ch], pb);
                    }
                }
                _ => opm.write(reg, val),
            },
            VgmCmd::End => break,
            // OPN系コマンドはOPMパスでは発生しない（チップ判定で分岐済み）
            _ => {}
        }
    }
}

/// OPN系（YM2612/YM2203/YM2608）のピッチイベントを [PitchRow] 共通スキーマで出力する。
fn dump_pitch_opn(data: &[u8], data_start: usize, info: OpnInfo) {
    const SAMPLE_RATE: f64 = 44_100.0;
    let mut opn = OpnState::new();
    let mut ssg = SsgState::new();
    let ssg_clock = info.clock / info.ssg_clock_div;
    let mut samples: u64 = 0;
    let fm = info.fm_channels;

    let mut fm_note: Vec<Option<u8>> = vec![None; fm];
    let mut fm_base: Vec<f32> = vec![0.0; fm];

    for cmd in VgmIter::new(data, data_start) {
        let (port, reg, val) = match cmd {
            VgmCmd::Wait(n) => { samples += n as u64; continue; }
            VgmCmd::End => break,
            VgmCmd::Ym2612Write { port, reg, val } if info.write_kind == OpnWriteKind::Ym2612 => (port as usize, reg, val),
            VgmCmd::Ym2203Write { reg, val } if info.write_kind == OpnWriteKind::Ym2203 => (0, reg, val),
            VgmCmd::Ym2608Write { port, reg, val } if info.write_kind == OpnWriteKind::Ym2608 => (port as usize, reg, val),
            _ => continue,
        };

        let t = samples as f64 / SAMPLE_RATE;

        // --- SSG トーンピッチ（port0 の 0x00-0x05 = トーン周期lo/hi書き込み） ---
        // SSGはハードウェアPMSを持たないため、ピッチ変動は全てドライバのソフトビブラート
        // （周期レジスタの逐次書き換え）。各周期書き込み時の絶対ピッチを出力し、
        // 周期トラジェクトリが滑らかなビブラートかアーティファクトかを判別できるようにする。
        // chip="SSG", fnum列=トーン周期, note/pb_cents=絶対ピッチ(round+小数セント)。
        if info.has_ssg && port == 0 && reg < 0x06 {
            ssg.write(reg, val);
            let sc = (reg / 2) as usize;
            let period = ssg.tone_period(sc);
            let freq = ssg::period_to_freq(period, ssg_clock);
            if freq > 0.0 {
                let midi_f = opn::freq_to_midi(freq);
                let note = midi_f.round().clamp(0.0, 127.0) as u8;
                let pb_cents = (midi_f - note as f32) * 100.0;
                PitchRow {
                    t, chip: "SSG", ch: fm + sc, event: "ssg",
                    kc: None, kf: None, fnum: Some(period), block: None,
                    freq_hz: freq, midi_f, note, pb_cents,
                    alg: 0, carrier_muls: "psg".to_string(),
                    ref_midi: None, error_cents: None,
                }
                .print();
            }
            continue;
        }

        opn.write(port, reg, val);

        // 共通スキーマの PitchRow を組み立てて出力するヘルパー。
        let row = |ch: usize, event: &'static str, fnum: u16, block: u8, freq: f32, midi_f: f32, note: u8, pb_cents: f32, patch: &Ym38x6Patch| {
            let (alg, cmuls) = alg_carrier_muls(patch);
            PitchRow {
                t, chip: info.label_short(), ch, event,
                kc: None, kf: None, fnum: Some(fnum), block: Some(block),
                freq_hz: freq, midi_f, note, pb_cents, alg, carrier_muls: cmuls,
                ref_midi: None, error_cents: None,
            }
            .print();
        };

        match reg {
            0x28 if port == 0 => {
                if let Some((ch, slots)) = opn::decode_keyon(val) {
                    if ch < fm {
                        if slots != 0 {
                            let patch = opn.build_voice(ch).to_ym38x6_patch();
                            let (fnum, block) = opn.fnum_block(ch);
                            let freq = opn::fnum_block_to_freq(fnum, block, info.clock, info.fm_divisor);
                            let midi_f = opn::freq_to_midi(freq);
                            let note = midi_f.round().clamp(0.0, 127.0) as u8;
                            let (pb, _) = freq_pitch_bend(midi_f, note as f32);
                            let pb_cents = (pb as f32 - 8192.0) / 8192.0 * (PB_SENSITIVITY as f32) * 100.0;
                            row(ch, "on", fnum, block, freq, midi_f, note, pb_cents, &patch);
                            fm_note[ch] = Some(note);
                            fm_base[ch] = note as f32;
                        } else if fm_note[ch].take().is_some() {
                            // ノートオフは共通スキーマで note=直前値・freq情報なしを表現できないため、
                            // event="off" の行を最小限（pitch列は0埋め）で出す。
                            PitchRow {
                                t, chip: info.label_short(), ch, event: "off",
                                kc: None, kf: None, fnum: None, block: None,
                                freq_hz: 0.0, midi_f: 0.0, note: 0, pb_cents: 0.0,
                                alg: 0, carrier_muls: String::new(),
                                ref_midi: None, error_cents: None,
                            }
                            .print();
                        }
                    }
                }
            }
            // 0xA0 のみ（shadow register仕様）: ピッチ確定イベントとして出力。
            0xA0..=0xA2 => {
                let cip = (reg - 0xA0) as usize;
                let ch = port * 3 + cip;
                if ch < fm && fm_note[ch].is_some() {
                    let patch = opn.build_voice(ch).to_ym38x6_patch();
                    let (fnum, block) = opn.fnum_block(ch);
                    let freq = opn::fnum_block_to_freq(fnum, block, info.clock, info.fm_divisor);
                    let midi_f = opn::freq_to_midi(freq);
                    let (pb, oor) = freq_pitch_bend(midi_f, fm_base[ch]);
                    let pb_cents = (pb as f32 - 8192.0) / 8192.0 * (PB_SENSITIVITY as f32) * 100.0;
                    let event = if oor { "retrig" } else { "pitch" };
                    row(ch, event, fnum, block, freq, midi_f, fm_note[ch].unwrap(), pb_cents, &patch);
                    if oor {
                        let note = midi_f.round().clamp(0.0, 127.0) as u8;
                        fm_base[ch] = note as f32;
                        fm_note[ch] = Some(note);
                    }
                }
            }
            _ => {}
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    // 1. VGZ/VGM 読み込み
    let data = vgm::load(&args.input)?;
    let header = vgm::parse_header(&data)?;

    // 2. 音源チップ判定（YM2151=従来パス / OPN系=新パス）
    let chip = detect_chip(&header).ok_or(
        "YM2151 も OPN系(YM2612/YM2203/YM2608) も見つかりません（対応外のVGMです）",
    )?;

    match chip {
        SourceChip::Opm => {
            eprintln!(
                "YM2151 clock: {} Hz, {} samples",
                header.ym2151_clock, header.total_samples
            );
            // --dump-pitch: SMF/バンクを書かず、ピッチ診断CSV（OPM/OPN共通スキーマ）を標準出力へ出す
            if args.dump_pitch {
                return dump_pitch(&data, header.data_start, &SourceChip::Opm);
            }
            // --wav/--out-wav: WAVのみを排他出力する（.mid/.38x6は出さない）。
            if let Some(wav_path) = &args.wav {
                return render_wav(&data, header.data_start, wav_path, args.gain);
            }
            run_opm_smf(&data, &header, &args)
        }
        SourceChip::Opn(info) => {
            eprintln!(
                "{} clock: {} Hz, {} samples (FM {}ch{})",
                info.label, info.clock, header.total_samples, info.fm_channels,
                if info.has_ssg { " + SSG 3ch" } else { "" },
            );
            if args.dump_pitch {
                return dump_pitch(&data, header.data_start, &SourceChip::Opn(info));
            }
            run_opn(&data, header.data_start, info, &args)
        }
    }
}

/// YM2151(OPM) の直接WAVレンダリング（--wav）。演奏ループは [play::opm_to_audio]、
/// 尾レンダリング＋書き出しは [play::finish_wav] へ委譲する。
fn render_wav(data: &[u8], data_start: usize, out: &Path, gain: f32) -> Result<(), String> {
    const SR: f32 = 44_100.0;
    let mut engine = ym38x6_core::Ym38x6Engine::new(SR);
    let audio = play::opm_to_audio(data, data_start, &mut engine);
    play::finish_wav(&mut engine, audio, SR, out, gain)
}

/// YM2151(OPM) の SMF + 音色バンクを出力する（従来パス）。演奏ループは [play::opm_to_smf] へ委譲する。
fn run_opm_smf(data: &[u8], header: &vgm::VgmHeader, args: &Args) -> Result<(), String> {
    let mut bank = PatchBank::new();
    let mut smf = play::opm_to_smf(data, header.data_start, &mut bank);

    bank.write(&args.out_bank, args.bank)?;
    println!("音色バンク: {} ({} 音色)", args.out_bank.display(), bank.len());

    smf.write(&args.out_midi)
        .map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
    println!("MIDI: {}", args.out_midi.display());

    Ok(())
}

/// OPN系の出力（SMF+バンク または WAV）。演奏ループは [play::process_opn] へ委譲する。
fn run_opn(data: &[u8], data_start: usize, info: OpnInfo, args: &Args) -> Result<(), String> {
    let total_ch = info.fm_channels + if info.has_ssg { 3 } else { 0 };
    let mut bank = PatchBank::new();

    let fm_vel = scaled_velocity(args.fm_gain);
    let ssg_vel = scaled_velocity(args.ssg_gain);

    // フィードバック帰還方式の上書き。WAVレンダリング前にプロセスグローバルへ設定。
    // 既定は2サンプル平均（実機準拠）なので --fb-2sample は明示指定用（通常不要）。
    if args.fb_2sample {
        ym38x6_core::set_feedback_two_sample(true);
        eprintln!("feedback帰還を2サンプル平均 (out[n-1]+out[n-2])/2 に設定（既定と同じ）");
    }
    if args.fb_1sample {
        ym38x6_core::set_feedback_two_sample(false);
        eprintln!("診断: feedback帰還を旧1サンプル (out[n-1]) に切替（既定=2サンプル平均とのA/B用）");
    }
    if let Some(m) = args.fb_max {
        ym38x6_core::set_feedback_scale_max(Some(m));
        eprintln!("実験: feedback_to_scale 最大値を {m} に上書き（既定1.8）");
    }
    if let Some(c) = args.only_ch {
        eprintln!("実験: エンジンチャンネル {c} のみ発音");
    }

    if let Some(wav_path) = &args.wav {
        let mut sink = WavSink::new(44_100.0, total_ch);
        play::process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);
        return sink.finish_and_write(wav_path, args.gain);
    }

    let mut sink = SmfSink::new(total_ch);
    play::process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);

    bank.write(&args.out_bank, args.bank)?;
    println!("音色バンク: {} ({} 音色)", args.out_bank.display(), bank.len());
    sink.smf
        .write(&args.out_midi)
        .map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
    println!("MIDI: {}", args.out_midi.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_velocity_maps_gain_to_velocity() {
        assert_eq!(scaled_velocity(1.0), 100); // 従来どおり
        assert_eq!(scaled_velocity(0.5), 50); // 半分
        assert_eq!(scaled_velocity(0.0), 1); // 下限クランプ（0ではなく1）
        assert_eq!(scaled_velocity(2.0), 127); // 上限クランプ（velocity頭打ち）
    }
}
