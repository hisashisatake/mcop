use egui::{self, Color32, Id, LayerId, Order, Painter, Pos2, Rect, Sense, Stroke, Vec2};

use crate::param_handle::IntParamHandle;

/// 質感LFO Destination（0=未接続/1=Pitch/2=Volume/3=TL/4=Cutoff、`FmLfoDestination`と同じ並び、
/// 2026-08-18並べ替え後）。`dest_index`は配列添字ではなく**destination値そのもの**として
/// 使われる（`texture_lfo_dest_jack`が`handle.set(dest_index as i32)`で直接書き込み、
/// `finish_texture_lfo_patchbay`が`layout.dests.get(current)`で値そのまま引く）。
/// index 0（Unplugged）には専用ジャックが無いため、この配列のindex 0は未使用のダミー。
const DEST_COLORS: [Color32; 5] = [
    Color32::from_rgb(0x40, 0x40, 0x40), // UNPLUGGED: 未使用（専用ジャックが無いため参照されない）
    Color32::from_rgb(0xE8, 0x6A, 0x5C), // PITCH: 赤
    Color32::from_rgb(0x5C, 0xB8, 0xE8), // VOLUME: 青
    Color32::from_rgb(0xE8, 0xC7, 0x5C), // TL: 黄
    Color32::from_rgb(0x7C, 0xD8, 0x7C), // CUTOFF: 緑
];

/// どこにも接続されていない状態を表すdestination値。`FmLfoDestination::Unplugged`
/// （sound-fm、discriminant=0）と一致させること。4個の行き先ジャックのような専用の
/// 描画は持たず、ケーブルをTEXTURE LFOパネル自身（`source_panel_rect`）へドロップすることで
/// この状態に遷移する。
const UNPLUGGED: usize = 0;

/// destinationの最大値（Cutoff）。`handle.value()`をクランプする上限に使う
/// （`UNPLUGGED`が0になったため、こちらは別定数として持つ必要がある。
/// 誤って`UNPLUGGED`を上限クランプに使うと全値が0へ潰れ、パッチベイが常時未接続になる）。
const DEST_MAX: usize = 4;

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
    /// TEXTURE LFOパネル自体の外形矩形（呼び出し側の`draw_panel`/`gen_panel`が
    /// `set_source_panel_rect`で埋める）。ここへケーブルをドロップすると未接続化する。
    source_panel_rect: Option<Rect>,
    /// index 0（Unplugged）は専用ジャックが無いため常に`None`のまま
    /// （`DEST_COLORS`と同じ「index 0はダミー」規約）。
    dests: [Option<Pos2>; 5],
    drag_live_pos: Option<Pos2>,
    drag_release_pos: Option<Pos2>,
}

impl JackLayout {
    pub fn new() -> Self {
        Self::default()
    }

    /// TEXTURE LFOパネルの外形矩形を記録する（`Frame::show`の`response.rect`）。
    /// このパネルの`body`を描画する呼び出し側（`interpret.rs`のプレビュー描画、
    /// `codegen.rs`が生成する`draw_param_panel`）だけが呼ぶ想定。
    pub fn set_source_panel_rect(&mut self, rect: Rect) {
        self.source_panel_rect = Some(rect);
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

/// 質感LFOの出力ジャック。TEXTURE LFOパネル内で1回呼ぶ。
/// 未接続時（行き先ジャックが全て非アクティブでドラッグできない）でも新規接続を作れるよう、
/// こちら側からもドラッグで行き先ジャックへ挿せる（着地判定は`finish_texture_lfo_patchbay`で
/// 行き先ジャックのドラッグと共通、TEXTURE LFOパネル自身へ戻せば未接続化も同様に効く）。
pub fn texture_lfo_source_jack(ui: &mut egui::Ui, layout: &mut JackLayout) {
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(JACK_RADIUS * 2.0), Sense::click_and_drag());
        let center = rect.center();
        layout.source = Some(center);

        let painter = ui.painter();
        painter.circle_filled(center, JACK_RADIUS, Color32::LIGHT_GRAY);
        painter.circle_stroke(center, JACK_RADIUS, Stroke::new(1.5, Color32::WHITE));
        painter.circle_filled(center, JACK_RADIUS * 0.35, Color32::from_black_alpha(170));

        if resp.dragged() {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                layout.drag_live_pos = Some(p);
            }
        }
        if resp.drag_stopped() {
            layout.drag_release_pos = Some(ui.input(|i| i.pointer.interact_pos()).unwrap_or(center));
        }
        let cursor = if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab };
        let _ = resp.on_hover_cursor(cursor);

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
    let current = handle.value().clamp(0, DEST_MAX as i32) as usize;
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
/// ドラッグ解放時の着地判定（TEXTURE LFOパネル自身へのドロップ＝未接続化、または
/// 最寄りジャックへのスナップ）と、実際のケーブル描画を行う。
/// ケーブルはForegroundレイヤーへ描くため、途中のパネル（`ui.group`）のクリップ矩形を
/// またいでも全区間が見える。クリップ自体はスクロールビューポート（`ui.clip_rect()`）に
/// 合わせるため、スクロールで隠れた領域まで描画がはみ出すことはない。
pub fn finish_texture_lfo_patchbay(ui: &mut egui::Ui, handle: &dyn IntParamHandle, layout: JackLayout) {
    if let Some(release_pos) = layout.drag_release_pos {
        let current = handle.value().clamp(0, DEST_MAX as i32) as usize;
        // TEXTURE LFOパネル自身（ケーブルの出所）へドロップした場合は無条件に未接続化する。
        // 行き先ジャックはこのパネルの外にあるため、他の候補と競合することはない。
        let dropped_on_source_panel = layout.source_panel_rect.is_some_and(|r| r.contains(release_pos));
        let target = if dropped_on_source_panel {
            Some(UNPLUGGED)
        } else {
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
            best.map(|(i, _)| i)
        };
        if let Some(i) = target {
            if i != current {
                handle.begin_edit();
                handle.set(i as i32);
                handle.end_edit();
                trigger_wobble(ui);
            }
        }
    }

    let Some(source) = layout.source else {
        return;
    };

    let clip_rect = ui.clip_rect();
    let layer_id = LayerId::new(Order::Foreground, Id::new(CABLE_LAYER_KEY));
    let painter = Painter::new(ui.ctx().clone(), layer_id, clip_rect);

    if let Some(live) = layout.drag_live_pos {
        // ドラッグ中は未接続（current==UNPLUGGED、行き先ジャックの現在地なし）でも
        // LFO OUTジャック自身から新規接続を引き出せるよう、接続状態によらず灰色の
        // 追従ケーブルを描く（行き先ジャック側からのドラッグと同じ見た目にする）。
        draw_cable(&painter, source, live, Color32::GRAY, Vec2::ZERO);
    } else {
        let current = handle.value().clamp(0, DEST_MAX as i32) as usize;
        // 未接続（current==UNPLUGGED==0）時は`dests[0]`が常に`None`（専用ジャックが無いため
        // 一度も書き込まれない）なので、確定済みケーブルは描かない。
        if let Some(settled) = layout.dests.get(current).copied().flatten() {
            let sway = wobble_offset(ui);
            draw_cable(&painter, source, settled, DEST_COLORS[current], sway);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `UNPLUGGED`(0)/`DEST_MAX`(4、Cutoff)/`DEST_COLORS`(5要素)/`JackLayout::dests`(5要素)の
    /// 整合を固定する。ここがずれると`clamp(0, UNPLUGGED as i32)`のような書き方へ先祖返りして
    /// 全destinationが0へ潰れる、あるいは`DEST_COLORS[dest_index]`が範囲外パニックする
    /// （2026-08-18のFmLfoDestination並べ替えで実際に踏んだ設計判断）。
    #[test]
    fn unplugged_and_dest_max_are_consistent_with_color_table() {
        assert_eq!(UNPLUGGED, 0, "UNPLUGGEDはFmLfoDestination::Unplugged(discriminant=0)と一致させること");
        assert_eq!(DEST_MAX, 4, "DEST_MAXはFmLfoDestination::Cutoff(discriminant=4、最大値)と一致させること");
        assert_eq!(DEST_COLORS.len(), DEST_MAX + 1, "DEST_COLORSは0..=DEST_MAXの全destination値を添字で引ける長さが必要");
        assert_eq!(JackLayout::new().dests.len(), DEST_MAX + 1, "destsもDEST_COLORSと同じ長さが必要");
    }
}
