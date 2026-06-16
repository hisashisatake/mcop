//! OPQ（YM3806）ボイスレジスタ → ym38x6 パッチへの変換ロジック。
//!
//! スケーリング規約は spec-sound.md「OPQから38x6へのコンバーター設計」(L768-791) に準拠。
//! この層は **入力フォーマット（def_seqs.h のバイト配置）に依存しない**。
//! def_seqs.h のパーサー（工程0でdef_seqs.h入手・構造確定後に実装）は
//! [`OpqVoice`] を組み立てるところまでを担当し、変換はここに閉じる。

use ym38x6_core::{ChannelParams, OperatorParams, PresetEntry, PresetFile, Ym38x6Patch};

// ---------------------------------------------------------------------------
// OPQ中間表現（各レジスタを実機のビット幅のまま保持する）
// ---------------------------------------------------------------------------

/// OPQオペレーター1個分のレジスタ値（実機のビット幅のまま）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpqOperator {
    /// Total Level（7bit, 0〜127。0=最大音量/0dB、127=最小音量/-95.25dB の減衰量）。
    pub tl: u8,
    /// Attack Rate（5bit, 0〜31）。
    pub ar: u8,
    /// Decay1 Rate（5bit, 0〜31）。
    pub d1r: u8,
    /// Decay2 Rate（5bit, 0〜31）。
    pub d2r: u8,
    /// Decay1 Level / Sustain Level（4bit, 0〜15）。
    pub d1l: u8,
    /// Release Rate（4bit, 0〜15）。
    pub rr: u8,
    /// Multiple（4bit, 0〜15）。OPM/OPN/OPQ/OPZ共通でそのまま流用。
    pub mul: u8,
    /// Detune（6bit, 0〜63。中心32=デチューンなし）。
    pub detune: u8,
    /// Key Scale Rate（2bit, 0〜3）。
    pub ksr: u8,
    /// AMS-EN（このオペレーターをAM変調対象にするか）。
    pub am_enable: bool,
}

/// OPQ 1ボイス（4オペレーター + チャンネル設定）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpqVoice {
    pub operators: [OpqOperator; 4],
    /// Algorithm / Connection（3bit, 0〜7）。ym38x6の`ALGORITHMS`と同一トポロジー。
    pub algorithm: u8,
    /// Feedback（3bit, 0〜7）。
    pub feedback: u8,
}

/// 名前付きボイス（def_seqs.h由来の音色名と本体）。
#[derive(Clone, Debug, PartialEq)]
pub struct NamedVoice {
    pub name: String,
    pub voice: OpqVoice,
}

// ---------------------------------------------------------------------------
// スカラー変換（spec-sound.md L768-791 準拠・線形で可逆）
// ---------------------------------------------------------------------------

/// OPQ 5bit レート（AR/D1R/D2R, 0〜31）+ KSR（0〜3）→ 38x6 rate（0〜255）。
///
/// mucom2x6 の `opn_rate_to_x6`（OPN実機カーブ）をそのまま流用する。
/// ユーザー判断により OPQ(YM3806) の eg_rate/KSR 機構は OPM/OPN系と同一前提で進める（2026-06-16）。
///
/// OPN/OPQ の eg_rate_eff = 2×rate + ksr_at_a4(ks)。
/// A4（key_code≈19）でのKSR貢献分を base rate に織り込むことで、
/// 38x6 の decay_to_delta / ar_to_delta と実機タイミングを合わせる。
/// - rate=0 → 0（フリーズ）
/// - eg_rate (2〜62) → 38x6 rate (1〜255) の線形マッピング
#[inline]
pub fn opq_rate_to_x6(rate_5bit: u8, ksr: u8) -> u8 {
    if rate_5bit == 0 {
        return 0;
    }
    // A4 の key_code ≈ 19 (block=4, note=A)
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ksr.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (2 * rate_5bit as u16 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

/// AR専用のオンセット補正バイアス（38x6 rate 加算値）。mucom2x6 と同値。
///
/// 38x6のアタックはdBリニアで、可聴オンセット（≈−30dB到達）はアタック時間の約69%地点に来る。
/// 一方OPN/OPQ実機のアタックは指数接近で −30dB到達は約24%地点と早い。同じ総アタック時間でも
/// 38x6の方が発音開始が遅れて聞こえる。変換側でARを速める（+30で時間約0.30倍）ことで
/// オンセットを実機に寄せる。詳細は mucom2x6/conv.rs の同名定数コメント参照。
const ATTACK_ONSET_BIAS: u16 = 30;

/// OPQ 5bit AR（0〜31）+ KSR（0〜3）→ 38x6 ar（0〜255）。
/// [opq_rate_to_x6] に [ATTACK_ONSET_BIAS] を加算し、dBリニアアタックのオンセット遅れを補正する。
#[inline]
pub fn opq_ar_to_x6(ar_5bit: u8, ksr: u8) -> u8 {
    if ar_5bit == 0 {
        return 0;
    }
    (opq_rate_to_x6(ar_5bit, ksr) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPQ 4bit RR（0〜15）+ KSR（0〜3）→ 38x6 rr（0〜255）。
/// RR の eg_rate_eff = 4×rr + 2 + ksr_at_a4(ks)。mucom2x6 の `opn_rr_to_x6` と同一。
#[inline]
pub fn opq_rr_to_x6(rr_4bit: u8, ksr: u8) -> u8 {
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ksr.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (4 * rr_4bit as u16 + 2 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

/// 3bit（0〜7）→ 8bit（0〜255）: ×36。Feedback に使用。
#[inline]
pub fn scale_3bit(v: u8) -> u8 {
    (v.min(7)) * 36
}

/// 2bit（0〜3）→ 8bit（0〜255）: ×85。KSR に使用。
#[inline]
pub fn scale_2bit(v: u8) -> u8 {
    (v.min(3)) * 85
}

/// Detune 6bit（0〜63, 中心32）→ DT1 8bit（中心128）: ×4。
#[inline]
pub fn detune_to_dt1(v: u8) -> u8 {
    (v.min(63)) * 4
}

/// Total Level: OPQ（減衰量 0=最大音量, 127=最小音量）→ 38x6（音量ノブ 0=最小, 254=最大）。
/// 極性反転 + ×2: `(127 - tl) * 2`。
#[inline]
pub fn tl_opq_to_x6(tl: u8) -> u8 {
    (127 - tl.min(127)) * 2
}

/// 逆変換（可逆性検証用 / 将来のOPQ書き戻し用）: `127 - (x6 / 2)`。
/// `tl_opq_to_x6`は偶数のみ生成するため0〜127で完全可逆。現状はテストからのみ使用。
#[allow(dead_code)]
#[inline]
pub fn tl_x6_to_opq(x6: u8) -> u8 {
    127 - (x6 / 2)
}

/// Decay1 Level / Sustain Level: OPQ（減衰量 0=減衰なし/フルレベル, 15=ほぼ無音）→
/// 38x6（サスティンレベル 0=ほぼ無音, 255=フルレベル）。TLと同じ極性反転 + ×17:
/// `(15 - sl) * 17`（`sl_to_level`のreg=0→sl=255 / reg=15→sl=0アンカーに対応）。
#[inline]
pub fn sl_opq_to_x6(sl: u8) -> u8 {
    (15 - sl.min(15)) * 17
}

/// 逆変換（可逆性検証用 / 将来のOPQ書き戻し用）: `15 - (x6 / 17)`。
/// `sl_opq_to_x6`は17の倍数のみ生成するため0〜15で完全可逆。現状はテストからのみ使用。
#[allow(dead_code)]
#[inline]
pub fn sl_x6_to_opq(x6: u8) -> u8 {
    15 - (x6 / 17)
}

// ---------------------------------------------------------------------------
// 構造体変換
// ---------------------------------------------------------------------------

impl OpqOperator {
    /// OPQオペレーター → ym38x6 `OperatorParams`。
    /// OPQに無いパラメーターはデフォルト/規約値で埋める:
    /// - `velocity_sensitivity = 0`（OPQにベロシティ感度レジスタ無し→実機挙動を再現, spec L789-791）
    /// - `waveform = 0`（サイン波。OPQはサイン固定）
    pub fn to_operator_params(self) -> OperatorParams {
        OperatorParams {
            tl: tl_opq_to_x6(self.tl),
            ar: opq_ar_to_x6(self.ar, self.ksr),
            d1r: opq_rate_to_x6(self.d1r, self.ksr),
            d2r: opq_rate_to_x6(self.d2r, self.ksr),
            d1l: sl_opq_to_x6(self.d1l),
            rr: opq_rr_to_x6(self.rr, self.ksr),
            mul: self.mul.min(15),
            dt1: detune_to_dt1(self.detune),
            ksr: scale_2bit(self.ksr),
            am_enable: self.am_enable,
            velocity_sensitivity: 0,
            waveform: 0,
            // 現状はデチューンを×4→DT1に載せるため、追加チューニングはオフセットなし(中心128)。
            // OPQ広レンジデチューンの高忠実変換を実装する際にここを使う。
            op_fine_tune: 128,
        }
    }
}

impl OpqVoice {
    /// OPQボイス → ym38x6 `Ym38x6Patch`。
    /// チャンネルのフィルター/音色LFO等、OPQに無い項目は`ChannelParams::default()`に従う。
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
                feedback: scale_3bit(self.feedback),
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
/// Programは0〜127のため、128件ごとに連番バンクへ分割する（`start_bank`, `start_bank+1`, ...）。
pub fn voices_to_preset_files(start_bank: u16, voices: &[NamedVoice]) -> Vec<PresetFile> {
    voices
        .chunks(128)
        .enumerate()
        .map(|(bank_index, chunk)| {
            let bank = start_bank + bank_index as u16;
            let presets = chunk
                .iter()
                .enumerate()
                .map(|(program, nv)| PresetEntry {
                    program: program as u8,
                    name: nv.name.clone(),
                    patch: nv.voice.to_ym38x6_patch(),
                })
                .collect();
            PresetFile::Presets { bank, presets }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_reaches_upper_bound() {
        assert_eq!(scale_3bit(7), 252);
        assert_eq!(scale_2bit(3), 255);
    }

    #[test]
    fn scaling_clamps_out_of_range_input() {
        assert_eq!(scale_3bit(99), 252);
        assert_eq!(scale_2bit(99), 255);
    }

    #[test]
    fn rate_zero_freezes_and_max_saturates() {
        // rate=0 は 38x6 rate=0（フリーズ）。
        assert_eq!(opq_rate_to_x6(0, 0), 0);
        assert_eq!(opq_ar_to_x6(0, 0), 0);
        // rate=31 は KSR 加算で eg_rate が上限に張り付き 255。
        assert_eq!(opq_rate_to_x6(31, 0), 255);
        assert_eq!(opq_rr_to_x6(15, 0), 255);
    }

    #[test]
    fn ar_applies_onset_bias() {
        // バイアスが効いている中域の AR で base + 30 になることを確認。
        let base = opq_rate_to_x6(18, 0);
        assert!(base < 255 - ATTACK_ONSET_BIAS as u8);
        assert_eq!(opq_ar_to_x6(18, 0), base + ATTACK_ONSET_BIAS as u8);
    }

    #[test]
    fn higher_ksr_speeds_up_rate() {
        // 同じ register rate でも KSR が大きいほど 38x6 rate は速く（大きく）なる。
        assert!(opq_rate_to_x6(10, 3) >= opq_rate_to_x6(10, 0));
    }

    #[test]
    fn detune_center_maps_to_128() {
        assert_eq!(detune_to_dt1(32), 128);
        assert_eq!(detune_to_dt1(0), 0);
        assert_eq!(detune_to_dt1(63), 252);
    }

    #[test]
    fn tl_polarity_inverts() {
        assert_eq!(tl_opq_to_x6(0), 254); // OPQ最大音量 → 38x6最大音量
        assert_eq!(tl_opq_to_x6(127), 0); // OPQ最小音量 → 38x6最小音量
    }

    #[test]
    fn tl_is_fully_reversible_over_full_range() {
        for tl in 0u8..=127 {
            assert_eq!(tl_x6_to_opq(tl_opq_to_x6(tl)), tl, "tl={tl}");
        }
    }

    #[test]
    fn sl_polarity_inverts() {
        assert_eq!(sl_opq_to_x6(0), 255); // OPQ減衰なし → 38x6フルレベル（サスティンする）
        assert_eq!(sl_opq_to_x6(15), 0); // OPQほぼ無音 → 38x6ほぼ無音（すぐ減衰）
    }

    #[test]
    fn sl_is_fully_reversible_over_full_range() {
        for sl in 0u8..=15 {
            assert_eq!(sl_x6_to_opq(sl_opq_to_x6(sl)), sl, "sl={sl}");
        }
    }

    #[test]
    fn operator_fills_38x6_specific_fields_with_defaults() {
        let op = OpqOperator {
            tl: 10,
            ar: 31,
            d1r: 20,
            d2r: 5,
            d1l: 8,
            rr: 15,
            mul: 3,
            detune: 40,
            ksr: 2,
            am_enable: true,
        };
        let p = op.to_operator_params();
        assert_eq!(p.tl, tl_opq_to_x6(10));
        assert_eq!(p.d1l, sl_opq_to_x6(8));
        assert_eq!(p.ar, opq_ar_to_x6(31, 2)); // KSR込みのレート変換 + オンセット補正
        assert_eq!(p.mul, 3);
        assert_eq!(p.dt1, 160);
        assert_eq!(p.ksr, 170);
        assert!(p.am_enable);
        assert_eq!(p.velocity_sensitivity, 0);
        assert_eq!(p.waveform, 0);
    }

    #[test]
    fn voice_maps_algorithm_and_feedback() {
        let voice = OpqVoice {
            operators: [OpqOperator::default(); 4],
            algorithm: 7,
            feedback: 7,
        };
        let patch = voice.to_ym38x6_patch();
        assert_eq!(patch.channel.algorithm, 7);
        assert_eq!(patch.channel.feedback, 252);
        // OPQに無いフィルター等はデフォルト（cutoff全開）
        assert_eq!(patch.channel.filter_cutoff, 255);
    }

    #[test]
    fn chunks_into_banks_of_128() {
        let voices: Vec<NamedVoice> = (0..130)
            .map(|i| NamedVoice {
                name: format!("V{i}"),
                voice: OpqVoice::default(),
            })
            .collect();
        let files = voices_to_preset_files(1, &voices);
        assert_eq!(files.len(), 2);
        match &files[0] {
            PresetFile::Presets { bank, presets } => {
                assert_eq!(*bank, 1);
                assert_eq!(presets.len(), 128);
                assert_eq!(presets[0].program, 0);
                assert_eq!(presets[127].program, 127);
            }
            _ => panic!("expected Presets"),
        }
        match &files[1] {
            PresetFile::Presets { bank, presets } => {
                assert_eq!(*bank, 2);
                assert_eq!(presets.len(), 2);
                assert_eq!(presets[0].program, 0);
            }
            _ => panic!("expected Presets"),
        }
    }

    #[test]
    fn output_json_round_trips_through_engine_schema() {
        // ym38x6-core の serde を再利用しているため、出力JSONは必ず再パース可能。
        let voices = [NamedVoice {
            name: "Test".to_string(),
            voice: OpqVoice {
                operators: [OpqOperator {
                    tl: 0,
                    ar: 31,
                    d1r: 10,
                    d2r: 4,
                    d1l: 2,
                    rr: 7,
                    mul: 1,
                    detune: 32,
                    ksr: 1,
                    am_enable: false,
                }; 4],
                algorithm: 4,
                feedback: 3,
            },
        }];
        let files = voices_to_preset_files(1, &voices);
        let json = files[0].to_json().expect("serialize");
        let parsed = PresetFile::from_json(&json).expect("deserialize");
        assert_eq!(parsed, files[0]);
    }
}
