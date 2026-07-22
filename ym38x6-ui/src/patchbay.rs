use egui::{self, Color32, Id, LayerId, Order, Painter, Pos2, Sense, Stroke, Vec2};

use crate::param_handle::IntParamHandle;

/// 質感LFO Destination（0=Pitch/1=Volume/2=TL/3=Cutoff、`Ym38x6LfoDestination`と同じ並び）。
const DEST_COLORS: [Color32; 4] = [
    Color32::from_rgb(0xE8, 0x6A, 0x5C), // PITCH: 赤
    Color32::from_rgb(0x5C, 0xB8, 0xE8), // VOLUME: 青
    Color32::from_rgb(0xE8, 0xC7, 0x5C), // TL: 黄
    Color32::from_rgb(0x7C, 0xD8, 0x7C), // CUTOFF: 緑
];

const JACK_RADIUS: f32 = 9.0;
/// クリック当たり判定・ドラッグ着地判定に使う半径（描画より広めに取る）。
const HIT_RADIUS: f32 = 20.0;
const CABLE_CORE_WIDTH: f32 = 5.0;
const CABLE_OUTLINE_WIDTH: f32 = 8.5;

/// 揺れアニメーションの開始時刻を保存するegui memoryキー。
const WOBBLE_ID_KEY: &str = "ym38x6_tx_patchbay_wobble_t";
const CABLE_LAYER_KEY: &str = "ym38x6_tx_patchbay_cable_layer";

/// 質感LFOパッチベイの1フレーム分のジャック配置・ドラッグ状態。
/// `draw_param_panel`内で各パネルを描画しながら埋め、最後に`finish_texture_lfo_patchbay`へ渡す。
///
/// パネルはスクロール内で上から順に描画されるため、出力ジャック（TEXTURE LFO）と
/// 4つの行き先ジャック（CHANNEL=TL / PITCH FG / CUTOFF FG / GAIN FG）は描画タイミングが
/// ばらばらになる。ドラッグ解放時の着地判定は全ジャック位置が出揃うフレーム末尾まで遅延させる
/// （同一フレーム内で完結するため、フレームをまたぐ古い座標を参照する心配はない）。
#[derive(Default)]
pub struct JackLayout {
    source: Option<Pos2>,
    dests: [Option<Pos2>; 4],
    drag_live_pos: Option<Pos2>,
    drag_release_pos: Option<Pos2>,
}

impl JackLayout {
    pub fn new() -> Self {
        Self::default()
    }
}

fn trigger_wobble(ui: &egui::Ui) {
    let now = ui.ctx().input(|i| i.time);
    ui.memory_mut(|m| m.data.insert_temp(Id::new(WOBBLE_ID_KEY), now));
}

/// 直近の接続変更からの経過時間に基づく減衰振動オフセット（「ぶらん」の揺れ）。
/// アニメーション中は`request_repaint`で再描画をリクエストし続ける。
fn wobble_offset(ui: &egui::Ui) -> Vec2 {
    let id = Id::new(WOBBLE_ID_KEY);
    let Some(t0) = ui.memory(|m| m.data.get_temp::<f64>(id)) else {
        return Vec2::ZERO;
    };
    let now = ui.ctx().input(|i| i.time);
    let dt = (now - t0) as f32;
    const DURATION: f32 = 0.8;
    if !(0.0..DURATION).contains(&dt) {
        return Vec2::ZERO;
    }
    ui.ctx().request_repaint();
    let decay = (-dt * 7.0).exp();
    let osc = (dt * 26.0).sin();
    Vec2::new(0.0, osc * decay * 14.0)
}

/// 指定色を暗くグレーへ寄せる（非アクティブなジャックの縁取り用）。
fn dim(c: Color32) -> Color32 {
    let f = 0.5;
    Color32::from_rgb((c.r() as f32 * f) as u8, (c.g() as f32 * f) as u8, (c.b() as f32 * f) as u8)
}

/// 太めのパッチケーブル（アウトライン＋コアの二重ベジェ）と両端の金具を描く。
/// `sway`は制御点に加える揺れオフセット（`wobble_offset`の戻り値、非アニメーション時はゼロ）。
fn draw_cable(painter: &Painter, a: Pos2, b: Pos2, color: Color32, sway: Vec2) {
    let k = (b.x - a.x).abs().max(30.0) * 0.5;
    let base_sag = 20.0;
    let p1 = Pos2::new(a.x + k, a.y + base_sag) + sway;
    let p2 = Pos2::new(b.x - k, b.y + base_sag) + sway;

    let outline = egui::epaint::CubicBezierShape::from_points_stroke(
        [a, p1, p2, b],
        false,
        Color32::TRANSPARENT,
        Stroke::new(CABLE_OUTLINE_WIDTH, Color32::from_black_alpha(160)),
    );
    painter.add(outline);
    let core = egui::epaint::CubicBezierShape::from_points_stroke(
        [a, p1, p2, b],
        false,
        Color32::TRANSPARENT,
        Stroke::new(CABLE_CORE_WIDTH, color),
    );
    painter.add(core);

    // ケーブル端末の金具。
    painter.circle_filled(a, 4.0, color);
    painter.circle_filled(b, 4.0, color);
}

/// 質感LFOの出力ジャック（固定・非ドラッグ）。TEXTURE LFOパネル内で1回呼ぶ。
pub fn texture_lfo_source_jack(ui: &mut egui::Ui, layout: &mut JackLayout) {
    ui.horizontal(|ui| {
        let (rect, _resp) = ui.allocate_exact_size(Vec2::splat(JACK_RADIUS * 2.0), Sense::hover());
        let center = rect.center();
        layout.source = Some(center);

        let painter = ui.painter();
        painter.circle_filled(center, JACK_RADIUS, Color32::LIGHT_GRAY);
        painter.circle_stroke(center, JACK_RADIUS, Stroke::new(1.5, Color32::WHITE));
        painter.circle_filled(center, JACK_RADIUS * 0.35, Color32::from_black_alpha(170));

        ui.label(egui::RichText::new("LFO OUT").size(9.0).strong());
    });
}

/// 質感LFOの行き先ジャック（Pitch=0/Volume=1/TL=2/Cutoff=3）。対応するパネル
/// （PITCH FG/GAIN FG/CHANNEL/CUTOFF FG）のヘッダー付近で呼ぶ。
///
/// 現在の行き先と一致する（`active`）場合はプラグとして描画し、ドラッグで別のジャックへ
/// 挿し直せる。一致しない場合は空のソケットとして描画し、クリックで即座にそのジャックへ
/// 繋ぎ替えられる。実際の接続変更（ドラッグの着地判定）は`finish_texture_lfo_patchbay`で
/// 全ジャック位置が出揃った後にまとめて行う。
pub fn texture_lfo_dest_jack(
    ui: &mut egui::Ui,
    handle: &dyn IntParamHandle,
    dest_index: usize,
    label: &str,
    layout: &mut JackLayout,
) {
    let current = handle.value().clamp(0, 3) as usize;
    let active = current == dest_index;
    let color = DEST_COLORS[dest_index];

    ui.horizontal(|ui| {
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::splat(JACK_RADIUS * 2.0), Sense::click_and_drag());
        let center = rect.center();
        layout.dests[dest_index] = Some(center);

        let painter = ui.painter();
        if active {
            painter.circle_filled(center, JACK_RADIUS, color);
            painter.circle_stroke(center, JACK_RADIUS, Stroke::new(1.5, Color32::WHITE));
            // 抜き挿しできることを示す小さなグリップの筋。
            painter.line_segment(
                [center + Vec2::new(-3.0, -2.0), center + Vec2::new(3.0, -2.0)],
                Stroke::new(1.0, Color32::from_black_alpha(150)),
            );
            painter.line_segment(
                [center + Vec2::new(-3.0, 2.0), center + Vec2::new(3.0, 2.0)],
                Stroke::new(1.0, Color32::from_black_alpha(150)),
            );

            if resp.dragged() {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    layout.drag_live_pos = Some(p);
                }
            }
            if resp.drag_stopped() {
                layout.drag_release_pos =
                    Some(ui.input(|i| i.pointer.interact_pos()).unwrap_or(center));
            }
            let cursor = if resp.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            };
            let _ = resp.on_hover_cursor(cursor);
        } else {
            painter.circle_filled(center, JACK_RADIUS, Color32::from_gray(40));
            painter.circle_stroke(center, JACK_RADIUS, Stroke::new(1.3, dim(color)));
            if resp.clicked() {
                handle.begin_edit();
                handle.set(dest_index as i32);
                handle.end_edit();
                trigger_wobble(ui);
            }
            let _ = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }

        ui.label(
            egui::RichText::new(label)
                .size(9.0)
                .color(if active { color } else { ui.style().visuals.weak_text_color() }),
        );
    });
}

/// 全ジャックの配置が出揃った後（`draw_param_panel`末尾）で1回呼ぶ。
/// ドラッグ解放時の着地判定（最寄りジャックへのスナップ）と、実際のケーブル描画を行う。
/// ケーブルはForegroundレイヤーへ描くため、途中のパネル（`ui.group`）のクリップ矩形を
/// またいでも全区間が見える。クリップ自体はスクロールビューポート（`ui.clip_rect()`）に
/// 合わせるため、スクロールで隠れた領域まで描画がはみ出すことはない。
pub fn finish_texture_lfo_patchbay(ui: &mut egui::Ui, handle: &dyn IntParamHandle, layout: JackLayout) {
    if let Some(release_pos) = layout.drag_release_pos {
        let current = handle.value().clamp(0, 3) as usize;
        let mut best: Option<(usize, f32)> = None;
        for (i, pos) in layout.dests.iter().enumerate() {
            if let Some(p) = pos {
                let d = p.distance(release_pos);
                let closer = match best {
                    Some((_, bd)) => d < bd,
                    None => true,
                };
                if d <= HIT_RADIUS && closer {
                    best = Some((i, d));
                }
            }
        }
        if let Some((i, _)) = best {
            if i != current {
                handle.begin_edit();
                handle.set(i as i32);
                handle.end_edit();
                trigger_wobble(ui);
            }
        }
    }

    let current = handle.value().clamp(0, 3) as usize;
    let (Some(source), Some(settled)) = (layout.source, layout.dests[current]) else {
        return;
    };

    let clip_rect = ui.clip_rect();
    let layer_id = LayerId::new(Order::Foreground, Id::new(CABLE_LAYER_KEY));
    let painter = Painter::new(ui.ctx().clone(), layer_id, clip_rect);

    if let Some(live) = layout.drag_live_pos {
        draw_cable(&painter, source, live, Color32::GRAY, Vec2::ZERO);
    } else {
        let sway = wobble_offset(ui);
        draw_cable(&painter, source, settled, DEST_COLORS[current], sway);
    }
}
