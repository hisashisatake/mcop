// ---------------------------------------------------------------------------
// TimeEg形状プレビュー（OP505のOP1〜4/PITCH FG/CUTOFF FG/GAIN FG各パネル左側の折れ線グラフ）
//
// `eg_preview.rs`（レート方式5段EG用）の姉妹モジュール。縦軸dB・横軸時間・固定スケール・
// ターゲット方式・レイズドコサイン整形といった設計原則はそのまま踏襲し、色定数・ベゼル/液晶背景・
// dB変換式（tl_to_db/level_contribution_db/DB_FLOOR）・曲線描画（draw_ramp）は
// `pub(crate)`昇格したeg_preview.rsの実装をそのまま共有する（同一crate内）。
//
// TimeEgとレート方式EGの構造上の違い（流用できない理由）:
// - フェーズ固定4本(AR/D1R/D2R/RR)ではなく、1〜8段の可変長リスト（`TimeEgParams::stage_count`）
// - 各段が`level`を独立に持つ（レート方式は「AR=常にTLへ完全到達」という前提があったが、
//   TimeEgでは段0から既に任意のlevelを持てる）
// - フリーズ特殊値が無い（`time=0`は「瞬時」でレート方式の`rate=0`＝フリーズとは意味が真逆。
//   `log_width`に`time_to_seconds(0)=0.0`を通すと自然に幅0になるため、レート方式のような
//   `rate_seconds_or_frozen`分岐・破線描画は不要）
// - ループは`loop_start`〜`loop_end`の任意区間（レート方式はAR/D1Rの2区間固定だった）
// - リリースは`release_start`から`stage_count-1`まで順に辿る多段リリース（レート方式は
//   RR1本のみで必ず無音へ着地したが、TimeEgのリリースは終着レベルが任意——例えばGain FGの
//   `rr=0`透過既定を変換したパッチはlevel=255で据え置く——なので、必ず床へ落ちるとは限らない）
//
// 実際の音（`sound_core::TimeEg::tick`/`advance`の状態機械）を`time_eg_layout`で忠実に
// 再現する：note-onは常にlevel=0.0から段0へ向かい、段を`0,1,...,loop_end`と辿る。loop_end到達時
// `loop_enabled`なら`loop_start`へ戻る（本プレビューでは2周描いて「ここが繰り返す」ことを示す）、
// でなければその段のレベルで静止（サステイン）。note-offは`release_start`段へ直接ジャンプし
// （その時点の現在レベルから）、以降`stage_count-1`まで順に辿って終わる。
// ---------------------------------------------------------------------------

use egui::{Pos2, Rect, Shape, Stroke, Ui, Vec2};
use sound_core::time_eg::time_to_seconds;
use sound_core::{TimeEgParams, MAX_STAGES};

use crate::eg_preview::{
    draw_ramp, level_contribution_db, tl_to_db, EgAmplitudeMapping, COLOR_BEZEL, COLOR_HELD, COLOR_PANEL, COLOR_RELEASE, DB_FLOOR,
};

/// `<style>`省略時の既定サイズ。TimeEgは最大8段(保持側最大16セグメント+リリース最大8セグメント)を
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

/// TL(0〜255)とTimeEgの1段(level)からターゲットdBを求める。`level_contribution_db`は
/// 「TLからの相対減衰」を返すため、`tl_db`に加算しDB_FLOORで下限クランプする
/// （`eg_preview::eg_preview`のTL→SL算出と同じ式。TimeEgは全段が同じ式で求まる——
/// レート方式のように「AR到達点=常にTL」という特別扱いが要らない）。
fn stage_target_db(mapping: EgAmplitudeMapping, tl_db: f32, level: u8) -> f32 {
    (tl_db + level_contribution_db(mapping, level as f32 / 255.0)).max(DB_FLOOR)
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
    /// ループ区間が`points`上で占める範囲（開始index, 終了index）。`loop_enabled=0`なら`None`。
    /// 2周目（＝実際にループし続ける区間）を指す。
    pub loop_span: Option<(usize, usize)>,
    /// リリースが始まる`points`上のindex（＝保持区間の最後の頂点）。
    pub release_point: usize,
    /// STAGE=1（全区間が1段だけ）のとき、保持側と完全に同じ段を指すリリース側の頂点index。
    /// 別の値を編集できるわけではなく「ノートオフ後もレベルが変化せず持続する」ことを示す
    /// だけの目印なので、グラフ右端に固定しドラッグ対象外にする（`points[0]`と同じ扱い）。
    /// STAGE>=2では常に`None`。
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
    /// trueならループ区間を2周描く（「ここが繰り返す」visual、`time_eg_layout`の既存動作）。
    /// falseなら1周のみ描く（エディタ用。描画段数が半減し団子問題がさらに緩和され、
    /// 「2周目の頂点はどの段を指すか」という編集対象の曖昧さも生じない）。
    loop_two_cycles: bool,
}

/// `TimeEgParams`から`TimeEgGeometry`を計算する共通実装。`inner`はウィジェットの描画可能領域
/// （パディング適用後の矩形）、`mapping`/`tl`は`eg_preview`と同じ意味（TLを持たないFGパネルは
/// tl=255で呼ぶ）。`time_eg_layout`/`time_eg_editor_layout`はこの薄いラッパー。
fn layout_impl(params: &TimeEgParams, inner: Rect, mapping: EgAmplitudeMapping, tl: u8, opts: LayoutOptions) -> TimeEgGeometry {
    let n = clamp_stage_count(params.stage_count);
    let loop_start = (params.loop_start as usize).min(n - 1);
    // loop_start > loop_endは設定不整合(UI編集中の一時状態等)。幾何計算がおかしくならないよう
    // loop_startへクランプする（実エンジンはこの場合stage_count終端で静止する挙動になるが、
    // プレビューでは「ループなしで1周だけ」と同じ絵になるよう単純化する）。
    let loop_end = (params.loop_end as usize).min(n - 1).max(loop_start);
    let release_start = (params.release_start as usize).min(n - 1);

    // 保持区間で辿る段indexの列: 0..=loop_end（1周目）。loop_enabledかつ2周描画モードなら
    // loop_start..=loop_endをもう1周（実機は無限に繰り返すが、プレビューは「ここが繰り返す」と
    // 分かるよう2周で打ち切る。eg_preview.rsのLoop=1描画と同じ方針）。エディタ用の1周描画モードは
    // 描画段数を減らしヒットテストしやすくするための選択（設計上の意味は変わらない）。
    let mut held_sequence: Vec<usize> = (0..=loop_end).collect();
    // 1周目の段数(=loop_end+1)。2周描画モードでは「2周目の先頭」を指すオフセットとして、
    // 1周描画モードでは（延長されないため）held_sequence自体の最終長として使う。
    let first_cycle_len = held_sequence.len();
    if params.loop_enabled != 0 && opts.loop_two_cycles {
        held_sequence.extend(loop_start..=loop_end);
    }

    // リリース区間で辿る段indexの列: release_start..n-1（現在レベルから順にそれぞれの
    // targetへ向かう多段リリース。以前のfor advance()と同じ順序を辿る）。
    let release_sequence: Vec<usize> = (release_start..n).collect();

    let tl_db = tl_to_db(tl);
    let x0 = inner.left();
    let width = inner.width();
    let drawn_count = (held_sequence.len() + release_sequence.len()).max(1);
    let scale = width / drawn_count as f32;
    let right_edge = x0 + width;
    let db_to_y = |db: f32| inner.bottom() - ((db.max(DB_FLOOR) - DB_FLOOR) / -DB_FLOOR) * inner.height();

    // note-onは常にlevel=0.0から始まる（TimeEg::note_on参照）。
    let start_db = stage_target_db(mapping, tl_db, 0);
    let mut points = vec![Pos2::new(x0, db_to_y(start_db))];
    let mut stage_of_point = vec![*held_sequence.first().unwrap_or(&0)];
    let mut x = x0;

    for &stage_idx in held_sequence.iter().chain(release_sequence.iter()) {
        let stage = params.stages[stage_idx];
        // time=0は「瞬時」を表す特殊値（レート方式のrate=0=フリーズとは意味が真逆）なので、
        // min_weightの床には引っかからず常に幅0（垂直）のままにする。
        let weight = if stage.time == 0 { 0.0 } else { log_width(time_to_seconds(stage.time)).max(opts.min_weight) };
        x = (x + weight * scale).min(right_edge);
        let target_db = stage_target_db(mapping, tl_db, stage.level);
        points.push(Pos2::new(x, db_to_y(target_db)));
        stage_of_point.push(stage_idx);
    }

    // points[0]がlevel=0の開始点ぶんオフセットしているため、held_sequence側のindexへ+1して
    // points indexへ変換する（points[j+1] == held_sequence[j]の対応関係）。
    // 2周描画: `first_cycle_len`が2周目の先頭（held_sequence内index）。
    // 1周描画: 延長されないため`first_cycle_len == held_sequence.len()`となり、
    //          1周目そのもの（loop_start..=loop_end）を指す。
    let loop_span = (params.loop_enabled != 0).then_some(if opts.loop_two_cycles {
        (first_cycle_len + 1, held_sequence.len())
    } else {
        (loop_start + 1, first_cycle_len)
    });
    let release_point = held_sequence.len();

    // STAGE=1（n==1）は保持区間・リリース区間とも同じ段0を指すため、points[2]は
    // points[1]と同じ値をドラッグする冗長な点になる。実時間の重みで置くと中途半端な位置に浮き
    // 「もう1段ある」ように誤読される（実機確認で判明）ため、右端に固定して目印化する。
    let sustain_terminal_point = if n == 1 {
        let last = points.len() - 1;
        points[last].x = right_edge;
        Some(last)
    } else {
        None
    };

    TimeEgGeometry { points, stage_of_point, loop_span, release_point, scale, sustain_terminal_point }
}

/// `TimeEgParams`から`TimeEgGeometry`を計算する（読み取り専用プレビュー用）。`inner`はウィジェットの
/// 描画可能領域（パディング適用後の矩形）、`mapping`/`tl`は`eg_preview`と同じ意味（TLを持たない
/// FGパネルはtl=255で呼ぶ）。
pub fn time_eg_layout(params: &TimeEgParams, inner: Rect, mapping: EgAmplitudeMapping, tl: u8) -> TimeEgGeometry {
    layout_impl(params, inner, mapping, tl, LayoutOptions { min_weight: 0.0, loop_two_cycles: true })
}

/// `time_eg_layout`のエディタ用バリアント（Step 8のGRAPHモードが使う）。速い段でも
/// `EDITOR_MIN_STAGE_WEIGHT`ぶんの最小幅を確保し、ループは1周のみ描く
/// （`LayoutOptions`のドキュメント参照。頂点のドラッグ・右クリック編集を実用的にするための調整で、
/// `time_eg_layout`自体の見た目・8件の既存テストには影響しない）。
pub fn time_eg_editor_layout(params: &TimeEgParams, inner: Rect, mapping: EgAmplitudeMapping, tl: u8) -> TimeEgGeometry {
    layout_impl(params, inner, mapping, tl, LayoutOptions { min_weight: EDITOR_MIN_STAGE_WEIGHT, loop_two_cycles: false })
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
            loop_end: 3,
            release_start: 4,
        }
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
            loop_end: 0,
            release_start: 0,
        };
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
        // stage_count=2に絞ると、loop_end/release_startも自動的にクランプされ描画段数が減る。
        params.stage_count = 2;
        params.loop_end = 1;
        params.release_start = 1;
        let g_small = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert!(g_small.points.len() < g_full.points.len());
        assert!(g_small.stage_of_point.iter().all(|&s| s < 2));
    }

    #[test]
    fn loop_enabled_draws_two_cycles() {
        let g = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let (lo, hi) = g.loop_span.expect("loop_enabled=1のはず");
        // loop_start(0)..=loop_end(3)は4段=3セグメント(頂点間の遷移数)。
        assert_eq!(hi - lo, 3, "loop_start..=loop_end間のセグメント数のはず");
        // 2周目の開始頂点(lo)は1周目の対応する頂点(points[1]、loop_start=0のtarget)と同じ高さ
        // （同じ段のtargetを2回描くだけなので）。
        assert_eq!(g.points[lo].y, g.points[1].y);
    }

    #[test]
    fn no_loop_has_no_loop_span() {
        let mut params = gain_switch_params();
        params.loop_enabled = 0;
        let g = time_eg_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert_eq!(g.loop_span, None);
    }

    #[test]
    fn release_point_matches_release_start_transition() {
        let g = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        // release_point以降の頂点は release_sequence = [4](release_start=4, stage_count=5) の1段のみ。
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

    #[test]
    fn editor_layout_draws_single_cycle() {
        // Step 8のGRAPHモード用: 2周描画(time_eg_layout)より頂点数が少なく(1周分)、
        // loop_start..=loop_end間のセグメント数は同じ(3)であるはず。
        let preview = time_eg_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        let editor = time_eg_editor_layout(&gain_switch_params(), rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert!(editor.points.len() < preview.points.len(), "1周描画は2周描画より頂点数が少ないはず");
        let (lo, hi) = editor.loop_span.expect("loop_enabled=1のはず");
        assert_eq!(hi - lo, 3, "1周描画でもloop_start..=loop_end間のセグメント数は変わらないはず");
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
            loop_end: 0,
            release_start: 0,
        };
        let g = time_eg_editor_layout(&params, rect(), EgAmplitudeMapping::AmplitudeLinear, 255);
        assert_eq!(g.points[0].x, g.points[1].x, "エディタ用レイアウトでもtime=0は幅0(垂直)のはず");
    }
}
