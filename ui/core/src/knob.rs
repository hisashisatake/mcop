use egui::{self, Pos2, Response, Sense, Stroke, Ui, Vec2, Widget};

use crate::param_handle::{BoolParamHandle, IntParamHandle};

/// ノブのスイープ角（度）。下方向に90度のギャップを設け、7時(最小)→12時(中間)→5時(最大)を描く。
const SWEEP_DEG: f32 = 270.0;
/// 通常ドラッグ時、1ピクセルあたりの正規化値の変化量（上方向ドラッグで増加）。
const DRAG_SPEED: f32 = 0.005;
/// Shift+ドラッグ時の微調整倍率。
const GRANULAR_MULT: f32 = 0.1;

/// パラメーターハンドルを操作する円形ノブウィジェット。
/// - 上方向ドラッグで増加・下方向で減少（Shiftで微調整）
/// - ダブルクリック / Ctrl+クリックでデフォルト値にリセット
/// - ホバーでパラメーター名と現在値をツールチップ表示
#[must_use = "ノブは ui.add(widget) で配置してください"]
pub struct Knob<'a> {
    handle: &'a dyn IntParamHandle,
    diameter: f32,
}

impl<'a> Knob<'a> {
    pub fn for_handle(handle: &'a dyn IntParamHandle) -> Self {
        Self {
            handle,
            diameter: 34.0,
        }
    }

    pub fn with_diameter(mut self, diameter: f32) -> Self {
        self.diameter = diameter;
        self
    }

    fn normalized(&self) -> f32 {
        let (min, max) = (self.handle.min(), self.handle.max());
        if max == min {
            0.0
        } else {
            (self.handle.value() - min) as f32 / (max - min) as f32
        }
    }

    fn set_normalized(&self, normalized: f32) {
        let (min, max) = (self.handle.min(), self.handle.max());
        let value = min + (normalized.clamp(0.0, 1.0) * (max - min) as f32).round() as i32;
        if value != self.handle.value() {
            self.handle.set(value);
        }
    }
}

impl Widget for Knob<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::splat(self.diameter), Sense::click_and_drag());

        if response.drag_started() {
            self.handle.begin_edit();
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
            self.handle.end_edit();
        }
        // ダブルクリック / Ctrl+クリックでデフォルト値へリセット
        if response.double_clicked() || (response.clicked() && ui.input(|i| i.modifiers.command)) {
            self.handle.begin_edit();
            self.handle.set(self.handle.default());
            self.handle.end_edit();
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

        response.on_hover_text(format!("{}: {}", self.handle.name(), self.handle.display()))
    }
}

/// 数値欄（`spin_control`の`desired_width`）の既定幅（px）。`TextStyle::Small`で4文字ぶん。
///
/// 3文字ぶん（24px）だったが、バイポーラノブ（`BipolarHandle`、`-128`〜`+127`）の導入で
/// 符号込み4文字が入らず`+127`が見切れたため1文字ぶん広げた。0〜255の単極性パラメーターも
/// 同じ幅を使い、横に並んだ数値欄の見た目を揃える（`TIME(m)`欄だけは6桁必要なので別枠）。
pub const SPIN_WIDTH_DEFAULT: f32 = 32.0;

/// スピンボタン（`repeat_button`）の一辺（px）。`ui.add_sized`で固定している値。
const SPIN_BUTTON_SIZE: f32 = 12.0;
/// `spin_control`内のウィジェット間隔（px）。
const SPIN_ITEM_SPACING: f32 = 2.0;

/// `spin_control`が実際に占める横幅（px）。
///
/// `TextEdit::desired_width`はテキスト領域ではなく**外形幅**（eguiは`allocate_width - margin`を
/// 内側のテキスト領域に使う。egui 0.34の`text_edit/builder.rs`参照）なので、
/// ボタン2個と間隔2個を足すだけで求まり、マージンを別途足してはいけない。
pub const fn spin_control_width(desired_width: f32) -> f32 {
    SPIN_BUTTON_SIZE * 2.0 + SPIN_ITEM_SPACING * 2.0 + desired_width
}

/// `knob`セル内のスピン行の実幅（px）。
const SPIN_ROW_WIDTH: f32 = spin_control_width(SPIN_WIDTH_DEFAULT);

/// ラベル付きノブ1セルの固定サイズ（px）。幅はスピン行(`SPIN_ROW_WIDTH`=60)がちょうど収まる値。
///
/// **`ui-codegen`の`parse.rs`にある`<knob>`の宣言サイズと一致させること**
/// （ui-codegenはegui非依存でこの定数を参照できないため、手で同期する。`<checkbox>`等と同じ扱い）。
pub const KNOB_CELL_SIZE: egui::Vec2 = egui::vec2(62.0, 66.0);

#[cfg(test)]
mod geometry_tests {
    use super::*;

    /// スピン行がセル幅を超えないこと。超えると隣のノブへはみ出して重なる。
    /// 数値欄幅（`SPIN_WIDTH_DEFAULT`）を広げるときはここで気づけるようにしておく。
    #[test]
    fn spin_row_fits_in_knob_cell() {
        assert!(
            SPIN_ROW_WIDTH <= KNOB_CELL_SIZE.x,
            "スピン行{SPIN_ROW_WIDTH}pxがセル幅{}pxを超えている。KNOB_CELL_SIZEとui-codegenのparse.rsを揃えて広げること",
            KNOB_CELL_SIZE.x
        );
    }

    /// ノブとスピン行の中心が一致すること（`knob`の`add_space`が入れる余白の検算）。
    #[test]
    fn knob_and_spin_row_share_horizontal_center() {
        let pad = ((KNOB_CELL_SIZE.x - SPIN_ROW_WIDTH) * 0.5).max(0.0);
        let knob_center = KNOB_CELL_SIZE.x * 0.5;
        let spin_center = pad + SPIN_ROW_WIDTH * 0.5;
        assert!(
            (knob_center - spin_center).abs() < 0.01,
            "ノブ中心{knob_center} と スピン行中心{spin_center} がずれている"
        );
    }
}

/// ラベル付きノブを1セル（`KNOB_CELL_SIZE`）に配置する。
/// 名称をノブの左上に置き、ノブ・数値欄＋スピンを縦に並べる。
/// セルを固定サイズ・固定レイアウトにすることで、横に並べたノブの高さが揃う。
pub fn knob(ui: &mut egui::Ui, handle: &dyn IntParamHandle, label: &str) {
    ui.allocate_ui_with_layout(
        KNOB_CELL_SIZE,
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            // 名称（左上）
            ui.label(egui::RichText::new(label).size(9.0));
            // ノブ（水平中央）
            ui.vertical_centered(|ui| {
                ui.add(Knob::for_handle(handle).with_diameter(28.0));
            });
            // 数値欄＋スピンボタン。ノブはセル幅の中央に置かれるので、スピン行を左寄せのまま
            // にすると両者の中心が「(セル幅 - 行幅) / 2」だけずれる（実機で違和感として指摘された）。
            // `vertical_centered`で包んでも中央寄せにはならない——`spin_control`は内部で
            // `ui.horizontal`を使い、その子uiは利用可能幅いっぱいのmax_rectを左端から取るため。
            // そこで差分の半分だけ明示的に空けて中心を揃える。
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(((KNOB_CELL_SIZE.x - SPIN_ROW_WIDTH) * 0.5).max(0.0));
                spin_control(ui, handle, egui::TextStyle::Small, SPIN_WIDTH_DEFAULT);
            });
        },
    );
}

/// `BoolParamHandle`をチェックボックスとして表示する（Loop/Curve/AM等の真偽パラメーター向け）。
pub fn bool_checkbox(ui: &mut egui::Ui, handle: &dyn BoolParamHandle, label: &str) {
    let mut value = handle.value();
    if ui.checkbox(&mut value, label).changed() {
        handle.begin_edit();
        handle.set(value);
        handle.end_edit();
    }
}

/// 押下開始で1回、その後は初期遅延を挟んで一定間隔で繰り返し`true`を返すボタン（長押しリピート）。
/// 一度ボタン上で押し始めたら、マウスがボタン矩形から外れてもマウスボタンを離すまでリピートを継続する。
pub(crate) fn repeat_button(ui: &mut egui::Ui, label: &str) -> bool {
    const INITIAL_DELAY: f64 = 0.4;
    const REPEAT_INTERVAL: f64 = 0.05;

    // ホバー/フォーカス時にボタンが膨張して縦幅が変わらないようにする（レイアウトのちらつき防止）
    ui.spacing_mut().button_padding = egui::vec2(1.0, 0.0);
    {
        let widgets = &mut ui.style_mut().visuals.widgets;
        widgets.inactive.expansion = 0.0;
        widgets.hovered.expansion = 0.0;
        widgets.active.expansion = 0.0;
    }
    // 固定サイズで確保し、状態によらず同じ縦幅にする
    let response = ui.add_sized(
        [12.0, 12.0],
        egui::Button::new(egui::RichText::new(label).size(9.0)),
    );
    let active_id = response.id.with("spin_active");
    let next_id = response.id.with("spin_next");

    let now = ui.input(|i| i.time);
    let active = ui
        .memory(|m| m.data.get_temp::<bool>(active_id))
        .unwrap_or(false);

    // ボタン上で新たに押下開始：即1回発火し、リピート状態を立てる
    if response.is_pointer_button_down_on() && !active {
        ui.memory_mut(|m| {
            m.data.insert_temp(active_id, true);
            m.data.insert_temp(next_id, now + INITIAL_DELAY);
        });
        ui.ctx().request_repaint();
        return true;
    }

    if active {
        // マウスボタンを離したらリピート停止（ボタン矩形を外れても押している限り継続する）
        if !ui.input(|i| i.pointer.primary_down()) {
            ui.memory_mut(|m| {
                m.data.remove::<bool>(active_id);
                m.data.remove::<f64>(next_id);
            });
            return false;
        }
        // 押下継続中はイベントが来なくても再描画を要求し続け、時間経過でリピート発火する
        ui.ctx().request_repaint();
        let next = ui
            .memory(|m| m.data.get_temp::<f64>(next_id))
            .unwrap_or(now);
        if now >= next {
            ui.memory_mut(|m| m.data.insert_temp(next_id, now + REPEAT_INTERVAL));
            return true;
        }
    }
    false
}

/// 直接入力できる数値欄に＋−ボタンを付けたスピンコントロール（横一列）。
/// - 数値欄：通常は現在値を表示、フォーカス中はユーザー入力をeguiのmemoryへ保持し、
///   Enterでパースして`[min, max]`へクランプ後書き戻す。
///   フォーカス中はカーソルキー↑↓でも1ステップずつ増減できる
/// - ＋−：長押しで連続増減（±1、`[min, max]`へクランプ）
/// `text_style`は数値欄のフォントサイズ（ノブの並びでは`TextStyle::Small`、他のUI要素と大きさを
/// 揃えたい単体使用では`TextStyle::Body`等を指定する）。`desired_width`は数値欄(px)。
/// 桁数はハンドルごとに異なる（バイポーラの符号込み4文字からミリ秒表示の6桁まで）ため、
/// 呼び出し側が明示する（旧実装は`text_style`から24px/44pxの2択を自動選択していたが、
/// TimeEgのTIME欄が300000msまで表示するようになり2択では足りなくなったため呼び出し側指定へ変更）。
/// 特別な理由がなければ`SPIN_WIDTH_DEFAULT`を渡すこと。
pub fn spin_control(ui: &mut egui::Ui, handle: &dyn IntParamHandle, text_style: egui::TextStyle, desired_width: f32) {
    let (min, max) = (handle.min(), handle.max());
    let set = |value: i32| {
        handle.begin_edit();
        handle.set(value.clamp(min, max));
        handle.end_edit();
    };
    let step_up = || (handle.value() + 1).clamp(min, max);
    let step_down = || (handle.value() - 1).clamp(min, max);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;

        // ---- − ボタン（左・長押しリピート） ----
        if repeat_button(ui, "\u{2212}") {
            set(step_down());
        }

        // ---- 数値入力欄 ----
        let id = ui.next_auto_id();
        ui.skip_ahead_auto_ids(1);
        let buffer_id = id.with("buf");
        let has_focus = ui.memory(|m| m.has_focus(id));
        let mut text = if has_focus {
            ui.memory(|m| m.data.get_temp::<String>(buffer_id))
                .unwrap_or_else(|| handle.display())
        } else {
            handle.display()
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(desired_width)
                .font(text_style)
                .horizontal_align(egui::Align::Center),
        );
        if response.has_focus() {
            ui.memory_mut(|m| m.data.insert_temp(buffer_id, text.clone()));
            // カーソルキー↑↓で増減（バッファを破棄して次フレームに新値を表示させる）
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                set(step_up());
                ui.memory_mut(|m| m.data.remove::<String>(buffer_id));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                set(step_down());
                ui.memory_mut(|m| m.data.remove::<String>(buffer_id));
            }
        }
        if response.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(value) = text.trim().parse::<i32>() {
                    set(value);
                }
            }
            ui.memory_mut(|m| m.data.remove::<String>(buffer_id));
        }

        // ---- ＋ボタン（右・長押しリピート） ----
        if repeat_button(ui, "+") {
            set(step_up());
        }
    });
}
