//! YM2203/YM2608 内蔵 SSG(PSG, AY-3-8910互換) のレジスタ状態と PSG 変換。
//!
//! SSG は OPN の port0 レジスタ `0x00-0x0F` に存在する。トーン3ch（A/B/C）を
//! 矩形波音色（[psg_patch]）で鳴らし、音量レジスタ(0x08-0x0A)のソフトエンベロープは
//! SMF では CC11、WAV ではキャリアTLへ反映する。
//!
//! 由来: ym38x6/tools/vgm2x6/src/ssg.rs（コミット535604f時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! vgm2x6側の修正は自動では反映されない（`git diff 535604f -- ym38x6/tools/vgm2x6/src/ssg.rs`
//! で追従漏れを確認できる）。
//!
//! `EnvGen`/`SsgState`/`period_to_freq`/`volume_to_cc11`はym38x6型に一切触れないため無変更。
//! `psg_patch`/`noise_patch`/`mix_patch`のみ、旧実装が`ym38x6_core::Ym38x6Patch`を組み立てて
//! `op505_core::adapter::convert_patch`（.38x6→.op505の2段変換）に通していたのを、
//! `Op505Patch`を直接構築するネイティブ実装へ置き換えた。焼き込んだ数値は
//! `tests/golden/ssg_{psg,noise_np00,mix_np00}.json`（Phase 0でデフォーク前に採取した
//! 2段変換の結果）と1ビットも変えていない（`patch`部分はUPDATE_GOLDEN更新時に無変化
//! 確認済み、`warnings`部分のみ空になるのが意図的な差分。spec-fm.md「デフォーク」節参照）。
//! Pitch/Cutoff/Gain FG・texture_lfoの値は`Ym38x6Patch::default()`由来で
//! `Op505ChannelParams::default()`とはビット表現が異なる（音は同一）。整理は別コミットで行う。

use op505_core::{Op505BipolarFg, Op505ChannelParams, Op505GainFg, Op505OperatorParams, Op505Patch};
use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};

// SSGクロックの分周値はチップ依存（`ssg_clock = chip_clock / ssg_clock_div`）。
// YM2203(OPN)=master/2、YM2608(OPNA)=master/4。6ch系は3ch系比でプリスケーラが2倍。
// 実際の値は OpnInfo::ssg_clock_div（[crate::play::detect_chip]）が保持する。
// トーン周波数 `f = ssg_clock / (16 * period)` の絶対音程に効く。

// ---------------------------------------------------------------------------
// AY-3-8910互換 ハードウェアエンベロープ発生器
// ---------------------------------------------------------------------------

/// AY-3-8910互換ハードウェアエンベロープ発生器。
///
/// 音量レジスタ(0x08-0x0A)のbit4=1(envelope_mode)のチャンネルは、
/// 直接音量ではなくここが出力する0〜15のレベルで発音する。
///
/// 0x0D書き込みで再スタートする（書き込まれた時点でのshapeとperiodが有効）。
///
/// # 10種類のエンベロープ形状（0x0Dの下位4bit）
/// bits[3:2:1:0] = CONT/ATT/ALT/HOLD
/// - 0x00-0x07 (CONT=0): ワンショット後にレベル0で停止
/// - 0x08: \\\\  鋸歯下降繰り返し (CONT=1 ATT=0 ALT=0 HOLD=0)
/// - 0x09: \\_  鋸歯下降後レベル0で停止 (CONT=1 ATT=0 ALT=0 HOLD=1)
/// - 0x0A: \\/\\ 三角波(下降→上昇繰り返し) (CONT=1 ATT=0 ALT=1 HOLD=0)
/// - 0x0B: \\‾  下降後レベル15で停止 (CONT=1 ATT=0 ALT=1 HOLD=1)
/// - 0x0C: ////  鋸歯上昇繰り返し (CONT=1 ATT=1 ALT=0 HOLD=0)
/// - 0x0D: /‾   上昇後レベル15で停止 (CONT=1 ATT=1 ALT=0 HOLD=1)
/// - 0x0E: /\\/  三角波(上昇→下降繰り返し) (CONT=1 ATT=1 ALT=1 HOLD=0)
/// - 0x0F: /_   上昇後レベル0で停止 (CONT=1 ATT=1 ALT=1 HOLD=1)
pub struct EnvGen {
    /// エンベロープ周期（0x0C*256 + 0x0B）。0は65536として扱う。
    period: u32,
    /// エンベロープ形状（0x0Dの下位4bit）。
    shape: u8,
    /// prescalerカウンタ蓄積値（単位: ssg_clock ticks）。
    accum: f32,
    /// 現在の出力レベル（0〜15）。
    level: u8,
    /// 現在の進行方向（true=上昇 0→15、false=下降 15→0）。
    ascending: bool,
    /// エンベロープ停止中（CONT=0の1サイクル完了後、またはHOLD=1の第1セグメント完了後）。
    holding: bool,
    /// 第2セグメント以降かどうか（HOLD判定に使用）。
    second_seg: bool,
}

impl EnvGen {
    pub fn new() -> Self {
        Self { period: 0, shape: 0, accum: 0.0, level: 0, ascending: false, holding: true, second_seg: false }
    }

    /// 0x0D書き込み時にエンベロープを再スタートする。
    pub fn trigger(&mut self, shape: u8) {
        self.shape = shape & 0x0F;
        self.accum = 0.0;
        self.second_seg = false;
        self.holding = false;
        let att = (self.shape >> 2) & 1 != 0;
        self.ascending = att;
        self.level = if att { 0 } else { 15 };
    }

    /// エンベロープ周期を更新する（0x0Bまたは0x0C書き込み時に SsgState から呼ぶ）。
    pub fn set_period_lo(&mut self, val: u8) {
        self.period = (self.period & 0xFF00) | val as u32;
    }

    pub fn set_period_hi(&mut self, val: u8) {
        self.period = (self.period & 0x00FF) | ((val as u32) << 8);
    }

    /// 現在の出力レベル（0〜15）を返す。
    pub fn current_level(&self) -> u8 {
        self.level
    }

    /// `samples` サンプル分だけエンベロープを進める。
    /// prescalerは ssg_clock/16 Hz で動く。
    pub fn tick(&mut self, samples: u32, ssg_clock: u32, sample_rate: u32) {
        if self.holding { return; }
        let p = if self.period == 0 { 65536u32 } else { self.period };
        // prescalerは ssg_clock/16 Hz。samplesサンプルで進む prescaler ticks 数。
        self.accum += samples as f32 * (ssg_clock as f32 / (16.0 * sample_rate as f32));
        let steps = (self.accum / p as f32) as u32;
        if steps > 0 {
            self.accum -= steps as f32 * p as f32;
            self.advance_steps(steps);
        }
    }

    /// `steps` エンベロープステップを適用する（1ステップ=1レベル変化）。
    fn advance_steps(&mut self, steps: u32) {
        let cont = (self.shape >> 3) & 1 != 0;
        let alt  = (self.shape >> 1) & 1 != 0;
        let hold = self.shape & 1 != 0;
        let mut rem = steps;

        while rem > 0 && !self.holding {
            // 現セグメントで残れるステップ数
            let steps_in_seg = if self.ascending {
                15u32.saturating_sub(self.level as u32)
            } else {
                self.level as u32
            };

            if rem <= steps_in_seg {
                if self.ascending { self.level += rem as u8; }
                else { self.level -= rem as u8; }
                rem = 0;
            } else {
                // セグメント境界に達した
                rem -= steps_in_seg + 1; // +1 for the boundary step
                self.level = if self.ascending { 15 } else { 0 };

                if !cont {
                    // CONT=0: ワンショット後レベル0で停止
                    self.holding = true;
                    self.level = 0;
                } else if hold && !self.second_seg {
                    // HOLD=1かつ第1セグメント完了: 現レベルで停止
                    self.holding = true;
                } else {
                    // 次セグメントへ
                    self.second_seg = true;
                    if alt {
                        self.ascending = !self.ascending;
                        self.level = if self.ascending { 0 } else { 15 };
                    } else {
                        // ALT=0: 同方向で巻き戻し（鋸歯）
                        self.level = if self.ascending { 0 } else { 15 };
                    }
                }
            }
        }
    }
}

/// SSG レジスタ状態（0x00-0x0F）。
pub struct SsgState {
    pub regs: [u8; 16],
    /// AY-3-8910互換ハードウェアエンベロープ発生器（全チャンネル共有、1基）。
    pub env: EnvGen,
}

impl SsgState {
    pub fn new() -> Self {
        Self { regs: [0u8; 16], env: EnvGen::new() }
    }

    pub fn write(&mut self, reg: u8, val: u8) {
        let r = reg as usize;
        if r < 16 {
            self.regs[r] = val;
        }
        // エンベロープ関連レジスタの書き込みを EnvGen へ連携する。
        // 0x0B/0x0C: 周期更新（エンベロープは再スタートしない）。
        // 0x0D: 形状書き込みで即座にエンベロープを再スタートする（AY-3-8910仕様）。
        match reg {
            0x0B => self.env.set_period_lo(val),
            0x0C => self.env.set_period_hi(val),
            0x0D => self.env.trigger(val),
            _ => {}
        }
    }

    /// トーンch(0=A/1=B/2=C)の周期（12bit）。
    pub fn tone_period(&self, ch: usize) -> u16 {
        let lo = self.regs[ch * 2] as u16;
        let hi = self.regs[ch * 2 + 1] as u16;
        ((hi & 0x0F) << 8) | lo
    }

    /// トーンch がミキサー(0x07)で有効か（bit 0-2 が0=有効）。
    pub fn tone_enabled(&self, ch: usize) -> bool {
        (self.regs[0x07] >> ch) & 1 == 0
    }

    /// ノイズch がミキサー(0x07)で有効か（bit 3-5 が0=有効）。
    pub fn noise_enabled(&self, ch: usize) -> bool {
        (self.regs[0x07] >> (ch + 3)) & 1 == 0
    }

    /// ノイズ周期（5bit、reg 0x06 bits[4:0]）。0のとき実質1と同等。
    pub fn noise_period(&self) -> u8 {
        self.regs[0x06] & 0x1F
    }

    /// 音量(0-15)。レジスタ 0x08+ch の bits0-3。
    pub fn volume(&self, ch: usize) -> u8 {
        self.regs[0x08 + ch] & 0x0F
    }

    /// ハードウェアエンベロープモード（0x08+ch の bit4）。
    pub fn envelope_mode(&self, ch: usize) -> bool {
        (self.regs[0x08 + ch] >> 4) & 1 != 0
    }

    /// 発音上の実効音量(0-15)。エンベロープモード時はハードウェアエンベロープ発生器の
    /// 現在レベルを返す（全チャンネル共有の1基が出力する値）。
    pub fn effective_volume(&self, ch: usize) -> u8 {
        if self.envelope_mode(ch) {
            self.env.current_level()
        } else {
            self.volume(ch)
        }
    }

    /// `samples` サンプル分エンベロープを進める（Waitイベント時に呼ぶ）。
    pub fn tick_envelope(&mut self, samples: u32, ssg_clock: u32, sample_rate: u32) {
        self.env.tick(samples, ssg_clock, sample_rate);
    }

}

/// SSG周期 → 周波数(Hz)。`f = ssg_clock / (16 * period)`。
pub fn period_to_freq(period: u16, ssg_clock: u32) -> f32 {
    if period == 0 {
        return 0.0;
    }
    ssg_clock as f32 / (16.0 * period as f32)
}

/// 実効音量(0-15) → CC11(エクスプレッション, 0-127) 線形マップ。
pub fn volume_to_cc11(vol: u8) -> u8 {
    (vol.min(15) as u16 * 127 / 15) as u8
}

// ---------------------------------------------------------------------------
// SSG合成用のOP505ネイティブ固定パッチ（トーン/ノイズ/混合）
// ---------------------------------------------------------------------------

/// stage0(attack, time=1, level=255) → stage1(release, level=0) の2段EG。
/// 旧2段変換（ar=255/d1r=0/d1l=255/d2r=0のOperatorParamsをconvert_eg_shapeへ通した結果）と
/// 1ビット一致するよう、`rr_time`（release段のtime値）だけを差し替えて共有する。
fn ssg_op_eg(rr_time: u8) -> TimeEgParams {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    stages[0] = TimeStage { time: 1, level: 255, curve: 0 };
    stages[1] = TimeStage { time: rr_time, level: 0, curve: 0 };
    TimeEgParams { stages, stage_count: 2, loop_enabled: 0, loop_start: 0, release_point: 0 , ..Default::default()}
}

/// rr=255（実機最速、旧EGレート換算で≒8.71ms）に対応するtime値
/// （`seconds_to_time`をT_MAX=300秒の現行テーブルで手計算した値。T_MAX変更時は要再計算）。
const RR_FAST_TIME: u8 = 45;
/// rr=0（旧EGのフリーズしない特殊値、284.9秒）に対応するtime値。
/// T_MAX=300秒はレート方式EGの理論最遅値284.9秒を余裕を持ってカバーするため、
/// クランプは発生しない（旧T_MAX=30秒時代はここでtime=255にクランプしていた名残の定数名）。
const RR_ZERO_TIME: u8 = 254;

/// 矩形波トーンオペレーター（[psg_patch]用、rr=255相当の最速リリース）。
fn tone_op_fast_release(tl: u8) -> Op505OperatorParams {
    Op505OperatorParams {
        tl,
        eg: ssg_op_eg(RR_FAST_TIME),
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 16, // 矩形波（square variant0）
        op_fine_tune: 128,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    }
}

/// 矩形波トーンオペレーター（[mix_patch]用、rr=0相当＝284.9秒をtime=255にクランプ）。
fn tone_op_transparent_release(tl: u8) -> Op505OperatorParams {
    Op505OperatorParams { eg: ssg_op_eg(RR_ZERO_TIME), ..tone_op_fast_release(tl) }
}

/// ノイズ専用オペレーター（波形番号32+NP、rr=255相当の最速リリース）。
/// エンジンが17bit LFSRをNP由来クロックレートでサンプル&ホールドしてノイズを生成する。
fn noise_op(tl: u8, np: u8) -> Op505OperatorParams {
    Op505OperatorParams { waveform: 32 + np.min(31), ..tone_op_fast_release(tl) }
}

/// チャンネル共通のFG既定値。`Ym38x6Patch::default()`由来のビット表現をそのまま焼き込む
/// （`Op505ChannelParams::default()`とは値が異なるが音は同一。モジュールdocコメント参照）。
fn ssg_channel(algorithm: u8) -> Op505ChannelParams {
    // Pitch/Cutoff FGは変調を一切かけない（SSG人工パッチには不要）。バイポーラレベル方式では
    // 「無変調」は Depth=0 かつ全段レベル=中央128 であり、**旧表現の levels=0 は
    // 「全開マイナス」を意味してしまう**（旧方式はDepth=128が中立だったため levels=0 が無害だった）。
    // `stage1_time`は旧ビット表現の名残で、無変調なので音には影響しない。
    let pitch_cutoff_eg = |stage1_time: u8| {
        let neutral = TimeStage { time: 0, level: sound_core::BIPOLAR_NEUTRAL_RAW, curve: 0 };
        let mut stages = [neutral; MAX_STAGES];
        stages[1] = TimeStage { time: stage1_time, ..neutral };
        TimeEgParams { stages, stage_count: 2, loop_enabled: 0, loop_start: 0, release_point: 0 , ..Default::default()}
    };
    let gain_fg = {
        let mut stages = [TimeStage::default(); MAX_STAGES];
        stages[0] = TimeStage { time: 1, level: 255, curve: 0 };
        stages[1] = TimeStage { time: 0, level: 255, curve: 0 };
        TimeEgParams { stages, stage_count: 2, loop_enabled: 0, loop_start: 0, release_point: 0 , ..Default::default()}
    };
    Op505ChannelParams {
        algorithm,
        pitch_fg: Op505BipolarFg { eg: pitch_cutoff_eg(54), depth: 0 },
        cutoff_fg: Op505BipolarFg { eg: pitch_cutoff_eg(255), depth: 0 },
        gain_fg: Op505GainFg { eg: gain_fg, depth: 255 },
        ..Op505ChannelParams::default()
    }
}

/// PSG風パッチ。波形メモリ音色の矩形波(waveform=16)を、アタック最速・減衰なし・
/// 最大サスティン・最速リリースのEGで鳴らす。キーオン中は一定音量の矩形波、
/// キーオフで即停止という、PSG(SSG/SN76489系)のトーン1チャンネルに近い挙動になる。
pub fn psg_patch() -> Op505Patch {
    Op505Patch {
        operators: [
            tone_op_fast_release(255),
            tone_op_fast_release(0),
            tone_op_fast_release(0),
            tone_op_fast_release(0),
        ],
        channel: ssg_channel(7),
    }
}

/// SSGノイズのみ用パッチ。ノイズOPを OP1 で鳴らす（Algorithm 7・OP2〜4ミュート）。
/// ノイズはピッチレスなので note 周波数には依存せず、NPで帯域（白〜低域）が決まる。
pub fn noise_patch(np: u8) -> Op505Patch {
    Op505Patch {
        operators: [noise_op(255, np), noise_op(0, np), noise_op(0, np), noise_op(0, np)],
        channel: ssg_channel(7),
    }
}

/// トーン+ノイズ混合パッチ（実機SSGのトーン・ノイズ同時出力）。
/// Algorithm 7 で OP1=矩形波（トーン、note周波数で鳴る）+ OP3=ノイズ（NPの帯域）の加算混合。
/// 実機の「トーンANDノイズ（ゲート）」ではなく加算近似だが、両方が同時に聞こえる。
pub fn mix_patch(np: u8) -> Op505Patch {
    Op505Patch {
        operators: [
            tone_op_transparent_release(255),
            tone_op_transparent_release(0),
            noise_op(255, np),
            tone_op_transparent_release(0),
        ],
        channel: ssg_channel(7),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_patch_combines_tone_and_noise() {
        // Algorithm 7、OP1=矩形(16)=トーン、OP3=ノイズ(32+NP)、OP2/4=ミュート。
        let p = mix_patch(5);
        assert_eq!(p.channel.algorithm, 7);
        assert_eq!(p.operators[0].waveform, 16);
        assert_eq!(p.operators[0].tl, 255);
        assert_eq!(p.operators[1].tl, 0);
        assert_eq!(p.operators[2].waveform, 37); // 32+5
        assert_eq!(p.operators[2].tl, 255);
        assert_eq!(p.operators[3].tl, 0);
    }

    #[test]
    fn noise_patch_maps_np_directly_to_waveform() {
        // OP1の波形番号 = 32 + NP（実機NP直対応、32〜63）。
        let p0 = noise_patch(0);
        let p31 = noise_patch(31);
        assert_eq!(p0.operators[0].waveform, 32);
        assert_eq!(p31.operators[0].waveform, 63);
        assert_eq!(p0.channel.algorithm, 7);
        // OP2〜4はミュート(TL=0)
        assert_eq!(p0.operators[1].tl, 0);
    }

    #[test]
    fn noise_patch_np_only_affects_waveform() {
        for np in 0u8..=31 {
            let base = noise_patch(0);
            let varied = noise_patch(np);
            for i in 0..4 {
                let mut b = base.operators[i];
                let mut v = varied.operators[i];
                b.waveform = 0;
                v.waveform = 0;
                assert_eq!(b, v, "noise_patch np={np} op{i} differs beyond waveform");
            }
            assert_eq!(varied.operators[0].waveform, 32 + np.min(31));

            let base_mix = mix_patch(0);
            let varied_mix = mix_patch(np);
            for i in 0..4 {
                let mut b = base_mix.operators[i];
                let mut v = varied_mix.operators[i];
                b.waveform = 0;
                v.waveform = 0;
                assert_eq!(b, v, "mix_patch np={np} op{i} differs beyond waveform");
            }
            assert_eq!(varied_mix.operators[2].waveform, 32 + np.min(31));
        }
    }

    #[test]
    fn tone_period_combines_lo_hi() {
        let mut s = SsgState::new();
        s.write(0x00, 0x34); // A lo
        s.write(0x01, 0x02); // A hi (4bit)
        assert_eq!(s.tone_period(0), 0x234);
    }

    #[test]
    fn mixer_enable_is_active_low() {
        let mut s = SsgState::new();
        s.write(0x07, 0b111_110); // ch0 tone有効(bit0=0)、ch1/2無効
        assert!(s.tone_enabled(0));
        assert!(!s.tone_enabled(1));
        assert!(!s.tone_enabled(2));
    }

    #[test]
    fn noise_enable_bits_3_to_5() {
        let mut s = SsgState::new();
        // bits0-2=111: tone全無効 / bit3=0: noise ch0有効 / bit4=1, bit5=1: noise ch1,2無効
        s.write(0x07, 0b00_110_111); // = 0x37
        assert!(!s.tone_enabled(0));
        assert!(!s.tone_enabled(1));
        assert!(!s.tone_enabled(2));
        assert!(s.noise_enabled(0));
        assert!(!s.noise_enabled(1));
        assert!(!s.noise_enabled(2));
    }

    #[test]
    fn noise_period_is_5bit() {
        let mut s = SsgState::new();
        s.write(0x06, 0xFF); // 上位3bitは無効、下位5bit=31
        assert_eq!(s.noise_period(), 31);
        s.write(0x06, 0x00);
        assert_eq!(s.noise_period(), 0);
    }

    #[test]
    fn volume_and_envelope_mode() {
        let mut s = SsgState::new();
        s.write(0x08, 0x0A); // vol=10, env off
        assert_eq!(s.volume(0), 10);
        assert!(!s.envelope_mode(0));
        assert_eq!(s.effective_volume(0), 10);
        s.write(0x09, 0x1F); // bit4=env mode, ch1
        assert!(s.envelope_mode(1));
        // エンベロープはトリガー(0x0D書き込み)前は初期状態（保持中、レベル0）。
        // effective_volume は env.current_level() を返す。
        assert_eq!(s.effective_volume(1), s.env.current_level());
        // 0x0D 書き込みでトリガー: ATT=1(shape=0x0C)なら 0→15 上昇、初期level=0。
        s.write(0x0D, 0x0C); // shape 0x0C: 上昇鋸歯繰り返し(ATT=1)
        assert_eq!(s.effective_volume(1), 0); // triggered → level=0(上昇開始)
        // 0x0D=0x08(ATT=0 下降): trigger後 level=15
        s.write(0x0D, 0x08);
        assert_eq!(s.effective_volume(1), 15); // triggered → level=15(下降開始)
    }

    #[test]
    fn period_freq_octave_relation() {
        let f = period_to_freq(284, 2_000_000); // ≈440Hz @ ssg_clock 2MHz
        assert!((f - 440.0).abs() < 5.0);
        // 周期半分で1オクターブ上
        let f2 = period_to_freq(142, 2_000_000);
        assert!((f2 / f - 2.0).abs() < 0.05);
    }

    #[test]
    fn volume_maps() {
        assert_eq!(volume_to_cc11(15), 127);
        assert_eq!(volume_to_cc11(0), 0);
    }
}
