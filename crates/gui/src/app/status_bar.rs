//! Status bar and window title.

use crate::*;

impl GuiApp {
    /// The always-visible status line: the outcome of the last action on
    /// the left, and which configuration file this window is editing on the
    /// right, so a second instance opened on a different file is never
    /// mistaken for the first.
    pub(crate) fn render_status_bar(&mut self, root: &mut egui::Ui, palette: &Palette) {
        let (text_color, accent) = self.status.colors(palette);
        egui::Panel::bottom("status")
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(self.status.background(palette))
                    .inner_margin(egui::Margin::symmetric(18, 8))
                    .stroke(egui::Stroke::NONE),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let (bar, _) = ui.allocate_exact_size(
                        egui::Vec2::new(3.0, ui.text_style_height(&egui::TextStyle::Body)),
                        egui::Sense::hover(),
                    );
                    if let Some(accent) = accent {
                        ui.painter()
                            .rect_filled(bar, egui::CornerRadius::same(2), accent);
                    }
                    ui.label(RichText::new(self.status.text()).color(text_color));
                    if self.diagnostics_running || self.pending_control > 0 {
                        ui.spinner();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let full_path = self.path.display().to_string();
                        ui.add(
                            egui::Label::new(
                                RichText::new(home_relative_path(
                                    &self.path,
                                    env::var_os("HOME").as_deref(),
                                ))
                                .monospace()
                                .small()
                                .color(palette.muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(format!(
                            "{}\n{full_path}",
                            self.strings.configuration_file()
                        ));
                    });
                });
            });
    }

    /// Keeps the window title in step with which file is open and whether it
    /// has unsaved edits, the way any other editor does, instead of showing
    /// a constant application name.
    pub(crate) fn sync_window_title(&mut self, ctx: &egui::Context) {
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        let title = if self.draft_is_dirty() {
            format!("• {name} — WayExpand")
        } else {
            format!("{name} — WayExpand")
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}
