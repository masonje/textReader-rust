//! eSpeak-NG: the `espeak-ng` program as an engine, and its library as the
//! phonemizer Piper needs (text -> IPA phonemes, one list per sentence).

use super::{Build, Engine, Voice, run_cancellable, which};
use anyhow::{Context, Result, anyhow};
use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use unicode_normalization::UnicodeNormalization;

// ---------------------------------------------------------------- engine

const PREFERRED: [(&str, &str); 4] = [
    ("en-us", "America (male)"),
    ("en-us+f3", "America (female)"),
    ("en-gb", "Great Britain (male)"),
    ("en-gb+f3", "Great Britain (female)"),
];

#[derive(Default)]
pub struct EspeakEngine {
    voices: OnceLock<Vec<Voice>>,
}

fn short_label(name: &str) -> String {
    let name = name.replace('_', " ");
    match name.strip_prefix("English (").and_then(|s| s.strip_suffix(')')) {
        Some(inner) => inner.to_string(),
        None => name,
    }
}

impl Engine for EspeakEngine {
    fn name(&self) -> &'static str {
        "eSpeak-NG"
    }
    fn ext(&self) -> &'static str {
        "wav"
    }
    fn missing(&self) -> Option<String> {
        (!which("espeak-ng")).then(|| "espeak-ng (system package)".into())
    }
    fn voices(&self) -> Vec<Voice> {
        self.voices
            .get_or_init(|| {
                let out = Command::new("espeak-ng").arg("--voices=en").output().map(|o| o.stdout).unwrap_or_default();
                let out = String::from_utf8_lossy(&out);
                let mut installed: Vec<Voice> = Vec::new();
                for line in out.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() < 5 || parts[4].starts_with("mb/") {
                        continue; // MBROLA voices need the separate mbrola program
                    }
                    let (language, gender, name) = (parts[1], parts[2], parts[3]);
                    if language == "variant" || language.contains("shaw") || installed.iter().any(|(id, _)| id == language) {
                        continue;
                    }
                    let gender = match gender.rsplit('/').next() {
                        Some("M") => " (male)",
                        Some("F") => " (female)",
                        _ => "",
                    };
                    installed.push((language.to_string(), format!("{}{gender}", short_label(name))));
                }
                let mut voices: Vec<Voice> = PREFERRED
                    .iter()
                    .filter(|(id, _)| installed.iter().any(|(i, _)| i == id.split('+').next().unwrap()))
                    .map(|(id, label)| (id.to_string(), label.to_string()))
                    .collect();
                let preferred_bases: Vec<&str> = PREFERRED.iter().map(|(id, _)| id.split('+').next().unwrap()).collect();
                voices.extend(installed.into_iter().filter(|(id, _)| !preferred_bases.contains(&id.as_str())));
                voices
            })
            .clone()
    }
    fn synthesize(&self, text: &str, voice: &str, out: &Path, build: &Build) -> Result<()> {
        let mut cmd = Command::new("espeak-ng");
        if !voice.is_empty() {
            cmd.args(["-v", voice]);
        }
        cmd.arg("-w").arg(out).arg("--stdin");
        run_cancellable(&mut cmd, Some(text), build, Duration::from_secs(120))
    }
}

// ---------------------------------------------------------------- phonemizer

struct Lib {
    _lib: Library,
    set_voice: unsafe extern "C" fn(*const c_char) -> c_int,
    to_phonemes: unsafe extern "C" fn(*mut *const c_void, c_int, c_int, *mut c_int) -> *const c_char,
}
unsafe impl Send for Lib {}

static LIB: OnceLock<std::result::Result<Mutex<Lib>, String>> = OnceLock::new();

fn lib() -> Result<&'static Mutex<Lib>> {
    LIB.get_or_init(|| load().map(Mutex::new).map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
}

fn load() -> Result<Lib> {
    let names: &[&str] = if cfg!(target_os = "windows") {
        &["libespeak-ng.dll", "espeak-ng.dll"]
    } else if cfg!(target_os = "macos") {
        &["libespeak-ng.dylib", "libespeak-ng.1.dylib", "/opt/homebrew/lib/libespeak-ng.dylib"]
    } else {
        &["libespeak-ng.so.1", "libespeak-ng.so"]
    };
    let lib = names
        .iter()
        .find_map(|n| unsafe { Library::new(*n).ok() })
        .ok_or_else(|| anyhow!("the espeak-ng library isn't installed (Piper needs it)"))?;
    unsafe {
        let init: unsafe extern "C" fn(c_int, c_int, *const c_char, c_int) -> c_int =
            *lib.get(b"espeak_Initialize\0").context("espeak_Initialize")?;
        // AUDIO_OUTPUT_RETRIEVAL: no sound device; we only want phonemes
        if init(1, 0, std::ptr::null(), 0) < 0 {
            return Err(anyhow!("espeak-ng failed to start"));
        }
        let set_voice = *lib.get(b"espeak_SetVoiceByName\0").context("espeak_SetVoiceByName")?;
        let to_phonemes = *lib
            .get(b"espeak_TextToPhonemesWithTerminator\0")
            .context("espeak-ng 1.52 or later is needed")?;
        Ok(Lib { _lib: lib, set_voice, to_phonemes })
    }
}

// Clause terminator flags (espeak-ng translate.h)
const INTONATION_MASK: c_int = 0x0000_7000;
const INTONATION_COMMA: c_int = 0x0000_1000;
const INTONATION_QUESTION: c_int = 0x0000_2000;
const INTONATION_EXCLAMATION: c_int = 0x0000_3000;
const INTONATION_FULL_STOP: c_int = 0x0000_0000;
const TYPE_CLAUSE: c_int = 0x0004_0000;
const TYPE_SENTENCE: c_int = 0x0008_0000;
const PAUSE_MASK: c_int = 0x0000_0FFF;

fn terminator(flags: c_int) -> (&'static str, bool) {
    let sentence = flags & TYPE_SENTENCE != 0;
    let punct = match flags & INTONATION_MASK {
        INTONATION_QUESTION => "?",
        INTONATION_EXCLAMATION => "!",
        INTONATION_COMMA if flags & PAUSE_MASK >= 30 => ";",
        INTONATION_COMMA => ",",
        INTONATION_FULL_STOP if sentence => ".",
        INTONATION_FULL_STOP if flags & TYPE_CLAUSE != 0 => ":",
        _ => "",
    };
    (punct, sentence)
}

/// Strip "(en)" style language-switch flags.
fn strip_flags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// IPA phonemes for `text`, one list of characters per sentence (as Piper does it).
pub fn phonemize(voice: &str, text: &str) -> Result<Vec<Vec<char>>> {
    let lib = lib()?.lock().unwrap();
    let voice = CString::new(voice)?;
    unsafe { (lib.set_voice)(voice.as_ptr()) };
    let text = CString::new(text.replace('\0', " "))?;
    let mut ptr: *const c_void = text.as_ptr() as *const c_void;
    let mut sentences = Vec::new();
    let mut current = String::new();
    while !ptr.is_null() {
        let mut flags: c_int = 0;
        // espeakCHARS_UTF8 = 1, espeakPHONEMES_IPA = 2
        let phonemes = unsafe { (lib.to_phonemes)(&mut ptr, 1, 2, &mut flags) };
        if phonemes.is_null() {
            break;
        }
        let clause = strip_flags(&unsafe { CStr::from_ptr(phonemes) }.to_string_lossy());
        let (punct, end_of_sentence) = terminator(flags);
        current.push_str(&clause);
        current.push_str(punct);
        if matches!(punct, "," | ":" | ";") {
            current.push(' ');
        }
        if end_of_sentence {
            sentences.push(current.nfd().collect());
            current.clear();
        }
    }
    if !current.is_empty() {
        sentences.push(current.nfd().collect());
    }
    Ok(sentences.into_iter().filter(|s: &Vec<char>| !s.is_empty()).collect())
}
