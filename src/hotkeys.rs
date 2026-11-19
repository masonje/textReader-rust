//! Global keyboard shortcuts, from the same `hotkeys` settings as the Python app.

use crate::controller::Controller;
use crate::settings::DEFAULT_HOTKEYS;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const LABELS: [(&str, &str); 4] = [
    ("read_selected_text", "Read Selected Text"),
    ("play", "Play/Resume"),
    ("pause", "Pause"),
    ("stop", "Stop"),
];

/// "Win+Shift+R" (the app's format) -> "Super+Shift+R" (global-hotkey's)
pub fn to_hotkey(text: &str) -> Option<HotKey> {
    let parts: Vec<String> = text
        .split('+')
        .map(|p| match p.trim().to_lowercase().as_str() {
            "win" | "cmd" | "super" => "Super".to_string(),
            "esc" | "escape" => "Escape".to_string(),
            other => other.to_string(),
        })
        .collect();
    let hotkey: HotKey = parts.join("+").parse().ok()?;
    // A shortcut needs a modifier, or every press of that key would be taken
    if hotkey.mods.is_empty() { None } else { Some(hotkey) }
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    actions: Arc<Mutex<HashMap<u32, &'static str>>>,
    controller: Arc<Controller>,
}

impl Hotkeys {
    pub fn start(controller: Arc<Controller>) -> Option<Self> {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("⚠️ Keyboard shortcuts aren't available: {e}");
                return None;
            }
        };
        let actions: Arc<Mutex<HashMap<u32, &'static str>>> = Arc::default();
        let (handler_actions, handler_controller) = (actions.clone(), controller.clone());
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            let action = handler_actions.lock().unwrap().get(&event.id).copied();
            println!("⌨ {}", action.unwrap_or("unknown shortcut"));
            match action {
                Some("read_selected_text") => handler_controller.read_selection(),
                Some("play") => handler_controller.play(),
                Some("pause") => handler_controller.toggle_pause(),
                Some("stop") => handler_controller.stop(),
                _ => {}
            }
        }));
        let mut hotkeys = Hotkeys { manager, registered: Vec::new(), actions, controller };
        for problem in hotkeys.reload() {
            eprintln!("⚠️ {problem}");
        }
        Some(hotkeys)
    }

    /// (Re-)register the shortcuts from settings. Returns any that couldn't be used.
    pub fn reload(&mut self) -> Vec<String> {
        let _ = self.manager.unregister_all(&self.registered);
        self.registered.clear();
        let mut actions = self.actions.lock().unwrap();
        actions.clear();
        let mut problems = Vec::new();
        for (action, _) in DEFAULT_HOTKEYS {
            let text = self.controller.settings.lock().unwrap().hotkey(action);
            let label = LABELS.iter().find(|(a, _)| *a == action).map(|(_, l)| *l).unwrap_or(action);
            match to_hotkey(&text) {
                Some(hotkey) => match self.manager.register(hotkey) {
                    Ok(()) => {
                        actions.insert(hotkey.id(), action);
                        self.registered.push(hotkey);
                        println!("  {text}  - {label}");
                    }
                    Err(e) => problems.push(format!("Couldn't use {text} for {label} (another program may have it): {e}")),
                },
                None => problems.push(format!("Couldn't understand the shortcut {text:?} for {label}")),
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_format_converts() {
        for text in ["Win+Esc", "Win+Shift+R", "Ctrl+Alt+P", "Win+Shift+S", "Ctrl+Alt+Shift+F9"] {
            assert!(to_hotkey(text).is_some(), "{text}");
        }
        assert_eq!(to_hotkey("Win+Esc"), to_hotkey("Super+Escape"));
        assert!(to_hotkey("R").is_none(), "a key with no modifier isn't allowed");
        assert!(to_hotkey("Win+Nonsense").is_none());
    }
}
