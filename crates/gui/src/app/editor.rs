//! Snippet editor: the first-run screen, the edit form, Try Live, editor actions, previews, and app detection.

use super::setup::onboarding_step;
use crate::*;

impl GuiApp {
    pub(crate) fn cancel_app_detection(&mut self) {
        if let Some(task) = self.app_detection.as_mut() {
            task.cancelled.store(true, Ordering::Release);
        }
    }

    /// A cancelled worker may finish after the editor has disappeared (for
    /// example after deleting the last snippet). Reap its result here so the
    /// task does not pin the receiver or block a future detection forever.
    pub(crate) fn reap_app_detection_without_editor(&mut self, ctx: &egui::Context) {
        if self.selected_index().is_some() {
            return;
        }
        let Some(task) = self.app_detection.as_ref() else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(_) | Err(mpsc::TryRecvError::Disconnected) => self.app_detection = None,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }

    /// Whether the editor holds edits that are not yet in the configuration.
    ///
    /// Called several times per frame (toolbar badge, action bar, window
    /// title), so the comma-separated fields are compared in place instead of
    /// rebuilding a joined `String` -- and a `Vec` to join from -- on each
    /// call. The cheap scalar comparisons are ordered first so a draft that
    /// differs at all usually answers before touching a list at all.
    pub(crate) fn draft_is_dirty(&self) -> bool {
        if self.new_draft {
            return self.draft.is_some();
        }
        let (Some(index), Some(draft)) = (self.selected_index(), self.draft.as_ref()) else {
            return false;
        };
        let expansion = &self.config.expansion[index];
        draft.trigger != expansion.trigger
            || draft.committed_aliases() != expansion.aliases
            || draft.description != expansion.description
            || draft.category != expansion.category
            || draft.replacement != expansion.replacement
            || draft.committed_tags() != expansion.tags
            || draft.committed_app_filter() != expansion.app_filter
            || draft.enabled != expansion.enabled
            || draft.match_mode != expansion.match_mode
            || draft.propagate_case != expansion.propagate_case
            || !draft.matches_command(expansion.command.as_ref())
    }

    /// Renders a live preview for a plain (non-command) draft. Must never be
    /// called for a command-backed draft: the engine could spawn the
    /// configured program continuously while the editor is simply open --
    /// including any side-effecting script the user has not even saved yet.
    /// Command previews are explicit and user-triggered instead; see
    /// `run_command_preview`.
    pub(crate) fn preview(&mut self) -> String {
        let source = if self.new_draft {
            if self.draft.is_none() {
                return self.strings.no_selection().into();
            }
            None
        } else {
            let Some(index) = self.selected_index() else {
                return self.strings.no_selection().into();
            };
            Some(&self.config.expansion[index])
        };
        if let Some((cached_revision, cached_input, cached_result)) = &self.preview_cache {
            if *cached_revision == self.preview_revision && cached_input == &self.preview_input {
                return cached_result.clone();
            }
        }
        let result = preview::render_with_library(
            source,
            &self.config.settings,
            &self.config.organization,
            self.draft.as_ref(),
            &self.preview_input,
            &self.preview_app,
            Some(&self.config),
        );
        self.preview_cache = Some((
            self.preview_revision,
            self.preview_input.clone(),
            result.clone(),
        ));
        result
    }

    pub(crate) fn invalidate_preview(&mut self) {
        self.preview_revision = self.preview_revision.wrapping_add(1);
        self.preview_cache = None;
    }

    /// Runs the draft's configured command exactly once, on explicit user
    /// request (a button click), and caches the result for display. This is
    /// the only place a command-backed draft's program should ever run
    /// before it is saved.
    pub(crate) fn run_command_preview(&mut self) {
        self.clear_command_preview();
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        self.command_preview_key = Some(self.preview_revision);
        let command = match draft.command_config() {
            Ok(Some(command)) => command,
            Ok(None) => {
                self.command_preview_result =
                    Some(Err(self.strings.status_enable_command_first().into()));
                return;
            }
            Err(error) => {
                self.command_preview_result =
                    Some(Err(self.strings.status_command_invalid(&error.to_string())));
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancelled);
        self.command_preview_result = None;
        self.command_preview_receiver = Some(receiver);
        self.command_preview_cancel = Some(cancelled);
        // `Strings` is a plain language tag, so the worker can phrase its own
        // failure in the user's language without borrowing the app.
        let strings = Strings::new(self.language);
        thread::spawn(move || {
            let result = wayexpand_core::run_command_cancellable(&command, &worker_cancel)
                .map_err(|error| strings.status_command_failed(&error.to_string()));
            let _ = sender.send(result);
        });
    }

    pub(crate) fn poll_command_preview(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.command_preview_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.command_preview_receiver = None;
                self.command_preview_cancel = None;
                self.command_preview_result = Some(result);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.command_preview_receiver = None;
                self.command_preview_cancel = None;
                self.command_preview_result =
                    Some(Err(self.strings.status_command_preview_failed().into()));
            }
        }
    }

    pub(crate) fn clear_command_preview(&mut self) {
        if let Some(cancel) = self.command_preview_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.command_preview_result = None;
        self.command_preview_key = None;
        self.command_preview_receiver = None;
    }

    pub(crate) fn render_editor(&mut self, root: &mut egui::Ui, palette: &Palette) {
        // egui sends text/key/paste/click events only on frames where user
        // input can mutate an editor control. Advancing this scalar revision
        // avoids hashing the full draft (which may contain megabytes of text)
        // on every otherwise-idle repaint. Pointer movement is intentionally
        // excluded so moving the mouse does not invalidate the preview.
        let editor_input = root.ctx().input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                        | egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            })
        });
        if editor_input {
            self.invalidate_preview();
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.background)
                    .inner_margin(egui::Margin::symmetric(22, 18)),
            )
            .show(root, |ui| {
                if self.selected_index().is_none()
                    && !self.new_draft
                    && !self.config.expansion.is_empty()
                {
                    ui.centered_and_justified(|ui| {
                        ui.label(self.strings.select_snippet_prompt());
                    });
                    return;
                }
                let Some(index) = self
                    .selected_index()
                    .or_else(|| self.new_draft.then_some(self.config.expansion.len()))
                else {
                    self.render_first_run(ui, palette);
                    return;
                };
                if index >= self.config.expansion.len() && !self.new_draft {
                    self.set_selected_index(None);
                    self.draft = None;
                    ui.label(self.strings.selection_stale());
                    return;
                }
                if self.draft.is_none() {
                    self.draft = self.config.expansion.get(index).map(Draft::from_expansion);
                }
                let command_backed = self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.command_enabled);
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt("editor_scroll")
                    .show(ui, |ui| {
                        self.render_edit_form(ui, palette, index, command_backed)
                    });
            });
    }

    /// The snippet edit form for the draft at `index` (an index equal to the
    /// library length is a new, unsaved snippet).
    fn render_edit_form(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        index: usize,
        command_backed: bool,
    ) {
        let mut detect_app_clicked = false;
        egui::Frame::new()
            .fill(palette.surface)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                theme::section_header(ui, "", self.strings.snippet_details());
                ui.add_space(6.0);
                let categories = self.categories().to_vec();
                let duplicate_trigger =
                    self.draft.as_ref().is_some_and(|draft| {
                        !draft.trigger.is_empty()
                            && self.config.expansion.iter().enumerate().any(
                                |(other_index, other)| {
                                    other_index != index && other.answers_to(&draft.trigger)
                                },
                            )
                    });
                let strings = &self.strings;
                let app_detecting = self
                    .app_detection
                    .as_ref()
                    .is_some_and(|task| !task.cancelled.load(Ordering::Acquire));
                let app_detection_busy = self.app_detection.is_some();
                let mut cancel_detection = false;
                let Some(draft) = self.draft.as_mut() else {
                    ui.label(strings.draft_unavailable());
                    return;
                };
                // Two aligned columns so every field starts at
                // the same x whatever the language's label
                // lengths. Rows are laid out in one pass by
                // `form_row` rather than by `egui::Grid`, which
                // sizes rows from the previous frame: wrapped
                // chips and hints overlapped the next row, and
                // one long hint pushed the card past the window.
                let body_font = egui::TextStyle::Body.resolve(ui.style());
                let label_width = [
                    strings.trigger(),
                    strings.aliases(),
                    strings.description(),
                    strings.tags(),
                    strings.category(),
                    strings.app_filter(),
                    strings.matching(),
                ]
                .iter()
                .map(|label| {
                    ui.painter()
                        .layout_no_wrap(
                            (*label).to_owned(),
                            body_font.clone(),
                            Color32::TRANSPARENT,
                        )
                        .size()
                        .x
                })
                .fold(0.0, f32::max);
                let columns = FormColumns::new(ui, label_width);
                let field_width = columns.field_width;
                let hint = |ui: &mut egui::Ui, text: RichText| {
                    ui.add(egui::Label::new(text).wrap());
                };

                form_row(ui, strings.trigger(), columns, |ui| {
                    ui.add(
                        TextEdit::singleline(&mut draft.trigger)
                            .margin(theme::FIELD_MARGIN)
                            .hint_text(strings.trigger_hint())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(field_width),
                    );
                    if duplicate_trigger {
                        hint(
                            ui,
                            RichText::new(strings.duplicate_trigger()).color(palette.danger),
                        );
                    } else {
                        hint(
                            ui,
                            RichText::new(strings.trigger_tip())
                                .small()
                                .color(palette.muted),
                        );
                    }
                });

                form_row(ui, strings.aliases(), columns, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        theme::token_editor(
                            ui,
                            palette,
                            &mut draft.aliases,
                            &mut draft.pending_alias,
                            theme::TokenEditorText {
                                add_hint: strings.add_alias(),
                                remove_hint: strings.remove_alias(),
                            },
                            160.0,
                        );
                    });
                });

                form_row(ui, strings.description(), columns, |ui| {
                    ui.add(
                        TextEdit::singleline(&mut draft.description)
                            .margin(theme::FIELD_MARGIN)
                            .hint_text(strings.description_hint())
                            .desired_width(field_width),
                    );
                });

                form_row(ui, strings.tags(), columns, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        theme::token_editor(
                            ui,
                            palette,
                            &mut draft.tags,
                            &mut draft.pending_tag,
                            theme::TokenEditorText {
                                add_hint: strings.add_tag(),
                                remove_hint: strings.remove_tag(),
                            },
                            160.0,
                        );
                    });
                });

                form_row(ui, strings.category(), columns, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let picker_width = if categories.is_empty() { 0.0 } else { 130.0 };
                        ui.add(
                            TextEdit::singleline(&mut draft.category)
                                .margin(theme::FIELD_MARGIN)
                                .hint_text(strings.category_hint())
                                .desired_width(
                                    (field_width - picker_width - ui.spacing().item_spacing.x)
                                        .max(120.0),
                                ),
                        );
                        if !categories.is_empty() {
                            egui::ComboBox::from_id_salt("category_picker")
                                .selected_text(strings.existing())
                                .width(picker_width - 20.0)
                                .show_ui(ui, |ui| {
                                    for category in categories {
                                        if ui
                                            .selectable_label(draft.category == category, &category)
                                            .clicked()
                                        {
                                            draft.category = category.clone();
                                        }
                                    }
                                });
                        }
                    });
                });

                form_row(ui, strings.app_filter(), columns, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        theme::token_editor(
                            ui,
                            palette,
                            &mut draft.app_filter,
                            &mut draft.pending_app,
                            theme::TokenEditorText {
                                add_hint: strings.add_app(),
                                remove_hint: strings.remove_app(),
                            },
                            180.0,
                        );
                        if app_detecting {
                            ui.spinner();
                            ui.label(strings.detecting_app())
                                .on_hover_text(strings.detect_app_tooltip());
                            if ui.small_button(strings.cancel()).clicked() {
                                // The spawned thread is not joined/cancelled --
                                // it may itself be stuck in a hung D-Bus call --
                                // just stop waiting on it and discard whatever
                                // it eventually sends.
                                cancel_detection = true;
                            }
                        } else if app_detection_busy {
                            ui.spinner();
                            ui.label(strings.stopping_app_detection())
                                .on_hover_text(strings.detect_app_tooltip());
                        } else if ui
                            .button(strings.detect_app())
                            .on_hover_text(strings.detect_app_tooltip())
                            .clicked()
                        {
                            detect_app_clicked = true;
                        }
                    });
                    hint(
                        ui,
                        RichText::new(if draft.app_filter.is_empty() {
                            strings.app_filter_help()
                        } else {
                            strings.window_tracking_warning()
                        })
                        .small()
                        .color(palette.muted),
                    );
                    ui.collapsing(strings.app_filter_advanced(), |ui| {
                        ui.label(
                            RichText::new(strings.app_filter_advanced_help())
                                .small()
                                .color(palette.muted),
                        );
                    });
                });

                form_row(
                    ui,
                    strings.matching(),
                    columns.with_label_offset(0.0),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut draft.enabled, strings.enabled());
                            ui.separator();
                            ui.radio_value(
                                &mut draft.match_mode,
                                MatchMode::Immediate,
                                strings.immediate(),
                            );
                            ui.radio_value(
                                &mut draft.match_mode,
                                MatchMode::WordBoundary,
                                strings.word_boundary(),
                            );
                            ui.separator();
                            ui.checkbox(&mut draft.propagate_case, strings.propagate_case())
                                .on_hover_text(strings.propagate_case_tooltip());
                        });
                    },
                );
                if cancel_detection {
                    if let Some(task) = self.app_detection.as_mut() {
                        task.cancelled.store(true, Ordering::Release);
                    }
                }
                let Some(draft) = self.draft.as_mut() else {
                    return;
                };
                ui.add_space(8.0);
                ui.label(self.strings.replacement());
                if command_backed {
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.strings.command_backed_help())
                                .small()
                                .color(palette.warning),
                        )
                        .wrap(),
                    );
                }
                if let Some(capabilities) = self.daemon_capabilities {
                    if let Some(limit) = capabilities.injection_max_text_chars {
                        let count = draft.replacement.chars().count();
                        if limit > 0 && count > limit {
                            let mode = capabilities.injection_mode.unwrap_or("active injection");
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        self.strings.insertion_limit_warning(count, limit, mode),
                                    )
                                    .small()
                                    .color(palette.warning),
                                )
                                .wrap(),
                            );
                        }
                    }
                }
                ui.add(
                    TextEdit::multiline(&mut draft.replacement)
                        .margin(theme::FIELD_MARGIN)
                        .id(egui::Id::new(REPLACEMENT_EDITOR_SALT))
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(9)
                        .desired_width(f32::INFINITY),
                );
            });
        self.handle_app_detection(ui, detect_app_clicked);
        ui.add_space(10.0);
        ui.collapsing(self.strings.template_variables(), |ui| {
            ui.label(
                RichText::new(self.strings.template_help())
                    .small()
                    .color(palette.muted),
            );
            ui.horizontal_wrapped(|ui| {
                for variable in TEMPLATE_VARIABLES {
                    if theme::secondary_button(ui, palette, variable)
                        .on_hover_text(self.strings.template_variable_description(variable))
                        .clicked()
                    {
                        if let Some(draft) = self.draft.as_mut() {
                            insert_into_replacement_editor(
                                ui.ctx(),
                                &mut draft.replacement,
                                variable,
                            );
                        }
                        self.invalidate_preview();
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(self.strings.preview_app());
            ui.add(
                TextEdit::singleline(&mut self.preview_app)
                    .margin(theme::FIELD_MARGIN)
                    .hint_text(self.strings.preview_app_hint())
                    .desired_width(300.0),
            );
        });
        ui.add_space(4.0);
        self.render_command_editor(ui, palette);
        // Save and Delete are a pinned action bar under this
        // scroll area (`render_editor_actions`): with a long
        // replacement open they used to scroll out of reach,
        // so the primary action of the window depended on
        // where the user happened to be scrolled to.
        self.render_preview_section(ui, palette);
        // The status line is a persistent bottom panel
        // (`render_status_bar`) rather than the tail of this
        // scroll area: a message about a failed save was
        // previously only visible after scrolling down to it.
        ui.add_space(4.0);
    }

    /// Start app-ID detection when requested and apply a finished detection.
    fn handle_app_detection(&mut self, ui: &mut egui::Ui, detect_app_clicked: bool) {
        if detect_app_clicked && self.app_detection.is_none() {
            // Run entirely off the UI thread. Tracker setup has
            // bounded D-Bus calls and a total readiness deadline,
            // but a slow session bus can still consume several
            // seconds. The receiver is polled below on every frame.
            use wayexpand_backend_kwin_window::KwinWindowTracker;
            use wayexpand_core::WindowTracker;
            let (sender, receiver) = mpsc::channel();
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancelled);
            self.app_detection = Some(AppDetectionTask {
                receiver,
                cancelled,
            });
            thread::spawn(move || {
                let detection = match KwinWindowTracker::new_cancellable(Some(&worker_cancel)) {
                    Ok(mut tracker) => {
                        if worker_cancel.load(Ordering::Acquire) {
                            let _ = sender.send(AppDetection::Unavailable);
                            return;
                        }
                        match tracker.next_window_timeout(std::time::Duration::from_secs(5)) {
                            Ok(Some(Some(window))) => AppDetection::Found(window),
                            Ok(Some(None)) => AppDetection::NoWindow,
                            Ok(None) | Err(_) => AppDetection::Unavailable,
                        }
                    }
                    Err(_) => AppDetection::Unavailable,
                };
                // The GUI may have given up waiting (Cancel, or the
                // window closed) by the time this send happens; that is
                // not an error, there is simply nothing left to notify.
                let _ = sender.send(detection);
            });
        }
        let detection_cancelled = self
            .app_detection
            .as_ref()
            .is_some_and(|task| task.cancelled.load(Ordering::Acquire));
        let detection_result = self
            .app_detection
            .as_ref()
            .map(|task| task.receiver.try_recv());
        if let Some(detection_result) = detection_result {
            match detection_result {
                Ok(AppDetection::Found(window)) => {
                    self.app_detection = None;
                    if !detection_cancelled {
                        let value = window.app_id.or(window.title).unwrap_or_default();
                        if value.is_empty() {
                            self.status =
                                Status::warning(self.strings.status_window_unidentified());
                        } else {
                            if let Some(draft) = self.draft.as_mut() {
                                if !draft.app_filter.contains(&value) {
                                    draft.app_filter.push(value.clone());
                                    self.invalidate_preview();
                                }
                            }
                            self.status =
                                Status::success(self.strings.status_app_filter_added(&value));
                        }
                    }
                }
                Ok(AppDetection::NoWindow) => {
                    self.app_detection = None;
                    if !detection_cancelled {
                        self.status = Status::warning(self.strings.status_no_focused_window());
                    }
                }
                Ok(AppDetection::Unavailable) => {
                    self.app_detection = None;
                    if !detection_cancelled {
                        self.status = Status::warning(self.strings.status_detection_unavailable());
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still waiting: request another repaint soon so
                    // this gets polled promptly instead of only on the
                    // next user-driven event, without busy-looping.
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // The sender was dropped without sending, which
                    // should not happen (the spawned thread always
                    // sends before exiting) -- treat it the same as an
                    // explicit Unavailable rather than waiting forever.
                    self.app_detection = None;
                    if !detection_cancelled {
                        self.status = Status::error(self.strings.status_detection_failed());
                    }
                }
            }
        }
    }

    /// Collapsible editor for a command-backed (dynamic) replacement.
    fn render_command_editor(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.collapsing(self.strings.dynamic_command(), |ui| {
            let Some(draft) = self.draft.as_mut() else {
                ui.label(self.strings.draft_unavailable());
                return;
            };
            ui.checkbox(&mut draft.command_enabled, self.strings.command_checkbox());
            ui.label(
                RichText::new(self.strings.command_help())
                    .small()
                    .color(palette.muted),
            );
            if draft.command_enabled {
                egui::Frame::new()
                    .fill(theme::tint(palette.warning, 30))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.colored_label(palette.warning, self.strings.command_warning());
                    });
            }
            ui.add_enabled_ui(draft.command_enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Execution type");
                    if ui
                        .radio(!draft.command_action_mode, "Direct executable")
                        .clicked()
                    {
                        draft.command_action_mode = false;
                        draft.command_action.clear();
                    }
                    if ui
                        .radio(draft.command_action_mode, "Managed action")
                        .clicked()
                    {
                        draft.command_action_mode = true;
                        draft.command_program.clear();
                        draft.command_args.clear();
                    }
                });
                if draft.command_action_mode {
                    ui.horizontal(|ui| {
                        ui.label("Action");
                        let action_ids = broker_action_ids();
                        egui::ComboBox::from_id_salt("broker_action_id")
                            .selected_text(if draft.command_action.is_empty() {
                                "Select or type an action"
                            } else {
                                &draft.command_action
                            })
                            .show_ui(ui, |ui| {
                                for action_id in action_ids {
                                    if ui
                                        .selectable_label(
                                            draft.command_action == action_id,
                                            &action_id,
                                        )
                                        .clicked()
                                    {
                                        draft.command_action = action_id;
                                    }
                                }
                            });
                        ui.add(
                            TextEdit::singleline(&mut draft.command_action)
                                .margin(theme::FIELD_MARGIN)
                                .hint_text("cluster-status")
                                .desired_width(240.0),
                        );
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(self.strings.program());
                        ui.add(
                            TextEdit::singleline(&mut draft.command_program)
                                .margin(theme::FIELD_MARGIN)
                                .hint_text(self.strings.program_hint())
                                .desired_width(300.0),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(self.strings.timeout_ms());
                    ui.add(
                        TextEdit::singleline(&mut draft.command_timeout_ms)
                            .margin(theme::FIELD_MARGIN)
                            .desired_width(90.0),
                    );
                    ui.label(self.strings.cache_ms());
                    ui.add(
                        TextEdit::singleline(&mut draft.command_cache_ms)
                            .margin(theme::FIELD_MARGIN)
                            .desired_width(90.0),
                    );
                });
                if !draft.command_action_mode {
                    ui.label(self.strings.arguments());
                    let mut remove_arg = None;
                    let mut move_arg = None;
                    for index in 0..draft.command_args.len() {
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::multiline(&mut draft.command_args[index])
                                    .margin(theme::FIELD_MARGIN)
                                    .desired_rows(1)
                                    .desired_width(
                                        (ui.available_width() - 108.0).max(MIN_FIELD_WIDTH),
                                    ),
                            );
                            if ui
                                .small_button("↑")
                                .on_hover_text(self.strings.move_up())
                                .clicked()
                                && index > 0
                            {
                                move_arg = Some((index, index - 1));
                            }
                            if ui
                                .small_button("↓")
                                .on_hover_text(self.strings.move_down())
                                .clicked()
                                && index + 1 < draft.command_args.len()
                            {
                                move_arg = Some((index, index + 1));
                            }
                            if ui
                                .small_button("×")
                                .on_hover_text(self.strings.remove_argument())
                                .clicked()
                            {
                                remove_arg = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove_arg {
                        draft.command_args.remove(index);
                    }
                    if let Some((from, to)) = move_arg {
                        draft.command_args.swap(from, to);
                    }
                    if ui.small_button(self.strings.add_argument()).clicked() {
                        draft.command_args.push(String::new());
                    }
                }
                ui.horizontal(|ui| {
                    ui.label(self.strings.environment())
                        .on_hover_text(self.strings.environment_tooltip());
                    let minimal = self.strings.environment_minimal();
                    let inherit = self.strings.environment_inherit();
                    egui::ComboBox::from_id_salt("command_environment")
                        .selected_text(match draft.command_environment {
                            wayexpand_core::CommandEnvironment::Minimal => minimal,
                            wayexpand_core::CommandEnvironment::Inherit => inherit,
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.command_environment,
                                wayexpand_core::CommandEnvironment::Minimal,
                                minimal,
                            );
                            ui.selectable_value(
                                &mut draft.command_environment,
                                wayexpand_core::CommandEnvironment::Inherit,
                                inherit,
                            );
                        });
                });
                ui.label(self.strings.pass_environment());
                let mut remove_env = None;
                for index in 0..draft.command_pass_env.len() {
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut draft.command_pass_env[index])
                                .margin(theme::FIELD_MARGIN)
                                .desired_width((ui.available_width() - 38.0).max(MIN_FIELD_WIDTH)),
                        );
                        if ui
                            .small_button("×")
                            .on_hover_text(self.strings.remove_environment_variable())
                            .clicked()
                        {
                            remove_env = Some(index);
                        }
                    });
                }
                if let Some(index) = remove_env {
                    draft.command_pass_env.remove(index);
                }
                if ui
                    .small_button(self.strings.add_environment_variable())
                    .clicked()
                {
                    draft.command_pass_env.push(String::new());
                }
            });
        });
    }

    /// Live preview of the draft, including the command preview runner.
    fn render_preview_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.add_space(14.0);
        theme::section_header(ui, "", self.strings.preview());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(self.strings.input());
            ui.add(
                TextEdit::singleline(&mut self.preview_input)
                    .margin(theme::FIELD_MARGIN)
                    .hint_text(self.strings.input_hint())
                    .desired_width(420.0),
            );
            if theme::secondary_button(ui, palette, self.strings.use_trigger()).clicked() {
                self.preview_input = self
                    .draft
                    .as_ref()
                    .map(|draft| draft.trigger.clone())
                    .unwrap_or_default();
            }
        });
        ui.add_space(4.0);
        let command_backed = self
            .draft
            .as_ref()
            .is_some_and(|draft| draft.command_enabled);
        if command_backed {
            // Never auto-run a configured program from a render/repaint
            // path: unlike a template, this has real side effects and
            // this code runs every frame the editor is open. Running is
            // opt-in via the button below, and only ever once per click.
            egui::Frame::new()
                .fill(theme::tint(palette.warning, 20))
                .stroke(egui::Stroke::new(1.0, palette.warning))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    let managed_action = self
                        .draft
                        .as_ref()
                        .is_some_and(|draft| draft.command_action_mode);
                    ui.label(if managed_action {
                        self.strings.managed_command_preview_help()
                    } else {
                        self.strings.direct_command_preview_help()
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let preview_is_current =
                            self.command_preview_key == Some(self.preview_revision);
                        if ui
                            .add_enabled(
                                self.command_preview_receiver.is_none(),
                                egui::Button::new(if self.command_preview_receiver.is_some() {
                                    self.strings.running()
                                } else if managed_action {
                                    self.strings.run_through_broker()
                                } else {
                                    self.strings.run_once()
                                }),
                            )
                            .clicked()
                        {
                            self.run_command_preview();
                        }
                        match self
                            .command_preview_result
                            .as_ref()
                            .filter(|_| preview_is_current)
                        {
                            Some(Ok(output)) => {
                                let shown: String = if output.chars().count() > 200 {
                                    output.chars().take(199).collect::<String>() + "…"
                                } else {
                                    output.clone()
                                };
                                ui.label(RichText::new(shown).monospace());
                            }
                            Some(Err(message)) => {
                                ui.colored_label(palette.danger, message);
                            }
                            None => {}
                        }
                    });
                });
        } else {
            let preview_text = self.preview();
            egui::Frame::new()
                .fill(palette.extreme_bg)
                .stroke(egui::Stroke::new(1.0, palette.accent))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&preview_text).monospace());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui
                                .small_button(self.strings.copy())
                                .on_hover_text(self.strings.copy_tooltip())
                                .clicked()
                            {
                                ui.ctx().copy_text(preview_text.clone());
                                self.status = Status::success(self.strings.status_preview_copied());
                            }
                        });
                    });
                });
        }
    }

    /// First-run screen shown while the library is empty and no new
    /// snippet is being drafted: route status, setup, and a test snippet.
    fn render_first_run(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let desktop = env::var("XDG_CURRENT_DESKTOP")
            .or_else(|_| env::var("XDG_SESSION_DESKTOP"))
            .unwrap_or_else(|_| self.strings.unknown_desktop().into());
        let is_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
        let app_context_available = self.backend_status.iter().any(|status| {
            status.kind == wayexpand_core::BackendKind::WindowTracker
                && status.state == BackendState::Available
        });
        let recommendation = self.recommended_route.and_then(route_recommendation);
        let keyboard_probe = recommendation
            .map(|route| route.capture_state)
            .unwrap_or(BackendState::NotImplemented);
        let injection_probe = recommendation
            .map(|route| route.injection_state)
            .unwrap_or(BackendState::NotImplemented);
        let (recommended_route, recommended_route_detail) =
            self.strings.onboarding_recommended_route(recommendation);
        let daemon_running = self.daemon_reachable == Some(true);
        ui.vertical_centered(|ui| {
            ui.add_space(26.0);
            ui.label(RichText::new("⚡").size(40.0).color(palette.accent));
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.strings.welcome_title())
                    .size(28.0)
                    .strong()
                    .color(ui.visuals().strong_text_color()),
            );
            ui.label(RichText::new(self.strings.welcome_intro()).color(palette.muted));
            ui.add_space(22.0);
            theme::card(ui, palette, |ui| {
                ui.set_width(ui.available_width().min(600.0));
                ui.label(RichText::new(self.strings.onboarding_recommended_route_title()).strong());
                ui.label(RichText::new(recommended_route).color(palette.accent));
                ui.label(
                    RichText::new(recommended_route_detail)
                        .small()
                        .color(palette.muted),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(self.strings.onboarding_certification_note())
                        .small()
                        .color(palette.warning),
                );
            });
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width().min(600.0), ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    theme::card(ui, palette, |ui| {
                        ui.set_width(ui.available_width());
                        // Step 1: the daemon. Done is shown as done.
                        onboarding_step(
                            ui,
                            palette,
                            1,
                            self.strings.onboarding_turn_on_title(),
                            daemon_running,
                        );
                        ui.indent("step_turn_on", |ui| {
                            if daemon_running {
                                ui.label(
                                    RichText::new(self.strings.running_status())
                                        .color(palette.success),
                                );
                            } else {
                                ui.label(
                                    RichText::new(self.strings.onboarding_turn_on_help())
                                        .color(palette.muted),
                                );
                                ui.add_space(4.0);
                                self.turn_on_button(ui, palette);
                                if ui.link(self.strings.onboarding_evdev_setup()).clicked() {
                                    self.evdev_setup_acknowledged = false;
                                    self.evdev_setup_open = true;
                                }
                            }
                        });
                        ui.add_space(12.0);
                        // Step 2: a snippet to try.
                        onboarding_step(
                            ui,
                            palette,
                            2,
                            self.strings.onboarding_snippet_title(),
                            false,
                        );
                        ui.indent("step_snippet", |ui| {
                            ui.label(
                                RichText::new(self.strings.onboarding_snippet_help())
                                    .color(palette.muted),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if theme::primary_button(
                                    ui,
                                    palette,
                                    self.strings.create_test_snippet(),
                                )
                                .clicked()
                                {
                                    self.create_test_snippet();
                                }
                                if theme::secondary_button(ui, palette, self.strings.new_button())
                                    .clicked()
                                {
                                    self.request_action(PendingAction::New);
                                }
                                if theme::secondary_button(
                                    ui,
                                    palette,
                                    self.strings.import_espanso(),
                                )
                                .clicked()
                                {
                                    self.import_open = true;
                                    self.import_preview = None;
                                }
                            });
                        });
                        ui.add_space(12.0);
                        // Step 3: where to try it.
                        onboarding_step(
                            ui,
                            palette,
                            3,
                            self.strings.onboarding_safety_title(),
                            false,
                        );
                        ui.indent("step_try", |ui| {
                            ui.label(
                                RichText::new(self.strings.onboarding_try_text())
                                    .color(palette.muted),
                            );
                        });
                        ui.add_space(10.0);
                        ui.collapsing(self.strings.onboarding_details_title(), |ui| {
                            ui.label(self.strings.onboarding_desktop(&desktop, is_wayland));
                            ui.label(
                                RichText::new(self.strings.onboarding_probe_caveat())
                                    .small()
                                    .color(palette.muted),
                            );
                            if let Some(route) = recommendation {
                                ui.label(
                                    RichText::new(
                                        self.strings.onboarding_capture_detail(route.capture_label),
                                    )
                                    .small()
                                    .monospace()
                                    .color(palette.muted),
                                );
                                ui.label(
                                    RichText::new(
                                        self.strings
                                            .onboarding_injection_detail(route.injection_label),
                                    )
                                    .small()
                                    .monospace()
                                    .color(palette.muted),
                                );
                            }
                            ui.horizontal(|ui| {
                                ui.label(self.strings.onboarding_keyboard_label());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            self.strings.onboarding_backend_state(keyboard_probe),
                                        );
                                    },
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(self.strings.onboarding_injection_label());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            self.strings.onboarding_backend_state(injection_probe),
                                        );
                                    },
                                );
                            });
                            ui.label(
                                RichText::new(
                                    self.strings.onboarding_detection(app_context_available),
                                )
                                .small()
                                .color(palette.muted),
                            );
                            ui.label(
                                RichText::new(self.strings.onboarding_app_caveat())
                                    .small()
                                    .color(palette.muted),
                            );
                            ui.label(
                                RichText::new(self.strings.onboarding_certification_note())
                                    .small()
                                    .color(palette.warning),
                            );
                        });
                    });
                },
            );
        });
    }

    /// Pinned under the editor (above Save) so the one thing that proves the
    /// library works is always in view, not below the snippet form.
    pub(crate) fn render_try_live_panel(&mut self, root: &mut egui::Ui, palette: &Palette) {
        if self.config.expansion.is_empty() {
            return;
        }
        egui::Panel::bottom("try_live")
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.background)
                    .inner_margin(egui::Margin::symmetric(22, 10))
                    .stroke(egui::Stroke::NONE),
            )
            .show(root, |ui| self.render_try_live(ui, palette));
    }

    /// "Matcher preview": exercise core matching and template rendering in
    /// an in-process field. This is not a desktop integration test.
    pub(crate) fn render_try_live(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        egui::Frame::new()
            .fill(theme::tint(palette.accent, 18))
            .stroke(egui::Stroke::new(1.0, theme::tint(palette.accent, 110)))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let arrow = if self.try_live_open { "⏷" } else { "⏵" };
                    let toggle = ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!(
                                    "{arrow}  ⚡ {}",
                                    self.strings.try_live_title()
                                ))
                                .strong()
                                .color(palette.accent),
                            )
                            .frame(false),
                        )
                        .on_hover_text(self.strings.try_live_help());
                    if toggle.clicked() {
                        self.try_live_open = !self.try_live_open;
                    }
                    if self.playground.expansions > 0 {
                        theme::pill(
                            ui,
                            self.strings.try_live_stats(
                                self.playground.expansions,
                                self.playground.keystrokes_saved,
                            ),
                            palette.success,
                            theme::tint(palette.success, 34),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.try_live_open
                            && !self.playground.text.is_empty()
                            && ui.small_button(self.strings.clear()).clicked()
                        {
                            self.playground.clear();
                        }
                        // Once something has expanded, the stats say it all.
                        if self.try_live_open && self.playground.expansions == 0 {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(self.strings.try_live_help())
                                        .small()
                                        .color(palette.muted),
                                )
                                .truncate(),
                            );
                        }
                    });
                });
                if !self.try_live_open {
                    return;
                }
                ui.add_space(4.0);
                if self.playground.form_detected {
                    ui.label(
                        RichText::new(
                            "Form snippet detected. Form expansion cannot be simulated in Try It Live yet.",
                        )
                        .small()
                        .color(palette.warning),
                    );
                }
                let id = egui::Id::new("try_live_field");
                let output = TextEdit::multiline(&mut self.playground.text)
                    .id(id)
                    .margin(theme::FIELD_MARGIN)
                    .hint_text(self.strings.try_live_hint())
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .show(ui);
                if output.response.response.changed() {
                    let text = self.playground.text.clone();
                    let caret_at_end = output
                        .cursor_range
                        .is_none_or(|range| range.primary.index.0 == text.chars().count());
                    if let Some(rewrite) =
                        self.playground
                            .edit(&self.config, &self.preview_app, &text, caret_at_end)
                    {
                        let caret = rewrite
                            .caret
                            .unwrap_or_else(|| rewrite.text.chars().count());
                        self.playground.text = rewrite.text;
                        let mut state = output.state;
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(caret),
                            )));
                        state.store(ui.ctx(), id);
                    }
                }
            });
    }

    /// The editor's pinned action bar. It is a panel rather than the last
    /// row of the editor's scroll area so the primary action stays on screen
    /// however far the snippet's replacement text scrolls.
    pub(crate) fn render_editor_actions(&mut self, root: &mut egui::Ui, palette: &Palette) {
        if self.selected_index().is_none() && !self.new_draft {
            return;
        }
        egui::Panel::bottom("editor_actions")
            // Without the rule the editor's content scrolls flush against the
            // buttons and the bar stops reading as a fixed surface.
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(22, 10))
                    .stroke(egui::Stroke::NONE),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let dirty = self.draft_is_dirty();
                    if ui
                        .add_enabled_ui(dirty, |ui| {
                            theme::primary_button(ui, palette, self.strings.save_changes())
                        })
                        .inner
                        .on_hover_text(self.strings.save_tooltip())
                        .on_disabled_hover_text(self.strings.no_changes_to_save())
                        .clicked()
                    {
                        self.save_selected();
                    }
                    if self.new_draft {
                        if theme::secondary_button(ui, palette, self.strings.cancel()).clicked() {
                            self.abandon_new_draft();
                            self.status = Status::info(self.strings.ready());
                        }
                    } else if theme::danger_button(ui, palette, self.strings.delete()).clicked() {
                        self.request_action(PendingAction::Delete);
                    }
                    if dirty {
                        theme::pill(
                            ui,
                            self.strings.unsaved_changes(),
                            palette.warning,
                            theme::tint(palette.warning, 38),
                        );
                    }
                });
            });
    }
}

/// Column geometry shared by the rows of the snippet form./// Column geometry shared by the rows of the snippet form.
#[derive(Clone, Copy)]
pub(crate) struct FormColumns {
    label_width: f32,
    field_width: f32,
    /// Space above a label so its text lines up with the text inside the
    /// (padded) field beside it.
    label_offset: f32,
}

impl FormColumns {
    const GAP: f32 = 4.0;

    fn new(ui: &egui::Ui, label_width: f32) -> Self {
        let spacing = ui.spacing().item_spacing.x;
        Self {
            label_width,
            field_width: (ui.available_width() - label_width - spacing - Self::GAP).max(160.0),
            label_offset: f32::from(theme::FIELD_MARGIN.top),
        }
    }

    fn with_label_offset(self, label_offset: f32) -> Self {
        Self {
            label_offset,
            ..self
        }
    }
}

/// One label/field row of the snippet form. The field column is a bounded
/// vertical layout, so its content wraps inside the card and the row grows to
/// fit it in the same frame.
pub(crate) fn form_row(
    ui: &mut egui::Ui,
    label: &str,
    columns: FormColumns,
    add_field: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(columns.label_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(columns.label_width);
                ui.add_space(columns.label_offset);
                ui.label(label);
            },
        );
        ui.add_space(FormColumns::GAP);
        ui.vertical(|ui| {
            ui.set_max_width(columns.field_width);
            add_field(ui);
        });
    });
}

/// Insert `text` at the replacement editor's caret/// Insert `text` at the replacement editor's caret (replacing any selection)
/// and leave the caret after it, so inserting `{{cursor}}` lands where the
/// user was typing rather than at the end of the snippet. Falls back to
/// appending when the editor has no remembered caret yet.
pub(crate) fn insert_into_replacement_editor(
    ctx: &egui::Context,
    replacement: &mut String,
    text: &str,
) {
    let id = egui::Id::new(REPLACEMENT_EDITOR_SALT);
    let mut state = egui::text_edit::TextEditState::load(ctx, id);
    let selection = state
        .as_ref()
        .and_then(|state| state.cursor.char_range())
        .map(|range| {
            let range = range.as_sorted_char_range();
            range.start.0..range.end.0
        });
    let caret = insert_at_char_range(replacement, selection, text);
    if let Some(state) = state.as_mut() {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(caret),
            )));
    }
    if let Some(state) = state {
        state.store(ctx, id);
    }
    ctx.memory_mut(|memory| memory.request_focus(id));
}

/// Replace the characters in `selection` (a char-index range, clamped to the
/// text) with `insert`, or append when there is no selection. Returns the
/// char index just after the inserted text.
pub(crate) fn insert_at_char_range(
    text: &mut String,
    selection: Option<std::ops::Range<usize>>,
    insert: &str,
) -> usize {
    let length = text.chars().count();
    let range = selection.unwrap_or(length..length);
    let start = range.start.min(length);
    let end = range.end.clamp(start, length);
    let byte_at = |text: &str, index: usize| {
        text.char_indices()
            .nth(index)
            .map_or(text.len(), |(byte, _)| byte)
    };
    let (start_byte, end_byte) = (byte_at(text, start), byte_at(text, end));
    text.replace_range(start_byte..end_byte, insert);
    start + insert.chars().count()
}
