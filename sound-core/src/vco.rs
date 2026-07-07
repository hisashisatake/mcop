// ---------------------------------------------------------------------------
// VCO 抽象境界 と 後段プロセッサー境界
//
// フェーズ7（MC-505風モジュレーション拡張）の土台。発振源（VCO）を差し替え
// 可能な抽象として切り出す。spec.md「設計方針：VCO抽象とモジュレーション層」参照。
//
//   Vco             ← 「ピッチ付き発振源」のインターフェース（発振エンジン全体）
//   AudioProcessor  ← Vcoのrender出力を後段で加工するDSPの共通境界
//
// どちらも発振原理に依存しないプリミティブ型のみで構成し、sound-core（38x6非依存）
// に純粋な共通インターフェースとして置く。
// ---------------------------------------------------------------------------

/// 発振エンジン（音源）の共通インターフェース。
///
/// `ym38x6-core`の`Ym38x6Engine`はこの1実装であり、将来別の発振原理を持つエンジン
/// （PCM・減算合成・物理モデル等）を同じトレイトで差し替え/追加できるようにするための境界。
///
/// 発振原理に依存しない「演奏ライフサイクル」層のみを含む。音色パッチ設定など実装固有の
/// 構造は各エンジンの具象APIに残す（このトレイトはプリミティブ型のみで構成される）。
/// 発音時の音色は実装側が事前に保持したものを使う（38x6なら`set_patch`で設定したパッチ）。
///
/// `Send`はオーディオスレッドへ移送するため必須。object-safe（`Box<dyn Vco>`可能）。
pub trait Vco: Send {
    /// 指定チャンネルIDへ現在の音色で発音する（同IDが発音中なら残響から再アタック）。
    fn note_on(&mut self, channel: usize, frequency: f32, velocity: u8);

    /// 指定チャンネルをキーオフ（Releaseへ移行）する。
    fn note_off(&mut self, channel: usize);

    /// `output`にnum_channels（ステレオ等）でインターリーブ出力する（既存信号へ加算）。
    fn render(&mut self, output: &mut [f32], num_channels: usize);

    /// 発音中チャンネルのピッチベンド量（セント）を設定する。
    fn set_pitch_bend(&mut self, channel: usize, cents: f32);

    /// `channel >> 7`が一致する全チャンネルへピッチベンドを一括適用する（MIDIチャンネル単位）。
    fn set_pitch_bend_group(&mut self, group: usize, cents: f32);

    /// 発音中チャンネルの音量ゲイン（0.0〜1.0）を設定する。
    fn set_channel_volume(&mut self, channel: usize, gain: f32);

    /// `channel >> 7`が一致する全チャンネルの音量ゲインを一括設定する。
    fn set_channel_volume_group(&mut self, group: usize, gain: f32);
}

/// 音源（`Vco`）の`render`出力バッファを後段で加工するDSPの共通インターフェース。
///
/// マスター段のフィルター/アンプ/エフェクトをこのトレイトで数珠繋ぎできる。
/// `MasterEffects`（Reverb/Chorus）が最初の実装。
///
/// 注：フェーズ7のMC-505様式VCF/VCAは**ボイス内**（キーオン連動EG・サンプル単位）であり、
/// この全ボイス合算後のマスター段バッファ加工とは別レイヤーである。
///
/// `Send`はオーディオスレッド移送のため必須。object-safe（`Box<dyn AudioProcessor>`可能）。
pub trait AudioProcessor: Send {
    /// インターリーブ済みf32バッファをin-placeで加工する。
    fn process(&mut self, buffer: &mut [f32], num_channels: usize);
}
