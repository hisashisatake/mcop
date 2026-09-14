use crate::knob::{spin_control, Knob, SPIN_WIDTH_DEFAULT};
use crate::param_handle::{BoolParamHandle, IntParamHandle};

/// GM2/GS準拠のReverbタイプ名（宣言順=NRPN値0〜7、ym38x6-core ReverbTypeに対応）。
pub const REVERB_TYPE_NAMES: [&str; 8] = [
    "Room1", "Room2", "Room3", "Hall1", "Hall2", "Plate", "Delay", "Pan.Delay",
];
/// GM2/GS準拠のChorusタイプ名（宣言順=NRPN値0〜7、ym38x6-core ChorusTypeに対応）。
pub const CHORUS_TYPE_NAMES: [&str; 8] = [
    "Chorus1", "Chorus2", "Chorus3", "Chorus4", "Fb.Chorus", "Flanger", "Sh.Delay", "Sh.DelayFb",
];
/// Delay/Panning Delayのテンポ同期有効/無効（0/1）。FX側パラメーターは全てintのため
/// `<enum>`で0/1を選ばせる（`BoolField`はパッチ側専用の作りのため流用しない）。
pub const DELAY_SYNC_NAMES: [&str; 2] = ["OFF", "ON"];
/// TimeEgのテンポ同期音価名（宣言順=`sound_core::sync_note_beats`のindex0〜19、所要時間の昇順）。
/// index10="1/4"が既定。付点はD、3連はTのサフィックス。
/// `sync_rate`(0〜255)はこの20音価を`sound_core::sync_note_anchor()`のアンカーとして踏むので、
/// ドロップダウン表示は`nearest_sync_note()`でこの配列へ逆引きする。
pub const SYNC_NOTE_NAMES: [&str; sound_core::SYNC_NOTE_COUNT] = [
    "1/32T", "1/32", "1/16T", "1/32D", "1/16", "1/8T", "1/16D", "1/8", "1/4T", "1/8D", "1/4",
    "1/2T", "1/4D", "1/2", "1/1T", "1/2D", "1/1", "1/1D", "2/1", "4/1",
];
/// TimeEgのretrigger_mode名（宣言順=0/1、`sound_core::RETRIGGER_MODE_*`に対応）。
pub const RETRIGGER_MODE_NAMES: [&str; 2] = ["Continue", "Reset"];
/// TimeEgのtexture名（宣言順=0〜7、`sound_core::TEXTURE_*`に対応。2026-09-14再採番、
/// TRIANGLE〜SQUAREは`template_params()`が生成する幾何学的テンプレート波形、S&H以降は
/// 旧質感LFOの後継（乱数系）。memory `project_fg_free_rate_texture_templates_design.md`参照）。
pub const TEXTURE_NAMES: [&str; 8] =
    ["OFF", "TRIANGLE", "SAW UP", "SAW DOWN", "SQUARE", "S&H", "Random", "Chaos"];
/// SYNC OFF時、`free_rate`が動かせる可変幅の表示名（宣言順=0〜3、
/// `sound_core::RATE_RANGE_MULTIPLIERS`に対応）。
pub const RATE_RANGE_NAMES: [&str; 4] = ["×2", "×4", "×8", "×16"];
/// フィルタータイプ名（宣言順=0/1/2、`sound_core::vcf::FilterType::from_u8`に対応。
/// 2以上は全てBP扱いになるため3値で足りる）。
pub const FILTER_TYPE_NAMES: [&str; 3] = ["LP", "HP", "BP"];

/// 固定候補名一覧から1つを選ぶ汎用コンボボックス。Reverb/Chorus Type等の
/// enum的パラメーター(0〜names.len()-1)向け。`salt`はウィジェットごとにIDを一意にする値。
/// `height`は確保する縦幅（既定66.0は`<knob>`とのRow内揃え用、`<stack>`縦積み時は
/// `panel.xml`の`<enum height="...">`属性で個別に縮められる。`ui-codegen`の
/// `Size{w:100.0, h:height}`と一致させること）。
pub fn enum_selector(
    ui: &mut egui::Ui,
    handle: &dyn IntParamHandle,
    label: &str,
    names: &[&str],
    salt: usize,
    height: f32,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(100.0, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.label(egui::RichText::new(label).size(9.0));
            let current = handle.value().clamp(0, names.len() as i32 - 1) as usize;
            egui::ComboBox::from_id_salt(("enum_sel", salt, label))
                .selected_text(names[current])
                .width(100.0)
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        if ui.selectable_label(current == i, *name).clicked() {
                            handle.begin_edit();
                            handle.set(i as i32);
                            handle.end_edit();
                        }
                    }
                });
        },
    );
}

/// `sync_rate`(0〜255)の表示文字列。アンカー上なら音価名そのまま、外れていれば
/// 最寄り音価に`~`を付けて近似であることを示す（ツールチップとComboBoxの両方で使う）。
pub fn sync_rate_display(rate: u8) -> String {
    let (index, exact) = sound_core::nearest_sync_note(rate);
    let name = SYNC_NOTE_NAMES[index as usize];
    if exact {
        name.to_string()
    } else {
        format!("~{name}")
    }
}

/// TimeEgのテンポ同期レート用の複合ウィジェット（幅100×高さ70）。
/// 連続ノブと音価ドロップダウンが**同じ`sync_rate`**の2つの見え方になっているため、
/// ノブを回すとドロップダウンの表示が最寄り音価へ追従し、ドロップダウンで音価を選ぶと
/// ノブがそのアンカー値へ飛ぶ。値が1つしかないので両者がずれることは構造的に起こらない。
///
/// 既存の`<enum label="NOTE">`（100×66）を置き換える想定で、縦は+4pxに収めてある
/// （`<time-eg-editor>`の固定高さ245pxを超えるとFGパネルのレイアウトが崩れるため）。
pub fn sync_rate_selector(ui: &mut egui::Ui, handle: &dyn IntParamHandle, label: &str, salt: usize) {
    ui.allocate_ui_with_layout(
        egui::vec2(100.0, 70.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.label(egui::RichText::new(label).size(9.0));
            // ノブ（水平中央）。knob()セルと違い数値欄は置かず、下のComboBoxが表示を兼ねる。
            ui.vertical_centered(|ui| {
                ui.add(Knob::for_handle(handle).with_diameter(28.0));
            });
            let (current, _) = sound_core::nearest_sync_note(handle.value().clamp(0, 255) as u8);
            egui::ComboBox::from_id_salt(("sync_rate_sel", salt, label))
                .selected_text(sync_rate_display(handle.value().clamp(0, 255) as u8))
                .width(100.0)
                .show_ui(ui, |ui| {
                    for (i, name) in SYNC_NOTE_NAMES.iter().enumerate() {
                        if ui.selectable_label(current as usize == i, *name).clicked() {
                            handle.begin_edit();
                            handle.set(sound_core::sync_note_anchor(i as u8) as i32);
                            handle.end_edit();
                        }
                    }
                });
        },
    );
}

/// `free_rate`(0〜255)の表示文字列（"×1.41"形式、`sound_core::free_rate_scale`と同じ式）。
/// `fg_rate_selector`のノブのツールチップ専用（数値欄は生値のまま表示する。理由は
/// `FreeRateTooltipHandle`のdocコメント参照）。
pub fn free_rate_display(free_rate: u8, rate_range: u8) -> String {
    let params = sound_core::TimeEgParams { free_rate, rate_range, ..sound_core::TimeEgParams::default() };
    format!("×{:.2}", sound_core::free_rate_scale(&params))
}

/// `free_rate`ハンドルの`display()`だけを倍率表示("×1.41")へ差し替える軽量アダプタ。
/// `value()`/`min()`/`max()`/`set()`は`inner`へ素通しするため、ノブのドラッグ・リセット動作は
/// 一切変わらない。`fg_rate_selector`はこのアダプタをノブにだけ渡し、数値欄（`spin_control`）には
/// `inner`をそのまま渡す——同じハンドルをノブと数値欄の両方に使う`knob()`と違い、この2つを
/// 呼び出し側で分けることで「ノブは倍率、数値欄は生値」という非対称表示を実現する
/// （memory `project_knob_display_vs_raw_input.md`の教訓：`spin_control`は`display()`を
/// 直接入力の初期バッファにも使うため、生値以外を返すハンドルをそのまま渡すと直接入力が壊れる）。
struct FreeRateTooltipHandle<'a> {
    inner: &'a dyn IntParamHandle,
    rate_range: u8,
}

impl IntParamHandle for FreeRateTooltipHandle<'_> {
    fn value(&self) -> i32 {
        self.inner.value()
    }
    fn min(&self) -> i32 {
        self.inner.min()
    }
    fn max(&self) -> i32 {
        self.inner.max()
    }
    fn default(&self) -> i32 {
        self.inner.default()
    }
    fn name(&self) -> String {
        self.inner.name()
    }
    fn display(&self) -> String {
        free_rate_display(self.value().clamp(0, 255) as u8, self.rate_range)
    }
    fn begin_edit(&self) {
        self.inner.begin_edit();
    }
    fn set(&self, value: i32) {
        self.inner.set(value);
    }
    fn end_edit(&self) {
        self.inner.end_edit();
    }
}

/// TimeEg FGのRATE用の複合ウィジェット（幅100×高さ70、`sync_rate_selector`と同じ枠）。
/// SYNC ON/OFFで参照先そのものを切り替える（`sync_rate`/`free_rate`は別フィールドで
/// 互いに独立、`project_fg_free_rate_texture_templates_design.md`参照）。
/// - SYNC=ON: `sync_rate_selector`と同じ見た目（ノブ+音価ドロップダウン、`sync_rate`）。
/// - SYNC=OFF: ノブは`free_rate`（ツールチップに倍率）、下は数値欄（`spin_control`、生値0〜255）。
pub fn fg_rate_selector(
    ui: &mut egui::Ui,
    sync: &dyn BoolParamHandle,
    sync_rate: &dyn IntParamHandle,
    free_rate: &dyn IntParamHandle,
    rate_range: u8,
    label: &str,
    salt: usize,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(100.0, 70.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.label(egui::RichText::new(label).size(9.0));
            if sync.value() {
                ui.vertical_centered(|ui| {
                    ui.add(Knob::for_handle(sync_rate).with_diameter(28.0));
                });
                let (current, _) = sound_core::nearest_sync_note(sync_rate.value().clamp(0, 255) as u8);
                egui::ComboBox::from_id_salt(("fg_rate_sel", salt, label))
                    .selected_text(sync_rate_display(sync_rate.value().clamp(0, 255) as u8))
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for (i, name) in SYNC_NOTE_NAMES.iter().enumerate() {
                            if ui.selectable_label(current as usize == i, *name).clicked() {
                                sync_rate.begin_edit();
                                sync_rate.set(sound_core::sync_note_anchor(i as u8) as i32);
                                sync_rate.end_edit();
                            }
                        }
                    });
            } else {
                let tooltip_handle = FreeRateTooltipHandle { inner: free_rate, rate_range };
                ui.vertical_centered(|ui| {
                    ui.add(Knob::for_handle(&tooltip_handle).with_diameter(28.0));
                });
                // `knob()`セルと同じ中央寄せ（KNOB_CELL_SIZEではなくこのウィジェットの100px幅基準）。
                let row_width = crate::knob::spin_control_width(SPIN_WIDTH_DEFAULT);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(((100.0 - row_width) * 0.5).max(0.0));
                    spin_control(ui, free_rate, egui::TextStyle::Small, SPIN_WIDTH_DEFAULT);
                });
            }
        },
    );
}
