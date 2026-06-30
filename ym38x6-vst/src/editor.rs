use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use std::sync::{Arc, Mutex};
use ym38x6_core::{presets_dir, PresetBank, Ym38x6Patch};

use crate::knob::Knob;
use crate::params::Ym38x6Params;

pub(crate) struct EditorState {
    pub(crate) preset_bank: PresetBank,
    pub(crate) selected_key: Option<(u16, u8)>,
}

/// ラベル付きノブを1セル（幅62×高さ66）に配置する。
/// 名称をノブの左上に置き、ノブ・数値欄＋スピンを縦に並べる。
/// セルを固定サイズ・固定レイアウトにすることで、横に並べたノブの高さが揃う。
fn knob<P: Param>(ui: &mut egui::Ui, setter: &ParamSetter, param: &P, label: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(62.0, 66.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            // 名称（左上）
            ui.label(egui::RichText::new(label).size(9.0));
            // ノブ（水平中央）
            ui.vertical_centered(|ui| {
                ui.add(Knob::for_param(param, setter).with_diameter(28.0));
            });
            // 数値欄＋スピンボタン
            spin_control(ui, setter, param);
        },
    );
}

/// 波形パラメーター(0〜255)のカテゴリ一覧（表示名, レンジ開始値）。
const WAVEFORM_CATEGORIES: [(&str, i32); 6] = [
    ("Sine", 0),
    ("Saw", 8),
    ("Square", 16),
    ("Triangle", 24),
    ("Noise", 32),
    ("User", 64),
];

/// サイン系(0〜7)のOPZ8バリアント名。waveform.rsのgen_op_*に対応。
const SINE_VARIANTS: [&str; 8] = [
    "Full", "Sin\u{b2}", "Half", "Half Sin\u{b2}", "2x Half", "2x Sin\u{b2} Half", "2x Abs Half",
    "2x Pos\u{b2} Half",
];
/// ノコギリ系(8〜15)・三角系(24〜31)で共通のOPZ8バリアント名（opz_variant関数を共用）。
const OPZ_VARIANTS: [&str; 8] = [
    "Full", "Squared", "Half", "Half Sq", "2x Half", "2x Sq Half", "2x Abs Half", "2x Pos\u{b2} Half",
];
/// 矩形系(16〜23)のバリアント名（PWMデューティ+half系、waveform.rsのgen_square_family）。
const SQUARE_VARIANTS: [&str; 8] = [
    "PWM 50%", "PWM 33%", "PWM 25%", "PWM 16.7%", "PWM 12.5%", "PWM 6.25%", "Half", "2x Half",
];

/// オペレーターのWaveformパラメーター専用セレクター。
/// カテゴリ(Sine/Saw/Square/Triangle/Noise/User)をコンボボックスで選び、
/// ビルトイン4種はさらにOPZ8バリアントを選択する。
/// Noiseはノイズ周期NP(0〜31)、Userはユーザー波形スロット(64〜255、ロードUIは別途必要)を直接入力する。
/// `salt`はオペレーターごとにコンボボックスのIDを一意にするためのインデックス。
fn waveform_selector(ui: &mut egui::Ui, setter: &ParamSetter, param: &IntParam, salt: usize) {
    let current = param.modulated_plain_value();
    let set = |value: i32| {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
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

/// 押下開始で1回、その後は初期遅延を挟んで一定間隔で繰り返し`true`を返すボタン（長押しリピート）。
/// 一度ボタン上で押し始めたら、マウスがボタン矩形から外れてもマウスボタンを離すまでリピートを継続する。
fn repeat_button(ui: &mut egui::Ui, label: &str) -> bool {
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
///   Enterで`string_to_normalized_value`によりパースして書き戻す。
///   フォーカス中はカーソルキー↑↓でも1ステップずつ増減できる
/// - ＋−：長押しで連続増減（`Param::next_step`/`previous_step`、整数パラメーターは±1）
fn spin_control<P: Param>(ui: &mut egui::Ui, setter: &ParamSetter, param: &P) {
    let set = |value: P::Plain| {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    };
    let step_up = || param.next_step(param.modulated_plain_value(), false);
    let step_down = || param.previous_step(param.modulated_plain_value(), false);

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
                .unwrap_or_else(|| param.to_string())
        } else {
            param.to_string()
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(24.0)
                .font(egui::TextStyle::Small)
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
                if let Some(normalized) = param.string_to_normalized_value(&text) {
                    set(param.preview_plain(normalized));
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

pub(crate) fn create_editor(
    egui_state: Arc<EguiState>,
    pending_gui_preset: Arc<Mutex<Option<Ym38x6Patch>>>,
    params: Arc<Ym38x6Params>,
) -> Option<Box<dyn Editor>> {
    let resize_state = egui_state.clone();
    create_egui_editor(
        egui_state,
        EditorState {
            preset_bank: PresetBank::load_from_dir(&presets_dir()),
            selected_key: None,
        },
        EguiSettings::default(),
        |_ctx, _queue, _state| {},
        move |ui, setter, _queue, state| {
            ResizableWindow::new("ym38x6_resize")
                .min_size((640.0, 480.0))
                .show(ui, &resize_state, |ui| {
                    // ---- プリセットブラウザ（左サイドバー・縦いっぱい） ----
                    egui::Panel::left("presets_panel")
                        .resizable(false)
                        .exact_size(180.0)
                        .show_inside(ui, |ui| {
                            ui.label(egui::RichText::new("PRESETS").strong());
                            egui::ScrollArea::vertical()
                                .id_salt("presets")
                                .show(ui, |ui| {
                                    for ((bank, program), preset) in state.preset_bank.sorted_entries() {
                                        let label = if bank == 0 {
                                            format!("{program:03} {}", preset.name)
                                        } else {
                                            format!("[{bank:04X}:{program:03}] {}", preset.name)
                                        };
                                        let selected = state.selected_key == Some((bank, program));
                                        if ui.selectable_label(selected, &label).clicked() {
                                            state.selected_key = Some((bank, program));
                                            *pending_gui_preset.lock().unwrap() = Some(preset.patch);
                                            macro_rules! set {
                                                ($p:expr, $v:expr) => {
                                                    setter.begin_set_parameter(&$p);
                                                    setter.set_parameter(&$p, $v);
                                                    setter.end_set_parameter(&$p);
                                                };
                                            }
                                            let ch = &preset.patch.channel;
                                            set!(params.algorithm, ch.algorithm as i32);
                                            set!(params.feedback, ch.feedback as i32);
                                            set!(params.tone_freq, ch.tone_lfo_freq as i32);
                                            set!(params.tone_pmd, ch.tone_lfo_pmd as i32);
                                            set!(params.tone_amd, ch.tone_lfo_amd as i32);
                                            set!(params.tone_delay, ch.tone_lfo_delay as i32);
                                            set!(params.pms, ch.pms as i32);
                                            set!(params.ams, ch.ams as i32);
                                            set!(params.cutoff, ch.filter_cutoff as i32);
                                            set!(params.resonance, ch.filter_resonance as i32);
                                            set!(params.feg_a, ch.filter_eg_attack as i32);
                                            set!(params.feg_d, ch.filter_eg_decay as i32);
                                            set!(params.feg_s, ch.filter_eg_sustain as i32);
                                            set!(params.feg_r, ch.filter_eg_release as i32);
                                            set!(params.feg_depth, ch.filter_eg_depth as i32);
                                            for (i, op) in preset.patch.operators.iter().enumerate() {
                                                let op_p = &params.operators[i];
                                                set!(op_p.tl, op.tl as i32);
                                                set!(op_p.ar, op.ar as i32);
                                                set!(op_p.d1r, op.d1r as i32);
                                                set!(op_p.d2r, op.d2r as i32);
                                                set!(op_p.d1l, op.d1l as i32);
                                                set!(op_p.rr, op.rr as i32);
                                                set!(op_p.mul, op.mul as i32);
                                                set!(op_p.dt1, op.dt1 as i32);
                                                set!(op_p.ksr, op.ksr as i32);
                                                set!(op_p.ame, op.am_enable);
                                                set!(op_p.vel_sens, op.velocity_sensitivity as i32);
                                                set!(op_p.op_fine_tune, op.op_fine_tune as i32);
                                                set!(op_p.waveform, op.waveform as i32);
                                            }
                                        }
                                    }
                                });
                        });

                    // ---- 残りのパラメーター（右側・縦スクロール） ----
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            // ---- Operators（各opを横一列で表示し、OP1→OP4を縦に積む。最優先で上に表示） ----
                            for i in 0..4 {
                                let op = &params.operators[i];
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(egui::RichText::new(format!("OP {}", i + 1)).strong());
                                        knob(ui, setter, &op.tl, "TL");
                                        knob(ui, setter, &op.ar, "AR");
                                        knob(ui, setter, &op.d1r, "D1R");
                                        knob(ui, setter, &op.d2r, "D2R");
                                        knob(ui, setter, &op.d1l, "D1L");
                                        knob(ui, setter, &op.rr, "RR");
                                        knob(ui, setter, &op.mul, "MUL");
                                        knob(ui, setter, &op.dt1, "DT1");
                                        knob(ui, setter, &op.ksr, "KSR");
                                        knob(ui, setter, &op.vel_sens, "VEL");
                                        knob(ui, setter, &op.op_fine_tune, "FINE");
                                        let mut am = op.ame.modulated_plain_value();
                                        if ui.checkbox(&mut am, "AM").changed() {
                                            setter.begin_set_parameter(&op.ame);
                                            setter.set_parameter(&op.ame, am);
                                            setter.end_set_parameter(&op.ame);
                                        }
                                        waveform_selector(ui, setter, &op.waveform, i);
                                    });
                                });
                            }

                            // ---- チャンネル固有 / パフォーマンスLFO / トーンLFO（横一列） ----
                            ui.horizontal(|ui| {
                                ui.group(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new("CHANNEL").strong());
                                        ui.horizontal_wrapped(|ui| {
                                            knob(ui, setter, &params.algorithm, "ALG");
                                            knob(ui, setter, &params.feedback, "FB");
                                        });
                                    });
                                });

                                ui.group(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new("PERF LFO").strong());
                                        ui.horizontal_wrapped(|ui| {
                                            knob(ui, setter, &params.lfo_rate, "L.RATE");
                                            knob(ui, setter, &params.lfo_depth, "L.DEP");
                                            knob(ui, setter, &params.lfo_delay, "L.DLY");
                                        });
                                    });
                                });

                                ui.group(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new("TONE LFO").strong());
                                        ui.horizontal_wrapped(|ui| {
                                            knob(ui, setter, &params.tone_freq, "T.FRQ");
                                            knob(ui, setter, &params.tone_pmd, "T.PMD");
                                            knob(ui, setter, &params.tone_amd, "T.AMD");
                                            knob(ui, setter, &params.tone_delay, "T.DLY");
                                            knob(ui, setter, &params.pms, "PMS");
                                            knob(ui, setter, &params.ams, "AMS");
                                        });
                                    });
                                });
                            });

                            // ---- フィルター / マスターエフェクト（横一列） ----
                            ui.horizontal(|ui| {
                                ui.group(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new("FILTER").strong());
                                        ui.horizontal_wrapped(|ui| {
                                            knob(ui, setter, &params.cutoff, "CUT");
                                            knob(ui, setter, &params.resonance, "RES");
                                            knob(ui, setter, &params.feg_a, "F.A");
                                            knob(ui, setter, &params.feg_d, "F.D");
                                            knob(ui, setter, &params.feg_s, "F.S");
                                            knob(ui, setter, &params.feg_r, "F.R");
                                            knob(ui, setter, &params.feg_depth, "F.DEP");
                                        });
                                    });
                                });

                                ui.group(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new("MASTER EFFECT").strong());
                                        ui.horizontal_wrapped(|ui| {
                                            knob(ui, setter, &params.rev_send, "REV");
                                            knob(ui, setter, &params.cho_send, "CHO");
                                            knob(ui, setter, &params.reverb_time, "R.TIME");
                                            knob(ui, setter, &params.chorus_mod_rate, "C.RATE");
                                            knob(ui, setter, &params.chorus_mod_depth, "C.DEP");
                                            knob(ui, setter, &params.chorus_feedback, "C.FB");
                                            knob(ui, setter, &params.chorus_send_to_reverb, "C>R");
                                        });
                                    });
                                });
                            });
                        });
                    });
                });
        },
    )
}
