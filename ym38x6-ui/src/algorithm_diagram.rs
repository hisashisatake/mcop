// ---------------------------------------------------------------------------
// アルゴリズム結線図（CHANNELパネル内、ALGノブの隣に置く液晶風ダイアグラム）
//
// ym38x6-core::algorithm::ALGORITHMSの結線（モジュレーター→ターゲット/キャリア/
// フィードバック対象）を、OP1〜4の番号ボックス＋矢印で可視化する。ym38x6-uiは
// sound-core専用でym38x6-core非依存の方針（eg_preview.rsのtl_to_dbと同じ理由）のため、
// 結線データ自体はここに直接複製する。ym38x6-core::algorithm::ALGORITHMSを変更したら
// routes()/carriers()もあわせて直すこと。
// ---------------------------------------------------------------------------

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, Ui, Vec2};

const WIDTH: f32 = 150.0;
const HEIGHT: f32 = 100.0;
const PAD: f32 = 6.0;
const BOX_SIZE: f32 = 15.0;
const ARROWHEAD_LEN: f32 = 5.0;
const ARROWHEAD_WIDTH: f32 = 3.5;

const COLOR_BOX: Color32 = Color32::from_gray(150);
const COLOR_BOX_STROKE: Color32 = Color32::from_gray(230);
const COLOR_TEXT: Color32 = Color32::from_gray(20);
/// モジュレーター→ターゲットの変調経路（note-on/DR/SRと同系統の緑）。
const COLOR_ROUTE: Color32 = Color32::from_rgb(80, 220, 90);
/// キャリア出力（チャンネル出力へ直接合算される矢印）。
const COLOR_CARRIER: Color32 = Color32::from_rgb(60, 210, 255);
/// フィードバック（自己変調）ループ。
const COLOR_FEEDBACK: Color32 = Color32::from_rgb(230, 80, 70);

/// 各アルゴリズムのオペレーターボックス中心座標（内枠の正規化座標0.0〜1.0）。
/// インデックス0〜3がOP1〜4に対応。ym38x6-core::algorithm::ALGORITHMSのroutes/carriersと
/// 矛盾しないよう、変調元が左、変調先が右に来るように手で配置している。
fn box_positions(algorithm: u8) -> [Pos2; 4] {
    match algorithm {
        // 0: O1 -> O2 -> O3 -> O4（直列4連）
        // OP4(box3)がキャンバス右端に近すぎるとキャリア出力矢印が箱に食い込むため、
        // 各OPの間隔を詰めて右端に余裕を残す。
        0 => [
            Pos2::new(0.14, 0.5),
            Pos2::new(0.36, 0.5),
            Pos2::new(0.58, 0.5),
            Pos2::new(0.78, 0.5),
        ],
        // 1: (O1 + O2) -> O3 -> O4
        // OP1(box0)は自己フィードバックループが上に回り込むため、y=0.30で上に余白を確保する。
        1 => [
            Pos2::new(0.14, 0.30),
            Pos2::new(0.14, 0.75),
            Pos2::new(0.50, 0.5),
            Pos2::new(0.78, 0.5),
        ],
        // 2: (O1 + (O2 -> O3)) -> O4
        2 => [
            Pos2::new(0.14, 0.30),
            Pos2::new(0.38, 0.75),
            Pos2::new(0.60, 0.75),
            Pos2::new(0.78, 0.5),
        ],
        // 3: ((O1 -> O2) + O3) -> O4
        // OP1・OP2(box0/1)は同じ行なので、フィードバック余白確保のため両方y=0.30に揃える。
        3 => [
            Pos2::new(0.14, 0.30),
            Pos2::new(0.38, 0.30),
            Pos2::new(0.14, 0.75),
            Pos2::new(0.78, 0.5),
        ],
        // 4: (O1 -> O2) + (O3 -> O4)（2系統並列）
        4 => [
            Pos2::new(0.18, 0.30),
            Pos2::new(0.58, 0.30),
            Pos2::new(0.18, 0.75),
            Pos2::new(0.58, 0.75),
        ],
        // 5: O1 -> O2, O1 -> O3, O1 -> O4（1対3ファンアウト）
        5 => [
            Pos2::new(0.16, 0.5),
            Pos2::new(0.60, 0.15),
            Pos2::new(0.60, 0.5),
            Pos2::new(0.60, 0.85),
        ],
        // 6: (O1 -> O2) + O3 + O4
        6 => [
            Pos2::new(0.14, 0.30),
            Pos2::new(0.55, 0.30),
            Pos2::new(0.14, 0.62),
            Pos2::new(0.14, 0.88),
        ],
        // 7: O1 + O2 + O3 + O4（全並列）
        _ => [
            Pos2::new(0.30, 0.28),
            Pos2::new(0.30, 0.48),
            Pos2::new(0.30, 0.68),
            Pos2::new(0.30, 0.88),
        ],
    }
}

/// モジュレーター→ターゲットの変調経路（op indexは0〜3）。
/// ym38x6-core::algorithm::ALGORITHMS\[n\].routesと一致させること。
fn routes(algorithm: u8) -> &'static [(usize, usize)] {
    match algorithm {
        0 => &[(0, 1), (1, 2), (2, 3)],
        1 => &[(0, 2), (1, 2), (2, 3)],
        2 => &[(0, 3), (1, 2), (2, 3)],
        3 => &[(0, 1), (1, 3), (2, 3)],
        4 => &[(0, 1), (2, 3)],
        5 => &[(0, 1), (0, 2), (0, 3)],
        6 => &[(0, 1)],
        _ => &[],
    }
}

/// チャンネル出力へ直接合算されるキャリアのop index一覧。
/// ym38x6-core::algorithm::ALGORITHMS\[n\].carriersと一致させること。
fn carriers(algorithm: u8) -> &'static [usize] {
    match algorithm {
        0..=3 => &[3],
        4 => &[1, 3],
        5 | 6 => &[1, 2, 3],
        _ => &[0, 1, 2, 3],
    }
}

/// 始点/終点を結ぶ直線＋先端の三角矢印を描く。ボックスの縁から生えるように、
/// 始点/終点それぞれをBOX_SIZE/2ぶん線分の内側へ寄せる。
fn draw_arrow(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    let delta = to - from;
    let len = delta.length();
    if len < 1.0 {
        return;
    }
    let dir = delta / len;
    let shrink = BOX_SIZE / 2.0 + 1.0;
    let start = from + dir * shrink.min(len / 2.0);
    let end = to - dir * shrink.min(len / 2.0);

    painter.add(Shape::line_segment([start, end], Stroke::new(1.3, color)));

    let normal = Vec2::new(-dir.y, dir.x);
    let tip = end;
    let base = end - dir * ARROWHEAD_LEN;
    let left = base + normal * ARROWHEAD_WIDTH;
    let right = base - normal * ARROWHEAD_WIDTH;
    painter.add(Shape::convex_polygon(vec![tip, left, right], color, Stroke::NONE));
}

/// フィードバック（自己変調）を、ボックス右辺（出力側）から出て上を回り込み、
/// ボックス左辺（入力側）へ戻る直線ループで描く（自分の出力を自分の入力へ回り込ませる形。
/// 参照画像のOPQマニュアル図に合わせ、円弧ではなく直線のみで構成する）。
/// ym38x6-coreの全アルゴリズムでfeedback_op=0（OP1固定）のため、常にcenters\[0\]に対して描く。
fn draw_feedback_loop(painter: &egui::Painter, box_center: Pos2) {
    let half = BOX_SIZE / 2.0;
    // box_center.yそのままにして、変調経路（緑）の出口と同じ高さで分岐点を重ねる
    // （描画順は呼び出し側でフィードバック→緑の順にし、重なった部分は緑が上に来るようにする）。
    let side_y = box_center.y;
    let out_point = Pos2::new(box_center.x + half, side_y);
    let in_point = Pos2::new(box_center.x - half, side_y);
    // ボックス幅そのままだと回り込みが分かりにくいため、左右にwidth_extraぶん張り出してから
    // 上へ回り込ませる（out_point/in_pointからいったん外側へ、そこから上→横→下の順）。
    let width_extra = BOX_SIZE * 0.5;
    let height = BOX_SIZE * 1.2;
    let out_side = Pos2::new(out_point.x + width_extra, side_y);
    let in_side = Pos2::new(in_point.x - width_extra, side_y);
    let top_y = side_y - height;
    let up_right = Pos2::new(out_side.x, top_y);
    let up_left = Pos2::new(in_side.x, top_y);

    painter.add(Shape::line(
        vec![out_point, out_side, up_right, up_left, in_side, in_point],
        Stroke::new(1.2, COLOR_FEEDBACK),
    ));

    // ループ終端（in_point、ボックス左辺）に右向き（in_side→in_pointの実際の進行方向）の矢頭を付ける。
    let dir = Vec2::new(1.0, 0.0);
    let normal = Vec2::new(-dir.y, dir.x);
    let base = in_point - dir * ARROWHEAD_LEN * 0.8;
    let left = base + normal * ARROWHEAD_WIDTH * 0.8;
    let right = base - normal * ARROWHEAD_WIDTH * 0.8;
    painter.add(Shape::convex_polygon(vec![in_point, left, right], COLOR_FEEDBACK, Stroke::NONE));
}

/// ALGノブの値(0〜7)から結線図を描く。CHANNELパネル内、ALG/FBノブの隣に配置する
/// 固定サイズの液晶風ウィジェット。
pub fn algorithm_diagram(ui: &mut Ui, algorithm: u8) {
    let algorithm = algorithm.min(7);
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(WIDTH, HEIGHT), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();

    // 背景（ベゼル＋液晶面のニュアンス。eg_preview.rsと共通の見た目）
    painter.rect_filled(rect, 3.0, Color32::from_gray(35));
    let inner = rect.shrink(PAD);
    painter.rect_filled(inner, 2.0, Color32::from_gray(72));

    let to_screen = |p: Pos2| Pos2::new(inner.left() + p.x * inner.width(), inner.top() + p.y * inner.height());
    let centers: [Pos2; 4] = box_positions(algorithm).map(to_screen);

    // ボックスを土台として先に描き、矢印は全て後から重ねる（先に矢印→後でボックスの順だと
    // 矢頭がボックス塗りつぶしに隠れて消えてしまうため）。
    for (i, &c) in centers.iter().enumerate() {
        let box_rect = Rect::from_center_size(c, Vec2::splat(BOX_SIZE));
        painter.rect_filled(box_rect, 2.0, COLOR_BOX);
        painter.rect_stroke(box_rect, 2.0, Stroke::new(1.0, COLOR_BOX_STROKE), egui::StrokeKind::Inside);
        painter.text(c, Align2::CENTER_CENTER, format!("{}", i + 1), FontId::monospace(9.0), COLOR_TEXT);
    }

    // フィードバック（赤）を先に描き、変調経路/キャリア出力（緑・水色）を後から重ねて描く。
    // OP1の出口では分岐点が同じ高さで重なるため、後から描く緑を上（前面）にする。
    // 全アルゴリズム共通でfeedback_op=0（OP1固定）。
    draw_feedback_loop(&painter, centers[0]);

    for &(from, to) in routes(algorithm) {
        draw_arrow(&painter, centers[from], centers[to], COLOR_ROUTE);
    }

    for &c in carriers(algorithm) {
        let start = centers[c];
        let end = Pos2::new(inner.right(), start.y);
        draw_arrow(&painter, start, end, COLOR_CARRIER);
    }
}
