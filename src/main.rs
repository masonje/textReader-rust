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
mod word_timing;

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

/// `--write-icons <dir>`: the app icon at the usual sizes, as <dir>/<n>x<n>/apps/clipboard-reader.png
/// (used by scripts/install.sh, so installing needs no image tools).
fn write_icons(dir: &std::path::Path) -> anyhow::Result<()> {
    let (rgba, w, h) = decode_icon().ok_or_else(|| anyhow::anyhow!("couldn't read the icon"))?;
    for size in [16u32, 32, 48, 64, 128, 256, 512] {
        let out = shrink_icon(&rgba, w, h, size);
        let folder = dir.join(format!("{size}x{size}/apps"));
        std::fs::create_dir_all(&folder)?;
        let file = std::fs::File::create(folder.join("clipboard-reader.png"))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&out)?;
    }
    Ok(())
}

/// Scale an RGBA image down to size x size, averaging every source pixel that falls in each output pixel.
pub fn shrink_icon(rgba: &[u8], w: u32, h: u32, size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        let (y0, y1) = (y * h / size, ((y + 1) * h / size).max(y * h / size + 1));
        for x in 0..size {
            let (x0, x1) = (x * w / size, ((x + 1) * w / size).max(x * w / size + 1));
            let mut sum = [0u64; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * w + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += rgba[i + c] as u64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let o = ((y * size + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    out
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
    if args.first().map(String::as_str) == Some("--write-icons") {
        let Some(dir) = args.get(1) else {
            eprintln!("usage: clipboard-reader --write-icons <icon folder>");
            std::process::exit(2);
        };
        if let Err(e) = write_icons(std::path::Path::new(dir)) {
            eprintln!("❌ {e:#}");
            std::process::exit(1);
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

    let (size, min_size) = ui::start_size(&controller);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Clipboard Reader")
        .with_app_id("clipboard-reader")
        .with_inner_size(size)
        .with_min_inner_size(min_size);
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
