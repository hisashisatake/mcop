//! レベルメーター（ピークバー + クリップ検出ランプ）ウィジェット。
//!
//! `MeterHandle`は`IntParamHandle`と違い読み取り専用（オーディオスレッドが書き、GUIは読むだけ
//! のデータ）。ピークホールドと減衰は毎フレームここで計算する（`ui.memory()`へホールド値を
//! 保持する、`eg_preview.rs`のカラーパレット踏襲パターンと同じ考え方）。

use egui::{self, Color32, Sense, Ui};

use crate::param_handle::MeterHandle;

pub const LEVEL_METER_SIZE: egui::Vec2 = egui::vec2(40.0, 66.0);

const COLOR_BEZEL: Color32 = Color32::from_gray(35);
const COLOR_PANEL: Color32 = Color32::from_gray(15);
const COLOR_BAR_NORMAL: Color32 = Color32::from_rgb(80, 220, 90);
const COLOR_BAR_HOT: Color32 = Color32::from_rgb(230, 200, 60);
const COLOR_CLIP_ON: Color32 = Color32::from_rgb(230, 60, 50);
const COLOR_CLIP_OFF: Color32 = Color32::from_gray(60);

/// 1秒あたりのホールド値の減衰量（正規化ピーク値の単位）。
const HOLD_DECAY_PER_SEC: f32 = 1.2;
/// このレベルを超えたバーは警告色（黄）で表示する。
const HOT_THRESHOLD: f32 = 0.9;

/// ホールド値の1フレーム分の更新。新しいピークが現在のホールド値より高ければ即座に
/// そこへ飛びつき、そうでなければ時間経過で減衰する（egui非依存の純粋関数、テスト用に分離）。
fn update_hold(current_hold: f32, new_peak: f32, dt_secs: f32) -> f32 {
    let decayed = (current_hold - HOLD_DECAY_PER_SEC * dt_secs.max(0.0)).max(0.0);
    decayed.max(new_peak.clamp(0.0, 1.0))
}

/// レベルメーターを描画する。クリックでクリップ検出をリセットする。
pub fn level_meter(ui: &mut Ui, handle: &dyn MeterHandle, label: &str) {
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

    ui.allocate_ui_with_layout(LEVEL_METER_SIZE, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.label(egui::RichText::new(label).size(9.0));

        let bezel_size = egui::vec2(LEVEL_METER_SIZE.x, LEVEL_METER_SIZE.y - 20.0);
        let (rect, response) = ui.allocate_exact_size(bezel_size, Sense::click());
        if response.clicked() {
            handle.reset_clip();
        }
        if !ui.is_rect_visible(rect) {
            return;
        }

        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, COLOR_BEZEL);
        let pad = 2.0;
        let inner = rect.shrink(pad);

        // クリップランプ（上部、幅いっぱい）。
        let clip_h = 6.0;
        let clip_rect = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), clip_h));
        painter.rect_filled(clip_rect, 1.0, if m.clipped { COLOR_CLIP_ON } else { COLOR_CLIP_OFF });

        // L/Rバー領域（クリップランプの下）。
        let bar_top = inner.min.y + clip_h + pad;
        let bar_area = egui::Rect::from_min_max(egui::pos2(inner.min.x, bar_top), inner.max);
        painter.rect_filled(bar_area, 2.0, COLOR_PANEL);

        let bar_w = (bar_area.width() - pad) / 2.0;
        let draw_bar = |x0: f32, level: f32| {
            let h = bar_area.height() * level.clamp(0.0, 1.0);
            let bar_rect =
                egui::Rect::from_min_max(egui::pos2(x0, bar_area.max.y - h), egui::pos2(x0 + bar_w, bar_area.max.y));
            let color = if level > HOT_THRESHOLD { COLOR_BAR_HOT } else { COLOR_BAR_NORMAL };
            painter.rect_filled(bar_rect, 0.0, color);
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
}
