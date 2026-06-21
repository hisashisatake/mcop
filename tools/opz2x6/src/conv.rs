//! TX81Z（OPZ/YM2414）レジスタ値 → ym38x6 パッチへの変換ロジック。

use ym38x6_core::{ChannelParams, OperatorParams, PresetEntry, Ym38x6Patch};

use crate::parse::OpzVoice;

// ---------------------------------------------------------------------------
// キャリア判定テーブル（ym38x6-core/src/algorithm.rs から複製）
// index = algorithm (0-7), value = 38x6 operators[] のキャリアインデックス一覧
// ---------------------------------------------------------------------------

const CARRIERS: [&[usize]; 8] = [
    &[3],          // 0: O1→O2→O3→O4
    &[3],          // 1: (O1+O2)→O3→O4
    &[3],          // 2: (O1+(O2→O3))→O4
    &[3],          // 3: ((O1→O2)+O3)→O4
    &[1, 3],       // 4: (O1→O2)+(O3→O4)
    &[1, 2, 3],    // 5: (O1→O2)+(O1→O3)+(O1→O4)
    &[1, 2, 3],    // 6: (O1→O2)+O3+O4
    &[0, 1, 2, 3], // 7: O1+O2+O3+O4（全並列）
];

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// OUT (TX81Z Output Level 0-99, 99=最大) → 38x6 TL（キャリア用、0=無音, 254=最大）。
fn out_to_tl(out: u8) -> u8 {
    (out.min(99) as f32 / 99.0 * 254.0).round() as u8
}

/// モジュレーター TL 天井の既定値。
///
/// エンジンの `modulation` は位相サイクル単位（1.0=1周=2πrad）で、変調深度は
/// β[rad] = tl_to_gain(tl) × FM_MODULATION_INDEX_SCALE(4.0) × 2π。
/// TL=254 → β≈24rad（ノイズ）、TL=200 → β≈2.4rad。
/// 200 だと弱打でも明るすぎ（ブラス寄り）になり、180 の方が聴感が良いと確認したため 180 を既定とする。
/// 180 を基準にしても velocity_sensitivity(KVS) が強打時にモジュレーター実効 TL を ~190 まで
/// 押し上げるため、弱打=穏やか／強打=明るい のグラデーションになる。
/// キャリアは音量に直結するため天井を設けず out_to_tl を使う。
pub const DEFAULT_MOD_TL_CAP: u8 = 180;

/// OUT → 38x6 TL（モジュレーター用、上限 `cap`）。
fn out_to_tl_mod(out: u8, cap: u8) -> u8 {
    (out.min(99) as f32 / 99.0 * cap as f32).round() as u8
}

/// D1L/SL（OPM型 4-bit 0-15）→ 38x6 D1L。
///
/// reg はサスティンレベルの減衰量（reg=0で0dB=減衰なし、reg=15で-93dB≈無音、reg<15は3dB/step）。
/// **キャリア**は 38x6 オペレーターの sustain_level(=`d1l/255` リニア振幅, operator.rs) に合わせて
/// dB をリニア振幅 10^(db/20) に変換する。旧実装は dB を線形に 0-255 へ写していたため、中間 reg
/// （例 reg=13=-39dB）が 0.58(≈-4.7dB) の高い保持レベルになり、撥弦/打鍵系のキャリアが減衰せず
/// 静的化していた（撥弦が「鳴りっぱなし」になる）。
/// **モジュレーター**は音色（明るさ）維持のため従来の dB 線形写像のまま。端点(reg=0→255, reg=15→0)は両者一致。
fn sl_to_x6(reg: u8, is_carrier: bool) -> u8 {
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    if is_carrier {
        (10f32.powf(db / 20.0) * 255.0).round() as u8
    } else {
        (255.0 * (1.0 + db / 93.0)).round() as u8
    }
}

/// TX81Z DET (0-6, 3=中心) → 38x6 dt1（中心128）。
fn det_to_x6(det: u8) -> u8 {
    // DET 3=無デチューン → OPM DT1=0（+0¢）
    // DET 4,5,6 = 正方向（増大）→ DT1=1,2,3
    // DET 0,1,2 = 負方向（増大）→ DT1=7,6,5（DET 0が最強）
    const DT1_FROM_DET: [u8; 7] = [7, 6, 5, 0, 1, 2, 3];
    let dt1 = DT1_FROM_DET[det.min(6) as usize];
    const DT1_TO_X6: [u8; 8] = [128, 131, 134, 136, 128, 125, 122, 120];
    DT1_TO_X6[dt1 as usize]
}

/// TX81Z FREQ (0-63) → 38x6 (MUL 0-15, op_fine_tune 0-255)。
///
/// TX81Z の周波数コースは整数 MUL 比（0.5, 1, 2, ..., 15）を基数として
/// {1.0, √2, 2^(2/3), 2^(3/4)} の4段階で刻む（= 4種 × 16グループ = 64値）。
/// 最近傍の整数 MUL を選び、差分セントを op_fine_tune に写像する。
pub fn freq_to_mul_fine(freq: u8) -> (u8, u8) {
    const SCALE: [f32; 4] = [1.0, 1.414_213_6, 1.587_401, 1.681_793];
    const BASE: [f32; 16] = [
        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0,
        8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    let f = freq.min(63) as usize;
    let ratio = BASE[f / 4] * SCALE[f % 4];

    // 最近傍の整数MUL（対数空間距離）
    let mut best_mul = 0u8;
    let mut best_dist = f32::MAX;
    for m in 0u8..=15 {
        let mul_ratio = if m == 0 { 0.5_f32 } else { m as f32 };
        let dist = (ratio / mul_ratio).log2().abs();
        if dist < best_dist {
            best_dist = dist;
            best_mul = m;
        }
    }

    let mul_ratio = if best_mul == 0 { 0.5_f32 } else { best_mul as f32 };
    let cents = (ratio / mul_ratio).log2() * 1200.0;
    // op_fine_tune: 中心128, 1単位 = 1200/127 ≈ 9.45¢
    let oft = (128.0 + (cents * 127.0 / 1200.0)).round().clamp(0.0, 255.0) as u8;
    (best_mul, oft)
}

/// A4 相当のキーコード（KSR rate scaling の焼き込み量基準）。
const KEY_CODE_A4: u16 = 19;

/// rs(0-3) に応じた KSR(rate key scaling)の加算量。
fn ksr_add(rs: u8) -> u16 {
    let ksr_shift = 3u16.saturating_sub(rs.min(3) as u16);
    KEY_CODE_A4 >> ksr_shift
}

/// OPM型5-bitレート（AR/D1R/D2R, 0-31）→ 38x6 rate。
///
/// **キャリア**は KSR(A4キーコード)の焼き込みを廃止する。38x6エンジンは実行時に
/// `ksr_rate_multiplier(rs, note)` でノート依存KSRを別途適用するため、A4(実行時倍率1.0)では
/// 焼き込み分(rs=3で+19)が二重適用ぎみに効き、減衰が速すぎて撥弦/打鍵キャリアが一瞬で
/// 持続レベルに落ちて静的化していた。KSRは実行時に一元適用とし、レートは基準値とする。
/// **モジュレーター**は音色（明るさの時間変化）維持のため従来どおり KSR を焼き込む。
fn opm_rate_to_x6(rate: u8, rs: u8, is_carrier: bool) -> u8 {
    if rate == 0 { return 0; }
    let add = if is_carrier { 0 } else { ksr_add(rs) };
    let eg_rate = (2 * rate as u16 + add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

const ATTACK_ONSET_BIAS: u16 = 30;

fn ar_to_x6(ar: u8, rs: u8, is_carrier: bool) -> u8 {
    if ar == 0 { return 0; }
    (opm_rate_to_x6(ar, rs, is_carrier) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPM型4-bitリリースレート（RR, 0-15）→ 38x6 rr。
/// KSR焼き込みの扱いは [opm_rate_to_x6] と同様（キャリアは廃止、モジュレーターは従来どおり）。
fn rr_to_x6(rr: u8, rs: u8, is_carrier: bool) -> u8 {
    let add = if is_carrier { 0 } else { ksr_add(rs) };
    let eg_rate = (4 * rr as u16 + 2 + add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// TX81Z AMS (0-3) → 38x6 ams（opm2x6 と同実装）。
fn ams_to_x6(reg: u8) -> u8 {
    if reg == 0 { return 0; }
    (1u16 + 127 * (reg.min(3) as u16 - 1)) as u8
}

/// TX81Z PMS (0-7) → 38x6 pms（opm2x6 と同実装）。
fn pms_to_x6(reg: u8) -> u8 {
    if reg == 0 { return 0; }
    (1.0_f32 + 254.0 * (reg.min(7) - 1) as f32 / 6.0).round() as u8
}

/// AMD/PMD (0-99) → 38x6 depth（0-255 線形スケール）。
fn lfo_depth_to_x6(reg: u8) -> u8 {
    (reg as f32 * 255.0 / 99.0).round() as u8
}

/// TX81Z FB (0-7) → 38x6 feedback（opm2x6 と同実装、FB×36）。
fn fb_to_x6(fb: u8) -> u8 {
    fb.min(7) * 36
}

// ---------------------------------------------------------------------------
// オペレーター変換
// ---------------------------------------------------------------------------

fn convert_op(op: &crate::parse::OpzOpData, is_carrier: bool, opts: ConvOptions) -> OperatorParams {
    let mod_tl_cap = opts.mod_tl_cap;
    let (mul, op_fine_tune) = freq_to_mul_fine(op.freq);

    // EGT=1 のとき D2R を強制的に高値にしてリリース挙動を作る
    // (TX81Z は EGT=1 で D1L で止まらず一定レートで減衰 = sustain-less decay)
    let d2r = if op.egt != 0 && op.d2r == 0 { 20 } else { op.d2r };

    let mut params = OperatorParams {
        tl: if is_carrier { out_to_tl(op.out) } else { out_to_tl_mod(op.out, mod_tl_cap) },
        ar: ar_to_x6(op.ar, op.rs, is_carrier),
        d1r: opm_rate_to_x6(op.d1r, op.rs, is_carrier),
        d2r: opm_rate_to_x6(d2r, op.rs, is_carrier),
        d1l: sl_to_x6(op.d1l, is_carrier),
        rr: rr_to_x6(op.rr, op.rs, is_carrier),
        mul,
        dt1: det_to_x6(op.det),
        ksr: opts.ksr_override.unwrap_or(op.rs.min(3) * 85),
        am_enable: op.ame,
        // キャリアは velocity_sensitivity=0（38x6の「velocity=音量」設計を維持）
        // モジュレーターは KVS を写像: KVS(0-7) → 0..70
        // (* 255/7 は実効TLを最大にクランプさせすぎるため * 10 に抑制)
        velocity_sensitivity: if is_carrier { 0 } else { op.kvs.min(7) * 10 },
        waveform: op.ow.min(7),
        op_fine_tune,
    };

    // 味付け: キャリアのサステイン延長（実機忠実から意図的に離す）。
    if is_carrier && opts.carrier_sustain > 0.0 {
        let k = opts.carrier_sustain.clamp(0.0, 1.0);
        // D1L を満レベル方向へ持ち上げる（最大で残差の 70% まで）。
        let d1l = params.d1l as f32;
        params.d1l = (d1l + (255.0 - d1l) * 0.7 * k).round().clamp(0.0, 255.0) as u8;
        // 減衰レートを遅くする（値が小さいほど遅い）。D2R は鳴りの伸びに直結するため強めに。
        params.d1r = (params.d1r as f32 * (1.0 - 0.60 * k)).round().clamp(0.0, 255.0) as u8;
        params.d2r = (params.d2r as f32 * (1.0 - 0.85 * k)).round().clamp(0.0, 255.0) as u8;
    }

    params
}

// ---------------------------------------------------------------------------
// ボイス変換
// ---------------------------------------------------------------------------

/// 変換オプション（音質追い込み用の上書き群）。
#[derive(Clone, Copy, Debug)]
pub struct ConvOptions {
    /// モジュレーター TL 天井。
    pub mod_tl_cap: u8,
    /// チャンネルフィードバックの上書き（`Some(n)` で 38x6 feedback を直接指定、`None` で .syx 由来）。
    /// 切り分け診断用：`Some(0)` でフィードバックを無効化できる。
    pub fb_override: Option<u8>,
    /// 全オペレーターの KSR（鍵盤レート追従）上書き（`None` で .syx 由来）。
    /// 切り分け診断用：`Some(0)` で高音のエンベロープ加速を弱められる。
    pub ksr_override: Option<u8>,
    /// キャリアのサステイン延長（味付け用、0.0=実機忠実 .. 1.0=最大延長）。
    ///
    /// TX81Z のファクトリー音色（特にエレピ/ピアノ）は実機からして打鍵的で減衰が速く、
    /// 「楽器として伸びが欲しい」場合に実機から意図的に離して鳴りを伸ばす。
    /// キャリアのみ D1L（サステインレベル）を満レベル方向へ持ち上げ、D1R/D2R（減衰レート）を
    /// 遅くする。モジュレーターには触れず音色の明るさ変化は保つ。
    pub carrier_sustain: f32,
    /// ローパスフィルターのカットオフ上書き（味付け用、`None`=全開255、20kHz）。
    ///
    /// 倍音過多/耳障りな高域を抑えるためのレバー。`--mod-cap`/`--fb` で変調自体を削ると
    /// FMサイドバンドが作る基音まで失われ音程感が壊れる（高い音だけ残る）。フィルターなら
    /// 変調はそのままに出力の高域だけ削るので、低域の基音を保ったまま明るさを落とせる。
    /// 値は 0〜255（指数で 20Hz〜20kHz、180≈2.8kHz / 200≈4.5kHz）。
    pub filter_cutoff: Option<u8>,
}

impl Default for ConvOptions {
    fn default() -> Self {
        Self {
            mod_tl_cap: DEFAULT_MOD_TL_CAP,
            fb_override: None,
            ksr_override: None,
            carrier_sustain: 0.0,
            filter_cutoff: None,
        }
    }
}

/// OpzVoice → Ym38x6Patch（オプション指定）。
pub fn voice_to_patch_opts(voice: &OpzVoice, opts: ConvOptions) -> Ym38x6Patch {
    let alg = voice.algorithm.min(7) as usize;
    let carriers = CARRIERS[alg];

    // ops[] = [OP4, OP3, OP2, OP1]（parse.rs で結線順4/3/2/1 に整列済み）を
    // 38x6 operators[0..3] へ写像する。
    //
    // 38x6 の ALGORITHMS は ymfm の OPN系 s_algorithm_ops を移植したもので、
    // operators[0..3] は ymfm の m_op[0..3]（アルゴリズム上の O1/O2/O3/O4）と一致する。
    // OPZ(YM2414) は OPM系チップなので、レジスタ slot → m_op に slot1↔slot2 の
    // インターリーブが入る（ymfm `opz_registers::operator_map`：m_op=[slot0,slot2,slot1,slot3]）。
    // TX81Z が VMEM の OP1〜OP4 を slot0〜slot3 へ素直に書く（OP1→slot0 … OP4→slot3）ため、
    //   m_op = [slot0, slot2, slot1, slot3] = [OP1, OP3, OP2, OP4]
    // となる。よって operators = [OP1, OP3, OP2, OP4] = [ops[3], ops[1], ops[2], ops[0]]。
    //
    // 旧実装は単純逆順 [OP1, OP2, OP3, OP4] で、この slot1↔2 入替（OP2↔OP3）を
    // 落としていた。そのため非整数キャリア比パッチ（LoTine81Z 等、alg4 のキャリアが
    // OP2/OP4 にずれて基音 1.0× を失い tritone 上ずり）で音程が狂っていた。
    // ymfm OPZ 参照（opzref）で alg4=LoTine が基音 415Hz に復帰し、alg2=GrandPiano の
    // 倍音は実機録音と同等を維持することを確認済み（slot1↔2 はチップ固有で全 alg 共通、
    // アルゴリズム番号の再マップは不要）。
    const OP_SRC: [usize; 4] = [3, 1, 2, 0]; // operators[i] ← ops[OP_SRC[i]]
    let operators = std::array::from_fn(|i| {
        let op = &voice.ops[OP_SRC[i]];
        let is_carrier = carriers.contains(&i);
        convert_op(op, is_carrier, opts)
    });

    Ym38x6Patch {
        operators,
        channel: ChannelParams {
            algorithm: alg as u8,
            feedback: opts.fb_override.unwrap_or_else(|| fb_to_x6(voice.feedback)),
            tone_lfo_freq: (voice.lfo_spd as f32 * 255.0 / 99.0).round() as u8,
            tone_lfo_pmd: lfo_depth_to_x6(voice.pmd),
            tone_lfo_amd: lfo_depth_to_x6(voice.amd),
            tone_lfo_delay: (voice.lfo_dly as f32 * 255.0 / 99.0).round() as u8,
            pms: pms_to_x6(voice.pms),
            ams: ams_to_x6(voice.ams),
            filter_cutoff: opts.filter_cutoff.unwrap_or(255),
            ..ChannelParams::default()
        },
    }
}

/// OpzVoice → Ym38x6Patch（既定オプション）。
pub fn voice_to_patch(voice: &OpzVoice) -> Ym38x6Patch {
    voice_to_patch_opts(voice, ConvOptions::default())
}

/// OpzVoice → PresetEntry（オプション指定）。
pub fn voice_to_entry_opts(voice: &OpzVoice, opts: ConvOptions) -> PresetEntry {
    PresetEntry {
        program: (voice.number % 128) as u8,
        name: voice.name.clone(),
        patch: voice_to_patch_opts(voice, opts),
    }
}

/// OpzVoice → PresetEntry（既定オプション）。
pub fn voice_to_entry(voice: &OpzVoice) -> PresetEntry {
    voice_to_entry_opts(voice, ConvOptions::default())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::OpzOpData;

    #[test]
    fn out_to_tl_polarity() {
        assert_eq!(out_to_tl(0), 0);    // 無音
        assert_eq!(out_to_tl(99), 254); // 最大（キャリア用）
        assert!(out_to_tl(50) > 0 && out_to_tl(50) < 254);
    }

    #[test]
    fn out_to_tl_mod_caps_at_given_cap() {
        assert_eq!(out_to_tl_mod(0, 200), 0);
        assert_eq!(out_to_tl_mod(99, 200), 200);
        assert!(out_to_tl_mod(50, 200) > 0 && out_to_tl_mod(50, 200) < 200);
        // 天井を変えると最大値も追従する
        assert_eq!(out_to_tl_mod(99, 254), 254);
        assert_eq!(out_to_tl_mod(99, 180), 180);
    }

    #[test]
    fn det_center_no_detune() {
        assert_eq!(det_to_x6(3), 128); // 中心 = 無デチューン
    }

    #[test]
    fn det_positive_and_negative() {
        assert!(det_to_x6(4) > 128); // 正方向
        assert!(det_to_x6(5) > det_to_x6(4)); // 大きくなる
        assert!(det_to_x6(2) < 128); // 負方向
        assert!(det_to_x6(1) < det_to_x6(2)); // 絶対値増大
    }

    #[test]
    fn freq_integer_mul_gives_center_fine() {
        // FREQ 4 = 1.0x (MUL=1), FREQ 8 = 2.0x (MUL=2) etc. → op_fine_tune=128
        let (m, oft) = freq_to_mul_fine(4);
        assert_eq!(m, 1);
        assert_eq!(oft, 128);

        let (m, oft) = freq_to_mul_fine(8);
        assert_eq!(m, 2);
        assert_eq!(oft, 128);

        let (m, oft) = freq_to_mul_fine(0); // 0.5x
        assert_eq!(m, 0);
        assert_eq!(oft, 128);
    }

    #[test]
    fn sl_to_x6_carrier_linear_amplitude() {
        // キャリア: 端点は不変
        assert_eq!(sl_to_x6(0, true), 255); // 0dB=減衰なし=フル保持
        assert_eq!(sl_to_x6(15, true), 0);  // -93dB≈無音
        // reg=13(-39dB)はリニア振幅 0.0112 → 約3（撥弦キャリアが静的化していた回帰テスト）
        assert_eq!(sl_to_x6(13, true), 3);
        // モジュレーターは従来の dB 線形写像（音色維持）: reg=13 は 148 のまま
        assert_eq!(sl_to_x6(13, false), 148);
        // 端点はキャリア/モジュレーターで一致
        assert_eq!(sl_to_x6(0, false), 255);
        assert_eq!(sl_to_x6(15, false), 0);
    }

    #[test]
    fn rate_ksr_carrier_vs_modulator() {
        // D1R=16, rs=3: モジュレーターは KSR 焼き込みで速い、キャリアは KSR なしで遅い
        let carr = opm_rate_to_x6(16, 3, true);
        let modu = opm_rate_to_x6(16, 3, false);
        assert!(carr < modu, "キャリアは KSR 無しで遅い(値が小さい): carr={carr} mod={modu}");
    }

    #[test]
    fn freq_sqrt2_interval_near_tritone() {
        // FREQ 5 = 1.414x: MUL=1 か MUL=2 の近傍（600¢）
        let (m, oft) = freq_to_mul_fine(5);
        // ±600¢ なので either MUL=1(oft>128) or MUL=2(oft<128)
        assert!(m == 1 || m == 2);
        if m == 1 { assert!(oft > 128); } else { assert!(oft < 128); }
    }

    #[test]
    fn kvs_carrier_is_zero() {
        let op = OpzOpData { kvs: 7, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 0, "carrier velocity_sensitivity must be 0");
    }

    #[test]
    fn kvs_modulator_maps_70_at_max() {
        let op = OpzOpData { kvs: 7, freq: 4, det: 3, out: 50, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 70);
    }

    #[test]
    fn kvs_modulator_zero_stays_zero() {
        let op = OpzOpData { kvs: 0, freq: 4, det: 3, out: 50, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 0);
    }

    #[test]
    fn egt1_forces_d2r_nonzero() {
        let op = OpzOpData { d2r: 0, egt: 1, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, ConvOptions::default());
        assert!(p.d2r > 0, "EGT=1 with D2R=0 should force d2r > 0");
    }

    #[test]
    fn egt0_d2r_zero_stays_zero() {
        let op = OpzOpData { d2r: 0, egt: 0, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, ConvOptions::default());
        assert_eq!(p.d2r, 0, "EGT=0 D2R=0 → sustain型（d2r=0のまま）");
    }

    #[test]
    fn waveform_direct_copy() {
        let op = OpzOpData { ow: 5, freq: 4, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, ConvOptions::default());
        assert_eq!(p.waveform, 5);
    }

    #[test]
    fn op_order_reversal() {
        // ops[] = [OP4, OP3, OP2, OP1]。端点は OP4↔OP1 が入れ替わる:
        // operators[0]=OP1(ops[3]), operators[3]=OP4(ops[0])
        let mut voice = OpzVoice::default();
        voice.ops[0] = OpzOpData { out: 10, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() }; // OP4
        voice.ops[3] = OpzOpData { out: 80, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() }; // OP1
        let patch = voice_to_patch(&voice);
        // operators[0] ← OP1 (out=80) → tl should be large
        assert!(patch.operators[0].tl > patch.operators[3].tl,
            "operators[0](OP1,out=80) should have higher tl than operators[3](OP4,out=10)");
    }

    #[test]
    fn opz_slot_interleave_swaps_op2_op3() {
        // OPZ(YM2414) の slot1↔slot2 インターリーブ（ymfm operator_map）を反映し、
        // operators = [OP1, OP3, OP2, OP4] になることを検証する。
        // ops[] = [OP4, OP3, OP2, OP1] に判別用 freq（→mul）を仕込む。
        let mut voice = OpzVoice::default();
        voice.ops[0] = OpzOpData { freq: 8,  det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP4 → mul2
        voice.ops[1] = OpzOpData { freq: 12, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP3 → mul3
        voice.ops[2] = OpzOpData { freq: 16, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP2 → mul4
        voice.ops[3] = OpzOpData { freq: 4,  det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP1 → mul1
        let p = voice_to_patch(&voice);
        assert_eq!(p.operators[0].mul, 1, "operators[0]=OP1");
        assert_eq!(p.operators[1].mul, 3, "operators[1]=OP3 (slot1↔2 interleave)");
        assert_eq!(p.operators[2].mul, 4, "operators[2]=OP2 (slot1↔2 interleave)");
        assert_eq!(p.operators[3].mul, 2, "operators[3]=OP4");
    }
}
