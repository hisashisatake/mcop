use crate::knob::Knob;
use crate::param_handle::IntParamHandle;

/// GM2/GS準拠のReverbタイプ名（宣言順=NRPN値0〜7、ym38x6-core ReverbTypeに対応）。
pub const REVERB_TYPE_NAMES: [&str; 8] = [
    "Room1", "Room2", "Room3", "Hall1", "Hall2", "Plate", "Delay", "Pan.Delay",
];
/// GM2/GS準拠のChorusタイプ名（宣言順=NRPN値0〜7、ym38x6-core ChorusTypeに対応）。
pub const CHORUS_TYPE_NAMES: [&str; 8] = [
    "Chorus1", "Chorus2", "Chorus3", "Chorus4", "Fb.Chorus", "Flanger", "Sh.Delay", "Sh.DelayFb",
];
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
/// TimeEgのtexture名（宣言順=0〜3、`sound_core::TEXTURE_*`に対応。旧質感LFOのS&H/Random/
/// Chaos波形の後継、memory `project_texture_lfo_retirement.md`参照）。
pub const TEXTURE_NAMES: [&str; 4] = ["OFF", "S&H", "Random", "Chaos"];
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
