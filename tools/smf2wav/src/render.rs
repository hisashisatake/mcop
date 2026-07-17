//! SMF を `ym38x6-core` で再生し、mono f32 バッファへレンダリングする。
//!
//! ボイスIDは `midi_channel*128 + note` とし、エンジンの HashMap チャンネル管理で
//! ポリフォニーをそのまま扱う（VST と同じID符号化方式）。
//! プログラムチェンジ → `PatchBank::patch(program)` を音色として使う。
//!
//! CC/NRPN の解釈は `ym38x6-vst`（38x6 VST3/CLAP プラグイン）と同等にしてあり、DAW等で
//! VST を使って書き込んだ「パッチ外の揺れ」（ビブラート/トレモロ/オートワウ/質感LFO/
//! アフタータッチ/エフェクト）をマルチティンバーで再現する（フェーズ7ステップ9）。
//! VST はプラグイン単一音色前提でシャドウ状態をプラグイングローバルに持つが、smf2wav は
//! マルチティンバーのため [`ChannelState`] を **MIDIチャンネル別** に16個持つ。
//!
//! 対応SMFイベント:
//!   - Note On/Off、Program Change（CC102代替含む）、Tempo
//!   - Pitch Bend（チャンネル単位、RPN(0,0)でセンシティビティ設定可）
//!   - CC1/76/77/78: Pitch FG（ビブラート）への演奏補正（spec-sound.md「演奏層による補正」）
//!   - CC7/CC11: GM2音量（実効ゲイン=(cc7/127)²×(cc11/127)²）
//!   - CC64 Sustain / CC66 Sostenuto / CC67 Soft Pedal（ホールドフラグ方式）
//!   - CC91/93 + NRPN(0,2)〜(0,8): マスターエフェクト（Reverb/Chorus、`MasterEffects`）
//!   - NRPN(0,0)〜(0,33): 質感LFO・FG Loop/Curve・Algorithm/Waveform/Filter/AT/OP F-Number
//!   - Channel Pressure / Poly Key Pressure: AT Destination（NRPN(0,16)/(0,17)）
//!   - CC103〜106: Operator Key On/Off
//!   - CC120 All Sound Off（即時消音）/ CC121 Reset All Controllers（③層のみ）/
//!     CC123 All Notes Off（リリース）
//!
//! CC/NRPN の変更は当該チャンネルの発音中ボイスへ即時伝播する（[`apply_live`]）。
//! smf2wav はイベント境界で正確にレンダリングするため、VSTのブロック量子化より正確。

use std::collections::HashMap;

use ym38x6_core::{
    cc76_to_rate_scale, lfo_fade_mode_from_index, AudioProcessor, ChorusType, MasterEffects,
    ReverbType, Vco, Ym38x6Engine, Ym38x6Patch,
};

use crate::bank::PatchBank;
use crate::midi::{
    apply_expression_modulation, apply_soft_pedal, cc_to_u7, cc_to_u8, channel_gain,
    ExpressionDestination, RpnSelection,
};
use crate::smf::{parse_smf, EvKind};

/// MIDIノート番号の総数（0〜127）。ノート番号をそのままボイスIDの下位に使うため、
/// 発音中ボイス走査ループの上限に使う（VST の `MIDI_NOTE_COUNT` と同じ）。
const MIDI_NOTE_COUNT: usize = 128;

/// 1つのMIDIチャンネルの解釈状態（CC/NRPN シャドウ）。16個で全チャンネルを管理する。
///
/// VST（`ym38x6-vst/src/lib.rs`）のプラグイングローバル・シャドウフィールドを
/// **チャンネル別** に持ち直したもの。NRPN で上書きされる離散/焼き込みフィールドは
/// `Option`（None=ベースパッチ値のまま＝「NRPN は現在のパッチの当該フィールドのみ書き換え」）、
/// CC1/76/77/78 の Pitch FG 補正は中立既定の加算値として常時適用する。
struct ChannelState {
    /// 現在のプログラム番号（Program Change / CC102 で更新）。
    program: u8,

    // --- RPN/NRPN 選択状態 ---
    rpn_selection: RpnSelection,
    nrpn_msb: u8,
    nrpn_lsb: u8,
    rpn_msb: u8,
    rpn_lsb: u8,
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
    pitch_fg_cc1: u8,            // CC1 Modulation Wheel（0〜127）
    pitch_fg_cc76: u8,           // CC76 Vibrato Rate（0〜127、64=無補正）
    pitch_fg_cc77_depth_add: u8, // CC77 Vibrato Depth（0〜255、Depthへ0起点加算）
    pitch_fg_cc78: u8,           // CC78 Vibrato Delay（0〜127、64=無補正）
    pitch_fg_rpn0_5: u8,         // RPN(0,5) Modulation Depth Range（GM2、既定64）

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
    pitch_fg_loop: Option<u8>,
    pitch_fg_curve: Option<u8>,
    cutoff_fg_loop: Option<u8>,
    cutoff_fg_curve: Option<u8>,
    gain_fg_loop: Option<u8>,
    gain_fg_curve: Option<u8>,
    /// OP単位F-Number上書き（NRPN(0,18)〜(0,21)、13bit、Some時のみ set_operator_f_number）。
    operator_f_number_override: [Option<u16>; 4],

    // --- アフタータッチ ---
    at_destination: ExpressionDestination,
    poly_at_destination: ExpressionDestination,
    channel_pressure: u8,
    poly_pressure: HashMap<u8, u8>,

    // --- CC2(ブレス)/CC4(フット) ---（NRPN(0,34)/(0,35)で行先選択、既定はCC2→TLキャリア一括／
    // CC4→Filter Cutoff＝手動ワウ）
    cc2: u8,
    cc4: u8,
    cc2_destination: ExpressionDestination,
    cc4_destination: ExpressionDestination,

    // --- ペダル（CC64 Sustain / CC66 Sostenuto / CC67 Soft）---
    /// 物理的に押下中の鍵（bit N = note N。NoteOnでセット、NoteOffでクリア）。
    keys_down: u128,
    pedal_down: bool,
    /// ペダル（CC64またはCC66）保持中に保留したノートオフ（bit N = note N が保留中）。
    pending_release: u128,
    /// CC66 ON時点でkeys_downをスナップショットしたノート（bit N = note N）。CC66 OFFで解除。
    sostenuto: u128,
    /// Soft Pedal（CC67）の深さ（0〜127、0=無効）。
    cc67: u8,
    /// cc67>0の間にNote-Onしたノート（bit N = note N）。実効TL/Cutoff減算の対象。
    soft_notes: u128,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            program: 0,
            rpn_selection: RpnSelection::None,
            nrpn_msb: 0xFF,
            nrpn_lsb: 0xFF,
            rpn_msb: 0xFF,
            rpn_lsb: 0xFF,
            data_entry_msb: 0,
            data_entry_lsb: 0,
            pitch_bend_range: 2.0,
            bend_cents: 0.0,
            cc7: 127,
            cc11: 127,
            pitch_fg_cc1: 0,
            pitch_fg_cc76: 64,
            pitch_fg_cc77_depth_add: 0,
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
            pitch_fg_loop: None,
            pitch_fg_curve: None,
            cutoff_fg_loop: None,
            cutoff_fg_curve: None,
            gain_fg_loop: None,
            gain_fg_curve: None,
            operator_f_number_override: [None; 4],
            at_destination: ExpressionDestination::default(),
            poly_at_destination: ExpressionDestination::default(),
            channel_pressure: 0,
            poly_pressure: HashMap::new(),
            cc2: 0,
            cc4: 0,
            cc2_destination: ExpressionDestination::TlCarriers,
            cc4_destination: ExpressionDestination::FilterCutoff,
            keys_down: 0,
            pedal_down: false,
            pending_release: 0,
            sostenuto: 0,
            cc67: 0,
            soft_notes: 0,
        }
    }
}

/// ベースパッチ（プログラムの音色）に、チャンネルの CC/NRPN シャドウ上書きと Pitch FG 演奏補正を
/// 重ねた実効パッチを組み立てる。VST の `build_patch()` を移植したもの。
///
/// AT（アフタータッチ）と CC76→rate_scale と OP F-Number は `ChannelParams` 外なので、
/// ここでは扱わず [`apply_live`]/[`note_on_voice`] 側で適用する。
fn build_effective_patch(base: &Ym38x6Patch, st: &ChannelState) -> Ym38x6Patch {
    let mut patch = *base;

    // 離散上書き（Some のときのみ）
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

    // FG Loop/Curve 上書き（Some のときのみ）
    if let Some(v) = st.pitch_fg_loop {
        patch.channel.pitch_fg.eg.loop_enabled = v;
    }
    if let Some(v) = st.pitch_fg_curve {
        patch.channel.pitch_fg.eg.curve = v;
    }
    if let Some(v) = st.cutoff_fg_loop {
        patch.channel.cutoff_fg.eg.loop_enabled = v;
    }
    if let Some(v) = st.cutoff_fg_curve {
        patch.channel.cutoff_fg.eg.curve = v;
    }
    if let Some(v) = st.gain_fg_loop {
        patch.channel.gain_fg.loop_enabled = v;
    }
    if let Some(v) = st.gain_fg_curve {
        patch.channel.gain_fg.curve = v;
    }

    // Pitch FG 演奏補正（CC1/77/78）。VST build_patch() 292-301行と同一計算。
    // CC76(Rate)は rate_scale 経由なのでここでは扱わない。
    let delay_delta = st.pitch_fg_cc78 as i32 - 64;
    patch.channel.pitch_fg.eg.delay =
        (patch.channel.pitch_fg.eg.delay as i32 + delay_delta).clamp(0, 255) as u8;
    // CC1 のセント換算分を Depth と同じ 0〜255 単位空間へ逆変換して加算する
    // （Pitch FG の (depth-128)/128*1200 セント変換式の逆算）。
    let cc1_cents = (st.pitch_fg_cc1 as f32 / 127.0) * (st.pitch_fg_rpn0_5 as f32 * 50.0 / 64.0);
    let cc1_depth_units = (cc1_cents / 1200.0 * 128.0).round() as i32;
    patch.channel.pitch_fg.depth = (patch.channel.pitch_fg.depth as i32
        + st.pitch_fg_cc77_depth_add as i32
        + cc1_depth_units)
        .clamp(0, 255) as u8;

    patch
}

/// CC/NRPN で変わった実効パッチ・CC76 rate_scale・AT・OP F-Number を、そのチャンネルの
/// 発音中ボイス全てへ伝播する（ライブ反映）。VST の毎ブロック伝播ループ（770-789行）を
/// 1チャンネル分に絞ったもの。非発音スロットへの set_* はエンジン側で no-op になる。
fn apply_live(engine: &mut Ym38x6Engine, chi: usize, st: &ChannelState, bank: &PatchBank) {
    let base = *bank.patch(st.program);
    let eff = build_effective_patch(&base, st);
    let rate_scale = cc76_to_rate_scale(st.pitch_fg_cc76);
    for note in 0..MIDI_NOTE_COUNT {
        let id = chi * 128 + note;
        let mut note_patch = eff;
        apply_expression_modulation(
            note as u8,
            &[
                (st.cc2, st.cc2_destination),
                (st.cc4, st.cc4_destination),
                (st.channel_pressure, st.at_destination),
            ],
            st.poly_at_destination,
            &st.poly_pressure,
            &mut note_patch,
        );
        // Soft Pedal（CC67）: NoteOn時点でcc67>0だったノートのみ、現在のcc67深さで
        // 実効TL(キャリア)/Cutoffを減算し続ける（apply_liveの毎回上書きでNoteOn時の
        // 減算が消えないようにする。深さは現在値を使う簡易版、spec-sound.md参照）。
        if st.soft_notes & (1u128 << note) != 0 {
            apply_soft_pedal(&mut note_patch, st.cc67);
        }
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

/// 1ボイスのノートオン。VST の note_on 適用（845-866行）と同型。
/// 現在の AT を焼き込んだ実効パッチで発音し、ベンド量・音量ゲイン・rate_scale・
/// OP F-Number を新ボイスへ反映する。
fn note_on_voice(
    engine: &mut Ym38x6Engine,
    chi: usize,
    note: u8,
    vel: u8,
    st: &ChannelState,
    bank: &PatchBank,
) {
    let base = *bank.patch(st.program);
    let mut eff = build_effective_patch(&base, st);
    // このノートの現在の AT/CC2/CC4 を焼き込む（以降の変化は apply_live で伝播）。
    apply_expression_modulation(
        note,
        &[
            (st.cc2, st.cc2_destination),
            (st.cc4, st.cc4_destination),
            (st.channel_pressure, st.at_destination),
        ],
        st.poly_at_destination,
        &st.poly_pressure,
        &mut eff,
    );
    // Soft Pedal（CC67）: NoteOn時点でcc67>0のノートのみ実効TL(キャリア)/Cutoffを減算する
    // （spec-sound.md「Soft Pedal（CC67）」）。soft_notesは呼び出し側でNoteOn時に立てる。
    if st.soft_notes & (1u128 << note) != 0 {
        apply_soft_pedal(&mut eff, st.cc67);
    }
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

/// CC99/98(NRPN)・CC101/100(RPN) 受信時に選択状態を更新する。
/// MSB,LSB=127,127（Null）の場合は選択解除。VST `update_rpn_selection` の移植。
fn update_rpn_selection(st: &mut ChannelState, is_nrpn: bool) {
    let (msb, lsb) = if is_nrpn { (st.nrpn_msb, st.nrpn_lsb) } else { (st.rpn_msb, st.rpn_lsb) };
    st.rpn_selection = if msb == 127 && lsb == 127 {
        RpnSelection::None
    } else if is_nrpn {
        RpnSelection::Nrpn(msb, lsb)
    } else {
        RpnSelection::Rpn(msb, lsb)
    };
}

/// NRPN(0,18)〜(0,21)：CC6(MSB)+CC38(LSB)の14bit値を13bit(0〜8191)にclampして
/// OP F-Number 上書きとして記録する（実際の発音中ボイスへの反映は `apply_live`）。
fn set_operator_f_number_override(st: &mut ChannelState, op_index: usize) {
    let combined = (st.data_entry_msb as u16) * 128 + st.data_entry_lsb as u16;
    st.operator_f_number_override[op_index] = Some(combined.min(8191));
}

/// CC6(Data Entry MSB)受信時、選択中のRPN/NRPNに応じて値を適用する。VST `handle_data_entry` の移植。
/// 戻り値=発音中ボイスへの伝播（`apply_live`）が必要かどうか。エフェクト系NRPNやRPN(0,0)は不要。
fn handle_data_entry(st: &mut ChannelState, effects: &mut MasterEffects, value: u8) -> bool {
    st.data_entry_msb = cc_to_u7(value);
    match st.rpn_selection {
        // RPN(0,0): Pitch Bend Sensitivity（半音）。次のベンドイベントで効くので伝播不要。
        RpnSelection::Rpn(0, 0) => {
            st.pitch_bend_range = cc_to_u7(value) as f32;
            false
        }
        // RPN(0,5): Modulation Depth Range（CC1 セント換算係数）。CC1 適用へ影響するため伝播。
        RpnSelection::Rpn(0, 5) => {
            st.pitch_fg_rpn0_5 = cc_to_u7(value);
            true
        }
        // NRPN(0,0): 質感LFO Destination（0=Pitch/1=Volume/2=TLキャリア/3=Cutoff）。
        RpnSelection::Nrpn(0, 0) => {
            st.texture_lfo_destination = Some(cc_to_u7(value).min(3));
            true
        }
        // NRPN(0,1): 質感LFO Waveform（0〜4）。
        RpnSelection::Nrpn(0, 1) => {
            st.texture_lfo_waveform = Some(cc_to_u7(value).min(4));
            true
        }
        // NRPN(0,2): Reverb Type（master）。
        RpnSelection::Nrpn(0, 2) => {
            effects.set_reverb_type(ReverbType::from_u8(cc_to_u7(value)));
            false
        }
        // NRPN(0,3): Chorus Type（master）。
        RpnSelection::Nrpn(0, 3) => {
            effects.set_chorus_type(ChorusType::from_u8(cc_to_u7(value)));
            false
        }
        // NRPN(0,4): Reverb Time（master）。
        RpnSelection::Nrpn(0, 4) => {
            effects.set_reverb_time(cc_to_u8(value));
            false
        }
        // NRPN(0,5): Chorus Mod Rate（master）。
        RpnSelection::Nrpn(0, 5) => {
            effects.set_chorus_mod_rate(cc_to_u8(value));
            false
        }
        // NRPN(0,6): Chorus Mod Depth（master）。
        RpnSelection::Nrpn(0, 6) => {
            effects.set_chorus_mod_depth(cc_to_u8(value));
            false
        }
        // NRPN(0,7): Chorus Feedback（master）。
        RpnSelection::Nrpn(0, 7) => {
            effects.set_chorus_feedback(cc_to_u8(value));
            false
        }
        // NRPN(0,8): Chorus Send To Reverb（master）。
        RpnSelection::Nrpn(0, 8) => {
            effects.set_chorus_send_to_reverb(cc_to_u8(value));
            false
        }
        // NRPN(0,9): Algorithm（0〜7）。
        RpnSelection::Nrpn(0, 9) => {
            st.algorithm = Some(cc_to_u7(value).min(7));
            true
        }
        // NRPN(0,10)〜(0,13): Waveform Op0〜3（0〜255）。
        RpnSelection::Nrpn(0, lsb @ 10..=13) => {
            st.operator_waveforms[(lsb - 10) as usize] = Some(cc_to_u8(value));
            true
        }
        // NRPN(0,14): Filter Type（0=LP/1=HP/2=BP）。
        RpnSelection::Nrpn(0, 14) => {
            st.filter_type = Some(cc_to_u7(value).min(2));
            true
        }
        // NRPN(0,15): Filter Self-Oscillation（0=OFF/1=ON）。
        RpnSelection::Nrpn(0, 15) => {
            st.filter_self_oscillation = Some(cc_to_u7(value) != 0);
            true
        }
        // NRPN(0,16): AT Destination（Channel Pressureの加算先）。
        RpnSelection::Nrpn(0, 16) => {
            st.at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        // NRPN(0,17): Poly AT Destination（Poly Key Pressureの加算先）。
        RpnSelection::Nrpn(0, 17) => {
            st.poly_at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        // NRPN(0,18)〜(0,21): Operator F-Number Op0〜3（CC6=MSB、CC38=LSBで14bit→13bit）。
        RpnSelection::Nrpn(0, lsb @ 18..=21) => {
            set_operator_f_number_override(st, (lsb - 18) as usize);
            true
        }
        // NRPN(0,22): 質感LFO Fade Mode（0=ON-IN/1=ON-OUT/2=OFF-IN/3=OFF-OUT）。
        RpnSelection::Nrpn(0, 22) => {
            st.texture_lfo_fade_mode = Some(lfo_fade_mode_from_index(cc_to_u7(value)) as u8);
            true
        }
        // NRPN(0,23)〜(0,27): 質感LFO Rate/Depth/Delay/FadeTime/Offset（焼き込み専用）。
        RpnSelection::Nrpn(0, 23) => {
            st.texture_lfo_rate = Some(cc_to_u8(value));
            true
        }
        RpnSelection::Nrpn(0, 24) => {
            st.texture_lfo_depth = Some(cc_to_u8(value));
            true
        }
        RpnSelection::Nrpn(0, 25) => {
            st.texture_lfo_delay = Some(cc_to_u8(value));
            true
        }
        RpnSelection::Nrpn(0, 26) => {
            st.texture_lfo_fade_time = Some(cc_to_u8(value));
            true
        }
        RpnSelection::Nrpn(0, 27) => {
            st.texture_lfo_offset = Some(cc_to_u8(value));
            true
        }
        // NRPN(0,28)〜(0,33): Pitch/Cutoff/Gain FG Loop/Curve（0=OFF/1=ON）。
        RpnSelection::Nrpn(0, 28) => {
            st.pitch_fg_loop = Some(u8::from(cc_to_u7(value) != 0));
            true
        }
        RpnSelection::Nrpn(0, 29) => {
            st.pitch_fg_curve = Some(u8::from(cc_to_u7(value) != 0));
            true
        }
        RpnSelection::Nrpn(0, 30) => {
            st.cutoff_fg_loop = Some(u8::from(cc_to_u7(value) != 0));
            true
        }
        RpnSelection::Nrpn(0, 31) => {
            st.cutoff_fg_curve = Some(u8::from(cc_to_u7(value) != 0));
            true
        }
        RpnSelection::Nrpn(0, 32) => {
            st.gain_fg_loop = Some(u8::from(cc_to_u7(value) != 0));
            true
        }
        RpnSelection::Nrpn(0, 33) => {
            st.gain_fg_curve = Some(u8::from(cc_to_u7(value) != 0));
            true
        }
        // NRPN(0,34): CC2(ブレス)Destination。
        RpnSelection::Nrpn(0, 34) => {
            st.cc2_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        // NRPN(0,35): CC4(フット)Destination。既定Filter Cutoff＝手動ワウ。
        RpnSelection::Nrpn(0, 35) => {
            st.cc4_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            true
        }
        _ => false,
    }
}

/// レンダリング済みサンプル位置を `target` まで進め、その区間をマスターエフェクトに通す。
/// チャンクはイベント境界に揃うため、エフェクトのオートメーションがサンプル正確に反映される。
fn render_chunk(
    engine: &mut Ym38x6Engine,
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
) -> Result<Vec<f32>, String> {
    let (division, events) = parse_smf(data)?;

    let mut engine = Ym38x6Engine::new(sample_rate);
    // SMF内蔵のマスターエフェクト（CC91/93・NRPN(0,2)〜(0,8)で駆動）。既定 send=0 で透過。
    // main.rs の `--reverb-*`（fx.rs）はこれとは独立した後段の診断用リバーブ。
    let mut effects = MasterEffects::new(sample_rate);
    let mut out: Vec<f32> = Vec::new();

    let mut channels: Vec<ChannelState> = (0..16).map(|_| ChannelState::default()).collect();

    let mut tempo_us: f64 = 500_000.0; // 既定 120BPM
    let mut spt = tempo_us / 1_000_000.0 * sample_rate as f64 / division as f64; // samples/tick
    let mut cur_tick: u64 = 0;
    let mut sample_pos: f64 = 0.0;
    let mut rendered: usize = 0;

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
                return Ok(out);
            }
        }
        render_chunk(&mut engine, &mut effects, &mut out, &mut rendered, target);

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
                let bit = 1u128 << note;
                // 弾き直したらペダル保留を解除する（離鍵→ペダルアップ前に再度弾いた場合、
                // 古い保留ビットが残っていると鍵盤を押している最中にペダルアップでnote_offが
                // 誤発火する。spec-sound.md「サステインペダル（CC64）の実装方針」参照）。
                channels[chi].pending_release &= !bit;
                channels[chi].keys_down |= bit;
                // Soft Pedal（CC67）: ON中に新規キーオンしたノートのみ対象（spec-sound.md参照）。
                if channels[chi].cc67 > 0 {
                    channels[chi].soft_notes |= bit;
                } else {
                    channels[chi].soft_notes &= !bit;
                }
                note_on_voice(&mut engine, chi, note, vel, &channels[chi], bank);
            }
            EvKind::NoteOff(ch, note) => {
                let chi = ch as usize;
                let id = chi * 128 + note as usize;
                let bit = 1u128 << note;
                channels[chi].poly_pressure.remove(&note);
                channels[chi].keys_down &= !bit;
                let held = channels[chi].pedal_down || channels[chi].sostenuto & bit != 0;
                if held {
                    // いずれかのペダル保持中: Note Offを保留する
                    channels[chi].pending_release |= bit;
                } else {
                    engine.note_off(id);
                    channels[chi].soft_notes &= !bit;
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
                channels[chi].poly_pressure.insert(note, cc_to_u8(value));
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
    Ok(out)
}

/// `candidates & pending_release` のうち、`other_held`（他のペダルが保持中のノート）にも
/// `keys_down`（再押下中のノート）にも該当しないものを note_off して `pending_release`/
/// `soft_notes` から除去する。CC64/CC66 OFF・CC121 の解放処理で共有する
/// （CC64 OFF: candidates=pending_release, other_held=sostenuto。
/// 　CC66 OFF: candidates=sostenuto, other_held=pedal_down相当の全ビット。
/// 　CC121: candidates=pending_release, other_held=0）。
fn release_unheld(engine: &mut Ym38x6Engine, st: &mut ChannelState, chi: usize, candidates: u128, other_held: u128) {
    let mut mask = candidates & st.pending_release;
    while mask != 0 {
        let note = mask.trailing_zeros() as usize;
        let bit = 1u128 << note;
        mask &= mask - 1;
        if other_held & bit == 0 && st.keys_down & bit == 0 {
            engine.note_off(chi * 128 + note);
            st.pending_release &= !bit;
            st.soft_notes &= !bit;
        }
    }
}

/// 1つのコントロールチェンジを処理する。VST の process() 内 CC match（896-978行）＋
/// handle_data_entry を per-channel に移植したもの。
#[allow(clippy::too_many_arguments)]
fn handle_control_change(
    engine: &mut Ym38x6Engine,
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
            channels[chi].pitch_fg_cc77_depth_add = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        78 => {
            channels[chi].pitch_fg_cc78 = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        // CC2(ブレス)/CC4(フット): Expression Destination（NRPN(0,34)/(0,35)）へ加算 → 発音中ボイスへ伝播。
        // 既定はCC2→TLキャリア一括、CC4→Filter Cutoff（手動ワウ）。
        2 => {
            channels[chi].cc2 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        4 => {
            channels[chi].cc4 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank);
        }
        // NRPN/RPN 選択（CC99/98=NRPN MSB/LSB、CC101/100=RPN MSB/LSB）
        98 => {
            channels[chi].nrpn_lsb = cc_to_u7(val);
            update_rpn_selection(&mut channels[chi], true);
        }
        99 => {
            channels[chi].nrpn_msb = cc_to_u7(val);
            update_rpn_selection(&mut channels[chi], true);
        }
        100 => {
            channels[chi].rpn_lsb = cc_to_u7(val);
            update_rpn_selection(&mut channels[chi], false);
        }
        101 => {
            channels[chi].rpn_msb = cc_to_u7(val);
            update_rpn_selection(&mut channels[chi], false);
        }
        // CC6 Data Entry MSB: 選択中の RPN/NRPN へ値を適用。
        6 => {
            if handle_data_entry(&mut channels[chi], effects, val) {
                apply_live(engine, chi, &channels[chi], bank);
            }
        }
        // CC38 Data Entry LSB: OP F-Number(NRPN 0,18〜21選択中)の下位7bit。
        38 => {
            channels[chi].data_entry_lsb = cc_to_u7(val);
            if let RpnSelection::Nrpn(0, lsb @ 18..=21) = channels[chi].rpn_selection {
                set_operator_f_number_override(&mut channels[chi], (lsb - 18) as usize);
                apply_live(engine, chi, &channels[chi], bank);
            }
        }
        // CC64 Sustain Pedal（ホールドフラグ方式）。sostenuto保持中・再押下中のノートは
        // release_unheldが残す（CC66/CC64を併用した場合の解放順序）。
        64 => {
            if val >= 64 {
                channels[chi].pedal_down = true;
            } else {
                channels[chi].pedal_down = false;
                let candidates = channels[chi].pending_release;
                let other_held = channels[chi].sostenuto;
                release_unheld(engine, &mut channels[chi], chi, candidates, other_held);
            }
        }
        // CC66 Sostenuto: ON時点でkeys_down中のノートのみをlatchし、CC66 OFF（かつ
        // CC64も踏まれていない）までReleaseに入らせない（spec-sound.md「Sostenuto（CC66）」）。
        66 => {
            if val >= 64 {
                channels[chi].sostenuto = channels[chi].keys_down;
            } else {
                let candidates = channels[chi].sostenuto;
                let other_held = if channels[chi].pedal_down { u128::MAX } else { 0 };
                release_unheld(engine, &mut channels[chi], chi, candidates, other_held);
                channels[chi].sostenuto = 0;
            }
        }
        // CC67 Soft Pedal: 深さを保持するのみ。ON中に新規キーオンしたノートのみへの
        // 適用はnote_on_voice/apply_live側（soft_notesビット）で行う（spec-sound.md参照）。
        67 => {
            channels[chi].cc67 = cc_to_u7(val);
        }
        // CC121 Reset All Controllers: ③ジェスチャー層のみリセットする（②パート状態・
        // ①音色は保持、spec-sound.md「補強規則」）。CC64/66/67ペダル・Pitch Bend・
        // CC1・アフタータッチが対象。CC2/CC4/CC7/CC11/CC76〜78/センド/RPN等は保持。
        121 => {
            let candidates = channels[chi].pending_release;
            release_unheld(engine, &mut channels[chi], chi, candidates, 0);
            channels[chi].pedal_down = false;
            channels[chi].sostenuto = 0;
            channels[chi].cc67 = 0;
            channels[chi].soft_notes = 0;
            channels[chi].pitch_fg_cc1 = 0;
            channels[chi].bend_cents = 0.0;
            channels[chi].channel_pressure = 0;
            channels[chi].poly_pressure.clear();
            engine.set_pitch_bend_group(chi, 0.0);
            apply_live(engine, chi, &channels[chi], bank);
        }
        // CC91/93: マスターエフェクト送りレベル（master）。
        91 => effects.set_reverb_send(cc_to_u8(val)),
        93 => effects.set_chorus_send(cc_to_u8(val)),
        // CC102: Program Change 代替（VST3で MidiProgramChange が届かないため。smf2wav では
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
            channels[chi].keys_down = 0;
            channels[chi].pending_release = 0;
            channels[chi].pedal_down = false;
            channels[chi].sostenuto = 0;
            channels[chi].soft_notes = 0;
        }
        // CC123 All Notes Off: 通常のNote-Off相当（リリースして自然減衰）。
        123 => {
            for note in 0..MIDI_NOTE_COUNT {
                engine.note_off(chi * 128 + note);
            }
            channels[chi].keys_down = 0;
            channels[chi].pending_release = 0;
            channels[chi].sostenuto = 0;
            channels[chi].soft_notes = 0;
        }
        // Bank Select（CC0/32）等は smf2wav では無視（単一バンクのため）。
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 中立の ChannelState では実効パッチがベースと一致する（Pitch FG 補正が全て無補正）。
    #[test]
    fn effective_patch_neutral_equals_base() {
        let base = Ym38x6Patch::default();
        let st = ChannelState::default();
        let eff = build_effective_patch(&base, &st);
        assert_eq!(eff, base);
    }

    /// CC77(Vibrato Depth)は Pitch FG Depth へ 0 起点で加算される（既定 depth=128）。
    #[test]
    fn cc77_adds_to_pitch_fg_depth() {
        let base = Ym38x6Patch::default();
        let mut st = ChannelState::default();
        st.pitch_fg_cc77_depth_add = 40;
        let eff = build_effective_patch(&base, &st);
        assert_eq!(eff.channel.pitch_fg.depth, 168); // 128 + 40
    }

    /// CC78(Vibrato Delay)は Pitch FG Delay へ 64 中心の相対補正。上下端で clamp する。
    #[test]
    fn cc78_shifts_pitch_fg_delay_with_clamp() {
        let mut base = Ym38x6Patch::default();
        base.channel.pitch_fg.eg.delay = 100;
        let mut st = ChannelState::default();
        st.pitch_fg_cc78 = 127; // +63
        assert_eq!(build_effective_patch(&base, &st).channel.pitch_fg.eg.delay, 163);
        st.pitch_fg_cc78 = 0; // -64 → clamp 0（100-64=36 なので下端ではないが計算確認）
        assert_eq!(build_effective_patch(&base, &st).channel.pitch_fg.eg.delay, 36);
        base.channel.pitch_fg.eg.delay = 10;
        assert_eq!(build_effective_patch(&base, &st).channel.pitch_fg.eg.delay, 0); // 10-64 clamp 0
    }

    /// CC1(Mod Wheel)は RPN(0,5) 係数でセント換算し Depth 単位空間へ加算する。
    #[test]
    fn cc1_adds_cents_scaled_depth() {
        let base = Ym38x6Patch::default();
        let mut st = ChannelState::default();
        st.pitch_fg_cc1 = 127; // 最大
        st.pitch_fg_rpn0_5 = 64; // ≈50セント
                                 // cc1_cents = 1.0 * (64*50/64) = 50、depth_units = round(50/1200*128)=5
        let eff = build_effective_patch(&base, &st);
        assert_eq!(eff.channel.pitch_fg.depth, 133);
    }

    /// NRPN(0,23) 質感LFO Rate は cc_to_u8（×2）でシャドウへ入り、実効パッチへ反映される。
    #[test]
    fn nrpn_texture_lfo_rate_into_patch() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        st.rpn_selection = RpnSelection::Nrpn(0, 23);
        let needs = handle_data_entry(&mut st, &mut fx, 100);
        assert!(needs);
        assert_eq!(st.texture_lfo_rate, Some(200));
        let eff = build_effective_patch(&Ym38x6Patch::default(), &st);
        assert_eq!(eff.channel.texture_lfo.rate, 200);
    }

    /// NRPN(0,9) Algorithm 上書きは実効パッチの algorithm を置き換える。
    #[test]
    fn nrpn_algorithm_override() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        st.rpn_selection = RpnSelection::Nrpn(0, 9);
        assert!(handle_data_entry(&mut st, &mut fx, 5));
        assert_eq!(st.algorithm, Some(5));
        let eff = build_effective_patch(&Ym38x6Patch::default(), &st);
        assert_eq!(eff.channel.algorithm, 5);
    }

    /// エフェクト系 NRPN(0,2〜8) はボイス伝播不要（false）を返す。
    #[test]
    fn nrpn_effects_return_no_voice_update() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        for lsb in [2u8, 3, 4, 5, 6, 7, 8] {
            st.rpn_selection = RpnSelection::Nrpn(0, lsb);
            assert!(!handle_data_entry(&mut st, &mut fx, 64), "NRPN(0,{lsb}) should not need voice update");
        }
    }

    /// NRPN(0,18)＝OP0 F-Number は CC6(MSB)+CC38(LSB)の14bit→13bit clamp。
    #[test]
    fn nrpn_operator_f_number_14bit() {
        let mut st = ChannelState::default();
        let mut fx = MasterEffects::new(44_100.0);
        st.rpn_selection = RpnSelection::Nrpn(0, 18);
        st.data_entry_lsb = 10;
        assert!(handle_data_entry(&mut st, &mut fx, 60)); // msb=60 → 60*128+10 = 7690
        assert_eq!(st.operator_f_number_override[0], Some(7690));
        // 8191 超は clamp
        st.data_entry_lsb = 127;
        assert!(handle_data_entry(&mut st, &mut fx, 127)); // 127*128+127 = 16383 → 8191
        assert_eq!(st.operator_f_number_override[0], Some(8191));
    }

    /// AT の TL キャリア一括加算（apply_expression_modulation の複製がVSTと同挙動）。
    #[test]
    fn at_tl_carriers_adds_pressure() {
        use std::collections::HashMap;
        let mut patch = Ym38x6Patch::default();
        patch.channel.algorithm = 7; // 全OPキャリア
        for op in patch.operators.iter_mut() {
            op.tl = 100;
        }
        apply_expression_modulation(
            60,
            &[(50, ExpressionDestination::TlCarriers)],
            ExpressionDestination::LfoPmd,
            &HashMap::new(),
            &mut patch,
        );
        for op in patch.operators.iter() {
            assert_eq!(op.tl, 150);
        }
    }

    /// CC4(フット)の既定行先 Filter Cutoff が filter_cutoff へ加算される（手動ワウ）。
    #[test]
    fn cc4_default_destination_adds_to_filter_cutoff() {
        use std::collections::HashMap;
        let mut patch = Ym38x6Patch::default();
        patch.channel.filter_cutoff = 100;
        let st = ChannelState { cc4: 80, ..ChannelState::default() };
        apply_expression_modulation(
            60,
            &[(st.cc2, st.cc2_destination), (st.cc4, st.cc4_destination)],
            st.poly_at_destination,
            &HashMap::new(),
            &mut patch,
        );
        assert_eq!(patch.channel.filter_cutoff, 180); // 100 + 80、既定CC4→FilterCutoff
    }

    /// CC2(ブレス)とAT(Channel Pressure)が同じ行先を指すとき、両方の値が加算される。
    #[test]
    fn cc2_and_at_same_destination_are_additive() {
        use std::collections::HashMap;
        let mut patch = Ym38x6Patch::default();
        patch.channel.algorithm = 7;
        for op in patch.operators.iter_mut() {
            op.tl = 50;
        }
        let mut st = ChannelState { cc2: 30, ..ChannelState::default() };
        st.at_destination = ExpressionDestination::TlCarriers; // 既定CC2行先(TlCarriers)と一致させる
        apply_expression_modulation(
            60,
            &[(st.cc2, st.cc2_destination), (st.cc4, st.cc4_destination), (20, st.at_destination)],
            st.poly_at_destination,
            &HashMap::new(),
            &mut patch,
        );
        for op in patch.operators.iter() {
            assert_eq!(op.tl, 100); // 50 + 30(CC2) + 20(AT)
        }
    }

    /// CC2/CC4 が 0（未操作）のときはパッチが無変化。
    #[test]
    fn cc2_cc4_zero_leaves_patch_unchanged() {
        use std::collections::HashMap;
        let base = Ym38x6Patch::default();
        let mut patch = base;
        let st = ChannelState::default();
        apply_expression_modulation(
            60,
            &[(st.cc2, st.cc2_destination), (st.cc4, st.cc4_destination)],
            st.poly_at_destination,
            &HashMap::new(),
            &mut patch,
        );
        assert_eq!(patch, base);
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

    /// ループする Pitch FG を持つ、発音する1音色バンクを作る。
    /// depth=128（中立）ではビブラートが出ず、CC1でdepthがずれると出る。
    fn vibrato_bank() -> PatchBank {
        let mut patch = Ym38x6Patch::default();
        patch.channel.algorithm = 7; // 全OP独立キャリア
        patch.operators[0].tl = 220;
        patch.operators[0].mul = 1;
        patch.operators[0].ar = 255;
        patch.operators[0].d1l = 255;
        patch.operators[0].rr = 200;
        patch.channel.pitch_fg.eg.loop_enabled = 1;
        patch.channel.pitch_fg.eg.ar = 220;
        patch.channel.pitch_fg.eg.d1r = 220;
        PatchBank::from_patches(&[patch]).unwrap()
    }

    /// 発音中に届いた CC1(Mod Wheel) が発音中ボイスへライブ伝播し、出力が変わることを確認する。
    /// note-on(tick0) → CC1(tick240、mid-note) → note-off(tick480)。CC1有無で後半が異なる。
    #[test]
    fn cc1_live_propagation_changes_sounding_voice() {
        let bank = vibrato_bank();
        let sr = 8000.0;
        let note_on = (0u32, vec![0x90, 69, 100]);
        let note_off = (480u32, vec![0x80, 69, 0]);

        // CC1 無し
        let smf_a = build_smf(&[note_on.clone(), note_off.clone()]);
        let buf_a = render_smf(&smf_a, &bank, sr, 0.1, Some(1.0)).unwrap();

        // tick240 で CC1=127（発音中）
        let smf_b = build_smf(&[
            note_on,
            (240u32, vec![0xB0, 1, 127]),
            (240u32, vec![0x80, 69, 0]), // note_off の delta を分割（合計480）
        ]);
        let buf_b = render_smf(&smf_b, &bank, sr, 0.1, Some(1.0)).unwrap();

        // どちらも無音でない
        assert!(buf_a.iter().any(|s| s.abs() > 1e-4), "CC1無し出力が無音");
        assert!(buf_b.iter().any(|s| s.abs() > 1e-4), "CC1有り出力が無音");
        // ライブ伝播が効いていれば波形が異なる
        let n = buf_a.len().min(buf_b.len());
        let differs = (0..n).any(|i| (buf_a[i] - buf_b[i]).abs() > 1e-4);
        assert!(differs, "発音中CC1が出力に反映されていない（ライブ伝播が効いていない）");
    }

    /// 代表的な CC/NRPN を一通り含む SMF がクラッシュせず発音することを確認する（スモーク）。
    #[test]
    fn mixed_cc_nrpn_smoke() {
        let bank = vibrato_bank();
        let events = vec![
            (0u32, vec![0x90, 60, 100]),               // Note On
            (0, vec![0xB0, 99, 0]),                    // NRPN MSB=0
            (0, vec![0xB0, 98, 23]),                   // NRPN LSB=23（質感LFO Rate）
            (0, vec![0xB0, 6, 100]),                   // Data Entry
            (0, vec![0xB0, 99, 0]),                    // NRPN MSB=0
            (0, vec![0xB0, 98, 9]),                    // NRPN LSB=9（Algorithm）
            (0, vec![0xB0, 6, 7]),                     // Data Entry（alg7=全OP独立、op0キャリア維持）
            (10, vec![0xB0, 76, 90]),                  // CC76 Vibrato Rate
            (10, vec![0xB0, 91, 60]),                  // CC91 Reverb Send
            (0, vec![0xB0, 99, 0]),                    // NRPN MSB=0
            (0, vec![0xB0, 98, 2]),                    // NRPN LSB=2（Reverb Type）
            (0, vec![0xB0, 6, 3]),                     // Data Entry
            (10, vec![0xD0, 40]),                      // Channel Pressure（AT）
            (10, vec![0xA0, 60, 30]),                  // Poly Pressure（AT）
            (10, vec![0xE0, 0x00, 0x50]),              // Pitch Bend
            (240, vec![0x80, 60, 0]),                  // Note Off
        ];
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, 8000.0, 0.1, Some(1.0)).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "混在CC/NRPN出力が無音");
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// d1l=255（フルサステイン）・rr=255（最速リリース≈8.71ms）の1音色バンク。
    /// CC64ホールドフラグ方式の検証用：ペダル無しなら release ですぐ無音化する。
    fn sustain_pedal_bank() -> PatchBank {
        let mut patch = Ym38x6Patch::default();
        patch.channel.algorithm = 7; // 全OP独立キャリア
        patch.operators[0].tl = 220;
        patch.operators[0].mul = 1;
        patch.operators[0].ar = 255;
        patch.operators[0].d1l = 255;
        patch.operators[0].rr = 255;
        PatchBank::from_patches(&[patch]).unwrap()
    }

    /// CC64(サステインペダル)ON中はNote Offが保留され、鳴り続けることを確認する。
    #[test]
    fn cc64_holds_release_while_pedal_down() {
        let bank = sustain_pedal_bank();
        let sr = 8000.0;
        let tail_window = 200; // 25ms分。rr=255の減衰(≈8.71ms)が完了しきる時間。

        // ペダル無し: note off で即 release に入り、テール終端では無音のはず。
        let smf_no_pedal = build_smf(&[(0u32, vec![0x90, 69, 100]), (480u32, vec![0x80, 69, 0])]);
        let buf_no_pedal = render_smf(&smf_no_pedal, &bank, sr, 0.2, Some(1.0)).unwrap();
        let no_pedal_tail_rms = rms(&buf_no_pedal[buf_no_pedal.len() - tail_window..]);
        assert!(no_pedal_tail_rms < 1e-3, "ペダル無しなのに末尾で鳴り続けている: rms={no_pedal_tail_rms}");

        // ペダル有り: 同じタイミングでnote offが来てもreleaseへ入らず鳴り続けるはず。
        let smf_pedal = build_smf(&[
            (0u32, vec![0xB0, 64, 127]), // CC64 ON
            (0u32, vec![0x90, 69, 100]),
            (480u32, vec![0x80, 69, 0]),
        ]);
        let buf_pedal = render_smf(&smf_pedal, &bank, sr, 0.2, Some(1.0)).unwrap();
        let pedal_tail_rms = rms(&buf_pedal[buf_pedal.len() - tail_window..]);
        assert!(pedal_tail_rms > 0.05, "ペダル保持中なのに音が消えている: rms={pedal_tail_rms}");
    }

    /// 弾き直し（同ノートの再Note-On）でペダル保留ビットがクリアされることを確認する。
    /// クリアしないと、鍵盤を押したままペダルアップしただけで誤って音が切れる
    /// （spec-sound.md「サステインペダル（CC64）の実装方針」参照）。
    #[test]
    fn cc64_retrigger_clears_pending_release_before_pedal_up() {
        let bank = sustain_pedal_bank();
        let sr = 8000.0;
        let tail_window = 200;

        let smf = build_smf(&[
            (0u32, vec![0xB0, 64, 127]), // CC64 ON
            (0u32, vec![0x90, 69, 100]), // Note On
            (240u32, vec![0x80, 69, 0]), // Note Off（保留になる）
            (10u32, vec![0x90, 69, 100]), // 弾き直し（鍵盤はまだ押されたまま、Note Offは送らない）
            (230u32, vec![0xB0, 64, 0]), // CC64 OFF（鍵盤はまだ押されているので切れてはいけない）
        ]);
        let buf = render_smf(&smf, &bank, sr, 0.2, Some(1.0)).unwrap();
        let tail_rms = rms(&buf[buf.len() - tail_window..]);
        assert!(
            tail_rms > 0.05,
            "弾き直した鍵盤がまだ押されているのにペダルアップで音が消えた（stale pending_releaseの再発）: rms={tail_rms}"
        );
    }

    /// CC66(Sostenuto)はON時点で押下中のノートのみ保持し、ON後に新規キーオンしたノートは
    /// 対象外であることを確認する（spec-sound.md「Sostenuto（CC66）」）。
    #[test]
    fn cc66_sostenuto_holds_only_notes_down_at_press() {
        let bank = sustain_pedal_bank();
        let sr = 8000.0;
        let tail_window = 200;

        // CC66 ON以前から押下中のノートはlatchされ、離鍵しても鳴り続ける。
        let smf_latched = build_smf(&[
            (0u32, vec![0x90, 69, 100]),  // Note On A（先に押す）
            (10u32, vec![0xB0, 66, 127]), // CC66 ON（A latch）
            (10u32, vec![0x80, 69, 0]),   // Note Off A（sostenutoで保留されるはず）
        ]);
        let buf_latched = render_smf(&smf_latched, &bank, sr, 0.2, None).unwrap();
        let tail_latched = rms(&buf_latched[buf_latched.len() - tail_window..]);
        assert!(tail_latched > 0.05, "CC66 ON時点で押下中のノートが保持されていない: rms={tail_latched}");

        // CC66 ON後に新規キーオンしたノートはlatch対象外で、離鍵すれば即releaseするはず。
        let smf_unlatched = build_smf(&[
            (0u32, vec![0xB0, 66, 127]),  // CC66 ON（何も押下中でない状態でlatch）
            (10u32, vec![0x90, 72, 100]), // Note On C（CC66 ON後に押す）
            (240u32, vec![0x80, 72, 0]),  // Note Off C（latch対象外なので即release）
        ]);
        let buf_unlatched = render_smf(&smf_unlatched, &bank, sr, 0.2, None).unwrap();
        let tail_unlatched = rms(&buf_unlatched[buf_unlatched.len() - tail_window..]);
        assert!(
            tail_unlatched < 1e-3,
            "CC66 ON後に弾いたノートがsostenutoに保持されてしまっている: rms={tail_unlatched}"
        );
    }

    /// CC66(Sostenuto)とCC64(Sustain)を併用したとき、片方をOFFにしても他方が保持中なら
    /// 音が消えず、両方OFFで初めて解放されることを確認する。
    #[test]
    fn cc66_cc64_combined_release_requires_both_off() {
        let bank = sustain_pedal_bank();
        let sr = 8000.0;
        let tail_window = 200;

        // CC66 OFFしてもCC64がまだ踏まれていれば保持され続けるはず。
        let smf_cc64_still_down = build_smf(&[
            (0u32, vec![0x90, 69, 100]),  // Note On A
            (10u32, vec![0xB0, 66, 127]), // CC66 ON（A latch）
            (10u32, vec![0x80, 69, 0]),   // Note Off A（sostenuto保留）
            (10u32, vec![0xB0, 64, 127]), // CC64 ON
            (10u32, vec![0xB0, 66, 0]),   // CC66 OFF（CC64がまだ踏まれている）
        ]);
        let buf1 = render_smf(&smf_cc64_still_down, &bank, sr, 0.2, None).unwrap();
        let tail1 = rms(&buf1[buf1.len() - tail_window..]);
        assert!(tail1 > 0.05, "CC64保持中なのにCC66 OFFで音が消えた: rms={tail1}");

        // さらにCC64もOFFにすれば解放されるはず。
        let smf_both_off = build_smf(&[
            (0u32, vec![0x90, 69, 100]),
            (10u32, vec![0xB0, 66, 127]),
            (10u32, vec![0x80, 69, 0]),
            (10u32, vec![0xB0, 64, 127]),
            (10u32, vec![0xB0, 66, 0]),
            (10u32, vec![0xB0, 64, 0]), // CC64 OFFでようやく解放
        ]);
        let buf2 = render_smf(&smf_both_off, &bank, sr, 0.2, None).unwrap();
        let tail2 = rms(&buf2[buf2.len() - tail_window..]);
        assert!(tail2 < 1e-3, "両ペダルOFF後も音が消えない: rms={tail2}");
    }

    /// CC67(Soft Pedal)ON中に発音したノートは実効TL（キャリア）が減算され出力が小さくなる。
    #[test]
    fn cc67_soft_reduces_output() {
        let mut patch = Ym38x6Patch::default();
        patch.channel.algorithm = 7; // 全OP独立キャリア
        patch.operators[0].tl = 180; // 減算後も無音にならない中間値
        patch.operators[0].mul = 1;
        patch.operators[0].ar = 255;
        patch.operators[0].d1l = 255;
        patch.operators[0].rr = 255;
        let bank = PatchBank::from_patches(&[patch]).unwrap();
        let sr = 8000.0;
        let tail_secs = 0.05;
        let window = 200;

        let smf_hard = build_smf(&[(0u32, vec![0x90, 69, 100])]);
        let buf_hard = render_smf(&smf_hard, &bank, sr, tail_secs, None).unwrap();
        let rms_hard = rms(&buf_hard[buf_hard.len() - window..]);

        let smf_soft = build_smf(&[
            (0u32, vec![0xB0, 67, 127]), // CC67 ON（最大深さ）
            (1u32, vec![0x90, 69, 100]),
        ]);
        let buf_soft = render_smf(&smf_soft, &bank, sr, tail_secs, None).unwrap();
        let rms_soft = rms(&buf_soft[buf_soft.len() - window..]);

        assert!(
            rms_soft < rms_hard * 0.9,
            "Soft Pedal(深さ127)でも音量が下がっていない: hard={rms_hard} soft={rms_soft}"
        );
    }

    /// CC120(All Sound Off)はリリースを経ず即座に消音し、CC123(All Notes Off)は通常の
    /// Note-Off相当でReleaseしながら減衰することを確認する（区別、spec-sound.md参照）。
    #[test]
    fn cc120_silences_immediately_vs_cc123_releases() {
        let bank = sustain_pedal_bank();
        let sr = 8000.0;
        let tail_secs = 0.03; // 30ms（rr=255の減衰≈8.71msを含む）
        let tail_len = (tail_secs * sr) as usize;
        let head_window = 16; // 直後2msの窓

        let smf_120 = build_smf(&[(0u32, vec![0x90, 69, 100]), (240u32, vec![0xB0, 120, 127])]);
        let buf_120 = render_smf(&smf_120, &bank, sr, tail_secs, None).unwrap();
        // CC直前（定常状態）の振幅を基準に、絶対値でなく相対比較する
        // （EGカーブの形状に依存しないようにするため）。
        let steady_120 = rms(&buf_120[buf_120.len() - tail_len - head_window..buf_120.len() - tail_len]);
        let head_120 = rms(&buf_120[buf_120.len() - tail_len..buf_120.len() - tail_len + head_window]);
        assert!(
            head_120 < steady_120 * 0.01,
            "CC120(All Sound Off)直後に即座消音していない: steady={steady_120} head={head_120}"
        );

        let smf_123 = build_smf(&[(0u32, vec![0x90, 69, 100]), (240u32, vec![0xB0, 123, 127])]);
        let buf_123 = render_smf(&smf_123, &bank, sr, tail_secs, None).unwrap();
        let steady_123 = rms(&buf_123[buf_123.len() - tail_len - head_window..buf_123.len() - tail_len]);
        let head_123 = rms(&buf_123[buf_123.len() - tail_len..buf_123.len() - tail_len + head_window]);
        assert!(
            head_123 > steady_123 * 0.1,
            "CC123(All Notes Off)直後はReleaseで鳴っているはず: steady={steady_123} head={head_123}"
        );

        let end_123 = rms(&buf_123[buf_123.len() - head_window..]);
        assert!(end_123 < 1e-3, "CC123のReleaseが完了していない: rms={end_123}");
    }

    /// CC121(Reset All Controllers)は保留中のペダル保持ノートを解放し、pedal_down等の
    /// ③層状態をリセットすることを確認する（spec-sound.md「補強規則」）。
    #[test]
    fn cc121_resets_pedals_and_releases_pending_notes() {
        let bank = sustain_pedal_bank();
        let sr = 8000.0;
        let tail_window = 200;

        // CC64保持中に保留されたノートがCC121で解放される。
        let smf_pending_release = build_smf(&[
            (0u32, vec![0xB0, 64, 127]),  // CC64 ON
            (10u32, vec![0x90, 69, 100]), // Note On
            (240u32, vec![0x80, 69, 0]),  // Note Off（保留）
            (10u32, vec![0xB0, 121, 0]),  // Reset All Controllers
        ]);
        let buf1 = render_smf(&smf_pending_release, &bank, sr, 0.2, None).unwrap();
        let tail1 = rms(&buf1[buf1.len() - tail_window..]);
        assert!(tail1 < 1e-3, "CC121で保留ノートが解放されていない: rms={tail1}");

        // CC121後はpedal_downがリセットされ、新規ノートはCC64を送り直さない限り保持されない。
        let smf_pedal_state = build_smf(&[
            (0u32, vec![0xB0, 64, 127]),  // CC64 ON
            (10u32, vec![0xB0, 121, 0]),  // Reset All Controllers（pedal_downをリセット）
            (10u32, vec![0x90, 69, 100]), // Note On
            (240u32, vec![0x80, 69, 0]),  // Note Off
        ]);
        let buf2 = render_smf(&smf_pedal_state, &bank, sr, 0.2, None).unwrap();
        let tail2 = rms(&buf2[buf2.len() - tail_window..]);
        assert!(
            tail2 < 1e-3,
            "CC121後もpedal_downが残っている（新規ノートが保持されてしまう）: rms={tail2}"
        );
    }
}
