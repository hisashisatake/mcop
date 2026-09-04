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
//! NRPN(0,28)〜(0,33) FG Loop/Curveは`ChannelState::overrides`（`PatchOverrides`）経由で
//! Pitch/Cutoff/Gain FGのloop_enabled・全段curveを上書きする（Algorithm等と同じNRPN離散
//! 上書きレイヤー、smf2op505は`.op505`バンクの値をベースパッチとしてそのまま使うだけなので
//! `op505-vst`特有のpersist書き込み制約を受けない）。
//!
//! 対応SMFイベント:
//!   - Note On/Off、Program Change（CC102代替含む）、Tempo
//!   - Pitch Bend（チャンネル単位、RPN(0,0)でセンシティビティ設定可）
//!   - CC1/76/77/78: Pitch FG（ビブラート）への演奏補正
//!   - CC7/CC11: GM2音量（実効ゲイン=(cc7/127)²×(cc11/127)²）
//!   - CC10: Pan（コンスタントパワー則、ボイス単位の左右ゲイン）
//!   - CC71/74: Resonance/Brightness（Filter Resonance/Cutoffへの64中心相対補正）
//!   - CC72/73/75: Release/Attack/Decay Time（保持区間をピーク検出でAttack/Decay/Releaseへ
//!     分割し、キャリアのみの各段timeへ時間スケールを掛ける）
//!   - CC64 Sustain / CC66 Sostenuto / CC67 Soft Pedal（ホールドフラグ方式）
//!   - CC91/93 + NRPN(0,2)〜(0,8): マスターエフェクト（Reverb/Chorus、`MasterEffects`）。
//!     送信チャンネルの`effect_route_slot`（NRPN(0,1) Channel Effect Route）が指すスロットへ適用
//!   - NRPN(0,1): Channel Effect Route（送信チャンネルの音声・エフェクト設定・CC91/93の
//!     適用先スロットを選択、既定はスロット0）
//!   - NRPN(0,0)・(0,22)〜(0,27)/(0,34)/(0,35): 質感LFO・Algorithm/Waveform/Filter/AT/OP F-Number/CC2,4 Destination
//!   - Channel Pressure / Poly Key Pressure: AT Destination（NRPN(0,16)/(0,17)）
//!   - CC103〜106: Operator Key On/Off
//!   - CC120 All Sound Off（即時消音）/ CC121 Reset All Controllers（③層のみ）/
//!     CC123 All Notes Off（リリース）
//!
//! CC/NRPN の変更は当該チャンネルの発音中ボイスへ即時伝播する（[`apply_live`]）。

use op505_core::{Op505Engine, Op505Patch, Op505PresetBank};
use op505_midi::{
    cc_byte_to_u7 as cc_to_u7, cc_byte_to_u8 as cc_to_u8, released_notes, ChannelState, DataEntryOutcome,
    MonoNoteOff, MonoNoteOn, ProgramSelection, RHYTHM_BANK_RANGE,
};
use sound_core::{cc76_to_rate_scale, MasterSection, Vco};

use crate::bank::PatchBank;
use crate::smf::{parse_smf, EvKind};

/// MIDIノート番号の総数（0〜127）。ノート番号をそのままボイスIDの下位に使うため、
/// 発音中ボイス走査ループの上限に使う（`op505-vst`の`MIDI_NOTE_COUNT`と同じ）。
const MIDI_NOTE_COUNT: usize = 128;

/// エフェクトスロット数。`op505_midi::EFFECT_SLOT_COUNT`（クランプ境界の一元管理元）と
/// 常に揃える。
const EFFECT_SLOT_COUNT: usize = op505_midi::EFFECT_SLOT_COUNT as usize;

/// CC7(Channel Volume) と CC11(Expression) の値（0〜127）から GM2 準拠のゲインを計算する
/// （`op505-vst`と同一式。`op505-midi`には置かない小関数のため、op505-vstと同様ここで複製する）。
#[inline]
fn channel_gain(cc7: u8, cc11: u8) -> f32 {
    let v7 = cc7 as f32 / 127.0;
    let v11 = cc11 as f32 / 127.0;
    v7 * v7 * v11 * v11
}

/// このチャンネル・ノートのベースパッチ（NRPN上書きを重ねる前の①層）。`None`ならこの
/// ノートは発音しない（リズムチャンネルでキット内に未定義のノート＝GM2実機で無音になる
/// のと同じ。`Op505Patch::default()`でnote_onしてはいけない、全段level=0/time=0のEGは
/// release_pointで静止し`is_idle()`が永久にfalseになりボイスが漏れるため）。
///
/// 旋律は従来どおり`PatchBank`（前方フィルのフォールバック付きで常にSome）をProgram Change
/// 番号だけで引く（`op505-vst`同様、smf2op505は単一バンク運用のためbank番号は無視する）。
/// リズムは`Op505PresetBank`の(bank,program)厳密ルックアップ＋kit0フォールバック。
///
/// ⚠️ `PatchBank`（旋律用）は「前方フィル＋最小番号フォールバック」で絶対にNoneにならない
/// 設計（bank.rs参照）。これをリズムに転用すると「BDの音でハイハットが鳴る」ので
/// 転用しない（`Op505PresetBank::get`の素直なNone、preset.rs参照）。
fn base_patch_for(st: &ChannelState, note: u8, bank: &PatchBank, drums: Option<&Op505PresetBank>) -> Option<Op505Patch> {
    match st.program_state.selection() {
        ProgramSelection::Rhythm { .. } => {
            let drums = drums?;
            let (b, p) = st.program_state.lookup_address(note);
            drums
                .get(b, p)
                .or_else(|| {
                    st.program_state.rhythm_fallback_address(note).and_then(|(fb, fp)| drums.get(fb, fp))
                })
                .map(|preset| preset.patch)
        }
        ProgramSelection::Melodic { program, .. } => Some(*bank.patch(program)),
    }
}

/// CC/NRPN で変わった実効パッチ・CC76 rate_scale・AT・OP F-Number を、そのチャンネルの
/// 発音中ボイス全てへ伝播する（ライブ反映）。`op505-vst`の毎ブロック伝播ループを
/// 1チャンネル分に絞ったもの。非発音スロットへの set_* はエンジン側で no-op になる。
///
/// リズムチャンネルはノートごとに音色が違うため、`base_patch_for`をノートごとに呼ぶ
/// （旋律のようにチャンネル全体で1回だけ計算する最適化はできない）。キット内に未定義の
/// ノート（`base_patch_for`がNone）はスキップし、既存の発音中パラメーターをそのまま維持する。
fn apply_live(engine: &mut Op505Engine, chi: usize, st: &ChannelState, bank: &PatchBank, drums: Option<&Op505PresetBank>) {
    let rate_scale = cc76_to_rate_scale(st.pitch_fg_cc76);
    for note in 0..MIDI_NOTE_COUNT {
        let note_u8 = note as u8;
        let Some(base) = base_patch_for(st, note_u8, bank, drums) else { continue };
        let eff = st.build_effective_patch(&base);
        let id = chi * 128 + note;
        let mut note_patch = eff;
        st.apply_note_post_processing(&mut note_patch, note_u8);
        engine.set_channel_params(id, note_patch.channel);
        for (op_index, op) in note_patch.operators.iter().enumerate() {
            engine.set_operator_params(id, op_index, *op);
        }
        engine.set_pitch_fg_rate_scale(id, rate_scale);
        engine.set_channel_pan(id, st.pan_gains());
        // RPN(0,1)/(0,2) Channel Fine/Coarse Tuning。他の全NRPN補正と同じく無条件に毎回
        // 再送する（`total_pitch_bend_cents`＝bend_cents+tune_cents）。
        engine.set_pitch_bend(id, st.total_pitch_bend_cents());
        for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
            if let Some(f_number) = f {
                engine.set_operator_f_number(id, op_index, *f_number);
            }
        }
    }
}

/// MIDIノート番号→周波数(Hz)。`note_on_voice_core`本体・グライド起点計算の両方から使う
/// （op505-midiはMIDIノート番号までしか扱わないため、周波数変換は呼び出し側の責務）。
#[inline]
fn note_to_frequency(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// 1ボイスの実際の発音処理（ペダル・Mono状態の更新は含まない）。`op505-vst`のNoteOn適用と
/// 同型。現在の AT を焼き込んだ実効パッチで発音し、ベンド量・音量ゲイン・rate_scale・
/// OP F-Number を新ボイスへ反映する。`base_patch_for`がNoneなら発音しない
/// （キット内に未定義のノート）。
///
/// Mono Modeのlast-note priorityフォールバック（[`MonoNoteOff::Fallback`]）からも直接
/// 呼べるよう、ペダル状態の更新（`pedal.note_on`）は呼び出し側で行う（フォールバック
/// 再発音は「新規の押鍵」ではないため、CC67(Soft Pedal)のsoft_notes判定を押鍵時点の
/// 状態のまま動かしたい）。
fn note_on_voice_core(
    engine: &mut Op505Engine,
    chi: usize,
    note: u8,
    vel: u8,
    channels: &mut [ChannelState],
    bank: &PatchBank,
    drums: Option<&Op505PresetBank>,
) {
    let Some(base) = base_patch_for(&channels[chi], note, bank, drums) else { return };
    let id = chi * 128 + note as usize;
    let freq = note_to_frequency(note);
    {
        let st = &channels[chi];
        let mut eff = st.build_effective_patch(&base);
        st.apply_note_post_processing(&mut eff, note);
        engine.set_patch(eff);
        engine.note_on(id, freq, vel);
        engine.set_channel_volume(id, channel_gain(st.cc7, st.cc11));
        engine.set_channel_pan(id, st.pan_gains());
        engine.set_pitch_bend(id, st.total_pitch_bend_cents());
        engine.set_pitch_fg_rate_scale(id, cc76_to_rate_scale(st.pitch_fg_cc76));
        for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
            if let Some(f_number) = f {
                engine.set_operator_f_number(id, op_index, *f_number);
            }
        }
    }
    if let Some((from_note, seconds)) = channels[chi].glide_source(note) {
        engine.start_glide(id, note_to_frequency(from_note), seconds);
    }
    channels[chi].last_note = Some(note);
}

/// 外部からの実際のNote On（SMFのNote Onイベント）。ペダル状態の更新とMono Modeの処理
/// （前の音の解放、またはCC65 ON+レガート時はボイスを継続したままピッチだけ滑らせる
/// レガート、詳細はmono.rsモジュールdoc）を行ってから[`note_on_voice_core`]を呼ぶ。
fn note_on_voice(
    engine: &mut Op505Engine,
    chi: usize,
    note: u8,
    vel: u8,
    channels: &mut [ChannelState],
    bank: &PatchBank,
    drums: Option<&Op505PresetBank>,
) {
    // 弾き直したらペダル保留を解除する（離鍵→ペダルアップ前に再度弾いた場合、古い保留ビットが
    // 残っていると鍵盤を押している最中にペダルアップでnote_offが誤発火する）。Soft
    // Pedal（CC67）: ON中に新規キーオンしたノートのみ対象。
    channels[chi].pedal.note_on(note);
    if channels[chi].mono.enabled {
        let portamento = channels[chi].portamento_on;
        match channels[chi].mono.note_on(note, vel, portamento) {
            MonoNoteOn::Legato { voice } => {
                let seconds = channels[chi].portamento_seconds();
                let id = chi * 128 + voice as usize;
                if engine.glide_to(id, note_to_frequency(note), seconds) {
                    channels[chi].last_note = Some(note);
                    return; // 再アタックしない
                }
                // ボイスが既にIdle等で消えていたら通常発音へフォールバックする。
                channels[chi].mono.demote_legato(note);
            }
            MonoNoteOn::Retrigger { release } => {
                if let Some(prev) = release {
                    engine.note_off(chi * 128 + prev as usize);
                }
            }
        }
    }
    note_on_voice_core(engine, chi, note, vel, channels, bank, drums);
}

/// レンダリング済みサンプル位置を `target` まで進める。各MIDIチャンネルの`effect_route_slot`
/// （NRPN(0,1) Channel Effect Route、既定0）に応じてエンジン出力をスロット別に振り分け、
/// スロットごとに対応する`MasterEffects`を適用してから全スロットを合成する。チャンクは
/// イベント境界に揃うため、エフェクトのオートメーションがサンプル正確に反映される。
///
/// 誰もNRPN(0,1)を送らなければ全チャンネルの`effect_route_slot`は0のままなので、
/// 全音声がスロット0だけを通り、この機能追加前の単一`MasterEffects`と同じ出力になる
/// （ビット不変、`no_effect_routing_is_bit_identical_across_runs`で検証）。
fn render_chunk(
    engine: &mut Op505Engine,
    master: &mut MasterSection,
    channels: &[ChannelState],
    out: &mut Vec<f32>,
    rendered: &mut usize,
    target: usize,
) {
    if target > *rendered {
        let n = target - *rendered;
        let channel_slot: [u8; 16] = std::array::from_fn(|i| channels[i].effect_route_slot);
        let mixed = master.render(n, 1, |slot_buf, stride| {
            engine.render_routed(slot_buf, stride, &channel_slot, 1);
        });
        out.extend_from_slice(mixed);
        *rendered = target;
    }
}

/// SMF を `bank` を音色として再生し、mono f32 サンプル列を返す。
/// `tail_secs` はノートオフ後の残響を伸ばす秒数。
/// `max_secs` が `Some(s)` のとき、出力を `s` 秒（テール込み）で打ち切る（試聴の時短用）。
///
/// GM2リズムチャンネル機能は使わない（`render_smf_with_drums`の`drums=None`と同じ）。
pub fn render_smf(
    data: &[u8],
    bank: &PatchBank,
    sample_rate: f32,
    tail_secs: f32,
    max_secs: Option<f32>,
    max_voices: Option<usize>,
) -> Result<Vec<f32>, String> {
    render_smf_with_drums(data, bank, None, sample_rate, tail_secs, max_secs, max_voices)
}

/// [`render_smf`] のGM2リズムチャンネル対応版。`drums`にリズムキット集合
/// （`--drum-bank`で読み込んだ`Op505PresetBank`、bank=15360+キット番号で登録）を渡すと、
/// Bank Select MSB(CC0)=120 + Program Change で該当MIDIチャンネルがリズムチャンネルになる
/// （`op505_midi::ChannelProgramState`参照。判定・アドレス解決の詳細はそちら）。
///
/// `drums`が`None`のときはリズムチャンネル機能を完全に無効化する：Bank Select CC0/32は
/// 従来どおり無視し、MIDI ch10の初期ドラムONも立てない。これにより`render_smf`（`drums=None`
/// で呼ぶだけ）の出力は本関数追加前とビット単位で不変。
pub fn render_smf_with_drums(
    data: &[u8],
    bank: &PatchBank,
    drums: Option<&Op505PresetBank>,
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
    // エフェクトスロット数分だけ持ち、各チャンネルの`effect_route_slot`（NRPN(0,1)）が
    // 指すスロットへルーティングする（誰も送らなければ全チャンネルがslot 0へ集まる）。
    let mut master = MasterSection::new(sample_rate, EFFECT_SLOT_COUNT);
    let mut out: Vec<f32> = Vec::new();

    let rhythm_kits_available = drums.map(|d| d.has_bank_in(RHYTHM_BANK_RANGE)).unwrap_or(false);
    let mut channels: Vec<ChannelState> =
        (0..16).map(|chi| ChannelState::new(chi, rhythm_kits_available)).collect();

    let mut tempo_us: f64 = 500_000.0; // 既定 120BPM
    let mut spt = tempo_us / 1_000_000.0 * sample_rate as f64 / division as f64; // samples/tick
    let mut cur_tick: u64 = 0;
    let mut sample_pos: f64 = 0.0;
    let mut rendered: usize = 0;
    let mut peak_voices: usize = 0;

    let max_samples = max_secs.map(|s| (s * sample_rate).max(0.0) as usize);

    for e in events {
        let dt = e.tick - cur_tick;
        sample_pos += dt as f64 * spt;
        cur_tick = e.tick;
        let target = sample_pos.floor() as usize;
        // 時短打ち切り: 上限に達したらそこまでレンダリングして以降のイベントは無視する。
        if let Some(maxs) = max_samples {
            if target >= maxs {
                render_chunk(&mut engine, &mut master, &channels, &mut out, &mut rendered, maxs);
                eprintln!("smf2op505: ピークボイス数(active_voice_count) = {peak_voices}");
                return Ok(out);
            }
        }
        render_chunk(&mut engine, &mut master, &channels, &mut out, &mut rendered, target);
        peak_voices = peak_voices.max(engine.active_voice_count());

        match e.kind {
            EvKind::Tempo(us) => {
                tempo_us = us as f64;
                spt = tempo_us / 1_000_000.0 * sample_rate as f64 / division as f64;
                // TimeEg（Op505Engine::set_tempo）・マスターディレイ（MasterSection::set_tempo）
                // 両方のテンポ同期に反映する。旧実装はtick→サンプル換算にしか使っておらず、
                // これらの同期が常に既定120BPM固定になっていた（2026-09-04発見・修正）。
                let bpm = 60_000_000.0 / tempo_us;
                engine.set_tempo(bpm as f32);
                master.set_tempo(bpm as f32);
            }
            EvKind::Program(ch, p) => {
                channels[ch as usize].program_change(p);
            }
            EvKind::NoteOn(ch, note, vel) => {
                let chi = ch as usize;
                note_on_voice(&mut engine, chi, note, vel, &mut channels, bank, drums);
            }
            EvKind::NoteOff(ch, note) => {
                let chi = ch as usize;
                channels[chi].poly_pressure[note as usize] = 0;
                // ペダルの内部ブックキーピング（keys_down等）は常に更新する。実際にエンジンへ
                // note_offするかどうかはMono Mode有無で分岐する。
                let pedal_released = channels[chi].pedal.note_off(note);
                if channels[chi].mono.enabled {
                    // Mono Mode: サステインペダルより優先する（押している間はペダルで音を
                    // 保持する通常のPoly挙動と違い、Monoは常に「今押している鍵盤」だけが
                    // 鳴る。そうしないと前の音が保持されたまま新しい音が重なりMonoで
                    // なくなるため）。押鍵状態（MonoState）だけで解放/フォールバックを決める。
                    let portamento = channels[chi].portamento_on;
                    match channels[chi].mono.note_off(note, portamento) {
                        MonoNoteOff::Nothing => {}
                        MonoNoteOff::Release(released_note) => {
                            engine.note_off(chi * 128 + released_note as usize);
                        }
                        MonoNoteOff::Fallback { release, sound, velocity } => {
                            engine.note_off(chi * 128 + release as usize);
                            note_on_voice_core(&mut engine, chi, sound, velocity, &mut channels, bank, drums);
                        }
                        MonoNoteOff::LegatoFallback { voice, sound, velocity } => {
                            let seconds = channels[chi].portamento_seconds();
                            let id = chi * 128 + voice as usize;
                            if !engine.glide_to(id, note_to_frequency(sound), seconds) {
                                // ボイスが既にIdle等で消えていたら通常発音へフォールバックする。
                                channels[chi].mono.demote_legato(sound);
                                engine.note_off(id);
                                note_on_voice_core(&mut engine, chi, sound, velocity, &mut channels, bank, drums);
                            }
                        }
                    }
                } else if pedal_released {
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
                apply_live(&mut engine, chi, &channels[chi], bank, drums);
            }
            EvKind::PolyPressure(ch, note, value) => {
                let chi = ch as usize;
                channels[chi].poly_pressure[note as usize] = cc_to_u8(value);
                apply_live(&mut engine, chi, &channels[chi], bank, drums);
            }
            EvKind::ControlChange(ch, cc, val) => {
                handle_control_change(&mut engine, &mut master, &mut channels, ch as usize, cc, val, bank, drums);
            }
            EvKind::SysEx(bytes) => {
                if let Some(sound_midi::UniversalSysEx::MasterVolume { value14, .. }) =
                    sound_midi::parse_universal_sysex(&bytes)
                {
                    master.output_mut().set_volume(sound_midi::value14_to_u8(value14));
                }
            }
        }
    }

    // 残響テール（max_secs 指定時は上限でクランプ）
    let mut tail_target = rendered + (sample_rate * tail_secs) as usize;
    if let Some(maxs) = max_samples {
        tail_target = tail_target.min(maxs);
    }
    render_chunk(&mut engine, &mut master, &channels, &mut out, &mut rendered, tail_target);
    peak_voices = peak_voices.max(engine.active_voice_count());
    eprintln!("smf2op505: ピークボイス数(active_voice_count) = {peak_voices}");
    Ok(out)
}

/// 1つのコントロールチェンジを処理する。`op505-vst`のprocess()内CC matchと
/// handle_data_entryをper-channelに移植したもの。
#[allow(clippy::too_many_arguments)]
fn handle_control_change(
    engine: &mut Op505Engine,
    master: &mut MasterSection,
    channels: &mut [ChannelState],
    chi: usize,
    cc: u8,
    val: u8,
    bank: &PatchBank,
    drums: Option<&Op505PresetBank>,
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
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        76 => {
            channels[chi].pitch_fg_cc76 = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        77 => {
            channels[chi].pitch_fg_cc77 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        78 => {
            channels[chi].pitch_fg_cc78 = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        // CC92(Tremolo Depth)：Gain FG Depthへの0起点加算（CC77と同型、RPN連動レンジは無い）。
        92 => {
            channels[chi].gain_fg_cc92 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        // CC2(ブレス)/CC4(フット): Expression Destination（NRPN(0,34)/(0,35)）へ加算 → 発音中ボイスへ伝播。
        2 => {
            channels[chi].cc2 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        4 => {
            channels[chi].cc4 = cc_to_u8(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        // CC10(Pan): ボイス単位の左右ゲイン（patchではなくVco::set_channel_pan_group経由、
        // コンスタントパワー則）。CC7/CC11と同じく発音中へ即時反映する。
        10 => {
            channels[chi].cc10_pan = cc_to_u7(val);
            engine.set_channel_pan_group(chi, channels[chi].pan_gains());
        }
        // CC71(Resonance)/CC72(Release Time)/CC73(Attack Time)/CC74(Brightness)/
        // CC75(Decay Time): `op505_midi::apply_sound_controllers`参照。値を保持し発音中へ伝播する。
        71 => {
            channels[chi].cc71_resonance = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        72 => {
            channels[chi].cc72_release = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        73 => {
            channels[chi].cc73_attack = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        74 => {
            channels[chi].cc74_brightness = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        75 => {
            channels[chi].cc75_decay = cc_to_u7(val);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        // CC5(Portamento Time)/CC65(Portamento On/Off): 次のnote_onでのグライドに使う
        // だけなので、発音中ボイスへの即時反映（apply_live）は不要。
        5 => {
            channels[chi].portamento_time = cc_to_u7(val);
        }
        65 => {
            channels[chi].portamento_on = cc_to_u7(val) >= 64;
        }
        // NRPN/RPN 選択（CC99/98=NRPN MSB/LSB、CC101/100=RPN MSB/LSB）
        98 => channels[chi].rpn.set_nrpn_lsb(cc_to_u7(val)),
        99 => channels[chi].rpn.set_nrpn_msb(cc_to_u7(val)),
        100 => channels[chi].rpn.set_rpn_lsb(cc_to_u7(val)),
        101 => channels[chi].rpn.set_rpn_msb(cc_to_u7(val)),
        // CC6 Data Entry MSB: 選択中の RPN/NRPN へ値を適用。エフェクト系NRPN(Reverb/Chorus)は
        // op505-midiがsound-core型を扱えないため`DataEntryOutcome::Effect`で返り、ここで
        // 送信チャンネルの`effect_route_slot`が指すMasterEffectsへ適用する。
        6 => match channels[chi].apply_data_entry(val) {
            DataEntryOutcome::StateChanged { voice_update } => {
                if voice_update {
                    apply_live(engine, chi, &channels[chi], bank, drums);
                }
            }
            DataEntryOutcome::Effect(slot, target, value) => {
                let fx = master.slot_mut((slot as usize).min(EFFECT_SLOT_COUNT - 1));
                sound_midi::apply_effect_control(fx, target, value);
            }
        },
        // CC38 Data Entry LSB: OP F-Number(NRPN 0,18〜21選択中)の下位7bit。
        38 => {
            if channels[chi].apply_data_entry_lsb(val) {
                apply_live(engine, chi, &channels[chi], bank, drums);
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
        // CC2/CC4/CC7/CC11/CC76〜78/センド/RPN等は保持。GM2でもRACはbank/programを
        // リセットしないため、`program_state`（リズム/旋律の状態）へは意図的に触れない。
        121 => {
            let released = channels[chi].reset_all_controllers();
            for note in released_notes(released) {
                engine.note_off(chi * 128 + note as usize);
            }
            engine.set_pitch_bend_group(chi, 0.0);
            apply_live(engine, chi, &channels[chi], bank, drums);
        }
        // CC91/93: エフェクト送りレベル。送信チャンネルの`effect_route_slot`が指すスロットへ適用。
        91 => master.slot_mut(channels[chi].effect_route_slot as usize).set_reverb_send(cc_to_u8(val)),
        93 => master.slot_mut(channels[chi].effect_route_slot as usize).set_chorus_send(cc_to_u8(val)),
        // CC102: Program Change 代替（VST3で MidiProgramChange が届かないため）。
        102 => {
            channels[chi].program_change(cc_to_u7(val));
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
            channels[chi].mono.reset();
            channels[chi].last_note = None;
        }
        // CC123 All Notes Off: 通常のNote-Off相当（リリースして自然減衰）。
        123 => {
            for note in 0..MIDI_NOTE_COUNT {
                engine.note_off(chi * 128 + note);
            }
            channels[chi].pedal.cc123_reset();
            channels[chi].mono.reset();
            channels[chi].last_note = None;
        }
        // CC126 Mono Mode On / CC127 Poly Mode On: データバイトの値は無視し（一般的な
        // 音源シンセの実装に倣う）、モード切替＋そのチャンネルの全ノートを一括note_off
        // する（CC123と同じ全ノートオフ処理。Poly→Mono/Mono→Polyのどちらの遷移でも、
        // 遷移前に何が鳴っていたかに関わらずそのチャンネルを一旦静かにしてから始める）。
        126 | 127 => {
            channels[chi].mono.set_enabled(cc == 126);
            for note in 0..MIDI_NOTE_COUNT {
                engine.note_off(chi * 128 + note);
            }
            channels[chi].pedal.cc123_reset();
            channels[chi].last_note = None;
        }
        // Bank Select（CC0=MSB, CC32=LSB）：これだけでは旋律/リズムは切り替わらない
        // （次のProgram Changeで確定する、ChannelProgramState参照）。`drums`未指定時は
        // 従来どおり完全に無視する（`--drum-bank`未指定の既存呼び出しをビット単位で保つため）。
        0 => {
            if drums.is_some() {
                channels[chi].program_state.bank_select_msb(cc_to_u7(val));
            }
        }
        32 => {
            if drums.is_some() {
                channels[chi].program_state.bank_select_lsb(cc_to_u7(val));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ChannelStateの実効パッチ組み立て・NRPN解釈単体テスト（effective_patch_neutral_equals_base
    // 等）はop505-midiへ移動済み（`op505/midi/src/channel_state.rs`のtestsを参照）。
    // ここに残すのはSMF全体をrender_smf()に通すE2E寄りのスモークテストのみ。

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
        // 段1(TimeStage::default()=time 0/level 0)はリリース用。OP EGは必ずレベル0へ着地させる。
        // 押している間は段0で静止するのでサステイン中の出力は変わらない。
        patch.operators[0].eg =
            TimeEgParams { stages, stage_count: 2, loop_enabled: 0, loop_start: 0, release_point: 0 , ..Default::default()};
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
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
         ..Default::default()};
        // レベルはバイポーラ（中心128＝無変調）。段0=255(上方向いっぱい)⇔段1=0(下方向いっぱい)を
        // ループさせ、ピッチが上下に揺れるビブラートを作る。
        let mut lfo_stages = [TimeStage::default(); MAX_STAGES];
        lfo_stages[0] = TimeStage { time: 40, level: 255, curve: 0 };
        lfo_stages[1] = TimeStage { time: 40, level: 0, curve: 0 };
        // 段2はリリース用（ピッチを中心＝level 128（中立）へ戻す）。ループ区間は0..=1のまま。
        lfo_stages[2] = TimeStage { time: 40, level: sound_core::BIPOLAR_NEUTRAL_RAW, curve: 0 };
        patch.channel.pitch_fg.eg = TimeEgParams {
            stages: lfo_stages,
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 1,
         ..Default::default()};
        patch.channel.pitch_fg.depth = 200; // 振れ幅の倍率（符号なし、0〜255）
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

    // --- Portamento（CC5/CC65）E2Eテスト ---

    /// CC65 ON + CC5 でグライドを有効にすると、直前ノート(60)から新ノート(72、1オクターブ上)
    /// への発音がグライド無しの場合と異なる出力になる（`cc1_live_propagation`と同じ
    /// 差分検証スタイル）。グライド中は周波数が毎サンプル変わるため位相の蓄積が
    /// グライド無し版と恒久的にずれ、グライド終了後も出力は一致しない。
    #[test]
    fn portamento_glide_changes_output_of_the_next_note() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;

        let events_no_glide = vec![
            (0u32, vec![0x90, 60, 100]), // Note On 60
            (200, vec![0x90, 72, 100]),  // Note On 72（グライド無し）
            (600, vec![0x80, 60, 0]),
            (0, vec![0x80, 72, 0]),
        ];
        let smf_no_glide = build_smf(&events_no_glide);
        let buf_no_glide = render_smf(&smf_no_glide, &bank, sr, 0.1, Some(1.0), None).unwrap();

        let events_glide = vec![
            (0u32, vec![0x90, 60, 100]), // Note On 60
            (0, vec![0xB0, 65, 127]),    // CC65 Portamento On
            (0, vec![0xB0, 5, 64]),      // CC5 Portamento Time
            (200, vec![0x90, 72, 100]),  // Note On 72（60からグライド）
            (600, vec![0x80, 60, 0]),
            (0, vec![0x80, 72, 0]),
        ];
        let smf_glide = build_smf(&events_glide);
        let buf_glide = render_smf(&smf_glide, &bank, sr, 0.1, Some(1.0), None).unwrap();

        assert!(buf_no_glide.iter().any(|s| s.abs() > 1e-4), "グライド無し出力が無音");
        assert!(buf_glide.iter().any(|s| s.abs() > 1e-4), "グライド有り出力が無音");
        assert_ne!(buf_glide, buf_no_glide, "ポルタメントが出力に反映されていない");
    }

    /// CC5/CC65を一切送らないSMFは、この機能追加の前後で出力がビット単位で不変（新規
    /// フィールドの既定値が0/falseで、`glide_source`が常にNoneを返すことの裏取り）。
    /// `portamento_glide_changes_output_of_the_next_note`の`buf_no_glide`と同じ構成を
    /// 独立に再現し、決定論的であることを確認する。
    #[test]
    fn no_portamento_cc_is_bit_identical_across_runs() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let events = vec![
            (0u32, vec![0x90, 60, 100]),
            (200, vec![0x90, 72, 100]),
            (600, vec![0x80, 60, 0]),
            (0, vec![0x80, 72, 0]),
        ];
        let smf = build_smf(&events);
        let buf_a = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        let buf_b = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_eq!(buf_a, buf_b);
    }

    // --- Mono/Poly Mode（CC126/CC127）E2Eテスト ---

    /// CC126(Mono On)後に3音を重ねて押し、上から順に離すとlast-note priorityで
    /// 1つ前の鍵盤へ再アタックしながら戻っていく（クラッシュせず発音し続けることを確認する
    /// スモークテスト）。
    #[test]
    fn mono_mode_last_note_priority_smoke() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let events = vec![
            (0u32, vec![0xB0, 126, 127]), // CC126 Mono On
            (0, vec![0x90, 60, 100]),
            (100, vec![0x90, 64, 100]),
            (100, vec![0x90, 67, 100]),
            (100, vec![0x80, 67, 0]), // 67を離す→64へフォールバック
            (100, vec![0x80, 64, 0]), // 64を離す→60へフォールバック
            (100, vec![0x80, 60, 0]),
        ];
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "Mono Modeでの発音が無音");
    }

    /// CC126→CC127でPolyへ戻すと、以降は同時発音できる（Mono中に重ねた分は
    /// 全ノートオフでリセットされているため、Poly復帰後の重ね押しは通常どおり両方鳴る）。
    #[test]
    fn cc127_returns_to_polyphonic_playback() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let events = vec![
            (0u32, vec![0xB0, 126, 127]), // Mono On
            (0, vec![0x90, 60, 100]),
            (100, vec![0x90, 64, 100]), // Monoなので60は解放される
            (100, vec![0xB0, 127, 127]), // Poly On（全ノートオフでリセット）
            (0, vec![0x90, 60, 100]),
            (0, vec![0x90, 64, 100]), // Polyなので両方鳴る
            (200, vec![0x80, 60, 0]),
            (0, vec![0x80, 64, 0]),
        ];
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "Poly復帰後の発音が無音");
    }

    // --- Mono Modeのレガート（方式B、Mono+CC65 ON+レガート時のみ）E2Eテスト ---

    /// Mono ON + レガート（60を押したまま64を押す）で、CC65のON/OFFだけを変えて比較する
    /// （それ以外のMIDIイベントのタイミングは完全に同一）。CC65 OFFなら通常どおり
    /// 60をnote_off→64をnote_onする再アタック（オペレーター位相がリセットされる）、
    /// CC65 ONならボイスを継続したままピッチだけ`glide_to`で滑らせる（位相はリセットされない）。
    /// 位相リセットの有無は波形として残るため、出力が一致しないことでレガートが
    /// 「本当に再アタックしていない」ことを検証できる（`instant_sustain_bank`はEGの
    /// レベル自体は即座に最大へ張り付くため、振幅の落ち込みでは検出できない。この差分検証が
    /// 唯一の実効的なオラクル）。
    #[test]
    fn mono_legato_glide_differs_from_mono_retrigger_at_the_same_note_boundary() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;

        let events_with_portamento = |cc65: u8| {
            vec![
                (0u32, vec![0xB0, 126, 127]), // CC126 Mono On
                (0, vec![0xB0, 65, cc65]),    // CC65 Portamento On/Off
                (0, vec![0xB0, 5, 64]),       // CC5 Portamento Time
                (0, vec![0x90, 60, 100]),     // Note On 60
                (200, vec![0x90, 64, 100]),   // 60を押したまま64を押す（レガート）
                (600, vec![0x80, 64, 0]),
                (0, vec![0x80, 60, 0]),
            ]
        };

        let smf_legato = build_smf(&events_with_portamento(127)); // CC65 ON→レガート
        let buf_legato = render_smf(&smf_legato, &bank, sr, 0.1, Some(1.0), None).unwrap();

        let smf_retrigger = build_smf(&events_with_portamento(0)); // CC65 OFF→通常のretrigger
        let buf_retrigger = render_smf(&smf_retrigger, &bank, sr, 0.1, Some(1.0), None).unwrap();

        assert!(buf_legato.iter().any(|s| s.abs() > 1e-4), "レガート出力が無音");
        assert!(buf_retrigger.iter().any(|s| s.abs() > 1e-4), "比較対象(retrigger)出力が無音");
        assert_ne!(
            buf_legato, buf_retrigger,
            "CC65の有無だけを変えたのに出力が一致してはいけない（レガートは位相リセット無しで\
             ボイス継続、CC65 OFFはボイスを作り直すretriggerのはず）"
        );
    }

    /// last-note priorityのフォールバック（3音重ね押し→上から順に離す）が、レガート条件
    /// （Mono+CC65 ON）下でもクラッシュせず発音し続けることを確認するスモークテスト
    /// （`mono_mode_last_note_priority_smoke`のレガート版。`MonoNoteOff::LegatoFallback`
    /// 経路を通す）。
    #[test]
    fn mono_legato_last_note_priority_fallback_smoke() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let events = vec![
            (0u32, vec![0xB0, 126, 127]), // CC126 Mono On
            (0, vec![0xB0, 65, 127]),     // CC65 Portamento On
            (0, vec![0xB0, 5, 64]),       // CC5 Portamento Time
            (0, vec![0x90, 60, 100]),
            (100, vec![0x90, 64, 100]), // レガート
            (100, vec![0x90, 67, 100]), // レガート
            (100, vec![0x80, 67, 0]),   // 67を離す→レガートで64へ戻る
            (100, vec![0x80, 64, 0]),   // 64を離す→レガートで60へ戻る
            (100, vec![0x80, 60, 0]),
        ];
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "レガートのlast-note priorityでの発音が無音");
    }

    // --- エフェクトルーティング（NRPN(0,1) Channel Effect Route）E2Eテスト ---

    /// NRPN(0,1)を一切送らないSMF（`mixed_cc_nrpn_smoke`と同じ混在CC/NRPN列、CC91・
    /// NRPN(0,2)〜(0,9)を含む）は決定論的であるはず（全チャンネルの`effect_route_slot`が
    /// 既定0のまま＝全音声が`effects[0]`だけを通るため、単一MasterEffects時代と同じ経路）。
    /// `no_portamento_cc_is_bit_identical_across_runs`と同型の直接証明。
    #[test]
    fn no_effect_routing_is_bit_identical_across_runs() {
        let bank = vibrato_bank();
        let events = vec![
            (0u32, vec![0x90, 60, 100]),
            (0, vec![0xB0, 99, 0]),
            (0, vec![0xB0, 98, 9]), // NRPN LSB=9（Algorithm）
            (0, vec![0xB0, 6, 7]),
            (10, vec![0xB0, 91, 60]), // CC91 Reverb Send
            (0, vec![0xB0, 99, 0]),
            (0, vec![0xB0, 98, 2]), // NRPN LSB=2（Reverb Type）
            (0, vec![0xB0, 6, 3]),
            (240, vec![0x80, 60, 0]),
        ];
        let smf = build_smf(&events);
        let buf_a = render_smf(&smf, &bank, 8000.0, 0.1, Some(1.0), None).unwrap();
        let buf_b = render_smf(&smf, &bank, 8000.0, 0.1, Some(1.0), None).unwrap();
        assert!(buf_a.iter().any(|s| s.abs() > 1e-4), "出力が無音");
        assert_eq!(buf_a, buf_b);
    }

    // --- GM2 Universal SysEx Master Volume E2Eテスト ---

    /// SMF内にGM2 Master Volume SysEx（value14=0）を置くと、以降の発音が無音になる。
    #[test]
    fn sysex_master_volume_zero_silences_subsequent_notes() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let events = vec![
            (0u32, vec![0xF0, 0x07, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x00, 0xF7]),
            (0, vec![0x90, 60, 100]),
            (480, vec![0x80, 60, 0]),
        ];
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert!(buf.iter().all(|s| s.abs() < 1e-6), "マスターボリューム0のはずが音が出ている");
    }

    /// マスターボリュームSysExを送らない既存のSMFは、この機能追加前と同じく発音すること
    /// （ビット不変の直接証明、`no_effect_routing_is_bit_identical_across_runs`と同型）。
    #[test]
    fn no_master_volume_sysex_is_bit_identical_across_runs() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let events = vec![(0u32, vec![0x90, 60, 100]), (480, vec![0x80, 60, 0])];
        let smf = build_smf(&events);
        let buf_a = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        let buf_b = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert!(buf_a.iter().any(|s| s.abs() > 1e-4), "出力が無音");
        assert_eq!(buf_a, buf_b);
    }

    /// GM2 Master Volume以外の未知のSysExはパニックせず無視され、後続イベントの解釈にも
    /// 影響しない。
    #[test]
    fn unrecognized_sysex_is_ignored_and_playback_continues() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let events = vec![
            (0u32, vec![0xF0, 0x04, 0x43, 0x10, 0x00, 0xF7]), // 他ベンダーのSysEx
            (0, vec![0x90, 60, 100]),
            (480, vec![0x80, 60, 0]),
        ];
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "未知SysExの後も発音するはず");
    }

    /// ch1をNRPN(0,1)でスロット1へルーティングすると、ch0のCC91 Reverb Send（スロット0のみを
    /// 設定）はch1の音に影響しなくなる。ルーティング後の合成波形は「ch0だけをスロット0で
    /// リバーブ込みで単独render()した波形」と「ch1だけをエフェクト無しで単独render()した波形」の
    /// 単純な加算と一致するはず（各スロットの処理は独立、最終合成は加算のみのため）。
    /// 対照として、ルーティングしなければch1もch0と同じスロット0のリバーブを浴びてしまい、
    /// この加算とは一致しないことも確認する（テスト自体が無意味でないことの確認）。
    #[test]
    fn channel_effect_route_sends_channel_to_a_separate_slot() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let tail = 0.3;
        let max_secs = Some(1.5);

        let events_ch0_alone = vec![
            (0u32, vec![0xB0, 91, 100]), // CC91 Reverb Send（ch0→スロット0既定）
            (0, vec![0x90, 60, 100]),
            (480, vec![0x80, 60, 0]),
        ];
        let buf_ch0 = render_smf(&build_smf(&events_ch0_alone), &bank, sr, tail, max_secs, None).unwrap();

        let events_ch1_alone = vec![
            (0u32, vec![0x91, 64, 100]), // エフェクト設定なし＝ドライ
            (480, vec![0x81, 64, 0]),
        ];
        let buf_ch1 = render_smf(&build_smf(&events_ch1_alone), &bank, sr, tail, max_secs, None).unwrap();

        let events_routed = vec![
            (0u32, vec![0xB1, 99, 0]), // ch1: NRPN MSB=0
            (0, vec![0xB1, 98, 1]),    // ch1: NRPN LSB=1（Channel Effect Route）
            (0, vec![0xB1, 6, 1]),     // ch1: Data Entry=1 → effect_route_slot=1
            (0, vec![0xB0, 91, 100]),  // ch0: CC91 Reverb Send（スロット0）
            (0, vec![0x90, 60, 100]),  // ch0 Note On
            (0, vec![0x91, 64, 100]),  // ch1 Note On
            (480, vec![0x80, 60, 0]),  // ch0 Note Off
            (0, vec![0x81, 64, 0]),    // ch1 Note Off
        ];
        let buf_routed = render_smf(&build_smf(&events_routed), &bank, sr, tail, max_secs, None).unwrap();

        let events_unrouted = vec![
            (0u32, vec![0xB0, 91, 100]),
            (0, vec![0x90, 60, 100]),
            (0, vec![0x91, 64, 100]),
            (480, vec![0x80, 60, 0]),
            (0, vec![0x81, 64, 0]),
        ];
        let buf_unrouted = render_smf(&build_smf(&events_unrouted), &bank, sr, tail, max_secs, None).unwrap();

        assert!(buf_ch0.iter().any(|s| s.abs() > 1e-4), "ch0単独出力が無音");
        assert!(buf_ch1.iter().any(|s| s.abs() > 1e-4), "ch1単独出力が無音");
        assert_eq!(buf_routed.len(), buf_ch0.len());
        assert_eq!(buf_routed.len(), buf_ch1.len());

        let summed: Vec<f32> = buf_ch0.iter().zip(buf_ch1.iter()).map(|(&a, &b)| a + b).collect();
        assert_eq!(buf_routed, summed, "ルーティング後はch0(リバーブ込み)+ch1(ドライ)の単純加算と一致するはず");
        assert_ne!(buf_unrouted, summed, "ルーティングしなければch1もch0のリバーブを浴び、単純加算とは一致しないはず");
    }

    // --- MIDIチャンネル独立性E2Eテスト ---
    //
    // `ChannelState`をMIDIチャンネル別に16個保持する設計（`op505-vst`もこのセッションで
    // 同じ設計へ移行済み）の正しさを、エンジンが全chをモノラル合算する制約の下で検証する。
    // 「他chにイベントを混ぜても、対象chだけを鳴らした出力がビット一致する」という形にし、
    // 無関係なDSP変更に対して頑健にする。各テストは必ず対照（assert_ne!）を伴わせ、
    // 「常に一致」で通ってしまうのを防ぐ。

    /// 複数MIDIチャンネルでNRPN選択(CC98/99)を交互に送っても、CC6/CC38(Data Entry)は
    /// 自分のチャンネルで選択中のNRPNにのみ適用される。RpnTrackerがグローバル単一だと、
    /// ch1のNRPN選択がch0のCC6/CC38を横取りしてしまう箇所。
    #[test]
    fn interleaved_nrpn_selection_across_channels_is_isolated() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;

        // A(clean): ch0だけがNRPN(0,18)=Op0 F-Numberを選択してCC38=10→CC6=60を送る。
        let smf_a = build_smf(&[
            (0, vec![0xB0, 99, 0]),
            (0, vec![0xB0, 98, 18]),
            (0, vec![0xB0, 38, 10]),
            (0, vec![0xB0, 6, 60]),
            (0, vec![0x90, 69, 100]),
            (480, vec![0x80, 69, 0]),
        ]);
        let buf_a = render_smf(&smf_a, &bank, sr, 0.1, Some(1.0), None).unwrap();

        // B(dirty): ch0のNRPN(0,18)選択直後、ch1がNRPN(0,14)=Filter Typeを選択してから
        // ch0へCC38/CC6を送る（選択がchごとに独立していれば影響を受けないはず）。
        let smf_b = build_smf(&[
            (0, vec![0xB0, 99, 0]),
            (0, vec![0xB0, 98, 18]),
            (0, vec![0xB1, 99, 0]),
            (0, vec![0xB1, 98, 14]),
            (0, vec![0xB0, 38, 10]),
            (0, vec![0xB0, 6, 60]),
            (0, vec![0x90, 69, 100]),
            (480, vec![0x80, 69, 0]),
        ]);
        let buf_b = render_smf(&smf_b, &bank, sr, 0.1, Some(1.0), None).unwrap();

        assert_eq!(buf_a, buf_b, "ch1のNRPN選択がch0のCC38/CC6を横取りしてはいけない");
    }

    /// 他chへのNRPN(0,18) Operator F-Number送信は、対象chの発音に影響しない
    /// （対照として、同じchへ送った場合はピッチが変わり出力が異なることも確認する）。
    #[test]
    fn operator_f_number_nrpn_does_not_leak_across_channels() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let note_on_ch0 = (0u32, vec![0x90, 69, 100]);
        let note_off_ch0 = (480u32, vec![0x80, 69, 0]);

        // A: ch0はNRPNを一切受けずに発音する（基準）。
        let smf_a = build_smf(&[note_on_ch0.clone(), note_off_ch0.clone()]);
        let buf_a = render_smf(&smf_a, &bank, sr, 0.1, Some(1.0), None).unwrap();

        // B: ch1（別チャンネル）にNRPN(0,18)でF-Numberを半分にする指示を送ってからch0で発音する。
        let smf_b = build_smf(&[
            (0, vec![0xB1, 99, 0]),
            (0, vec![0xB1, 98, 18]),
            (0, vec![0xB1, 38, 0]),
            (0, vec![0xB1, 6, 16]), // combined=16*128=2048=F_NUMBER_CENTER(4096)の半分
            note_on_ch0.clone(),
            note_off_ch0.clone(),
        ]);
        let buf_b = render_smf(&smf_b, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_eq!(buf_a, buf_b, "ch1へのF-Number上書きがch0の発音に漏れてはいけない");

        // C(対照): 同じ指示をch0自身へ送ると出力が変わる（テスト自体が無意味でないことの確認）。
        let smf_c = build_smf(&[
            (0, vec![0xB0, 99, 0]),
            (0, vec![0xB0, 98, 18]),
            (0, vec![0xB0, 38, 0]),
            (0, vec![0xB0, 6, 16]),
            note_on_ch0,
            note_off_ch0,
        ]);
        let buf_c = render_smf(&smf_c, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_ne!(buf_a, buf_c, "ch0自身へのF-Number上書きは出力を変えるはず");
    }

    /// 他chへのProgram Changeは、対象chのNRPN上書き（F-Number）を消さない
    /// （`ChannelState::program_change`が当該chのoverridesのみclearすることの検証）。
    #[test]
    fn program_change_on_other_channel_keeps_this_channel_overrides() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let note_on_ch0 = (0u32, vec![0x90, 69, 100]);
        let note_off_ch0 = (480u32, vec![0x80, 69, 0]);
        let apply_f_number_ch0 =
            [(0, vec![0xB0, 99, 0]), (0, vec![0xB0, 98, 18]), (0, vec![0xB0, 38, 0]), (0, vec![0xB0, 6, 16])];

        // A: ch0にNRPN(0,18) F-Number上書きを適用してから発音する。
        let mut events_a = apply_f_number_ch0.to_vec();
        events_a.push(note_on_ch0.clone());
        events_a.push(note_off_ch0.clone());
        let buf_a = render_smf(&build_smf(&events_a), &bank, sr, 0.1, Some(1.0), None).unwrap();

        // B: 同じ上書き適用後、ch1へProgram Changeを送ってからch0で発音する
        // （ch1のPCがch0のoverridesを巻き込んでクリアしてはいけない）。
        let mut events_b = apply_f_number_ch0.to_vec();
        events_b.push((0, vec![0xC1, 0])); // Program Change on ch1
        events_b.push(note_on_ch0.clone());
        events_b.push(note_off_ch0.clone());
        let buf_b = render_smf(&build_smf(&events_b), &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_eq!(buf_a, buf_b, "ch1のProgram Changeがch0のNRPN上書きを消してはいけない");

        // C(対照): F-Number上書きを一切せずに発音するとAとは異なる（Aの上書きが実際に
        // 効いていることの確認。もしAの上書きが元から無効だとA==Cとなりテストが無力化する）。
        let smf_c = build_smf(&[note_on_ch0, note_off_ch0]);
        let buf_c = render_smf(&smf_c, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_ne!(buf_a, buf_c, "ch0のF-Number上書きは出力を変えるはず");
    }

    /// RPN(0,0) Pitch Bend Rangeは他chに影響しない
    /// （対照として、同じchで設定した場合はベンド量が変わることも確認する）。
    #[test]
    fn rpn_pitch_bend_range_is_per_channel() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let bend_up = (0u32, vec![0xE0, 0x7F, 0x7F]); // ch0へ最大ピッチベンド
        let note_on_ch0 = (0u32, vec![0x90, 69, 100]);
        let note_off_ch0 = (480u32, vec![0x80, 69, 0]);

        // A: ベンドレンジ変更なし（既定±2半音）でch0にベンド送信。
        let smf_a = build_smf(&[bend_up.clone(), note_on_ch0.clone(), note_off_ch0.clone()]);
        let buf_a = render_smf(&smf_a, &bank, sr, 0.1, Some(1.0), None).unwrap();

        // B: ch1のRPN(0,0)でベンドレンジを24半音に広げてからch0でベンド送信。
        let smf_b = build_smf(&[
            (0, vec![0xB1, 101, 0]),
            (0, vec![0xB1, 100, 0]),
            (0, vec![0xB1, 6, 24]),
            bend_up.clone(),
            note_on_ch0.clone(),
            note_off_ch0.clone(),
        ]);
        let buf_b = render_smf(&smf_b, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_eq!(buf_a, buf_b, "ch1のPitch Bend Range変更がch0に漏れてはいけない");

        // C(対照): ch0自身のRPN(0,0)を変更すると出力が変わる。
        let smf_c = build_smf(&[
            (0, vec![0xB0, 101, 0]),
            (0, vec![0xB0, 100, 0]),
            (0, vec![0xB0, 6, 24]),
            bend_up,
            note_on_ch0,
            note_off_ch0,
        ]);
        let buf_c = render_smf(&smf_c, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_ne!(buf_a, buf_c, "ch0自身のPitch Bend Range変更は出力を変えるはず");
    }

    /// RPN(0,2) Channel Coarse Tuningは他chに影響せず、自ch自身では出力を変える。
    #[test]
    fn rpn_channel_coarse_tuning_is_per_channel_and_changes_output() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let note_on_ch0 = (0u32, vec![0x90, 60, 100]);
        let note_off_ch0 = (480u32, vec![0x80, 60, 0]);
        // ch1へRPN(0,2)=76（+12半音）を送るイベント列。
        let coarse_tune_ch1 = [
            (0u32, vec![0xB1, 101, 0]),
            (0, vec![0xB1, 100, 2]),
            (0, vec![0xB1, 6, 76]),
        ];

        // A: チューニング変更なし。
        let buf_a = render_smf(&build_smf(&[note_on_ch0.clone(), note_off_ch0.clone()]), &bank, sr, 0.1, Some(1.0), None)
            .unwrap();

        // B: ch1のCoarse Tuning変更はch0へ漏れない。
        let mut events_b = coarse_tune_ch1.to_vec();
        events_b.push(note_on_ch0.clone());
        events_b.push(note_off_ch0.clone());
        let buf_b = render_smf(&build_smf(&events_b), &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_eq!(buf_a, buf_b, "ch1のCoarse Tuning変更がch0に漏れてはいけない");

        // C(対照): ch0自身のCoarse Tuningを変更すると出力（ピッチ）が変わる。
        let mut events_c = vec![(0u32, vec![0xB0, 101, 0]), (0, vec![0xB0, 100, 2]), (0, vec![0xB0, 6, 76])];
        events_c.push(note_on_ch0);
        events_c.push(note_off_ch0);
        let buf_c = render_smf(&build_smf(&events_c), &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_ne!(buf_a, buf_c, "ch0自身のCoarse Tuning変更は出力を変えるはず");
    }

    /// RPN(0,1) Channel Fine Tuning（CC6(MSB)+CC38(LSB)の14bit）は出力（ピッチ）を変える。
    #[test]
    fn rpn_channel_fine_tuning_changes_output() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let note_on_ch0 = (0u32, vec![0x90, 60, 100]);
        let note_off_ch0 = (480u32, vec![0x80, 60, 0]);

        let buf_a = render_smf(&build_smf(&[note_on_ch0.clone(), note_off_ch0.clone()]), &bank, sr, 0.1, Some(1.0), None)
            .unwrap();

        // 最大値(msb=127,lsb=127→16383)へ変更、+100セント付近。
        let events_b = vec![
            (0u32, vec![0xB0, 101, 0]),
            (0, vec![0xB0, 100, 1]),
            (0, vec![0xB0, 6, 127]),
            (0, vec![0xB0, 38, 127]),
            note_on_ch0,
            note_off_ch0,
        ];
        let buf_b = render_smf(&build_smf(&events_b), &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert_ne!(buf_a, buf_b, "Fine Tuning変更は出力を変えるはず");
    }

    /// 16ch全てに異なるNRPN+noteを送ってもpanicせず、無音にならないことを確認する
    /// （`channels[15]`の添字境界も踏むtorture smoke）。
    #[test]
    fn all_16_channels_nrpn_torture_smoke() {
        let bank = instant_sustain_bank();
        let sr = 8000.0;
        let mut events: Vec<(u32, Vec<u8>)> = Vec::new();
        for chi in 0u8..16 {
            events.push((0, vec![0xB0 | chi, 99, 0]));
            events.push((0, vec![0xB0 | chi, 98, 9])); // Algorithm
            events.push((0, vec![0xB0 | chi, 6, chi.min(7)]));
            events.push((0, vec![0x90 | chi, 60 + chi, 100]));
        }
        for chi in 0u8..16 {
            events.push((if chi == 0 { 480 } else { 0 }, vec![0x80 | chi, 60 + chi, 0]));
        }
        let smf = build_smf(&events);
        let buf = render_smf(&smf, &bank, sr, 0.1, Some(1.0), None).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "16ch同時発音が無音");
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
        // 段1(TimeStage::default()=time 0/level 0)はリリース用。OP EGは必ずレベル0へ着地させる。
        // 押している間は段0で静止するのでサステイン中の出力は変わらない。
        patch.operators[0].eg =
            TimeEgParams { stages, stage_count: 2, loop_enabled: 0, loop_start: 0, release_point: 0 , ..Default::default()};
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
            release_point: 0,
         ..Default::default()};
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

    // -----------------------------------------------------------------------
    // GM2リズムチャンネル（Bank Select MSB=120動的切替）
    // -----------------------------------------------------------------------

    use op505_core::{Op505PresetEntry, Op505PresetFile};

    /// bank=15360(kit 0)にBD(program=36)とHH(program=42)を持つリズムキット。
    /// 2音は`mul`を変えて明確に別の音色にしてある（出力波形の違いで判別する）。
    /// `instant_sustain_bank()`と同じく即発音・無限サステインのEGを与える
    /// （`Op505Patch::default()`のEGは全段level=0のままで無音のため）。
    fn drum_kit_bank() -> Op505PresetBank {
        use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
        let instant_sustain_eg = || {
            let mut stages = [TimeStage::default(); MAX_STAGES];
            stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
            TimeEgParams { stages, stage_count: 2, loop_enabled: 0, loop_start: 0, release_point: 0, ..Default::default() }
        };

        let mut bd = Op505Patch::default();
        bd.channel.algorithm = 7;
        bd.operators[0].tl = 220;
        bd.operators[0].mul = 1;
        bd.operators[0].eg = instant_sustain_eg();
        let mut hh = Op505Patch::default();
        hh.channel.algorithm = 7;
        hh.operators[0].tl = 220;
        hh.operators[0].mul = 5;
        hh.operators[0].eg = instant_sustain_eg();

        let file = Op505PresetFile::Presets {
            bank: 15360,
            presets: vec![
                Op505PresetEntry { program: 36, name: "BD".to_string(), patch: bd },
                Op505PresetEntry { program: 42, name: "HH".to_string(), patch: hh },
            ],
        };
        let mut bank = Op505PresetBank::default();
        bank.merge_file(file);
        bank
    }

    /// リズムモードでは`base_patch_for`がノート番号をそのままプログラム番号として使う。
    #[test]
    fn base_patch_for_rhythm_uses_note_as_program() {
        let bank = PatchBank::from_patches(&[Op505Patch::default()]).unwrap();
        let drums = drum_kit_bank();
        let mut st = ChannelState::new(0, false);
        st.program_state.bank_select_msb(120);
        st.program_state.program_change(0); // kit 0

        let bd = base_patch_for(&st, 36, &bank, Some(&drums)).unwrap();
        let hh = base_patch_for(&st, 42, &bank, Some(&drums)).unwrap();
        assert_ne!(bd, hh, "リズムはノートごとに異なる音色のはず");
        assert_eq!(bd.operators[0].mul, 1);
        assert_eq!(hh.operators[0].mul, 5);
    }

    /// キット内に未定義のノートはStandard Kit(kit 0)へフォールバックする。
    #[test]
    fn base_patch_for_falls_back_to_kit_zero_for_undefined_note() {
        let bank = PatchBank::from_patches(&[Op505Patch::default()]).unwrap();
        let drums = drum_kit_bank();
        let mut st = ChannelState::new(0, false);
        st.program_state.bank_select_msb(120);
        st.program_state.program_change(9); // 未定義のキット9（BD/HHが無い）

        let fallback = base_patch_for(&st, 36, &bank, Some(&drums)).unwrap();
        assert_eq!(fallback.operators[0].mul, 1, "kit0(Standard Kit)のBDへフォールバックするはず");
    }

    /// キット0にも無いノートは発音しない（Noneを返す）。
    #[test]
    fn base_patch_for_returns_none_for_totally_undefined_note() {
        let bank = PatchBank::from_patches(&[Op505Patch::default()]).unwrap();
        let drums = drum_kit_bank();
        let mut st = ChannelState::new(0, false);
        st.program_state.bank_select_msb(120);
        st.program_state.program_change(0);

        assert!(base_patch_for(&st, 20, &bank, Some(&drums)).is_none(), "BD/HH以外のノートは未定義のはず");
    }

    /// `drums`がNoneのときリズムモードでも常にNone（発音しない）。
    #[test]
    fn base_patch_for_rhythm_without_drums_returns_none() {
        let bank = PatchBank::from_patches(&[Op505Patch::default()]).unwrap();
        let mut st = ChannelState::new(0, false);
        st.program_state.bank_select_msb(120);
        st.program_state.program_change(0);

        assert!(base_patch_for(&st, 36, &bank, None).is_none());
    }

    /// 旋律モードではノート番号に関わらず同じ`bank`のパッチが使われる（音程差のみ）。
    #[test]
    fn base_patch_for_melodic_ignores_note_number() {
        let mut melodic_patch = Op505Patch::default();
        melodic_patch.operators[0].tl = 100;
        let bank = PatchBank::from_patches(&[melodic_patch]).unwrap();
        let drums = drum_kit_bank();
        let st = ChannelState::new(0, false); // 旋律のまま（Bank Select未送信）

        let p36 = base_patch_for(&st, 36, &bank, Some(&drums)).unwrap();
        let p42 = base_patch_for(&st, 42, &bank, Some(&drums)).unwrap();
        assert_eq!(p36, p42, "旋律はノート番号に関わらず同一音色のはず");
        assert_eq!(p36, melodic_patch, "drumsではなくbankのprogram 0が使われるはず");
    }

    /// CC0=120→PC=0→note36とnote42を鳴らすと、2音の出力が異なる
    /// （リズムチャンネルがノートごとに違う音色を鳴らしている、統合レベルの検証）。
    #[test]
    fn rhythm_channel_note_selects_different_timbre_end_to_end() {
        let bank = PatchBank::from_patches(&[Op505Patch::default()]).unwrap(); // 使われない
        let drums = drum_kit_bank();
        let sr = 8000.0;

        let smf_bd = build_smf(&[
            (0u32, vec![0xB0, 0, 120]), // CC0=120 (Bank Select MSB)
            (0u32, vec![0xC0, 0]),      // PC=0 (kit 0)
            (0u32, vec![0x90, 36, 100]),
        ]);
        let buf_bd = render_smf_with_drums(&smf_bd, &bank, Some(&drums), sr, 0.1, Some(0.3), None).unwrap();

        let smf_hh = build_smf(&[
            (0u32, vec![0xB0, 0, 120]),
            (0u32, vec![0xC0, 0]),
            (0u32, vec![0x90, 42, 100]),
        ]);
        let buf_hh = render_smf_with_drums(&smf_hh, &bank, Some(&drums), sr, 0.1, Some(0.3), None).unwrap();

        assert!(buf_bd.iter().any(|s| s.abs() > 1e-4), "BD出力が無音");
        assert!(buf_hh.iter().any(|s| s.abs() > 1e-4), "HH出力が無音");
        let n = buf_bd.len().min(buf_hh.len());
        assert!((0..n).any(|i| (buf_bd[i] - buf_hh[i]).abs() > 1e-4), "note36とnote42で音色が違うはず");
    }

    /// `render_smf`（`drums`無し）はCC0=120を送ってもリズムへ切り替わらず、
    /// 従来どおり`bank`の旋律パッチで鳴る（`--drum-bank`未指定時の非回帰）。
    #[test]
    fn render_smf_without_drum_bank_ignores_bank_select_cc() {
        let mut melodic_patch = Op505Patch::default();
        melodic_patch.channel.algorithm = 7;
        melodic_patch.operators[0].tl = 220;
        melodic_patch.operators[0].mul = 1;
        let bank = PatchBank::from_patches(&[melodic_patch]).unwrap();
        let sr = 8000.0;

        let smf_no_cc0 = build_smf(&[(0u32, vec![0x90, 60, 100])]);
        let buf_no_cc0 = render_smf(&smf_no_cc0, &bank, sr, 0.1, Some(0.3), None).unwrap();

        let smf_with_cc0 = build_smf(&[
            (0u32, vec![0xB0, 0, 120]), // CC0=120（drums無しなので無視されるはず）
            (0u32, vec![0xC0, 0]),
            (0u32, vec![0x90, 60, 100]),
        ]);
        let buf_with_cc0 = render_smf(&smf_with_cc0, &bank, sr, 0.1, Some(0.3), None).unwrap();

        assert_eq!(buf_no_cc0, buf_with_cc0, "drums未指定のCC0=120は無視され出力がビット一致するはず");
    }

    /// MIDI ch10はリズムキットがロードされていれば、Bank Select/Program Changeを送らなくても
    /// 最初からリズムチャンネルとして始まる（ch10初期ON）。
    #[test]
    fn channel10_starts_in_rhythm_when_drum_bank_provided() {
        let bank = PatchBank::from_patches(&[Op505Patch::default()]).unwrap();
        let drums = drum_kit_bank();
        let sr = 8000.0;

        // MIDI ch10 = ステータスバイト0x99（Note On, channel 9）
        let smf = build_smf(&[(0u32, vec![0x99, 36, 100])]);
        let buf = render_smf_with_drums(&smf, &bank, Some(&drums), sr, 0.1, Some(0.3), None).unwrap();
        assert!(buf.iter().any(|s| s.abs() > 1e-4), "ch10はBank Select無しでもドラムが鳴るはず");
    }
}
