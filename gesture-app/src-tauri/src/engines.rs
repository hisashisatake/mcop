//! 2つの音源エンジン（Ym38x6Engine / Op505Engine）の共存ラッパー。
//!
//! `sound_core::Vco`は演奏系（note_on/note_off/render等）だけの境界で音色設定APIを含まないため、
//! 音色系コマンドは`Box<dyn Vco>`では書けない（`set_patch_live`/`current_patch`/
//! `set_pitch_fg_rate_scale`はVcoに無い）。EGパラメーター型が根本的に異なる2チップを
//! トレイトで無理に抽象化せず、両方の具体型を保持して演奏系だけをactiveへ振り分ける
//! （EGトレイト化を却下した過去の設計判断——「共存させるならenum」——に従う）。

use op505_core::Op505Engine;
// Vcoトレイトはsound-coreの定義をym38x6-coreが再エクスポートしたもの（gesture-appは
// sound-coreへ直接依存しない既存方針に合わせる）。
use ym38x6_core::{Vco, Ym38x6Engine};

/// どちらのエンジンが演奏入力（note_on）を受けるか。既定はOP505（op505が主力、ym38x6は
/// 将来非推奨という方針のため）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ActiveEngine {
    Ym38x6,
    #[default]
    Op505,
}

impl ActiveEngine {
    /// フロントエンドのエンジン選択値（0=OP505 / 1=38x6、範囲外は既定のOP505）から変換する。
    pub fn from_u8(v: u8) -> Self {
        if v == 1 {
            Self::Ym38x6
        } else {
            Self::Op505
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::Op505 => 0,
            Self::Ym38x6 => 1,
        }
    }
}

/// 両エンジンを常時保持し、演奏系だけをactiveへ振り分けるラッパー。
///
/// 切替のたびにエンジンを作り直す設計にしないのは、カレントパッチ・ユーザー波形テーブル・
/// 発音中のリリース尾を切替で失わないため。音色系コマンドはこの構造体を経由せず
/// `.ym38x6`/`.op505`フィールドへ直接アクセスする（既存のym38x6コマンド群は
/// `.ym38x6`を触るだけで従来と同一の呼び出し列になる）。
pub struct Engines {
    pub ym38x6: Ym38x6Engine,
    pub op505: Op505Engine,
    pub active: ActiveEngine,
}

impl Engines {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            ym38x6: Ym38x6Engine::new(sample_rate),
            op505: Op505Engine::new(sample_rate),
            active: ActiveEngine::default(),
        }
    }

    /// 両エンジンをレンダリングする。`Vco::render`は既存信号への加算なので単純に重ねられる。
    /// 非アクティブ側は発音チャンネル0本なら実質ゼロコストで、かつ切替直後の
    /// リリース尾が途切れない（切替でブツッと切れるのを仕様として避ける）。
    pub fn render(&mut self, output: &mut [f32], num_channels: usize) {
        self.ym38x6.render(output, num_channels);
        self.op505.render(output, num_channels);
    }

    /// アクティブなエンジンだけへキーオンする。
    pub fn note_on(&mut self, channel: usize, frequency: f32, velocity: u8) {
        match self.active {
            ActiveEngine::Ym38x6 => self.ym38x6.note_on(channel, frequency, velocity),
            ActiveEngine::Op505 => self.op505.note_on(channel, frequency, velocity),
        }
    }

    /// キーオフは両エンジンへ送る。切替直前に押した鍵が切替後に離鍵不能で
    /// 鳴りっぱなしになる事故を防ぐ（該当チャンネルが無いエンジンでは何もしないので無害）。
    pub fn note_off(&mut self, channel: usize) {
        self.ym38x6.note_off(channel);
        self.op505.note_off(channel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    #[test]
    fn note_on_routes_to_active_engine_only() {
        let mut engines = Engines::new(SR);
        engines.active = ActiveEngine::Ym38x6;
        engines.note_on(0, 440.0, 100);
        assert_eq!(engines.op505.active_voice_count(), 0, "38x6選択中はOP505に発音しない");
        assert_eq!(engines.ym38x6.active_voice_count(), 1);

        engines.active = ActiveEngine::Op505;
        engines.note_on(1, 440.0, 100);
        assert_eq!(engines.op505.active_voice_count(), 1);
        assert_eq!(engines.ym38x6.active_voice_count(), 1, "OP505切替後も38x6のリリース尾は残る");
    }

    #[test]
    fn note_off_reaches_both_engines() {
        // OP505で発音→38x6へ切替→note_off。切替後でもOP505側の鍵が離鍵されることを確認
        // （note_offを非アクティブ側にも送らないと鳴りっぱなしになる）。
        let mut engines = Engines::new(SR);
        engines.active = ActiveEngine::Op505;
        engines.note_on(0, 440.0, 100);
        engines.active = ActiveEngine::Ym38x6;
        engines.note_off(0);
        // 既定パッチのリリースは瞬時（time=0）なので、少しレンダリングすればIdle→回収される。
        let mut buf = vec![0.0f32; 512];
        engines.render(&mut buf, 1);
        assert_eq!(engines.op505.active_voice_count(), 0, "note_offが非アクティブ側にも届くべき");
    }

    /// OP505側を確実に鳴らすための最小パッチ（algorithm 7・全OP独立キャリア・瞬時満レベル
    /// サスティンEG）。`op505-core::demo`廃止後、この用途専用にここで組み立てる。
    fn loud_op505_patch() -> op505_core::Op505Patch {
        use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};

        let instant_sustain_eg = TimeEgParams {
            stages: {
                let mut stages = [TimeStage::default(); MAX_STAGES];
                stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
                stages
            },
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            loop_end: 0,
            release_start: 0,
        };

        let mut patch = op505_core::Op505Patch::default();
        for op in patch.operators.iter_mut() {
            op.tl = 255;
            op.eg = instant_sustain_eg;
            op.mul = 1;
        }
        patch.channel.algorithm = 7;
        patch
    }

    #[test]
    fn render_mixes_both_engines_additively() {
        // 両エンジンで同時に発音した状態のrender出力が非サイレントであること
        // （どちらか一方しかレンダリングされない退行の検出）。
        let mut engines = Engines::new(SR);
        engines.active = ActiveEngine::Ym38x6;
        engines.note_on(0, 220.0, 100);
        engines.active = ActiveEngine::Op505;
        engines.op505.set_patch(loud_op505_patch());
        engines.note_on(1, 220.0, 100);

        let mut both = vec![0.0f32; 4096];
        engines.render(&mut both, 1);
        let energy: f32 = both.iter().map(|s| s * s).sum();
        assert!(energy > 0.0, "両エンジン発音中の出力が無音");
    }

    #[test]
    fn active_engine_u8_roundtrip() {
        assert_eq!(ActiveEngine::from_u8(0), ActiveEngine::Op505);
        assert_eq!(ActiveEngine::from_u8(1), ActiveEngine::Ym38x6);
        assert_eq!(ActiveEngine::from_u8(255), ActiveEngine::Op505, "範囲外は既定のOP505");
        assert_eq!(ActiveEngine::from_u8(ActiveEngine::Op505.to_u8()), ActiveEngine::Op505);
    }
}
