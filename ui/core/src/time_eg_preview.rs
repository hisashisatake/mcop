// ---------------------------------------------------------------------------
// TimeEg形状プレビュー（OP505のOP1〜4/PITCH FG/CUTOFF FG/GAIN FG各パネル左側の折れ線グラフ）
//
// `eg_preview.rs`（レート方式5段EG用）の姉妹モジュール。縦軸dB・横軸時間・固定スケール・
// ターゲット方式・レイズドコサイン整形といった設計原則はそのまま踏襲し、色定数・ベゼル/液晶背景・
// dB変換式（tl_to_db/level_contribution_db/DB_FLOOR）・曲線描画（draw_ramp）は
// `pub(crate)`昇格したeg_preview.rsの実装をそのまま共有する（同一crate内）。
//
// TimeEgとレート方式EGの構造上の違い（流用できない理由）:
// - フェーズ固定4本(AR/D1R/D2R/RR)ではなく、1〜`MAX_STAGES`段の可変長リスト（`TimeEgParams::stage_count`）
// - 各段が`level`を独立に持つ（レート方式は「AR=常にTLへ完全到達」という前提があったが、
//   TimeEgでは段0から既に任意のlevelを持てる）
// - フリーズ特殊値が無い（`time=0`は「瞬時」でレート方式の`rate=0`＝フリーズとは意味が真逆。
//   `log_width`に`time_to_seconds(0)=0.0`を通すと自然に幅0になるため、レート方式のような
//   `rate_seconds_or_frozen`分岐・破線描画は不要）
// - ループは`loop_start`〜`release_point`の任意区間（レート方式はAR/D1Rの2区間固定だった）
// - リリースは`release_point+1`から`stage_count-1`まで順に辿る多段リリース（レート方式は
//   RR1本のみで必ず無音へ着地したが、TimeEgのリリースは終着レベルが任意——例えばGain FGの
//   `rr=0`透過既定を変換したパッチはlevel=255で据え置く——なので、必ず床へ落ちるとは限らない）
//
// 実際の音（`sound_core::TimeEg::tick`/`advance`の状態機械）を`time_eg_layout`で忠実に
// 再現する：note-onは常にlevel=0.0から段0へ向かい、段を`0,1,...,release_point`と辿る。
// release_point到達時`loop_enabled`なら`loop_start`へ戻る（本プレビューでは2周描いて
// 「ここが繰り返す」ことを示す）、でなければその段のレベルで静止（サステイン）。
// note-offは`release_point+1`段へ入り（その時点の現在レベルから）、以降`stage_count-1`まで
// 中間段を飛ばさず順に辿って終わる。
//
// 保持区間`0..=release_point`とリリース区間`release_point+1..stage_count`は段リストを
// 過不足なく分割するため、重複（同じ段が二重に描かれクリック点が段数+1になる）も
// 隙間（どの区間にも属さない到達不能段）も構造的に発生しない。
// ---------------------------------------------------------------------------

use egui::{Pos2, Rect, Shape, Stroke, Ui, Vec2};
use sound_core::time_eg::time_to_seconds;
use sound_core::{TimeEgParams, MAX_STAGES};

use crate::eg_preview::{
    draw_ramp, level_contribution_db, tl_to_db, EgAmplitudeMapping, COLOR_BEZEL, COLOR_HELD, COLOR_PANEL, COLOR_RELEASE, DB_FLOOR,
};

/// `<style>`省略時の既定サイズ。TimeEgは最大`MAX_STAGES`段(保持側最大`MAX_STAGES*2`セグメント+リリース最大`MAX_STAGES`セグメント)を
/// 描くため、4フェーズ固定のレート方式版（84×66）より横に広く取る。
const DEFAULT_WIDTH: f32 = 130.0;
const DEFAULT_HEIGHT: f32 = 66.0;
const PAD: f32 = 6.0;
pub(crate) const DOT_RADIUS: f32 = 2.5;
const COLOR_DOT: egui::Color32 = egui::Color32::from_gray(200);
/// points[0](note-on開始点、常にlevel=0固定でドラッグ対象外)専用の色。他の頂点と同じ塗り丸だと
/// 「動かせそう」に見えてしまう(実機確認で判明)ため、輪郭のみの控えめな丸で区別する。
const COLOR_DOT_START: egui::Color32 = egui::Color32::from_gray(110);

/// `time_to_seconds`が写す秒数レンジ（`sound_core::time_eg`のT_MIN/T_MAXと一致させること）。
/// `pub(crate)`昇格は`time_eg_editor`（Step 8、同一crate内）が`width_to_time`の逆写像で
/// `log_width`と同じレンジを使う必要があるため。
pub(crate) const TIME_MIN_SECONDS: f32 = 0.001;
pub(crate) const TIME_MAX_SECONDS: f32 = 300.0;

fn clamp_stage_count(stage_count: u8) -> usize {
    (stage_count as usize).clamp(1, MAX_STAGES)
}

/// フェーズの実測秒数を横幅の重み(0.0〜1.0)へ変換する。`eg_preview::log_width`と同じ対数正規化
/// （`time=0`→0.0秒→重み0＝垂直＝瞬時、`time=255`→300秒→重み1.0＝最大幅）。
/// `pub(crate)`昇格せず複製するのは、フェーズ固有レンジ(AR/D1R等)を持つeg_preview版と違い
/// TimeEgは全段が同一レンジ(TIME_MIN_SECONDS〜TIME_MAX_SECONDS)を使うため、シグネチャが異なるため。
fn log_width(seconds: f32) -> f32 {
    let lo = TIME_MIN_SECONDS.log10();
    let hi = TIME_MAX_SECONDS.log10();
    ((seconds.max(TIME_MIN_SECONDS).log10() - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// パネル種別ごとのグラフY軸の床(dB)。`DbLinear`/`RawLinear`は式が`floor*(1-level)`という
/// floorに対する線形不変な形（floorをどう選んでも高さの割合は`level`に厳密比例する）なので、
/// OP(DbLinear)の物理値である`DB_FLOOR`(operator.rsの`env_amp`式由来、変更禁止)のまま据え置く。
/// `AmplitudeLinear`(GAIN FG)は真の対数（`20*log10(level)`）で、`DB_FLOOR`(-96)のままだと
/// レベル1〜255が高さの上半分に圧縮され下半分をどれだけドラッグしても無反応という操作性問題が
/// 実機確認で判明したため、床を浅くして実用域(level概ね16〜255)を高さ全体に広げる
/// （代償として極小レベル1〜15程度は床に張り付いて区別できなくなるが、GAIN FGの本命ユースケース
/// である「静止を挟んだ2値スイッチ」では実用上ほぼ支障がない）。
const AMPLITUDE_LINEAR_AXIS_FLOOR: f32 = -24.0;

pub(crate) fn axis_floor_db(mapping: EgAmplitudeMapping) -> f32 {
    match mapping {
        EgAmplitudeMapping::AmplitudeLinear => AMPLITUDE_LINEAR_AXIS_FLOOR,
        EgAmplitudeMapping::DbLinear | EgAmplitudeMapping::RawLinear => DB_FLOOR,
    }
}

/// キーオン起点および1段ループの跳ね戻し先に使う「無変調」レベル（生値）。
///
/// `sound_core::TimeEg`の`neutral_level`と必ず一致させること（`TimeEg::new_bipolar()`は
/// `BIPOLAR_NEUTRAL_RAW`、`TimeEg::new()`は0）。ずれるとグラフの開始点だけが実際の音と
/// 食い違い、「一番下から始まっているように見えるのに音は中央から始まる」という形で表面化する。
pub(crate) fn neutral_start_level(mapping: EgAmplitudeMapping) -> u8 {
    match mapping {
        // Pitch FG／Cutoff FG：レベルはバイポーラで、中央128が無変調。
        EgAmplitudeMapping::RawLinear => sound_core::BIPOLAR_NEUTRAL_RAW,
        // 振幅系（OP EG／VCA／FILTER）：0が無音＝無変調。
        EgAmplitudeMapping::DbLinear | EgAmplitudeMapping::AmplitudeLinear => 0,
    }
}

/// STAGES=0（FG無効化、`TimeEgProfile::min_stages=0`のときのみ到達）のグラフが描く
/// 「無効時の水平線」の高さ。**`neutral_start_level`を流用してはいけない**——あれは
/// `AmplitudeLinear`（Gain FG）に0（無音）を返すが、Gain FG無効時のエンジンの実挙動は
/// `gain_fg_out=1.0`（透過、`op505-core::Voice::tick`参照）であり0.0ではない。
/// ここを取り違えると「無効化したら無音になった」というグラフと実音の食い違いが起きる。
pub(crate) fn disabled_level(mapping: EgAmplitudeMapping) -> u8 {
    match mapping {
        // Pitch FG／Cutoff FG：無効時は変調量ゼロ＝バイポーラ中央128。
        EgAmplitudeMapping::RawLinear => sound_core::BIPOLAR_NEUTRAL_RAW,
        // Gain FG：無効時は×1.0の透過。振幅系のフルレベル255がそれに当たる。
        EgAmplitudeMapping::AmplitudeLinear => 255,
        // OP EG（DbLinear）はmin_stages>=2固定でSTAGES=0に到達しない。
        EgAmplitudeMapping::DbLinear => 0,
    }
}

/// TL(0〜255)とTimeEgの1段(level)からターゲットdBを求める。`level_contribution_db`は
/// 「TLからの相対減衰」を返すため、`tl_db`に加算し`floor`（`axis_floor_db`参照）で下限クランプする
/// （`eg_preview::eg_preview`のTL→SL算出と同じ式。TimeEgは全段が同じ式で求まる——
/// レート方式のように「AR到達点=常にTL」という特別扱いが要らない）。
/// `AmplitudeLinear`だけは`level_contribution_db`（内部floorが`DB_FLOOR`固定）を経由せず、
/// パネル固有の`floor`で直接クランプする（`DbLinear`/`RawLinear`はfloor不変なので共通実装のまま）。
pub(crate) fn stage_target_db(mapping: EgAmplitudeMapping, floor: f32, tl_db: f32, level: u8) -> f32 {
    let level_linear = level as f32 / 255.0;
    let contribution = match mapping {
        EgAmplitudeMapping::AmplitudeLinear => {
            if level_linear <= 0.0 {
                floor
            } else {
                (20.0 * level_linear.log10()).max(floor)
            }
        }
        EgAmplitudeMapping::DbLinear | EgAmplitudeMapping::RawLinear => level_contribution_db(mapping, level_linear),
    };
    (tl_db + contribution).max(floor)
}

/// `time_eg_preview`が計算した折れ線の幾何情報。egui非依存の純粋計算なので単体テスト可能。
/// 将来のドラッグ編集は、この構造体に「画面座標→time/level」の逆写像を足すだけで実装できる
/// （`points`/`stage_of_point`で「どの頂点がどの段に対応するか」が既に分かっているため）。
pub struct TimeEgGeometry {
    /// 描画順の頂点列。`points[0]`はnote-on直後のlevel=0開始点（どの段にも対応しない）。
    pub points: Vec<Pos2>,
    /// `points[i]`が対応する段のindex。`points[0]`（開始点）は便宜上`points[1]`と同じ段を指す
    /// （時刻0の開始点自体は編集対象にならないため、ドラッグ編集のヒットテストでは
    /// `points[1..]`のみを対象にすればよい）。
    pub stage_of_point: Vec<usize>,
    /// ループ区間が`points`上で占める折れ線の範囲（始点index, 終点index、両端含む）。
    /// ループが成立しない（`loop_enabled=0`、または`loop_start >= release_point`で1段ループ）なら`None`。
    /// 2周描画モードでは2周目（＝実際にループし続ける区間）を指す。
    /// 始点は段`loop_start`の**開始**頂点なので、`loop_start=0`なら`points[0]`（グラフ最左）になる。
    pub loop_span: Option<(usize, usize)>,
    /// リリースが始まる`points`上のindex（＝保持区間の最後の頂点）。`points[0]`が開始点ぶん
    /// オフセットしているため、これはそのままUI上の1始まりクリック点番号＝「リリース点」に一致する
    /// （`TimeEgParams::release_point`が指す0始まり段indexに+1した値）。
    pub release_point: usize,
    /// リリース区間が空（`TimeEgParams::release_point`が最終段）のとき、「ノートオフ後もレベルが
    /// 変化せず持続する」ことを示すためだけにグラフ右端へ置く終端頂点のindex。段の値を編集できる
    /// わけではないのでドラッグ対象外にする（`points[0]`と同じ扱い）。
    /// リリース区間があるときは常に`None`。
    pub sustain_terminal_point: Option<usize>,
    /// 1段(重み1.0)あたりのピクセル数（`width / drawn_count`）。Step 8のGRAPHモードが
    /// ドラッグ位置から`time`を逆算する（`width_to_time`）際、このレイアウト計算時と
    /// 同じ`scale`を使う必要があるため公開する。
    pub scale: f32,
}

/// エディタ用レイアウト（`time_eg_editor_layout`）で速い段に与える最小横幅重み（0.0〜1.0）。
/// Step 7の実機確認で判明した「対数重み付けにより速い段がグラフ左端に団子状に密集し
/// ドラッグのヒットテストが困難になる」問題への対策（`time_eg_editor_layout`参照）。
/// `time=0`の段は瞬時を表す特殊値のため、この床の対象外（幅0のまま、`layout_impl`のif分岐参照）。
const EDITOR_MIN_STAGE_WEIGHT: f32 = 0.15;

/// `layout_impl`の挙動を切り替えるオプション。`time_eg_layout`（読み取り専用プレビュー）と
/// `time_eg_editor_layout`（編集用、Step 8）は同じ幾何計算ロジックを共有しつつ、
/// 横幅の潰れ方とループの描画周数だけが異なる。
struct LayoutOptions {
    /// 各段（time!=0）の最小横幅重み。プレビューは0.0（純粋な対数幅）、エディタは
    /// `EDITOR_MIN_STAGE_WEIGHT`（速い段でもヒットテスト可能な最小幅を保証）。
    min_weight: f32,
    /// ループ区間を何周描くか（1=ループなし相当、実際には1周目のみ描画）。
    /// 2以上ならループを`loop_cycles`回描き、2周目以降にはループドリフト
    /// （`TimeEgParams::has_drift()`）を`sound_core::drift_accumulated_after_cycles`で
    /// 適用したレベルを使う（`time_eg_layout`の既定動作）。エディタ用の1周描画モードは
    /// 描画段数を減らしヒットテストしやすくするための選択（設計上の意味は変わらない）。
    loop_cycles: usize,
}

/// `TimeEgParams`から`TimeEgGeometry`を計算する共通実装。`inner`はウィジェットの描画可能領域
/// （パディング適用後の矩形）、`mapping`/`tl`は`eg_preview`と同じ意味（TLを持たないFGパネルは
/// tl=255で呼ぶ）。`time_eg_layout`/`time_eg_editor_layout`はこの薄いラッパー。
fn layout_impl(params: &TimeEgParams, inner: Rect, mapping: EgAmplitudeMapping, tl: u8, opts: LayoutOptions) -> TimeEgGeometry {
    if params.stage_count == 0 {
        // 無効状態（FGのみ、`TimeEgProfile::min_stages=0`で許容）。段が無いので折れ線ではなく
        // 「左右の輪郭丸＋中立レベルの水平線」だけを描く。左右の丸は`draw_geometry`が
        // `i==0`/`sustain_terminal_point`の条件でCOLOR_DOT_START（輪郭のみ）として描くため、
        // ここではpointsとsustain_terminal_pointを対応させるだけでよい。
        // release_point=0にすることで`draw_geometry`のセグメント色判定（`i-1>=release_point`）が
        // 真になり、線がCOLOR_RELEASE（赤）で描かれる（「エンジンから見て無変調＝リリース同然」
        // という位置づけに合わせた色）。
        let floor = axis_floor_db(mapping);
        let tl_db = tl_to_db(tl);
        let level_db = stage_target_db(mapping, floor, tl_db, disabled_level(mapping));
        let y = inner.bottom() - ((level_db.max(floor) - floor) / -floor) * inner.height();
        let points = vec![Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)];
        return TimeEgGeometry {
            points,
            stage_of_point: vec![0, 0],
            loop_span: None,
            release_point: 0,
            sustain_terminal_point: Some(1),
            scale: inner.width(),
        };
    }
    let n = clamp_stage_count(params.stage_count);
    let release_point = (params.release_point as usize).min(n - 1);
    // loop_start > release_pointは設定不整合。実エンジンもrelease_pointへクランプするので合わせる。
    // `loop_start == release_point`は1段ループで、入口レベルへ跳ね戻すノコギリ波になる
    // （sound-core/time_eg.rsのadvance参照）。
    let loop_start = (params.loop_start as usize).min(release_point);
    let loop_active = params.loop_enabled != 0;
    // 1段ループは周回のたびにレベルが不連続に跳ぶ。その跳ね先（段の入口レベル）。
    let single_stage_loop = loop_active && loop_start == release_point;
    let neutral_raw = neutral_start_level(mapping);
    let loop_entry_level_raw = if loop_start == 0 { neutral_raw } else { params.stages[loop_start - 1].level };

    // 周番号cycle(0=1周目、まだ一度も折り返していない)における、raw_levelのドリフト適用後レベルを
    // 返す。`sound_core::loop_pivot_level`/`drift_accumulated_after_cycles`/`apply_loop_drift`は
    // `sound_core::TimeEg`のランタイム計算と全く同じ式を使う自由関数（プレビューはTimeEgの
    // インスタンスを持たないため、N周先の見た目をここで直接計算する）。
    let drifted_level = |raw_level: u8, cycle: usize| -> u8 {
        if cycle == 0 || !params.has_drift() {
            return raw_level;
        }
        let pivot = sound_core::loop_pivot_level(params, neutral_raw as f32 / 255.0, loop_start, release_point);
        let (offset, gain) = sound_core::drift_accumulated_after_cycles(params, cycle);
        let drifted = sound_core::apply_loop_drift(pivot, raw_level as f32 / 255.0, offset, gain);
        (drifted * 255.0).round().clamp(0.0, 255.0) as u8
    };

    // 保持区間で辿る(段index, 周番号)の列: 1周目(cycle=0)は0..=release_point。ループ有効かつ
    // `opts.loop_cycles>=2`なら、loop_start..=release_pointをcycle=1,2,...と(loop_cycles-1)回
    // 追加で辿る（実機は無限に繰り返すが、プレビューは指定周数で打ち切る。周番号はドリフト計算に
    // そのまま渡す）。
    let mut held_sequence: Vec<(usize, usize)> = (0..=release_point).map(|s| (s, 0)).collect();
    // 1周目の段数(=release_point+1)。2周目以降の先頭を指すオフセットの基準として使う。
    let first_cycle_len = held_sequence.len();
    if loop_active {
        for cycle in 1..opts.loop_cycles {
            held_sequence.extend((loop_start..=release_point).map(|s| (s, cycle)));
        }
    }
    // ループ1周ぶんの段数（多段・1段どちらのループでも各周で共通）。
    let held_cycle_len = release_point - loop_start + 1;
    // 最終周（`opts.loop_cycles`周目、1-indexed）がheld_sequence上で開始するインデックス。
    // `opts.loop_cycles<2`（ループ描画なし）ならNone。
    let last_cycle_start_held_idx =
        (opts.loop_cycles >= 2).then(|| first_cycle_len + (opts.loop_cycles - 2) * held_cycle_len);

    // リリース区間で辿る(段index, 周番号)の列: release_point+1..n（現在レベルから順にそれぞれの
    // targetへ向かう多段リリース、ドリフトの影響を受けないためcycle=0固定）。保持区間と合わせて
    // 段リストをちょうど分割するので、両者は決して重ならない。release_pointが最終段のときだけ
    // 空になる（＝リリース無し。Gain FGの透過既定用）。
    let release_sequence: Vec<(usize, usize)> = (release_point + 1..n).map(|s| (s, 0)).collect();

    let floor = axis_floor_db(mapping);
    let tl_db = tl_to_db(tl);
    let x0 = inner.left();
    let width = inner.width();
    // リリース区間が空のときも1段ぶんの幅を確保しておく（後述の「持続を示す水平線」を
    // 右端まで伸ばす余地を残すため。確保しないと保持区間だけで横幅を使い切りうる）。
    let drawn_count = (held_sequence.len() + release_sequence.len().max(1)).max(1);
    let scale = width / drawn_count as f32;
    let right_edge = x0 + width;
    let db_to_y = |db: f32| inner.bottom() - ((db.max(floor) - floor) / -floor) * inner.height();

    // note-onの起点はパネル種別で決まる（TimeEg::note_onの`neutral_level`と対応）。
    // 振幅系は0（無音）、バイポーラのPitch/Cutoff FGは中央128（無変調）。
    let start_db = stage_target_db(mapping, floor, tl_db, neutral_raw);
    let mut points = vec![Pos2::new(x0, db_to_y(start_db))];
    let mut stage_of_point = vec![held_sequence.first().map(|&(s, _)| s).unwrap_or(0)];
    let mut x = x0;

    // 1段ループを複数周描くときは、周回の境目ごとにレベルが入口へ跳ね戻る。周回のたびに
    // 同じxへ頂点をもう1つ置いて幅0の縦線として明示する（描かないと各周が平坦な線になり、
    // 実際の音と食い違って見える）。跳ね戻し先自体もその周のドリフトが乗る。
    let jump_ats: Vec<(usize, usize)> = if single_stage_loop && opts.loop_cycles >= 2 {
        (1..opts.loop_cycles)
            .map(|cycle| (first_cycle_len + (cycle - 1) * held_cycle_len, cycle))
            .collect()
    } else {
        Vec::new()
    };
    let jump_extra = jump_ats.len();
    // 最終周の開始頂点（points配列上のインデックス）。ジャンプ挿入で位置がずれうるため、
    // 実際の構築過程で観測して記録する（計算式での事前導出はジャンプ数に依存して複雑になり
    // バグを呼びやすいため避けた）。
    let mut loop_span_start_point: Option<usize> = None;

    for (i, &(stage_idx, cycle)) in held_sequence.iter().chain(release_sequence.iter()).enumerate() {
        if last_cycle_start_held_idx == Some(i) {
            loop_span_start_point = Some(points.len() - 1);
        }
        if let Some(&(_, jump_cycle)) = jump_ats.iter().find(|&&(idx, _)| idx == i) {
            let entry_level = drifted_level(loop_entry_level_raw, jump_cycle);
            let entry_db = stage_target_db(mapping, floor, tl_db, entry_level);
            points.push(Pos2::new(x, db_to_y(entry_db)));
            stage_of_point.push(stage_idx);
        }
        let stage = params.stages[stage_idx];
        // time=0は「瞬時」を表す特殊値（レート方式のrate=0=フリーズとは意味が真逆）なので、
        // min_weightの床には引っかからず常に幅0（垂直）のままにする。
        let weight = if stage.time == 0 { 0.0 } else { log_width(time_to_seconds(stage.time)).max(opts.min_weight) };
        x = (x + weight * scale).min(right_edge);
        let level = drifted_level(stage.level, cycle);
        let target_db = stage_target_db(mapping, floor, tl_db, level);
        points.push(Pos2::new(x, db_to_y(target_db)));
        stage_of_point.push(stage_idx);
    }

    // ループ区間は段`loop_start..=release_point`で、段kはグラフ上`points[k]→points[k+1]`の線分。
    // したがって折れ線としての範囲は**始点`points[loop_start]`から終点`points[release_point+1]`まで**。
    // 始点側に+1すると1段ぶん右へずれる（実機確認で「L.START=0なのに三角マーカーが最左に来ない」
    // として発覚したバグ。`loop_start=0`のときの始点はlevel=0の開始点`points[0]`そのものになる）。
    // 複数周描画: 最終周の先頭段の始点が`loop_span_start_point`（構築過程で観測済み）、
    // 終点が`points[held_sequence.len()+jump_extra]`。1周描画: 延長されないので1周目そのもの
    // （`points[loop_start]`〜`points[first_cycle_len]`）。
    let loop_span = loop_active.then_some(if let Some(start) = loop_span_start_point {
        (start, held_sequence.len() + jump_extra)
    } else {
        (loop_start, first_cycle_len)
    });
    let release_point_index = held_sequence.len() + jump_extra;

    // リリース区間が空（`release_point`が最終段）のときは、note-off後もレベルが変化せず
    // 持続することを示す水平線を右端まで伸ばす。段に対応しない目印なのでドラッグ対象外にする
    // （`points[0]`と同じ扱い）。この形はGain FGの透過既定＝ゲートを一切閉じない専用で、
    // OP EG/Pitch FG/Cutoff FGはエディタ側がSTAGE>=2と最終段level=0を強制するため発生しない。
    let sustain_terminal_point = if release_sequence.is_empty() {
        let last = points.len() - 1;
        points.push(Pos2::new(right_edge, points[last].y));
        stage_of_point.push(release_point);
        Some(points.len() - 1)
    } else {
        None
    };

    TimeEgGeometry { points, stage_of_point, loop_span, release_point: release_point_index, scale, sustain_terminal_point }
}

/// ループドリフト無効時にプレビューが描く周数（「ここが繰り返す」visual、従来からの既定）。
const LOOP_PREVIEW_CYCLES_STATIC: usize = 2;
/// ループドリフト有効時にプレビューが描く周数。中心/振れ幅が周を追うごとに動いていく様子を
/// 見せるため、静的ループより多めに描く（プランのユーザー選択：「ドリフト時のみ4周描く」）。
const LOOP_PREVIEW_CYCLES_DRIFTING: usize = 4;

/// `TimeEgParams`から`TimeEgGeometry`を計算する（読み取り専用プレビュー用）。`inner`はウィジェットの
/// 描画可能領域（パディング適用後の矩形）、`mapping`/`tl`は`eg_preview`と同じ意味（TLを持たない
/// FGパネルはtl=255で呼ぶ）。ループドリフト（`level_drift`/`depth_drift`）が有効なパッチは
/// 4周、それ以外は従来通り2周描く（中立時は既存の見た目・8件の既存テストと完全に一致する）。
pub fn time_eg_layout(params: &TimeEgParams, inner: Rect, mapping: EgAmplitudeMapping, tl: u8) -> TimeEgGeometry {
    let loop_cycles = if params.has_drift() { LOOP_PREVIEW_CYCLES_DRIFTING } else { LOOP_PREVIEW_CYCLES_STATIC };
    layout_impl(params, inner, mapping, tl, LayoutOptions { min_weight: 0.0, loop_cycles })
}

/// `time_eg_layout`のエディタ用バリアント（Step 8のGRAPHモードが使う）。速い段でも
/// `EDITOR_MIN_STAGE_WEIGHT`ぶんの最小幅を確保し、ループは1周のみ描く
/// （`LayoutOptions`のドキュメント参照。頂点のドラッグ・右クリック編集を実用的にするための調整で、
/// `time_eg_layout`自体の見た目・8件の既存テストには影響しない）。ドリフトの有無に関わらず
/// 常に1周（編集対象の曖昧さを避けるため、`time_eg_layout`側の周数切り替えとは独立）。
pub fn time_eg_editor_layout(params: &TimeEgParams, inner: Rect, mapping: EgAmplitudeMapping, tl: u8) -> TimeEgGeometry {
    layout_impl(params, inner, mapping, tl, LayoutOptions { min_weight: EDITOR_MIN_STAGE_WEIGHT, loop_cycles: 1 })
}

/// `TimeEgGeometry`をeguiで描画する（保持区間=緑、リリース区間=赤。曲線整形は各段の`curve`に従う）。
/// `time_eg_editor`（同一クレート内、Step 8）もGRAPHモードの描画で共有するため`pub(crate)`。
/// `dot_radius`は頂点ドットの半径をpx単位で明示指定する（線の太さは従来通り`ui_scale`依存のまま、
/// ドットだけ独立して大きくできるように分離してある。`time_eg_editor`のGRAPHモードは
/// 固定の大きめ半径`EDITOR_DOT_RADIUS_PX`を渡し、読み取り専用プレビューは`DOT_RADIUS * ui_scale`を渡す）。
pub(crate) fn draw_geometry(painter: &egui::Painter, params: &TimeEgParams, geometry: &TimeEgGeometry, ui_scale: f32, dot_radius: f32) {
    for i in 1..geometry.points.len() {
        let from = geometry.points[i - 1];
        let to = geometry.points[i];
        let stage_idx = geometry.stage_of_point[i];
        let curved = params.stages[stage_idx].curve != 0;
        let color = if i - 1 >= geometry.release_point { COLOR_RELEASE } else { COLOR_HELD };
        draw_ramp(painter, from, to, color, false, curved, ui_scale);
    }
    for (i, &p) in geometry.points.iter().enumerate() {
        if i == 0 || geometry.sustain_terminal_point == Some(i) {
            painter.circle_stroke(p, dot_radius, Stroke::new(1.0, COLOR_DOT_START));
        } else {
            painter.circle_filled(p, dot_radius, COLOR_DOT);
        }
    }
}

/// TL(0〜255、TLを持たないFGパネルは255)と`TimeEgParams`から、保持区間=緑・リリース区間=赤に
/// 塗り分けた折れ線グラフを描く（`eg_preview`のTimeEg版、読み取り専用）。
/// `size`はpanel.xmlの`<style>`相当の占有サイズ（`DEFAULT_WIDTH`×`DEFAULT_HEIGHT`からの
/// 等方拡大率をパディング・線幅・頂点ドット半径へ一律に乗じる）。
pub fn time_eg_preview(ui: &mut Ui, size: Vec2, mapping: EgAmplitudeMapping, tl: u8, params: TimeEgParams) {
    let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let ui_scale = (size.x / DEFAULT_WIDTH).min(size.y / DEFAULT_HEIGHT);
    let pad = PAD * ui_scale;

    painter.rect_filled(rect, 3.0, COLOR_BEZEL);
    let inner = rect.shrink(pad);
    painter.rect_filled(inner, 2.0, COLOR_PANEL);

    let geometry = time_eg_layout(&params, inner, mapping, tl);
    draw_geometry(painter, &params, &geometry, ui_scale, DOT_RADIUS * ui_scale);

    // ループ区間を強調する下線（loop_span内の頂点の直下に一本引く）。
    if let Some((lo, hi)) = geometry.loop_span {
        let y = inner.bottom() + 2.0 * ui_scale;
        let from = Pos2::new(geometry.points[lo].x, y);
        let to = Pos2::new(geometry.points[hi].x, y);
        painter.add(Shape::line(vec![from, to], Stroke::new(1.5 * ui_scale, COLOR_HELD)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sound_core::TimeStage;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0))
    }

    fn stages(entries: &[(u8, u8, u8)]) -> [TimeStage; MAX_STAGES] {
        let mut stages = [TimeStage::default(); MAX_STAGES];
        for (i, &(time, level, curve)) in entries.iter().enumerate() {
            stages[i] = TimeStage { time, level, curve };
        }
        stages
    }

    /// e3相当（かつてのop505-coreデモパッチ`demo_patch(0)`のgain_fgと同じ形、現在は廃止済み）:
    /// 静止を挟んだ音量2値スイッチ、4段ループ+リリース。
    fn gain_switch_params() -> TimeEgParams {
        TimeEgParams {
            stages: stages(&[(15, 230, 0), (40, 230, 0), (15, 40, 0), (40, 40, 0), (100, 0, 0)]),
            stage_count: 5,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 3,
         ..Default::default()}
    }

    #[test]
    fn vertex_x_is_monotonically_increasing() {
        let g = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        for w in g.points.windows(2) {
            assert!(w[1].x >= w[0].x, "頂点のxは単調増加のはず: {:?}", g.points);
        }
    }

    #[test]
    fn time_zero_stage_has_zero_width() {
        // time=0の段は瞬時（幅0=垂直）。直前の頂点と同じx座標になるはず。
        let params = TimeEgParams {
            stages: stages(&[(0, 255, 0)]),
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
         ..Default::default()};
        let g = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert_eq!(g.points[0].x, g.points[1].x, "time=0は幅0(垂直)のはず");
    }

    #[test]
    fn flat_stage_is_horizontal() {
        // levelが直前と同値(静止段)の区間は完全に水平（d5/e3で刺さった「静止を挟んだ切替」の核）。
        let g = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        // held_sequence = [0,1,2,3, 0,1,2,3]（loop_enabled=1で2周）。points[1]=stage0到達,
        // points[2]=stage1到達（静止段、level=stage0と同値のためy一致）。
        assert_eq!(g.points[1].y, g.points[2].y, "stage0→stage1(静止)はyが変わらないはず");
        assert_eq!(g.points[3].y, g.points[4].y, "stage2→stage3(静止)はyが変わらないはず");
    }

    #[test]
    fn stages_beyond_stage_count_are_not_drawn() {
        let mut params = gain_switch_params();
        params.stage_count = 5;
        let g_full = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        // stage_count=2に絞ると、release_pointも自動的にクランプされ描画段数が減る。
        params.stage_count = 2;
        params.release_point = 0;
        let g_small = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert!(g_small.points.len() < g_full.points.len());
        assert!(g_small.stage_of_point.iter().all(|&s| s < 2));
    }

    #[test]
    fn loop_enabled_draws_two_cycles() {
        let g = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let (lo, hi) = g.loop_span.expect("loop_enabled=1のはず");
        // loop_start(0)..=release_point(3)は4段＝折れ線4セグメント。
        assert_eq!(hi - lo, 4, "loop_start..=release_point間のセグメント数のはず");
        // ループの始点と終点は同じレベル（1周して同じ場所へ戻るため）。
        assert!((g.points[lo].y - g.points[hi].y).abs() < 1e-3, "ループは同じレベルへ閉じるはず");
    }

    /// ループドリフト有効時は4周描き、かつループが「同じレベルへ閉じない」（螺旋状に動く）
    /// ことをグラフ上でも確認する。中立時の対テスト`loop_enabled_draws_two_cycles`と対になる。
    #[test]
    fn drift_draws_four_cycles_and_does_not_close() {
        let mut params = gain_switch_params();
        params.level_drift = 40; // 128未満=下降方向
        let g = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let (lo, hi) = g.loop_span.expect("loop_enabled=1のはず");
        // 4周描画: 最終周(4周目)のセグメント数は中立時と同じ4段分のまま
        // （周を追加しても1周ぶんのセグメント数自体は変わらない）。
        assert_eq!(hi - lo, 4, "最終周もloop_start..=release_point間の4セグメントのはず");
        // level_driftが効いていれば、最終周の始点・終点は中立時より低いレベル（=大きいy）になる。
        let neutral = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let (neutral_lo, _) = neutral.loop_span.expect("loop_enabled=1のはず");
        assert!(
            g.points[lo].y > neutral.points[neutral_lo].y + 1.0,
            "level_drift適用後は中立時よりレベルが下がっている（y座標が大きい）はず: drifted={} neutral={}",
            g.points[lo].y,
            neutral.points[neutral_lo].y
        );
    }

    /// ループドリフトが無効（既定値128/128）なら、4周描画ロジック自体は`has_drift()`で
    /// 早期リターンし、中立時と完全に同じジオメトリになることを固定する
    /// （周数の切り替え条件が誤って常時4周になっていないかの回帰防止）。
    #[test]
    fn default_drift_produces_identical_geometry_to_two_cycle_baseline() {
        let params = gain_switch_params();
        assert!(!params.has_drift());
        let g = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        // 2周描画（従来動作）と同じセグメント数・座標になっているはず。
        let (lo, hi) = g.loop_span.expect("loop_enabled=1のはず");
        assert_eq!(hi - lo, 4);
        assert!((g.points[lo].y - g.points[hi].y).abs() < 1e-3, "中立時は従来通りループが閉じるはず");
    }

    /// L.START=0のループ開始マーカーはグラフ最左（level=0の開始点`points[0]`）に来る。
    /// 段kは`points[k]→points[k+1]`の線分なので、ループ区間`loop_start..=release_point`の
    /// 始点は`points[loop_start]`。ここを+1すると1段ぶん右へずれる（実機確認で発覚したバグ）。
    #[test]
    fn loop_start_marker_anchors_at_stage_entry_vertex() {
        let mut params = gain_switch_params();
        params.loop_start = 0;
        let g = time_eg_editor_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let (lo, hi) = g.loop_span.expect("loop_enabled=1のはず");
        assert_eq!(lo, 0, "loop_start=0の始点はpoints[0]（グラフ最左）のはず");
        assert_eq!(hi, g.release_point, "ループ終端はリリース点マーカーと同じ頂点のはず");

        params.loop_start = 2;
        let g = time_eg_editor_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let (lo, _) = g.loop_span.expect("loop_enabled=1のはず");
        assert_eq!(lo, 2, "loop_start=2の始点はpoints[2]のはず");
    }

    #[test]
    fn no_loop_has_no_loop_span() {
        let mut params = gain_switch_params();
        params.loop_enabled = 0;
        let g = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert_eq!(g.loop_span, None);
    }

    #[test]
    fn release_point_matches_release_transition() {
        let g = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        // release_point以降の頂点は release_sequence = [4](release_point=3, stage_count=5) の1段のみ。
        assert_eq!(g.points.len() - 1 - g.release_point, 1, "リリースはstage4の1段だけのはず");
        assert_eq!(*g.stage_of_point.last().unwrap(), 4);
    }

    #[test]
    fn gain_switch_demo_has_two_flat_regions() {
        // e3(旧op505-core demo_patch(0))の核＝静止を挟んだ2値スイッチが、保持区間に2つの水平区間として現れる。
        let g = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let flat_count = g.points.windows(2).take(g.release_point).filter(|w| (w[1].y - w[0].y).abs() < 1e-3).count();
        assert!(flat_count >= 2, "静止区間が2つ以上見えるはず: flat_count={flat_count}");
    }

    /// STAGES=0（無効状態、FGのみ）は段のある通常レイアウトを一切通らず、左右2点だけの
    /// 水平線ジオメトリを返すはず。`disabled_level`が返す高さで描かれ、`release_point=0`
    /// （`draw_geometry`がCOLOR_RELEASEで塗る条件）・両端とも`sustain_terminal_point`扱い
    /// （輪郭丸、`draw_geometry`のdoc参照）になっているかを確認する。
    #[test]
    fn disabled_state_draws_flat_line_at_disabled_level() {
        for mapping in [EgAmplitudeMapping::RawLinear, EgAmplitudeMapping::AmplitudeLinear] {
            let p = TimeEgParams { stage_count: 0, ..TimeEgParams::default() };
            let g = time_eg_editor_layout(&p, rect(), mapping, 255);
            assert_eq!(g.points.len(), 2, "{mapping:?}: 無効状態は左右2点だけのはず");
            assert!((g.points[0].y - g.points[1].y).abs() < 1e-3, "{mapping:?}: 水平線のはず");
            assert_eq!(g.stage_of_point, vec![0, 0]);
            assert_eq!(g.release_point, 0, "{mapping:?}: セグメントがCOLOR_RELEASEで描かれるはず");
            assert_eq!(g.sustain_terminal_point, Some(1), "{mapping:?}: 右端も輪郭丸扱いのはず");
            assert_eq!(g.loop_span, None);

            let floor = axis_floor_db(mapping);
            let tl_db = tl_to_db(255);
            let expected_db = stage_target_db(mapping, floor, tl_db, disabled_level(mapping));
            let expected_y = rect().bottom() - ((expected_db.max(floor) - floor) / -floor) * rect().height();
            assert!(
                (g.points[0].y - expected_y).abs() < 1e-3,
                "{mapping:?}: disabled_levelの高さで描かれるはず (actual={}, expected={expected_y})",
                g.points[0].y
            );
        }
    }

    /// Gain FG無効時は×1.0（透過）であって0.0（無音）ではない
    /// （`op505-core::Voice::tick`のスキップ実装と一致させる。plan
    /// `docs/timeeg-fg-disable-plan.md`の🔴落とし穴参照）。透過(255)はバイポーラ中央(128)より
    /// 高いレベルなので、同じ描画領域ではAmplitudeLinearの方が上（yが小さい）に描かれるはず。
    #[test]
    fn disabled_gain_fg_is_drawn_above_neutral_bipolar_not_at_silence() {
        let p = TimeEgParams { stage_count: 0, ..TimeEgParams::default() };
        let bipolar = time_eg_editor_layout(&p, rect(), EgAmplitudeMapping::RawLinear, 255);
        let gain = time_eg_editor_layout(&p, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert!(
            gain.points[0].y < bipolar.points[0].y,
            "Gain FG無効(255=透過)はバイポーラ中央(128)より上に描かれるはず: gain_y={} bipolar_y={}",
            gain.points[0].y,
            bipolar.points[0].y
        );
    }

    #[test]
    fn editor_layout_draws_single_cycle() {
        // Step 8のGRAPHモード用: 2周描画(time_eg_layout)より頂点数が少なく(1周分)、
        // loop_start..=release_point間のセグメント数は同じ(4)であるはず。
        let preview = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let editor = time_eg_editor_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert!(editor.points.len() < preview.points.len(), "1周描画は2周描画より頂点数が少ないはず");
        let (lo, hi) = editor.loop_span.expect("loop_enabled=1のはず");
        assert_eq!(hi - lo, 4, "1周描画でもloop_start..=release_point間のセグメント数は変わらないはず");
    }

    #[test]
    fn editor_layout_enforces_min_stage_width() {
        // gain_switch_paramsのstage0はtime=15(速い段)。1周描画時の描画段数は
        // held(0..=loop_end=3で4段)+release(1段)=5、scale=width/5。
        let g = time_eg_editor_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let increment = g.points[1].x - g.points[0].x;
        let scale = rect().width() / 5.0;
        assert!(
            increment >= EDITOR_MIN_STAGE_WEIGHT * scale - 1e-3,
            "速い段でもEDITOR_MIN_STAGE_WEIGHT分の最小幅を確保するはず: increment={increment}"
        );
    }

    #[test]
    fn editor_layout_keeps_time_zero_vertical() {
        // min_weightの床はtime=0(瞬時の特殊値)には適用されない。エディタ用レイアウトでも
        // 幅0(垂直)のままであるべき(time_zero_stage_has_zero_widthのエディタ版)。
        let params = TimeEgParams {
            stages: stages(&[(0, 255, 0)]),
            stage_count: 1,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
         ..Default::default()};
        let g = time_eg_editor_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert_eq!(g.points[0].x, g.points[1].x, "エディタ用レイアウトでもtime=0は幅0(垂直)のはず");
    }
}
