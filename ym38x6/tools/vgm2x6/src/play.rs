//! OPM/OPN共通の演奏ロジック（VGMコマンド列 → SMFイベント列 または WAV直描画）。
//!
//! チップ判定（[detect_chip]）・SMF/WAV出力先の抽象（[OpnSink]）・OPN共通ループ
//! （[process_opn]）・OPM専用ループ（[opm_to_smf]/[opm_to_audio]）を持つ。
//! bin側（vgm2x6本体）と外部クレート（vgm2op505等）の双方から呼ばれる想定。

use std::path::Path;

use crate::opm::{compute_pitch_bend, kc_to_midi_note, pb_to_semitones, OpmState, PB_SENSITIVITY};
use crate::opn::{self, OpnState};
use crate::patch::PatchBank;
use crate::smf::SmfBuilder;
use crate::ssg::{self, mix_patch, noise_patch, psg_patch, SsgState};
use crate::vgm::{VgmCmd, VgmHeader, VgmIter};
use ym38x6_core::{Vco, Ym38x6Patch};

// ===========================================================================
// チップ判定
// ===========================================================================

/// 入力VGMの音源チップ種別。
pub enum SourceChip {
    Opm,
    Opn(OpnInfo),
}

/// OPN書き込みコマンドの種類（どのVGMコマンドを横取りするか）。
#[derive(Clone, Copy, PartialEq)]
pub enum OpnWriteKind {
    Ym2612,
    Ym2203,
    Ym2608,
}

/// 検出したOPN系チップの構成。
#[derive(Clone, Copy)]
pub struct OpnInfo {
    pub label: &'static str,
    pub write_kind: OpnWriteKind,
    /// チップ入力クロック（Hz）。dual-chipビットはマスク済み。
    pub clock: u32,
    /// FMチャンネル数（YM2203=3、YM2612/YM2608=6）。
    pub fm_channels: usize,
    /// F-Number→周波数式の分母 factor（[opn::FM_DIVISOR_6CH] / [opn::FM_DIVISOR_3CH]）。
    pub fm_divisor: u32,
    /// SSGクロックの分周値（`ssg_clock = clock / ssg_clock_div`）。
    /// 6ch系(YM2608)はSSGプリスケーラも3ch系(YM2203)の2倍（YM2203=2 / YM2608=4）。
    /// この差を無視すると6ch系のSSGが1オクターブ高く変換される。
    pub ssg_clock_div: u32,
    /// SSG(PSG) を内蔵するか（YM2203/YM2608=true、YM2612=false）。
    pub has_ssg: bool,
}

impl OpnInfo {
    /// CSV出力用の短いチップ名（OPM/OPN2/OPN/OPNA）。
    pub fn label_short(&self) -> &'static str {
        match self.write_kind {
            OpnWriteKind::Ym2612 => "OPN2",
            OpnWriteKind::Ym2203 => "OPN",
            OpnWriteKind::Ym2608 => "OPNA",
        }
    }
}

/// VGMヘッダーのクロック欄から音源チップを判定する。YM2151を最優先、次にOPN系。
pub fn detect_chip(h: &VgmHeader) -> Option<SourceChip> {
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
/// 感度は [crate::opm::PB_SENSITIVITY] 半音（OPMパスと共通）。
pub fn freq_pitch_bend(cur_midi: f32, base_midi: f32) -> (i16, bool) {
    let delta = cur_midi - base_midi;
    let oor = delta.abs() > PB_SENSITIVITY as f32;
    let bend = (8192.0 + delta / PB_SENSITIVITY as f32 * 8192.0)
        .round()
        .clamp(0.0, 16383.0) as i16;
    (bend, oor)
}

// ===========================================================================
// OPN変換の出力先抽象（SMF / WAV直描画）
// ===========================================================================

/// OPN変換の出力先抽象。FM/SSGの楽音イベントを SMF（[SmfSink]）または
/// WAV直描画（[WavSink]）へ振り分ける。`tick` はSMF用（WAVは無視）。
pub trait OpnSink {
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
pub struct SmfSink {
    pub smf: SmfBuilder,
    last_program: Vec<Option<u8>>,
    last_cc11: Vec<i16>,
}

impl SmfSink {
    pub fn new(total_ch: usize) -> Self {
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
pub struct WavSink {
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
    pub fn new(sr: f32, total_ch: usize) -> Self {
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
    pub fn finish_and_write(self, out: &Path, gain: f32) -> Result<(), String> {
        let WavSink { mut engine, audio, sr, .. } = self;
        finish_wav(&mut engine, audio, sr, out, gain)
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
        self.engine.set_patch(self.patches[ch]);
        self.engine.note_on(ch, f, vel);
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

// ===========================================================================
// WAV仕上げ・書き出し（OPM/OPN共通）
// ===========================================================================

/// mono / 16bit PCM の最小 WAV ライター。
pub fn write_wav_mono16(path: &Path, samples: &[f32], sr: u32) -> std::io::Result<()> {
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

/// リリースの尾を1秒分レンダリングし、gain適用＋クリップ報告してWAVを書き出す。
/// OPM（[opm_to_audio]の直接ループ）・OPN（[WavSink]）共通のWAV仕上げ処理。
pub fn finish_wav(
    engine: &mut ym38x6_core::Ym38x6Engine,
    mut audio: Vec<f32>,
    sr: f32,
    out: &Path,
    gain: f32,
) -> Result<(), String> {
    let mut tail = vec![0.0f32; sr as usize];
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

    write_wav_mono16(out, &audio, sr as u32)
        .map_err(|e| format!("WAV書き込み失敗: {}: {e}", out.display()))?;
    println!(
        "WAV: {} ({:.1}秒, gain={:.2}, 素ピーク={:.3}, 出力ピーク={:.3}{})",
        out.display(),
        audio.len() as f32 / sr,
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

// ===========================================================================
// OPM(YM2151) 演奏ループ
// ===========================================================================

/// YM2151(OPM) の VGM を走査し、SMFイベント列を構築する（音色重複排除は`bank`が担う）。
/// ピッチベンド感度設定・キーオン/オフ・KC/KF変更ロジックを全て含む。
pub fn opm_to_smf(data: &[u8], data_start: usize, bank: &mut PatchBank) -> SmfBuilder {
    let mut opm = OpmState::new();
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

    for cmd in VgmIter::new(data, data_start) {
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

    smf
}

/// YM2151(OPM) の VGM を走査し、`engine`を直接駆動してPCMを生成する（WAV直描画、SMF非経由）。
///
/// DAW/VST/SMFを経由しないため、ピッチベンド感度RPNやホストのMIDI処理に依存しない
/// 「変換器＋エンジンが本来出すべき音」が得られる。リリースの尾（tail）は含まない
/// （呼び出し側が[finish_wav]で1秒分追加する）。
pub fn opm_to_audio(data: &[u8], data_start: usize, engine: &mut ym38x6_core::Ym38x6Engine) -> Vec<f32> {
    let mut opm = OpmState::new();
    let mut bank = PatchBank::new();
    let mut audio: Vec<f32> = Vec::new();

    let mut active:   [bool; 8] = [false; 8];
    let mut base_kc:  [u8;   8] = [0;     8];
    // 各チャンネルが現在鳴らしているパッチ（kc-reon時の再キーオンに再利用）。
    let mut cur_patch: [Ym38x6Patch; 8] = [Default::default(); 8];

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
                        engine.set_patch(patch);
                        engine.note_on(ch, midi_to_freq(note), 100);
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
                            engine.set_patch(cur_patch[ch]);
                            engine.note_on(ch, midi_to_freq(note), 100);
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

    audio
}

// ===========================================================================
// OPN系（YM2612 / YM2203 / YM2608）演奏ループ
// ===========================================================================

/// OPN系VGMを走査し、FM + SSG の楽音イベントを `sink` へ送る共通処理。
/// バンク（音色重複排除）は `bank` が保持する。
pub fn process_opn(
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
