//! マスターリバーブの後段適用。
//!
//! DAW（REAPER 等）で `.38x6` を鳴らすときと同じく、ドライ出力に
//! `MasterEffects` のリバーブセンドを後段で掛けて聴感を再現するためのヘルパー。
//! `send`=0 のときは何もしない（＝従来どおりの完全ドライ）。

use ym38x6_core::{MasterEffects, ReverbType};

/// マスターリバーブ設定。
pub struct ReverbConfig {
    /// リバーブセンド（0=無効＝ドライ, 255=最大）。CC91 相当。
    pub send: u8,
    /// リバーブタイプ（0〜7、`ReverbType::from_u8`）。
    pub reverb_type: u8,
    /// リバーブタイム（0〜255）。
    pub time: u8,
}

impl Default for ReverbConfig {
    fn default() -> Self {
        // エンジン既定（タイプ=Hall1, time=128）に合わせる。send=0=ドライ。
        Self { send: 0, reverb_type: ReverbType::default() as u8, time: 128 }
    }
}

/// インターリーブ済みバッファにマスターリバーブを後段適用する。
/// `cfg.send`=0 のときは何もしない（ドライのまま）。
///
/// `MasterEffects` は状態（残響テール）を持つため、レンダリング済みの
/// 全バッファを先頭から1パスで処理する（チャンク分割処理と等価）。
pub fn apply_reverb(buf: &mut [f32], num_channels: usize, sample_rate: f32, cfg: &ReverbConfig) {
    if cfg.send == 0 {
        return;
    }
    let mut fx = MasterEffects::new(sample_rate);
    fx.set_reverb_type(ReverbType::from_u8(cfg.reverb_type));
    fx.set_reverb_time(cfg.time);
    fx.set_reverb_send(cfg.send);
    fx.process(buf, num_channels);
}
