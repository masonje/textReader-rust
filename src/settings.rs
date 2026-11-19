//! ~/.textReader/settings.json, shared with the Python version of the app.
//!
//! Only the keys this version uses are read, and saving re-reads the file and
//! changes just those keys, so settings the Python app owns are left alone.

use serde_json::{Map, Value, json};
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".textReader")
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub const DEFAULT_HOTKEYS: [(&str, &str); 4] = [
    ("read_selected_text", "Win+Esc"),
    ("play", "Win+Shift+R"),
    ("pause", "Win+Shift+P"),
    ("stop", "Win+Shift+S"),
];

#[derive(Clone, Debug)]
pub struct Settings {
    raw: Map<String, Value>,
}

impl Settings {
    pub fn load() -> Self {
        let raw = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        Settings { raw }
    }

    pub fn engine(&self) -> String {
        self.raw.get("tts_engine").and_then(Value::as_str).unwrap_or("Piper").to_string()
    }

    pub fn speed(&self) -> f64 {
        self.raw.get("playback_speed").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.5, 2.0)
    }

    pub fn voice(&self, engine: &str) -> Option<String> {
        self.raw.get("voices")?.get(engine)?.as_str().map(str::to_string)
    }

    pub fn engine_disabled(&self, engine: &str) -> bool {
        self.raw
            .get("disabled_engines")
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(engine)))
    }

    pub fn voice_disabled(&self, engine: &str, voice: &str) -> bool {
        self.raw
            .get("disabled_voices")
            .and_then(|v| v.get(engine))
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(voice)))
    }

    pub fn hotkey(&self, action: &str) -> String {
        let default = DEFAULT_HOTKEYS.iter().find(|(a, _)| *a == action).map(|(_, k)| *k).unwrap_or("");
        self.raw
            .get("hotkeys")
            .and_then(|h| h.get(action))
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    }

    pub fn set_engine(&mut self, engine: &str) {
        self.raw.insert("tts_engine".into(), json!(engine));
        self.save(|file| {
            file.insert("tts_engine".into(), json!(engine));
        });
    }

    pub fn set_speed(&mut self, speed: f64) {
        let speed = (speed * 10.0).round() / 10.0;
        self.raw.insert("playback_speed".into(), json!(speed));
        self.save(|file| {
            file.insert("playback_speed".into(), json!(speed));
        });
    }

    pub fn set_voice(&mut self, engine: &str, voice: &str) {
        fn put(map: &mut Map<String, Value>, engine: &str, voice: &str) {
            let voices = map.entry("voices").or_insert_with(|| json!({}));
            if !voices.is_object() {
                *voices = json!({});
            }
            voices.as_object_mut().unwrap().insert(engine.into(), json!(voice));
        }
        put(&mut self.raw, engine, voice);
        self.save(|file| put(file, engine, voice));
    }

    /// Re-read the file, apply `change`, and write it back.
    fn save(&self, change: impl FnOnce(&mut Map<String, Value>)) {
        let path = settings_path();
        let mut file = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        change(&mut file);
        let _ = std::fs::create_dir_all(data_dir());
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&Value::Object(file)).unwrap_or_default();
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}
