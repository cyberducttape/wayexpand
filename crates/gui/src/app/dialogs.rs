//! Pending-action confirmation (unsaved changes) and dialog stacking.

use crate::*;

impl GuiApp {
    pub(crate) fn maybe_execute_pending_action(&mut self) {
        if self.pending_action.is_some() && !self.draft_is_dirty() {
            if let Some(action) = self.pending_action.take() {
                self.execute_action(action);
            }
        }
    }

    pub(crate) fn request_action(&mut self, action: PendingAction) {
        if matches!(&action, PendingAction::Select(index) if self.selected_index() == Some(*index))
        {
            return;
        }
        self.cancel_app_detection();
        if matches!(&action, PendingAction::Delete) && !self.draft_is_dirty() {
            self.pending_action = Some(action);
            return;
        }
        if self.draft_is_dirty() {
            self.pending_action = Some(action);
        } else {
            self.execute_action(action);
        }
    }

    pub(crate) fn execute_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Select(index) => self.select(index),
            PendingAction::New => self.create_new_snippet(),
            PendingAction::Duplicate => self.duplicate_selected(),
            PendingAction::Delete => self.perform_delete_selected(),
            PendingAction::Reload => self.perform_reload(),
            PendingAction::Undo => self.undo(),
            PendingAction::Close => self.close_after_confirm = true,
        }
    }

    pub(crate) fn discard_pending(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        if self.new_draft {
            self.abandon_new_draft();
        }
        self.execute_action(action);
    }

    pub(crate) fn abandon_new_draft(&mut self) {
        if !self.new_draft {
            return;
        }
        let origin_id = self.new_draft_origin.take();
        let origin = origin_id
            .as_ref()
            .and_then(|id| self.config.expansion.iter().position(|item| &item.id == id))
            .or_else(|| (!self.config.expansion.is_empty()).then_some(0));
        self.new_draft = false;
        self.set_selected_index(origin);
        self.draft = origin.map(|index| Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = origin
            .map(|index| self.config.expansion[index].trigger.clone())
            .unwrap_or_default();
        self.clear_command_preview();
    }

    pub(crate) fn save_and_execute_pending(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        self.save_selected();
        if !self.draft_is_dirty() {
            self.execute_action(action);
        } else {
            self.pending_action = Some(action);
        }
    }

    pub(crate) fn any_dialog_open(&self) -> bool {
        self.diagnostics_open || self.import_open || self.settings_open || self.evdev_setup_open
    }

    /// Escape closes one dialog at a time, most recently opened first. The
    /// import dialog keeps its preview open on the first Escape so a loaded
    /// library is not discarded by a keystroke meant to dismiss something
    /// else.
    pub(crate) fn close_topmost_dialog(&mut self) {
        // The save/discard/delete prompt is modal and sits above everything
        // else, so Escape answers it (as Cancel) before closing any window.
        if self.pending_action.is_some() {
            self.pending_action = None;
        } else if self.evdev_setup_open {
            self.evdev_setup_open = false;
        } else if self.settings_open {
            self.settings_open = false;
        } else if self.import_open {
            if self.import_preview.is_some() {
                self.import_preview = None;
            } else {
                self.import_open = false;
            }
        } else if self.diagnostics_open {
            self.diagnostics_open = false;
        }
    }

    pub(crate) fn render_pending_action(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.pending_action.is_some() {
            // A real modal: the rest of the window is dimmed and ignores
            // clicks. As a floating window, clicking another snippet while
            // this prompt was open silently replaced the pending action.
            let dirty = self.draft_is_dirty();
            let response = egui::Modal::new(egui::Id::new("pending_action_modal"))
                .frame(
                    egui::Frame::popup(&ctx.global_style())
                        .fill(palette.surface)
                        .inner_margin(egui::Margin::same(20)),
                )
                .show(ctx, |ui| {
                    ui.set_max_width(460.0);
                    ui.heading(if dirty {
                        self.strings.unsaved_title()
                    } else {
                        self.strings.delete_button()
                    });
                    ui.add_space(4.0);
                    let action = match self.pending_action {
                        Some(PendingAction::Select(_)) => self.strings.unsaved_switching(),
                        Some(PendingAction::New) => self.strings.unsaved_creating(),
                        Some(PendingAction::Duplicate) => self.strings.unsaved_duplicating(),
                        Some(PendingAction::Delete) => self.strings.unsaved_deleting(),
                        Some(PendingAction::Reload) => self.strings.unsaved_reloading(),
                        Some(PendingAction::Undo) => self.strings.unsaved_undoing(),
                        Some(PendingAction::Close) => self.strings.unsaved_closing(),
                        None => "continuing",
                    };
                    if dirty {
                        ui.label(self.strings.save_before(action));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, palette, self.strings.save_continue())
                                .clicked()
                            {
                                self.save_and_execute_pending();
                            }
                            if theme::secondary_button(ui, palette, self.strings.discard())
                                .clicked()
                            {
                                self.discard_pending();
                            }
                            if theme::secondary_button(ui, palette, self.strings.cancel()).clicked()
                            {
                                self.pending_action = None;
                            }
                        });
                    } else if matches!(self.pending_action, Some(PendingAction::Delete)) {
                        ui.label(self.strings.delete_confirm());
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::danger_button(ui, palette, self.strings.delete_button())
                                .clicked()
                            {
                                self.pending_action = None;
                                self.execute_action(PendingAction::Delete);
                            }
                            if theme::secondary_button(ui, palette, self.strings.cancel()).clicked()
                            {
                                self.pending_action = None;
                            }
                        });
                    }
                });
            // Backdrop click or Escape means Cancel.
            if response.should_close() {
                self.pending_action = None;
            }
        }
    }
}
