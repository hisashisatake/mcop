//! OP505直接変換のPatchConverter/WavEngine実装。CLI（main.rs）とゴールデンテスト
//! （tests/golden.rs）の両方から参照する。

use op505_core::{Op505Engine, Op505Patch};
use vgm2x6::patch::PatchConverter;
use vgm2x6::play::WavEngine;
use ym38x6_core::Ym38x6Patch;

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

/// `vgm2x6::patch::PatchConverter::from_opn`は`mucom2x6::conv::OpnVoice`固定のシグネチャを
/// 要求する（vgm2x6が今も同型に依存しているため）。デフォークでmucom2op505が独自の
/// （フィールド構成は同一の）`OpnVoice`型を持つようになったため、フィールドコピーで橋渡しする。
/// **Phase 2でvgm2op505自体をym38x6依存から複製・再構築する際にこの変換は不要になる**
/// （vgm2op505がmucom2op505::map::OpnVoiceを直接使うようになるため）。
fn to_local_opn_voice(v: &mucom2x6::conv::OpnVoice) -> mucom2op505::map::OpnVoice {
    mucom2op505::map::OpnVoice {
        operators: std::array::from_fn(|i| {
            let op = &v.operators[i];
            mucom2op505::map::OpnOperator {
                tl: op.tl,
                ar: op.ar,
                d1r: op.d1r,
                d2r: op.d2r,
                d1l: op.d1l,
                rr: op.rr,
                mul: op.mul,
                dt1: op.dt1,
                ks: op.ks,
                am_enable: op.am_enable,
            }
        }),
        algorithm: v.algorithm,
        feedback: v.feedback,
    }
}

/// [`to_local_opn_voice`]のOPM版。`vgm2x6::patch::PatchConverter::from_opm`が要求する
/// `opm2x6::parse::OpmVoice`固定シグネチャを、opm2op505の独自（フィールド構成は同一の）
/// `OpmVoice`型へフィールドコピーで橋渡しする。Phase 2で不要になる暫定コード。
fn to_local_opm_voice(v: &opm2x6::parse::OpmVoice) -> opm2op505::parse::OpmVoice {
    fn conv_op(op: &opm2x6::parse::OpmOpReg) -> opm2op505::parse::OpmOpReg {
        opm2op505::parse::OpmOpReg {
            ar: op.ar, d1r: op.d1r, d2r: op.d2r, rr: op.rr, d1l: op.d1l, tl: op.tl,
            ks: op.ks, mul: op.mul, dt1: op.dt1, dt2: op.dt2, ams_en: op.ams_en,
        }
    }
    opm2op505::parse::OpmVoice {
        number: v.number,
        name: v.name.clone(),
        lfrq: v.lfrq,
        amd: v.amd,
        pmd: v.pmd,
        lfo_wf: v.lfo_wf,
        fl: v.fl,
        con: v.con,
        ams: v.ams,
        pms: v.pms,
        slot: v.slot,
        m1: conv_op(&v.m1),
        c1: conv_op(&v.c1),
        m2: conv_op(&v.m2),
        c2: conv_op(&v.c2),
    }
}

/// OPM/OPNボイスをOP505パッチへ直接変換する。SSG固定パッチ（x6ネイティブ人工パッチ）のみ
/// `op505_core::adapter::convert_patch`（.38x6→.op505の既存2段変換）を経由する。
pub struct Op505Converter {
    pub attack: AttackMode,
}

impl PatchConverter for Op505Converter {
    type Patch = Op505Patch;

    fn from_opm(&mut self, v: &opm2x6::parse::OpmVoice) -> (Op505Patch, Vec<String>) {
        opm2op505::conv::voice_to_op505_patch(
            &to_local_opm_voice(v),
            opm2op505::parse::OperatorOrder::Direct,
            self.attack.to_opm(),
        )
    }
    fn from_opn(&mut self, v: &mucom2x6::conv::OpnVoice) -> (Op505Patch, Vec<String>) {
        mucom2op505::conv::voice_to_op505_patch(&to_local_opn_voice(v), self.attack.to_mucom())
    }
    fn from_x6(&mut self, p: &Ym38x6Patch) -> (Op505Patch, Vec<String>) {
        op505_core::adapter::convert_patch(p)
    }
}

/// OP505直接レンダリング用の`Op505Engine`ラッパー（orphan rule対策のnewtype）。
pub struct Engine505(pub Op505Engine);

impl WavEngine for Engine505 {
    type Patch = Op505Patch;
    fn new(sr: f32) -> Self {
        Engine505(Op505Engine::new(sr))
    }
    fn set_patch(&mut self, patch: Op505Patch) {
        self.0.set_patch(patch);
    }
    fn note_on(&mut self, ch: usize, freq: f32, vel: u8) {
        sound_core::Vco::note_on(&mut self.0, ch, freq, vel);
    }
    fn note_off(&mut self, ch: usize) {
        sound_core::Vco::note_off(&mut self.0, ch);
    }
    fn set_pitch_bend(&mut self, ch: usize, cents: f32) {
        sound_core::Vco::set_pitch_bend(&mut self.0, ch, cents);
    }
    fn set_channel_volume(&mut self, ch: usize, gain: f32) {
        sound_core::Vco::set_channel_volume(&mut self.0, ch, gain);
    }
    fn render(&mut self, buf: &mut [f32]) {
        sound_core::Vco::render(&mut self.0, buf, 1);
    }
}
