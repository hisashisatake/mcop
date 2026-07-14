//! OPN（YM2608/OPNA）ボイスレジスタ → ym38x6 パッチへの変換ロジック。
//!
//! スケーリング規約は psr2x6/conv.rs（spec-sound.md L768-791 準拠）と共通。
//! OPN固有の差異: DT1（3bit、0-7、signed）→ 38x6 dt1（8bit、中心128）の変換のみ異なる。

use ym38x6_core::{ChannelParams, OperatorParams, PresetEntry, PresetFile, Ym38x6Patch};

// ---------------------------------------------------------------------------
// OPN中間表現（レジスタ値のまま保持、アルゴリズム順: op0=OP1, op1=OP2, op2=OP3, op3=OP4）
// ---------------------------------------------------------------------------

/// OPNオペレーター1個分のレジスタ値（アルゴリズム順のop番号で保持）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpnOperator {
    /// Total Level（7bit, 0〜127。0=最大音量、127=最小音量）。
    pub tl: u8,
    /// Attack Rate（5bit, 0〜31）。
    pub ar: u8,
    /// Decay Rate / D1R（5bit, 0〜31）。
    pub d1r: u8,
    /// Sustain Rate / D2R（5bit, 0〜31）。
    pub d2r: u8,
    /// Sustain Level / D1L（4bit, 0〜15）。
    pub d1l: u8,
    /// Release Rate（4bit, 0〜15）。
    pub rr: u8,
    /// Multiple（4bit, 0〜15）。OPM/OPN/OPQ/OPZ共通。
    pub mul: u8,
    /// Detune1（3bit, 0〜7。OPN符号付きエンコード: 0/4=±0, 1-3=正方向, 5-7=負方向）。
    pub dt1: u8,
    /// Key Scale Rate（2bit, 0〜3）。
    pub ks: u8,
    /// AMS-EN（このオペレーターをAM変調対象にするか）。
    pub am_enable: bool,
}

/// OPN 1ボイス（4オペレーター + チャンネル設定）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpnVoice {
    /// アルゴリズム順: [op0=OP1(FB), op1=OP2, op2=OP3, op3=OP4(carrier for ALG0)]
    pub operators: [OpnOperator; 4],
    /// Algorithm（3bit, 0〜7）。OPN のアルゴリズム番号は ym38x6 ALGORITHMS と同一トポロジー。
    pub algorithm: u8,
    /// Feedback（3bit, 0〜7）。
    pub feedback: u8,
}

/// 名前付きボイス（元のスロット番号を保持）。
#[derive(Clone, Debug, PartialEq)]
pub struct NamedVoice {
    /// MUCOM88 のスロット番号（0〜255）。プログラム番号への対応付けに使う。
    pub slot: u16,
    pub name: String,
    pub voice: OpnVoice,
}

// ---------------------------------------------------------------------------
// スカラー変換（psr2x6/conv.rs と同一）
// ---------------------------------------------------------------------------

/// 5bit（0〜31）→ 8bit（0〜255）: ×8。AR/D1R/D2R に使用。
#[inline]
pub fn scale_5bit(v: u8) -> u8 {
    v.min(31) * 8
}

/// 4bit（0〜15）→ 8bit（0〜255）: ×17。RR に使用。
#[inline]
pub fn scale_4bit(v: u8) -> u8 {
    v.min(15) * 17
}

/// 3bit（0〜7）→ 8bit（0〜255）: ×36。Feedback に使用。
#[inline]
pub fn scale_3bit(v: u8) -> u8 {
    v.min(7) * 36
}

/// OPN KS(0-3、rate scaling)→ 38x6 ksr(0-255、実行時の音域依存レート倍率)。
///
/// エンジンの`ksr_rate_multiplier`は`exponent=ksr/255`の線形カーブ
/// （ksr=255で1オクターブごとに2倍、ksr=0で音域に依らず常に1.0）。
/// 実機の絶対keycode法則(eg_rate=2R+keycode>>(3-KS))はKS=0〜3で2倍刻み
/// (1x,2x,4x,8x)のため、本来はKS=0→exponent 0.125だが、実機比較
/// （MUCOM88実機C2/C6）で「KS=0は音域依存がほぼ無い方が近い」と判定
/// されたため、KS=0のみ理論値でなく聴感を優先してksr=0(フラット)とする。
/// KS=2=128(exponent≈0.5)は他コンバーター(opz2x6)のGrandPianoキャリア実機比較で
/// 検証済み、KS=3=255(exponent=1.0)は理論値。opz2x6::conv::ks_to_ksrと同一テーブル。
#[inline]
pub fn ks_to_ksr(v: u8) -> u8 {
    const TABLE: [u8; 4] = [0, 64, 128, 255];
    TABLE[v.min(3) as usize]
}

/// Total Level: OPN（0=最大音量, 127=最小音量）→ 38x6（0=最小, 254=最大）。
/// 極性反転 + ×2: `(127 - tl) * 2`。
#[inline]
pub fn tl_opn_to_x6(tl: u8) -> u8 {
    (127 - tl.min(127)) * 2
}

/// Sustain Level: OPN（0=減衰なし, 15=ほぼ無音）→ 38x6（0=ほぼ無音, 255=フルレベル）。
/// 極性反転: `(15 - sl) * 17`。
#[inline]
pub fn sl_opn_to_x6(sl: u8) -> u8 {
    (15 - sl.min(15)) * 17
}

/// OPN DT1（3bit, 0〜7）→ 38x6 dt1（8bit, 中心128=±0, ±50セント）。
///
/// OPN DT1 エンコード（実機 YM2608 の仕様準拠）:
/// - 0, 4: ±0セント
/// - 1: +1.08¢, 2: +2.16¢, 3: +3.24¢（正方向）
/// - 5: -1.08¢, 6: -2.16¢, 7: -3.24¢（負方向、bit2=1）
///
/// 38x6 DT1 は中心128で ±50セント（128/50 ≈ 2.56 units/cent）。
#[inline]
pub fn opn_dt1_to_x6(dt1: u8) -> u8 {
    // ルックアップテーブル（38x6 DT1 units、中心128）
    const TABLE: [u8; 8] = [128, 131, 134, 136, 128, 125, 122, 120];
    TABLE[(dt1 & 7) as usize]
}

/// OPN 5bit レート（AR/D1R/D2R, 0〜31）+ KS（0〜3）→ 38x6 rate（0〜255）。
///
/// OPN の eg_rate_eff = 2×rate + ksr_at_a4(ks)。
/// A4（key_code≈19）でのKSR貢献分を base rate に織り込むことで、
/// 38x6 の decay_to_delta / ar_to_delta と OPN の実機タイミングを合わせる。
/// - rate=0 → 0（フリーズ）
/// - eg_rate (2〜62) → 38x6 rate (1〜255) の線形マッピング
pub fn opn_rate_to_x6(rate_5bit: u8, ks: u8) -> u8 {
    if rate_5bit == 0 {
        return 0;
    }
    // A4のkeycode=19の導出はopz2x6::conv::KEY_CODE_A4のコメント参照（(block<<2)|(note>>2)、block=4,note=12）。
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (2 * rate_5bit as u16 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

/// AR専用のオンセット補正バイアス（38x6 rate 加算値）。
///
/// 38x6のアタックはdBリニア（設計原則：ノブ全域で知覚的に均等）で、出力dB=−96×(1−env_level)が
/// 時間に対して直線的に上昇する。このため可聴オンセット（≈−30dB到達）はアタック時間の約69%地点に来る。
/// 一方OPN実機のアタックは「減衰量が現在値に比例して減る」指数接近で、−30dB到達は約24%地点と早い。
/// 同じ総アタック時間でも38x6の方が発音開始が遅れて聞こえ、ALG4等の複数キャリア音色で
/// オペレーター間の発音タイミングがばらついて感じられる。
///
/// 38x6のアタック形状は原則上維持したいので、変換側でARを速める（=総アタックを短くする）ことで
/// オンセットをOPNに寄せる。38x6 rate→時間は指数なので、rateへの一定加算=時間の一定倍率。
/// +30で時間は約0.30倍（38x6オンセット0.69×0.30≒0.21 ≈ OPN 0.24）となりオンセットがほぼ一致する。
/// 代償としてスウェル（緩やかな膨らみ）は短くなる。これはdBリニア維持下での知覚的妥協。
const ATTACK_ONSET_BIAS: u16 = 30;

/// OPN 5bit AR（0〜31）+ KS（0〜3）→ 38x6 ar（0〜255）。
///
/// [opn_rate_to_x6] に [ATTACK_ONSET_BIAS] を加算し、dBリニアアタックの可聴オンセット遅れを補正する。
/// Decay/Release は形状がOPNと一致するため補正不要（[opn_rate_to_x6] / [opn_rr_to_x6] をそのまま使う）。
pub fn opn_ar_to_x6(ar_5bit: u8, ks: u8) -> u8 {
    if ar_5bit == 0 {
        return 0;
    }
    (opn_rate_to_x6(ar_5bit, ks) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPN 4bit RR（0〜15）+ KS（0〜3）→ 38x6 rr（0〜255）。
///
/// OPN RR の eg_rate_eff = 4×rr + 2 + ksr_at_a4(ks)。
pub fn opn_rr_to_x6(rr_4bit: u8, ks: u8) -> u8 {
    // A4のkeycode=19の導出はopz2x6::conv::KEY_CODE_A4のコメント参照（(block<<2)|(note>>2)、block=4,note=12）。
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (4 * rr_4bit as u16 + 2 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

// ---------------------------------------------------------------------------
// 構造体変換
// ---------------------------------------------------------------------------

impl OpnOperator {
    /// OPNオペレーター → ym38x6 `OperatorParams`。
    pub fn to_operator_params(self) -> OperatorParams {
        OperatorParams {
            tl: tl_opn_to_x6(self.tl),
            ar: opn_ar_to_x6(self.ar, self.ks),
            d1r: opn_rate_to_x6(self.d1r, self.ks),
            d2r: opn_rate_to_x6(self.d2r, self.ks),
            d1l: sl_opn_to_x6(self.d1l),
            rr: opn_rr_to_x6(self.rr, self.ks),
            mul: self.mul.min(15),
            dt1: opn_dt1_to_x6(self.dt1),
            ksr: ks_to_ksr(self.ks),
            am_enable: self.am_enable,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
            floor: 0,
            loop_enabled: 0,
            curve: 0,
        }
    }
}

impl OpnVoice {
    /// OPNボイス → ym38x6 `Ym38x6Patch`。
    pub fn to_ym38x6_patch(self) -> Ym38x6Patch {
        Ym38x6Patch {
            operators: [
                self.operators[0].to_operator_params(),
                self.operators[1].to_operator_params(),
                self.operators[2].to_operator_params(),
                self.operators[3].to_operator_params(),
            ],
            channel: ChannelParams {
                algorithm: self.algorithm.min(7),
                // OPN FB(3bit, 0〜7) → 38x6 feedback(0〜255)。
                // scale_3bit(7)=252 → feedback_to_scale≈1.7サイクル（実機@113等のFBノイズ再現に合わせ調整）。
                feedback: scale_3bit(self.feedback.min(7)),
                ..ChannelParams::default()
            },
        }
    }
}

/// `PresetFile` のバンク番号を取り出す。
pub fn bank_of(file: &PresetFile) -> u16 {
    match file {
        PresetFile::Presets { bank, .. } | PresetFile::Programs { bank, .. } => *bank,
    }
}

/// `PresetFile` 内のプリセット件数。
pub fn preset_count(file: &PresetFile) -> usize {
    match file {
        PresetFile::Presets { presets, .. } => presets.len(),
        PresetFile::Programs { programs, .. } => programs.len(),
    }
}

/// ボイス列を `.38x6` プリセットファイル群へ変換する。
///
/// MUCOM88 のスロット番号をそのままプログラム番号として保持:
/// - slot  0〜127 → Bank `start_bank`,   program = slot
/// - slot 128〜255 → Bank `start_bank+1`, program = slot - 128
pub fn voices_to_preset_files(start_bank: u16, voices: &[NamedVoice]) -> Vec<PresetFile> {
    let mut bank0: Vec<PresetEntry> = Vec::new();
    let mut bank1: Vec<PresetEntry> = Vec::new();
    for nv in voices {
        let entry = PresetEntry {
            program: (nv.slot % 128) as u8,
            name: nv.name.clone(),
            patch: nv.voice.to_ym38x6_patch(),
        };
        if nv.slot < 128 {
            bank0.push(entry);
        } else {
            bank1.push(entry);
        }
    }
    let mut files = Vec::new();
    if !bank0.is_empty() {
        files.push(PresetFile::Presets { bank: start_bank, presets: bank0 });
    }
    if !bank1.is_empty() {
        files.push(PresetFile::Presets { bank: start_bank + 1, presets: bank1 });
    }
    files
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tl_polarity_inverts() {
        assert_eq!(tl_opn_to_x6(0), 254);
        assert_eq!(tl_opn_to_x6(127), 0);
    }

    #[test]
    fn sl_polarity_inverts() {
        assert_eq!(sl_opn_to_x6(0), 255);
        assert_eq!(sl_opn_to_x6(15), 0);
    }

    #[test]
    fn scaling_bounds() {
        assert_eq!(scale_5bit(31), 248);
        assert_eq!(scale_4bit(15), 255);
        assert_eq!(scale_3bit(7), 252);
        assert_eq!(ks_to_ksr(3), 255);
    }

    #[test]
    fn opn_dt1_center_values() {
        assert_eq!(opn_dt1_to_x6(0), 128); // ±0
        assert_eq!(opn_dt1_to_x6(4), 128); // ±0 (同じ)
    }

    #[test]
    fn opn_dt1_positive_ascending() {
        assert!(opn_dt1_to_x6(1) > 128);
        assert!(opn_dt1_to_x6(2) > opn_dt1_to_x6(1));
        assert!(opn_dt1_to_x6(3) > opn_dt1_to_x6(2));
    }

    #[test]
    fn opn_dt1_negative_descending() {
        assert!(opn_dt1_to_x6(5) < 128);
        assert!(opn_dt1_to_x6(6) < opn_dt1_to_x6(5));
        assert!(opn_dt1_to_x6(7) < opn_dt1_to_x6(6));
    }

    #[test]
    fn opn_ar_applies_onset_bias() {
        // AR=0 はフリーズ（バイアスなし）
        assert_eq!(opn_ar_to_x6(0, 0), 0);
        // 中速ARはバイアス分だけ opn_rate_to_x6 より大きくなる（=より速い）
        let base = opn_rate_to_x6(18, 0);
        assert_eq!(opn_ar_to_x6(18, 0), base + ATTACK_ONSET_BIAS as u8);
        // 既に255付近の高速ARは255にクランプされる
        assert_eq!(opn_ar_to_x6(31, 0), 255);
    }

    #[test]
    fn operator_defaults_for_opn_specific_fields() {
        let op = OpnOperator {
            tl: 0,
            ar: 31,
            d1r: 10,
            d2r: 4,
            d1l: 2,
            rr: 7,
            mul: 2,
            dt1: 0,
            ks: 1,
            am_enable: false,
        };
        let p = op.to_operator_params();
        assert_eq!(p.velocity_sensitivity, 0);
        assert_eq!(p.waveform, 0);
        assert_eq!(p.op_fine_tune, 128);
        assert_eq!(p.dt1, 128); // dt1=0 → center
        assert_eq!(p.ksr, 64);  // ks=1 → ks_to_ksr(1)=64
    }

    #[test]
    fn voice_maps_algorithm_and_feedback() {
        let voice = OpnVoice {
            operators: [OpnOperator::default(); 4],
            algorithm: 4,
            feedback: 7,
        };
        let patch = voice.to_ym38x6_patch();
        assert_eq!(patch.channel.algorithm, 4);
        assert_eq!(patch.channel.feedback, 252); // scale_3bit(7)=252 → feedback_to_scale≈1.7サイクル
        assert_eq!(patch.channel.filter_cutoff, 255);
    }
}
