#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod op505_presets;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use op505_core::{
    build_op505_registry, op505_presets_dir, Op505BankRegistry, Op505BipolarFg, Op505ChannelParams, Op505Engine, Op505Patch, Op505PresetBank,
};
use sound_core::{
    cc76_to_rate_scale, cutoff_depth, pitch_depth_cents, seconds_to_time, volume_depth,
    AudioProcessor, ChorusType, MasterEffects, ReverbType, TimeEgParams, TimeStage,
    Vco, BIPOLAR_NEUTRAL_RAW, MAX_STAGES, RETRIGGER_MODE_RESET,
};
use sound_fm::chip_lfo::chip_lfo_freq_to_hz;
use std::sync::{Arc, Mutex};

/// Destination（`0`〜`4`、`FmLfoDestination`の値と同じ並び）。旧`sound_fm::FmLfoDestination`は
/// 質感LFO退役に伴い削除済みのため、gesture-app内だけで使う最小限の解釈をここに持つ
/// （NRPN(0,0)と同じ生値: Unplugged=0/Pitch=1/Volume=2/TlCarrier=3/Cutoff=4）。
#[derive(Clone, Copy, PartialEq)]
enum PerformanceLfoDestination {
    Unplugged,
    Pitch,
    Volume,
    TlCarrier,
    Cutoff,
}

impl PerformanceLfoDestination {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Pitch,
            2 => Self::Volume,
            3 => Self::TlCarrier,
            4 => Self::Cutoff,
            _ => Self::Unplugged,
        }
    }
}

/// Vキーの演奏系LFO（ビブラート/トレモロ/オートワウ）が上書きする直前の、音色プリセット本来の
/// Pitch/Gain/Cutoff FG設定。質感LFO退役（`ChannelParams.texture_lfo`廃止）に伴い、演奏系LFOは
/// 独立したLFOスロットではなく各FG自体を一時的に差し替える方式になったため、`op505_set_patch`/
/// `op505_set_program`が呼ばれるたびにここを更新し、`op505_set_performance_lfo`は毎回ここを
/// 基準に3つのFGを再構築する（音色プリセットのFGを演奏系LFOが恒久的に破壊しないため。
/// 詳細はmemory `project_texture_lfo_retirement.md`）。
#[derive(Clone, Copy)]
struct OriginalFgs {
    pitch_fg: Op505BipolarFg,
    gain_fg: TimeEgParams,
    gain_fg_to_master: bool,
    gain_fg_to_operators: bool,
    cutoff_fg: Op505BipolarFg,
}

impl Default for OriginalFgs {
    fn default() -> Self {
        Self::capture(&Op505ChannelParams::default())
    }
}

impl OriginalFgs {
    fn capture(ch: &Op505ChannelParams) -> Self {
        Self {
            pitch_fg: ch.pitch_fg,
            gain_fg: ch.gain_fg,
            gain_fg_to_master: ch.gain_fg_to_master,
            gain_fg_to_operators: ch.gain_fg_to_operators,
            cutoff_fg: ch.cutoff_fg,
        }
    }
}

/// 中央(128)でdelay秒待機→谷(0)へ瞬時ジャンプ→山(255)⇄谷(0)を`half_period_a`/`half_period_b`秒
/// ずつ往復する4段バイポーラループ（Pitch FG/Cutoff FGのビブラート/オートワウ共通形）。
fn build_bipolar_vibrato_eg(delay_seconds: f32, half_period_a: f32, half_period_b: f32) -> TimeEgParams {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    stages[0] = TimeStage { time: seconds_to_time(delay_seconds), level: BIPOLAR_NEUTRAL_RAW, curve: 0 };
    stages[1] = TimeStage { time: 0, level: 0, curve: 0 };
    stages[2] = TimeStage { time: seconds_to_time(half_period_a), level: 255, curve: 0 };
    stages[3] = TimeStage { time: seconds_to_time(half_period_b), level: 0, curve: 0 };
    TimeEgParams {
        stages,
        stage_count: 4,
        loop_enabled: 1,
        loop_start: 2,
        release_point: 3,
        retrigger_mode: RETRIGGER_MODE_RESET,
        ..TimeEgParams::default()
    }
}

/// Gain FGのトレモロループ（`op505_core::chip_lfo_am_to_gain_fg`と同型の5段構成）。
/// 「平坦T/4→下降T/4→上昇T/4→平坦T/4+delay」でクランプ済み三角波のAM変調を厳密に再現する。
fn build_tremolo_gain_fg(delay_seconds: f32, hz: f32, depth: f32) -> TimeEgParams {
    let quarter_period = 0.25 / hz;
    let half_period = 0.5 / hz;
    let floor_level = ((1.0 - depth) * 255.0).round().clamp(0.0, 255.0) as u8;

    let mut stages = [TimeStage::default(); MAX_STAGES];
    stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
    stages[1] = TimeStage { time: seconds_to_time(delay_seconds + quarter_period), level: 255, curve: 0 };
    stages[2] = TimeStage { time: seconds_to_time(quarter_period), level: floor_level, curve: 0 };
    stages[3] = TimeStage { time: seconds_to_time(quarter_period), level: 255, curve: 0 };
    stages[4] = TimeStage { time: seconds_to_time(half_period), level: 255, curve: 0 };
    TimeEgParams {
        stages,
        stage_count: 5,
        loop_enabled: 1,
        loop_start: 2,
        release_point: 4,
        retrigger_mode: RETRIGGER_MODE_RESET,
        ..TimeEgParams::default()
    }
}

/// 指定チャンネルIDへキーオンする。チャンネルIDは呼び出し側（フロントエンド）が
/// 安定したスロット番号として供給する。発音中/リリース中のチャンネルが既にあっても、
/// エンベロープを即座にカットしてAttackから再開する（実機Key-On挙動に準拠＝同音チョーク）。
/// 押し直し時に同じスロットIDを渡すことで、直前のリリーステールがチョークされる。
/// 音色は`op505_set_program`/`op505_set_patch`で設定した保存済みパッチ（`current_patch`）を使う。
#[tauri::command]
fn note_on(
    engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>,
    channel: usize,
    frequency: f32,
    velocity: u8,
) {
    engine.lock().unwrap().note_on(channel, frequency, velocity);
}

#[tauri::command]
fn note_off(engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>, channel: usize) {
    engine.lock().unwrap().note_off(channel);
}

/// Pitch FGループのベース周期（`pitch_fg_rate_scale`=1.0＝CC76無補正のとき）。
/// AR≈85ms・D1R≈85msで往復1周期≈170ms≈5.9Hzのビブラートになる（旧ym38x6版のレート方式EGでの
/// 初期案AR=136/D1R=199の実測時間を踏襲）。
const PITCH_FG_VIBRATO_AR_SECONDS: f32 = 0.085;
const PITCH_FG_VIBRATO_D1R_SECONDS: f32 = 0.085;

/// OP505エンジンの演奏系モジュレーション（Vキーのビブラート⇔トレモロ切替）を設定する。
/// `destination`は`PerformanceLfoDestination`の値（Unplugged=0/Pitch=1/Volume=2/TlCarrier=3/
/// Cutoff=4、旧`FmLfoDestination`と同じ生値）。
///
/// 質感LFO退役（`ChannelParams.texture_lfo`廃止）に伴い、独立したLFOスロットは無くなった。
/// 代わりに、選ばれたdestinationに対応するFG（Pitch/Gain/Cutoff）自体をループへ差し替え、
/// **選ばれなかったFGは`original_fgs`（`op505_set_patch`/`op505_set_program`が更新する、
/// 音色プリセット本来の値）へ毎回戻す**。呼び出しのたびに3つのFG全てをこの規則で再構築するため、
/// 呼び出し順序に依存せず一貫した結果になる（音色プリセットのFGを演奏系LFOが恒久的に
/// 破壊しないための設計、詳細はmemory `project_texture_lfo_retirement.md`）。
///
/// - Pitch: 中央でdelay待機→谷へ瞬時ジャンプ→山⇄谷を往復する4段ループ（旧CHIP LFOピッチ経路の
///   `chip_lfo_pitch_to_pitch_fg`と同型）。速さはベース周期を固定し、CC76由来の
///   `pitch_fg_rate_scale`（`set_pitch_fg_rate_scale`）でスケールする。
/// - Volume/TlCarrier: Gain FGの5段トレモロループ（旧CHIP LFO AM経路の`chip_lfo_am_to_gain_fg`と
///   同型）。`rate`をそのまま周期のHzへ変換する（Gain FG専用のrate_scale APIが無いため）。
/// - Cutoff: Cutoff FGの4段オートワウループ（Pitchと同型、半周期を`rate`から直接計算）。
#[tauri::command]
fn op505_set_performance_lfo(
    engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>,
    original_fgs: tauri::State<'_, Mutex<OriginalFgs>>,
    channel: usize,
    rate: u8,
    delay: u8,
    destination: u8,
    cc77: u8,
    cc1: u8,
    mod_depth_range: u8,
) {
    let dest = PerformanceLfoDestination::from_u8(destination);
    let orig = *original_fgs.lock().unwrap();
    let mut engine = engine.lock().unwrap();
    let mut patch = engine.current_patch();
    let delay_seconds = delay as f32 / 255.0 * 10.0;

    if dest == PerformanceLfoDestination::Pitch {
        let cents = pitch_depth_cents(cc77, cc1, mod_depth_range);
        let depth = ((cents.abs() / 1200.0) * 255.0).round().clamp(0.0, 255.0) as u8;
        patch.channel.pitch_fg = Op505BipolarFg {
            eg: build_bipolar_vibrato_eg(delay_seconds, PITCH_FG_VIBRATO_AR_SECONDS, PITCH_FG_VIBRATO_D1R_SECONDS),
            depth,
        };
        // CC76(Vibrato Rate)相当をrate_scaleへ変換して発音中の該当チャンネルへ即時反映する
        // （AR/D1Rの生値0〜255をmain.jsのC/Bキー(rate)域からCC76の0〜127域へ線形換算）。
        let cc76 = (rate as u32 * 127 / 255) as u8;
        engine.set_pitch_fg_rate_scale(channel, cc76_to_rate_scale(cc76));
    } else {
        patch.channel.pitch_fg = orig.pitch_fg;
        engine.set_pitch_fg_rate_scale(channel, 1.0);
    }

    if dest == PerformanceLfoDestination::Volume || dest == PerformanceLfoDestination::TlCarrier {
        let depth = volume_depth(cc77, cc1);
        let hz = chip_lfo_freq_to_hz(rate);
        patch.channel.gain_fg = build_tremolo_gain_fg(delay_seconds, hz, depth);
        patch.channel.gain_fg_to_master = true;
        patch.channel.gain_fg_to_operators = false;
    } else {
        patch.channel.gain_fg = orig.gain_fg;
        patch.channel.gain_fg_to_master = orig.gain_fg_to_master;
        patch.channel.gain_fg_to_operators = orig.gain_fg_to_operators;
    }

    if dest == PerformanceLfoDestination::Cutoff {
        let depth = cutoff_depth(cc77, cc1).round().clamp(0.0, 255.0) as u8;
        let half_period = 0.5 / chip_lfo_freq_to_hz(rate);
        patch.channel.cutoff_fg =
            Op505BipolarFg { eg: build_bipolar_vibrato_eg(delay_seconds, half_period, half_period), depth };
    } else {
        patch.channel.cutoff_fg = orig.cutoff_fg;
    }

    engine.set_patch_live(patch);
}

/// マスターエフェクト（Reverb/Chorus）を設定する。
/// `reverb_type`/`chorus_type`は0〜7（spec.md マスターエフェクトセクションのenum参照）
#[tauri::command]
fn set_master_effects(
    effects: tauri::State<'_, Arc<Mutex<MasterEffects>>>,
    reverb_send: u8,
    reverb_type: u8,
    reverb_time: u8,
    chorus_send: u8,
    chorus_type: u8,
    chorus_mod_rate: u8,
    chorus_mod_depth: u8,
    chorus_feedback: u8,
    chorus_send_to_reverb: u8,
) {
    let mut fx = effects.lock().unwrap();
    fx.set_reverb_send(reverb_send);
    fx.set_reverb_type(ReverbType::from_u8(reverb_type));
    fx.set_reverb_time(reverb_time);
    fx.set_chorus_send(chorus_send);
    fx.set_chorus_type(ChorusType::from_u8(chorus_type));
    fx.set_chorus_mod_rate(chorus_mod_rate);
    fx.set_chorus_mod_depth(chorus_mod_depth);
    fx.set_chorus_feedback(chorus_feedback);
    fx.set_chorus_send_to_reverb(chorus_send_to_reverb);
}

/// テンポ（BPM）をエンジンへ設定する。TimeEgのテンポ同期（`sync_enabled`）が対象区間の
/// 速度を決めるのに使う。フロントエンドのタップテンポUIから呼ばれる（`main.js`参照）。
#[tauri::command]
fn set_tempo(engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>, bpm: f32) {
    engine.lock().unwrap().set_tempo(bpm);
}

/// OP505のカレントパッチを設定し、発音中のチャンネルへも即座に反映する
/// （音色エディタのノブ操作向け）。
/// `Op505Patch`は専用DTOを介さず直接シリアライズする（op505-coreの型が既にserde対応で、
/// 後方互換の負債も無いため。フィールド名の安定性は`op505_patch_json_keys_are_stable`テストで担保する）。
///
/// `original_fgs`（Vキー演奏系LFOが基準にする「音色プリセット本来のFG」）もここで更新する。
/// エディタでGain FG等を弄っている最中にVキーを操作した場合、直前のエディタ操作結果が
/// 新しい基準になる（トレモロ中に弄った値が新しい「オリジナル」になる、という直感的な挙動）。
#[tauri::command]
fn op505_set_patch(
    engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>,
    original_fgs: tauri::State<'_, Mutex<OriginalFgs>>,
    patch: Op505Patch,
) {
    *original_fgs.lock().unwrap() = OriginalFgs::capture(&patch.channel);
    engine.lock().unwrap().set_patch_live(patch);
}

/// OP505エンジンの現在のカレントパッチを読み取る（読み取り専用、エンジンへは反映しない）。
/// 音色エディタ起動時、main.js側のデモ選択/Bank・Program変換で既に設定済みのパッチを
/// エディタのローカル状態へ同期するために使う。
#[tauri::command]
fn op505_get_current_patch(engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>) -> Op505Patch {
    engine.lock().unwrap().current_patch()
}

/// (bank, program)に対応する`.op505`プリセットをカレントパッチに設定する（次のnote-onから適用）。
/// 解決順位はレジストリ→`Op505PresetBank`（`op505_core::resolve_patch`参照）。
/// 見つからなければエンジンには触れず`None`を返す（`Op505Patch::default()`はtl=0で無音のため、
/// 黙って無音へ切り替えるより「見つからない」を呼び出し側に伝えて現在の音を維持するほうが安全）。
#[tauri::command]
fn op505_set_program(
    engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>,
    original_fgs: tauri::State<'_, Mutex<OriginalFgs>>,
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank_state: tauri::State<'_, Mutex<Op505PresetBank>>,
    bank: u16,
    program: u8,
) -> Option<Op505Patch> {
    let patch = op505_core::resolve_patch(&registry.lock().unwrap(), &bank_state.lock().unwrap(), bank, program)?;
    *original_fgs.lock().unwrap() = OriginalFgs::capture(&patch.channel);
    engine.lock().unwrap().set_patch(patch);
    Some(patch)
}

/// `op505_presets_dir()`から`.op505`プリセットを読み直し、読み込んだプリセット総数を返す。
/// アプリ起動中に外部（op505_probe/opz2op505等の変換ツールや手編集）でプリセットファイルが
/// 追加・更新された場合に、再起動せず反映するためのコマンド。`Op505PresetBank`（フォールバック用）
/// と`Op505BankRegistry`（Open/Save/Save Asの担当ファイル管理）の両方を作り直す
/// （セッション中のOpen/Save/Save Asによる担当ファイルの変更はこの操作で失われる）。
#[tauri::command]
fn op505_reload_presets(bank_state: tauri::State<'_, Mutex<Op505PresetBank>>, registry: tauri::State<'_, Mutex<Op505BankRegistry>>) -> usize {
    let dir = op505_presets_dir();
    let reloaded = Op505PresetBank::load_from_dir(&dir);
    let count = reloaded.sorted_entries().len();
    *bank_state.lock().unwrap() = reloaded;
    *registry.lock().unwrap() = build_op505_registry(&dir);
    count
}

fn main() {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no output device available");
    let supported = device
        .default_output_config()
        .expect("no default output config");

    let num_channels = supported.channels() as usize;
    let sample_rate = supported.sample_rate().0 as f32;
    let stream_config: cpal::StreamConfig = supported.into();

    let engine = Arc::new(Mutex::new(Op505Engine::new(sample_rate)));
    let engine_audio = Arc::clone(&engine);
    let effects = Arc::new(Mutex::new(MasterEffects::new(sample_rate)));
    let effects_audio = Arc::clone(&effects);

    // presets_dir()の読み込みは起動時にここで1回だけ行う（%APPDATA%\op505\presets）。
    // - op505_bank: フォールバック用の読み取り専用集合（起動時1回）。
    // - op505_registry: bank番号ごとの担当ファイル。Open/Save/Save Asがセッション中に直接更新する。
    let op505_bank = Op505PresetBank::load_from_dir(&op505_presets_dir());
    let op505_registry = build_op505_registry(&op505_presets_dir());

    let stream = device
        .build_output_stream::<f32, _, _>(
            &stream_config,
            move |output: &mut [f32], _| {
                output.fill(0.0);
                if let Ok(mut eng) = engine_audio.try_lock() {
                    eng.render(output, num_channels);
                }
                if let Ok(mut fx) = effects_audio.try_lock() {
                    fx.process(output, num_channels);
                }
            },
            |err| eprintln!("audio error: {err}"),
            None,
        )
        .expect("failed to build output stream");

    stream.play().expect("failed to start audio stream");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(engine)
        .manage(effects)
        .manage(Mutex::new(op505_bank))
        .manage(Mutex::new(op505_registry))
        .manage(Mutex::new(OriginalFgs::default()))
        .invoke_handler(tauri::generate_handler![
            note_on,
            note_off,
            set_master_effects,
            op505_set_performance_lfo,
            set_tempo,
            op505_set_patch,
            op505_get_current_patch,
            op505_set_program,
            op505_reload_presets,
            op505_presets::op505_list_bank_entries,
            op505_presets::op505_get_bank_program,
            op505_presets::op505_open_patch_file,
            op505_presets::op505_save_patch_overwrite,
            op505_presets::op505_save_patch_as,
            op505_presets::op505_add_preset,
            op505_presets::op505_delete_preset,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    drop(stream);
}
