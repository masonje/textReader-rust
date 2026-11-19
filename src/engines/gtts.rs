//! gTTS: Google Translate's speech (the same requests the gTTS Python package makes).
//! One voice; the Google domain changes the accent.

use super::{Build, Engine, Voice};
use anyhow::{Result, anyhow};
use base64::Engine as _;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const ACCENTS: [(&str, &str); 8] = [
    ("com", "US English"),
    ("co.uk", "British English"),
    ("com.au", "Australian English"),
    ("ca", "Canadian English"),
    ("co.in", "Indian English"),
    ("ie", "Irish English"),
    ("co.za", "South African English"),
    ("com.ng", "Nigerian English"),
];

const MAX_CHARS: usize = 100; // Google takes at most 100 characters per request

pub struct Gtts;

/// Split into pieces of at most MAX_CHARS, preferring sentence and clause ends, then spaces.
pub fn split_text(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut clause = String::new();
    for ch in text.chars() {
        clause.push(ch);
        if matches!(ch, '.' | '!' | '?' | ';' | ':' | ',' | '\n' | '…') {
            pieces.push(std::mem::take(&mut clause));
        }
    }
    pieces.push(clause);
    let mut out: Vec<String> = Vec::new();
    for piece in pieces {
        let piece = piece.split_whitespace().collect::<Vec<_>>().join(" ");
        if piece.trim_matches(|c: char| !c.is_alphanumeric()).is_empty() {
            if let Some(last) = out.last_mut() {
                if last.len() + piece.len() < MAX_CHARS {
                    last.push_str(&piece);
                }
            }
            continue;
        }
        // Join short clauses together, and break long ones at spaces
        for word in piece.split(' ') {
            match out.last_mut() {
                Some(last) if last.chars().count() + 1 + word.chars().count() <= MAX_CHARS && !ends_sentence(last) => {
                    last.push(' ');
                    last.push_str(word);
                }
                _ => out.push(word.chars().take(MAX_CHARS).collect()),
            }
        }
    }
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

fn ends_sentence(s: &str) -> bool {
    s.ends_with(['.', '!', '?'])
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn request(text: &str, tld: &str) -> Result<Vec<u8>> {
    let parameter = serde_json::to_string(&serde_json::json!([text, "en", null, "null"]))?;
    let rpc = serde_json::to_string(&serde_json::json!([[["jQ1olc", parameter, null, "generic"]]]))?;
    let body = format!("f.req={}&", url_encode(&rpc));
    let url = format!("https://translate.google.{tld}/_/TranslateWebserverUi/data/batchexecute");
    let mut response = ureq::post(&url)
        .header("Referer", "http://translate.google.com/")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/47.0.2526.106 Safari/537.36",
        )
        .header("Content-Type", "application/x-www-form-urlencoded;charset=utf-8")
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .send(body.as_str())?;
    let text = response.body_mut().read_to_string()?;
    let mut audio = Vec::new();
    for line in text.lines().filter(|l| l.contains("jQ1olc")) {
        let marker = r#"jQ1olc","[\""#;
        if let Some(start) = line.find(marker).map(|i| i + marker.len()) {
            if let Some(end) = line[start..].find(r#"\"]"#) {
                audio.extend(base64::engine::general_purpose::STANDARD.decode(&line[start..start + end])?);
            }
        }
    }
    Ok(audio)
}

impl Engine for Gtts {
    fn name(&self) -> &'static str {
        "gTTS"
    }
    fn ext(&self) -> &'static str {
        "mp3"
    }
    fn missing(&self) -> Option<String> {
        None
    }
    fn online(&self) -> Option<&'static str> {
        Some("Google")
    }
    fn about(&self) -> &'static str {
        "Google Translate's text-to-speech. A clear, natural female voice; the accent changes with the Google domain."
    }
    fn voices(&self) -> Vec<Voice> {
        ACCENTS.iter().map(|(id, label)| (id.to_string(), label.to_string())).collect()
    }
    fn synthesize(&self, text: &str, voice: &str, out: &Path, build: &Build) -> Result<()> {
        let tld = if voice.is_empty() { "com" } else { voice };
        let mut file = std::fs::File::create(out)?;
        for piece in split_text(text) {
            build.check()?;
            // Google sometimes answers with no audio (usually when limiting requests); retry once.
            let mut audio = request(&piece, tld)?;
            if audio.is_empty() {
                std::thread::sleep(Duration::from_millis(1500));
                build.check()?;
                audio = request(&piece, tld)?;
            }
            if audio.is_empty() {
                return Err(anyhow!("Google sent no audio (it may be limiting requests; try again shortly)"));
            }
            file.write_all(&audio)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_fit_and_keep_all_words() {
        let text = "This is a sentence. ".repeat(20) + &"word ".repeat(60);
        let pieces = split_text(&text);
        assert!(pieces.iter().all(|p| p.chars().count() <= MAX_CHARS));
        let joined: Vec<&str> = pieces.iter().flat_map(|p| p.split(' ')).collect();
        let original: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(joined, original);
    }

    #[test]
    fn short_text_is_one_piece() {
        assert_eq!(split_text("Hello there, world!"), vec!["Hello there, world!"]);
    }
}
