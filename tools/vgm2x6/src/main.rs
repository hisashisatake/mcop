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
//! - `--fb-2sample`【実験用】: feedback帰還を2サンプル平均に切替（既定は1サンプル帰還）。
//! - `--only-ch <n>`【実験用】: 指定エンジンチャンネルのみ発音（FM 0..fm、SSG fm..fm+3）。音程ズレの単一ch分離用。

mod opm;
mod opn;
mod patch;
mod smf;
mod ssg;
mod vgm;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use opm::{
    compute_pitch_bend, kc_kf_to_ref_midi, kc_to_midi_note, pb_to_semitones,
    OpmState, PB_SENSITIVITY,
};
use opn::OpnState;
use patch::PatchBank;
use smf::SmfBuilder;
use ssg::SsgState;
use vgm::{VgmCmd, VgmIter};
use ym38x6_core::{algorithm, SoundEngine, Ym38x6Patch};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vgm2x6: {msg}");
            eprintln!(
                "usage: vgm2x6 <input.vgz|.vgm> [--out <dir>] \
                 [--out-bank <file>] [--out-midi <file>] [--bank N] [--dump-pitch] [--out-wav <file>] [--wav] [--gain <factor>] [--fm-gain <factor>] [--ssg-gain <factor>] [--fb-max <f>] [--fb-2sample] [--only-ch <n>]"
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
    /// 【実験用】2サンプル平均帰還に切替えるか（既定false=1サンプル帰還）。
    fb_2sample: bool,
    /// 【実験用】指定したエンジンチャンネルのみ発音する（FM 0..fm、SSG fm..fm+3）。
    /// 音程ズレを単一チャンネルに分離して測定するためのデバッグフラグ。
    only_ch: Option<usize>,
}

/// 音量ゲイン係数(線形)を基準velocity 100 に適用する。0.5→50、2.0→127(頭打ち)。
/// FM音色は velocity_sensitivity=0 のため、velocityスケールは明るさを変えず音量のみに効く。
fn scaled_velocity(gain: f32) -> u8 {
    (100.0 * gain).round().clamp(1.0, 127.0) as u8
}

/// PSG風パッチ。波形メモリ音色の矩形波(waveform=16)を、アタック最速・減衰なし・
/// 最大サスティン・最速リリースのADSRで鳴らす。キーオン中は一定音量の矩形波、
/// キーオフで即停止という、PSG(SSG/SN76489系)のトーン1チャンネルに近い挙動になる。
/// release=255(最速≒8.71ms)はエンジンの最短値。release=0だとrr_to_delta(0)=284.9秒となり
/// SMFパス(異note=別VoiceID)では note_off後も延々と鳴り続けてしまう。
fn psg_patch() -> ym38x6_core::Ym38x6Patch {
    // 波形番号: sine=0 / saw=8 / square=16 / triangle=24（preset.rs の基本4波形に準拠）
    ym38x6_core::waveform_memory_patch(
        16,
        ym38x6_core::AdsrParams { attack: 255, decay: 0, sustain: 255, release: 255 },
    )
}

/// ノイズ専用OperatorParams（波形番号32+NP）。アタック最速・減衰なし・最大サスティン・最速リリース。
/// エンジンが17bit LFSRをNP由来クロックレートでサンプル&ホールドしてノイズを生成する。
fn noise_op(np: u8) -> ym38x6_core::OperatorParams {
    ym38x6_core::OperatorParams {
        tl: 255,
        ar: 255,
        d1r: 0,
        d2r: 0,
        d1l: 255,
        rr: 255, // rr=0は284.9秒リリースになるためSMFパスで鳴りっぱなしになる。最速(≒8.71ms)を使う。
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 32 + np.min(31), // 32〜63 = ノイズ（color=NP）
        op_fine_tune: 128,
    }
}

/// SSGノイズのみ用パッチ。ノイズOPを OP1 で鳴らす（Algorithm 7・OP2〜4ミュート）。
/// ノイズはピッチレスなので note 周波数には依存せず、NPで帯域（白〜低域）が決まる。
fn noise_patch(np: u8) -> ym38x6_core::Ym38x6Patch {
    use ym38x6_core::Ym38x6Patch;
    let mut patch = Ym38x6Patch::default();
    patch.channel.algorithm = 7;
    let op1 = noise_op(np);
    patch.operators[0] = op1;
    let muted = ym38x6_core::OperatorParams { tl: 0, ..op1 };
    patch.operators[1] = muted;
    patch.operators[2] = muted;
    patch.operators[3] = muted;
    patch
}

/// トーン+ノイズ混合パッチ（実機SSGのトーン・ノイズ同時出力）。
/// Algorithm 7 で OP1=矩形波（トーン、note周波数で鳴る）+ OP3=ノイズ（NPの帯域）の加算混合。
/// 実機の「トーンANDノイズ（ゲート）」ではなく加算近似だが、両方が同時に聞こえる。
fn mix_patch(np: u8) -> ym38x6_core::Ym38x6Patch {
    use ym38x6_core::{OperatorParams, Ym38x6Patch};
    let mut patch = Ym38x6Patch::default();
    patch.channel.algorithm = 7;
    // OP1: 矩形波（トーン）。psg_patch と同じ素直な矩形オシレーター。
    let tone = OperatorParams {
        tl: 255,
        ar: 255,
        d1r: 0,
        d2r: 0,
        d1l: 255,
        rr: 0,
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 16, // 矩形波（square variant0）
        op_fine_tune: 128,
    };
    patch.operators[0] = tone;
    patch.operators[1] = OperatorParams { tl: 0, ..tone };
    patch.operators[2] = noise_op(np); // OP3: ノイズ
    patch.operators[3] = OperatorParams { tl: 0, ..tone };
    patch
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

    Ok(Args { input, out_bank, out_midi, bank, dump_pitch, wav, gain, fm_gain, ssg_gain, fb_max, fb_2sample, only_ch })
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

/// 直接WAVレンダリング（--wav）。
///
/// VGMを走査し、run()と同じ音程ロジック（ノートオン/オフ・KC/KF→ピッチベンド・
/// プログラムチェンジ）で ym38x6-core を駆動し、演奏をそのままWAV（mono 44.1kHz 16bit）へ
/// 書き出す。DAW/VST/SMFを経由しないため、ピッチベンド感度RPNやホストのMIDI処理に依存しない
/// 「変換器＋エンジンが本来出すべき音」が得られる。
///
/// `gain`は書き出し前に全サンプルへ掛ける出力レベル倍率（クリップ対策。1.0で素通し）。
fn render_wav(data: &[u8], data_start: usize, out: &Path, gain: f32) -> Result<(), String> {
    const SR: f32 = 44_100.0;

    let mut opm = OpmState::new();
    let mut bank = PatchBank::new();
    let mut engine = ym38x6_core::Ym38x6Engine::new(SR);
    let mut audio: Vec<f32> = Vec::new();

    let mut active:   [bool; 8] = [false; 8];
    let mut base_kc:  [u8;   8] = [0;     8];
    // 各チャンネルが現在鳴らしているパッチ（kc-reon時の再キーオンに再利用）。
    // PSGモードの後ろ3chは矩形波の固定パッチ、それ以外はOPM抽出音色。
    let mut cur_patch: [ym38x6_core::Ym38x6Patch; 8] = [Default::default(); 8];

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
                        let patch = bank.patch_at(idx);
                        cur_patch[ch] = patch;
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
                            engine.note_on_with_velocity(ch, midi_to_freq(note), 100, cur_patch[ch]);
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
            // OPN系コマンドはOPMパスでは発生しない（チップ判定で分岐済み）
            _ => {}
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

/// YM2151(OPM) の SMF + 音色バンクを出力する（従来パス）。
fn run_opm_smf(data: &[u8], header: &vgm::VgmHeader, args: &Args) -> Result<(), String> {
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
    for cmd in VgmIter::new(data, header.data_start) {
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
                            // Program Change (0xCn) + CC102 の両方を送ることで
                            // CLAP（Program Change）と VST3（CC102 = PC 代替）の両方に対応する。
                            // CC102 は GM2 未定義ブロック（102-119）の先頭。CC92 は GM2 で
                            // Effects 2 Depth（トレモロ）に予約されているため使わない。
                            if active_patch[ch] != Some(patch_idx) {
                                let prog = (patch_idx % 128) as u8;
                                smf.add_program_change(tr, tick, ch as u8, prog);
                                smf.add_cc(tr, tick, ch as u8, 102, prog);
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

            // OPN系コマンドはOPMパスでは発生しない（チップ判定で分岐済み）
            _ => {}
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

// ===========================================================================
// OPN系（YM2612 / YM2203 / YM2608）パス
// ===========================================================================

/// 入力VGMの音源チップ種別。
enum SourceChip {
    Opm,
    Opn(OpnInfo),
}

/// OPN書き込みコマンドの種類（どのVGMコマンドを横取りするか）。
#[derive(Clone, Copy, PartialEq)]
enum OpnWriteKind {
    Ym2612,
    Ym2203,
    Ym2608,
}

/// 検出したOPN系チップの構成。
#[derive(Clone, Copy)]
struct OpnInfo {
    label: &'static str,
    write_kind: OpnWriteKind,
    /// チップ入力クロック（Hz）。dual-chipビットはマスク済み。
    clock: u32,
    /// FMチャンネル数（YM2203=3、YM2612/YM2608=6）。
    fm_channels: usize,
    /// F-Number→周波数式の分母 factor（[opn::FM_DIVISOR_6CH] / [opn::FM_DIVISOR_3CH]）。
    fm_divisor: u32,
    /// SSGクロックの分周値（`ssg_clock = clock / ssg_clock_div`）。
    /// 6ch系(YM2608)はSSGプリスケーラも3ch系(YM2203)の2倍（YM2203=2 / YM2608=4）。
    /// この差を無視すると6ch系のSSGが1オクターブ高く変換される。
    ssg_clock_div: u32,
    /// SSG(PSG) を内蔵するか（YM2203/YM2608=true、YM2612=false）。
    has_ssg: bool,
}

impl OpnInfo {
    /// CSV出力用の短いチップ名（OPM/OPN2/OPN/OPNA）。
    fn label_short(&self) -> &'static str {
        match self.write_kind {
            OpnWriteKind::Ym2612 => "OPN2",
            OpnWriteKind::Ym2203 => "OPN",
            OpnWriteKind::Ym2608 => "OPNA",
        }
    }
}

/// VGMヘッダーのクロック欄から音源チップを判定する。YM2151を最優先、次にOPN系。
fn detect_chip(h: &vgm::VgmHeader) -> Option<SourceChip> {
    // dual-chipビット(bit30/31)を落としてクロック値だけ取り出す。
    let clk = |v: u32| v & 0x3FFF_FFFF;
    if h.ym2151_clock != 0 {
        return Some(SourceChip::Opm);
    }
    if h.ym2608_clock != 0 {
        return Some(SourceChip::Opn(OpnInfo {
            label: "YM2608(OPNA)",
            write_kind: OpnWriteKind::Ym2608,
            clock: clk(h.ym2608_clock),
            fm_channels: 6,
            fm_divisor: opn::FM_DIVISOR_6CH, // 6ch系=288
            ssg_clock_div: 4,                // OPNA SSG = master/4
            has_ssg: true,
        }));
    }
    if h.ym2203_clock != 0 {
        return Some(SourceChip::Opn(OpnInfo {
            label: "YM2203(OPN)",
            write_kind: OpnWriteKind::Ym2203,
            clock: clk(h.ym2203_clock),
            fm_channels: 3,
            fm_divisor: opn::FM_DIVISOR_3CH, // 3ch系=144（A=440実機検証済み）
            ssg_clock_div: 2,                // OPN SSG = master/2
            has_ssg: true,
        }));
    }
    if h.ym2612_clock != 0 {
        return Some(SourceChip::Opn(OpnInfo {
            label: "YM2612(OPN2)",
            write_kind: OpnWriteKind::Ym2612,
            clock: clk(h.ym2612_clock),
            fm_channels: 6,
            fm_divisor: opn::FM_DIVISOR_6CH, // 6ch系=288（YM2608類推、Genesis曲での実機検証は未実施）
            ssg_clock_div: 2,                // YM2612はSSG非搭載のため未使用
            has_ssg: false,
        }));
    }
    None
}

/// 現在ピッチ(浮動MIDI) と基準ノートから MIDI ピッチベンド(0-16383) と範囲外フラグを求める。
/// 感度は [PB_SENSITIVITY] 半音（OPMパスと共通）。
fn freq_pitch_bend(cur_midi: f32, base_midi: f32) -> (i16, bool) {
    let delta = cur_midi - base_midi;
    let oor = delta.abs() > PB_SENSITIVITY as f32;
    let bend = (8192.0 + delta / PB_SENSITIVITY as f32 * 8192.0)
        .round()
        .clamp(0.0, 16383.0) as i16;
    (bend, oor)
}

/// OPN変換の出力先抽象。FM/SSGの楽音イベントを SMF（[SmfSink]）または
/// WAV直描画（[WavSink]）へ振り分ける。`tick` はSMF用（WAVは無視）。
trait OpnSink {
    fn set_pitch_bend_sensitivity(&mut self, ch: usize, semitones: u8);
    /// ノートオン前に呼ぶ。SMFはPC+CC102、WAVはチャンネルのパッチを記憶。
    fn program_change(&mut self, tick: u64, ch: usize, program: u8, patch: Ym38x6Patch);
    fn note_on(&mut self, tick: u64, ch: usize, note: u8, freq: f32, vel: u8);
    fn note_off(&mut self, tick: u64, ch: usize, note: u8);
    /// `pb`=SMF用ベンド値, `cents`=WAV用セント。
    fn pitch_bend(&mut self, tick: u64, ch: usize, pb: i16, cents: f32);
    /// SSG音量(0-15)。SMFはCC11、WAVはキャリアOP1のTLへ反映。
    fn expression(&mut self, tick: u64, ch: usize, vol: u8);
    fn wait(&mut self, samples: u32);
}

/// SMF + 音色バンク出力用のシンク。
struct SmfSink {
    smf: SmfBuilder,
    last_program: Vec<Option<u8>>,
    last_cc11: Vec<i16>,
}

impl SmfSink {
    fn new(total_ch: usize) -> Self {
        Self {
            smf: SmfBuilder::new(),
            last_program: vec![None; total_ch],
            last_cc11: vec![-1; total_ch],
        }
    }
}

impl OpnSink for SmfSink {
    fn set_pitch_bend_sensitivity(&mut self, ch: usize, semitones: u8) {
        self.smf.set_pitch_bend_sensitivity(ch + 1, ch as u8, semitones);
    }
    fn program_change(&mut self, tick: u64, ch: usize, program: u8, _patch: Ym38x6Patch) {
        if self.last_program[ch] != Some(program) {
            self.smf.add_program_change(ch + 1, tick, ch as u8, program);
            self.smf.add_cc(ch + 1, tick, ch as u8, 102, program);
            self.last_program[ch] = Some(program);
        }
    }
    fn note_on(&mut self, tick: u64, ch: usize, note: u8, _freq: f32, vel: u8) {
        self.smf.add_note_on(ch + 1, tick, ch as u8, note, vel);
    }
    fn note_off(&mut self, tick: u64, ch: usize, note: u8) {
        self.smf.add_note_off(ch + 1, tick, ch as u8, note);
    }
    fn pitch_bend(&mut self, tick: u64, ch: usize, pb: i16, _cents: f32) {
        self.smf.add_pitch_bend(ch + 1, tick, ch as u8, pb);
    }
    fn expression(&mut self, tick: u64, ch: usize, vol: u8) {
        let cc = ssg::volume_to_cc11(vol) as i16;
        if cc != self.last_cc11[ch] {
            self.smf.add_cc(ch + 1, tick, ch as u8, 11, cc as u8);
            self.last_cc11[ch] = cc;
        }
    }
    fn wait(&mut self, _samples: u32) {}
}

/// WAV直描画用のシンク。`tick`は無視し、エンジンを直接駆動する。
struct WavSink {
    engine: ym38x6_core::Ym38x6Engine,
    audio: Vec<f32>,
    patches: Vec<Ym38x6Patch>,
    sr: f32,
    /// 各チャンネルの現在のピッチベンド量（セント）。note_on後に再適用するため保持する。
    /// SMFパスと同様に「整数ノート周波数 + ベンド」で発音するための土台
    /// （note_onに正確な周波数を渡すとベンド基準が二重計上され2音目以降がズレる）。
    bend_cents: Vec<f32>,
    /// チャンネルごとの現在のチャンネルゲイン（CC11/expression由来）。
    /// note_on時に新ボイスへ再適用するため保持する（Channel::newはgain=1.0で初期化するため）。
    ch_gain: Vec<f32>,
}

impl WavSink {
    fn new(sr: f32, total_ch: usize) -> Self {
        Self {
            engine: ym38x6_core::Ym38x6Engine::new(sr),
            audio: Vec::new(),
            patches: vec![Ym38x6Patch::default(); total_ch],
            sr,
            bend_cents: vec![0.0; total_ch],
            ch_gain: vec![1.0f32; total_ch],
        }
    }

    /// リリースの尾を1秒分レンダリングし、gain適用＋クリップ報告してWAVを書き出す。
    fn finish_and_write(mut self, out: &Path, gain: f32) -> Result<(), String> {
        let mut tail = vec![0.0f32; self.sr as usize];
        self.engine.render(&mut tail, 1);
        self.audio.extend_from_slice(&tail);

        let raw_peak = self.audio.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        if gain != 1.0 {
            for s in self.audio.iter_mut() {
                *s *= gain;
            }
        }
        let out_peak = raw_peak * gain;
        let clipped = self.audio.iter().filter(|&&s| s.abs() > 1.0).count();
        write_wav_mono16(out, &self.audio, self.sr as u32)
            .map_err(|e| format!("WAV書き込み失敗: {}: {e}", out.display()))?;
        println!(
            "WAV: {} ({:.1}秒, gain={:.2}, 素ピーク={:.3}, 出力ピーク={:.3}{})",
            out.display(),
            self.audio.len() as f32 / self.sr,
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
}

impl OpnSink for WavSink {
    fn set_pitch_bend_sensitivity(&mut self, _ch: usize, _semitones: u8) {}
    fn program_change(&mut self, _tick: u64, ch: usize, _program: u8, patch: Ym38x6Patch) {
        self.patches[ch] = patch;
    }
    fn note_on(&mut self, _tick: u64, ch: usize, note: u8, _freq: f32, vel: u8) {
        // SMFパスと同じく「整数ノート周波数」で発音し、端数はベンドで乗せる。
        // 正確な周波数(freq)を渡すと、その後のベンド(整数ノート基準のセント)が
        // 二重計上され、2音目以降が初音の端数ぶんズレる（ノイズはピッチ無視のため影響なし）。
        let f = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
        self.engine.note_on_with_velocity(ch, f, vel, self.patches[ch]);
        // Channel::newはgain=1.0で初期化するため、現在のch_gainを再適用する。
        self.engine.set_channel_volume(ch, self.ch_gain[ch]);
        self.engine.set_pitch_bend(ch, self.bend_cents[ch]);
    }
    fn note_off(&mut self, _tick: u64, ch: usize, _note: u8) {
        self.engine.note_off(ch);
    }
    fn pitch_bend(&mut self, _tick: u64, ch: usize, _pb: i16, cents: f32) {
        self.bend_cents[ch] = cents;
        self.engine.set_pitch_bend(ch, cents);
    }
    fn expression(&mut self, _tick: u64, ch: usize, vol: u8) {
        // SSG音量(0-15)をチャンネルゲインへ変換してエンジンへ反映する。
        // VST側CC11と同じ二乗カーブ(vol/15)^2を使うことで、TL直操作より実機AY-3-8910に近い
        // 音量カーブになり、vol=7で約-13dBとなる（TL線形マッピングでは-51dBだった）。
        let gain = (vol as f32 / 15.0).powi(2);
        self.ch_gain[ch] = gain;
        self.engine.set_channel_volume(ch, gain);
    }
    fn wait(&mut self, samples: u32) {
        let mut buf = vec![0.0f32; samples as usize];
        self.engine.render(&mut buf, 1);
        self.audio.extend_from_slice(&buf);
    }
}

/// OPN系VGMを走査し、FM + SSG の楽音イベントを `sink` へ送る共通処理。
/// バンク（音色重複排除）は `bank` が保持する。
fn process_opn(
    data: &[u8],
    data_start: usize,
    info: OpnInfo,
    bank: &mut PatchBank,
    sink: &mut dyn OpnSink,
    fm_vel: u8,
    ssg_vel: u8,
    only_ch: Option<usize>,
) {
    let fm = info.fm_channels;
    let total_ch = fm + if info.has_ssg { 3 } else { 0 };
    let ssg_clock = info.clock / info.ssg_clock_div;
    // 単一チャンネル分離（--only-ch）用フィルター。Noneなら全チャンネル発音。
    let keep = |ech: usize| only_ch.map_or(true, |k| k == ech);

    for ch in 0..total_ch {
        sink.set_pitch_bend_sensitivity(ch, PB_SENSITIVITY);
    }

    let mut opn = OpnState::new();
    let mut ssg = SsgState::new();
    let mut samples: u64 = 0;

    // FM演奏状態
    let mut fm_note: Vec<Option<u8>> = vec![None; fm];
    let mut fm_base: Vec<f32> = vec![0.0; fm];
    let mut fm_pb: Vec<i16> = vec![8192; fm];

    // SSG演奏状態（3ch）
    let mut ssg_note: [Option<u8>; 3] = [None; 3];
    let mut ssg_base: [f32; 3] = [0.0; 3];
    let mut ssg_pb: [i16; 3] = [8192; 3];
    let mut ssg_vol: [u8; 3] = [255; 3]; // 直前のexpression音量（初期値は無効値）
    // 同一タイムステップで溜まったSSGレジスタ書き込みのうち、まだエンジンへ確定していない
    // チャンネルのdirtyフラグ。Wait直前にまとめてeval（確定）することで、トーン周期の
    // lo/hi分割書き込みが生む中間周期（旧hi<<8|新lo 等の幽霊ピッチ）がエンジンに届くのを防ぐ。
    // AY/SSGは0xA0のような確定ラッチを持たないため、FMの0xA4シャドウ方式は使えずコアレスで対応する。
    let mut ssg_dirty: [bool; 3] = [false; 3];

    for cmd in VgmIter::new(data, data_start) {
        let (port, reg, val) = match cmd {
            VgmCmd::Wait(n) => {
                // Wait（=オーディオ描画）直前に、溜まったSSG書き込みを最終状態で確定する。
                // ハードウェアエンベロープを n サンプル進め、エンベロープモード中のチャンネルを
                // dirty にして最新レベルを確実に反映させる（エンベロープは毎フレーム変化しうる）。
                ssg.tick_envelope(n, ssg_clock, 44_100);
                for sc in 0..3 {
                    if ssg.envelope_mode(sc) {
                        ssg_dirty[sc] = true;
                    }
                }
                let tick = SmfBuilder::samples_to_ticks(samples);
                for sc in 0..3 {
                    if ssg_dirty[sc] {
                        ssg_dirty[sc] = false;
                        if keep(fm + sc) {
                            eval_ssg_channel(
                                sc, fm, &ssg, ssg_clock, bank, tick, sink, ssg_vel,
                                &mut ssg_note[sc], &mut ssg_base[sc], &mut ssg_pb[sc], &mut ssg_vol[sc],
                            );
                        }
                    }
                }
                samples += n as u64;
                sink.wait(n);
                continue;
            }
            VgmCmd::End => break,
            VgmCmd::Ym2612Write { port, reg, val } if info.write_kind == OpnWriteKind::Ym2612 => {
                (port as usize, reg, val)
            }
            VgmCmd::Ym2203Write { reg, val } if info.write_kind == OpnWriteKind::Ym2203 => {
                (0usize, reg, val)
            }
            VgmCmd::Ym2608Write { port, reg, val } if info.write_kind == OpnWriteKind::Ym2608 => {
                (port as usize, reg, val)
            }
            _ => continue,
        };

        let tick = SmfBuilder::samples_to_ticks(samples);

        // --- SSG（port0 の 0x00-0x0F） ---
        // レジスタ状態だけ更新し、影響chをdirtyにする。実際のeval（エンジンへの確定）は
        // 次のWait直前にまとめて行う（中間周期の幽霊ピッチを排除＝コアレス）。
        if info.has_ssg && port == 0 && reg < 0x10 {
            ssg.write(reg, val);
            match reg {
                0x00 | 0x01 => ssg_dirty[0] = true,
                0x02 | 0x03 => ssg_dirty[1] = true,
                0x04 | 0x05 => ssg_dirty[2] = true,
                // noise period/mixer/envelope は全chへ影響
                0x06 | 0x07 | 0x0B | 0x0C | 0x0D => ssg_dirty = [true; 3],
                0x08 => ssg_dirty[0] = true,
                0x09 => ssg_dirty[1] = true,
                0x0A => ssg_dirty[2] = true,
                _ => {}
            }
            continue;
        }

        // --- FM ---
        opn.write(port, reg, val);
        match reg {
            // キーオン/オフ（port0のグローバルレジスタ）
            0x28 if port == 0 => {
                if let Some((ch, slots)) = opn::decode_keyon(val) {
                    if ch < fm && keep(ch) {
                        if slots != 0 {
                            // 既存ノートを解放してから新規キーオン
                            if let Some(prev) = fm_note[ch].take() {
                                sink.note_off(tick, ch, prev);
                            }
                            let voice = opn.build_voice(ch);
                            let patch = voice.to_ym38x6_patch();
                            let idx = bank.find_or_insert_fixed(patch, "opn_voice");
                            let (fnum, block) = opn.fnum_block(ch);
                            let freq = opn::fnum_block_to_freq(fnum, block, info.clock, info.fm_divisor);
                            let midi_f = opn::freq_to_midi(freq);
                            let note = midi_f.round().clamp(0.0, 127.0) as u8;
                            fm_base[ch] = note as f32;
                            let (pb, _) = freq_pitch_bend(midi_f, note as f32);
                            sink.program_change(tick, ch, (idx % 128) as u8, patch);
                            sink.pitch_bend(tick, ch, pb, (midi_f - note as f32) * 100.0);
                            sink.note_on(tick, ch, note, freq, fm_vel);
                            fm_note[ch] = Some(note);
                            fm_pb[ch] = pb;
                        } else if let Some(prev) = fm_note[ch].take() {
                            sink.note_off(tick, ch, prev);
                        }
                    }
                }
            }
            // F-Number ロウバイト（0xA0）書き込み → ピッチ確定・ベンド更新。
            //
            // OPN実機では 0xA4（Block + FNUM高3bit）はシャドウレジスタに格納されるだけで
            // チャンネル周波数は変化しない。0xA0（FNUM低8bit）を書いた瞬間に両方が同時
            // 適用されて周波数が確定する。
            // 0xA4書き込み時にピッチ計算すると「新hiバイト + 旧loバイト」の不正周波数で
            // 誤ピッチベンドや誤OORリトリガーが発生するため、0xA0のみで更新する。
            0xA0..=0xA2 => {
                let cip = (reg - 0xA0) as usize;
                let ch = port * 3 + cip;
                if ch < fm && keep(ch) && fm_note[ch].is_some() {
                    let (fnum, block) = opn.fnum_block(ch);
                    let freq = opn::fnum_block_to_freq(fnum, block, info.clock, info.fm_divisor);
                    let midi_f = opn::freq_to_midi(freq);
                    let (pb, oor) = freq_pitch_bend(midi_f, fm_base[ch]);
                    if oor {
                        if let Some(prev) = fm_note[ch].take() {
                            sink.note_off(tick, ch, prev);
                        }
                        let note = midi_f.round().clamp(0.0, 127.0) as u8;
                        fm_base[ch] = note as f32;
                        let (pb2, _) = freq_pitch_bend(midi_f, note as f32);
                        sink.pitch_bend(tick, ch, pb2, (midi_f - note as f32) * 100.0);
                        sink.note_on(tick, ch, note, freq, fm_vel);
                        fm_note[ch] = Some(note);
                        fm_pb[ch] = pb2;
                    } else if pb != fm_pb[ch] {
                        sink.pitch_bend(tick, ch, pb, (midi_f - fm_base[ch]) * 100.0);
                        fm_pb[ch] = pb;
                    }
                }
            }
            // 0xA4（Block + FNUM高3bit）書き込み: レジスタ状態の更新のみ（opn.write()済み）。
            // ピッチ更新は次の 0xA0 書き込み時に行う（OPN shadow register仕様準拠）。
            0xA4..=0xA6 => {}
            _ => {}
        }
    }

    // 残ノートをすべて解放
    let end_tick = SmfBuilder::samples_to_ticks(samples);
    // 末尾に溜まったSSG書き込みを確定してから解放する（最後の音の状態を取りこぼさない）。
    if info.has_ssg {
        for sc in 0..3 {
            if ssg_dirty[sc] && keep(fm + sc) {
                eval_ssg_channel(
                    sc, fm, &ssg, ssg_clock, bank, end_tick, sink, ssg_vel,
                    &mut ssg_note[sc], &mut ssg_base[sc], &mut ssg_pb[sc], &mut ssg_vol[sc],
                );
            }
        }
    }
    for ch in 0..fm {
        if let Some(note) = fm_note[ch].take() {
            sink.note_off(end_tick, ch, note);
        }
    }
    if info.has_ssg {
        for sc in 0..3 {
            if let Some(note) = ssg_note[sc].take() {
                sink.note_off(end_tick, fm + sc, note);
            }
        }
    }
}

/// SSG 1チャンネル分の発音状態を再評価し、必要なイベントを `sink` へ送る。
/// `ech = fm + sc` がエンジン/トラック上のチャンネル番号。
/// トーン有効時は [psg_patch]（矩形波）、ノイズのみ有効時は [noise_patch]（高帰還FM）を使う。
/// OOR retrigger 時もパッチを更新するため、ノイズ周期変化に追随できる。
#[allow(clippy::too_many_arguments)]
fn eval_ssg_channel(
    sc: usize,
    fm: usize,
    ssg: &SsgState,
    ssg_clock: u32,
    bank: &mut PatchBank,
    tick: u64,
    sink: &mut dyn OpnSink,
    vel: u8,
    note: &mut Option<u8>,
    base: &mut f32,
    pb: &mut i16,
    vol: &mut u8,
) {
    let ech = fm + sc;
    let tone_on   = ssg.tone_enabled(sc);
    let noise_on  = ssg.noise_enabled(sc);
    let period    = ssg.tone_period(sc);
    let noise_per = ssg.noise_period();
    let evol      = ssg.effective_volume(sc);
    // トーンは period>0 が必須。ノイズはNP=0も1扱いで鳴るため noise_on のみで可。
    let tone_active = tone_on && period > 0;
    let should_sound = (tone_active || noise_on) && evol > 0;

    if !should_sound {
        if let Some(prev) = note.take() {
            sink.note_off(tick, ech, prev);
        }
        return;
    }

    // 発音周波数: トーンが有効ならトーン周波数（混合時もOP1のトーンに使う）、
    // ノイズのみならノイズ周波数（ただしノイズはピッチレスでエンジン側はfreq無視）。
    let freq = if tone_active {
        ssg::period_to_freq(period, ssg_clock)
    } else {
        ssg::period_to_freq(noise_per.max(1) as u16, ssg_clock)
    };
    let midi_f = opn::freq_to_midi(freq);

    // 実機SSGミキサーに沿ってパッチを選択（find_or_insert_fixed で重複排除）:
    //   トーン+ノイズ → mix_patch（OP1矩形+OP3ノイズ加算）
    //   トーンのみ    → psg_patch（矩形）
    //   ノイズのみ    → noise_patch（NPの帯域）
    let (patch, program) = if tone_active && noise_on {
        let p = mix_patch(noise_per);
        let prog = (bank.find_or_insert_fixed(p, "ssg_mix") % 128) as u8;
        (p, prog)
    } else if tone_active {
        let p = psg_patch();
        let prog = (bank.find_or_insert_fixed(p, "psg_square") % 128) as u8;
        (p, prog)
    } else {
        let p = noise_patch(noise_per);
        let prog = (bank.find_or_insert_fixed(p, "ssg_noise") % 128) as u8;
        (p, prog)
    };

    if note.is_none() {
        // 新規キーオン
        let n = midi_f.round().clamp(0.0, 127.0) as u8;
        *base = n as f32;
        let (b, _) = freq_pitch_bend(midi_f, n as f32);
        sink.program_change(tick, ech, program, patch);
        sink.pitch_bend(tick, ech, b, (midi_f - n as f32) * 100.0);
        sink.note_on(tick, ech, n, freq, vel);
        sink.expression(tick, ech, evol);
        *note = Some(n);
        *pb = b;
        *vol = evol;
    } else {
        // 継続中: ピッチ更新（範囲外なら再キー）と音量更新
        let (b, oor) = freq_pitch_bend(midi_f, *base);
        if oor {
            if let Some(prev) = note.take() {
                sink.note_off(tick, ech, prev);
            }
            let n = midi_f.round().clamp(0.0, 127.0) as u8;
            *base = n as f32;
            let (b2, _) = freq_pitch_bend(midi_f, n as f32);
            // OOR retrigger 時もパッチを更新（ノイズ周期変化への追随）
            sink.program_change(tick, ech, program, patch);
            sink.pitch_bend(tick, ech, b2, (midi_f - n as f32) * 100.0);
            sink.note_on(tick, ech, n, freq, vel);
            // program_changeでパッチが満レベル(tl=255)へリセットされるため、現在の音量を
            // 必ず再適用する。これを怠ると、音量一定のまま大きくピッチが動く音色
            // （ドラムのピッチ降下スイープ等、evol==*volでOOR）が再キーオン直後に
            // フル音量で「ブッ」と鳴る。新規キーオン側(note.is_none())と同じ扱いに揃える。
            sink.expression(tick, ech, evol);
            *note = Some(n);
            *pb = b2;
            *vol = evol;
        } else if b != *pb {
            sink.pitch_bend(tick, ech, b, (midi_f - *base) * 100.0);
            *pb = b;
        }
        if evol != *vol {
            sink.expression(tick, ech, evol);
            *vol = evol;
        }
    }
}

/// OPN系の出力（SMF+バンク または WAV）。
fn run_opn(data: &[u8], data_start: usize, info: OpnInfo, args: &Args) -> Result<(), String> {
    let total_ch = info.fm_channels + if info.has_ssg { 3 } else { 0 };
    let mut bank = PatchBank::new();

    let fm_vel = scaled_velocity(args.fm_gain);
    let ssg_vel = scaled_velocity(args.ssg_gain);

    // 【実験用】フィードバック帰還方式の上書き。WAVレンダリング前にプロセスグローバルへ設定。
    if args.fb_2sample {
        ym38x6_core::set_feedback_two_sample(true);
        eprintln!("実験: feedback帰還を2サンプル平均 (out[n-1]+out[n-2])/2 に切替");
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
        process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);
        return sink.finish_and_write(wav_path, args.gain);
    }

    let mut sink = SmfSink::new(total_ch);
    process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);

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
    fn mix_patch_combines_tone_and_noise() {
        // Algorithm 7、OP1=矩形(16)=トーン、OP3=ノイズ(32+NP)、OP2/4=ミュート。
        let p = mix_patch(5);
        assert_eq!(p.channel.algorithm, 7);
        assert_eq!(p.operators[0].waveform, 16);
        assert_eq!(p.operators[0].tl, 255);
        assert_eq!(p.operators[1].tl, 0);
        assert_eq!(p.operators[2].waveform, 37); // 32+5
        assert_eq!(p.operators[2].tl, 255);
        assert_eq!(p.operators[3].tl, 0);
    }

    #[test]
    fn scaled_velocity_maps_gain_to_velocity() {
        assert_eq!(scaled_velocity(1.0), 100); // 従来どおり
        assert_eq!(scaled_velocity(0.5), 50); // 半分
        assert_eq!(scaled_velocity(0.0), 1); // 下限クランプ（0ではなく1）
        assert_eq!(scaled_velocity(2.0), 127); // 上限クランプ（velocity頭打ち）
    }

    #[test]
    fn noise_patch_maps_np_directly_to_waveform() {
        // OP1の波形番号 = 32 + NP（実機NP直対応、32〜63）。
        let p0 = noise_patch(0);
        let p31 = noise_patch(31);
        assert_eq!(p0.operators[0].waveform, 32);
        assert_eq!(p31.operators[0].waveform, 63);
        assert_eq!(p0.channel.algorithm, 7);
        // OP2〜4はミュート(TL=0)
        assert_eq!(p0.operators[1].tl, 0);
    }
}
