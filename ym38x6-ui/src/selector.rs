use crate::param_handle::IntParamHandle;

/// GM2/GS準拠のReverbタイプ名（宣言順=NRPN値0〜7、ym38x6-core ReverbTypeに対応）。
pub const REVERB_TYPE_NAMES: [&str; 8] = [
    "Room1", "Room2", "Room3", "Hall1", "Hall2", "Plate", "Delay", "Pan.Delay",
];
/// GM2/GS準拠のChorusタイプ名（宣言順=NRPN値0〜7、ym38x6-core ChorusTypeに対応）。
pub const CHORUS_TYPE_NAMES: [&str; 8] = [
    "Chorus1", "Chorus2", "Chorus3", "Chorus4", "Fb.Chorus", "Flanger", "Sh.Delay", "Sh.DelayFb",
];
/// パフォーマンスLFOの波形名（宣言順=NRPN(0,1)値0〜7、ym38x6-core LfoWaveformに対応）。
pub const LFO_WAVEFORM_NAMES: [&str; 8] = [
    "Triangle", "Sine", "Square", "S&H", "Saw", "Trapezoid", "Random", "Chaos",
];
/// パフォーマンスLFOのFadeモード名（宣言順=NRPN(0,22)値0〜3、ym38x6-core LfoFadeModeに対応）。
pub const LFO_FADE_MODE_NAMES: [&str; 4] = ["ON-IN", "ON-OUT", "OFF-IN", "OFF-OUT"];

/// 固定候補名一覧から1つを選ぶ汎用コンボボックス。Reverb/Chorus Type等の
/// enum的パラメーター(0〜names.len()-1)向け。`salt`はウィジェットごとにIDを一意にする値。
pub fn enum_selector(
    ui: &mut egui::Ui,
    handle: &dyn IntParamHandle,
    label: &str,
    names: &[&str],
    salt: usize,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(100.0, 66.0),
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
