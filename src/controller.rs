//! The reading logic, shared by the window, the global shortcuts and the tray.
//! Everything here is thread-safe; the window only shows `snapshot()`.

use crate::clipboard::{self, Clip};
use crate::engines::{self, Build, Engine, Voice};
use crate::player::Player;
use crate::pronunciation;
use crate::settings::{Settings, data_dir};
use crate::word_timing::{self, Span};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Stopped,
    Building(String),
    Playing,
    Paused,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: Status,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub has_audio: bool,
    pub message: Option<String>,
    pub voice_used: Option<String>,
    pub surprise: bool,
    pub building_for: Option<f64>, // seconds the current build has been running
}

/// The text being read and when each word is spoken (for the Full view).
#[derive(Default)]
pub struct Reading {
    pub text: String,
    pub spans: Vec<Span>,
    pub timeline: Vec<f64>,
    pub seq: u64,
}

impl Reading {
    /// The word being spoken at `position` seconds into the audio.
    pub fn word_at(&self, position: f64) -> Option<usize> {
        if self.timeline.is_empty() { None } else { Some(word_timing::word_at(&self.timeline, position).unwrap_or(0)) }
    }
}

impl Snapshot {
    pub fn reading(&self) -> bool {
        self.status != Status::Stopped
    }
}

#[derive(Default)]
struct State {
    build: Option<Arc<AtomicBool>>, // cancel flag of the build in progress
    progress: String,
    last_audio: Option<PathBuf>,
    message: Option<(String, Instant)>,
    cold: bool, // this build downloaded or loaded a voice
    downloads: HashMap<String, (Arc<AtomicBool>, String)>, // "engine|voice" -> (cancel, progress)
    voice_used: Option<(String, String)>, // (engine, voice label) of the last read
    seq: u64,
    reading: Arc<Reading>,
    build_started: Option<Instant>,
}

pub struct Controller {
    pub settings: Mutex<Settings>,
    pub engines: Vec<Arc<dyn Engine>>,
    player: Player,
    state: Mutex<State>,
    voice_cache: Mutex<HashMap<String, Vec<Voice>>>,
    repaint: OnceLock<Box<dyn Fn() + Send + Sync>>,
    pub tray_ok: AtomicBool,        // the tray icon is up, so the window can close to it
    show_requested: AtomicBool,     // the tray asked for the hidden window back
}

impl Controller {
    pub fn new(player: Player) -> Arc<Self> {
        let _ = std::fs::create_dir_all(data_dir());
        cleanup();
        Arc::new(Controller {
            settings: Mutex::new(Settings::load()),
            engines: engines::all(),
            player,
            state: Mutex::new(State::default()),
            voice_cache: Mutex::new(HashMap::new()),
            repaint: OnceLock::new(),
            tray_ok: AtomicBool::new(false),
            show_requested: AtomicBool::new(false),
        })
    }

    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.repaint.set(Box::new(f));
    }

    fn repaint(&self) {
        if let Some(f) = self.repaint.get() {
            f();
        }
    }

    pub fn engine(&self, name: &str) -> Option<Arc<dyn Engine>> {
        self.engines.iter().find(|e| e.name() == name).cloned()
    }

    /// Engines that are installed and not turned off in Options.
    pub fn enabled_engines(&self) -> Vec<&'static str> {
        let settings = self.settings.lock().unwrap();
        self.engines.iter().map(|e| e.name()).filter(|n| !settings.engine_disabled(n)).collect()
    }

    pub fn current_engine(&self) -> Arc<dyn Engine> {
        let name = self.settings.lock().unwrap().engine();
        self.engine(&name).unwrap_or_else(|| self.engines[0].clone())
    }

    /// The engine's voices, minus those turned off in Options.
    pub fn voices(&self, engine: &str) -> Vec<Voice> {
        let list = {
            let mut cache = self.voice_cache.lock().unwrap();
            cache
                .entry(engine.to_string())
                .or_insert_with(|| self.engine(engine).map(|e| e.voices()).unwrap_or_default())
                .clone()
        };
        let settings = self.settings.lock().unwrap();
        let enabled: Vec<Voice> = list.iter().filter(|(id, _)| !settings.voice_disabled(engine, id)).cloned().collect();
        if enabled.is_empty() { list } else { enabled }
    }

    pub fn current_voice(&self, engine: &str) -> Option<Voice> {
        let voices = self.voices(engine);
        let saved = self.settings.lock().unwrap().voice(engine);
        voices.iter().find(|(id, _)| Some(id) == saved.as_ref()).or(voices.first()).cloned()
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().unwrap();
        let status = if state.build.is_some() {
            Status::Building(state.progress.clone())
        } else if self.player.busy() {
            if self.player.paused() { Status::Paused } else { Status::Playing }
        } else {
            Status::Stopped
        };
        Snapshot {
            status,
            position: self.player.position(),
            duration: self.player.duration(),
            has_audio: state.last_audio.as_ref().is_some_and(|p| p.exists()),
            message: state.message.as_ref().filter(|(_, t)| t.elapsed().as_secs() < 8).map(|(m, _)| m.clone()),
            voice_used: state.voice_used.as_ref().map(|(engine, voice)| format!("{voice} · {engine}")),
            building_for: state.build.as_ref().and(state.build_started).map(|t| t.elapsed().as_secs_f64()),
            surprise: self.settings.lock().unwrap().get("surprise_voice").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    pub fn reading(&self) -> Arc<Reading> {
        self.state.lock().unwrap().reading.clone()
    }

    /// The tray asks for the window to come back (it's picked up by the window's next frame).
    pub fn request_show(&self) {
        self.show_requested.store(true, Ordering::SeqCst);
        self.repaint();
    }

    pub fn take_show_request(&self) -> bool {
        self.show_requested.swap(false, Ordering::SeqCst)
    }

    /// Closing the window hides it to the tray instead of quitting (Options ▸ Misc).
    pub fn close_to_tray(&self) -> bool {
        self.settings.lock().unwrap().get("close_to_tray").and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn set_close_to_tray(&self, on: bool) {
        self.settings.lock().unwrap().set("close_to_tray", Value::Bool(on));
    }

    pub fn set_surprise(&self, on: bool) {
        self.settings.lock().unwrap().set("surprise_voice", Value::Bool(on));
    }

    /// A random engine and voice from those that are on and ready (no download needed), avoiding
    /// the last one used when there's a choice, as the Python app's Surprise me does.
    fn pick_surprise(&self, exclude: &[&str]) -> Option<(Arc<dyn Engine>, Voice)> {
        let mut candidates = Vec::new();
        for name in self.enabled_engines() {
            let Some(engine) = self.engine(name) else { continue };
            if exclude.contains(&name) || engine.missing().is_some() {
                continue;
            }
            for voice in self.voices(name) {
                if engine.download_state(&voice.0).is_some_and(|(downloaded, _)| !downloaded) {
                    continue;
                }
                candidates.push((engine.clone(), voice));
            }
        }
        let last = self.state.lock().unwrap().voice_used.clone();
        if candidates.len() > 1 {
            if let Some((engine, label)) = last {
                let others: Vec<_> = candidates.iter().filter(|(e, v)| !(e.name() == engine && voice_label(&v.1) == label)).cloned().collect();
                if !others.is_empty() {
                    candidates = others;
                }
            }
        }
        if candidates.is_empty() {
            return None;
        }
        let i = (uuid::Uuid::new_v4().as_u128() % candidates.len() as u128) as usize;
        Some(candidates.swap_remove(i))
    }

    fn say(&self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("⚠️ {message}");
        self.state.lock().unwrap().message = Some((message, Instant::now()));
        self.repaint();
    }

    // ------------------------------------------------------------ actions

    /// Read the selected text, or the clipboard (the read shortcut / Read Clipboard).
    pub fn read_selection(self: &Arc<Self>) {
        let this = self.clone();
        std::thread::spawn(move || match clipboard::text_to_read() {
            Clip::Text(text) => this.read(text),
            Clip::Empty => this.say("No text to read: select or copy some first."),
            Clip::NoAnswer => this.say("The clipboard didn't answer (the app you copied from may be busy). Try copying again."),
        });
    }

    /// Build audio for `text` with the current engine and voice, then play it.
    pub fn read(self: &Arc<Self>, text: String) {
        if self.snapshot().surprise {
            if let Some((engine, voice)) = self.pick_surprise(&[]) {
                println!("🎲 Surprise voice: {} ({})", voice_label(&voice.1), engine.name());
                return self.start(text, engine, voice, true, true);
            }
        }
        let engine = self.current_engine();
        let voice = self.current_voice(engine.name()).unwrap_or_default();
        self.speak(text, engine, voice, true);
    }

    /// Speak `text` with a given engine and voice (Preview and "Hear it" use this).
    /// `with_rules`: apply pronunciations and filters first.
    pub fn speak(self: &Arc<Self>, text: String, engine: Arc<dyn Engine>, voice: Voice, with_rules: bool) {
        self.start(text, engine, voice, with_rules, false);
    }

    /// `surprise`: if the engine fails, try up to two other random ones instead of giving up.
    fn start(self: &Arc<Self>, text: String, engine: Arc<dyn Engine>, voice: Voice, with_rules: bool, surprise: bool) {
        let cancel = Arc::new(AtomicBool::new(false));
        let seq = {
            let mut state = self.state.lock().unwrap();
            if let Some(old) = state.build.replace(cancel.clone()) {
                old.store(true, Ordering::SeqCst); // a new read cancels the previous build
            }
            state.progress = String::new();
            state.build_started = Some(Instant::now());
            state.message = None;
            state.seq += 1;
            state.seq
        };
        self.player.stop();
        self.repaint();
        println!("Speaking: {}…", text.chars().take(50).collect::<String>().replace('\n', " "));
        let this = self.clone();
        std::thread::spawn(move || {
            let (mut engine, mut voice) = (engine, voice);
            let mut failed: Vec<&'static str> = Vec::new();
            loop {
                match this.build_once(&text, &engine, &voice, with_rules, seq, &cancel) {
                    Some(error) if surprise && failed.len() < 2 && !cancel.load(Ordering::SeqCst) => {
                        // Surprise me: try a different engine instead of giving up
                        failed.push(engine.name());
                        let Some(pick) = this.pick_surprise(&failed) else {
                            this.finish_failed(seq, error);
                            break;
                        };
                        println!("⚠️ {} failed ({error}); trying {} ({})", engine.name(), voice_label(&pick.1.1), pick.0.name());
                        this.state.lock().unwrap().progress = format!("{} didn't work. Trying {} instead…", engine.name(), pick.0.name());
                        this.repaint();
                        (engine, voice) = pick;
                    }
                    Some(error) => {
                        this.finish_failed(seq, error);
                        break;
                    }
                    None => break,
                }
            }
            this.repaint();
        });
    }

    fn finish_failed(&self, seq: u64, error: String) {
        let mut state = self.state.lock().unwrap();
        if state.seq != seq {
            return;
        }
        state.build = None;
        drop(state);
        self.say(error);
    }

    /// Build and play once. Returns an error message if the engine failed (None when done or cancelled).
    fn build_once(self: &Arc<Self>, text: &str, engine: &Arc<dyn Engine>, voice: &Voice, with_rules: bool, seq: u64, cancel: &Arc<AtomicBool>) -> Option<String> {
        let (spoken, edits) = if with_rules { self.apply_rules(text, engine.name()) } else { (text.to_string(), Vec::new()) };
        if spoken != text {
            println!("  (after pronunciations and filters: {}…)", spoken.chars().take(60).collect::<String>().replace('\n', " "));
        }
        if spoken.trim().is_empty() {
            return Some("Nothing left to read: your pronunciation settings skip all of this text".into());
        }
        let out = data_dir().join(format!("clipboard_speech_rs.{}", engine.ext()));
        let tmp = data_dir().join(format!("clipboard_speech_rs.building{seq}.{}", engine.ext()));
        let progress_owner = self.clone();
        let build = Build {
            cancel: cancel.clone(),
            progress: Box::new(move |text| {
                let mut state = progress_owner.state.lock().unwrap();
                state.progress = text;
                state.cold = true; // a download or model load: don't count this build's speed
                drop(state);
                progress_owner.repaint();
            }),
        };
        self.state.lock().unwrap().cold = false;
        let started = Instant::now();
        let result = engine.synthesize(&spoken, &voice.0, &tmp, &build);
        let mut state = self.state.lock().unwrap();
        let current = state.seq == seq && !cancel.load(Ordering::SeqCst);
        match result {
            Ok(()) if current => {
                state.build = None;
                let _ = std::fs::rename(&tmp, &out);
                state.last_audio = Some(out.clone());
                state.voice_used = Some((engine.name().to_string(), voice_label(&voice.1)));
                let spans = word_timing::tokenize(text);
                let spoken_words = (!edits.is_empty()).then(|| pronunciation::spoken_words(text, &spans, &edits));
                state.reading = Arc::new(Reading { text: text.to_string(), spans, timeline: Vec::new(), seq });
                drop(state);
                // A Piper voice that was just downloaded no longer needs "· download"
                self.voice_cache.lock().unwrap().remove(engine.name());
                let build_seconds = started.elapsed().as_secs_f64();
                println!("Built in {build_seconds:.1}s with {}", engine.name());
                let speed = self.settings.lock().unwrap().speed();
                if self.player.play(&out, speed) {
                    if let Some(audio_seconds) = self.player.duration() {
                        if !self.state.lock().unwrap().cold {
                            self.record_speed(engine.name(), &voice.0, build_seconds, audio_seconds);
                        }
                        self.time_words(seq, &out, audio_seconds, spoken_words);
                    }
                }
                None
            }
            Err(e) if current && !engines::is_cancelled(&e) => {
                drop(state);
                let _ = std::fs::remove_file(&tmp);
                Some(format!("{} couldn't read it: {e:#}", engine.name()))
            }
            _ => {
                drop(state);
                let _ = std::fs::remove_file(&tmp);
                None
            }
        }
    }

    /// Estimate when each word is spoken: an even spread at once, then refined with the pauses in the audio.
    fn time_words(&self, seq: u64, audio: &std::path::Path, duration: f64, spoken: Option<Vec<String>>) {
        let set = |silences: &[word_timing::Silence]| {
            let mut state = self.state.lock().unwrap();
            if state.reading.seq != seq {
                return false;
            }
            let r = &state.reading;
            let timeline = word_timing::build_timeline(&r.text, duration, silences, &r.spans, spoken.as_deref());
            state.reading = Arc::new(Reading { text: r.text.clone(), spans: r.spans.clone(), timeline, seq });
            true
        };
        if set(&[]) {
            self.repaint();
            let silences = word_timing::detect_silences(audio);
            set(&silences);
        }
    }

    /// Fold one build into the voice's running average (recent reads count most), as the Python app does.
    fn record_speed(&self, engine: &str, voice: &str, build_seconds: f64, audio_seconds: f64) {
        if audio_seconds < 0.5 || build_seconds <= 0.0 {
            return;
        }
        let mut settings = self.settings.lock().unwrap();
        let mut stats = settings.get("voice_stats").cloned().unwrap_or_else(|| serde_json::json!({}));
        if !stats.is_object() {
            stats = serde_json::json!({});
        }
        let key = format!("{engine}|{voice}");
        let (runs, rtf) = stats
            .get(&key)
            .map(|s| (s["runs"].as_u64().unwrap_or(0), s["rtf"].as_f64().unwrap_or(0.0)))
            .unwrap_or((0, 0.0));
        let weight = runs.min(9) as f64;
        let rtf = (rtf * weight + build_seconds / audio_seconds) / (weight + 1.0);
        stats[key] = serde_json::json!({"runs": runs + 1, "rtf": rtf});
        settings.set("voice_stats", stats);
    }

    /// (seconds of build per second of speech, number of reads) for a voice, or the engine's average.
    pub fn speed_of(&self, engine: &str, voice: Option<&str>) -> Option<(f64, u64)> {
        let settings = self.settings.lock().unwrap();
        let stats = settings.get("voice_stats")?.as_object()?;
        let matching: Vec<(f64, u64)> = stats
            .iter()
            .filter(|(k, _)| match voice {
                Some(v) => k.as_str() == format!("{engine}|{v}"),
                None => k.starts_with(&format!("{engine}|")),
            })
            .filter_map(|(_, s)| Some((s["rtf"].as_f64()?, s["runs"].as_u64()?)))
            .collect();
        let runs: u64 = matching.iter().map(|(_, r)| r).sum();
        if runs == 0 {
            return None;
        }
        Some((matching.iter().map(|(rtf, r)| rtf * *r as f64).sum::<f64>() / runs as f64, runs))
    }

    // ------------------------------------------------------------ Options: on/off, downloads

    fn names(settings: &Settings, key: &str) -> Vec<String> {
        settings.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
    }

    /// Turn an engine on or off. At least one engine always stays on.
    pub fn set_engine_on(&self, engine: &str, on: bool) {
        let mut settings = self.settings.lock().unwrap();
        let mut off = Self::names(&settings, "disabled_engines");
        off.retain(|n| n != engine);
        if !on {
            off.push(engine.to_string());
            if self.engines.iter().all(|e| off.iter().any(|n| n == e.name())) {
                return;
            }
        }
        settings.set("disabled_engines", serde_json::json!(off));
        // If the current engine was turned off, switch to the first one still on
        if !on && settings.engine() == engine {
            if let Some(next) = self.engines.iter().map(|e| e.name()).find(|n| !off.iter().any(|o| o == n)) {
                settings.set_engine(next);
            }
        }
    }

    /// Turn voices on or off. At least one voice per engine always stays on.
    pub fn set_voices_on(&self, engine: &str, voices: &[String], on: bool) {
        let all: Vec<String> = {
            let mut cache = self.voice_cache.lock().unwrap();
            cache.entry(engine.to_string()).or_insert_with(|| self.engine(engine).map(|e| e.voices()).unwrap_or_default()).iter().map(|(id, _)| id.clone()).collect()
        };
        let mut settings = self.settings.lock().unwrap();
        let mut map = settings.get("disabled_voices").cloned().filter(Value::is_object).unwrap_or_else(|| serde_json::json!({}));
        let mut off: Vec<String> = map.get(engine).and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        off.retain(|v| !voices.contains(v));
        if !on {
            off.extend(voices.iter().cloned());
            if all.iter().all(|v| off.contains(v)) {
                // Keep the first one on
                if let Some(first) = all.first() {
                    off.retain(|v| v != first);
                }
            }
        }
        map[engine] = serde_json::json!(off);
        settings.set("disabled_voices", map);
    }

    /// Every voice the engine has (including turned-off ones), for Options.
    pub fn all_voices(&self, engine: &str) -> Vec<Voice> {
        let mut cache = self.voice_cache.lock().unwrap();
        cache.entry(engine.to_string()).or_insert_with(|| self.engine(engine).map(|e| e.voices()).unwrap_or_default()).clone()
    }

    pub fn download_progress(&self, engine: &str, voice: &str) -> Option<String> {
        self.state.lock().unwrap().downloads.get(&format!("{engine}|{voice}")).map(|(_, p)| p.clone())
    }

    pub fn download_voice(self: &Arc<Self>, engine: &str, voice: &str) {
        let Some(e) = self.engine(engine) else { return };
        let key = format!("{engine}|{voice}");
        let cancel = Arc::new(AtomicBool::new(false));
        self.state.lock().unwrap().downloads.insert(key.clone(), (cancel.clone(), "Starting download…".into()));
        let (this, voice) = (self.clone(), voice.to_string());
        std::thread::spawn(move || {
            let owner = this.clone();
            let progress_key = key.clone();
            let build = Build {
                cancel,
                progress: Box::new(move |text| {
                    if let Some(entry) = owner.state.lock().unwrap().downloads.get_mut(&progress_key) {
                        entry.1 = text;
                    }
                    owner.repaint();
                }),
            };
            let result = e.download(&voice, &build);
            this.state.lock().unwrap().downloads.remove(&key);
            this.voice_cache.lock().unwrap().remove(e.name());
            if let Err(err) = result {
                if !engines::is_cancelled(&err) {
                    this.say(format!("The download failed: {err:#}"));
                }
            }
            this.repaint();
        });
    }

    pub fn cancel_download(&self, engine: &str, voice: &str) {
        if let Some((cancel, _)) = self.state.lock().unwrap().downloads.get(&format!("{engine}|{voice}")) {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn delete_voice(&self, engine: &str, voice: &str) {
        if let Some(e) = self.engine(engine) {
            let _ = e.delete(voice);
            self.voice_cache.lock().unwrap().remove(engine);
        }
    }

    /// The text the voice is given: links/paths/codes shortened and word rules applied.
    pub fn spoken_text(&self, text: &str, engine: &str) -> String {
        self.apply_rules(text, engine).0
    }

    fn apply_rules(&self, text: &str, engine: &str) -> (String, Vec<pronunciation::Edit>) {
        let settings = self.settings.lock().unwrap();
        let rules = pronunciation::clean_rules(settings.get("pronunciations"));
        let filters = pronunciation::clean_filters(settings.get("text_filters"));
        drop(settings);
        pronunciation::apply(text, &rules, Some(engine), Some(&filters))
    }

    /// Play: resume after a pause, or replay the last audio.
    pub fn play(&self) {
        if self.player.paused() {
            self.player.resume();
        } else if let Some(path) = self.state.lock().unwrap().last_audio.clone().filter(|p| p.exists()) {
            let speed = self.settings.lock().unwrap().speed();
            self.player.play(&path, speed);
        }
        self.repaint();
    }

    pub fn toggle_pause(&self) {
        if self.player.paused() {
            self.player.resume();
        } else if self.player.busy() {
            self.player.pause();
        }
        self.repaint();
    }

    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(cancel) = state.build.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        drop(state);
        self.player.stop();
        self.repaint();
    }

    pub fn set_speed(&self, speed: f64) {
        self.player.set_speed(speed);
        self.settings.lock().unwrap().set_speed(speed);
    }

    pub fn set_engine(&self, name: &str) {
        self.settings.lock().unwrap().set_engine(name);
    }

    pub fn set_voice(&self, engine: &str, voice: &str) {
        self.settings.lock().unwrap().set_voice(engine, voice);
    }

    pub fn shutdown(&self) {
        self.stop();
        cleanup();
    }
}

/// "Amy · medium · download" -> "Amy"
fn voice_label(label: &str) -> String {
    label.split(" · ").next().unwrap_or(label).to_string()
}

fn cleanup() {
    if let Ok(entries) = std::fs::read_dir(data_dir()) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("clipboard_speech_rs.") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Options actions, against a temporary settings folder.
    #[test]
    fn options_actions_save_and_keep_one_on() {
        let home = std::env::temp_dir().join(format!("clipboard-reader-test-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".textReader")).unwrap();
        std::fs::write(home.join(".textReader/settings.json"), r#"{"tts_engine": "gTTS", "python_only_key": 42}"#).unwrap();
        // Only this test changes HOME, and it runs before anything reads it in this process
        unsafe { std::env::set_var("HOME", &home) };
        let c = Controller::new(Player::new().expect("libmpv"));

        // Turning every engine off leaves the last one on, and the current engine moves off a turned-off one
        for e in c.engines.clone() {
            c.set_engine_on(e.name(), false);
        }
        let on = c.enabled_engines();
        assert_eq!(on.len(), 1, "{on:?}");
        assert_eq!(c.current_engine().name(), on[0]);

        // Turning every gTTS voice off leaves the first one on
        let ids: Vec<String> = c.all_voices("gTTS").into_iter().map(|v| v.0).collect();
        c.set_voices_on("gTTS", &ids, false);
        assert_eq!(c.voices("gTTS").len(), 1);
        c.set_voices_on("gTTS", &ids, true);
        assert_eq!(c.voices("gTTS").len(), ids.len());

        // Speed stats average like the Python app's
        c.record_speed("gTTS", "com", 1.0, 10.0);
        c.record_speed("gTTS", "com", 3.0, 10.0);
        let (rtf, runs) = c.speed_of("gTTS", Some("com")).unwrap();
        assert_eq!(runs, 2);
        assert!((rtf - 0.2).abs() < 1e-9, "{rtf}");

        // Rules and filters are applied to what's read; keys the Python app owns survive
        c.settings.lock().unwrap().set("pronunciations", serde_json::json!([{"text": "SQL", "mode": "spell"}]));
        assert_eq!(c.spoken_text("Run SQL https://example.com/a/b", "gTTS"), "Run ess cue el example.com path b");
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".textReader/settings.json")).unwrap()).unwrap();
        assert_eq!(saved["python_only_key"], 42);
        assert!(saved["voice_stats"]["gTTS|com"]["runs"].as_u64() == Some(2));
        let _ = std::fs::remove_dir_all(&home);
    }
}
