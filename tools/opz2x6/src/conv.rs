//! TX81Z（OPZ/YM2414）レジスタ値 → ym38x6 パッチへの変換ロジック。

use ym38x6_core::{ChannelParams, OperatorParams, PresetEntry, Ym38x6Patch};

use crate::parse::OpzVoice;

// ---------------------------------------------------------------------------
// キャリア判定テーブル（ym38x6-core/src/algorithm.rs から複製）
// index = algorithm (0-7), value = 38x6 operators[] のキャリアインデックス一覧
// ---------------------------------------------------------------------------

pub const CARRIERS: [&[usize]; 8] = [
    &[3],          // 0: O1→O2→O3→O4
    &[3],          // 1: (O1+O2)→O3→O4
    &[3],          // 2: (O1+(O2→O3))→O4
    &[3],          // 3: ((O1→O2)+O3)→O4
    &[1, 3],       // 4: (O1→O2)+(O3→O4)
    &[1, 2, 3],    // 5: (O1→O2)+(O1→O3)+(O1→O4)
    &[1, 2, 3],    // 6: (O1→O2)+O3+O4
    &[0, 1, 2, 3], // 7: O1+O2+O3+O4（全並列）
];

/// 指定アルゴリズムでオペレーター`operator_index`（38x6 operators[]のインデックス）が
/// キャリアかどうか。opzref（レジスタ直書き検証ツール）からも共有で使う。
pub fn is_carrier(alg: u8, operator_index: usize) -> bool {
    CARRIERS[alg.min(7) as usize].contains(&operator_index)
}

/// TX81Z Aalg：アルゴリズムによる減衰量（実機のTLレジスタへの追加減衰、キャリアのみに適用）。
///
/// 出典: nornandブログ「TX81Zを解析した（Operator Output Level編）」の箇条書き
/// （アルゴリズム1234=減衰0／5=op1,3が8・op2,4が0／67=op1,2,3が13・op4が0／8=全op16）。
/// この「減衰を受けるオペレーター」の集合は[CARRIERS]（キャリア）と完全に一致し、
/// 減衰量はキャリア本数nに対し`-20*log10(n)`dB（並列合成時の位相加算ヘッドルーム補正、
/// 0.75dB/stepでほぼ厳密に8/13/16と一致）。モジュレーターおよび単一キャリアのアルゴリズムは0。
const ALG_ATTEN_BY_CARRIER_COUNT: [u8; 5] = [0, 0, 8, 13, 16];

/// アルゴリズム(0-7)のキャリアに適用するAalg減衰量。opzrefからも共有で使う。
pub fn alg_atten(alg: u8) -> u8 {
    ALG_ATTEN_BY_CARRIER_COUNT[CARRIERS[alg.min(7) as usize].len().min(4)]
}

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// TX81Z Operator Output Level(OL, 0-99, 99=最大) → 実機のTLレジスタ加算値(Aol, 0-127, 0=最大)。
///
/// 出典: nornandブログ「TX81Zを解析した（Operator Output Level編）」
/// (https://nornand.hatenablog.com/entry/2020/11/21/201911、TX81Zシステムroom解析による実測値)。
/// V_TL = A_vol + A_alg + A_ol + A_ls + A_kvs + A_ebs という式のA_ol項で、OL=20以上は
/// 1刻みの線形（Aol=99-OL）だが、OL=19以下で急激に非線形化する（無音側を急に絞る）。
/// 旧実装（out/99*254の単純線形近似）はこの非線形域を再現していなかった。
const OL_TO_AOL: [u8; 100] = [
    127, 122, 118, 114, 110, 107, 104, 102, 100, 98, // OL=0..9
    96, 94, 92, 90, 88, 86, 85, 84, 82, 81, // OL=10..19
    79, 78, 77, 76, 75, 74, 73, 72, 71, 70, // OL=20..29
    69, 68, 67, 66, 65, 64, 63, 62, 61, 60, // OL=30..39
    59, 58, 57, 56, 55, 54, 53, 52, 51, 50, // OL=40..49
    49, 48, 47, 46, 45, 44, 43, 42, 41, 40, // OL=50..59
    39, 38, 37, 36, 35, 34, 33, 32, 31, 30, // OL=60..69
    29, 28, 27, 26, 25, 24, 23, 22, 21, 20, // OL=70..79
    19, 18, 17, 16, 15, 14, 13, 12, 11, 10, // OL=80..89
    9, 8, 7, 6, 5, 4, 3, 2, 1, 0, // OL=90..99
];

/// TX81Z Operator Output Level(OL, 0-99) → 実機のTLレジスタ加算値(Aol, 0-127)。
/// opzref（レジスタ直書き検証ツール）からも共有で使う。
pub fn ol_to_atten(ol: u8) -> u8 {
    OL_TO_AOL[ol.min(99) as usize]
}

/// Aol(0-127, TX81Z実機のTLレジスタ加算値。0=最大出力、127=最小) → 38x6 TL(0-255, 0=無音, 255=最大)。
/// 両者ともdB線形スケール（OPM系TLは0.75dB/step、38x6は0.373dB/stepでいずれも約95.25dB幅）
/// なので、単純な向き反転+ビット幅リスケールで変換できる。
fn aol_to_tl(aol: u8) -> u8 {
    ((127 - aol.min(127)) as f32 / 127.0 * 255.0).round() as u8
}

/// OUT (TX81Z Output Level 0-99, 99=最大) → 38x6 TL（キャリア用、0=無音, 255=最大）。
/// `extra_atten` はAalg（アルゴリズムによる追加減衰、[alg_atten]参照）。
fn out_to_tl(out: u8, extra_atten: u8) -> u8 {
    aol_to_tl(ol_to_atten(out).saturating_add(extra_atten).min(127))
}

/// モジュレーター TL 天井の既定値。
///
/// 2026-07-01は「実機録音とのRMSE比較で天井なしが最良」としてこの天井を既定から外したが、
/// 2026-07-02にGrandPiano等をopz2x6→38x6エンジン経由（`opzref`のymfm直接レンダリングではなく）
/// で試聴したところ、天井なしは明確にノイジーだった。フィードバック無効化(`--fb 0`)・波形強制
/// サイン化のいずれでも改善せず、天井180で初めてノイズが解消した（ユーザー確認、2026-07-02）。
/// RMSE比較はサンプル単位の誤差であり聴感上の「ノイズっぽさ」（高域のギラつき等、全体エネルギー
/// への寄与が小さい成分）を捉えにくいため、聴感の結果を優先しこの値を既定へ戻した。
pub const DEFAULT_MOD_TL_CAP: u8 = 180;

/// OUT → 38x6 TL（モジュレーター用）。`cap` が `Some` なら上限を設ける（既定は`Some(DEFAULT_MOD_TL_CAP)`）、
/// `None` にするとキャリアと同じ `out_to_tl` を使う（天井なし＝実機のAol/Aalgをそのまま反映、
/// 診断用。2026-07-02時点ではノイジーになることを確認済みなので通常は使わない）。
fn out_to_tl_mod(out: u8, cap: Option<u8>) -> u8 {
    match cap {
        Some(cap) => {
            let aol = ol_to_atten(out);
            ((127 - aol.min(127)) as f32 / 127.0 * cap as f32).round() as u8
        }
        None => out_to_tl(out, 0),
    }
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

/// TX81Z FREQ coarse(0-63) → [group(0-3), order基準値]。
///
/// TX81Zの周波数比は「4個おきの連続ブロック」のような単純な規則ではなく、coarse値が
/// 4グループ（グループごとに異なる線形係数）へ複雑にスクランブルされて割り当てられている。
/// 旧実装は前者の単純化された誤った仮定を使っており、coarse値が大きくなるほど誤差が拡大していた
/// （例: coarse=22で旧実装7.937 vs 実測7.00、+13.4%の誤差。FM倍音比としては協和(7.00)から
/// 不協和寸前(7.937≒8倍音直下)への変質に相当し、金属的・非整数倍音的な音色劣化の主因だった）。
/// 出典: https://mgregory22.me/tx81z/freqratios.html （TX81Zの周波数比を実測でリバースエンジニアリング。
/// 著者自身「端数に説明のつかない周期誤差が残る」と明記しており完全な精度は無いが、
/// 旧実装より大幅に正確）。
/// fine(0-15、TX81Zの別パラメーターだがVMEMダンプには含まれないため常に0として扱う)は
/// `order = order_base + fine` として加算される。
const COARSE_TO_GROUP: [(u8, u16); 64] = [
    (0, 0), (1, 0), (2, 0), (3, 0), (0, 8), (1, 8), (2, 8), (3, 8), (0, 24), (1, 24),
    (0, 40), (2, 24), (3, 24), (0, 56), (1, 40), (2, 40), (0, 72), (3, 40), (1, 56), (0, 88),
    (2, 56), (3, 56), (0, 104), (1, 72), (2, 72), (0, 120), (1, 88), (3, 72), (0, 136), (2, 88),
    (1, 104), (0, 152), (3, 88), (2, 104), (0, 168), (1, 120), (0, 184), (3, 104), (2, 120), (1, 136),
    (0, 200), (3, 120), (0, 216), (1, 152), (2, 136), (0, 232), (1, 168), (3, 136), (2, 152), (1, 184),
    (2, 168), (3, 152), (1, 200), (2, 184), (3, 168), (1, 216), (2, 200), (3, 184), (1, 232), (2, 216),
    (3, 200), (2, 232), (3, 216), (3, 232),
];

/// [COARSE_TO_GROUP]の4グループそれぞれの(基準値, 増分)係数。ratio = base + step * order。
const FREQ_RATIO_COEFFS: [(f32, f32); 4] = [
    (0.50, 0.0625),
    (0.71, 0.088105),
    (0.78, 0.098145),
    (0.87, 0.108105),
];

/// TX81Z FREQ coarse(0-63) → 周波数比率(ratio)。opz2x6/opzref共通で使う変換の核。
pub fn coarse_to_ratio(coarse: u8) -> f32 {
    let (group, order_base) = COARSE_TO_GROUP[coarse.min(63) as usize];
    let (base, step) = FREQ_RATIO_COEFFS[group as usize];
    base + step * order_base as f32
}

/// TX81Z FREQ (0-63) → 38x6 (MUL 0-15, op_fine_tune 0-255)。
/// 最近傍の整数 MUL を選び、差分セントを op_fine_tune に写像する。
pub fn freq_to_mul_fine(freq: u8) -> (u8, u8) {
    let ratio = coarse_to_ratio(freq);

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
/// 標準OPM/OPZのnote code表（C=0,C#=1,D=2,D#=4,E=5,F=6,F#=8,G=9,G#=10,A=12,A#=13,B=14。
/// opzref::NOTECODEと同一）を用いて、block_freq上位5bitのkeycode=(block<<2)|(note>>2)
/// （ymfm opm_key_code_to_phase_step/ymfm_opz.cppのkeycode定義と同一式）にA4(block=4, note=12)を
/// 代入すると (4<<2)|(12>>2) = 19。他ツールのKEY_CODE_A4も同じ導出。
const KEY_CODE_A4: u16 = 19;

/// rs(0-3) に応じた KSR(rate key scaling)の加算量。
fn ksr_add(rs: u8) -> u16 {
    let ksr_shift = 3u16.saturating_sub(rs.min(3) as u16);
    KEY_CODE_A4 >> ksr_shift
}

/// TX81Z RS(0-3、rate scaling)→ 38x6 ksr(0-255、実行時の音域依存レート倍率)。
///
/// エンジンの`ksr_rate_multiplier`は`exponent=ksr/255`の線形カーブ
/// （ksr=255で1オクターブごとに2倍、ksr=0で音域に依らず常に1.0）。
/// 実機の絶対keycode法則(eg_rate=2R+keycode>>(3-RS))はRS=0〜3で2倍刻み
/// (1x,2x,4x,8x)のため、本来はRS=0→exponent 0.125だが、過去の実機比較
/// （mucom2x6、MUCOM88実機C2/C6）で「RS=0は音域依存がほぼ無い方が近い」と
/// 判定されたため、RS=0のみ理論値でなく聴感を優先してksr=0(フラット)とする。
/// RS=2=128(exponent≈0.5)はGrandPianoキャリアの実機比較で検証済み、
/// RS=3=255(exponent=1.0)は理論値。
fn ks_to_ksr(rs: u8) -> u8 {
    const TABLE: [u8; 4] = [0, 64, 128, 255];
    TABLE[rs.min(3) as usize]
}

/// OPM型5-bitレート（AR/D1R/D2R, 0-31）→ 38x6 rate。
///
/// A4キーコードのKSR焼き込みをキャリア・モジュレーター双方に適用する。
/// 【2026-07-02改訂】旧実装はキャリアの焼き込みを廃止し、ノート依存KSRを実行時の
/// `ksr_rate_multiplier(rs, note)` のみに任せていたが、当時のエンジン側実装は
/// A4未満で倍率を1.0にクランプしており、低音キャリアのKSRが実質消失していた
/// （opzref実機忠実レンダリングとのEG減衰スロープ比較で発覚。GrandPiano D2の
/// キャリアで実機-13.5dB/sに対し-3.8dB/sと約3.5倍遅かった）。
/// クランプを撤去した（`ksr_rate_multiplier`側）ことで「A4焼き込み＋実行時倍率」が
/// 実機の絶対keycode法則（eg_rate=2R+keycode>>(3-KS)）と数学的に等価になったため、
/// キャリアの焼き込みも復活しモジュレーターと同じ扱いに戻す。
fn opm_rate_to_x6(rate: u8, rs: u8) -> u8 {
    if rate == 0 { return 0; }
    let eg_rate = (2 * rate as u16 + ksr_add(rs)).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

const ATTACK_ONSET_BIAS: u16 = 30;

fn ar_to_x6(ar: u8, rs: u8) -> u8 {
    if ar == 0 { return 0; }
    (opm_rate_to_x6(ar, rs) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPM型4-bitリリースレート（RR, 0-15）→ 38x6 rr。
/// KSR焼き込みの扱いは [opm_rate_to_x6] と同様（キャリア・モジュレーター共通）。
fn rr_to_x6(rr: u8, rs: u8) -> u8 {
    let eg_rate = (4 * rr as u16 + 2 + ksr_add(rs)).min(62);
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

fn convert_op(op: &crate::parse::OpzOpData, is_carrier: bool, alg_atten: u8, opts: ConvOptions) -> OperatorParams {
    let mod_tl_cap = opts.mod_tl_cap;
    let (mul, op_fine_tune) = freq_to_mul_fine(op.freq);

    // EGT=1 のとき D2R を強制的に高値にしてリリース挙動を作る
    // (TX81Z は EGT=1 で D1L で止まらず一定レートで減衰 = sustain-less decay)
    let d2r = if op.egt != 0 && op.d2r == 0 { 20 } else { op.d2r };

    let mut params = OperatorParams {
        tl: if is_carrier { out_to_tl(op.out, alg_atten) } else { out_to_tl_mod(op.out, mod_tl_cap) },
        ar: ar_to_x6(op.ar, op.rs),
        d1r: opm_rate_to_x6(op.d1r, op.rs),
        d2r: opm_rate_to_x6(d2r, op.rs),
        d1l: sl_to_x6(op.d1l, is_carrier),
        rr: rr_to_x6(op.rr, op.rs),
        mul,
        dt1: det_to_x6(op.det),
        ksr: opts.ksr_override.unwrap_or(ks_to_ksr(op.rs)),
        am_enable: op.ame,
        // キャリアは velocity_sensitivity=0（38x6の「velocity=音量」設計を維持）
        // モジュレーターは KVS を写像: KVS(0-7) → 0..168
        // 出典: nornandブログ「導出方法を考えてみた」のattKVS(kvs,velocity)導出式。
        // 弱打(velocity=1)→強打(velocity=127)のAkvs減衰スイングをkvs=1..7で計算すると
        // 厳密に `kvs*12`（TLレジスタ0-127スケール、0.75dB/step）になる。38x6のTLは同じ
        // 約95.25dB幅を255段階(0.373dB/step、ちょうど2倍解像度)で表すため換算係数は正確に2倍
        // → kvs*24。旧実装は255/7(≈36/step)を試して高velocity域でTLがクランプされすぎたため
        // *10に抑制していたが、今回は実機準拠の値として*24を採用（base_tlが高いモジュレーターは
        // 依然クランプの可能性があり、要聴感検証）。
        velocity_sensitivity: if is_carrier { 0 } else { op.kvs.min(7) * 24 },
        waveform: op.ow.min(7),
        op_fine_tune,
        floor: 0,
        loop_enabled: 0,
        curve: 0,
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
    /// モジュレーター TL 天井。既定は`Some(DEFAULT_MOD_TL_CAP)`。`None`にすると天井なし
    /// （実機のAol/Aalgをそのまま反映、診断用。ノイジーになることを確認済みなので通常は使わない）。
    pub mod_tl_cap: Option<u8>,
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
            mod_tl_cap: Some(DEFAULT_MOD_TL_CAP),
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
    // operators[0..3] は ymfm の m_op[0..3]（アルゴリズム上の O1/O2/O3/O4、O1が最深
    // モジュレーター兼フィードバック対象、O4がキャリア。YM2608マニュアル図2-3の
    // S1(FB)→S2→S3→S4=Cと一致）を意味する。
    //
    // OPZ(YM2414)は OPM系チップなので、レジスタ物理slot → m_op に slot1↔slot2 の
    // インターリーブが入る（ymfm `opz_registers::operator_map`：m_op=[slot0,slot2,slot1,slot3]）。
    // 一方 TX81Z の VMEM ファイルはバイトオフセット順で OP4(0-9)/OP2(10-19)/OP3(20-29)/OP1(30-39)
    // と書かれており（parse.rs参照）、これはバルクダンプがチップ内部レジスタをそのまま
    // 反映したものなので、物理slot0=OP4, slot1=OP2, slot2=OP3, slot3=OP1 と対応する。
    // よって m_op = [slot0, slot2, slot1, slot3] = [OP4, OP3, OP2, OP1] = ops[] そのもの
    // （恒等写像）。
    //
    // 旧実装は [ops[3], ops[1], ops[2], ops[0]]（OP1をoperators[0]=フィードバック対象へ）
    // だったが、これは「VMEMがOP1→slot0の素直な順で書かれる」という誤った前提に基づいていた。
    // TX81Z公式ドキュメント（fm_overview: 「OP4 modulates OP3, 3 modulates 2, 2 modulates 1」）・
    // NOZ氏の解説（OPP系はOPN/OPM系と結線順が逆）・ymfm OPZ参照(opzref)での実測波形
    // （旧写像は全サンプル±最大振幅で暴れる純ノイズ、恒等写像は滑らかな減衰包絡線）の
    // 三点で恒等写像が正しいことを確認した。
    const OP_SRC: [usize; 4] = [0, 1, 2, 3]; // operators[i] ← ops[OP_SRC[i]]（恒等写像）
    let atten = alg_atten(alg as u8);
    let operators = std::array::from_fn(|i| {
        let op = &voice.ops[OP_SRC[i]];
        let is_carrier = carriers.contains(&i);
        convert_op(op, is_carrier, atten, opts)
    });

    Ym38x6Patch {
        operators,
        channel: ChannelParams {
            algorithm: alg as u8,
            feedback: opts.fb_override.unwrap_or_else(|| fb_to_x6(voice.feedback)),
            chip_lfo_freq: (voice.lfo_spd as f32 * 255.0 / 99.0).round() as u8,
            chip_lfo_pmd: lfo_depth_to_x6(voice.pmd),
            chip_lfo_amd: lfo_depth_to_x6(voice.amd),
            chip_lfo_delay: (voice.lfo_dly as f32 * 255.0 / 99.0).round() as u8,
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
        assert_eq!(out_to_tl(0, 0), 0);    // 無音
        assert_eq!(out_to_tl(99, 0), 255); // 最大（キャリア用、Aalgなし）
        assert!(out_to_tl(50, 0) > 0 && out_to_tl(50, 0) < 255);
    }

    #[test]
    fn ol_to_atten_matches_reference_table() {
        // nornandブログ実測値の境界チェック（線形域と非線形域の境目・両端点）。
        assert_eq!(ol_to_atten(99), 0);
        assert_eq!(ol_to_atten(79), 20);
        assert_eq!(ol_to_atten(20), 79); // ここまでは1刻みの線形（Aol=99-OL）
        assert_eq!(ol_to_atten(19), 81); // ここから非線形域（2刻みに拡大）
        assert_eq!(ol_to_atten(0), 127);
    }

    #[test]
    fn conv_options_default_caps_modulator_tl() {
        // 2026-07-02: 天井なしはノイジーと判明したため、既定でDEFAULT_MOD_TL_CAP(180)を適用する。
        assert_eq!(ConvOptions::default().mod_tl_cap, Some(DEFAULT_MOD_TL_CAP));
        let op = OpzOpData { out: 99, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, 0, ConvOptions::default());
        assert_eq!(p.tl, DEFAULT_MOD_TL_CAP, "既定ではモジュレーターTLがDEFAULT_MOD_TL_CAPで頭打ちになるはず");
    }

    #[test]
    fn out_to_tl_mod_caps_at_given_cap() {
        assert_eq!(out_to_tl_mod(0, Some(200)), 0);
        assert_eq!(out_to_tl_mod(99, Some(200)), 200);
        assert!(out_to_tl_mod(50, Some(200)) > 0 && out_to_tl_mod(50, Some(200)) < 200);
        // 天井を変えると最大値も追従する
        assert_eq!(out_to_tl_mod(99, Some(254)), 254);
        assert_eq!(out_to_tl_mod(99, Some(180)), 180);
    }

    #[test]
    fn out_to_tl_mod_uncapped_matches_carrier_scaling() {
        // capがNone(既定)ならキャリア(Aalgなし)と同じout_to_tlになる（天井なし＝実機忠実）
        for out in [0u8, 50, 74, 77, 99] {
            assert_eq!(out_to_tl_mod(out, None), out_to_tl(out, 0));
        }
    }

    #[test]
    fn alg_atten_matches_reference_table() {
        // nornandブログの箇条書き（キャリア本数=1..4 → 減衰0/0/8/13/16）と
        // CARRIERSのキャリア本数が一致することを検証する。
        assert_eq!(alg_atten(0), 0); // アルゴリズム1(reg0, 単一キャリア)
        assert_eq!(alg_atten(1), 0);
        assert_eq!(alg_atten(2), 0);
        assert_eq!(alg_atten(3), 0);
        assert_eq!(alg_atten(4), 8);  // アルゴリズム5(reg4, 2キャリア)
        assert_eq!(alg_atten(5), 13); // アルゴリズム6(reg5, 3キャリア)
        assert_eq!(alg_atten(6), 13); // アルゴリズム7(reg6, 3キャリア)
        assert_eq!(alg_atten(7), 16); // アルゴリズム8(reg7, 4キャリア=全並列)
    }

    #[test]
    fn alg_atten_reduces_multi_carrier_carrier_tl() {
        // 4キャリア(alg7)ではAalg=16減衰が乗るぶん、単一キャリア(alg0)よりtlが小さくなる。
        let single = out_to_tl(99, alg_atten(0));
        let quad = out_to_tl(99, alg_atten(7));
        assert!(quad < single, "4キャリアのtl({quad})は単一キャリアのtl({single})より小さいはず");
    }

    #[test]
    fn is_carrier_matches_carriers_table() {
        assert!(is_carrier(0, 3) && !is_carrier(0, 0)); // alg0: キャリアはoperators[3]のみ
        assert!(is_carrier(4, 1) && is_carrier(4, 3) && !is_carrier(4, 0)); // alg4: キャリアは1,3
        assert!((0..4).all(|i| is_carrier(7, i))); // alg7: 全オペレーターがキャリア
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
    fn rate_ksr_baked_for_all_operators() {
        // D1R=16: rs(rate scaling)が大きいほどA4キーコードの焼き込み量が増え速くなる。
        // 【2026-07-02改訂】キャリア・モジュレーターを区別せず両方に焼き込む
        // （旧実装はキャリアのみ焼き込みを廃止していたが、実行時KSR倍率の
        // 低音クランプ撤去とセットで復活させた。opz2x6/conv.rs 冒頭コメント参照）。
        let rs0 = opm_rate_to_x6(16, 0);
        let rs3 = opm_rate_to_x6(16, 3);
        assert!(rs0 < rs3, "rsが大きいほど焼き込み量が増え速い(値が大きい): rs0={rs0} rs3={rs3}");
    }

    #[test]
    fn coarse_to_ratio_matches_reference_table() {
        // https://mgregory22.me/tx81z/freqratios.html の実測値(fine=0)との照合。
        // GrandPianoパッチ(Yamaha Factory Bank A voice0)で実際に使われている値を含む。
        assert!((coarse_to_ratio(0) - 0.50).abs() < 1e-3);
        assert!((coarse_to_ratio(4) - 1.00).abs() < 1e-3);
        assert!((coarse_to_ratio(8) - 2.00).abs() < 1e-3);
        assert!((coarse_to_ratio(13) - 4.00).abs() < 1e-3);
        assert!((coarse_to_ratio(22) - 7.00).abs() < 1e-3); // 旧実装は7.937(+13.4%誤差)だった
        assert!((coarse_to_ratio(25) - 8.00).abs() < 1e-3);
    }

    #[test]
    fn kvs_carrier_is_zero() {
        let op = OpzOpData { kvs: 7, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, 0, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 0, "carrier velocity_sensitivity must be 0");
    }

    #[test]
    fn kvs_modulator_maps_168_at_max() {
        // kvs*24（実機attKVS導出式から: 弱打→強打の減衰スイングkvs*12を38x6のTL解像度(2倍)へ換算）
        let op = OpzOpData { kvs: 7, freq: 4, det: 3, out: 50, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, 0, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 168);
    }

    #[test]
    fn kvs_modulator_zero_stays_zero() {
        let op = OpzOpData { kvs: 0, freq: 4, det: 3, out: 50, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, 0, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 0);
    }

    #[test]
    fn egt1_forces_d2r_nonzero() {
        let op = OpzOpData { d2r: 0, egt: 1, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, 0, ConvOptions::default());
        assert!(p.d2r > 0, "EGT=1 with D2R=0 should force d2r > 0");
    }

    #[test]
    fn egt0_d2r_zero_stays_zero() {
        let op = OpzOpData { d2r: 0, egt: 0, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, 0, ConvOptions::default());
        assert_eq!(p.d2r, 0, "EGT=0 D2R=0 → sustain型（d2r=0のまま）");
    }

    #[test]
    fn waveform_direct_copy() {
        let op = OpzOpData { ow: 5, freq: 4, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, 0, ConvOptions::default());
        assert_eq!(p.waveform, 5);
    }

    #[test]
    fn op_order_is_identity() {
        // ops[] = [OP4, OP3, OP2, OP1] がそのまま operators[0..3] になる
        // （OP4=最深モジュレーター兼フィードバック対象→operators[0]、OP1=キャリア→operators[3]）。
        let mut voice = OpzVoice::default();
        voice.ops[0] = OpzOpData { out: 80, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() }; // OP4
        voice.ops[3] = OpzOpData { out: 10, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() }; // OP1
        let patch = voice_to_patch(&voice);
        // operators[0] ← OP4 (out=80) → tl should be large
        assert!(patch.operators[0].tl > patch.operators[3].tl,
            "operators[0](OP4,out=80) should have higher tl than operators[3](OP1,out=10)");
    }

    #[test]
    fn op_order_no_interleave() {
        // ops[] = [OP4, OP3, OP2, OP1] の順序をそのまま保つ（恒等写像）ことを検証する。
        // ops[] に判別用 freq（→mul、いずれも coarse_to_ratio が整数比になる値）を仕込む。
        let mut voice = OpzVoice::default();
        voice.ops[0] = OpzOpData { freq: 8,  det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP4 → ratio2.0,mul2
        voice.ops[1] = OpzOpData { freq: 13, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP3 → ratio4.0,mul4
        voice.ops[2] = OpzOpData { freq: 22, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP2 → ratio7.0,mul7
        voice.ops[3] = OpzOpData { freq: 25, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() }; // OP1 → ratio8.0,mul8
        let p = voice_to_patch(&voice);
        assert_eq!(p.operators[0].mul, 2, "operators[0]=OP4");
        assert_eq!(p.operators[1].mul, 4, "operators[1]=OP3");
        assert_eq!(p.operators[2].mul, 7, "operators[2]=OP2");
        assert_eq!(p.operators[3].mul, 8, "operators[3]=OP1");
    }
}
