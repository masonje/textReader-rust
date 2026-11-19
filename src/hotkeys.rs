//! Global keyboard shortcuts, from the same `hotkeys` settings as the Python app.

use crate::controller::Controller;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::collections::HashMap;
use std::sync::Arc;

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
    parts.join("+").parse().ok()
}

/// Register the shortcuts. Keep the returned manager alive for as long as they should work.
pub fn register(controller: Arc<Controller>) -> Option<GlobalHotKeyManager> {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("⚠️ Keyboard shortcuts aren't available: {e}");
            return None;
        }
    };
    let mut actions: HashMap<u32, &'static str> = HashMap::new();
    for (action, _) in crate::settings::DEFAULT_HOTKEYS {
        let text = controller.settings.lock().unwrap().hotkey(action);
        match to_hotkey(&text) {
            Some(hotkey) => match manager.register(hotkey) {
                Ok(()) => {
                    actions.insert(hotkey.id(), action);
                    println!("  {text}  - {action}");
                }
                Err(e) => eprintln!("⚠️ Couldn't use {text} for {action} (another program may have it): {e}"),
            },
            None => eprintln!("⚠️ Couldn't understand the shortcut {text:?} for {action}"),
        }
    }
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state != HotKeyState::Pressed {
            return;
        }
        let action = actions.get(&event.id).copied();
        println!("⌨ {}", action.unwrap_or("unknown shortcut"));
        match action {
            Some("read_selected_text") => controller.read_selection(),
            Some("play") => controller.play(),
            Some("pause") => controller.toggle_pause(),
            Some("stop") => controller.stop(),
            _ => {}
        }
    }));
    Some(manager)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_format_converts() {
        for text in ["Win+Esc", "Win+Shift+R", "Ctrl+Alt+P", "Win+Shift+S"] {
            assert!(to_hotkey(text).is_some(), "{text}");
        }
        assert_eq!(to_hotkey("Win+Esc"), to_hotkey("Super+Escape"));
    }
}
