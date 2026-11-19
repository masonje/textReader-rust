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
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
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
    let mut data = rgba.to_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1); // RGBA -> ARGB
    }
    let icon = vec![ksni::Icon { width: width as i32, height: height as i32, data }];
    match (Tray { controller, icon }).spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("⚠️ No tray icon: {e}");
            None
        }
    }
}
