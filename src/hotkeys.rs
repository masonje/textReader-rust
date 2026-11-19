//! Global keyboard shortcuts, from the same `hotkeys` settings as the Python app.
//!
//! Like the Python app (pynput), this watches every key press with X's RECORD extension
//! instead of grabbing the key combinations. Grabbing fails on desktops whose menu owns the
//! Win key (MATE's Brisk menu takes the whole keyboard as soon as Win goes down), and
//! watching never takes a key away from other programs.

use crate::controller::Controller;
use crate::settings::DEFAULT_HOTKEYS;
use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, Mutex};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::record::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _};

pub const LABELS: [(&str, &str); 4] = [
    ("read_selected_text", "Read Selected Text"),
    ("play", "Play/Resume"),
    ("pause", "Pause"),
    ("stop", "Stop"),
];

/// A shortcut: the names of the keys held together, e.g. {"win", "shift", "r"}.
pub type Combo = BTreeSet<String>;

const MODIFIERS: [&str; 4] = ["win", "ctrl", "alt", "shift"];

/// "Win+Shift+R" -> {"win", "shift", "r"}. Needs at least one modifier and one known key.
pub fn to_hotkey(text: &str) -> Option<Combo> {
    let combo: Combo = text
        .split('+')
        .map(|p| match p.trim().to_lowercase().as_str() {
            "cmd" | "command" | "super" | "windows" | "meta" => "win".to_string(),
            "control" => "ctrl".to_string(),
            "escape" => "esc".to_string(),
            "return" => "enter".to_string(),
            "pageup" | "pgup" => "page_up".to_string(),
            "pagedown" | "pgdn" => "page_down".to_string(),
            other => other.to_string(),
        })
        .filter(|p| !p.is_empty())
        .collect();
    let keys: Vec<&String> = combo.iter().filter(|p| !MODIFIERS.contains(&p.as_str())).collect();
    let has_modifier = combo.iter().any(|p| MODIFIERS.contains(&p.as_str()));
    // A shortcut needs a modifier, or every press of that key would trigger it
    (has_modifier && keys.len() == 1 && keysym_of(keys[0]).is_some()).then_some(combo)
}

/// The X keysym for a key name (only used to check the name is a real key).
fn keysym_of(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        return (c.is_ascii_graphic()).then_some(c.to_ascii_lowercase() as u32);
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0xffbe + n - 1);
    }
    SPECIAL.iter().find(|(_, n)| *n == name).map(|(k, _)| *k)
}

const SPECIAL: [(u32, &str); 26] = [
    (0xff1b, "esc"),
    (0xff0d, "enter"),
    (0xff8d, "enter"),
    (0x0020, "space"),
    (0xff09, "tab"),
    (0xff08, "backspace"),
    (0xffff, "delete"),
    (0xff63, "insert"),
    (0xff50, "home"),
    (0xff57, "end"),
    (0xff55, "page_up"),
    (0xff56, "page_down"),
    (0xff51, "left"),
    (0xff52, "up"),
    (0xff53, "right"),
    (0xff54, "down"),
    (0xff13, "pause"),
    (0xff61, "print_screen"),
    (0xffeb, "win"),
    (0xffec, "win"),
    (0xffe3, "ctrl"),
    (0xffe4, "ctrl"),
    (0xffe9, "alt"),
    (0xffea, "alt"),
    (0xffe1, "shift"),
    (0xffe2, "shift"),
];

/// The name of the key with this keysym, as shortcuts write it.
fn key_name(keysym: u32) -> Option<String> {
    match keysym {
        0xffe7 | 0xffe8 | 0xfe03 => Some("alt".into()), // Meta, AltGr
        0x21..=0x7e => Some((keysym as u8 as char).to_ascii_lowercase().to_string()),
        0xffbe..=0xffd5 => Some(format!("f{}", keysym - 0xffbe + 1)),
        _ => SPECIAL.iter().find(|(k, _)| *k == keysym).map(|(_, n)| n.to_string()),
    }
}

type Shortcuts = Arc<Mutex<Vec<(&'static str, Combo)>>>;

pub struct Hotkeys {
    shortcuts: Shortcuts,
    controller: Arc<Controller>,
}

impl Hotkeys {
    pub fn start(controller: Arc<Controller>) -> Option<Self> {
        let shortcuts: Shortcuts = Arc::default();
        let mut hotkeys = Hotkeys { shortcuts: shortcuts.clone(), controller: controller.clone() };
        for problem in hotkeys.reload() {
            eprintln!("⚠️ {problem}");
        }
        std::thread::spawn(move || {
            if let Err(e) = watch_keys(&shortcuts, &controller) {
                eprintln!("⚠️ Keyboard shortcuts aren't available: {e}");
            }
        });
        Some(hotkeys)
    }

    /// (Re-)read the shortcuts from settings. Returns any that couldn't be used.
    pub fn reload(&mut self) -> Vec<String> {
        let mut shortcuts = Vec::new();
        let mut problems = Vec::new();
        for (action, _) in DEFAULT_HOTKEYS {
            let text = self.controller.settings.lock().unwrap().hotkey(action);
            let label = LABELS.iter().find(|(a, _)| *a == action).map(|(_, l)| *l).unwrap_or(action);
            match to_hotkey(&text) {
                Some(combo) => {
                    println!("  {text}  - {label}");
                    shortcuts.push((action, combo));
                }
                None => problems.push(format!("Couldn't understand the shortcut {text:?} for {label}")),
            }
        }
        *self.shortcuts.lock().unwrap() = shortcuts;
        problems
    }
}

fn run(action: &str, controller: &Arc<Controller>) {
    println!("⌨ {action}");
    let (action, controller) = (action.to_string(), controller.clone());
    // On its own thread, so a slow action never holds up watching the keyboard
    std::thread::spawn(move || match action.as_str() {
        "read_selected_text" => controller.read_selection(),
        "play" => controller.play(),
        "pause" => controller.toggle_pause(),
        "stop" => controller.stop(),
        _ => {}
    });
}

/// Watch every key press and release (X RECORD), and run the shortcuts whose keys are all held.
fn watch_keys(shortcuts: &Shortcuts, controller: &Arc<Controller>) -> Result<(), Box<dyn std::error::Error>> {
    // RECORD needs two connections: one to set it up, one that receives the key events
    let (ctrl, _) = x11rb::connect(None)?;
    let (data, _) = x11rb::connect(None)?;
    if ctrl.extension_information(record::X11_EXTENSION_NAME)?.is_none() {
        return Err("the X server doesn't have the RECORD extension".into());
    }

    // Keycode -> key name, from the keyboard layout
    let setup = ctrl.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = ctrl.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = mapping.keysyms_per_keycode.max(1) as usize;
    let mut names: Vec<Option<String>> = vec![None; 256];
    for (i, syms) in mapping.keysyms.chunks(per).enumerate() {
        names[min as usize + i] = syms.iter().find(|&&s| s != 0).and_then(|&s| key_name(s));
    }

    let context = ctrl.generate_id()?;
    let empty = record::Range8 { first: 0, last: 0 };
    let empty_ext = record::ExtRange { major: empty, minor: record::Range16 { first: 0, last: 0 } };
    let range = record::Range {
        core_requests: empty,
        core_replies: empty,
        ext_requests: empty_ext,
        ext_replies: empty_ext,
        delivered_events: empty,
        device_events: record::Range8 { first: xproto::KEY_PRESS_EVENT, last: xproto::KEY_RELEASE_EVENT },
        errors: empty,
        client_started: false,
        client_died: false,
    };
    ctrl.record_create_context(context, 0, &[record::CS::ALL_CLIENTS.into()], &[range])?.check()?;

    const RECORD_FROM_SERVER: u8 = 0;
    let mut held: HashSet<String> = HashSet::new();
    let mut active: HashSet<&'static str> = HashSet::new();
    for reply in data.record_enable_context(context)? {
        let reply = reply?;
        if reply.category != RECORD_FROM_SERVER || reply.client_swapped {
            continue;
        }
        // Core events are 32 bytes each: type, then the keycode
        for event in reply.data.chunks_exact(32) {
            let pressed = match event[0] & 0x7f {
                xproto::KEY_PRESS_EVENT => true,
                xproto::KEY_RELEASE_EVENT => false,
                _ => continue,
            };
            let Some(name) = names[event[1] as usize].clone() else { continue };
            let shortcuts = shortcuts.lock().unwrap();
            if pressed {
                held.insert(name);
                if let Some((action, _)) = shortcuts.iter().find(|(a, combo)| !active.contains(a) && combo.iter().all(|k| held.contains(k))) {
                    active.insert(action);
                    run(action, controller);
                }
            } else {
                held.remove(&name);
                active.retain(|a| shortcuts.iter().any(|(b, combo)| b == a && combo.iter().all(|k| held.contains(k))));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_format_converts() {
        for text in ["Win+Esc", "Win+Shift+R", "Ctrl+Alt+P", "Win+Shift+S", "Ctrl+Alt+Shift+F9", "Super+Escape"] {
            assert!(to_hotkey(text).is_some(), "{text}");
        }
        assert_eq!(to_hotkey("Win+Esc"), to_hotkey("Super+Escape"));
        assert_eq!(to_hotkey("Shift+Win+r"), to_hotkey("Win+Shift+R"));
        assert!(to_hotkey("R").is_none(), "a key with no modifier isn't allowed");
        assert!(to_hotkey("Win+Shift").is_none(), "modifiers alone aren't a shortcut");
        assert!(to_hotkey("Win+Nonsense").is_none());
    }

    #[test]
    fn key_names_match_shortcuts() {
        assert_eq!(key_name(0x52).as_deref(), Some("r")); // R (with Shift)
        assert_eq!(key_name(0xff1b).as_deref(), Some("esc"));
        assert_eq!(key_name(0xffeb).as_deref(), Some("win"));
        assert_eq!(key_name(0xffc9).as_deref(), Some("f12"));
        assert_eq!(key_name(0xffe7).as_deref(), Some("alt"));
    }
}
