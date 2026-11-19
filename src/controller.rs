//! The reading logic, shared by the window, the global shortcuts and the tray.
//! Everything here is thread-safe; the window only shows `snapshot()`.

use crate::clipboard::{self, Clip};
use crate::engines::{self, Build, Engine, Voice};
use crate::player::Player;
use crate::settings::{Settings, data_dir};
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
    voice_used: Option<String>,
    seq: u64,
}

pub struct Controller {
    pub settings: Mutex<Settings>,
    pub engines: Vec<Arc<dyn Engine>>,
    player: Player,
    state: Mutex<State>,
    voice_cache: Mutex<HashMap<String, Vec<Voice>>>,
    repaint: OnceLock<Box<dyn Fn() + Send + Sync>>,
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
            voice_used: state.voice_used.clone(),
        }
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
        let engine = self.current_engine();
        let voice = self.current_voice(engine.name()).unwrap_or_default();
        let cancel = Arc::new(AtomicBool::new(false));
        let seq = {
            let mut state = self.state.lock().unwrap();
            if let Some(old) = state.build.replace(cancel.clone()) {
                old.store(true, Ordering::SeqCst); // a new read cancels the previous build
            }
            state.progress = String::new();
            state.message = None;
            state.seq += 1;
            state.seq
        };
        self.player.stop();
        self.repaint();
        println!("Speaking: {}…", text.chars().take(50).collect::<String>().replace('\n', " "));
        let this = self.clone();
        std::thread::spawn(move || {
            let out = data_dir().join(format!("clipboard_speech_rs.{}", engine.ext()));
            let tmp = data_dir().join(format!("clipboard_speech_rs.building{seq}.{}", engine.ext()));
            let progress_owner = this.clone();
            let build = Build {
                cancel: cancel.clone(),
                progress: Box::new(move |text| {
                    progress_owner.state.lock().unwrap().progress = text;
                    progress_owner.repaint();
                }),
            };
            let started = Instant::now();
            let result = engine.synthesize(&text, &voice.0, &tmp, &build);
            let mut state = this.state.lock().unwrap();
            let current = state.seq == seq && !cancel.load(Ordering::SeqCst);
            if current {
                state.build = None;
            }
            match result {
                Ok(()) if current => {
                    let _ = std::fs::rename(&tmp, &out);
                    state.last_audio = Some(out.clone());
                    state.voice_used = Some(format!("{} · {}", voice.1.trim_end_matches(" · download"), engine.name()));
                    drop(state);
                    // A Piper voice that was just downloaded no longer needs "· download"
                    this.voice_cache.lock().unwrap().remove(engine.name());
                    println!("Built in {:.1}s with {}", started.elapsed().as_secs_f64(), engine.name());
                    let speed = this.settings.lock().unwrap().speed();
                    this.player.play(&out, speed);
                }
                Err(e) if current && !engines::is_cancelled(&e) => {
                    drop(state);
                    let _ = std::fs::remove_file(&tmp);
                    this.say(format!("{} couldn't read it: {e:#}", engine.name()));
                }
                _ => {
                    drop(state);
                    let _ = std::fs::remove_file(&tmp);
                }
            }
            this.repaint();
        });
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

fn cleanup() {
    if let Ok(entries) = std::fs::read_dir(data_dir()) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("clipboard_speech_rs.") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
