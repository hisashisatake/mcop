use egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::ipc;
use crate::shift_keys;

/// 白鍵のオクターブ内オフセット（C,D,E,F,G,A,B）。
const WHITE_OFFSETS: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
/// 画面に表示するオクターブ数。
const OCTAVES_VISIBLE: i32 = 4;
/// 表示する白鍵の総数（4オクターブ×7）。
const WHITE_COUNT: usize = (OCTAVES_VISIBLE * 7) as usize;
/// 白鍵どうしの境界数（黒鍵が入りうる位置の総数）。
const BLACK_SLOT_COUNT: usize = WHITE_COUNT - 1;
/// タイピングキーを割り当てる開始位置（白鍵インデックス）。7=2オクターブめの先頭
/// （ZがC4になるよう、初期base_octave=3と組み合わせる。1オクターブめは表示のみ＝マウス専用）。
const TYPING_WHITE_START: usize = 7;

/// ZXCV行（白鍵側、Z〜/まで全10キー）。
const WHITE_KEYS: [(egui::Key, &str); 10] = [
    (egui::Key::Z, "Z"),
    (egui::Key::X, "X"),
    (egui::Key::C, "C"),
    (egui::Key::V, "V"),
    (egui::Key::B, "B"),
    (egui::Key::N, "N"),
    (egui::Key::M, "M"),
    (egui::Key::Comma, ","),
    (egui::Key::Period, "."),
    (egui::Key::Slash, "/"),
];
/// ASDF行（黒鍵側、A〜;まで全10キー）。先頭の"A"はタイピング範囲の最初の白鍵の手前
/// （常にB-C間=黒鍵なし）に対応するため常に無効。残り9キーがその白鍵から続く境界に
/// 割り当たり、黒鍵が存在しない境界（E-F間・B-C間）に当たるキーは自動的に無効になる。
const BLACK_KEYS: [(egui::Key, &str); 10] = [
    (egui::Key::A, "A"),
    (egui::Key::S, "S"),
    (egui::Key::D, "D"),
    (egui::Key::F, "F"),
    (egui::Key::G, "G"),
    (egui::Key::H, "H"),
    (egui::Key::J, "J"),
    (egui::Key::K, "K"),
    (egui::Key::L, "L"),
    (egui::Key::Semicolon, ";"),
];

const MIN_BASE_OCTAVE: i32 = 0;
const MAX_BASE_OCTAVE: i32 = 7;
/// 初期base_octave。TYPING_WHITE_START(=7、2オクターブめ先頭)がC4になる値
/// （octave_number = base_octave + TYPING_WHITE_START/7 = base_octave + 1 = 4）。
const DEFAULT_BASE_OCTAVE: i32 = 3;

/// 鍵盤の状態（表示オクターブ＋押下状態。フレームをまたいだエッジ検出に使う）。
pub struct KeyboardState {
    base_octave: i32,
    white_active: [bool; WHITE_COUNT],
    black_active: [bool; BLACK_SLOT_COUNT],
}

impl KeyboardState {
    pub fn new() -> Self {
        Self {
            base_octave: DEFAULT_BASE_OCTAVE,
            white_active: [false; WHITE_COUNT],
            black_active: [false; BLACK_SLOT_COUNT],
        }
    }

    fn shift_octave(&mut self, delta: i32) {
        self.base_octave = (self.base_octave + delta).clamp(MIN_BASE_OCTAVE, MAX_BASE_OCTAVE);
    }
}

/// 表示中の白鍵`gi`番目(0始まり)が属するオクターブ番号（Cn表記のn）。
fn octave_number(base_octave: i32, gi: usize) -> i32 {
    base_octave + (gi / 7) as i32
}

/// 表示中の白鍵`gi`番目(0始まり)のMIDIノート番号。
fn white_midi(base_octave: i32, gi: usize) -> i32 {
    12 * (octave_number(base_octave, gi) + 1) + WHITE_OFFSETS[gi % 7]
}

/// 白鍵`gi`と`gi+1`の間に黒鍵が存在する場合、そのMIDIノート番号を返す
/// （E-F間・B-C間は半音差が1のため黒鍵なし＝None）。
fn black_midi_between(base_octave: i32, gi: usize) -> Option<i32> {
    let w0 = white_midi(base_octave, gi);
    let w1 = white_midi(base_octave, gi + 1);
    (w1 - w0 == 2).then_some(w0 + 1)
}

/// 白鍵`gi`番目に割り当たるタイピングキー（範囲外はNone＝マウス専用の表示鍵）。
fn white_key_binding(gi: usize) -> Option<(egui::Key, &'static str)> {
    let li = gi as i32 - TYPING_WHITE_START as i32;
    (0..10).contains(&li).then(|| WHITE_KEYS[li as usize])
}

/// 白鍵境界`b`（白鍵bとb+1の間）に割り当たるタイピングキー（範囲外・無効境界はNone）。
fn black_key_binding(b: usize) -> Option<(egui::Key, &'static str)> {
    let li = b as i32 - TYPING_WHITE_START as i32 + 1;
    (0..10).contains(&li).then(|| BLACK_KEYS[li as usize])
}

fn midi_to_freq(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0)
}

/// ZXCV(白鍵)/ASDF(黒鍵)の全キーで鍵盤をシミュレートする幅広ミニキーボード。
/// 画面下に直接張り付く想定（呼び出し側でフレーム無しのPanel::bottomに入れること）。
/// 4オクターブ分を表示し、左右端のボタン（または左右Shiftキー）でオクターブを移動できる。
/// タイピング操作は2オクターブめ（ZがC4になる位置）のみ反応し、それ以外の表示鍵は
/// マウスクリックのみで発音する。
pub fn draw_keyboard(ui: &mut egui::Ui, state: &mut KeyboardState) {
    if shift_keys::take_left_edge() {
        state.shift_octave(-1);
    }
    if shift_keys::take_right_edge() {
        state.shift_octave(1);
    }

    let label_h = 14.0;
    let white_h = 64.0;
    let black_h = white_h * 0.62;
    let button_w = 28.0;
    let total_h = label_h + white_h;

    ui.spacing_mut().item_spacing.x = 0.0;
    ui.horizontal(|ui| {
        if ui
            .add_sized([button_w, total_h], egui::Button::new("◀"))
            .on_hover_text("オクターブダウン（左Ctrl）")
            .clicked()
        {
            state.shift_octave(-1);
        }

        let avail = (ui.available_width() - button_w).max(0.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(avail, total_h), Sense::hover());
        let label_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), label_h));
        let piano_rect = Rect::from_min_size(
            Pos2::new(rect.left(), rect.top() + label_h),
            Vec2::new(rect.width(), white_h),
        );
        draw_octave_labels(ui, label_rect, state.base_octave);
        draw_piano(ui, piano_rect, white_h, black_h, state);

        if ui
            .add_sized([button_w, total_h], egui::Button::new("▶"))
            .on_hover_text("オクターブアップ（右Ctrl）")
            .clicked()
        {
            state.shift_octave(1);
        }
    });
}

/// 鍵盤の外側（上）に、表示中の各オクターブの先頭C（C3/C4/C5...）をラベル表示する。
/// オクターブ移動のたびに`base_octave`から再計算されるため、表示は常に最新を反映する。
/// Zキーが現在割り当たっているオクターブ（TYPING_WHITE_START）のラベルだけ色を変える。
fn draw_octave_labels(ui: &mut egui::Ui, rect: Rect, base_octave: i32) {
    let white_w = rect.width() / WHITE_COUNT as f32;
    let painter = ui.painter();
    for gi in (0..WHITE_COUNT).step_by(7) {
        let x = rect.left() + gi as f32 * white_w;
        let label = format!("C{}", octave_number(base_octave, gi));
        let color = if gi == TYPING_WHITE_START {
            Color32::from_rgb(220, 110, 70)
        } else {
            Color32::from_gray(170)
        };
        painter.text(
            Pos2::new(x + 3.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::monospace(10.0),
            color,
        );
    }
}

fn draw_piano(ui: &mut egui::Ui, rect: Rect, white_h: f32, black_h: f32, state: &mut KeyboardState) {
    let white_w = rect.width() / WHITE_COUNT as f32;
    let black_w = white_w * 0.62;
    let ctx = ui.ctx().clone();
    let base_octave = state.base_octave;

    // 白鍵（先にすべて描画し、黒鍵を後段で上書き描画する）。
    for gi in 0..WHITE_COUNT {
        let key_binding = white_key_binding(gi);
        let x0 = rect.left() + gi as f32 * white_w;
        let key_rect = Rect::from_min_size(Pos2::new(x0, rect.top()), Vec2::new(white_w - 1.0, white_h));
        let midi = white_midi(base_octave, gi);
        let channel = 300 + gi;
        let active = &mut state.white_active[gi];
        interact_key(ui, &ctx, key_rect, false, key_binding, midi, channel, active);
    }

    // 黒鍵（白鍵境界ごとに、実在する位置だけ描画する）。
    for b in 0..BLACK_SLOT_COUNT {
        let Some(midi) = black_midi_between(base_octave, b) else { continue };
        let key_binding = black_key_binding(b);
        let center_x = rect.left() + (b + 1) as f32 * white_w;
        let key_rect = Rect::from_min_size(
            Pos2::new(center_x - black_w / 2.0, rect.top()),
            Vec2::new(black_w, black_h),
        );
        let channel = 400 + b;
        let active = &mut state.black_active[b];
        interact_key(ui, &ctx, key_rect, true, key_binding, midi, channel, active);
    }
}

#[allow(clippy::too_many_arguments)]
fn interact_key(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    key_rect: Rect,
    is_black: bool,
    key_binding: Option<(egui::Key, &'static str)>,
    midi: i32,
    channel: usize,
    active: &mut bool,
) {
    let id = ui.make_persistent_id(("editor_kbd", is_black, channel));
    let response = ui.interact(key_rect, id, Sense::click_and_drag());

    let kbd_down = key_binding.is_some_and(|(k, _)| ctx.input(|i| i.key_down(k)));
    let mouse_down = response.is_pointer_button_down_on();
    let down_now = kbd_down || mouse_down;

    if down_now && !*active {
        ipc::note_on(channel, midi_to_freq(midi));
    } else if !down_now && *active {
        ipc::note_off(channel);
    }
    *active = down_now;

    let painter = ui.painter();
    let (fill, text_color) = if down_now {
        (Color32::from_rgb(110, 170, 255), Color32::BLACK)
    } else if is_black {
        (Color32::from_rgb(25, 25, 25), Color32::from_gray(200))
    } else {
        (Color32::from_gray(235), Color32::from_gray(70))
    };
    painter.rect_filled(key_rect, 2.0, fill);
    painter.rect_stroke(key_rect, 2.0, Stroke::new(1.0, Color32::from_gray(20)), egui::StrokeKind::Inside);
    if let Some((_, label)) = key_binding {
        painter.text(
            Pos2::new(key_rect.center().x, key_rect.bottom() - 10.0),
            egui::Align2::CENTER_BOTTOM,
            label,
            FontId::monospace(10.0),
            text_color,
        );
    }
}
