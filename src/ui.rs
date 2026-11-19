//! The Compact view: engine, voice, speed and the playback buttons.

use crate::controller::{Controller, Status};
use crate::hotkeys::Hotkeys;
use crate::options::Options;
use eframe::egui;
use std::sync::Arc;
use std::time::Duration;

pub struct ReaderApp {
    controller: Arc<Controller>,
    hotkeys: Option<Hotkeys>,
    options: Options,
    speed: f64,
}

impl ReaderApp {
    pub fn new(controller: Arc<Controller>, hotkeys: Option<Hotkeys>, ctx: &egui::Context) -> Self {
        let repaint_ctx = ctx.clone();
        controller.set_repaint(move || repaint_ctx.request_repaint());
        let speed = controller.settings.lock().unwrap().speed();
        let mut options = Options::new();
        if let Some(tab) = std::env::args().find_map(|a| a.strip_prefix("--options").map(|t| t.trim_start_matches('=').to_string())) {
            options.open_tab(&controller, &tab);
        }
        ReaderApp { controller, hotkeys, options, speed }
    }

    fn hint(&self, what: &str, action: &str) -> String {
        let key = self.controller.settings.lock().unwrap().hotkey(action);
        format!("{what}\nShortcut: {key}")
    }
}

fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

impl eframe::App for ReaderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let snap = self.controller.snapshot();
        let reading = snap.reading();
        if reading {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            let c = &self.controller;

            egui::Grid::new("form").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                // Engine and Voice are locked while reading; press Stop to change them
                ui.label("Engine");
                let engine = c.current_engine().name();
                ui.add_enabled_ui(!reading, |ui| {
                    egui::ComboBox::from_id_salt("engine").width(220.0).selected_text(engine).show_ui(ui, |ui| {
                        for name in c.enabled_engines() {
                            if ui.selectable_label(name == engine, name).clicked() && name != engine {
                                c.set_engine(name);
                            }
                        }
                    });
                });
                ui.end_row();

                ui.label("Voice");
                let current = c.current_voice(engine);
                ui.add_enabled_ui(!reading, |ui| {
                    egui::ComboBox::from_id_salt("voice")
                        .width(220.0)
                        .selected_text(current.as_ref().map(|v| v.1.as_str()).unwrap_or("Default"))
                        .show_ui(ui, |ui| {
                            for (id, label) in c.voices(engine) {
                                let selected = current.as_ref().is_some_and(|v| v.0 == id);
                                if ui.selectable_label(selected, &label).clicked() {
                                    c.set_voice(engine, &id);
                                }
                            }
                        });
                });
                ui.end_row();

                // Speed can change while reading
                ui.label("Speed");
                let slider = egui::Slider::new(&mut self.speed, 0.5..=2.0).step_by(0.1).suffix("x").fixed_decimals(1);
                if ui.add(slider).changed() {
                    c.set_speed(self.speed);
                }
                ui.end_row();
            });

            ui.add_space(4.0);
            let wide = egui::vec2(ui.available_width(), 34.0);
            let read = ui
                .add_sized(wide, egui::Button::new(egui::RichText::new("📋  Read Clipboard").strong()))
                .on_hover_text(self.hint("Read the selected text aloud (or the clipboard, if nothing is selected)", "read_selected_text"));
            if read.clicked() {
                c.read_selection();
            }

            let half = egui::vec2((ui.available_width() - 8.0) / 2.0, 28.0);
            ui.horizontal(|ui| {
                let playing = snap.status == Status::Playing;
                let (label, hint) = if playing {
                    ("⏸  Pause", self.hint("Pause reading", "pause"))
                } else {
                    ("▶  Play", self.hint("Play, or resume after a pause", "play"))
                };
                let can_play = playing || snap.status == Status::Paused || snap.has_audio;
                if ui.add_enabled(can_play, egui::Button::new(label).min_size(half)).on_hover_text(hint).clicked() {
                    if playing { c.toggle_pause() } else { c.play() }
                }
                if ui
                    .add_enabled(reading, egui::Button::new("⏹  Stop").min_size(half))
                    .on_hover_text(self.hint("Stop reading", "stop"))
                    .clicked()
                {
                    c.stop();
                }
            });

            if ui
                .add_sized(egui::vec2(ui.available_width(), 26.0), egui::Button::new("⚙  Options…"))
                .on_hover_text("Engines & voices, pronunciations, links/paths/codes, and keyboard shortcuts")
                .clicked()
            {
                self.options.open(c, None);
            }

            let fraction = match (snap.position, snap.duration) {
                (Some(p), Some(d)) if d > 0.0 => (p / d) as f32,
                _ => 0.0,
            };
            ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).desired_height(6.0));

            ui.horizontal(|ui| {
                let status = match &snap.status {
                    Status::Stopped => "Stopped".to_string(),
                    Status::Building(p) if !p.is_empty() => p.clone(),
                    Status::Building(_) => "Building audio…".to_string(),
                    Status::Playing => "Playing".to_string(),
                    Status::Paused => "Paused".to_string(),
                };
                ui.label(status);
                if let (Some(p), Some(d)) = (snap.position, snap.duration) {
                    // Shown in real time, so it follows the speed
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{} / {}", format_time(p / self.speed), format_time(d / self.speed)));
                    });
                }
            });
            if let Some(message) = &snap.message {
                ui.colored_label(ui.visuals().warn_fg_color, message);
            } else if let Some(used) = &snap.voice_used {
                ui.weak(used);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.weak(format!("Clipboard Reader {} (Rust preview)", env!("CARGO_PKG_VERSION")));
            });
        });
        self.options.show(&ctx, &self.controller, &mut self.hotkeys, reading);
    }
}
