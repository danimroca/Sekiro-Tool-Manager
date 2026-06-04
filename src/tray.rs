use std::sync::mpsc;

use ksni::blocking::TrayMethods;

/// Messages sent from the tray thread to the iced event loop.
#[derive(Debug, Clone)]
pub enum TrayMessage {
    Show,
    LaunchGame,
    LaunchAll,
    Quit,
}

/// Spawns the tray icon via ksni (StatusNotifierItem / D-Bus).
/// Returns the receiving end of a channel — the caller should poll this
/// to convert tray events into iced `Message`s.
pub fn spawn() -> mpsc::Receiver<TrayMessage> {
    let (tx, rx) = mpsc::channel();

    let icon = load_icon();

    let tray = TrayData {
        tx,
        icon,
    };

    // The blocking spawn() runs the D-Bus event loop on a background thread
    // and returns immediately. We leak the handle so the tray stays alive
    // for the entire process lifetime (Quit calls std::process::exit(0)).
    match tray.spawn() {
        Ok(handle) => {
            std::mem::forget(handle);
        }
        Err(e) => {
            eprintln!("tray: failed to spawn: {e}");
        }
    }

    rx
}

/// Load the 22×22 tray icon from embedded PNG data and convert to
/// ksni::Icon (ARGB32, network byte order).
fn load_icon() -> ksni::Icon {
    let img = image::load_from_memory(include_bytes!("../assets/tray-icon.png"))
        .expect("tray: failed to load embedded icon")
        .into_rgba8();
    let (width, height) = img.dimensions();
    let mut data = img.into_raw();
    // ksni expects ARGB32 (network byte order); image crate gives us RGBA
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }
    ksni::Icon {
        width: width as i32,
        height: height as i32,
        data,
    }
}

struct TrayData {
    tx: mpsc::Sender<TrayMessage>,
    icon: ksni::Icon,
}

impl ksni::Tray for TrayData {
    fn id(&self) -> String {
        "sekiro-launcher".into()
    }

    fn title(&self) -> String {
        "Sekiro Launcher".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.icon.clone()]
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        use ksni::MenuItem;

        vec![
            StandardItem {
                label: "Show Launcher".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayMessage::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Launch Game".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayMessage::LaunchGame);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Launch All".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayMessage::LaunchAll);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
