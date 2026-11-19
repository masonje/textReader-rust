//! The main window: the Compact view (controls only) and the Full view (controls plus the
//! text being read, with the current word highlighted).

use crate::controller::{Controller, Reading, Snapshot, Status};
use crate::hotkeys::Hotkeys;
use crate::options::Options;
use eframe::egui;
use egui::text::{CCursor, LayoutJob, TextFormat};
use std::sync::Arc;
use std::time::Duration;

pub const COMPACT_SIZE: [f32; 2] = [340.0, 340.0];
const CONTROLS_WIDTH: f32 = 320.0;
const FULL_DEFAULT: [f32; 2] = [860.0, 460.0];
const READING_MIN_WIDTH: f32 = 320.0;

/// Leading/trailing punctuation dropped from a right-clicked word, as the Python app does.
const WORD_TRIM: &[char] = &['.', ',', ';', ':', '!', '?', '"', '\'', '“', '”', '‘', '’', '(', ')', '[', ']', '{', '}'];

pub struct ReaderApp {
    controller: Arc<Controller>,
    hotkeys: Option<Hotkeys>,
    options: Options,
    speed: f64,
    full: bool,
    full_size: Option<egui::Vec2>, // the Full view's size, remembered between switches
    shown_word: Option<(u64, Option<usize>)>, // (reading, word) last scrolled to
    menu_word: Option<String>,                // the word right-clicked in the reading text
}

/// The window size to start with, from the saved view.
pub fn start_size(controller: &Controller) -> ([f32; 2], [f32; 2]) {
    let settings = controller.settings.lock().unwrap();
    if settings.get("view_mode").and_then(|v| v.as_str()) == Some("full") {
        let size = parse_size(settings.get("full_window_size").and_then(|v| v.as_str()).unwrap_or("")).unwrap_or(FULL_DEFAULT.into());
        ([size.x, size.y], full_min())
    } else {
        (COMPACT_SIZE, COMPACT_SIZE)
    }
}

fn full_min() -> [f32; 2] {
    [CONTROLS_WIDTH + READING_MIN_WIDTH, COMPACT_SIZE[1]]
}

/// "860x460" (the Python app's format; it may have "+x+y" after it)
fn parse_size(text: &str) -> Option<egui::Vec2> {
    let (w, h) = text.split('+').next()?.split_once('x')?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    let [min_w, min_h] = full_min();
    (w > 0.0 && h > 0.0).then(|| egui::vec2(w.max(min_w), h.max(min_h)))
}

impl ReaderApp {
    pub fn new(controller: Arc<Controller>, hotkeys: Option<Hotkeys>, ctx: &egui::Context) -> Self {
        let repaint_ctx = ctx.clone();
        controller.set_repaint(move || repaint_ctx.request_repaint());
        let (speed, full, full_size) = {
            let settings = controller.settings.lock().unwrap();
            (
                settings.speed(),
                settings.get("view_mode").and_then(|v| v.as_str()) == Some("full"),
                parse_size(settings.get("full_window_size").and_then(|v| v.as_str()).unwrap_or("")),
            )
        };
        let mut options = Options::new();
        if let Some(tab) = std::env::args().find_map(|a| a.strip_prefix("--options").map(|t| t.trim_start_matches('=').to_string())) {
            options.open_tab(&controller, &tab);
        }
        ReaderApp { controller, hotkeys, options, speed, full, full_size, shown_word: None, menu_word: None }
    }

    fn hint(&self, what: &str, action: &str) -> String {
        let key = self.controller.settings.lock().unwrap().hotkey(action);
        format!("{what}\nShortcut: {key}")
    }

    fn save_full_size(&self) {
        if let Some(size) = self.full_size {
            let text = format!("{}x{}", size.x.round(), size.y.round());
            self.controller.settings.lock().unwrap().set("full_window_size", serde_json::Value::String(text));
        }
    }

    fn set_view(&mut self, ctx: &egui::Context, full: bool) {
        if !full {
            self.save_full_size();
        }
        self.full = full;
        self.controller.settings.lock().unwrap().set("view_mode", serde_json::json!(if full { "full" } else { "compact" }));
        let (size, min) = if full {
            (self.full_size.unwrap_or(FULL_DEFAULT.into()), full_min().into())
        } else {
            (COMPACT_SIZE.into(), COMPACT_SIZE.into())
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        self.shown_word = None;
    }

    /// Engine, Voice, Speed, the buttons and the status: the whole Compact view.
    fn controls(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        let reading = snap.reading();
        let c = self.controller.clone();

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

        let mut switch = None;
        ui.horizontal(|ui| {
            let half = egui::vec2(half.x, 26.0);
            if ui
                .add(egui::Button::new("⚙  Options…").min_size(half))
                .on_hover_text("Engines & voices, pronunciations, links/paths/codes, and keyboard shortcuts")
                .clicked()
            {
                self.options.open(&c, None);
            }
            let (label, hint) = if self.full {
                ("◂  Compact", "Show only the controls")
            } else {
                ("Full View  ▸", "Show the text as it's read, with the current word highlighted")
            };
            if ui.add(egui::Button::new(label).min_size(half)).on_hover_text(hint).clicked() {
                switch = Some(!self.full);
            }
        });
        if let Some(full) = switch {
            self.set_view(ui.ctx(), full);
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
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if self.full {
                // The Full view's settings, at the bottom of the controls
                let mut surprise = snap.surprise;
                if ui
                    .checkbox(&mut surprise, "Surprise me with a random voice")
                    .on_hover_text("Each read uses a random engine and voice from those turned on in Options (skipping voices that need a download)")
                    .changed()
                {
                    c.set_surprise(surprise);
                }
                ui.separator();
            } else {
                ui.vertical_centered(|ui| ui.weak(format!("Clipboard Reader {} (Rust preview)", env!("CARGO_PKG_VERSION"))));
            }
        });
    }

    /// The Full view's reading panel: the text, words already spoken dimmed, the current one highlighted.
    fn reading_panel(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let reading = self.controller.reading();
        let word = match snap.position {
            Some(p) if matches!(snap.status, Status::Playing | Status::Paused) => reading.word_at(p),
            _ => None,
        };
        ui.horizontal(|ui| {
            ui.heading("Now Reading");
            if let Some(used) = &snap.voice_used {
                ui.weak(used);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match word {
                Some(i) => ui.weak(format!("Word {} of {}", i + 1, reading.spans.len())),
                None if !reading.spans.is_empty() => ui.weak(format!("{} words", reading.spans.len())),
                None => ui.weak(""),
            });
        });
        ui.add_space(4.0);

        let visuals = ui.visuals().clone();
        egui::Frame::new()
            .fill(visuals.extreme_bg_color)
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if reading.text.is_empty() {
                        ui.weak("Text appears here while it's read aloud, with the current word highlighted.\n\nRight-click a word to change how it's read.");
                        return;
                    }
                    let job = reading_job(&reading, word, ui.available_width(), &visuals);
                    let (pos, galley, response) = egui::Label::new(job).selectable(false).sense(egui::Sense::click()).layout_in_ui(ui);
                    ui.painter().galley(pos, galley.clone(), visuals.text_color());

                    // Keep the current word in view (only when it changes, so you can scroll away)
                    if self.shown_word != Some((reading.seq, word)) {
                        self.shown_word = Some((reading.seq, word));
                        if let Some(i) = word {
                            let chars = reading.text[..reading.spans[i].0].chars().count();
                            let rect = galley.pos_from_cursor(CCursor::new(chars)).translate(pos.to_vec2());
                            ui.scroll_to_rect(rect.expand(30.0), None);
                        }
                    }

                    // Right-click a word to change how it's read
                    if response.secondary_clicked() {
                        self.menu_word = response
                            .interact_pointer_pos()
                            .and_then(|p| word_under(&reading, galley.cursor_from_pos(p - pos).index.0));
                    }
                    let c = self.controller.clone();
                    let options = &mut self.options;
                    let menu_word = self.menu_word.clone();
                    response.context_menu(|ui| {
                        if let Some(word) = &menu_word {
                            let verb = if options.has_rule(&c, word) { "Edit" } else { "Change" };
                            if ui.button(format!("{verb} how “{word}” is read…")).clicked() {
                                options.open(&c, Some(word));
                                ui.close();
                            }
                        }
                        if ui.button("Pronunciations…").clicked() {
                            options.open_tab(&c, "pronunciations");
                            ui.close();
                        }
                    });
                });
            });
    }
}

/// While audio builds: what's happening, and a way to cancel (like the Python app's Building Audio window).
/// Shown after a moment, so quick builds don't flash it.
fn building_window(ctx: &egui::Context, controller: &Controller, snap: &Snapshot) {
    let Status::Building(progress) = &snap.status else { return };
    if snap.building_for.is_none_or(|t| t < 0.3) {
        ctx.request_repaint_after(Duration::from_millis(100));
        return;
    }
    // "Downloading voice: 12.3 of 63 MB (19%)" -> 0.19
    let percent = progress
        .rsplit_once('(')
        .and_then(|(_, p)| p.strip_suffix("%)"))
        .and_then(|p| p.parse::<f32>().ok());
    let downloading = progress.starts_with("Downloading");
    let title = if downloading { "Downloading Voice" } else { "Building Audio" };
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id("clipboard-reader")
        .with_inner_size([360.0, 118.0])
        .with_resizable(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("building"), builder, |ui, _class| {
        let cancel = ui.ctx().input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape));
        if cancel {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if progress.is_empty() { "Building audio for playback…" } else { progress.as_str() });
            });
            let bar = match percent {
                Some(p) => egui::ProgressBar::new(p / 100.0),
                None => {
                    // No measure of progress: a bar that sweeps back and forth
                    let t = ui.input(|i| i.time) as f32;
                    egui::ProgressBar::new((t * 0.8).sin().abs())
                }
            };
            ui.add(bar.desired_height(6.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let elapsed = snap.building_for.unwrap_or(0.0);
                if ui.button("Cancel").on_hover_text("Stop building this audio (Esc)").clicked() || cancel {
                    println!("✖ Cancelled audio build");
                    controller.stop();
                }
                ui.weak(format!("{elapsed:.0}s"));
            });
        });
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    });
}

/// The reading text as one layout: spoken words dimmed, the current word highlighted.
fn reading_job(reading: &Reading, word: Option<usize>, width: f32, visuals: &egui::Visuals) -> LayoutJob {
    let font = egui::FontId::proportional(16.0);
    let format = |color, background| TextFormat {
        font_id: font.clone(),
        color,
        background,
        line_height: Some(25.0),
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let text = &reading.text;
    match word.and_then(|i| reading.spans.get(i)) {
        Some(&(start, end)) => {
            job.append(&text[..start], 0.0, format(visuals.weak_text_color(), egui::Color32::TRANSPARENT));
            job.append(&text[start..end], 0.0, format(visuals.strong_text_color(), visuals.selection.bg_fill));
            job.append(&text[end..], 0.0, format(visuals.text_color(), egui::Color32::TRANSPARENT));
        }
        None => job.append(text, 0.0, format(visuals.text_color(), egui::Color32::TRANSPARENT)),
    }
    job
}

/// The word at character `index` of the reading text, without surrounding punctuation.
fn word_under(reading: &Reading, index: usize) -> Option<String> {
    let byte = reading.text.char_indices().nth(index).map(|(b, _)| b).unwrap_or(reading.text.len());
    let &(start, end) = reading.spans.iter().find(|(s, e)| *s <= byte && byte < *e)?;
    let word = reading.text[start..end].trim_matches(WORD_TRIM);
    (!word.is_empty() && word.chars().count() <= 60).then(|| word.to_string())
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
            ui.ctx().request_repaint_after(Duration::from_millis(if self.full { 60 } else { 100 }));
        }
        let ctx = ui.ctx().clone();
        if self.full {
            if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                self.full_size = Some(rect.size());
            }
            egui::Panel::left("controls").exact_size(CONTROLS_WIDTH).resizable(false).show(ui, |ui| {
                ui.add_space(8.0);
                self.controls(ui, &snap);
            });
            egui::CentralPanel::default().show(ui, |ui| self.reading_panel(ui, &snap));
        } else {
            egui::CentralPanel::default().show(ui, |ui| self.controls(ui, &snap));
        }
        building_window(&ctx, &self.controller, &snap);
        self.options.show(&ctx, &self.controller, &mut self.hotkeys, reading);
    }

    fn on_exit(&mut self) {
        if self.full {
            self.save_full_size();
        }
    }
}
