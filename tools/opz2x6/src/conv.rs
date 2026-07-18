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

/// モジュレーター TL 天井のオプトイン値（既定は天井なし、[out_to_tl_mod]参照）。
///
/// 2026-07-01は「実機録音とのRMSE比較で天井なしが最良」としてこの天井を既定から外したが、
/// 2026-07-02にGrandPiano等をopz2x6→38x6エンジン経由で試聴したところ天井なしがノイジーに感じられ、
/// 天井180を既定へ戻した経緯がある。
///
/// **【2026-07-18 再評価・既定を天井なしへ差し戻し】** この2026-07-02の聴感判定は、
/// D1L極性反転バグ（[[project_opz2x6_d1l_polarity_bug]]、2026-07-15修正）が混入した状態での
/// 比較だった疑いが濃厚（同バグは全音色のエンベロープ極性を反転させており、聴感比較の前提が
/// 汚染されていた）。修正後にopzref(ymfm実機参照)を基準として再測定したところ、天井なし
/// （[out_to_tl_mod]の`None`分岐）の方がA028 RichHarpsi・D023 FM Hi-Hats等で実機の明るさ・
/// ノイズ感に明確に近く（例: A028のスペクトル重心が実機の27%→89%に改善、D023のノイズ成分
/// (スペクトル平坦度)も0→実機の70%相当まで回復）、ユーザー試聴でも天井なしを支持する結果に
/// 反転した。この定数はオプトイン値（`--mod-cap 180`等）として残す。
pub const DEFAULT_MOD_TL_CAP: u8 = 180;

/// OUT → 38x6 TL（モジュレーター用）。`cap` が `Some` なら上限を設ける（`--mod-cap <N>`指定時、
/// オプトイン）。`None`（既定）だとキャリアと同じ `out_to_tl` を使う＝天井なし・実機のAol/Aalgを
/// そのまま反映する実機忠実な経路（[DEFAULT_MOD_TL_CAP]のコメント参照）。
fn out_to_tl_mod(out: u8, cap: Option<u8>) -> u8 {
    match cap {
        Some(cap) => {
            let aol = ol_to_atten(out);
            ((127 - aol.min(127)) as f32 / 127.0 * cap as f32).round() as u8
        }
        None => out_to_tl(out, 0),
    }
}

/// D1L/SL（TX81Zパネル値 4-bit 0-15）→ 38x6 D1L。
///
/// **【2026-07-15 極性修正】** sysex(VCED/VMEM)のD1Lは**パネル極性**（panel=15でフルサステイン、
/// panel=0で無音まで減衰）であり、OPMレジスタ極性（reg=15で-93dB）とは天地逆。
/// 旧実装はopm2x6のレジスタ用式をパネル値にそのまま流用しており全音色で反転していた
/// （ファクトリーバンクの持続系音色SynString/FrenchHorn/Alarm CallはすべてD1L=15、
/// 減衰系GrandPianoはD1L=0＝レジスタ極性だと弦・ホルンが即死する不合理な解釈になる）。
/// 冒頭で `reg = 15 - panel` に変換してから既存のレジスタ極性カーブに通す。
/// opzref(main.rs)のレジスタ直書きにも同じ反転が必要（オラクル汚染回避のため同時修正）。
///
/// reg（変換後）はサスティンレベルの減衰量（reg=0で0dB=減衰なし、reg=15で-93dB≈無音、3dB/step）。
///
/// **【2026-07-15 dB線形写像へ再統一】** 38x6エンジンのEGレベルは**dBリニア**
/// （operator.rs: `env_amp = 10^(-(1-level)*4.8)`、level=d1l/255でlevel 0.5=-48dB）なので、
/// dBを線形に0-255へ写す。旧実装はキャリアのみ「d1l=リニア振幅」という誤った前提で
/// `10^(db/20)*255` を返しており、-6dB保持のつもりの値がエンジン解釈で-48dB（ほぼ無音）に
/// なっていた（LiteHarpsiキャリアがプチノイズ化した直接原因）。
/// なお過去に一度dB線形へ修正して「聴感で不自然」と撤回した経緯があるが、
/// その聴感判断はD1L極性反転バグ（上記）がある状態で行われたもので汚染されていた。
/// 極性修正とセットで再統一する。キャリア/モジュレーターの区別は不要になった。
fn sl_to_x6(panel: u8) -> u8 {
    let reg = 15 - panel.min(15);
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    (255.0 * (1.0 + db / 93.0)).round() as u8
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

/// TX81Z FREQ (coarse 0-63, fine 0-15) → 周波数比率(ratio) 実測テーブル。index = `16*coarse + fine`。
///
/// **【2026-07-19 実測テーブルへ差し替え】** 旧実装はmgregory22のリバースエンジニアリング
/// （coarse値を4グループの線形係数へスクランブルし`ratio = base + step*(order_base + fine)`で
/// 近似する方式）を使っていたが、DXConvert（martintarenskeen、実績あるTX81Z⇄DX系sysex変換ツール）の
/// `freq_4op`テーブルと突き合わせたところ、fine扱いに大きな不整合があった:
/// - 平均15.6¢の誤差に加え、低coarse×高fineの隅で最大753.9¢（約6半音）の移調バグ。
///   実機テーブルは低coarseでfineが途中で頭打ち（例: coarse=0はfine=7の0.93で飽和し以降クランプ）
///   になるのに対し、線形近似はfineを際限なく加算し続けていた。
/// - 一方でopzref（fine完全無視）は平均199.6¢・最大1149¢で全域大外れだった。
///
/// DXConvertの`freq_4op`（`fourop.py`、コメント「4op frequencies (16*CRS+FINE)」）は
/// `16*coarse+fine`でインデックスする1024要素の実測テーブルで、TX81Z/DX11のfine分割挙動
/// （coarse間をfineが非線形に補間し、低coarseでは早期飽和する）をそのまま含む権威データ。
/// これを直接引くことで opz2x6/opzref 双方が実機のfine挙動に一致する。
/// fine≠0音色（A008 LoTine81Z等、carrierがcoarse=5/fine=1でratio≈1.49＝5度上に鳴る「移調」も
/// **実機仕様**）を正しく再現する。
///
/// 出典の`freq_4op`には非単調な2セル（coarse=38 fine=12の13.37、coarse=57 fine=10の21.48）が
/// あり、DXConvert側の転記/測定由来の誤記と思われるが、権威データへの忠実性を優先し原典のまま採録する
/// （いずれも高coarse域でファクトリー音色がまず踏まない）。
#[rustfmt::skip]
const FREQ_4OP: [f32; 1024] = [
    0.50, 0.56, 0.62, 0.68, 0.75, 0.81, 0.87, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, // coarse=0
    0.71, 0.79, 0.88, 0.96, 1.05, 1.14, 1.23, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, // coarse=1
    0.78, 0.88, 0.98, 1.07, 1.17, 1.27, 1.37, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, // coarse=2
    0.87, 0.97, 1.08, 1.18, 1.29, 1.40, 1.51, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, // coarse=3
    1.00, 1.06, 1.12, 1.18, 1.25, 1.31, 1.37, 1.43, 1.50, 1.56, 1.62, 1.68, 1.75, 1.81, 1.87, 1.93, // coarse=4
    1.41, 1.49, 1.58, 1.67, 1.76, 1.85, 1.93, 2.02, 2.11, 2.20, 2.29, 2.37, 2.46, 2.55, 2.64, 2.73, // coarse=5
    1.57, 1.66, 1.76, 1.86, 1.96, 2.06, 2.15, 2.25, 2.35, 2.45, 2.55, 2.64, 2.74, 2.84, 2.94, 3.04, // coarse=6
    1.73, 1.83, 1.94, 2.05, 2.16, 2.27, 2.37, 2.48, 2.59, 2.70, 2.81, 2.91, 3.02, 3.13, 3.24, 3.35, // coarse=7
    2.00, 2.06, 2.12, 2.18, 2.25, 2.31, 2.37, 2.43, 2.50, 2.56, 2.62, 2.68, 2.75, 2.81, 2.87, 2.93, // coarse=8
    2.82, 2.90, 2.99, 3.08, 3.17, 3.26, 3.34, 3.43, 3.52, 3.61, 3.70, 3.78, 3.87, 3.96, 4.05, 4.14, // coarse=9
    3.00, 3.06, 3.12, 3.18, 3.25, 3.31, 3.37, 3.43, 3.50, 3.56, 3.62, 3.68, 3.75, 3.81, 3.87, 3.93, // coarse=10
    3.14, 3.23, 3.33, 3.43, 3.53, 3.63, 3.72, 3.82, 3.92, 4.02, 4.12, 4.21, 4.31, 4.41, 4.51, 4.61, // coarse=11
    3.46, 3.56, 3.67, 3.78, 3.89, 4.00, 4.10, 4.21, 4.32, 4.43, 4.54, 4.64, 4.75, 4.86, 4.97, 5.08, // coarse=12
    4.00, 4.06, 4.12, 4.18, 4.25, 4.31, 4.37, 4.43, 4.50, 4.56, 4.62, 4.68, 4.75, 4.81, 4.87, 4.93, // coarse=13
    4.24, 4.31, 4.40, 4.49, 4.58, 4.67, 4.75, 4.84, 4.93, 5.02, 5.11, 5.19, 5.28, 5.37, 5.46, 5.55, // coarse=14
    4.71, 4.80, 4.90, 5.00, 5.10, 5.20, 5.29, 5.39, 5.49, 5.59, 5.69, 5.78, 5.88, 5.98, 6.08, 6.18, // coarse=15
    5.00, 5.06, 5.12, 5.18, 5.25, 5.31, 5.37, 5.43, 5.50, 5.56, 5.62, 5.68, 5.75, 5.81, 5.87, 5.93, // coarse=16
    5.19, 5.29, 5.40, 5.51, 5.62, 5.73, 5.83, 5.94, 6.05, 6.16, 6.27, 6.37, 6.48, 6.59, 6.70, 6.81, // coarse=17
    5.65, 5.72, 5.81, 5.90, 5.99, 6.08, 6.16, 6.25, 6.34, 6.43, 6.52, 6.60, 6.69, 6.78, 6.87, 6.96, // coarse=18
    6.00, 6.06, 6.12, 6.18, 6.25, 6.31, 6.37, 6.43, 6.50, 6.56, 6.62, 6.68, 6.75, 6.81, 6.87, 6.93, // coarse=19
    6.28, 6.37, 6.47, 6.57, 6.67, 6.77, 6.86, 6.96, 7.06, 7.16, 7.26, 7.35, 7.45, 7.55, 7.65, 7.75, // coarse=20
    6.92, 7.02, 7.13, 7.24, 7.35, 7.46, 7.56, 7.67, 7.78, 7.89, 8.00, 8.10, 8.21, 8.32, 8.43, 8.54, // coarse=21
    7.00, 7.06, 7.12, 7.18, 7.25, 7.31, 7.37, 7.43, 7.50, 7.56, 7.62, 7.68, 7.75, 7.81, 7.87, 7.93, // coarse=22
    7.07, 7.13, 7.22, 7.31, 7.40, 7.49, 7.57, 7.66, 7.75, 7.84, 7.93, 8.01, 8.10, 8.19, 8.28, 8.37, // coarse=23
    7.85, 7.94, 8.04, 8.14, 8.24, 8.34, 8.43, 8.53, 8.63, 8.73, 8.83, 8.92, 9.02, 9.12, 9.22, 9.32, // coarse=24
    8.00, 8.06, 8.12, 8.18, 8.25, 8.31, 8.37, 8.43, 8.50, 8.56, 8.62, 8.68, 8.75, 8.81, 8.87, 8.93, // coarse=25
    8.48, 8.54, 8.63, 8.72, 8.81, 8.90, 8.98, 9.07, 9.16, 9.25, 9.34, 9.42, 9.51, 9.60, 9.69, 9.78, // coarse=26
    8.65, 8.75, 8.86, 8.97, 9.08, 9.19, 9.29, 9.40, 9.51, 9.62, 9.73, 9.83, 9.94, 10.05, 10.16, 10.27, // coarse=27
    9.00, 9.06, 9.12, 9.18, 9.25, 9.31, 9.37, 9.43, 9.50, 9.56, 9.62, 9.68, 9.75, 9.81, 9.87, 9.93, // coarse=28
    9.42, 9.51, 9.61, 9.71, 9.81, 9.91, 10.00, 10.10, 10.20, 10.30, 10.40, 10.49, 10.59, 10.69, 10.79, 10.89, // coarse=29
    9.89, 9.95, 10.04, 10.13, 10.22, 10.31, 10.39, 10.48, 10.57, 10.66, 10.75, 10.83, 10.92, 11.01, 11.10, 11.19, // coarse=30
    10.00, 10.06, 10.12, 10.18, 10.25, 10.31, 10.37, 10.43, 10.50, 10.56, 10.62, 10.68, 10.75, 10.81, 10.87, 10.93, // coarse=31
    10.38, 10.48, 10.59, 10.70, 10.81, 10.92, 11.02, 11.13, 11.24, 11.35, 11.46, 11.56, 11.67, 11.78, 11.89, 12.00, // coarse=32
    10.99, 11.08, 11.18, 11.28, 11.38, 11.48, 11.57, 11.67, 11.77, 11.87, 11.97, 12.06, 12.16, 12.26, 12.36, 12.46, // coarse=33
    11.00, 11.06, 11.12, 11.18, 11.25, 11.31, 11.37, 11.43, 11.50, 11.56, 11.62, 11.68, 11.75, 11.81, 11.87, 11.93, // coarse=34
    11.30, 11.36, 11.45, 11.54, 11.63, 11.72, 11.80, 11.89, 11.98, 12.07, 12.16, 12.24, 12.33, 12.42, 12.51, 12.60, // coarse=35
    12.00, 12.06, 12.12, 12.18, 12.25, 12.31, 12.37, 12.43, 12.50, 12.56, 12.62, 12.68, 12.75, 12.81, 12.87, 12.93, // coarse=36
    12.11, 12.21, 12.32, 12.43, 12.54, 12.65, 12.75, 12.86, 12.97, 13.08, 13.19, 13.29, 13.40, 13.51, 13.62, 13.73, // coarse=37
    12.56, 12.65, 12.75, 12.85, 12.95, 13.05, 13.14, 13.24, 13.34, 13.44, 13.54, 13.63, 13.37, 13.83, 13.93, 14.03, // coarse=38 (fine=12の13.37は原典の非単調誤記)
    12.72, 12.77, 12.86, 12.95, 13.04, 13.13, 13.21, 13.30, 13.39, 13.48, 13.57, 13.65, 13.74, 13.83, 13.92, 14.01, // coarse=39
    13.00, 13.06, 13.12, 13.18, 13.25, 13.31, 13.37, 13.43, 13.50, 13.56, 13.62, 13.68, 13.75, 13.81, 13.87, 13.93, // coarse=40
    13.84, 13.94, 14.05, 14.16, 14.27, 14.38, 14.48, 14.59, 14.70, 14.81, 14.92, 15.02, 15.13, 15.24, 15.35, 15.46, // coarse=41
    14.00, 14.06, 14.12, 14.18, 14.25, 14.31, 14.37, 14.43, 14.50, 14.56, 14.62, 14.68, 14.75, 14.81, 14.87, 14.93, // coarse=42
    14.10, 14.18, 14.27, 14.36, 14.45, 14.54, 14.62, 14.71, 14.80, 14.89, 14.98, 15.06, 15.15, 15.24, 15.33, 15.42, // coarse=43
    14.13, 14.22, 14.32, 14.42, 14.52, 14.62, 14.71, 14.81, 14.91, 15.01, 15.11, 15.20, 15.30, 15.40, 15.50, 15.60, // coarse=44
    15.00, 15.06, 15.12, 15.18, 15.25, 15.31, 15.37, 15.43, 15.50, 15.56, 15.62, 15.68, 15.75, 15.81, 15.87, 15.93, // coarse=45
    15.55, 15.59, 15.68, 15.77, 15.86, 15.95, 16.03, 16.12, 16.21, 16.30, 16.39, 16.47, 16.56, 16.65, 16.74, 16.83, // coarse=46
    15.57, 15.67, 15.78, 15.89, 16.00, 16.11, 16.21, 16.32, 16.43, 16.54, 16.65, 16.75, 16.86, 16.97, 17.08, 17.19, // coarse=47
    15.70, 15.79, 15.89, 15.99, 16.09, 16.19, 16.28, 16.38, 16.48, 16.58, 16.68, 16.77, 16.87, 16.97, 17.07, 17.17, // coarse=48
    16.96, 17.00, 17.09, 17.18, 17.27, 17.36, 17.44, 17.53, 17.62, 17.71, 17.80, 17.88, 17.97, 18.06, 18.15, 18.24, // coarse=49
    17.27, 17.36, 17.46, 17.56, 17.66, 17.76, 17.85, 17.95, 18.05, 18.15, 18.25, 18.35, 18.44, 18.54, 18.64, 18.74, // coarse=50
    17.30, 17.40, 17.51, 17.62, 17.73, 17.84, 17.94, 18.05, 18.16, 18.27, 18.38, 18.48, 18.59, 18.70, 18.81, 18.92, // coarse=51
    18.37, 18.41, 18.50, 18.59, 18.68, 18.77, 18.85, 18.94, 19.03, 19.12, 19.21, 19.29, 19.38, 19.47, 19.56, 19.65, // coarse=52
    18.84, 18.93, 19.03, 19.13, 19.23, 19.33, 19.42, 19.52, 19.62, 19.72, 19.82, 19.91, 20.01, 20.11, 20.21, 20.31, // coarse=53
    19.03, 19.13, 19.24, 19.35, 19.46, 19.57, 19.67, 19.78, 19.89, 20.00, 20.11, 20.21, 20.32, 20.43, 20.54, 20.65, // coarse=54
    19.78, 19.82, 19.91, 20.00, 20.09, 20.18, 20.26, 20.35, 20.44, 20.53, 20.62, 20.70, 20.79, 20.88, 20.97, 21.06, // coarse=55
    20.41, 20.50, 20.60, 20.70, 20.80, 20.90, 20.99, 21.09, 21.19, 21.29, 21.39, 21.48, 21.58, 21.68, 21.78, 21.88, // coarse=56
    20.76, 20.86, 20.97, 21.08, 21.19, 21.30, 21.40, 21.51, 21.62, 21.73, 21.48, 21.94, 22.05, 22.16, 22.27, 22.38, // coarse=57 (fine=10の21.48は原典の非単調誤記)
    21.20, 21.23, 21.32, 21.41, 21.50, 21.59, 21.67, 21.76, 21.85, 21.94, 22.03, 22.11, 22.20, 22.29, 22.38, 22.47, // coarse=58
    21.98, 22.07, 22.17, 22.27, 22.37, 22.47, 22.56, 22.66, 22.76, 22.86, 22.96, 23.05, 23.15, 23.25, 23.35, 23.45, // coarse=59
    22.49, 22.59, 22.70, 22.81, 22.92, 23.03, 23.13, 23.24, 23.35, 23.46, 23.57, 23.67, 23.78, 23.89, 24.00, 24.11, // coarse=60
    23.55, 23.64, 23.74, 23.84, 23.94, 24.04, 24.13, 24.23, 24.33, 24.43, 24.53, 24.62, 24.72, 24.82, 24.92, 25.02, // coarse=61
    24.22, 24.32, 24.43, 24.54, 24.65, 24.76, 24.86, 24.97, 25.08, 25.19, 25.30, 25.40, 25.51, 25.62, 25.73, 25.84, // coarse=62
    25.95, 26.05, 26.16, 26.27, 26.38, 26.49, 26.59, 26.70, 26.81, 26.92, 27.03, 27.13, 27.24, 27.35, 27.46, 27.57, // coarse=63
];

/// TX81Z FREQ coarse(0-63) → 周波数比率(ratio)。opz2x6/opzref共通で使う変換の核（fine=0固定）。
pub fn coarse_to_ratio(coarse: u8) -> f32 {
    coarse_fine_to_ratio(coarse, 0)
}

/// TX81Z FREQ coarse(0-63) + fine(0-15) → 周波数比率(ratio)。
/// DXConvert実測テーブル[FREQ_4OP]を`16*coarse+fine`で引く。
pub fn coarse_fine_to_ratio(coarse: u8, fine: u8) -> f32 {
    FREQ_4OP[16 * coarse.min(63) as usize + fine.min(15) as usize]
}

/// TX81Z FREQ (0-63) + fine(0-15) → 38x6 (MUL 0-15, op_fine_tune 0-255)。
/// 最近傍の整数 MUL を選び、差分セントを op_fine_tune に写像する。
pub fn freq_to_mul_fine(freq: u8, fine: u8) -> (u8, u8) {
    let ratio = coarse_fine_to_ratio(freq, fine);

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
    let (mul, op_fine_tune) = freq_to_mul_fine(op.freq, op.fine);

    // EGT=1 のとき D2R を強制的に高値にしてリリース挙動を作る
    // (TX81Z は EGT=1 で D1L で止まらず一定レートで減衰 = sustain-less decay)
    let d2r = if op.egt != 0 && op.d2r == 0 { 20 } else { op.d2r };

    // KVS写像（モジュレーターのみ、キャリアは0=velocity一本化）: KVS(0-7) → sens = kvs*24
    // （nornand attKVS導出のvelocityスイングkvs*12レジスタstep × 38x6の2倍解像度）。
    let vel_sens = if is_carrier { 0 } else { op.kvs.min(7) * 24 };

    let mut params = OperatorParams {
        // 【2026-07-18 KVSアンカー修正】実機のKVSはOL額面からの追加減衰
        // （V_TL = … + A_ol + A_kvs、A_kvsはvelocity=最大で0）なのに対し、38x6エンジンの
        // effective_tl は base_tl + (velocity/127)*sens の上乗せ方向。旧実装はモジュレーターの
        // base_tl にOL額面をそのまま入れており、velocity>0 で常に実機より sens 分（kvs=4で
        // 最大+96step≈+36dB）過大な変調になっていた（mod_cap撤廃でBank Aピアノ系が
        // ノイズ化した回帰の真因。β≈25radの過変調でFMサイドバンドがナイキストを超え折り返す）。
        // velocity=127 でOL額面に一致するよう base = 額面 - sens にアンカーする。
        tl: if is_carrier {
            out_to_tl(op.out, alg_atten)
        } else {
            out_to_tl_mod(op.out, mod_tl_cap).saturating_sub(vel_sens)
        },
        ar: ar_to_x6(op.ar, op.rs),
        d1r: opm_rate_to_x6(op.d1r, op.rs),
        d2r: opm_rate_to_x6(d2r, op.rs),
        d1l: sl_to_x6(op.d1l),
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
        // → kvs*24。base_tl側を額面-sensへアンカーする理由は上記tlフィールドのコメント参照。
        velocity_sensitivity: vel_sens,
        waveform: op.ow.min(7),
        op_fine_tune,
        floor: 0,
        loop_enabled: 0,
        curve: 0,
        // EGSFT(0-3、96/48/24/12dB)を0-255連続へ写像（0/85/170/255）。
        // OP1は実機で常にoff固定という制約があるが、38x6は忠実エミュを追わない方針
        // (project_38x6_identity_and_layering)のためコンバーター側で吸収せず、
        // パース値をそのまま写像する。
        eg_shift: op.egsft.min(3) * 85,
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
    /// モジュレーター TL 天井。既定は`None`（天井なし＝実機のAol/Aalgをそのまま反映、実機忠実）。
    /// `Some(n)`（`--mod-cap <n>`、例: [DEFAULT_MOD_TL_CAP]=180）で変調を抑えたい場合のオプトイン。
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
            mod_tl_cap: None,
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
    fn conv_options_default_has_no_modulator_cap() {
        // 2026-07-18: D1L極性バグ修正後にopzref基準で再評価し、天井なし(None)を既定へ戻した
        // （[DEFAULT_MOD_TL_CAP]のコメント参照）。180は`--mod-cap`のオプトイン値として残る。
        assert_eq!(ConvOptions::default().mod_tl_cap, None);
        let op = OpzOpData { out: 99, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, 0, ConvOptions::default());
        assert_eq!(p.tl, out_to_tl(99, 0), "既定ではモジュレーターTLがキャリアと同じ天井なしカーブになるはず");
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
        let (m, oft) = freq_to_mul_fine(4, 0);
        assert_eq!(m, 1);
        assert_eq!(oft, 128);

        let (m, oft) = freq_to_mul_fine(8, 0);
        assert_eq!(m, 2);
        assert_eq!(oft, 128);

        let (m, oft) = freq_to_mul_fine(0, 0); // 0.5x
        assert_eq!(m, 0);
        assert_eq!(oft, 128);
    }

    #[test]
    fn freq_to_mul_fine_nonzero_fine_shifts_op_fine_tune() {
        // FREQ=4(ratio 1.0, group0, order_base=8, step=0.0625)。
        // fine=4 → order=12 → ratio=0.5+0.0625*12=1.25 → 対数距離ではMUL=1が最近傍のまま
        // (log2(1.25/1)≈0.322 < log2(1.25/2)の絶対値≈0.678)。
        // 差分 log2(1.25)*1200≈386¢分、op_fine_tuneがfine=0時より上昇するはず。
        let (m0, oft0) = freq_to_mul_fine(4, 0);
        let (m4, oft4) = freq_to_mul_fine(4, 4);
        assert_eq!(m0, 1);
        assert_eq!(m4, 1);
        assert!(oft4 > oft0, "fine増加で比率が上がりop_fine_tuneも増えるはず: oft0={oft0} oft4={oft4}");
    }

    #[test]
    fn sl_to_x6_panel_polarity_db_linear() {
        // 【2026-07-15 極性修正+dB線形統一】引数はパネル極性: panel=15がフルサステイン、
        // panel=0が無音まで減衰。ファクトリーバンクの持続系音色(SynString/FrenchHorn/Alarm Call)は
        // D1L=15、減衰系(GrandPiano)はD1L=0 — この向きが正。
        assert_eq!(sl_to_x6(15), 255); // フルサステイン(reg=0, 0dB)
        assert_eq!(sl_to_x6(0), 0);    // 無音まで減衰(reg=15, -93dB)
        // dB線形写像: panel=2(reg=13, -39dB) → 255*(1-39/93) = 148
        assert_eq!(sl_to_x6(2), 148);
        // LiteHarpsiキャリア回帰: panel=13(reg=2, -6dB) → 255*(1-6/93) = 239。
        // エンジンのdBリニアEG解釈で-(1-239/255)*96 ≈ -6dB(振幅0.5)となり正しく保持される。
        // 旧リニア振幅写像は128を返し、エンジン解釈で-48dB(ほぼ無音)=プチノイズ化していた。
        assert_eq!(sl_to_x6(13), 239);
        // panel>15の防御(clamp): 16は15と同じ
        assert_eq!(sl_to_x6(16), sl_to_x6(15));
    }

    #[test]
    fn sl_to_x6_sustain_idiom_survives() {
        // Alarm Call回帰: D1R=31 + D1L=15(パネル)は4op機の定番「ディケイスキップ=純サステイン」
        // イディオム。旧極性では「最速で-93dBへ」に化けて9msのプチノイズになっていた。
        let op = OpzOpData { d1l: 15, d1r: 31, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, 0, ConvOptions::default());
        assert_eq!(p.d1l, 255, "パネルD1L=15はフルサステインに写像されるべき");
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
        // DXConvert freq_4op 実測テーブル(fine=0)との照合。
        // GrandPianoパッチ(Yamaha Factory Bank A voice0)で実際に使われている値を含む。
        assert!((coarse_to_ratio(0) - 0.50).abs() < 1e-3);
        assert!((coarse_to_ratio(4) - 1.00).abs() < 1e-3);
        assert!((coarse_to_ratio(8) - 2.00).abs() < 1e-3);
        assert!((coarse_to_ratio(13) - 4.00).abs() < 1e-3);
        assert!((coarse_to_ratio(22) - 7.00).abs() < 1e-3); // 旧近似は7.937(+13.4%誤差)だった
        assert!((coarse_to_ratio(25) - 8.00).abs() < 1e-3);
    }

    #[test]
    fn coarse_fine_to_ratio_uses_measured_table() {
        // DXConvert freq_4op を 16*coarse+fine で引く。
        // coarse=4: 1.00, 1.06, 1.12, ... （fineでcoarse間を非線形補間）
        assert_eq!(coarse_fine_to_ratio(4, 0), 1.00);
        assert_eq!(coarse_fine_to_ratio(4, 1), 1.06);
        assert_eq!(coarse_fine_to_ratio(4, 15), 1.93);
        // A008 LoTine81Zのcarrier: coarse=5,fine=1 → 1.49(≈5度上)。これは実機仕様。
        assert_eq!(coarse_fine_to_ratio(5, 1), 1.49);
        // 低coarseはfineが早期飽和する（旧線形近似がここで最大753.9¢移調していた）:
        // coarse=0はfine=7の0.93以降クランプ。
        assert_eq!(coarse_fine_to_ratio(0, 7), 0.93);
        assert_eq!(coarse_fine_to_ratio(0, 15), 0.93);
        // fine>15はclampされ、16も15と同じ結果になる
        assert_eq!(coarse_fine_to_ratio(4, 16), coarse_fine_to_ratio(4, 15));
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
