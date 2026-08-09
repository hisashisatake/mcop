use crate::param_handle::IntParamHandle;

/// 波形パラメーター(0〜255)のカテゴリ一覧（表示名, レンジ開始値）。
pub const WAVEFORM_CATEGORIES: [(&str, i32); 6] = [
    ("Sine", 0),
    ("Saw", 8),
    ("Square", 16),
    ("Triangle", 24),
    ("Noise", 32),
    ("User", 64),
];

/// サイン系(0〜7)のOPZ8バリアント名。ym38x6-core waveform.rsのgen_op_*に対応。
pub const SINE_VARIANTS: [&str; 8] = [
    "Full", "Sin\u{b2}", "Half", "Half Sin\u{b2}", "2x Half", "2x Sin\u{b2} Half", "2x Abs Half",
    "2x Pos\u{b2} Half",
];
/// ノコギリ系(8〜15)・三角系(24〜31)で共通のOPZ8バリアント名（opz_variant関数を共用）。
pub const OPZ_VARIANTS: [&str; 8] = [
    "Full", "Squared", "Half", "Half Sq", "2x Half", "2x Sq Half", "2x Abs Half", "2x Pos\u{b2} Half",
];
/// 矩形系(16〜23)のバリアント名（PWMデューティ+half系、ym38x6-core waveform.rsのgen_square_family）。
pub const SQUARE_VARIANTS: [&str; 8] = [
    "PWM 50%", "PWM 33%", "PWM 25%", "PWM 16.7%", "PWM 12.5%", "PWM 6.25%", "Half", "2x Half",
];

/// オペレーターのWaveformパラメーター専用セレクター。
/// カテゴリ(Sine/Saw/Square/Triangle/Noise/User)をコンボボックスで選び、
/// ビルトイン4種はさらにOPZ8バリアントを選択する。
/// Noiseはノイズ周期NP(0〜31)、Userはユーザー波形スロット(64〜255、ロードUIは別途必要)を直接入力する。
/// `salt`はオペレーターごとにコンボボックスのIDを一意にするためのインデックス。
pub fn waveform_selector(ui: &mut egui::Ui, handle: &dyn IntParamHandle, salt: usize) {
    let current = handle.value();
    let set = |value: i32| {
        handle.begin_edit();
        handle.set(value);
        handle.end_edit();
    };

    ui.allocate_ui_with_layout(
        egui::vec2(130.0, 66.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.label(egui::RichText::new("WAVE").size(9.0));

            let (cat_label, base, variant_names): (&str, i32, Option<&[&str; 8]>) = match current {
                0..=7 => ("Sine", 0, Some(&SINE_VARIANTS)),
                8..=15 => ("Saw", 8, Some(&OPZ_VARIANTS)),
                16..=23 => ("Square", 16, Some(&SQUARE_VARIANTS)),
                24..=31 => ("Triangle", 24, Some(&OPZ_VARIANTS)),
                32..=63 => ("Noise", 32, None),
                _ => ("User", 64, None),
            };
            // カテゴリ切り替え時、ビルトイン4種同士ならバリアント位置を保つ（例: Sine#3→Saw#3）。
            let preserved_variant = if current < 32 { current % 8 } else { 0 };

            egui::ComboBox::from_id_salt(("wf_cat", salt))
                .selected_text(cat_label)
                .width(80.0)
                .show_ui(ui, |ui| {
                    for (label, cat_base) in WAVEFORM_CATEGORIES {
                        if ui.selectable_label(cat_label == label, label).clicked() {
                            let new_value = match label {
                                "Noise" => 32,
                                "User" => 64,
                                _ => cat_base + preserved_variant,
                            };
                            set(new_value);
                        }
                    }
                });

            match variant_names {
                Some(names) => {
                    let variant = (current - base).clamp(0, 7) as usize;
                    egui::ComboBox::from_id_salt(("wf_var", salt))
                        .selected_text(names[variant])
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for (i, name) in names.iter().enumerate() {
                                if ui.selectable_label(variant == i, *name).clicked() {
                                    set(base + i as i32);
                                }
                            }
                        });
                }
                None if cat_label == "Noise" => {
                    let mut np = (current - 32).clamp(0, 31);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("NP").size(9.0));
                        if ui.add(egui::DragValue::new(&mut np).range(0..=31)).changed() {
                            set(32 + np);
                        }
                    });
                }
                None => {
                    let mut slot = current.clamp(64, 255);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Slot").size(9.0));
                        if ui.add(egui::DragValue::new(&mut slot).range(64..=255)).changed() {
                            set(slot);
                        }
                    });
                }
            }
        },
    );
}
