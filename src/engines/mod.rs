//! Speech engines. Each one writes the audio for some text to a file.

use anyhow::{Result, anyhow};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub mod edge;
mod edge_drm;
pub mod espeak;
pub mod gtts;
pub mod piper;

/// (voice id, label shown in the Voice list)
pub type Voice = (String, String);

/// Passed to an engine while it builds audio.
pub struct Build {
    pub cancel: Arc<AtomicBool>,
    /// Progress text for the status line (e.g. a voice download).
    pub progress: Box<dyn Fn(String) + Send + Sync>,
}

impl Build {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled() { Err(anyhow!(Cancelled)) } else { Ok(()) }
    }
}

#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cancelled")
    }
}
impl std::error::Error for Cancelled {}

pub fn is_cancelled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Cancelled>().is_some()
}

pub trait Engine: Send + Sync {
    /// Same names as the Python app, so settings.json is shared.
    fn name(&self) -> &'static str;
    /// "wav" or "mp3": whatever the engine produces natively.
    fn ext(&self) -> &'static str;
    /// Why the engine can't run, if it can't.
    fn missing(&self) -> Option<String>;
    fn voices(&self) -> Vec<Voice>;
    fn synthesize(&self, text: &str, voice: &str, out: &Path, build: &Build) -> Result<()>;
}

pub fn all() -> Vec<Arc<dyn Engine>> {
    vec![
        Arc::new(piper::Piper::default()),
        Arc::new(edge::Edge::default()),
        Arc::new(gtts::Gtts),
        Arc::new(espeak::EspeakEngine::default()),
    ]
}

/// Run a command (optionally feeding it `input`), killing it on cancel or timeout.
pub fn run_cancellable(cmd: &mut Command, input: Option<&str>, build: &Build, timeout: Duration) -> Result<()> {
    cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd.spawn()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() { Ok(()) } else { Err(anyhow!("{:?} failed ({status})", cmd.get_program())) };
        }
        if build.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(Cancelled));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("{:?} timed out", cmd.get_program()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}
