//! xml-panel-dsl のネイティブeguiプレビュー（`tools/xml-panel-dsl/index.html`のブラウザ版と対をなす）。
//!
//! ブラウザ版（`preview-wasm`、WebGLキャンバス）ではプレビューの分離・ドッキングが実現できない
//! （キャンバスを別ウィンドウへ移すとWebGLコンテキストが失われる）ため、その機能を含むフル版として
//! eframe nativeで新規提供する。プレビュー描画自体は`ui_core::interpret::draw_panel_from_ir`
//! （`preview-wasm`と共有、チップ非依存化済み）をそのまま呼ぶだけで、`panel.xml`の生成経路
//! （build.rs/interpret.rs）には一切手を入れない。

use std::path::Path;

use egui::text::{CCursor, CCursorRange};
use egui::widgets::text_edit::TextEditState;
use ui_core::interpret::{draw_panel_from_ir, HandleStore};

/// エディタでTabキーが押されたときのインデント幅（4スペース）。
const IND: &str = "    ";

/// `panel.xml`正本の既定パス（`CARGO_MANIFEST_DIR`基準、CWDに依存しない）。既定はop505側
/// （op505が主力、ym38x6は凍結。「ファイルを開く」で`ym38x6/ui/src/panel.xml`へ切り替えられる）。
fn default_xml_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ui/src/panel.xml")
}

/// 起動時は`panel.xml`正本をそのまま読む（このツールの主用途が正本の往復編集のため）。
/// 読めなかった場合はサンプルXMLで代用せず空で開始する（正本と紛らわしいコピーを持たない）。
fn load_default_xml() -> (String, String) {
    let path = default_xml_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => (text, "panel.xml".to_string()),
        Err(_) => (String::new(), "(未読込)".to_string()),
    }
}

/// eguiの既定フォントにはCJKグリフが含まれず日本語ラベルが豆腐(□)化するため、
/// Windows標準の日本語フォント(游ゴシック)をフォールバックとして追加する
/// （このツールはこのマシン専用の開発ツールのため、バイナリへの埋め込みはせずシステムフォントを都度読む）。
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\YuGothR.ttc") {
        let mut font_data = egui::FontData::from_owned(bytes);
        font_data.index = 0; // .ttc内の最初のフェイス(Yu Gothic Regular)
        fonts.font_data.insert("jp".to_owned(), std::sync::Arc::new(font_data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("jp".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("xml-panel-dsl preview (native)"),
        ..Default::default()
    };
    eframe::run_native(
        "xml-panel-dsl preview (native)",
        native_options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

/// [start, end) の行頭を検出する（`\n`区切り、charインデックス）。
fn line_bounds(chars: &[char], pos: usize) -> (usize, usize) {
    let start = chars[..pos].iter().rposition(|&c| c == '\n').map(|i| i + 1).unwrap_or(0);
    let end = chars[pos..].iter().position(|&c| c == '\n').map(|i| pos + i).unwrap_or(chars.len());
    (start, end)
}

/// `from`位置から最大4文字分の半角スペースの連続長を数える。
fn leading_spaces(chars: &[char], from: usize) -> usize {
    let mut n = 0;
    while n < IND.len() && chars.get(from + n) == Some(&' ') {
        n += 1;
    }
    n
}

/// TabキーによるXMLテキストのインデント/デデント（`tools/xml-panel-dsl/index.html`のJS版と同一挙動）。
/// 戻り値は (新テキスト, 新選択開始charインデックス, 新選択終了charインデックス)。
fn apply_tab(text: &str, sel_start: usize, sel_end: usize, shift: bool) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();

    if sel_start == sel_end {
        if !shift {
            let mut new_chars = chars;
            for (i, c) in IND.chars().enumerate() {
                new_chars.insert(sel_start + i, c);
            }
            let new_pos = sel_start + IND.chars().count();
            (new_chars.into_iter().collect(), new_pos, new_pos)
        } else {
            let (line_start, _) = line_bounds(&chars, sel_start);
            let remove_len = leading_spaces(&chars, line_start);
            let mut new_chars = chars;
            new_chars.drain(line_start..line_start + remove_len);
            let new_pos = sel_start.saturating_sub(remove_len).max(line_start);
            (new_chars.into_iter().collect(), new_pos, new_pos)
        }
    } else {
        let (block_start, _) = line_bounds(&chars, sel_start);
        let (_, block_end_raw) = line_bounds(&chars, sel_end);
        // 選択終端がちょうど行頭(直前の改行)にある場合、その空行は対象に含めない。
        let block_end = if sel_end > sel_start && chars.get(sel_end - 1) == Some(&'\n') {
            sel_end - 1
        } else {
            block_end_raw
        };

        let block: String = chars[block_start..block_end].iter().collect();
        let mut first_line_delta: isize = 0;
        let mut total_delta: isize = 0;
        let new_lines: Vec<String> = block
            .split('\n')
            .enumerate()
            .map(|(i, line)| {
                if !shift {
                    if i == 0 {
                        first_line_delta = IND.chars().count() as isize;
                    }
                    total_delta += IND.chars().count() as isize;
                    format!("{IND}{line}")
                } else {
                    let line_chars: Vec<char> = line.chars().collect();
                    let remove_len = leading_spaces(&line_chars, 0);
                    if i == 0 {
                        first_line_delta = -(remove_len as isize);
                    }
                    total_delta -= remove_len as isize;
                    line_chars[remove_len..].iter().collect()
                }
            })
            .collect();
        let new_block = new_lines.join("\n");

        let mut new_chars = chars;
        new_chars.splice(block_start..block_end, new_block.chars());
        let new_text: String = new_chars.into_iter().collect();
        let new_sel_start = (sel_start as isize + first_line_delta).max(0) as usize;
        let new_sel_end = (sel_end as isize + total_delta).max(0) as usize;
        (new_text, new_sel_start, new_sel_end)
    }
}

struct App {
    xml: String,
    file_name: String,
    store: HandleStore,
    layout: Option<ui_codegen::Layout>,
    error: Option<String>,
    rust_out: String,
    show_rust: bool,
    undocked: bool,
    panel_width: f32,
}

impl App {
    fn new() -> Self {
        let (xml, file_name) = load_default_xml();
        let mut app = Self {
            xml,
            file_name,
            store: HandleStore::new(),
            layout: None,
            error: None,
            rust_out: String::new(),
            show_rust: false,
            undocked: false,
            panel_width: 1320.0,
        };
        app.reparse();
        app
    }

    /// `self.xml`を元にレイアウト（プレビュー用）とRust出力（コピー用）を再計算する。
    fn reparse(&mut self) {
        // 空（＝正本を読めなかった、または全消しした直後）はエラー扱いにせず、案内文のまま待つ。
        if self.xml.trim().is_empty() {
            self.layout = None;
            self.error = None;
            self.rust_out.clear();
            return;
        }
        match ui_codegen::parse_layout(&self.xml) {
            Ok(layout) => {
                self.layout = Some(layout);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.rust_out = ui_codegen::generate_rust(&self.xml).unwrap_or_default();
    }

    /// Tab/Shift+Tabキーによるインデント編集を、XMLエディタにフォーカスがある間だけ横取りする。
    /// キー消費はTextEditウィジェットが描画される前に行う必要がある
    /// （でないとegui既定のフォーカス移動が先に処理されてしまう）。
    fn handle_tab_key(&mut self, ctx: &egui::Context, editor_id: egui::Id) {
        if !ctx.memory(|m| m.has_focus(editor_id)) {
            return;
        }
        // Modifiers::matches_logicallyは「余分なshift/altは無視」する向きに緩いため、
        // より限定的なShift+Tabを先にconsumeしないと単独Tab判定に食われてしまう。
        let shift_tab = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
        let tab = !shift_tab && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        if !shift_tab && !tab {
            return;
        }
        let Some(mut state) = TextEditState::load(ctx, editor_id) else { return };
        let Some(range) = state.cursor.char_range() else { return };
        let sorted = range.as_sorted_char_range();
        let (new_text, new_start, new_end) = apply_tab(&self.xml, sorted.start, sorted.end, shift_tab);
        self.xml = new_text;
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(new_start), CCursor::new(new_end))));
        state.store(ctx, editor_id);
        self.reparse();
    }

    fn draw_editor(&mut self, ui: &mut egui::Ui) {
        let editor_id = egui::Id::new("xml_editor");
        self.handle_tab_key(ui.ctx(), editor_id);

        ui.horizontal(|ui| {
            ui.label(&self.file_name);
            if ui.button("開く").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file() {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        self.xml = text;
                        self.file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        self.reparse();
                    }
                }
            }
            if ui.button("名前を付けて保存").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("XML", &["xml"]).set_file_name(&self.file_name).save_file() {
                    let _ = std::fs::write(&path, &self.xml);
                    self.file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                }
            }
        });
        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 70), err);
            ui.separator();
        }

        egui::ScrollArea::vertical().id_salt("xml_editor_scroll").show(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.xml)
                    .id(editor_id)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(40),
            );
            if response.changed() {
                self.reparse();
            }
        });
    }

    fn draw_preview_inner(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::horizontal().id_salt("preview_hscroll").show(ui, |ui| {
            ui.set_min_width(self.panel_width);
            ui.set_max_width(self.panel_width);
            match &self.layout {
                Some(layout) => draw_panel_from_ir(ui, layout, &mut self.store),
                None => {
                    ui.label("XMLを開くか編集すると、ここに実描画プレビューが表示されます。");
                }
            }
        });
    }

    fn draw_rust_out(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("クリップボードへコピー").clicked() {
                ui.ctx().copy_text(self.rust_out.clone());
            }
        });
        ui.separator();
        egui::ScrollArea::both().id_salt("rust_out_scroll").show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.rust_out.clone())
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("XML panel DSL (native)");
                ui.separator();
                ui.label("PANEL W");
                ui.add(egui::Slider::new(&mut self.panel_width, 360.0..=1800.0).suffix("px"));
                ui.separator();
                ui.checkbox(&mut self.show_rust, "Rust出力を表示");
                ui.separator();
                let dock_label = if self.undocked { "ドッキングする" } else { "プレビューを分離" };
                if ui.button(dock_label).clicked() {
                    self.undocked = !self.undocked;
                }
            });
        });

        egui::Panel::left("editor_panel")
            .resizable(true)
            .default_size(ui.available_width() * 0.5)
            .show_inside(ui, |ui| self.draw_editor(ui));

        if self.show_rust {
            egui::Panel::right("rust_panel")
                .resizable(true)
                .default_size(ui.available_width() * 0.4)
                .show_inside(ui, |ui| self.draw_rust_out(ui));
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if self.undocked {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.label("プレビューは別ウィンドウで表示中です。");
                    ui.label("ウィンドウを閉じるとここに戻ります。");
                });
            } else {
                self.draw_preview_inner(ui);
            }
        });

        if self.undocked {
            let viewport_id = egui::ViewportId::from_hash_of("xml-panel-dsl-preview-window");
            ui.ctx().show_viewport_immediate(
                viewport_id,
                egui::ViewportBuilder::default().with_title("プレビュー").with_inner_size([960.0, 800.0]),
                |ui, _class| {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        self.draw_preview_inner(ui);
                    });
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        self.undocked = false;
                    }
                },
            );
        }
    }
}
