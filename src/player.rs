//! Audio playback through libmpv, loaded at run time.
//!
//! mpv plays the file as built and changes speed while playing, keeping the
//! voice's pitch. Positions and durations are in the file's own time (at 1x).

use anyhow::{Context, Result, anyhow};
use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

type Handle = *mut c_void;

struct Api {
    _lib: Library,
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    set_option_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    get_property_string: unsafe extern "C" fn(Handle, *const c_char) -> *mut c_char,
    free: unsafe extern "C" fn(*mut c_void),
    wait_event: unsafe extern "C" fn(Handle, c_double) -> *mut c_int,
    terminate_destroy: unsafe extern "C" fn(Handle),
}

pub struct Player {
    api: Api,
    handle: Handle,
    loaded: AtomicBool, // a file was started and has not been stopped
    stopping: std::sync::Arc<AtomicBool>,
    events: Option<std::thread::JoinHandle<()>>,
}

// libmpv's client API is thread-safe.
unsafe impl Send for Player {}
unsafe impl Sync for Player {}

fn library_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.2.dylib", "libmpv.dylib", "/opt/homebrew/lib/libmpv.dylib", "/usr/local/lib/libmpv.dylib"]
    } else {
        &["libmpv.so.2", "libmpv.so.1", "libmpv.so"]
    }
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

impl Player {
    pub fn new() -> Result<Self> {
        let lib = library_names()
            .iter()
            .find_map(|name| unsafe { Library::new(*name).ok() })
            .ok_or_else(|| anyhow!("libmpv not found (install libmpv2)"))?;
        let api = unsafe {
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes()).with_context(|| format!("libmpv has no {}", $name))?
                };
            }
            Api {
                create: sym!("mpv_create"),
                initialize: sym!("mpv_initialize"),
                set_option_string: sym!("mpv_set_option_string"),
                command: sym!("mpv_command"),
                set_property_string: sym!("mpv_set_property_string"),
                get_property_string: sym!("mpv_get_property_string"),
                free: sym!("mpv_free"),
                wait_event: sym!("mpv_wait_event"),
                terminate_destroy: sym!("mpv_terminate_destroy"),
                _lib: lib,
            }
        };
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(anyhow!("mpv_create failed"));
        }
        let mut options = vec![
            ("video", "no".to_string()),
            ("audio-display", "no".to_string()),
            ("idle", "yes".to_string()),
            ("keep-open", "no".to_string()),
            ("input-default-bindings", "no".to_string()),
            ("terminal", "no".to_string()),
            ("audio-pitch-correction", "yes".to_string()),
            ("ytdl", "no".to_string()),
        ];
        if let Ok(log) = std::env::var("TR_MPV_LOG") {
            options.push(("log-file", log));
        }
        for (name, value) in &options {
            unsafe { (api.set_option_string)(handle, c(name).as_ptr(), c(value).as_ptr()) };
        }
        if unsafe { (api.initialize)(handle) } < 0 {
            unsafe { (api.terminate_destroy)(handle) };
            return Err(anyhow!("mpv_initialize failed"));
        }
        // Drain mpv's event queue so it never fills up (stopping before mpv is destroyed).
        let stopping = std::sync::Arc::new(AtomicBool::new(false));
        let (wait, handle_addr, stop) = (api.wait_event, handle as usize, stopping.clone());
        let events = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let event = unsafe { wait(handle_addr as Handle, 0.2) };
                if !event.is_null() && unsafe { *event } == 1 {
                    break; // MPV_EVENT_SHUTDOWN
                }
            }
        });
        Ok(Player { api, handle, loaded: AtomicBool::new(false), stopping, events: Some(events) })
    }

    fn get(&self, name: &str) -> Option<String> {
        let ptr = unsafe { (self.api.get_property_string)(self.handle, c(name).as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
        unsafe { (self.api.free)(ptr as *mut c_void) };
        Some(value)
    }

    fn set(&self, name: &str, value: &str) {
        unsafe { (self.api.set_property_string)(self.handle, c(name).as_ptr(), c(value).as_ptr()) };
    }

    /// Start playing `path` and wait (briefly) until mpv has really started it.
    pub fn play(&self, path: &std::path::Path, speed: f64) -> bool {
        self.set("speed", &format!("{speed:.2}"));
        self.set("pause", "no");
        let path = c(&path.to_string_lossy());
        let cmd = c("loadfile");
        let mode = c("replace");
        let args = [cmd.as_ptr(), path.as_ptr(), mode.as_ptr(), std::ptr::null()];
        unsafe { (self.api.command)(self.handle, args.as_ptr()) };
        self.loaded.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.get("time-pos").is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    pub fn pause(&self) {
        self.set("pause", "yes");
    }

    pub fn resume(&self) {
        self.set("pause", "no");
    }

    pub fn stop(&self) {
        self.loaded.store(false, Ordering::SeqCst);
        self.set("pause", "no");
        let cmd = c("stop");
        let args = [cmd.as_ptr(), std::ptr::null()];
        unsafe { (self.api.command)(self.handle, args.as_ptr()) };
    }

    /// True while a file is playing or paused.
    pub fn busy(&self) -> bool {
        self.loaded.load(Ordering::SeqCst) && self.get("idle-active").as_deref() == Some("no")
    }

    pub fn paused(&self) -> bool {
        self.busy() && self.get("pause").as_deref() == Some("yes")
    }

    pub fn position(&self) -> Option<f64> {
        if !self.busy() {
            return None;
        }
        self.get("time-pos")?.parse().ok()
    }

    pub fn duration(&self) -> Option<f64> {
        if !self.busy() {
            return None;
        }
        self.get("duration")?.parse().ok()
    }

    pub fn set_speed(&self, speed: f64) {
        self.set("speed", &format!("{speed:.2}"));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(events) = self.events.take() {
            let _ = events.join();
        }
        unsafe { (self.api.terminate_destroy)(self.handle) };
    }
}
