//! SMF を `op505-core` で再生し、mono f32 バッファへレンダリングする。
//!
//! ボイスIDは `midi_channel*128 + note` とし、エンジンの内部チャンネル管理で
//! ポリフォニーをそのまま扱う（`op505-vst`と同じID符号化方式）。
//! プログラムチェンジ → `PatchBank::patch(program)` を音色として使う。
//!
//! CC/NRPN の解釈は `op505-vst`（OP505 VST3/CLAP プラグイン）と`op505-midi`クレートを
//! 共有しており、常に同じ解釈になる（詳細はspec-fm.md 8章「CC/NRPN解釈は共有クレート化」）。
//! VST はプラグイン単一音色前提でシャドウ状態をプラグイングローバルに持つが、smf2op505 は
//! マルチティンバーのため [`ChannelState`] を **MIDIチャンネル別** に16個持つ。
//!
//! op505のTimeEg 7本（OP1〜4 EG・Pitch/Cutoff/Gain FG）はDAWパラメーターではなくpersist状態
//! のため、`.op505`バンクの値をそのまま使う。NRPNからのFG Loop/Curve上書きはop505では欠番
//! （`op505_midi::ControlTarget::ReservedFgLoopCurve`）であり、ここでも何もしない。
//!
//! 対応SMFイベント:
//!   - Note On/Off、Program Change（CC102代替含む）、Tempo
//!   - Pitch Bend（チャンネル単位、RPN(0,0)でセンシティビティ設定可）
//!   - CC1/76/77/78: Pitch FG（ビブラート）への演奏補正
//!   - CC7/CC11: GM2音量（実効ゲイン=(cc7/127)²×(cc11/127)²）
//!   - CC64 Sustain / CC66 Sostenuto / CC67 Soft Pedal（ホールドフラグ方式）
//!   - CC91/93 + NRPN(0,2)〜(0,8): マスターエフェクト（Reverb/Chorus、`MasterEffects`）
//!   - NRPN(0,0)〜(0,27)/(0,34)/(0,35): 質感LFO・Algorithm/Waveform/Filter/AT/OP F-Number/CC2,4 Destination
//!   - Channel Pressure / Poly Key Pressure: AT Destination（NRPN(0,16)/(0,17)）
//!   - CC103〜106: Operator Key On/Off
//!   - CC120 All Sound Off（即時消音）/ CC121 Reset All Controllers（③層のみ）/
//!     CC123 All Notes Off（リリース）
//!
//! CC/NRPN の変更は当該チャンネルの発音中ボイスへ即時伝播する（[`apply_live`]）。

use op505_core::{Op505Engine, Op505Patch};
use op505_midi::{
    apply_expression_modulation, apply_pitch_fg_expression, apply_soft_pedal,
    cc_to_u7 as cc_norm_to_u7, cc_to_u8 as cc_norm_to_u8, control_target, released_notes, ControlTarget,
    ExpressionDestination, PedalState, RpnTracker,
};
use sound_core::{cc76_to_rate_scale, lfo_fade_mode_from_index, AudioProcessor, ChorusType, MasterEffects, ReverbType, Vco};

use crate::bank::PatchBank;
use crate::smf::{parse_smf, EvKind};

/// SMF/MIDIの生CC値（0〜127の7bit整数）を内部表現（0〜255）へ変換する。
/// `op505_midi::cc_to_u8`はVST（nice-plugの正規化済みf32(0.0〜1.0)パラメーター）向けのため、
/// SMFの整数バイト値をここで正規化してから橋渡しする（決定事項「CC値→内部値の変換式は
/// VST式（round(cc/127*255)）に統一する」を、入力型の違いを吸収しつつ満たす）。
fn cc_to_u8(value: u8) -> u8 {
    cc_norm_to_u8(value as f32 / 127.0)
}

/// SMF/MIDIの生CC値（0〜127）をそのまま7bit値として使う（`cc_to_u8`と同じ橋渡し）。
fn cc_to_u7(value: u8) -> u8 {
    cc_norm_to_u7(value as f32 / 127.0)
}

/// MIDIノート番号の総数（0〜127）。ノート番号をそのままボイスIDの下位に使うため、
/// 発音中ボイス走査ループの上限に使う（`op505-vst`の`MIDI_NOTE_COUNT`と同じ）。
const MIDI_NOTE_COUNT: usize = 128;

/// CC7(Channel Volume) と CC11(Expression) の値（0〜127）から GM2 準拠のゲインを計算する
/// （`op505-vst`と同一式。`op505-midi`には置かない小関数のため、op505-vstと同様ここで複製する）。
#[inline]
fn channel_gain(cc7: u8, cc11: u8) -> f32 {
    let v7 = cc7 as f32 / 127.0;
    let v11 = cc11 as f32 / 127.0;
    v7 * v7 * v11 * v11
}

/// 1つのMIDIチャンネルの解釈状態（CC/NRPN シャドウ）。16個で全チャンネルを管理する。
///
/// `op505-vst`のプラグイングローバル・シャドウフィールドを**チャンネル別**に持ち直したもの。
/// NRPN で上書きされる離散/焼き込みフィールドは `Option`（None=ベースパッチ値のまま＝
/// 「NRPN は現在のパッチの当該フィールドのみ書き換え」）、CC1/76/77/78 の Pitch FG 補正は
/// 中立既定の加算値として常時適用する。
struct ChannelState {
    /// 現在のプログラム番号（Program Change / CC102 で更新）。
    program: u8,

    rpn: RpnTracker,
    data_entry_msb: u8,
    data_entry_lsb: u8,

    // --- ピッチベンド ---
    /// ピッチベンド感度（半音）。RPN(0,0)で変更、既定±2半音。
    pitch_bend_range: f32,
    /// 現在のベンド量（セント）。note_on 時に新ボイスへ再適用する。
    bend_cents: f32,

    // --- 音量（CC7/CC11、GM2）---
    cc7: u8,
    cc11: u8,

    // --- Pitch FG 演奏補正（中立既定、常時適用）---
    pitch_fg_cc1: u8,       // CC1 Modulation Wheel（0〜127）
    pitch_fg_cc76: u8,      // CC76 Vibrato Rate（0〜127、64=無補正）
    pitch_fg_cc77: u8,      // CC77 Vibrato Depth（0〜255、Depthへ0起点加算）
    pitch_fg_cc78: u8,      // CC78 Vibrato Delay（0〜127、64=無補正）
    pitch_fg_rpn0_5: u8,    // RPN(0,5) Modulation Depth Range（GM2、既定64）

    // --- NRPN 離散/焼き込み上書き（None=ベースパッチ値）---
    algorithm: Option<u8>,
    operator_waveforms: [Option<u8>; 4],
    filter_type: Option<u8>,
    filter_self_oscillation: Option<bool>,
    texture_lfo_destination: Option<u8>,
    texture_lfo_waveform: Option<u8>,
    texture_lfo_fade_mode: Option<u8>,
    texture_lfo_rate: Option<u8>,
    texture_lfo_depth: Option<u8>,
    texture_lfo_delay: Option<u8>,
    texture_lfo_fade_time: Option<u8>,
    texture_lfo_offset: Option<u8>,
    /// OP単位F-Number上書き（NRPN(0,18)〜(0,21)、13bit、Some時のみ set_operator_f_number）。
    operator_f_number_override: [Option<u16>; 4],

    // --- アフタータッチ ---
    at_destination: ExpressionDestination,
    poly_at_destination: ExpressionDestination,
    channel_pressure: u8,
    /// Poly Key Pressure（ノート番号→圧力値）。`op505-vst`と同じくノート番号(0〜127)で
    /// 直接引ける固定長配列にしている（旧`ym38x6/tools/smf2wav`のHashMapからの変更点）。
    poly_pressure: [u8; 128],

    // --- CC2(ブレス)/CC4(フット) ---（NRPN(0,34)/(0,35)で行先選択、既定はCC2→TLキャリア一括／
    // CC4→Filter Cutoff＝手動ワウ）
    cc2: u8,
    cc4: u8,
    cc2_destination: ExpressionDestination,
    cc4_destination: ExpressionDestination,

    // --- ペダル（CC64 Sustain / CC66 Sostenuto / CC67 Soft）---
    pedal: PedalState,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            program: 0,
            rpn: RpnTracker::default(),
            data_entry_msb: 0,
            data_entry_lsb: 0,
            pitch_bend_range: 2.0,
            bend_cents: 0.0,
            cc7: 127,
            cc11: 127,
            pitch_fg_cc1: 0,
            pitch_fg_cc76: 64,
            pitch_fg_cc77: 0,
            pitch_fg_cc78: 64,
            pitch_fg_rpn0_5: 64,
            algorithm: None,
            operator_waveforms: [None; 4],
            filter_type: None,
            filter_self_oscillation: None,
            texture_lfo_destination: None,
            texture_lfo_waveform: None,
            texture_lfo_fade_mode: None,
            texture_lfo_rate: None,
            texture_lfo_depth: None,
            texture_lfo_fade_time: None,
            texture_lfo_delay: None,
            texture_lfo_offset: None,
            operator_f_number_override: [None; 4],
            at_destination: ExpressionDestination::default(),
            poly_at_destination: ExpressionDestination::default(),
            channel_pressure: 0,
            poly_pressure: [0; 128],
            cc2: 0,
            cc4: 0,
            cc2_destination: ExpressionDestination::TlCarriers,
            cc4_destination: ExpressionDestination::FilterCutoff,
            pedal: PedalState::default(),
        }
    }
}

/// ベースパッチ（プログラムの音色）に、チャンネルの NRPN 離散/焼き込み上書きを重ねた
/// 実効パッチを組み立てる。`op505-vst`の`build_patch()`を移植したもの。
///
/// Pitch FG 演奏補正（CC1/76/77/78）・AT（アフタータッチ）・Soft PedalはChannelParams外の
/// 後処理のため、ここでは扱わず[`apply_live`]/[`note_on_voice`]側で
/// `apply_pitch_fg_expression`/`apply_expression_modulation`/`apply_soft_pedal`を適用する
/// （`op505-vst`と同じ「note_patchへの後処理」パターン）。
fn build_effective_patch(base: &Op505Patch, st: &ChannelState) -> Op505Patch {
    let mut patch = *base;

    if let Some(v) = st.algorithm {
        patch.channel.algorithm = v;
    }
    for (i, wf) in st.operator_waveforms.iter().enumerate() {
        if let Some(v) = wf {
            patch.operators[i].waveform = *v;
        }
    }
    if let Some(v) = st.filter_type {
        patch.channel.filter_type = v;
    }
    if let Some(v) = st.filter_self_oscillation {
        patch.channel.filter_self_oscillation = v;
    }

    // 質感LFO 焼き込み上書き（Some のときのみ）
    if let Some(v) = st.texture_lfo_destination {
        patch.channel.texture_lfo.destination = v;
    }
    if let Some(v) = st.texture_lfo_waveform {
        patch.channel.texture_lfo.waveform = v;
    }
    if let Some(v) = st.texture_lfo_fade_mode {
        patch.channel.texture_lfo.fade_mode = v;
    }
    if let Some(v) = st.texture_lfo_rate {
        patch.channel.texture_lfo.rate = v;
    }
    if let Some(v) = st.texture_lfo_depth {
        patch.channel.texture_lfo.depth = v;
    }
    if let Some(v) = st.texture_lfo_delay {
        patch.channel.texture_lfo.delay = v;
    }
    if let Some(v) = st.texture_lfo_fade_time {
        patch.channel.texture_lfo.fade_time = v;
    }
    if let Some(v) = st.texture_lfo_offset {
        patch.channel.texture_lfo.offset = v;
    }

    patch
}

/// note_patchへ、CC2/CC4/AT/Pitch FG演奏補正/Soft Pedalを一括で後適用する
/// （`apply_live`/`note_on_voice`共通の後処理列、順序も含め`op505-vst`のprocess()に合わせる）。
fn apply_note_post_processing(patch: &mut Op505Patch, note: u8, st: &ChannelState) {
    apply_expression_modulation(
        note,
        &[
            (st.cc2, st.cc2_destination),
            (st.cc4, st.cc4_destination),
            (st.channel_pressure, st.at_destination),
        ],
        st.poly_at_destination,
        &st.poly_pressure,
        patch,
    );
    apply_pitch_fg_expression(patch, st.pitch_fg_cc1, st.pitch_fg_cc77, st.pitch_fg_cc78, st.pitch_fg_rpn0_5);
    if st.pedal.soft_notes & (1u128 << note) != 0 {
        apply_soft_pedal(patch, st.pedal.cc67);
    }
}

/// CC/NRPN で変わった実効パッチ・CC76 rate_scale・AT・OP F-Number を、そのチャンネルの
/// 発音中ボイス全てへ伝播する（ライブ反映）。`op505-vst`の毎ブロック伝播ループを
/// 1チャンネル分に絞ったもの。非発音スロットへの set_* はエンジン側で no-op になる。
fn apply_live(engine: &mut Op505Engine, chi: usize, st: &ChannelState, bank: &PatchBank) {
    let base = *bank.patch(st.program);
    let eff = build_effective_patch(&base, st);
    let rate_scale = cc76_to_rate_scale(st.pitch_fg_cc76);
    for note in 0..MIDI_NOTE_COUNT {
        let id = chi * 128 + note;
        let mut note_patch = eff;
        apply_note_post_processing(&mut note_patch, note as u8, st);
        engine.set_channel_params(id, note_patch.channel);
        for (op_index, op) in note_patch.operators.iter().enumerate() {
            engine.set_operator_params(id, op_index, *op);
        }
        engine.set_pitch_fg_rate_scale(id, rate_scale);
        for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
            if let Some(f_number) = f {
                engine.set_operator_f_number(id, op_index, *f_number);
            }
        }
    }
}

/// 1ボイスのノートオン。`op505-vst`のNoteOn適用と同型。
/// 現在の AT を焼き込んだ実効パッチで発音し、ベンド量・音量ゲイン・rate_scale・
/// OP F-Number を新ボイスへ反映する。
fn note_on_voice(engine: &mut Op505Engine, chi: usize, note: u8, vel: u8, st: &ChannelState, bank: &PatchBank) {
    let base = *bank.patch(st.program);
    let mut eff = build_effective_patch(&base, st);
    apply_note_post_processing(&mut eff, note, st);
    let id = chi * 128 + note as usize;
    let freq = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
    engine.set_patch(eff);
    engine.note_on(id, freq, vel);
    engine.set_channel_volume(id, channel_gain(st.cc7, st.cc11));
    engine.set_pitch_bend(id, st.bend_cents);
    engine.set_pitch_fg_rate_scale(id, cc76_to_rate_scale(st.pitch_fg_cc76));
    for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
        if let Some(f_number) = f {
            engine.set_operator_f_number(id, op_index, *f_number);
        }
    }
}

/// NRPN(0,18)〜(0,21)：CC6(MSB)+CC38(LSB)の14bit値を13bit(0〜8191)にclampして
/// OP F-Number 上書きとして記録する（実際の発音中ボイスへの反映は `apply_live`）。
fn set_operator_f_number_override(st: &mut ChannelState, op_index: usize) {
    let combined = (st.data_entry_msb as u16) * 128 + st.data_entry_lsb as u16;
    st.operator_f_number_override[op_index] = Some(combined.min(8191));
}

/// CC6(Data Entry MSB)受信時、`op505_midi::control_target`で解決した制御対象に応じて
/// 値を適用する。`op505-vst`の`handle_data_entry`の移植。戻り値=発音中ボイスへの伝播
/// （`apply_live`）が必要かどうか。エフェクト系NRPNやRPN(0,0)は不要。
///
/// `ControlTarget::ReservedFgLoopCurve`（op505のTimeEg 7本はpersist状態でNRPNからは
/// 触らない欠番）は何もしない。
fn handle_data_entry(st: &mut ChannelState, effects: &mut MasterEffects, value: u8) -> bool {
    st.data_entry_msb = cc_to_u7(value);
    match control_target(st.rpn.selection) {
        ControlTarget::PitchBendRange => {
            st.pitch_bend_range = cc_to_u7(value) as f32;
            false
        }
        ControlTarget::ModulationDepthRange => {
            st.pitch_fg_rpn0_5 = cc_to_u7(value);
            true
        }
        ControlTarget::TextureLfoDestination => {
            st.texture_lfo_destination = Some(cc_to_u7(value).min(4));
            true
        }
        ControlTarget::TextureLfoWaveform => {
            st.texture_lfo_waveform = Some(cc_to_u7(value).min(4));
            true
        }
        ControlTarget::ReverbType => {
            effects.set_reverb_type(ReverbType::from_u8(cc_to_u7(value)));
            false
        }
        ControlTarget::ChorusType => {
            effects.set_chorus_type(ChorusType::from_u8(cc_to_u7(value)));
            false
        }
        ControlTarget::ReverbTime => {
            effects.set_reverb_time(cc_to_u8(value));
            false
        }
        ControlTarget::ChorusModRate => {
            effects.set_chorus_mod_rate(cc_to_u8(value));
            false
        }
        ControlTarget::ChorusModDepth => {
            effects.set_chorus_mod_depth(cc_to_u8(value));
            false
        }
        ControlTarget::ChorusFeedback => {
            effects.set_chorus_feedback(cc_to_u8(value));
            false
        }
        ControlTarget::ChorusSendToReverb => {
            effects.set_chorus_send_to_reverb(cc_to_u8(value));
            false
        }
        ControlTarget::Algorithm => {
            st.algorithm = Some(cc_to_u7(value).min(7));
            true
        }
        ControlTarget::OperatorWaveform(op_index) => {
            st.operator_waveforms[op_index as usize] = Some(cc_to_u8(value));
            true
        }
        ControlTarget::FilterType => {
            st.filter_type = Some(cc_to_u7(value).min(2));
            true
        }
        ControlTarget::FilterSelfOscillation => {
            st.filter_self_oscillation = Some(cc_to_u7(value) != 0);
            true
        }
        ControlTarget::AtDestination => {
            st.at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        ControlTarget::PolyAtDestination => {
            st.poly_at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        ControlTarget::OperatorFNumber(op_index) => {
            set_operator_f_number_override(st, op_index as usize);
            true
        }
        ControlTarget::TextureLfoFadeMode => {
            st.texture_lfo_fade_mode = Some(lfo_fade_mode_from_index(cc_to_u7(value)) as u8);
            true
        }
        ControlTarget::TextureLfoRate => {
            st.texture_lfo_rate = Some(cc_to_u8(value));
            true
        }
        ControlTarget::TextureLfoDepth => {
            st.texture_lfo_depth = Some(cc_to_u8(value));
            true
        }
        ControlTarget::TextureLfoDelay => {
            st.texture_lfo_delay = Some(cc_to_u8(value));
            true
        }
        ControlTarget::TextureLfoFadeTime => {
            st.texture_lfo_fade_time = Some(cc_to_u8(value));
            true
        }
        ControlTarget::TextureLfoOffset => {
            st.texture_lfo_offset = Some(cc_to_u8(value));
            true
        }
        ControlTarget::ReservedFgLoopCurve => false,
        ControlTarget::Cc2Destination => {
            st.cc2_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        ControlTarget::Cc4Destination => {
            st.cc4_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        ControlTarget::Unassigned => false,
    }
}

/// レンダリング済みサンプル位置を `target` まで進め、その区間をマスターエフェクトに通す。
/// チャンクはイベント境界に揃うため、エフェクトのオートメーションがサンプル正確に反映される。
fn render_chunk(
    engine: &mut Op505Engine,
    effects: &mut MasterEffects,
    out: &mut Vec<f32>,
    rendered: &mut usize,
    target: usize,
) {
    if target > *rendered {
        let n = target - *rendered;
        let mut buf = vec![0.0f32; n];
        engine.render(&mut buf, 1);
        effects.process(&mut buf, 1);
        out.extend_from_slice(&buf);
        *rendered = target;
    }
}

/// SMF を `bank` を音色として再生し、mono f32 サンプル列を返す。
/// `tail_secs` はノートオフ後の残響を伸ばす秒数。
/// `max_secs` が `Some(s)` のとき、出力を `s` 秒（テール込み）で打ち切る（試聴の時短用）。
pub fn render_smf(
    data: &[u8],
    bank: &PatchBank,
    sample_rate: f32,
    tail_secs: f32,
    max_secs: Option<f32>,
    max_voices: Option<usize>,
) -> Result<Vec<f32>, String> {
    let (division, events) = parse_smf(data)?;

    let mut engine = Op505Engine::new(sample_rate);
    // EXPERIMENT(max-voices): 同時発音数上限のA/B計測用（Noneはエンジン既定を使う）。
    if let Some(n) = max_voices {
        engine.set_max_voices(n);
    }
    // SMF内蔵のマスターエフェクト（CC91/93・NRPN(0,2)〜(0,8)で駆動）。既定 send=0 で透過。
    // main.rs の `--reverb-*`（op505-tools::fx）はこれとは独立した後段の診断用リバーブ。
    let mut effects = MasterEffects::new(sample_rate);
    let mut out: Vec<f32> = Vec::new();

    let mut channels: Vec<ChannelState> = (0..16).map(|_| ChannelState::default()).collect();

    let mut tempo_us: f64 = 500_000.0; // 既定 120BPM
    let mut spt = tempo_us / 1_000_000.0 * sample_rate as f64 / division as f64; // samples/tick
    let mut cur_tick: u64 = 0;
    let mut sample_pos: f64 = 0.0;
    let mut rendered: usize = 0;
    let mut peak_voices: usize = 0;

    let max_samples = max_secs.map(|s| (s * sample_rate).max(0.0) as usize);

    for e in &events {
        let dt = e.tick - cur_tick;
        sample_pos += dt as f64 * spt;
        cur_tick = e.tick;
        let target = sample_pos.floor() as usize;
        // 時短打ち切り: 上限に達したらそこまでレンダリングして以降のイベントは無視する。
        if let Some(maxs) = max_samples {
            if target >= maxs {
                render_chunk(&mut engine, &mut effects, &mut out, &mut rendered, maxs);
                eprintln!("smf2op505: ピークボイス数(active_voice_count) = {peak_voices}");
                return Ok(out);
            }
        }
        render_chunk(&mut engine, &mut effects, &mut out, &mut rendered, target);
        peak_voices = peak_voices.max(engine.active_voice_count());

        match e.kind {
            EvKind::Tempo(us) => {
                tempo_us = us as f64;
                spt = tempo_us / 1_000_000.0 * sample_rate as f64 / division as f64;
            }
            EvKind::Program(ch, p) => {
                channels[ch as usize].program = p;
            }
            EvKind::NoteOn(ch, note, vel) => {
                let chi = ch as usize;
                // 弾き直したらペダル保留を解除する（離鍵→ペダルアップ前に再度弾いた場合、
                // 古い保留ビットが残っていると鍵盤を押している最中にペダルアップでnote_offが
                // 誤発火する）。Soft Pedal（CC67）: ON中に新規キーオンしたノートのみ対象。
                channels[chi].pedal.note_on(note);
                note_on_voice(&mut engine, chi, note, vel, &channels[chi], bank);
            }
            EvKind::NoteOff(ch, note) => {
                let chi = ch as usize;
                channels[chi].poly_pressure[note as usize] = 0;
                if channels[chi].pedal.note_off(note) {
                    engine.note_off(chi * 128 + note as usize);
                }
            }
            EvKind::PitchBend(ch, raw) => {
                let chi = ch as usize;
                // raw: -8192〜8191、8192=±1半音×pitch_bend_range半音
                let cents = raw as f32 / 8192.0 * channels[chi].pitch_bend_range * 100.0;
                channels[chi].bend_cents = cents;
                engine.set_pitch_bend_group(chi, cents);
            }
            EvKind::ChannelPressure(ch, value) => {
                let chi = ch as usize;
                channels[chi].channel_pressure = cc_to_u8(value);
                apply_live(&mut engine, chi, &channels[chi], bank);
            }
            EvKind::PolyPressure(ch, note, value) => {
                let chi = ch as usize;
                channels[chi].poly_pressure[note as usize] = cc_to_u8(value);
                apply_live(&mut engine, chi, &channels[chi], bank);
            }
            EvKind::ControlChange(ch, cc, val) => {
                handle_control_change(&mut engine, &mut effects, &mut channels, ch as usize, cc, val, bank);
            }
        }
    }

    // 残響テール（max_secs 指定時は上限でクランプ）
    let mut tail_target = rendered + (sample_rate * tail_secs) as usize;
    if let Some(maxs) = max_samples {
        tail_target = tail_target.min(maxs);
    }
    render_chunk(&mut engine, &mut effects, &mut out, &mut rendered, tail_target);
    peak_voices = peak_voices.max(engine.active_voice_count());
    eprintln!("smf2op505: ピークボイス数(active_voice_count) = {peak_voices}");
    Ok(out)
}

/// 1つのコントロールチェンジを処理する。`op505-vst`のprocess()内CC matchと
/// handle_data_entryをper-channelに移植したもの。
#[allow(clippy::too_many_arguments)]
fn handle_control_change(
    engine: &mut Op505Engine,
    effects: &mut MasterEffects,
    channels: &mut [ChannelState],
    chi: usize,
    cc: u8,
    val: u8,
    bank: &PatchBank,
) {
    match cc {
        // CC7/CC11: GM2音量。実効ゲイン=(cc7/127)²×(cc11/127)²。発音中へ即時反映。
        7 => {
            channels[chi].cc7 = cc_to_u7(val);
            let gain = channel_gain(channels[chi].cc7, channels[chi].cc11);
            engine.set_channel_volume_group(chi, gain);
        }
        11 => {
            channels[chi].cc11 = cc_to_u7(val);
            let gain = channel_gain(channels[chi].cc7, channels[chi].cc11);
            engine.set_channel_volume_group(chi, gain);
        }
        // CC1/76/77/78: Pitch FG 演奏補正 → 発音中ボイスへ伝播。
        1 => {
            channels[chi].pitch_fg_cc1 = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        76 => {
            channels[chi].pitch_fg_cc76 = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        77 => {
            channels[chi].pitch_fg_cc77 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        78 => {
            channels[chi].pitch_fg_cc78 = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        // CC2(ブレス)/CC4(フット): Expression Destination（NRPN(0,34)/(0,35)）へ加算 → 発音中ボイスへ伝播。
        2 => {
            channels[chi].cc2 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        4 => {
            channels[chi].cc4 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        // NRPN/RPN 選択（CC99/98=NRPN MSB/LSB、CC101/100=RPN MSB/LSB）
        98 => channels[chi].rpn.set_nrpn_lsb(cc_to_u7(val)),
        99 => channels[chi].rpn.set_nrpn_msb(cc_to_u7(val)),
        100 => channels[chi].rpn.set_rpn_lsb(cc_to_u7(val)),
        101 => channels[chi].rpn.set_rpn_msb(cc_to_u7(val)),
        // CC6 Data Entry MSB: 選択中の RPN/NRPN へ値を適用。
        6 => {
            if handle_data_entry(&mut channels[chi], effects, val) {
                apply_live(engine, chi, &channels[chi], bank);
            }
        }
        // CC38 Data Entry LSB: OP F-Number(NRPN 0,18〜21選択中)の下位7bit。
        38 => {
            channels[chi].data_entry_lsb = cc_to_u7(val);
            if let ControlTarget::OperatorFNumber(op_index) = control_target(channels[chi].rpn.selection) {
                set_operator_f_number_override(&mut channels[chi], op_index as usize);
                apply_live(engine, chi, &channels[chi], bank);
            }
        }
        // CC64 Sustain Pedal（ホールドフラグ方式）。
        64 => {
            let released = channels[chi].pedal.cc64(val);
            for note in released_notes(released) {
                engine.note_off(chi * 128 + note as usize);
            }
        }
        // CC66 Sostenuto: ON時点でkeys_down中のノートのみをlatchし、CC66 OFF（かつ
        // CC64も踏まれていない）までReleaseに入らせない。
        66 => {
            let released = channels[chi].pedal.cc66(val);
            for note in released_notes(released) {
                engine.note_off(chi * 128 + note as usize);
            }
        }
        // CC67 Soft Pedal: 深さを保持するのみ。ON中に新規キーオンしたノートのみへの
        // 適用はnote_on_voice/apply_live側（soft_notesビット）で行う。
        67 => {
            channels[chi].pedal.cc67(cc_to_u7(val));
        }
        // CC121 Reset All Controllers: ③ジェスチャー層のみリセットする（②パート状態・
        // ①音色は保持）。CC64/66/67ペダル・Pitch Bend・CC1・アフタータッチが対象。
        // CC2/CC4/CC7/CC11/CC76〜78/センド/RPN等は保持。
        121 => {
            let released = channels[chi].pedal.cc121();
            for note in released_notes(released) {
                engine.note_off(chi * 128 + note as usize);
            }
            channels[chi].pitch_fg_cc1 = 0;
            channels[chi].bend_cents = 0.0;
            channels[chi].channel_pressure = 0;
            channels[chi].poly_pressure = [0; 128];
            engine.set_pitch_bend_group(chi, 0.0);
            apply_live(engine, chi, &channels[chi], bank);
        }
        // CC91/93: マスターエフェクト送りレベル（master）。
        91 => effects.set_reverb_send(cc_to_u8(val)),
        93 => effects.set_chorus_send(cc_to_u8(val)),
        // CC102: Program Change 代替（VST3で MidiProgramChange が届かないため。smf2op505 では
        // 単一バンクなので Bank Select CC0/32 は使わず、プログラム番号のみ更新する）。
        102 => {
            channels[chi].program = cc_to_u7(val);
        }
        // CC103〜106: Operator Key On/Off（≧64でキーオン/<64でキーオフ、全OP独立）。
        103..=106 => {
            let op_index = (cc - 103) as usize;
            let key_on = cc_to_u7(val) >= 64;
            for note in 0..MIDI_NOTE_COUNT {
                let id = chi * 128 + note;
                if key_on {
                    engine.note_on_operator(id, op_index);
                } else {
                    engine.note_off_operator(id, op_index);
                }
            }
        }
        // CC120 All Sound Off: リリースを経ず即座に消音する（GM2準拠、CC123のリリース
        // とは区別する）。`silence_group`はnote_offのReleaseを経ないため残響も無い。
        120 => {
            engine.silence_group(chi);
            channels[chi].pedal.cc120_reset();
        }
        // CC123 All Notes Off: 通常のNote-Off相当（リリースして自然減衰）。
        123 => {
            for note in 0..MIDI_NOTE_COUNT {
                engine.note_off(chi * 128 + note);
            }
            channels[chi].pedal.cc123_reset();
        }
        // Bank Select（CC0/32）等は smf2op505 では無視（単一バンクのため）。
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 中立の ChannelState では実効パッチがベースと一致する。
    #[test]
    fn effective_patch_neutral_equals_base() {
        let base = Op505Patch::default();
        let st = ChannelState::default();
        let eff = build_effective_patch(&base, &st);
        assert_eq!(eff, base);
    }

    /// NRPN(0,23) 質感LFO Rate はVST式（round(cc/127*255)）でシャドウへ入り、実効パッチへ反映される。
    #[test]
    fn nrpn_texture_lfo_rate_into_patch() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        st.rpn.set_nrpn_msb(0);
        st.rpn.set_nrpn_lsb(23);
        let needs = handle_data_entry(&mut st, &mut fx, 100);
        assert!(needs);
        // round(100/127*255) = round(200.79) = 201（決定事項「CC値→内部値の変換式はVST式に統一する」）。
        assert_eq!(st.texture_lfo_rate, Some(201));
        let eff = build_effective_patch(&Op505Patch::default(), &st);
        assert_eq!(eff.channel.texture_lfo.rate, 201);
    }

    /// NRPN(0,9) Algorithm 上書きは実効パッチの algorithm を置き換える。
    #[test]
    fn nrpn_algorithm_override() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        st.rpn.set_nrpn_msb(0);
        st.rpn.set_nrpn_lsb(9);
        assert!(handle_data_entry(&mut st, &mut fx, 5));
        assert_eq!(st.algorithm, Some(5));
        let eff = build_effective_patch(&Op505Patch::default(), &st);
        assert_eq!(eff.channel.algorithm, 5);
    }

    /// エフェクト系 NRPN(0,2〜8) はボイス伝播不要（false）を返す。
    #[test]
    fn nrpn_effects_return_no_voice_update() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        for lsb in [2u8, 3, 4, 5, 6, 7, 8] {
            st.rpn.set_nrpn_msb(0);
            st.rpn.set_nrpn_lsb(lsb);
            assert!(!handle_data_entry(&mut st, &mut fx, 64), "NRPN(0,{lsb}) should not need voice update");
        }
    }

    /// NRPN(0,28)〜(0,33)（op505ではReservedFgLoopCurve）は何も変えずfalseを返す。
    #[test]
    fn nrpn_reserved_fg_loop_curve_is_noop() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        for lsb in 28u8..=33 {
            st.rpn.set_nrpn_msb(0);
            st.rpn.set_nrpn_lsb(lsb);
            assert!(!handle_data_entry(&mut st, &mut fx, 127), "NRPN(0,{lsb}) should be a no-op");
        }
    }

    /// NRPN(0,18)＝OP0 F-Number は CC6(MSB)+CC38(LSB)の14bit→13bit clamp。
    #[test]
    fn nrpn_operator_f_number_14bit() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        st.rpn.set_nrpn_msb(0);
        st.rpn.set_nrpn_lsb(18);
        st.data_entry_lsb = 10;
        assert!(handle_data_entry(&mut st, &mut fx, 60)); // msb=60 → 60*128+10 = 7690
        assert_eq!(st.operator_f_number_override[0], Some(7690));
        // 8191 超は clamp
        st.data_entry_lsb = 127;
        assert!(handle_data_entry(&mut st, &mut fx, 127)); // 127*128+127 = 16383 → 8191
        assert_eq!(st.operator_f_number_override[0], Some(8191));
    }

    // --- E2E smoke: 手書きSMFで発音中CC1のライブ伝播を検証 ---

    /// 可変長数値（delta-time）をエンコードする。
    fn vlq(mut v: u32) -> Vec<u8> {
        let mut buf = vec![(v & 0x7F) as u8];
        v >>= 7;
        while v > 0 {
            buf.insert(0, ((v & 0x7F) as u8) | 0x80);
            v >>= 7;
        }
        buf
    }

    /// (delta, midiバイト列) の列から Format0 SMF（division=480）を組み立てる。
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
        smf.extend_from_slice(&480u16.to_be_bytes()); // division
        smf.extend_from_slice(b"MTrk");
        smf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        smf.extend_from_slice(&track);
        smf
    }

    /// フルサステイン(d1l/level最大)・即発音の1音色バンク（1段TimeEg、無限サステイン）。
    fn instant_sustain_bank() -> PatchBank {
        use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
        let mut patch = Op505Patch::default();
        patch.channel.algorithm = 7; // 全OP独立キャリア
        patch.operators[0].tl = 220;
        patch.operators[0].mul = 1;
        let mut stages = [TimeStage::default(); MAX_STAGES];
        stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
        patch.operators[0].eg =
            TimeEgParams { stages, stage_count: 1, loop_enabled: 0, loop_start: 0, loop_end: 0, release_start: 0 };
        PatchBank::from_patches(&[patch]).unwrap()
    }

    /// ループするPitch FGを持つ、発音する1音色バンクを作る（ビブラート確認用）。
    fn vibrato_bank() -> PatchBank {
        use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
        let mut patch = Op505Patch::default();
        patch.channel.algorithm = 7;
        patch.operators[0].tl = 220;
        patch.operators[0].mul = 1;
        let mut carrier_stages = [TimeStage::default(); MAX_STAGES];
        carrier_stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
        patch.operators[0].eg = TimeEgParams {
            stages: carrier_stages,
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            loop_end: 0,
            release_start: 0,
        };
        let mut lfo_stages = [TimeStage::default(); MAX_STAGES];
        lfo_stages[0] = TimeStage { time: 40, level: 255, curve: 0 };
        lfo_stages[1] = TimeStage { time: 40, level: 0, curve: 0 };
        patch.channel.pitch_fg.eg = TimeEgParams {
            stages: lfo_stages,
            stage_count: 2,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 1,
            release_start: 2,
        };
        patch.channel.pitch_fg.depth = 200; // 中立(128)からずらして揺れを出す
        PatchBank::from_patches(&[patch]).unwrap()
    }

    /// 発音中に届いた CC1(Mod Wheel) が発音中ボイスへライブ伝播し、出力が変わることを確認する。
    #[test]
    fn cc1_live_propagation_changes_sounding_voice() {
        let bank = vibrato_bank();
        let sr = 8000.0;
        let note_on = (0u32, vec![0x90, 69, 100]);
        let note_off = (480u32, vec![0x80, 69, 0]);

        // CC1 無し
        let smf_a = build_smf(&[note_on.clone(), note_off.clone()]);
        let buf_a = render_smf(&smf_a, &bank, sr, 0.1, Some(1.0), None).unwrap();

        // tick240 で CC1=127（発音中）
        let smf_b = build_smf(&[
            note_on,
            (240u32, vec![0xB0, 1, 127]),
            (240u32, vec![0x80, 69, 0]), // note_off の delta を分割（合計480）
        ]);
        let buf_b = render_smf(&smf_b, &bank, sr, 0.1, Some(1.0), None).unwrap();

        assert!(buf_a.iter().any(|s| s.abs() > 1e-4), "CC1無し出力が無音");
        assert!(buf_b.iter().any(|s| s.abs() > 1e-4), "CC1有り出力が無音");
        let n = buf_a.len().min(buf_b.len());
        let differs = (0..n).any(|i| (buf_a[i] - buf_b[i]).abs() > 1e-4);
        assert!(differs, "発音中CC1が出力に反映されていない（ライブ伝播が効いていない）");
    }

    /// 代表的な CC/NRPN を一通り含む SMF がクラッシュせず発音することを確認する（スモーク）。
    #[test]
    fn mixed_cc_nrpn_smoke() {
        let bank = vibrato_bank();
        let events = vec![
            (0u32, vec![0x90, 60, 100]), // Note On
            (0, vec![0xB0, 99, 0]),      // NRPN MSB=0
            (0, vec![0xB0, 98, 23]),     // NRPN LSB=23（質感LFO Rate）
            (0, vec![0xB0, 6, 100]),     // Data Entry
            (0, vec![0xB0, 99, 0]),      // NRPN MSB=0
            (0, vec![0xB0, 98, 9]),      // NRPN LSB=9（Algorithm）
            (0, vec![0xB0, 6, 7]),       // Data Entry（alg7=全OP独立、op0キャリア維持）
            (10, vec![0xB0, 76, 90]),    // CC76 Vibrato Rate
            (10, vec![0xB0, 91, 60]),    // CC91 Reverb Send
            (0, vec![0xB0, 99, 0]),      // NRPN MSB=0
            (0, vec![0xB0, 98, 2]),      // NRPN LSB=2（Reverb Type）
            (0, vec![0xB0, 6, 3]),       // Data Entry
            (10, vec![0xD0, 40]),        // Channel Pressure（AT）
            (10, vec![0xA0, 60, 30]),    // Poly Pressure（AT）
            (10, vec![0xE0, 0x00, 0x50]), // Pitch Bend
            (240, vec![0x80, 60, 0]),    // Note Off
        ];
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, 8000.0, 0.1, Some(1.0), None).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "混在CC/NRPN出力が無音");
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// CC64(サステインペダル)ON中はNote Offが保留され、鳴り続けることを確認する。
    #[test]
    fn cc64_holds_release_while_pedal_down() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let tail_window = 200;

        let smf_no_pedal = build_smf(&[(0u32, vec![0x90, 69, 100]), (480u32, vec![0x80, 69, 0])]);
        let buf_no_pedal = render_smf(&smf_no_pedal, &bank, sr, 0.2, Some(1.0), None).unwrap();
        let no_pedal_tail_rms = rms(&buf_no_pedal[buf_no_pedal.len() - tail_window..]);
        assert!(no_pedal_tail_rms < 1e-3, "ペダル無しなのに末尾で鳴り続けている: rms={no_pedal_tail_rms}");

        let smf_pedal = build_smf(&[
            (0u32, vec![0xB0, 64, 127]), // CC64 ON
            (0u32, vec![0x90, 69, 100]),
            (480u32, vec![0x80, 69, 0]),
        ]);
        let buf_pedal = render_smf(&smf_pedal, &bank, sr, 0.2, Some(1.0), None).unwrap();
        let pedal_tail_rms = rms(&buf_pedal[buf_pedal.len() - tail_window..]);
        assert!(pedal_tail_rms > 0.05, "ペダル保持中なのに音が消えている: rms={pedal_tail_rms}");
    }

    /// 弾き直し（同ノートの再Note-On）でペダル保留ビットがクリアされることを確認する。
    #[test]
    fn cc64_retrigger_clears_pending_release_before_pedal_up() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let tail_window = 200;

        let smf = build_smf(&[
            (0u32, vec![0xB0, 64, 127]),  // CC64 ON
            (0u32, vec![0x90, 69, 100]),  // Note On
            (240u32, vec![0x80, 69, 0]),  // Note Off（保留になる）
            (10u32, vec![0x90, 69, 100]), // 弾き直し（鍵盤はまだ押されたまま）
            (230u32, vec![0xB0, 64, 0]),  // CC64 OFF（鍵盤はまだ押されているので切れてはいけない）
        ]);
        let buf = render_smf(&smf, &bank, sr, 0.2, Some(1.0), None).unwrap();
        let tail_rms = rms(&buf[buf.len() - tail_window..]);
        assert!(
            tail_rms > 0.05,
            "弾き直した鍵盤がまだ押されているのにペダルアップで音が消えた（stale pending_releaseの再発）: rms={tail_rms}"
        );
    }

    /// CC66(Sostenuto)はON時点で押下中のノートのみ保持し、ON後に新規キーオンしたノートは対象外。
    #[test]
    fn cc66_sostenuto_holds_only_notes_down_at_press() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let tail_window = 200;

        let smf_latched = build_smf(&[
            (0u32, vec![0x90, 69, 100]),
            (10u32, vec![0xB0, 66, 127]),
            (10u32, vec![0x80, 69, 0]),
        ]);
        let buf_latched = render_smf(&smf_latched, &bank, sr, 0.2, None, None).unwrap();
        let tail_latched = rms(&buf_latched[buf_latched.len() - tail_window..]);
        assert!(tail_latched > 0.05, "CC66 ON時点で押下中のノートが保持されていない: rms={tail_latched}");

        let smf_unlatched = build_smf(&[
            (0u32, vec![0xB0, 66, 127]),
            (10u32, vec![0x90, 72, 100]),
            (240u32, vec![0x80, 72, 0]),
        ]);
        let buf_unlatched = render_smf(&smf_unlatched, &bank, sr, 0.2, None, None).unwrap();
        let tail_unlatched = rms(&buf_unlatched[buf_unlatched.len() - tail_window..]);
        assert!(
            tail_unlatched < 1e-3,
            "CC66 ON後に弾いたノートがsostenutoに保持されてしまっている: rms={tail_unlatched}"
        );
    }

    /// CC66(Sostenuto)とCC64(Sustain)を併用したとき、両方OFFで初めて解放される。
    #[test]
    fn cc66_cc64_combined_release_requires_both_off() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let tail_window = 200;

        let smf_cc64_still_down = build_smf(&[
            (0u32, vec![0x90, 69, 100]),
            (10u32, vec![0xB0, 66, 127]),
            (10u32, vec![0x80, 69, 0]),
            (10u32, vec![0xB0, 64, 127]),
            (10u32, vec![0xB0, 66, 0]),
        ]);
        let buf1 = render_smf(&smf_cc64_still_down, &bank, sr, 0.2, None, None).unwrap();
        let tail1 = rms(&buf1[buf1.len() - tail_window..]);
        assert!(tail1 > 0.05, "CC64保持中なのにCC66 OFFで音が消えた: rms={tail1}");

        let smf_both_off = build_smf(&[
            (0u32, vec![0x90, 69, 100]),
            (10u32, vec![0xB0, 66, 127]),
            (10u32, vec![0x80, 69, 0]),
            (10u32, vec![0xB0, 64, 127]),
            (10u32, vec![0xB0, 66, 0]),
            (10u32, vec![0xB0, 64, 0]),
        ]);
        let buf2 = render_smf(&smf_both_off, &bank, sr, 0.2, None, None).unwrap();
        let tail2 = rms(&buf2[buf2.len() - tail_window..]);
        assert!(tail2 < 1e-3, "両ペダルOFF後も音が消えない: rms={tail2}");
    }

    /// CC67(Soft Pedal)ON中に発音したノートは実効TL（キャリア）が減算され出力が小さくなる。
    #[test]
    fn cc67_soft_reduces_output() {
        use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
        let mut patch = Op505Patch::default();
        patch.channel.algorithm = 7;
        patch.operators[0].tl = 180; // 減算後も無音にならない中間値
        patch.operators[0].mul = 1;
        let mut stages = [TimeStage::default(); MAX_STAGES];
        stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
        patch.operators[0].eg =
            TimeEgParams { stages, stage_count: 1, loop_enabled: 0, loop_start: 0, loop_end: 0, release_start: 0 };
        let bank = PatchBank::from_patches(&[patch]).unwrap();
        let sr = 8000.0;
        let tail_secs = 0.05;
        let window = 200;

        let smf_hard = build_smf(&[(0u32, vec![0x90, 69, 100])]);
        let buf_hard = render_smf(&smf_hard, &bank, sr, tail_secs, None, None).unwrap();
        let rms_hard = rms(&buf_hard[buf_hard.len() - window..]);

        let smf_soft = build_smf(&[
            (0u32, vec![0xB0, 67, 127]), // CC67 ON（最大深さ）
            (1u32, vec![0x90, 69, 100]),
        ]);
        let buf_soft = render_smf(&smf_soft, &bank, sr, tail_secs, None, None).unwrap();
        let rms_soft = rms(&buf_soft[buf_soft.len() - window..]);

        assert!(
            rms_soft < rms_hard * 0.9,
            "Soft Pedal(深さ127)でも音量が下がっていない: hard={rms_hard} soft={rms_soft}"
        );
    }

    /// CC120(All Sound Off)はリリースを経ず即座に消音し、CC123(All Notes Off)は通常の
    /// Note-Off相当でReleaseしながら減衰することを確認する（区別）。
    #[test]
    fn cc120_silences_immediately_vs_cc123_releases() {
        use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
        // 2段EG：段0(time=0)で即フルレベルへ到達しサステイン点で静止、note_offで段1(約8ms)へ
        // 向けて減衰する（rr=255相当の高速リリースだが、head_window(2ms)より確実に長くして
        // 「即時消音(CC120)」と「短いが有限のリリース(CC123)」を区別できるようにする）。
        let mut patch = Op505Patch::default();
        patch.channel.algorithm = 7;
        patch.operators[0].tl = 220;
        patch.operators[0].mul = 1;
        let mut stages = [TimeStage::default(); MAX_STAGES];
        stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
        stages[1] = TimeStage { time: 52, level: 0, curve: 0 }; // time_to_seconds(52) ≈ 12.6ms

        patch.operators[0].eg = TimeEgParams {
            stages,
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 0,
            loop_end: 0,
            release_start: 1,
        };
        let bank = PatchBank::from_patches(&[patch]).unwrap();
        let sr = 8000.0;
        let tail_secs = 0.03;
        let tail_len = (tail_secs * sr) as usize;
        let head_window = 16;

        let smf_120 = build_smf(&[(0u32, vec![0x90, 69, 100]), (240u32, vec![0xB0, 120, 127])]);
        let buf_120 = render_smf(&smf_120, &bank, sr, tail_secs, None, None).unwrap();
        let steady_120 = rms(&buf_120[buf_120.len() - tail_len - head_window..buf_120.len() - tail_len]);
        let head_120 = rms(&buf_120[buf_120.len() - tail_len..buf_120.len() - tail_len + head_window]);
        assert!(
            head_120 < steady_120 * 0.01,
            "CC120(All Sound Off)直後に即座消音していない: steady={steady_120} head={head_120}"
        );

        let smf_123 = build_smf(&[(0u32, vec![0x90, 69, 100]), (240u32, vec![0xB0, 123, 127])]);
        let buf_123 = render_smf(&smf_123, &bank, sr, tail_secs, None, None).unwrap();
        let steady_123 = rms(&buf_123[buf_123.len() - tail_len - head_window..buf_123.len() - tail_len]);
        let head_123 = rms(&buf_123[buf_123.len() - tail_len..buf_123.len() - tail_len + head_window]);
        assert!(
            head_123 > steady_123 * 0.1,
            "CC123(All Notes Off)直後はReleaseで鳴っているはず: steady={steady_123} head={head_123}"
        );
    }

    /// CC121(Reset All Controllers)は保留中のペダル保持ノートを解放し、pedal_down等の
    /// ③層状態をリセットすることを確認する。
    #[test]
    fn cc121_resets_pedals_and_releases_pending_notes() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let tail_window = 200;

        let smf_pending_release = build_smf(&[
            (0u32, vec![0xB0, 64, 127]),
            (10u32, vec![0x90, 69, 100]),
            (240u32, vec![0x80, 69, 0]),
            (10u32, vec![0xB0, 121, 0]),
        ]);
        let buf1 = render_smf(&smf_pending_release, &bank, sr, 0.2, None, None).unwrap();
        let tail1 = rms(&buf1[buf1.len() - tail_window..]);
        assert!(tail1 < 1e-3, "CC121で保留ノートが解放されていない: rms={tail1}");

        let smf_pedal_state = build_smf(&[
            (0u32, vec![0xB0, 64, 127]),
            (10u32, vec![0xB0, 121, 0]),
            (10u32, vec![0x90, 69, 100]),
            (240u32, vec![0x80, 69, 0]),
        ]);
        let buf2 = render_smf(&smf_pedal_state, &bank, sr, 0.2, None, None).unwrap();
        let tail2 = rms(&buf2[buf2.len() - tail_window..]);
        assert!(
            tail2 < 1e-3,
            "CC121後もpedal_downが残っている（新規ノートが保持されてしまう）: rms={tail2}"
        );
    }
}
