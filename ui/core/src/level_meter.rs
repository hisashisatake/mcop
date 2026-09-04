//! レベルメーター（ピークバー + クリップ検出ランプ）ウィジェット。
//!
//! `MeterHandle`は`IntParamHandle`と違い読み取り専用（オーディオスレッドが書き、GUIは読むだけ
//! のデータ）。ピークホールドと減衰は毎フレームここで計算する（`ui.memory()`へホールド値を
//! 保持する、`eg_preview.rs`のカラーパレット踏襲パターンと同じ考え方）。

use egui::{self, Color32, Sense, Ui};

use crate::param_handle::MeterHandle;

/// 既定サイズ。`ui_codegen::ir::Style::default().level_meter_size`と一致させること
/// （panel.xmlで`<level-meter>`にwidth/height属性を書かなかった場合のフォールバック）。
/// 高さは`SEGMENT_HEIGHT_PX`・`DEFAULT_SEGMENT_GAP_PX`から逆算した必要バー高さ47px
/// （`bar_area_required_height`参照）に、クリップランプ・パディング・ラベル分を足した値。
pub const LEVEL_METER_SIZE: egui::Vec2 = egui::vec2(40.0, 77.0);

const COLOR_BEZEL: Color32 = Color32::from_gray(35);
const COLOR_PANEL: Color32 = Color32::from_gray(15);
const COLOR_SEG_GREEN: Color32 = Color32::from_rgb(0, 255, 0);
const COLOR_SEG_YELLOW: Color32 = Color32::from_rgb(255, 255, 0);
const COLOR_SEG_RED: Color32 = Color32::from_rgb(255, 0, 0);
const COLOR_CLIP_ON: Color32 = Color32::from_rgb(255, 0, 0);
const COLOR_CLIP_OFF: Color32 = Color32::from_gray(60);

/// 1秒あたりのホールド値の減衰量（正規化ピーク値の単位）。
const HOLD_DECAY_PER_SEC: f32 = 1.2;
/// セグメント積み上げバーの段数。
const SEGMENT_COUNT: usize = 16;
/// セグメント1個の高さ（px、整数固定）。隙間(gap)も含めて常に整数pxに固定することで、
/// 各セグメントの上端・下端が常にピクセル境界に一致し、`with_round_to_pixels`でのスナップ
/// 結果が場所によってブレない（非整数だと丸め位置ごとに隙間が0px/1px/2pxとバラつく）。
const SEGMENT_HEIGHT_PX: f32 = 2.0;
/// セグメント間の隙間（px）の既定値。`set_segment_gap_px`で未設定のときに使う。
/// 実際に描画で使う際は整数pxへ丸める（`SEGMENT_HEIGHT_PX`と同じ理由）。
pub const DEFAULT_SEGMENT_GAP_PX: f32 = 1.0;
/// この正規化位置（0.0〜1.0）を超えるセグメントは黄色。
const YELLOW_FROM: f32 = 0.7;
/// この正規化位置を超えるセグメントは赤。
const RED_FROM: f32 = 0.9;

fn segment_gap_id() -> egui::Id {
    egui::Id::new("op505_level_meter_segment_gap_px")
}

/// セグメント間の隙間（px）をアプリ全体の設定として書き込む。呼び出し側（standalone等の
/// 設定ファイル読み込み処理）が起動時・設定変更時に呼ぶ想定（`0.0`で隙間なし）。
pub fn set_segment_gap_px(ctx: &egui::Context, gap_px: f32) {
    ctx.data_mut(|d| d.insert_persisted(segment_gap_id(), gap_px.max(0.0)));
}

/// セグメント数×`SEGMENT_HEIGHT_PX`＋隙間×(セグメント数-1)から、バー領域に必要な高さを
/// 逆算する。`<level-meter height="...">`（`panel.xml`）はこの値に、クリップランプ・
/// パディング・ラベル分を足した高さに合わせておくと、余白なくぴったり収まる。
pub fn bar_area_required_height(gap_px: f32) -> f32 {
    SEGMENT_COUNT as f32 * SEGMENT_HEIGHT_PX + (SEGMENT_COUNT as f32 - 1.0) * gap_px.round()
}

/// ホールド値の1フレーム分の更新。新しいピークが現在のホールド値より高ければ即座に
/// そこへ飛びつき、そうでなければ時間経過で減衰する（egui非依存の純粋関数、テスト用に分離）。
fn update_hold(current_hold: f32, new_peak: f32, dt_secs: f32) -> f32 {
    let decayed = (current_hold - HOLD_DECAY_PER_SEC * dt_secs.max(0.0)).max(0.0);
    decayed.max(new_peak.clamp(0.0, 1.0))
}

/// レベルメーターを描画する。クリックでクリップ検出をリセットする。`size`は外枠全体の
/// サイズ（`panel.xml`の`<level-meter width="..." height="...">`、省略時は`LEVEL_METER_SIZE`）。
pub fn level_meter(ui: &mut Ui, handle: &dyn MeterHandle, label: &str, size: egui::Vec2) {
    let m = handle.snapshot();
    let id = ui.id().with((handle.name(), "level_meter"));
    let dt = ui.input(|i| i.stable_dt);

    let hold_l: f32 = ui.memory(|mem| mem.data.get_temp(id.with("hold_l"))).unwrap_or(0.0);
    let hold_r: f32 = ui.memory(|mem| mem.data.get_temp(id.with("hold_r"))).unwrap_or(0.0);
    let new_hold_l = update_hold(hold_l, m.peak_l, dt);
    let new_hold_r = update_hold(hold_r, m.peak_r, dt);
    ui.memory_mut(|mem| {
        mem.data.insert_temp(id.with("hold_l"), new_hold_l);
        mem.data.insert_temp(id.with("hold_r"), new_hold_r);
    });

    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.label(egui::RichText::new(label).size(9.0));

        let bezel_size = egui::vec2(size.x, size.y - 20.0);
        let (rect, response) = ui.allocate_exact_size(bezel_size, Sense::click());
        if response.clicked() {
            handle.reset_clip();
        }
        if !ui.is_rect_visible(rect) {
            return;
        }

        let painter = ui.painter();
        // eguiは既定で矩形の縁にアンチエイリアシング（ぼかし）をかけるため、1px幅の隙間の
        // ように小さい要素はぼやけて消えてしまう。`with_round_to_pixels(true)`で矩形の境界を
        // 実ピクセル境界へスナップし、ぼかしを抑えてくっきり描く（ドットバイドットに近づける）。
        let rect_filled_crisp = |rect: egui::Rect, corner_radius: f32, color: Color32| {
            painter.add(egui::epaint::RectShape::filled(rect, corner_radius, color).with_round_to_pixels(true));
        };
        rect_filled_crisp(rect, 3.0, COLOR_BEZEL);
        let pad = 2.0;
        let inner = rect.shrink(pad);
        let bar_w = (inner.width() - pad) / 2.0;

        // クリップランプ（L/Rそれぞれの列の真上に独立表示）。
        let clip_h = 4.0;
        let clip_color = if m.clipped { COLOR_CLIP_ON } else { COLOR_CLIP_OFF };
        let clip_rect_l = egui::Rect::from_min_size(inner.min, egui::vec2(bar_w, clip_h));
        let clip_rect_r =
            egui::Rect::from_min_size(egui::pos2(inner.min.x + bar_w + pad, inner.min.y), egui::vec2(bar_w, clip_h));
        rect_filled_crisp(clip_rect_l, 1.0, clip_color);
        rect_filled_crisp(clip_rect_r, 1.0, clip_color);

        // L/Rバー領域（クリップランプの下）。widthいっぱいの背景は利用可能領域全体に塗るが、
        // セグメント自体は「必要な高さ」ぶんだけを下揃えで使う（下＝レベル0側を基準にする
        // ことで、`size`がわずかに大きくても上側に余白ができるだけで隙間の均一さは崩れない）。
        let bar_top = inner.min.y + clip_h + pad;
        let bar_area_full = egui::Rect::from_min_max(egui::pos2(inner.min.x, bar_top), inner.max);
        rect_filled_crisp(bar_area_full, 2.0, COLOR_PANEL);

        // gapは整数pxに丸める（`SEGMENT_HEIGHT_PX`と同じく、非整数だとピクセルスナップ後の
        // 隙間が場所ごとにブレるため）。
        let gap = ui
            .ctx()
            .data_mut(|d| d.get_persisted::<f32>(segment_gap_id()))
            .unwrap_or(DEFAULT_SEGMENT_GAP_PX)
            .round();
        let used_h = bar_area_required_height(gap).min(bar_area_full.height());
        let bar_area =
            egui::Rect::from_min_max(egui::pos2(bar_area_full.min.x, bar_area_full.max.y - used_h), bar_area_full.max);
        let draw_bar = |x0: f32, level: f32| {
            let level = level.clamp(0.0, 1.0);
            for i in 0..SEGMENT_COUNT {
                let seg_bottom = bar_area.max.y - i as f32 * (SEGMENT_HEIGHT_PX + gap);
                let seg_top = seg_bottom - SEGMENT_HEIGHT_PX;
                // このセグメントが担う正規化レンジの中央値。ここより上まで達していれば点灯。
                let seg_center = (i as f32 + 0.5) / SEGMENT_COUNT as f32;
                let seg_top_pos = (i as f32 + 1.0) / SEGMENT_COUNT as f32;
                let color_on = if seg_top_pos > RED_FROM {
                    COLOR_SEG_RED
                } else if seg_top_pos > YELLOW_FROM {
                    COLOR_SEG_YELLOW
                } else {
                    COLOR_SEG_GREEN
                };
                let color = if level >= seg_center { color_on } else { COLOR_PANEL };
                let seg_rect = egui::Rect::from_min_max(egui::pos2(x0, seg_top), egui::pos2(x0 + bar_w, seg_bottom));
                rect_filled_crisp(seg_rect, 0.0, color);
            }
        };
        draw_bar(bar_area.min.x, new_hold_l);
        draw_bar(bar_area.min.x + bar_w + pad, new_hold_r);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_jumps_up_immediately_on_higher_peak() {
        assert_eq!(update_hold(0.2, 0.8, 0.016), 0.8);
    }

    #[test]
    fn hold_decays_over_time_when_peak_is_lower() {
        let decayed = update_hold(1.0, 0.0, 0.5);
        assert!(decayed < 1.0 && decayed > 0.0, "0.5秒後は減衰しているが0にはまだ届かないはず: {decayed}");
    }

    #[test]
    fn hold_does_not_go_negative() {
        assert_eq!(update_hold(0.05, 0.0, 10.0), 0.0);
    }

    #[test]
    fn hold_clamps_peak_above_one() {
        assert_eq!(update_hold(0.0, 1.5, 0.0), 1.0);
    }

    #[test]
    fn zero_dt_does_not_decay() {
        assert_eq!(update_hold(0.5, 0.0, 0.0), 0.5);
    }

    #[test]
    fn bar_area_required_height_matches_default_gap() {
        // SEGMENT_COUNT=16, SEGMENT_HEIGHT_PX=2.0, gap=1.0 -> 16*2 + 15*1 = 47.0
        assert_eq!(bar_area_required_height(DEFAULT_SEGMENT_GAP_PX), 47.0);
    }

    #[test]
    fn bar_area_required_height_rounds_non_integer_gap() {
        // 1.4pxは1pxへ丸められる。
        assert_eq!(bar_area_required_height(1.4), bar_area_required_height(1.0));
    }

    #[test]
    fn bar_area_required_height_zero_gap_uses_segment_height_only() {
        assert_eq!(bar_area_required_height(0.0), 16.0 * SEGMENT_HEIGHT_PX);
    }
}
