//! Getting the text to read: the current selection, or else the clipboard.
//!
//! The program that owns the selection has to hand the text over. If it's busy
//! or stuck that can take forever, so every read has a time limit.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub enum Clip {
    Text(String),
    Empty,
    NoAnswer,
}

fn xclip(selection: &str, timeout: Duration) -> Clip {
    let child = Command::new("xclip")
        .args(["-selection", selection, "-o"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return Clip::Empty };
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().unwrap_or_default();
                let text = String::from_utf8_lossy(&bytes).to_string();
                return if status.success() && !text.trim().is_empty() { Clip::Text(text) } else { Clip::Empty };
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Clip::NoAnswer;
            }
        }
    }
}

/// The selected text if there is any, otherwise the clipboard.
pub fn text_to_read() -> Clip {
    if let Clip::Text(text) = xclip("primary", Duration::from_millis(500)) {
        return Clip::Text(text);
    }
    xclip("clipboard", Duration::from_secs(2))
}
