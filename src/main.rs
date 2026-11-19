//! Clipboard Reader (Rust version): reads selected text aloud.

mod clipboard;
mod controller;
mod engines;
mod hotkeys;
mod options;
mod player;
mod pronunciation;
mod settings;
mod tray;
mod ui;

use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const ICON: &[u8] = include_bytes!("../assets/pireader.png");

fn decode_icon() -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    Some((buf, info.width, info.height))
}

/// `--check-engines`: build a short phrase with every engine and report.
fn check_engines() -> i32 {
    let mut failures = 0;
    let build = engines::Build { cancel: Arc::new(AtomicBool::new(false)), progress: Box::new(|_| {}) };
    let dir = std::env::temp_dir();
    for engine in engines::all() {
        let name = engine.name();
        if let Some(missing) = engine.missing() {
            println!("{name:<10} MISSING: {missing}");
            failures += 1;
            continue;
        }
        let voices = engine.voices();
        // Prefer a voice that is already downloaded
        let voice = voices.iter().find(|(_, label)| !label.ends_with("· download")).or(voices.first()).cloned().unwrap_or_default();
        let out = dir.join(format!("clipboard-reader-check.{}", engine.ext()));
        let start = std::time::Instant::now();
        match engine.synthesize("This is a test of the speech engine.", &voice.0, &out, &build) {
            Ok(()) => {
                let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                println!(
                    "{name:<10} OK: {} voices, tested {} ({:.1}s, {} KB)",
                    voices.len(),
                    voice.0,
                    start.elapsed().as_secs_f64(),
                    size / 1024
                );
            }
            Err(e) => {
                println!("{name:<10} FAILED: {e:#}");
                failures += 1;
            }
        }
        let _ = std::fs::remove_file(&out);
    }
    if failures > 0 { 1 } else { 0 }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("Clipboard Reader {} (Rust)", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.first().map(String::as_str) == Some("--phonemes") {
        // Debugging: show the phonemes Piper is given for some text
        match engines::espeak::phonemize("en-us", &args[1..].join(" ")) {
            Ok(sentences) => sentences.iter().for_each(|s| println!("{}", s.iter().collect::<String>())),
            Err(e) => eprintln!("{e:#}"),
        }
        return;
    }
    if args.iter().any(|a| a == "--check-engines") {
        std::process::exit(check_engines());
    }

    let player = match player::Player::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("❌ {e:#}");
            std::process::exit(1);
        }
    };
    let controller = controller::Controller::new(player);
    println!("==================================================");
    println!("Clipboard Reader {} (Rust) started", env!("CARGO_PKG_VERSION"));
    println!("==================================================");
    println!("🎮 Keyboard Controls:");
    let hotkeys = hotkeys::Hotkeys::start(controller.clone());

    let icon = decode_icon();
    let _tray = icon.as_ref().and_then(|(rgba, w, h)| tray::start(controller.clone(), rgba, *w, *h));

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Clipboard Reader")
        .with_app_id("clipboard-reader")
        .with_inner_size([340.0, 340.0])
        .with_min_inner_size([300.0, 320.0]);
    if let Some((rgba, width, height)) = icon {
        viewport = viewport.with_icon(egui::IconData { rgba, width, height });
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let app_controller = controller.clone();
    let result = eframe::run_native(
        "Clipboard Reader",
        options,
        Box::new(move |cc| Ok(Box::new(ui::ReaderApp::new(app_controller, hotkeys, &cc.egui_ctx)))),
    );
    controller.shutdown();
    if let Err(e) = result {
        eprintln!("❌ The window couldn't open: {e}");
        std::process::exit(1);
    }
}
