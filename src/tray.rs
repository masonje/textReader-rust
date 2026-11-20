//! System tray icon (StatusNotifierItem over D-Bus; no GTK needed).

use crate::controller::Controller;
use std::sync::Arc;

pub struct Tray {
    controller: Arc<Controller>,
    icon: Vec<ksni::Icon>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "clipboard-reader".into()
    }
    fn title(&self) -> String {
        "Clipboard Reader".into()
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.controller.request_show();
    }
    /// Middle-click. Ayatana's indicator applet (Ubuntu MATE's top panel) opens the menu on a left
    /// click and never sends Activate, but it does pass on a middle click.
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.controller.request_show();
    }
    fn icon_name(&self) -> String {
        "clipboard-reader".into() // installed by scripts/install.sh; the pixmaps below are used when it isn't
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show Window".into(),
                activate: Box::new(|tray: &mut Self| tray.controller.request_show()),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Read Clipboard".into(),
                activate: Box::new(|tray: &mut Self| tray.controller.read_selection()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop".into(),
                activate: Box::new(|tray: &mut Self| tray.controller.stop()),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.controller.shutdown();
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Start the tray icon. Returns None (with a message) where there is no tray.
pub fn start(controller: Arc<Controller>, rgba: &[u8], width: u32, height: u32) -> Option<ksni::blocking::Handle<Tray>> {
    use ksni::blocking::TrayMethods;
    // Small sizes only: tray hosts (MATE's among them) show a broken icon if sent the full-size picture
    let icon = [16u32, 22, 24, 32, 48, 64]
        .into_iter()
        .map(|size| {
            let mut data = crate::shrink_icon(rgba, width, height, size);
            for pixel in data.chunks_exact_mut(4) {
                pixel.rotate_right(1); // RGBA -> ARGB
            }
            ksni::Icon { width: size as i32, height: size as i32, data }
        })
        .collect();
    let controller_ok = controller.clone();
    let controller_ok = &controller_ok.tray_ok;
    match (Tray { controller, icon }).spawn() {
        Ok(handle) => {
            controller_ok.store(true, std::sync::atomic::Ordering::SeqCst);
            Some(handle)
        }
        Err(e) => {
            eprintln!("⚠️ No tray icon: {e}");
            None
        }
    }
}

