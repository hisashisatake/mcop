use egui::{self, Pos2, Response, Sense, Stroke, Ui, Vec2, Widget};
use nice_plug::prelude::*;

/// ノブのスイープ角（度）。下方向に90度のギャップを設け、7時(最小)→12時(中間)→5時(最大)を描く。
const SWEEP_DEG: f32 = 270.0;
/// 通常ドラッグ時、1ピクセルあたりの正規化値の変化量（上方向ドラッグで増加）。
const DRAG_SPEED: f32 = 0.005;
/// Shift+ドラッグ時の微調整倍率。
const GRANULAR_MULT: f32 = 0.1;

/// nice-plugパラメーターを操作する円形ノブウィジェット。
/// - 上方向ドラッグで増加・下方向で減少（Shiftで微調整）
/// - ダブルクリック / Ctrl+クリックでデフォルト値にリセット
/// - ホバーでパラメーター名と現在値をツールチップ表示
#[must_use = "ノブは ui.add(widget) で配置してください"]
pub(crate) struct Knob<'a, P: Param> {
    param: &'a P,
    setter: &'a ParamSetter<'a>,
    diameter: f32,
}

impl<'a, P: Param> Knob<'a, P> {
    pub(crate) fn for_param(param: &'a P, setter: &'a ParamSetter<'a>) -> Self {
        Self {
            param,
            setter,
            diameter: 34.0,
        }
    }

    pub(crate) fn with_diameter(mut self, diameter: f32) -> Self {
        self.diameter = diameter;
        self
    }

    fn normalized(&self) -> f32 {
        self.param.modulated_normalized_value()
    }

    fn set_normalized(&self, normalized: f32) {
        let value = self.param.preview_plain(normalized.clamp(0.0, 1.0));
        if value != self.param.modulated_plain_value() {
            self.setter.set_parameter(self.param, value);
        }
    }
}

impl<P: Param> Widget for Knob<'_, P> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::splat(self.diameter), Sense::click_and_drag());

        if response.drag_started() {
            self.setter.begin_set_parameter(self.param);
        }
        if response.dragged() {
            let speed = if ui.input(|i| i.modifiers.shift) {
                DRAG_SPEED * GRANULAR_MULT
            } else {
                DRAG_SPEED
            };
            // 画面のy軸は下向きなので、上方向ドラッグ（delta.y<0）で値が増加する
            let change = -response.drag_delta().y * speed;
            if change != 0.0 {
                self.set_normalized(self.normalized() + change);
                response.mark_changed();
            }
        }
        if response.drag_stopped() {
            self.setter.end_set_parameter(self.param);
        }
        // ダブルクリック / Ctrl+クリックでデフォルト値へリセット
        if response.double_clicked() || (response.clicked() && ui.input(|i| i.modifiers.command)) {
            self.setter.begin_set_parameter(self.param);
            self.setter
                .set_parameter(self.param, self.param.default_plain_value());
            self.setter.end_set_parameter(self.param);
            response.mark_changed();
        }

        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&response);
            let color = visuals.fg_stroke.color;
            let center = rect.center();
            let radius = self.diameter / 2.0 - 2.0;
            let painter = ui.painter();

            // 外周の円
            painter.circle_stroke(center, radius, Stroke::new(1.5, color));

            // 中心から外周へ伸びる指針
            let angle = (self.normalized() - 0.5) * SWEEP_DEG.to_radians();
            let dir = Vec2::new(angle.sin(), -angle.cos());
            let inner: Pos2 = center + dir * (radius * 0.35);
            let outer: Pos2 = center + dir * (radius * 0.92);
            painter.line_segment([inner, outer], Stroke::new(2.0, color));
        }

        response.on_hover_text(format!("{}: {}", self.param.name(), self.param.to_string()))
    }
}
