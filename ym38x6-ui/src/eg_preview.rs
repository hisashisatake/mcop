// ---------------------------------------------------------------------------
// EG形状プレビュー（OP1〜4/FILTER/VCA各パネル左側の折れ線グラフ）
//
// 実際の発音をトレースするのではなく、現在のTL/AR/D1R/D1L/D2R/RRノブ値から
// 「今この設定だとどんな形になるか」を静的に描くプレビュー（ノブを動かすとその場で更新される）。
//
// 構造（ユーザーとの対話で確定した仕様）:
//   縦軸=dB（0dB天井〜DB_FLOOR床）、横軸=時間。
//   AR: 床からTL（TLノブをdB変換した値。0dBが天井で、TLが低いほど天井より下に位置する）へ接続。
//   DR: TLからSL（D1LをdB変換しTLへ加算した値、dB同士の加算＝リニアゲイン同士の乗算に相当）へ接続。
//   SR: SLからさらにSR_DROP_DB（dBレンジの2/3）ぶん下降した点まで減衰（絶対位置ではなくSLからの
//       相対量なので、D1Lの違いがSR/Release側にも一貫して反映される）。
//   RR: SRの到達点から床（無音）へ接続。
// note-on側（AR/DR/SR）は緑、Release（RR）は赤。TLノブを持たないFILTER/VCAはtl=255
// （減衰なし＝天井0dB）で呼び出すことで、OPと全く同じ描画仕様のまま使う。
//
// 横幅の割り当て方針:
//   各フェーズの横幅は、そのフェーズの実測秒数（sound-coreのar_to_delta/decay_to_delta/
//   rr_to_deltaにsample_rate=1.0を渡すと閉形式で求まる。Eg::tick()の各フェーズは一定deltaの
//   線形ランプなのでサンプル単位のシミュレーションは不要）を、フェーズ固有のレンジ
//   （AR=[0.68ms,20.2秒] / D1R・D2R・RR=[8.71ms,284.9秒]）で対数目盛の0.0〜1.0へ写した値
//   （log_width）に比例させる。レンジ最速端(rate=255)がちょうど幅0＝グラフが垂直（瞬時）、
//   最遅端(rate=1/フリーズ)が幅1.0。実測秒数は≈6桁と桁違いに広いので対数で圧縮する。
//   スケールは固定（最大合計MAX_TOTAL_WEIGHTがウィジェット幅ちょうど）で、合計による正規化は
//   しない。正規化（可変スケール）だと1本のフェーズを動かすたびに全体のスケールが変わり、
//   触っていないフェーズの頂点まで左右に動いてしまうため。固定スケールなら各頂点のX座標は
//   そのフェーズ自身の値だけで決まって動かず、同じ秒数はどのパッチでも同じ幅で描かれる。
//   短い音ほど右側に余白が残る（実際の短さを表す）。
//
// 縦方向（到達レベル）の方針＝ターゲット方式:
//   各フェーズは有限レートなら必ず目標レベルまで完全到達する。レートの速さは横幅（秒数）と
//   傾きに現れ、到達レベル自体はレートに依らない。フリーズ(rate=0のD1R/D2R)のときのみ
//   開始レベルに留まる。これによりD1L（SLレベル）がSL点の高さへ直結して見える。
//   （旧・progress補間方式はD1Rが遅いとSL点がTL付近に留まり、D1Lを動かしても縦に動かないのが
//   欠点だった。その代わりrate=0とrate=1の間に到達レベルの段差が生じるが、これはフリーズ特殊値の
//   「減衰しない vs 極めてゆっくり減衰する」という本質的な違いを正直に表したもの。）
//
// 見た目・数式係数はすべて初期案（暫定）。CLAUDE.mdのテスト方針に従い、
// 実装後に見た目を見て調整する。
// ---------------------------------------------------------------------------

use egui::{Color32, Pos2, Shape, Stroke, Ui, Vec2};
use sound_core::eg::{ar_to_delta, decay_to_delta, rr_to_delta};

const WIDTH: f32 = 84.0;
const HEIGHT: f32 = 66.0;
const PAD: f32 = 6.0;

/// 縦軸の下限（dB、無音とみなす床）。
const DB_FLOOR: f32 = -96.0;
/// SR（Sustain Rate=D2Rクリープ）がSLからさらに下降する固定量（dBレンジの2/3）。
/// 絶対位置ではなくSLからの相対量にすることで、D1Lの違いがSR/Release側にも一貫して反映される。
const SR_DROP_DB: f32 = 96.0 * 2.0 / 3.0;

/// D1R/D2Rがフリーズ特殊値(rate=0)のとき、「rateが際限なく遅い」の極限として扱うための疑似秒数
/// （log_width()に通すとレンジ上端を超えてclampで最大幅に張り付く。フリーズを別分岐せず
/// 「最も遅い＝最も幅広い」として自然に扱うために使う）。
const FROZEN_SECONDS: f32 = 1.0e6;

/// log_width()が各フェーズの横幅を対数正規化する実測レンジ（フェーズごとに範囲が異なる）。
/// レンジ最速端(rate=255)がちょうど幅0（＝グラフが垂直＝瞬時）、最遅端(rate=1/フリーズ)が幅1.0。
/// 値はsound-core::eg（ar_to_delta/decay_to_delta/rr_to_deltaのアンカー）と一致させること。
const AR_MIN_SECONDS: f32 = 0.00068; // AR rate=255（最速）
const AR_MAX_SECONDS: f32 = 20.2; // AR rate=1（最遅）
const DECAY_MIN_SECONDS: f32 = 0.00871; // D1R/D2R/RR rate=255（最速）
const DECAY_MAX_SECONDS: f32 = 284.9; // D1R/D2R rate=1 ・ RR rate=0（最遅）

/// 横幅の固定スケール基準＝4フェーズ×log_width最大値(1.0)。この合計がウィジェット幅に
/// ちょうど収まるよう固定スケールを決める（正規化＝可変スケールをやめる理由は下記）。
const MAX_TOTAL_WEIGHT: f32 = 4.0;

/// note-on側（Attack/Decay1/Hold=AR/DR/SR）の色。
const COLOR_HELD: Color32 = Color32::from_rgb(80, 220, 90);
/// Release（RR、note-off後）の色。
const COLOR_RELEASE: Color32 = Color32::from_rgb(230, 80, 70);
const COLOR_SILENT: Color32 = Color32::from_gray(110);

/// 折れ線の各交点（頂点）に打つ丸印の色。計算結果を目視確認しやすいよう頂点ごとに色分けする。
const DOT_RADIUS: f32 = 2.5;
const COLOR_DOT_START: Color32 = Color32::from_gray(160);
const COLOR_DOT_TL: Color32 = Color32::from_rgb(255, 210, 60);
const COLOR_DOT_SL: Color32 = Color32::from_rgb(60, 210, 255);
const COLOR_DOT_HOLD: Color32 = Color32::from_rgb(230, 60, 230);
const COLOR_DOT_END: Color32 = Color32::from_rgb(240, 240, 240);

/// レート値(0〜255)が0→1（の全域）を渡りきるのに何秒かかるかを返す。
/// `to_delta(rate, 1.0)`が0（AR/D1R/D2Rのrate=0＝フリーズ特殊値）なら`None`。
fn segment_seconds(rate: u8, to_delta: fn(u8, f32) -> f32) -> Option<f32> {
    let delta = to_delta(rate, 1.0);
    if delta <= 0.0 {
        None
    } else {
        Some(1.0 / delta)
    }
}

/// D1R/D2Rのrate=0（フリーズ）を「rateが際限なく遅い」の極限として扱い、`segment_seconds`の
/// None(フリーズ)/Some(通常)を分岐せずFROZEN_SECONDS（＝最大幅）に写す。
fn rate_seconds_or_frozen(rate: u8, to_delta: fn(u8, f32) -> f32) -> f32 {
    segment_seconds(rate, to_delta).unwrap_or(FROZEN_SECONDS)
}

/// フェーズの実測秒数を横幅の重み(0.0〜1.0)へ変換する。フェーズ固有のレンジ[min,max]を対数で
/// 0.0〜1.0へ写す。最速端(min秒＝rate=255)はちょうど0.0＝幅ゼロ＝グラフが垂直（瞬時を表す）、
/// 最遅端(max秒＝rate=1/フリーズ)はちょうど1.0＝最大幅。フリーズ(FROZEN_SECONDS)はmaxを超えるので
/// clampで1.0に張り付く。フロアは設けない（rate=255を垂直にするため意図的にゼロまで潰す）。
fn log_width(seconds: f32, min_seconds: f32, max_seconds: f32) -> f32 {
    let lo = min_seconds.log10();
    let hi = max_seconds.log10();
    ((seconds.max(min_seconds).log10() - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// リニアゲイン(0.0〜1.0、D1Lノブの生値/255相当)をdBへ変換する（DB_FLOOR未満はクランプ）。
fn level_to_db(linear_fraction: f32) -> f32 {
    if linear_fraction <= 0.0 {
        DB_FLOOR
    } else {
        (20.0 * linear_fraction.log10()).max(DB_FLOOR)
    }
}

/// TLノブ(0〜255)をdBへ変換する。ym38x6-core::mapping::tl_to_gainと同じ理論値
/// （実機OPM TL、0.75dB/step・127段をtl=255(0dB)〜tl=0(-95.25dB)にアンカー）をdBのまま返す
/// （tl_to_gainはリニアゲインを返すため、このプレビューのdB軸ではその手前のdB値を直接使う）。
/// ym38x6-uiはエンジンクレート非依存を保つ方針のため、ここに複製する
/// （数式を変えたらym38x6-core側と揃えること）。
fn tl_to_db(tl: u8) -> f32 {
    (-95.25 * (255 - tl) as f32 / 255.0).max(DB_FLOOR)
}

/// TL/AR/D1R/D1L/D2R/RR(いずれも0〜255)から、AR/DR/SR=緑・RR=赤の2色に塗り分けた
/// 折れ線グラフを描く。パネル左側に配置する固定サイズの液晶風ウィジェット。
/// TLを持たないFILTER/VCAはtl=255（減衰なし）で呼び出す。
pub fn eg_preview(ui: &mut Ui, tl: u8, ar: u8, d1r: u8, d1l: u8, d2r: u8, rr: u8) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(WIDTH, HEIGHT), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();

    // 背景（ベゼル＋液晶面のニュアンス）
    painter.rect_filled(rect, 3.0, Color32::from_gray(35));
    let inner = rect.shrink(PAD);
    painter.rect_filled(inner, 2.0, Color32::from_gray(72));

    let db_to_y = |db: f32| inner.bottom() - ((db.max(DB_FLOOR) - DB_FLOOR) / -DB_FLOOR) * inner.height();
    let x0 = inner.left();
    let width = inner.width();

    // AR=0は「フリーズしたまま発音しない」特殊値（spec.md準拠）。床のフラット線のみ描いて終える。
    if ar == 0 {
        let y = db_to_y(DB_FLOOR);
        painter.add(Shape::line(vec![Pos2::new(x0, y), Pos2::new(x0 + width, y)], Stroke::new(1.5, COLOR_SILENT)));
        return;
    }

    let tl_db = tl_to_db(tl);

    // 各フェーズの実測秒数（AR=0は上で早期return済みなのでここではSome）。
    let ar_seconds = segment_seconds(ar, ar_to_delta).unwrap_or(0.0);
    let d1r_seconds = rate_seconds_or_frozen(d1r, decay_to_delta);
    // D1Rがフリーズ中はDecay2に自然到達しないため（Eg::tick()参照）、その場合はD2R自体の値に
    // よらずフリーズ（=SRの到達点に留まり続ける）扱いにする。
    let d2r_seconds = if d1r != 0 { rate_seconds_or_frozen(d2r, decay_to_delta) } else { FROZEN_SECONDS };
    let rr_seconds = segment_seconds(rr, rr_to_delta).unwrap_or(0.0);

    // 横幅の重み（実測秒数を各フェーズ固有レンジで対数正規化。rate=255は幅0＝垂直、最遅は幅1.0）。
    let attack_weight = log_width(ar_seconds, AR_MIN_SECONDS, AR_MAX_SECONDS);
    let decay1_weight = log_width(d1r_seconds, DECAY_MIN_SECONDS, DECAY_MAX_SECONDS);
    let hold_weight = log_width(d2r_seconds, DECAY_MIN_SECONDS, DECAY_MAX_SECONDS);
    let release_weight = log_width(rr_seconds, DECAY_MIN_SECONDS, DECAY_MAX_SECONDS);

    // 縦方向は「ターゲット方式」：各フェーズは有限レートなら必ず目標レベルまで完全到達し、
    // 到達の速さ（レート）は横幅（＝その秒数）と傾きに現れる。フリーズ(rate=0)のみ開始点に留まる。
    // これによりD1L（SLレベル）がSL点の高さに直結して見える（旧progress方式はD1Rが遅いと
    // SL点がTL付近に留まり、D1Lを動かしても縦に動かないのが欠点だった）。
    // AR: 床 -> TL（AR>0は常に完全到達）。
    // DR: TL -> SL。D1Rフリーズ(rate=0)ならTLに留まる。SLの絶対dB位置はTL+D1LのdB加算で求める。
    let sl_db = (tl_db + level_to_db(d1l as f32 / 255.0)).max(DB_FLOOR);
    let decay1_db = if d1r == 0 { tl_db } else { sl_db };

    // SR: SL -> SLからSR_DROP_DBぶん下降した点（絶対位置ではなくSL相対、D1Lの効果を保つため）。
    // D1RまたはD2RがフリーズならクリープせずSL（=decay1_db）に留まる。
    let sr_target_db = (decay1_db - SR_DROP_DB).max(DB_FLOOR);
    let hold_db = if d1r == 0 || d2r == 0 { decay1_db } else { sr_target_db };

    // RR: hold_db -> 床（RRはrate=0でも284.9秒の有限値でフリーズしない、常に完全到達）。

    // 各フェーズ幅を「そのフェーズ自身の実測秒数」だけで決めるため、正規化（可変スケール）ではなく
    // 固定スケールを使う。scaleを合計で割ると1本を動かすたびにscaleが変わり、触っていない
    // フェーズの頂点まで左右に動いてしまう。固定スケール（最大合計=MAX_TOTAL_WEIGHTが幅ちょうど）に
    // すれば、各頂点のX座標はそのフェーズ自身の値だけで決まり動かない。かつ同じ秒数はどのパッチでも
    // 同じ幅で描かれ、時間軸がパッチ間で揃う。短い音ほど右側に余白が残る＝実際の短さを正直に表す。
    let scale = width / MAX_TOTAL_WEIGHT;

    let mut x = x0;
    let start_pt = Pos2::new(x, db_to_y(DB_FLOOR));
    x += attack_weight * scale;
    let tl_pt = Pos2::new(x, db_to_y(tl_db));
    x += decay1_weight * scale;
    let sl_pt = Pos2::new(x, db_to_y(decay1_db));
    x += hold_weight * scale;
    let hold_pt = Pos2::new(x, db_to_y(hold_db));
    painter.add(Shape::line(vec![start_pt, tl_pt, sl_pt, hold_pt], Stroke::new(1.5, COLOR_HELD)));

    let release_start = hold_pt;
    x += release_weight * scale;
    // 固定スケールでは通常x < x0+width（短い音ほど手前で終わる）。最長エンベロープでちょうど
    // 右端に達する。浮動小数点誤差の安全弁としてclampは残す。
    let release_end = Pos2::new(x.min(x0 + width), db_to_y(DB_FLOOR));
    painter.add(Shape::line(vec![release_start, release_end], Stroke::new(1.5, COLOR_RELEASE)));

    // 各交点（頂点）に色分けした丸印を打つ（計算結果の目視確認用）。
    painter.circle_filled(start_pt, DOT_RADIUS, COLOR_DOT_START);
    painter.circle_filled(tl_pt, DOT_RADIUS, COLOR_DOT_TL);
    painter.circle_filled(sl_pt, DOT_RADIUS, COLOR_DOT_SL);
    painter.circle_filled(hold_pt, DOT_RADIUS, COLOR_DOT_HOLD);
    painter.circle_filled(release_end, DOT_RADIUS, COLOR_DOT_END);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_seconds_freezes_at_rate_zero_for_ar() {
        assert_eq!(segment_seconds(0, ar_to_delta), None);
    }

    #[test]
    fn segment_seconds_is_finite_and_decreasing_with_rate() {
        let slow = segment_seconds(1, ar_to_delta).unwrap();
        let fast = segment_seconds(255, ar_to_delta).unwrap();
        assert!(slow > fast, "レートが速いほど所要秒数は短いはず: slow={slow} fast={fast}");
        assert!((slow - 20.2).abs() < 1e-3);
        assert!((fast - 0.00068).abs() < 1e-6);
    }

    #[test]
    fn segment_seconds_rr_never_freezes() {
        // RRはrate=0でも284.9秒の有限値（decay_to_deltaと違いフリーズしない）
        assert_eq!(segment_seconds(0, rr_to_delta), Some(284.9));
    }

    #[test]
    fn level_to_db_bounds_and_monotonic() {
        assert!((level_to_db(1.0) - 0.0).abs() < 1e-6);
        assert_eq!(level_to_db(0.0), DB_FLOOR);
        assert!(level_to_db(0.5) > level_to_db(0.1));
    }

    #[test]
    fn tl_to_db_matches_engine_anchor_points() {
        assert!((tl_to_db(255) - 0.0).abs() < 1e-6);
        assert!((tl_to_db(0) - (-95.25)).abs() < 1e-3);
        assert!(tl_to_db(128) < tl_to_db(255));
    }

    #[test]
    fn log_width_is_monotonic_and_bounded() {
        // 遅い（秒数大）ほど幅は広く、速いほど狭い。0.0〜1.0の範囲に収まる。
        let slow = log_width(segment_seconds(1, ar_to_delta).unwrap(), AR_MIN_SECONDS, AR_MAX_SECONDS);
        let fast = log_width(segment_seconds(255, ar_to_delta).unwrap(), AR_MIN_SECONDS, AR_MAX_SECONDS);
        assert!(slow > fast, "遅いほど幅は広いはず: slow={slow} fast={fast}");
        assert!(fast >= 0.0 && slow <= 1.0 + 1e-6);
    }

    #[test]
    fn rate_255_is_vertical_zero_width_for_every_phase() {
        // 各Rを最速(255)にするとそのフェーズの幅がちょうど0＝グラフが垂直になる（要望仕様）。
        let ar = log_width(segment_seconds(255, ar_to_delta).unwrap(), AR_MIN_SECONDS, AR_MAX_SECONDS);
        let d = log_width(segment_seconds(255, decay_to_delta).unwrap(), DECAY_MIN_SECONDS, DECAY_MAX_SECONDS);
        let rr = log_width(segment_seconds(255, rr_to_delta).unwrap(), DECAY_MIN_SECONDS, DECAY_MAX_SECONDS);
        assert!(ar.abs() < 1e-6, "AR=255は幅0のはず: {ar}");
        assert!(d.abs() < 1e-6, "D1R/D2R=255は幅0のはず: {d}");
        assert!(rr.abs() < 1e-6, "RR=255は幅0のはず: {rr}");
    }

    #[test]
    fn log_width_ceiling_and_frozen_are_full_width() {
        // レンジ最遅端(rate=1相当)は幅1.0、フリーズ(FROZEN_SECONDS)もclampで幅1.0。
        assert!((log_width(AR_MAX_SECONDS, AR_MIN_SECONDS, AR_MAX_SECONDS) - 1.0).abs() < 1e-6);
        assert!((log_width(DECAY_MAX_SECONDS, DECAY_MIN_SECONDS, DECAY_MAX_SECONDS) - 1.0).abs() < 1e-6);
        assert!((log_width(FROZEN_SECONDS, DECAY_MIN_SECONDS, DECAY_MAX_SECONDS) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn max_total_weight_matches_four_full_phases() {
        // 全フェーズが最大幅（フリーズ/最遅）のとき合計がMAX_TOTAL_WEIGHTと一致し、
        // 固定スケールでちょうどウィジェット幅を埋めることを保証する。
        let full = log_width(FROZEN_SECONDS, DECAY_MIN_SECONDS, DECAY_MAX_SECONDS);
        assert!((full - 1.0).abs() < 1e-6);
        assert!((full * 4.0 - MAX_TOTAL_WEIGHT).abs() < 1e-6);
    }

    /// テスト用に、本体eg_preview()と同じ縦方向ターゲット方式でDR/SR到達レベルを再現するヘルパー。
    fn target_levels(tl: u8, d1l: u8, d1r: u8, d2r: u8) -> (f32, f32) {
        let tl_db = tl_to_db(tl);
        let sl_db = (tl_db + level_to_db(d1l as f32 / 255.0)).max(DB_FLOOR);
        let decay1_db = if d1r == 0 { tl_db } else { sl_db };
        let sr_target_db = (decay1_db - SR_DROP_DB).max(DB_FLOOR);
        let hold_db = if d1r == 0 || d2r == 0 { decay1_db } else { sr_target_db };
        (decay1_db, hold_db)
    }

    #[test]
    fn d1l_moves_sl_vertex_vertically_regardless_of_d1r_rate() {
        // 要望: D1Lを変えるとSL点が縦に動く。有限レートなら常にSLへ完全到達するため、
        // D1Rが遅く(rate=8)てもD1Lの違いがdecay1_dbにそのまま出る（旧progress方式では出なかった）。
        let (sl_lo, _) = target_levels(255, 100, 8, 128);
        let (sl_hi, _) = target_levels(255, 220, 8, 128);
        assert!(sl_hi > sl_lo, "D1Lを上げるとSL点は上がるはず: lo={sl_lo} hi={sl_hi}");
    }

    #[test]
    fn frozen_d1r_holds_at_tl_and_ignores_d1l() {
        // D1Rフリーズ(rate=0)はTLに留まる＝decay1_dbはTLで、D1Lによらず一定。
        let tl_db = tl_to_db(200);
        let (sl_a, _) = target_levels(200, 60, 0, 128);
        let (sl_b, _) = target_levels(200, 240, 0, 128);
        assert!((sl_a - tl_db).abs() < 1e-6 && (sl_b - tl_db).abs() < 1e-6);
    }
}
