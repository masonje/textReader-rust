//! The Options window: Engines & Voices, Pronunciations, Links/paths/codes, Shortcuts.

use crate::controller::Controller;
use crate::hotkeys::{self, Hotkeys};
use crate::pronunciation::{self, FILTERS, MODES, Rule};
use crate::settings::DEFAULT_HOTKEYS;
use eframe::egui;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Engines,
    Pronunciations,
    Filters,
    Shortcuts,
}

pub struct Options {
    pub open: bool,
    tab: Tab,
    selected_engine: Option<String>,
    // Pronunciations
    form: Rule,
    editing: Option<usize>,
    rules_message: Option<String>,
    // Links, paths & codes
    try_text: String,
    // Shortcuts
    shortcuts: Vec<String>,
    shortcuts_message: Option<String>,
}

const PREVIEW: &str = "Hello! This is a preview of this voice, reading a short sentence.";

fn speed_word(rtf: f64) -> &'static str {
    match rtf {
        r if r < 0.05 => "Very fast",
        r if r < 0.25 => "Fast",
        r if r < 0.6 => "Moderate",
        r if r < 1.0 => "Slow",
        _ => "Very slow",
    }
}

fn stars(n: u8) -> String {
    let n = n.min(5) as usize;
    "★".repeat(n) + &"☆".repeat(5 - n)
}

fn speed_sentence(stat: Option<(f64, u64)>) -> String {
    match stat {
        None => "Not measured yet. Read something or press Preview.".into(),
        Some((rtf, runs)) => {
            let seconds = rtf * 60.0;
            let took = if seconds >= 1.5 { format!("{seconds:.0} s") } else { format!("{seconds:.1} s") };
            format!("{}: about {took} to build 1 minute of speech ({runs} read{})", speed_word(rtf), if runs == 1 { "" } else { "s" })
        }
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1e6)
}

impl Options {
    pub fn new() -> Self {
        Options {
            open: false,
            tab: Tab::Engines,
            selected_engine: None,
            form: Rule::new("", "say", ""),
            editing: None,
            rules_message: None,
            try_text: "Read https://github.com/masonje/textReader/pull/6 and email jane.doe@example.com".into(),
            shortcuts: Vec::new(),
            shortcuts_message: None,
        }
    }

    /// Open the window, optionally on the Pronunciations tab with a word filled in.
    pub fn open(&mut self, controller: &Controller, word: Option<&str>) {
        self.open = true;
        self.shortcuts = DEFAULT_HOTKEYS.iter().map(|(a, _)| controller.settings.lock().unwrap().hotkey(a)).collect();
        if let Some(word) = word {
            // Show the word's rule if it has one, or start a new one for it
            self.tab = Tab::Pronunciations;
            self.rules_message = None;
            let rules = Self::rules(controller);
            match rules.iter().position(|r| r.text.to_lowercase() == word.trim().to_lowercase()) {
                Some(i) => {
                    self.editing = Some(i);
                    self.form = rules[i].clone();
                }
                None => {
                    self.editing = None;
                    self.form = Rule::new(word.trim(), "say", "");
                }
            }
        }
    }

    /// Whether a pronunciation rule exists for `word` (any case).
    pub fn has_rule(&self, controller: &Controller, word: &str) -> bool {
        Self::rules(controller).iter().any(|r| r.text.to_lowercase() == word.to_lowercase())
    }

    /// `--options=<tab>` on the command line: open on that tab (engines, pronunciations, filters, shortcuts).
    pub fn open_tab(&mut self, controller: &Controller, tab: &str) {
        self.open(controller, None);
        self.tab = match tab {
            "pronunciations" => Tab::Pronunciations,
            "filters" => Tab::Filters,
            "shortcuts" => Tab::Shortcuts,
            _ => Tab::Engines,
        };
    }

    pub fn show(&mut self, ctx: &egui::Context, c: &Arc<Controller>, hotkeys: &mut Option<Hotkeys>, reading: bool) {
        if !self.open {
            return;
        }
        let builder = egui::ViewportBuilder::default()
            .with_title("Options")
            .with_app_id("clipboard-reader")
            .with_inner_size([680.0, 520.0])
            .with_min_inner_size([520.0, 380.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("options"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Engines, "Engines & Voices");
                    ui.selectable_value(&mut self.tab, Tab::Pronunciations, "Pronunciations");
                    ui.selectable_value(&mut self.tab, Tab::Filters, "Links, paths & codes");
                    ui.selectable_value(&mut self.tab, Tab::Shortcuts, "Shortcuts");
                });
                ui.separator();
                match self.tab {
                    Tab::Engines => self.engines_tab(ui, c, reading),
                    Tab::Pronunciations => self.rules_tab(ui, c, reading),
                    Tab::Filters => self.filters_tab(ui, c, reading),
                    Tab::Shortcuts => self.shortcuts_tab(ui, c, hotkeys),
                }
            });
        });
    }

    // ------------------------------------------------------------ Engines & Voices

    fn engines_tab(&mut self, ui: &mut egui::Ui, c: &Arc<Controller>, reading: bool) {
        ui.label("Turn engines and voices on or off, try them, and download or delete voices. Turned-off ones don't appear in the Engine and Voice lists.");
        ui.add_space(4.0);
        let selected = self.selected_engine.clone().unwrap_or_else(|| c.current_engine().name().to_string());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for engine in c.engines.clone() {
                let name = engine.name();
                let engine_on = !c.settings.lock().unwrap().engine_disabled(name);
                let missing = engine.missing();
                ui.horizontal(|ui| {
                    let mut on = engine_on;
                    if ui.checkbox(&mut on, "").on_hover_text("Turn this engine on or off").changed() {
                        c.set_engine_on(name, on);
                    }
                    let open = selected == name;
                    let title = egui::RichText::new(format!("{} {name}", if open { "▾" } else { "▸" })).strong();
                    if ui.selectable_label(open, title).clicked() {
                        self.selected_engine = if open { Some(String::new()) } else { Some(name.to_string()) };
                    }
                    ui.weak(match engine.online() {
                        Some(company) => format!("Online ({company})"),
                        None => "Locally".into(),
                    });
                    ui.weak(c.speed_of(name, None).map(|(rtf, _)| speed_word(rtf)).unwrap_or("—"));
                    ui.weak(stars(engine.quality()));
                    if let Some(missing) = &missing {
                        ui.colored_label(ui.visuals().warn_fg_color, format!("not installed: {missing}"));
                    }
                });
                if selected != name {
                    continue;
                }
                ui.indent(name, |ui| {
                    ui.weak(engine.about());
                    match engine.online() {
                        Some(company) => ui.weak(format!("The text you read is sent to {company}'s servers, which send back the audio.")),
                        None => ui.weak("Made on this computer: nothing you read leaves it."),
                    };
                    let voices = c.all_voices(name);
                    ui.horizontal(|ui| {
                        if ui.button("All voices on").clicked() {
                            c.set_voices_on(name, &voices.iter().map(|v| v.0.clone()).collect::<Vec<_>>(), true);
                        }
                        if ui.button("All voices off").clicked() {
                            c.set_voices_on(name, &voices.iter().map(|v| v.0.clone()).collect::<Vec<_>>(), false);
                        }
                    });
                    egui::Grid::new(format!("voices-{name}")).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
                        for (id, label) in &voices {
                            let mut on = !c.settings.lock().unwrap().voice_disabled(name, id);
                            if ui.checkbox(&mut on, label.trim_end_matches(" · download")).changed() {
                                c.set_voices_on(name, std::slice::from_ref(id), on);
                            }
                            ui.weak(c.speed_of(name, Some(id)).map(|(rtf, _)| speed_word(rtf)).unwrap_or("—"))
                                .on_hover_text(speed_sentence(c.speed_of(name, Some(id))));
                            ui.horizontal(|ui| {
                                let downloading = c.download_progress(name, id);
                                let state = engine.download_state(id);
                                let ready = !matches!(state, Some((false, _)));
                                if ui
                                    .add_enabled(!reading && missing.is_none() && downloading.is_none(), egui::Button::new("▶ Preview"))
                                    .on_hover_text(if ready { "Speak a short sample with this voice" } else { "Downloads the voice first, then speaks a sample" })
                                    .clicked()
                                {
                                    c.speak(PREVIEW.into(), engine.clone(), (id.clone(), label.clone()), false);
                                }
                                match (state, downloading) {
                                    (_, Some(progress)) => {
                                        ui.weak(progress);
                                        if ui.small_button("Cancel").clicked() {
                                            c.cancel_download(name, id);
                                        }
                                    }
                                    (Some((false, size)), None) => {
                                        if ui.small_button(format!("Download ({})", megabytes(size))).clicked() {
                                            c.download_voice(name, id);
                                        }
                                    }
                                    (Some((true, size)), None) => {
                                        ui.weak(format!("Downloaded · {}", megabytes(size)));
                                        if ui
                                            .add_enabled(!reading, egui::Button::new("Delete download").small())
                                            .on_hover_text("Frees the space; the voice downloads again next time it's used")
                                            .clicked()
                                        {
                                            c.delete_voice(name, id);
                                        }
                                    }
                                    (None, None) => {}
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(6.0);
            }
        });
    }

    // ------------------------------------------------------------ Pronunciations

    fn rules(c: &Controller) -> Vec<Rule> {
        pronunciation::clean_rules(c.settings.lock().unwrap().get("pronunciations"))
    }

    fn save_rules(c: &Controller, rules: &[Rule]) {
        c.settings.lock().unwrap().set("pronunciations", Value::Array(rules.iter().map(Rule::to_json).collect()));
    }

    fn rules_tab(&mut self, ui: &mut egui::Ui, c: &Arc<Controller>, reading: bool) {
        ui.label("Change how words, names, abbreviations and symbols are read. Rules apply to every read.");
        ui.add_space(6.0);
        ui.strong(if self.editing.is_some() { "Edit rule" } else { "New rule" });
        egui::Grid::new("rule-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Word or text");
            ui.add(egui::TextEdit::singleline(&mut self.form.text).hint_text("e.g. Nguyen, SQL, [1]").desired_width(260.0));
            ui.end_row();
            ui.label("Read it");
            ui.horizontal(|ui| {
                for (mode, label) in MODES {
                    ui.radio_value(&mut self.form.mode, mode.to_string(), label);
                }
            });
            ui.end_row();
            if self.form.mode == "say" {
                ui.label("Say it as");
                ui.add(egui::TextEdit::singleline(&mut self.form.say).hint_text("how it sounds, e.g. win").desired_width(260.0));
                ui.end_row();
            }
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.form.whole_word, "Whole word only");
                ui.checkbox(&mut self.form.match_case, "Match upper/lower case exactly");
            });
            ui.end_row();
            ui.label("Use with");
            egui::ComboBox::from_id_salt("rule-engine")
                .selected_text(if self.form.engine.is_empty() { "All engines" } else { &self.form.engine })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.form.engine, String::new(), "All engines");
                    for e in &c.engines {
                        ui.selectable_value(&mut self.form.engine, e.name().to_string(), e.name());
                    }
                });
            ui.end_row();
        });
        ui.horizontal(|ui| {
            let valid = !self.form.text.trim().is_empty() && (self.form.mode != "say" || !self.form.say.trim().is_empty());
            let action = if self.editing.is_some() { "Save changes" } else { "Add rule" };
            if ui.add_enabled(valid, egui::Button::new(action)).clicked() {
                let mut rule = self.form.clone();
                rule.text = rule.text.trim().to_string();
                let mut rules = Self::rules(c);
                match self.editing {
                    Some(i) if i < rules.len() => rules[i] = rule,
                    _ => rules.push(rule),
                }
                Self::save_rules(c, &rules);
                self.rules_message = Some(format!("Saved. “{}” will be read {}.", self.form.text.trim(), match self.form.mode.as_str() {
                    "spell" => "letter by letter".to_string(),
                    "skip" => "as nothing (skipped)".to_string(),
                    _ => format!("as “{}”", self.form.say.trim()),
                }));
                self.editing = None;
                self.form = Rule::new("", "say", "");
            }
            if ui.button("New rule").clicked() {
                self.editing = None;
                self.form = Rule::new("", "say", "");
                self.rules_message = None;
            }
            ui.separator();
            ui.label("Hear it:");
            let can_hear = !reading && !self.form.text.trim().is_empty();
            let engine = c.current_engine();
            let voice = c.current_voice(engine.name()).unwrap_or_default();
            if ui.add_enabled(can_hear, egui::Button::new("As written")).clicked() {
                c.speak(self.form.text.clone(), engine.clone(), voice.clone(), false);
            }
            if ui.add_enabled(can_hear, egui::Button::new("With this rule")).clicked() {
                let mut rule = self.form.clone();
                rule.enabled = true;
                let (said, _) = pronunciation::apply(&self.form.text, &[rule], Some(engine.name()), None);
                c.speak(if said.trim().is_empty() { "(nothing)".into() } else { said }, engine, voice, false);
            }
        });
        if let Some(message) = &self.rules_message {
            ui.weak(message);
        }
        ui.separator();
        ui.strong(format!("Your rules ({})", Self::rules(c).len()));
        let mut rules = Self::rules(c);
        let mut changed = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if rules.is_empty() {
                ui.weak("No rules yet. Add one above, or right-click a word in the Full view.");
            }
            egui::Grid::new("rules").striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
                let mut delete = None;
                for (i, rule) in rules.iter_mut().enumerate() {
                    if ui.checkbox(&mut rule.enabled, "").on_hover_text("Turn this rule on or off").changed() {
                        changed = true;
                    }
                    ui.label(egui::RichText::new(&rule.text).strong());
                    ui.label(format!("→ {}", rule.describe()));
                    ui.weak(if rule.engine.is_empty() { "all engines".to_string() } else { rule.engine.clone() });
                    if ui.small_button("Edit").clicked() {
                        self.editing = Some(i);
                        self.form = rule.clone();
                        self.rules_message = None;
                    }
                    if ui.small_button("Delete").clicked() {
                        delete = Some(i);
                    }
                    ui.end_row();
                }
                if let Some(i) = delete {
                    rules.remove(i);
                    changed = true;
                    if self.editing == Some(i) {
                        self.editing = None;
                    }
                }
            });
        });
        if changed {
            Self::save_rules(c, &rules);
        }
    }

    // ------------------------------------------------------------ Links, paths & codes

    fn filters_tab(&mut self, ui: &mut egui::Ui, c: &Arc<Controller>, reading: bool) {
        ui.label("Choose how long, hard-to-listen-to text is read. Links, paths and codes are handled before word rules.");
        ui.add_space(4.0);
        let mut filters = pronunciation::clean_filters(c.settings.lock().unwrap().get("text_filters"));
        let mut changed = false;
        let engine = c.current_engine();
        let voice = c.current_voice(engine.name()).unwrap_or_default();
        egui::Grid::new("filters").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for f in &FILTERS {
                ui.strong(f.label);
                ui.vertical(|ui| {
                    ui.weak(f.example);
                    let current = filters.get(f.kind).cloned().unwrap_or_default();
                    let label = f.choices.iter().find(|(id, _)| *id == current).map(|(_, l)| *l).unwrap_or("");
                    egui::ComboBox::from_id_salt(f.kind).width(420.0).selected_text(label).show_ui(ui, |ui| {
                        for (id, label) in f.choices {
                            if ui.selectable_label(current == *id, *label).clicked() && current != *id {
                                filters.insert(f.kind.to_string(), id.to_string());
                                changed = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let said = pronunciation::filter_example(f.kind, &current);
                        ui.label(format!("Sounds like: “{said}”"));
                        if ui.add_enabled(!reading, egui::Button::new("▶ Hear").small()).clicked() {
                            c.speak(said, engine.clone(), voice.clone(), false);
                        }
                    });
                });
                ui.end_row();
            }
        });
        if changed {
            c.settings.lock().unwrap().set("text_filters", json!(filters));
        }
        ui.separator();
        ui.strong("Try it");
        ui.add(egui::TextEdit::multiline(&mut self.try_text).desired_rows(2).desired_width(f32::INFINITY));
        let spoken = c.spoken_text(&self.try_text, engine.name());
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Read as: “{spoken}”"));
            if ui.add_enabled(!reading && !self.try_text.trim().is_empty(), egui::Button::new("▶ Hear").small()).clicked() {
                c.speak(self.try_text.clone(), engine.clone(), voice.clone(), true);
            }
        });
    }

    // ------------------------------------------------------------ Shortcuts

    fn shortcuts_tab(&mut self, ui: &mut egui::Ui, c: &Arc<Controller>, hotkeys: &mut Option<Hotkeys>) {
        ui.label("Keyboard shortcuts work from any program. Write them like Win+Shift+R, Ctrl+Alt+P or Win+Esc.");
        ui.add_space(4.0);
        if self.shortcuts.len() != DEFAULT_HOTKEYS.len() {
            self.shortcuts = DEFAULT_HOTKEYS.iter().map(|(a, _)| c.settings.lock().unwrap().hotkey(a)).collect();
        }
        let mut all_valid = true;
        egui::Grid::new("shortcuts").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            for (i, (action, _)) in DEFAULT_HOTKEYS.iter().enumerate() {
                let label = hotkeys::LABELS.iter().find(|(a, _)| a == action).map(|(_, l)| *l).unwrap_or(action);
                ui.label(label);
                ui.add_sized([200.0, 22.0], egui::TextEdit::singleline(&mut self.shortcuts[i]));
                let parsed = hotkeys::to_hotkey(&self.shortcuts[i]);
                let duplicate = parsed.is_some()
                    && self.shortcuts.iter().enumerate().any(|(j, s)| j != i && hotkeys::to_hotkey(s) == parsed);
                if parsed.is_none() {
                    all_valid = false;
                    ui.colored_label(ui.visuals().warn_fg_color, "needs a modifier (Win, Ctrl, Alt or Shift) and a key");
                } else if duplicate {
                    all_valid = false;
                    ui.colored_label(ui.visuals().warn_fg_color, "used twice");
                } else {
                    ui.weak("✔");
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            let mut save = false;
            if ui.add_enabled(all_valid, egui::Button::new("Save")).clicked() {
                save = true;
            }
            if ui.button("Reset to defaults").clicked() {
                self.shortcuts = DEFAULT_HOTKEYS.iter().map(|(_, k)| k.to_string()).collect();
                save = true;
            }
            if save {
                let map: serde_json::Map<String, Value> =
                    DEFAULT_HOTKEYS.iter().zip(&self.shortcuts).map(|((a, _), s)| (a.to_string(), json!(s.trim()))).collect();
                c.settings.lock().unwrap().set("hotkeys", Value::Object(map));
                let problems = hotkeys.as_mut().map(Hotkeys::reload).unwrap_or_default();
                self.shortcuts_message = Some(if problems.is_empty() { "Shortcuts saved.".into() } else { problems.join("\n") });
            }
        });
        if let Some(message) = &self.shortcuts_message {
            ui.weak(message);
        }
    }
}
