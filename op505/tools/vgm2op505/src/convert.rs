//! OP505直接変換のパッチ変換ロジック。CLI（main.rs）とゴールデンテスト
//! （tests/golden.rs）の両方から参照する。
//!
//! デフォーク（ym38x6依存排除）により、`opm2op505::parse::OpmVoice`/
//! `mucom2op505::map::OpnVoice`を直接使うようになった。旧実装が持っていた
//! `to_local_opm_voice`/`to_local_opn_voice`（`vgm2x6::patch::PatchConverter`が要求する
//! `opm2x6`/`mucom2x6`型固定シグネチャへの橋渡し関数）は不要になった。
//!
//! **C2-2でジェネリクスを畳み込み済み**: vgm2x6由来の`PatchConverter`トレイト実装
//! （`from_opm`/`from_opn`/`from_fixed`）だったものを、Op505専用の素直な固有メソッドへ
//! 置き換えた（vgm2op505はOp505Converter以外の変換器を使わないため、トレイト経由の
//! 抽象化に意味がなくなっていた。SSG人工パッチはすでに`Op505Patch`として構築されているため
//! 旧`from_fixed`（恒等写像）自体が丸ごと不要になった）。

use op505_core::Op505Patch;

/// アタック立ち上がりの表現方法（opm2op505/mucom2op505の同形の別型を1本のCLI enumから写像する）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackMode {
    Bias,
    None,
    Curve,
}

impl AttackMode {
    pub fn to_opm(self) -> opm2op505::conv::AttackMode {
        match self {
            AttackMode::Bias => opm2op505::conv::AttackMode::Bias,
            AttackMode::None => opm2op505::conv::AttackMode::None,
            AttackMode::Curve => opm2op505::conv::AttackMode::Curve,
        }
    }
    pub fn to_mucom(self) -> mucom2op505::conv::AttackMode {
        match self {
            AttackMode::Bias => mucom2op505::conv::AttackMode::Bias,
            AttackMode::None => mucom2op505::conv::AttackMode::None,
            AttackMode::Curve => mucom2op505::conv::AttackMode::Curve,
        }
    }
}

pub fn parse_attack_mode(v: &str) -> Result<AttackMode, String> {
    match v.to_ascii_lowercase().as_str() {
        "bias" => Ok(AttackMode::Bias),
        "none" => Ok(AttackMode::None),
        "curve" => Ok(AttackMode::Curve),
        _ => Err(format!("--attack の値が不正(bias/none/curve): {v}")),
    }
}

/// OPM/OPNボイスをOP505パッチへ直接変換する。SSG合成パッチ（[crate::ssg]）は
/// 呼び出し側（[crate::play::eval_ssg_channel]）がネイティブに構築済みのため、
/// この変換器を経由しない。
#[derive(Clone, Copy)]
pub struct Op505Converter {
    pub attack: AttackMode,
}

impl Op505Converter {
    pub fn from_opm(&self, v: &opm2op505::parse::OpmVoice) -> (Op505Patch, Vec<String>) {
        opm2op505::conv::voice_to_op505_patch(v, opm2op505::parse::OperatorOrder::Direct, self.attack.to_opm())
    }
    pub fn from_opn(&self, v: &mucom2op505::map::OpnVoice) -> (Op505Patch, Vec<String>) {
        mucom2op505::conv::voice_to_op505_patch(v, self.attack.to_mucom())
    }
}
