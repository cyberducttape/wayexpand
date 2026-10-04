//! Snippet library: selection, filtered/visible indices, search index, and the snippet list.

use crate::*;

impl GuiApp {
    pub(crate) fn refresh_visible_indices_cache(&mut self) {
        let cache_valid = self.visible_indices_cache.as_ref().is_some_and(|cache| {
            cache.library_revision == self.library_revision
                && cache.filter == self.filter
                && cache.category_filter == self.category_filter
                && cache.search_fields == self.search_fields
        });
        if !cache_valid {
            let indices = self.search_index.visible_indices(
                &self.config,
                &self.filter,
                self.category_filter.as_deref(),
                self.search_fields,
            );
            self.visible_indices_cache = Some(library::VisibleIndicesCache {
                library_revision: self.library_revision,
                filter: self.filter.clone(),
                category_filter: self.category_filter.clone(),
                search_fields: self.search_fields,
                indices,
            });
        }
    }

    pub(crate) fn visible_indices(&self) -> &[usize] {
        &self
            .visible_indices_cache
            .as_ref()
            .expect("visible indices cache initialized")
            .indices
    }

    /// Refresh everything derived from the committed library. Every path
    /// that replaces `self.config` must call this (or refresh both caches).
    pub(crate) fn rebuild_search_index(&mut self) {
        self.search_index = library::SearchIndex::new(&self.config);
        self.library_revision = self.library_revision.wrapping_add(1);
        self.visible_indices_cache = None;
        self.playground.invalidate();
    }

    /// Distinct, sorted, non-empty categories currently in use — drives the
    /// sidebar filter chips and the editor's "pick existing" combo box.
    pub(crate) fn categories(&self) -> &[String] {
        self.search_index.categories()
    }

    pub(crate) fn select(&mut self, index: usize) {
        self.cancel_app_detection();
        self.new_draft = false;
        self.new_draft_origin = None;
        self.set_selected_index(Some(index));
        self.draft = Some(Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = self.config.expansion[index].trigger.clone();
        self.pending_action = None;
        self.clear_command_preview();
    }

    /// The selected index, but only while it still addresses a snippet.
    /// Every action that indexes `config.expansion` goes through this, so a
    /// selection left behind by a reload or an external edit reports "no
    /// snippet selected" instead of panicking on an out-of-range index.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        match self.selected_id.as_ref() {
            Some(id) => self
                .config
                .expansion
                .iter()
                .position(|entry| &entry.id == id),
            None => self
                .selected
                .filter(|index| *index < self.config.expansion.len()),
        }
    }

    pub(crate) fn set_selected_index(&mut self, index: Option<usize>) {
        self.selected = index.filter(|index| *index < self.config.expansion.len());
        self.selected_id = self
            .selected
            .map(|index| self.config.expansion[index].id.clone());
    }

    pub(crate) fn render_snippet_list(&mut self, root: &mut egui::Ui, palette: &Palette) {
        // The first-run editor is already a focused setup surface with one
        // explicit test-snippet action. Avoid a mostly empty library panel
        // with a competing New button until there is a saved item to browse.
        if self.config.expansion.is_empty() && !self.new_draft {
            return;
        }
        // Proportional first-open width so a small window still leaves the
        // editor usable; the user can drag it wider afterwards.
        let default_width = (root.available_width() * 0.34).clamp(250.0, 340.0);
        egui::Panel::left("snippets")
            .resizable(true)
            .default_size(default_width)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(14, 14)),
            )
            .show(root, |ui| {
                ui.label(
                    RichText::new(if self.filter.is_empty() {
                        self.strings.your_library()
                    } else {
                        self.strings.filtered_snippets()
                    })
                    .size(15.0)
                    .strong(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, palette, self.strings.new_button())
                        .on_hover_text(self.strings.new_tooltip())
                        .clicked()
                    {
                        self.request_action(PendingAction::New);
                    }
                    let can_duplicate = self.selected_index().is_some();
                    if ui
                        .add_enabled_ui(can_duplicate, |ui| {
                            theme::secondary_button(ui, palette, self.strings.duplicate())
                        })
                        .inner
                        .on_disabled_hover_text(self.strings.duplicate_needs_selection())
                        .clicked()
                    {
                        self.request_action(PendingAction::Duplicate);
                    }
                    if ui
                        .add_enabled_ui(!self.undo.is_empty() && !self.save_in_flight(), |ui| {
                            theme::secondary_button(
                                ui,
                                palette,
                                &self.strings.undo_button(self.undo.len()),
                            )
                        })
                        .inner
                        .on_disabled_hover_text(self.strings.status_nothing_to_undo())
                        .clicked()
                    {
                        self.request_action(PendingAction::Undo);
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                let font_scale = self.settings_font_scale.multiplier();
                let categories = self.categories();
                let mut category_action = None;
                if !categories.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        if theme::chip_scaled(
                            ui,
                            palette,
                            self.strings.all(),
                            self.category_filter.is_none(),
                            font_scale,
                        )
                        .clicked()
                        {
                            category_action = Some(None);
                        }
                        for category in categories {
                            let selected =
                                self.category_filter.as_deref() == Some(category.as_str());
                            if theme::chip_scaled(ui, palette, category, selected, font_scale)
                                .clicked()
                            {
                                category_action = Some(if selected {
                                    None
                                } else {
                                    Some(category.clone())
                                });
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                if let Some(category) = category_action {
                    self.category_filter = category;
                }
                // Filtering walks the whole library, so it happens once per
                // frame and the same result answers both the list and the
                // empty-state check below.
                self.refresh_visible_indices_cache();
                let visible_indices = self.visible_indices();
                let nothing_visible = visible_indices.is_empty();
                // Only the rows in view are laid out and painted
                // (`show_rows`), so a library of thousands of snippets costs
                // the same per frame as a short one. Rows have a fixed height,
                // and the 11 px gap reproduces the earlier spacing exactly.
                if nothing_visible {
                    if self.config.expansion.is_empty() {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("📭").size(28.0));
                            ui.label(
                                RichText::new(self.strings.no_snippets()).color(palette.muted),
                            );
                            ui.add_space(6.0);
                        });
                    } else if nothing_visible {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("🔍").size(28.0));
                            let reason = match (self.filter.is_empty(), &self.category_filter) {
                                (false, Some(category)) => {
                                    self.strings.no_matches_category(&self.filter, category)
                                }
                                (false, None) => self.strings.no_matches_filter(&self.filter),
                                (true, Some(category)) => {
                                    self.strings.no_snippets_category(category)
                                }
                                (true, None) => self.strings.no_snippets_match_filter().to_owned(),
                            };
                            ui.label(RichText::new(reason).color(palette.muted));
                            ui.add_space(6.0);
                            if theme::secondary_button(ui, palette, self.strings.clear_filters())
                                .clicked()
                            {
                                self.filter.clear();
                                self.category_filter = None;
                            }
                        });
                    }
                } else {
                    let row_height = 48.0 * font_scale;
                    let mut toggle_index = None;
                    let mut selected_index = None;
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 11.0;
                        ScrollArea::vertical().show_rows(
                            ui,
                            row_height,
                            visible_indices.len(),
                            |ui, rows| {
                                for &index in &visible_indices[rows] {
                                    let expansion = &self.config.expansion[index];
                                    // The row's `cmd` badge already marks command-backed
                                    // snippets, so the user's own description wins; the
                                    // generic summary only fills an otherwise empty line.
                                    let detail = if expansion.description.is_empty()
                                        && expansion.command.is_some()
                                    {
                                        self.strings.command_backed_summary()
                                    } else {
                                        expansion.description.as_str()
                                    };
                                    let response = theme::snippet_row_scaled(
                                        ui,
                                        palette,
                                        theme::SnippetRow {
                                            selected: self.selected_index() == Some(index),
                                            enabled: expansion.enabled,
                                            command_backed: expansion.command.is_some(),
                                            trigger: &expansion.trigger,
                                            detail,
                                            category: &expansion.category,
                                            detail_placeholder: self.strings.no_description(),
                                            toggle_hint: if expansion.enabled {
                                                self.strings.click_to_disable()
                                            } else {
                                                self.strings.click_to_enable()
                                            },
                                        },
                                        font_scale,
                                    );
                                    if response.toggle.clicked() {
                                        toggle_index = Some(index);
                                    } else if response.row.clicked() {
                                        selected_index = Some(index);
                                    }
                                }
                            },
                        );
                    });
                    if let Some(index) = toggle_index {
                        self.toggle_enabled(index);
                    } else if let Some(index) = selected_index {
                        self.request_action(PendingAction::Select(index));
                    }
                }
            });
    }
}
