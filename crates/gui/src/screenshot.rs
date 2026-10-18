//! Debug-build screenshot capture for checking the GUI's look.
//!
//! With `WAYEXPAND_GUI_SCREENSHOT=/path/shot.ppm`, a debug build renders a
//! few frames, saves the window as a binary PPM, and exits. Release builds
//! ignore the variable. `scripts/gui-screenshot.sh` runs this inside a
//! private headless compositor, so it never touches the user's desktop.

use std::path::PathBuf;

use eframe::egui;

/// Frames to let layout, fonts and the theme settle before capturing.
const SETTLE_FRAMES: u32 = 20;

pub(crate) struct ScreenshotCapture {
    path: PathBuf,
    frames: u32,
    requested: bool,
    /// `WAYEXPAND_GUI_SCREENSHOT_VIEW`: a screen to open before capturing
    /// (`light`, `settings`, `diagnostics`, `new`, `import`).
    view: Option<String>,
}

impl ScreenshotCapture {
    pub(crate) fn from_env() -> Option<Self> {
        if !cfg!(debug_assertions) {
            return None;
        }
        let path = std::env::var_os("WAYEXPAND_GUI_SCREENSHOT")?;
        Some(Self {
            path: PathBuf::from(path),
            frames: 0,
            requested: false,
            view: std::env::var("WAYEXPAND_GUI_SCREENSHOT_VIEW")
                .ok()
                .filter(|view| !view.is_empty()),
        })
    }

    /// The screen to open, once, on the first frame.
    pub(crate) fn take_view(&mut self) -> Option<String> {
        self.view.take()
    }

    /// Call once per frame.
    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        ctx.request_repaint();
        if !self.requested && self.frames >= SETTLE_FRAMES {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.requested = true;
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [width, height] = image.size;
            let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
            for pixel in &image.pixels {
                ppm.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b()]);
            }
            if let Err(error) = std::fs::write(&self.path, ppm) {
                eprintln!("could not write {}: {error}", self.path.display());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
