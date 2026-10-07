//! Saving, undo history, and reload: queued background saves, conflict handling, and bounded undo deltas.

use crate::*;

impl GuiApp {
    pub(crate) fn remember_undo(&mut self, previous: Config) {
        let Some(entry) = UndoEntry::between(&previous, &self.config) else {
            return;
        };
        if entry.estimated_bytes > MAX_UNDO_BYTES {
            self.undo.clear();
            self.undo_bytes = 0;
            return;
        }
        while self.undo.len() >= MAX_UNDO_HISTORY
            || self.undo_bytes.saturating_add(entry.estimated_bytes) > MAX_UNDO_BYTES
        {
            let Some(oldest) = self.undo.first() else {
                break;
            };
            self.undo_bytes = self.undo_bytes.saturating_sub(oldest.estimated_bytes);
            self.undo.remove(0);
        }
        self.undo_bytes = self.undo_bytes.saturating_add(entry.estimated_bytes);
        self.undo.push(entry);
    }

    pub(crate) fn queue_save_config(&mut self, candidate: Config, intent: SaveIntent) -> bool {
        if self.pending_save.is_some() {
            let request_id = self.next_save_id;
            self.next_save_id = self.next_save_id.wrapping_add(1).max(1);
            self.queued_save = Some(PendingSave {
                request_id,
                candidate,
                preview_revision: self.preview_revision,
                intent,
            });
            self.status = Status::info(self.strings.status_saving());
            return true;
        }
        let request_id = self.next_save_id;
        self.next_save_id = self.next_save_id.wrapping_add(1).max(1);
        self.dispatch_save(PendingSave {
            request_id,
            candidate,
            preview_revision: self.preview_revision,
            intent,
        })
    }

    pub(crate) fn dispatch_save(&mut self, pending: PendingSave) -> bool {
        let request_id = pending.request_id;
        let Some(sender) = self.runtime_sender.as_ref() else {
            // Headless unit tests construct GuiApp without the application
            // runtime. Production starts the coordinator before rendering.
            let result = runtime::save_config(
                self.path.clone(),
                pending.candidate.clone(),
                self.config_document.clone(),
                self.config_revision.clone(),
            );
            self.pending_save = Some(pending);
            self.finish_save(request_id, result);
            return true;
        };
        if sender
            .try_send(runtime::Request::SaveConfig {
                request_id,
                path: self.path.clone(),
                candidate: Box::new(pending.candidate.clone()),
                base_document: Box::new(self.config_document.clone()),
                expected_revision: self.config_revision.clone(),
            })
            .is_err()
        {
            self.status = Status::warning(self.strings.status_save_busy());
            return false;
        }
        self.pending_save = Some(pending);
        self.status = Status::info(self.strings.status_saving());
        true
    }

    pub(crate) fn dispatch_queued_save(&mut self) {
        if self.pending_save.is_none() {
            if let Some(pending) = self.queued_save.take() {
                let _ = self.dispatch_save(pending);
            }
        }
    }

    pub(crate) fn finish_save(
        &mut self,
        request_id: u64,
        result: Result<
            (wayexpand_core::ConfigRevision, toml_edit::DocumentMut),
            runtime::SaveFailure,
        >,
    ) {
        let Some(pending) = self.pending_save.take() else {
            return;
        };
        if pending.request_id != request_id {
            self.pending_save = Some(pending);
            return;
        }
        let (revision, document) = match result {
            Ok(result) => result,
            Err(runtime::SaveFailure::Conflict) => {
                self.status = Status::warning(self.strings.status_config_changed_externally());
                self.dispatch_queued_save();
                return;
            }
            Err(runtime::SaveFailure::Busy) => {
                self.status = Status::warning(self.strings.status_save_busy());
                self.dispatch_queued_save();
                return;
            }
            Err(runtime::SaveFailure::Failed(error)) => {
                let strings = &self.strings;
                self.status = Status::error(match &pending.intent {
                    SaveIntent::Settings => strings.status_settings_save_failed(&error),
                    SaveIntent::Import(_) => strings.status_import_save_failed(&error),
                    SaveIntent::Snippet { .. } => strings.status_save_failed(&error),
                    SaveIntent::Undo => strings.status_undo_save_failed(&error),
                    SaveIntent::Created => strings.status_create_failed(&error),
                    SaveIntent::Duplicated => strings.status_duplicate_failed(&error),
                    SaveIntent::Deleted { .. } => strings.status_delete_failed(&error),
                    SaveIntent::Toggled { .. } => strings.status_toggle_failed(&error),
                });
                self.dispatch_queued_save();
                return;
            }
        };
        self.config_document = document;
        self.config_revision = revision;
        let newer_draft_exists = self.preview_revision != pending.preview_revision;
        let candidate = pending.candidate;
        match pending.intent {
            SaveIntent::Settings => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                self.settings_buffer = self.config.settings.max_buffer_chars.to_string();
                self.settings_undo_chord =
                    self.config.settings.undo_chord.clone().unwrap_or_default();
                self.settings_open = false;
                self.settings_error = None;
                self.set_saved_status(Status::success(self.strings.status_settings_saved()));
            }
            SaveIntent::Import(message) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                self.set_selected_index((!self.config.expansion.is_empty()).then_some(0));
                self.draft = self
                    .selected
                    .map(|index| Draft::from_expansion(&self.config.expansion[index]));
                self.import_open = false;
                self.set_saved_status(Status::success(message));
            }
            SaveIntent::Snippet { is_new, index } => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                if is_new {
                    self.new_draft = false;
                    self.new_draft_origin = None;
                    self.set_selected_index(Some(index));
                }
                // The file now holds this candidate, so the in-memory config
                // must too, or a later save would silently revert it. Only
                // the editor keeps the edits typed while the save ran.
                if newer_draft_exists {
                    self.status =
                        Status::warning(self.strings.status_save_completed_with_newer_edits());
                    self.dispatch_queued_save();
                    return;
                }
                self.draft = self
                    .selected
                    .map(|selected| Draft::from_expansion(&self.config.expansion[selected]));
                self.preview_input = self
                    .selected
                    .map(|selected| self.config.expansion[selected].trigger.clone())
                    .unwrap_or_default();
                self.clear_command_preview();
                self.set_saved_status(Status::success(self.strings.status_snippet_saved()));
                self.maybe_execute_pending_action();
            }
            SaveIntent::Undo => {
                if let Some(entry) = self.undo.pop() {
                    self.undo_bytes = self.undo_bytes.saturating_sub(entry.estimated_bytes);
                }
                self.config = candidate;
                self.rebuild_search_index();
                let restored_selection = self
                    .selected_id
                    .as_ref()
                    .and_then(|id| self.config.expansion.iter().position(|item| &item.id == id))
                    .or_else(|| {
                        self.selected
                            .filter(|index| *index < self.config.expansion.len())
                    });
                self.set_selected_index(restored_selection);
                self.draft = self
                    .selected
                    .map(|index| Draft::from_expansion(&self.config.expansion[index]));
                self.clear_command_preview();
                self.set_saved_status(Status::success(self.strings.status_undone()));
            }
            SaveIntent::Created => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                self.select(0);
                self.set_saved_status(Status::success(self.strings.status_created()));
            }
            SaveIntent::Duplicated => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                self.select(self.config.expansion.len() - 1);
                self.set_saved_status(Status::success(self.strings.status_duplicated()));
            }
            SaveIntent::Deleted { index, trigger } => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                let next_selection = (!self.config.expansion.is_empty())
                    .then_some(index.min(self.config.expansion.len() - 1));
                self.set_selected_index(next_selection);
                self.draft = self
                    .selected
                    .map(|selected| Draft::from_expansion(&self.config.expansion[selected]));
                self.clear_command_preview();
                self.set_saved_status(Status::success(self.strings.status_deleted(&trigger)));
            }
            SaveIntent::Toggled {
                index,
                enabled,
                trigger,
            } => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                if self.selected == Some(index) {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.enabled = enabled;
                    }
                }
                let message = if enabled {
                    self.strings.status_snippet_enabled(&trigger)
                } else {
                    self.strings.status_snippet_disabled(&trigger)
                };
                self.set_saved_status(Status::success(message));
            }
        }
        self.dispatch_queued_save();
    }

    /// Reports a config change that was just saved to disk, then asks the
    /// running daemon to reload it. A failed reload request is appended to
    /// the status line rather than discarded -- and downgrades the tone from
    /// success to warning -- because otherwise the GUI reports success while
    /// the daemon keeps expanding the old config, and the user has no way to
    /// know the two have diverged.
    pub(crate) fn set_saved_status(&mut self, status: Status) {
        self.invalidate_preview();
        self.clear_command_preview();
        let Some(sender) = self.runtime_sender.as_ref() else {
            self.status = status;
            return;
        };
        match sender.try_send(runtime::Request::Control {
            daemon_operation: wayexpand_core::DaemonOperation::Reload,
            operation: runtime::Operation::Reload(status.clone()),
        }) {
            Ok(()) => {
                self.pending_control += 1;
                self.status = Status::info(self.strings.daemon_reloading());
            }
            Err(_) => {
                self.status = status.with_caveat(self.strings.background_queue_full());
            }
        }
    }

    pub(crate) fn perform_reload(&mut self) {
        if self.pending_reload_revision.is_some() {
            return;
        }
        let Some(sender) = self.runtime_sender.as_ref() else {
            self.status = Status::error(self.strings.background_runtime_stopped());
            return;
        };
        match sender.try_send(runtime::Request::ReloadConfig {
            path: self.path.clone(),
        }) {
            Ok(()) => {
                self.pending_reload_revision = Some(self.config_revision.clone());
                self.pending_control += 1;
                self.status = Status::info(self.strings.config_reload_running());
            }
            Err(_) => self.status = Status::warning(self.strings.background_queue_full()),
        }
    }

    pub(crate) fn apply_reload_snapshot(&mut self, snapshot: runtime::ReloadSnapshot) {
        let new_config = snapshot.config;
        let new_selected = self
            .selected_id
            .as_ref()
            .and_then(|id| {
                new_config
                    .expansion
                    .iter()
                    .position(|entry| &entry.id == id)
            })
            .or_else(|| (!new_config.expansion.is_empty()).then_some(0));
        let new_draft =
            new_selected.map(|index| Draft::from_expansion(&new_config.expansion[index]));
        let new_preview_input = new_selected
            .map(|index| new_config.expansion[index].trigger.clone())
            .unwrap_or_default();

        self.config = new_config;
        self.new_draft = false;
        self.new_draft_origin = None;
        self.search_index = snapshot.search_index;
        self.playground.invalidate();
        self.config_document = snapshot.document;
        self.config_revision = snapshot.revision;
        self.set_selected_index(new_selected);
        self.draft = new_draft;
        self.preview_input = new_preview_input;
        self.undo.clear();
        self.undo_bytes = 0;
        self.invalidate_preview();
        self.clear_command_preview();
        self.theme_refresh_pending = true;
        self.status = Status::success(self.strings.status_config_reloaded());
    }

    /// Undo restores against `self.config`, which only changes when a save
    /// completes. While a save is pending or queued, both the config and the
    /// top undo entry are about to change, so an undo computed now would pop
    /// the wrong entry and could silently discard the in-flight change.
    pub(crate) fn save_in_flight(&self) -> bool {
        self.pending_save.is_some() || self.queued_save.is_some()
    }

    pub(crate) fn undo(&mut self) {
        if self.save_in_flight() {
            self.status = Status::info(self.strings.status_saving());
            return;
        }
        let Some(entry) = self.undo.last() else {
            self.status = Status::warning(self.strings.status_nothing_to_undo());
            return;
        };
        let restored = match entry.restore(&self.config) {
            Ok(config) => config,
            Err(error) => {
                self.status = Status::error(self.strings.status_undo_save_failed(&error));
                return;
            }
        };
        self.queue_save_config(restored, SaveIntent::Undo);
    }
}

/// A bounded inverse operation. Snippet edits retain only the prior versions
/// of changed snippets; ordering is stored only for insert/delete/reorder
/// operations. Settings and policy sections are copied only when changed.
pub(crate) struct UndoEntry {
    prior_expansions: Vec<ExpansionConfig>,
    prior_order: Option<Vec<String>>,
    prior_hotkeys: Option<Vec<wayexpand_core::HotkeyConfig>>,
    prior_settings: Option<Settings>,
    prior_organization: Option<OrganizationPolicy>,
    estimated_bytes: usize,
}

impl UndoEntry {
    pub(crate) fn between(previous: &Config, current: &Config) -> Option<Self> {
        let current_by_id: std::collections::HashMap<_, _> = current
            .expansion
            .iter()
            .map(|expansion| (expansion.id.as_str(), expansion))
            .collect();
        let prior_expansions: Vec<_> = previous
            .expansion
            .iter()
            .filter(|expansion| current_by_id.get(expansion.id.as_str()) != Some(expansion))
            .cloned()
            .collect();
        let order_unchanged = previous
            .expansion
            .iter()
            .map(|expansion| expansion.id.as_str())
            .eq(current
                .expansion
                .iter()
                .map(|expansion| expansion.id.as_str()));
        let prior_order = (!order_unchanged).then(|| {
            previous
                .expansion
                .iter()
                .map(|expansion| expansion.id.clone())
                .collect()
        });
        let prior_hotkeys = (previous.hotkey != current.hotkey).then(|| previous.hotkey.clone());
        let prior_settings =
            (previous.settings != current.settings).then(|| previous.settings.clone());
        let prior_organization =
            (previous.organization != current.organization).then(|| previous.organization.clone());

        if prior_expansions.is_empty()
            && prior_order.is_none()
            && prior_hotkeys.is_none()
            && prior_settings.is_none()
            && prior_organization.is_none()
        {
            return None;
        }

        let mut entry = Self {
            prior_expansions,
            prior_order,
            prior_hotkeys,
            prior_settings,
            prior_organization,
            estimated_bytes: 0,
        };
        entry.estimated_bytes = entry.estimate_retained_bytes();
        Some(entry)
    }

    pub(crate) fn restore(&self, current: &Config) -> Result<Config, String> {
        let mut restored = current.clone();
        for previous in &self.prior_expansions {
            if let Some(expansion) = restored
                .expansion
                .iter_mut()
                .find(|expansion| expansion.id == previous.id)
            {
                *expansion = previous.clone();
            } else {
                restored.expansion.push(previous.clone());
            }
        }
        if let Some(order) = &self.prior_order {
            let by_id: std::collections::HashMap<_, _> = restored
                .expansion
                .into_iter()
                .map(|expansion| (expansion.id.clone(), expansion))
                .collect();
            restored.expansion = order
                .iter()
                .map(|id| {
                    by_id.get(id).cloned().ok_or_else(|| {
                        "undo history no longer matches the snippet library".to_owned()
                    })
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(hotkeys) = &self.prior_hotkeys {
            restored.hotkey.clone_from(hotkeys);
        }
        if let Some(settings) = &self.prior_settings {
            restored.settings.clone_from(settings);
        }
        if let Some(organization) = &self.prior_organization {
            restored.organization.clone_from(organization);
        }
        Ok(restored)
    }

    /// Retained size of this entry, as counted against the undo byte budget.
    #[cfg(test)]
    pub(crate) fn estimated_bytes(&self) -> usize {
        self.estimated_bytes
    }

    fn estimate_retained_bytes(&self) -> usize {
        fn serialized_estimate<T: serde::Serialize>(value: &T) -> usize {
            serde_json::to_vec(value)
                .map(|serialized| serialized.len())
                .unwrap_or(MAX_UNDO_BYTES.saturating_add(1))
        }

        let mut structural = std::mem::size_of::<Self>()
            .saturating_add(
                self.prior_expansions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ExpansionConfig>()),
            )
            .saturating_add(self.prior_order.as_ref().map_or(0, |order| {
                order
                    .capacity()
                    .saturating_mul(std::mem::size_of::<String>())
            }))
            .saturating_add(self.prior_hotkeys.as_ref().map_or(0, |hotkeys| {
                hotkeys
                    .capacity()
                    .saturating_mul(std::mem::size_of::<wayexpand_core::HotkeyConfig>())
            }));
        let nested_string_count = self
            .prior_expansions
            .iter()
            .map(|expansion| {
                expansion.tags.len()
                    + expansion.app_filter.len()
                    + expansion
                        .command
                        .as_ref()
                        .map_or(0, |command| command.args.len() + command.pass_env.len())
            })
            .sum::<usize>()
            .saturating_add(self.prior_hotkeys.as_ref().map_or(0, |hotkeys| {
                hotkeys
                    .iter()
                    .map(|hotkey| hotkey.command.args.len() + hotkey.command.pass_env.len())
                    .sum()
            }))
            .saturating_add(self.prior_organization.as_ref().map_or(0, |policy| {
                policy.allowed_backends.len() + policy.allowed_packs.len()
            }));
        structural = structural
            .saturating_add(nested_string_count.saturating_mul(std::mem::size_of::<String>()));
        let payload = serialized_estimate(&self.prior_expansions)
            .saturating_add(self.prior_order.as_ref().map_or(0, serialized_estimate))
            .saturating_add(self.prior_hotkeys.as_ref().map_or(0, serialized_estimate))
            .saturating_add(self.prior_settings.as_ref().map_or(0, serialized_estimate))
            .saturating_add(
                self.prior_organization
                    .as_ref()
                    .map_or(0, serialized_estimate),
            );
        structural.saturating_add(payload)
    }
}
