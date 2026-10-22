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
        let request_id = self.next_save_id;
        self.next_save_id = self.next_save_id.wrapping_add(1).max(1);
        let change = PendingSave {
            request_id,
            candidate,
            base: self.config.clone(),
            preview_revision: self.preview_revision,
            intent,
            attempts: 0,
        };
        if self.pending_save.is_some() || self.conflicted_save.is_some() {
            // Candidates are derived from `self.config`, which does not yet
            // hold the save in flight. Keep every queued change by folding
            // it into the queue rather than replacing it.
            self.queued_save = Some(match self.queued_save.take() {
                Some(queued) => combine_changes(queued, change),
                None => change,
            });
            self.status = Status::info(self.strings.status_saving());
            return true;
        }
        self.dispatch_save(change)
    }

    pub(crate) fn dispatch_save(&mut self, pending: PendingSave) -> bool {
        self.try_dispatch_save(pending).is_none()
    }

    /// Start saving `pending`, or hand it back if the background queue is
    /// full so the caller can keep it.
    fn try_dispatch_save(&mut self, pending: PendingSave) -> Option<PendingSave> {
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
            return None;
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
            return Some(pending);
        }
        self.pending_save = Some(pending);
        self.status = Status::info(self.strings.status_saving());
        None
    }

    /// Keep a change that could not be saved for an explicit retry or
    /// discard, folded into any change already kept.
    fn keep_failed_save(&mut self, change: PendingSave) {
        self.failed_save = Some(match self.failed_save.take() {
            Some(kept) => combine_changes(kept, change),
            None => change,
        });
        self.close_warned_failed_save = false;
    }

    /// Keep the change that was waiting on a conflict reload that failed.
    pub(crate) fn keep_conflicted_save(&mut self) {
        if let Some(conflicted) = self.conflicted_save.take() {
            self.keep_failed_save(conflicted);
        }
    }

    /// Save the kept change again. It is replayed onto the current
    /// configuration, which may have been reloaded from an external edit
    /// since, so anything both sides changed is reported, not overwritten.
    pub(crate) fn retry_failed_save(&mut self) {
        if self.save_in_flight() {
            self.status = Status::info(self.strings.status_saving());
            return;
        }
        let Some(failed) = self.failed_save.take() else {
            return;
        };
        let candidate = if failed.base == self.config {
            Ok(failed.candidate.clone())
        } else {
            rebase_change(
                &failed.base,
                &failed.candidate,
                &self.config,
                Rebase::DetectConflicts,
            )
        };
        match candidate {
            Ok(candidate) => {
                let retry = PendingSave {
                    candidate,
                    base: self.config.clone(),
                    attempts: 0,
                    ..failed
                };
                if let Some(retry) = self.try_dispatch_save(retry) {
                    self.keep_failed_save(retry);
                }
            }
            Err(conflicts) => {
                self.failed_save = Some(failed);
                self.status = Status::error(self.strings.status_not_saved_conflict(&conflicts));
            }
        }
    }

    pub(crate) fn discard_failed_save(&mut self) {
        if self.failed_save.take().is_some() {
            self.close_warned_failed_save = false;
            self.status = Status::info(self.strings.status_unsaved_change_discarded());
        }
    }

    pub(crate) fn dispatch_queued_save(&mut self) {
        if self.pending_save.is_some() || self.conflicted_save.is_some() {
            return;
        }
        let Some(mut queued) = self.queued_save.take() else {
            return;
        };
        if queued.base != self.config {
            // Replay the change onto the configuration that now holds the
            // earlier save; writing the stale snapshot would revert it.
            match rebase_change(
                &queued.base,
                &queued.candidate,
                &self.config,
                Rebase::LocalWins,
            ) {
                Ok(rebased) => {
                    queued.candidate = rebased;
                    queued.base = self.config.clone();
                }
                Err(conflicts) => {
                    self.status = Status::error(self.strings.status_not_saved_conflict(&conflicts));
                    return;
                }
            }
        }
        if let Some(queued) = self.try_dispatch_save(queued) {
            self.keep_failed_save(queued);
        }
    }

    /// Retry a save deferred by a busy configuration lock, once due.
    pub(crate) fn retry_busy_save(&mut self, now: Instant) {
        if self.save_retry_at.is_some_and(|due| now >= due) {
            self.save_retry_at = None;
            self.dispatch_queued_save();
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
                // Keep the change (and anything queued behind it), load the
                // version on disk, and merge or report it then. A failed save
                // is never treated as done, and replaying the queue against
                // the same stale revision could only fail again.
                let retained = match self.queued_save.take() {
                    Some(queued) => combine_changes(pending, queued),
                    None => pending,
                };
                self.conflicted_save = Some(retained);
                if self.runtime_sender.is_none() {
                    // Headless, like dispatch_save: load and merge in place.
                    match runtime::load_config_snapshot(self.path.clone()) {
                        Ok(snapshot) => self.resolve_conflicted_save(snapshot),
                        Err(error) => {
                            self.keep_conflicted_save();
                            self.status = Status::error(self.strings.status_reload_failed(&error));
                        }
                    }
                    return;
                }
                self.status = Status::warning(self.strings.status_config_changed_externally());
                self.perform_reload();
                if self.pending_reload_revision.is_none() {
                    // The reload could not even start: the change is not
                    // saved, and the queue must not wait on it forever.
                    self.keep_conflicted_save();
                    self.status = Status::error(self.strings.status_config_changed_externally());
                }
                return;
            }
            Err(runtime::SaveFailure::Busy) => {
                let mut retry = match self.queued_save.take() {
                    Some(queued) => combine_changes(pending, queued),
                    None => pending,
                };
                retry.attempts = retry.attempts.saturating_add(1);
                if retry.attempts > MAX_BUSY_SAVE_RETRIES {
                    self.keep_failed_save(retry);
                    self.status = Status::error(self.strings.status_not_saved_busy());
                    return;
                }
                self.queued_save = Some(retry);
                self.save_retry_at = Some(Instant::now() + BUSY_SAVE_RETRY_DELAY);
                self.status = Status::warning(self.strings.status_save_busy());
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
                self.keep_failed_save(pending);
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
                self.settings_keystroke_limit =
                    self.config.settings.keystroke_max_chars.to_string();
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

    /// Merge a change that hit a revision conflict into the version just
    /// loaded from disk. Without overlap it is saved on top of the external
    /// edit; with overlap the external version is loaded, the editor keeps
    /// the user's draft, and the status says exactly what was not saved.
    pub(crate) fn resolve_conflicted_save(&mut self, snapshot: runtime::ReloadSnapshot) {
        let Some(conflicted) = self.conflicted_save.take() else {
            self.apply_reload_snapshot(snapshot);
            return;
        };
        let merged = rebase_change(
            &conflicted.base,
            &conflicted.candidate,
            &snapshot.config,
            Rebase::DetectConflicts,
        );
        // Adopt the disk version as the base either way, so no later save
        // can overwrite the external edit without the user seeing it.
        self.config = snapshot.config;
        self.config_document = snapshot.document;
        self.config_revision = snapshot.revision;
        self.search_index = snapshot.search_index;
        self.undo.clear();
        self.undo_bytes = 0;
        self.invalidate_preview();
        match merged {
            Ok(candidate) => {
                let base = self.config.clone();
                if let Some(merged) = self.try_dispatch_save(PendingSave {
                    candidate,
                    base,
                    ..conflicted
                }) {
                    self.keep_failed_save(merged);
                }
            }
            Err(conflicts) => {
                self.queued_save = None;
                self.status = Status::error(self.strings.status_not_saved_conflict(&conflicts));
            }
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
        self.pending_save.is_some() || self.queued_save.is_some() || self.conflicted_save.is_some()
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

const MAX_BUSY_SAVE_RETRIES: u8 = 10;
const BUSY_SAVE_RETRY_DELAY: Duration = Duration::from_millis(200);

/// Fold a later change into an earlier unsaved one. Both were derived from
/// in-memory configurations; the later edit wins where they touch the same
/// thing, and nothing either changed is dropped.
fn combine_changes(earlier: PendingSave, later: PendingSave) -> PendingSave {
    let candidate = rebase_change(
        &later.base,
        &later.candidate,
        &earlier.candidate,
        Rebase::LocalWins,
    )
    .unwrap_or_else(|_| later.candidate.clone());
    PendingSave {
        candidate,
        base: earlier.base,
        attempts: earlier.attempts.max(later.attempts),
        ..later
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rebase {
    /// Replaying the user's own changes: the newer local edit wins.
    LocalWins,
    /// Replaying onto an external edit: anything both sides changed is a
    /// conflict that must not be overwritten silently.
    DetectConflicts,
}

/// Replay the change from `base` to `candidate` onto `onto`, by stable
/// snippet ID. Returns the triggers (or section names) that conflict.
pub(crate) fn rebase_change(
    base: &Config,
    candidate: &Config,
    onto: &Config,
    mode: Rebase,
) -> Result<Config, Vec<String>> {
    use std::collections::{HashMap, HashSet};
    let by_id = |config: &Config| {
        config
            .expansion
            .iter()
            .map(|expansion| (expansion.id.clone(), expansion.clone()))
            .collect::<HashMap<_, _>>()
    };
    let base_by_id = by_id(base);
    let candidate_by_id = by_id(candidate);
    let mut result = onto.clone();
    let mut result_by_id = by_id(onto)
        .into_iter()
        .map(|(id, expansion)| (id.to_owned(), expansion.clone()))
        .collect::<HashMap<_, _>>();
    let mut conflicts = Vec::new();
    let detect = mode == Rebase::DetectConflicts;
    // Removed locally.
    for (id, original) in &base_by_id {
        if candidate_by_id.contains_key(id) {
            continue;
        }
        let changed_externally = result_by_id.get(id) != Some(original);
        if detect && changed_externally && result_by_id.contains_key(id) {
            conflicts.push(original.trigger.clone());
            continue;
        }
        result_by_id.remove(id);
    }
    // Merge content by ID. Ordering is deliberately handled separately below;
    // mixing positional insertion with content replay loses pure reorders.
    for expansion in &candidate.expansion {
        match base_by_id.get(&expansion.id) {
            Some(original) if *original == *expansion => {}
            Some(_) => {
                // The other side made the same edit: nothing to replay.
                if result_by_id.get(&expansion.id) == Some(expansion) {
                    continue;
                }
                let changed_externally =
                    result_by_id.get(&expansion.id) != base_by_id.get(&expansion.id);
                if detect && changed_externally {
                    conflicts.push(expansion.trigger.clone());
                    continue;
                }
                result_by_id.insert(expansion.id.clone(), expansion.clone());
            }
            None => match result_by_id.get(&expansion.id) {
                Some(existing) if *existing == *expansion => {}
                Some(_) if detect => {
                    conflicts.push(expansion.trigger.clone());
                }
                Some(_) | None => {
                    result_by_id.insert(expansion.id.clone(), expansion.clone());
                }
            },
        }
    }

    let ids = |config: &Config| {
        config
            .expansion
            .iter()
            .map(|expansion| expansion.id.clone())
            .collect::<Vec<_>>()
    };
    let base_ids = ids(base);
    let candidate_ids = ids(candidate);
    let onto_ids = ids(onto);
    let base_set: HashSet<_> = base_ids.iter().collect();
    let candidate_set: HashSet<_> = candidate_ids.iter().collect();
    let onto_set: HashSet<_> = onto_ids.iter().collect();
    let base_order = |order: &[String], present: &HashSet<&String>| {
        order
            .iter()
            .filter(|id| base_set.contains(id) && present.contains(id))
            .cloned()
            .collect::<Vec<_>>()
    };
    let candidate_base_order = base_order(&candidate_ids, &candidate_set);
    let onto_base_order = base_order(&onto_ids, &onto_set);
    let candidate_base_ids = base_ids
        .iter()
        .filter(|id| candidate_set.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    let onto_base_ids = base_ids
        .iter()
        .filter(|id| onto_set.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    let local_reordered = candidate_base_order != candidate_base_ids;
    let external_reordered = onto_base_order != onto_base_ids;
    if detect && local_reordered && external_reordered && candidate_base_order != onto_base_order {
        conflicts.push("expansion order".to_owned());
    } else if candidate_ids != base_ids {
        // Replay sequence changes independently from content. The preferred
        // side owns a real reorder; otherwise preserve an external reorder
        // while still retaining locally inserted IDs.
        let prefer_candidate = !external_reordered || local_reordered;
        let preferred = if prefer_candidate {
            &candidate_ids
        } else {
            &onto_ids
        };
        let fallback = if prefer_candidate {
            &onto_ids
        } else {
            &candidate_ids
        };
        let mut seen = HashSet::new();
        result.expansion = preferred
            .iter()
            .chain(fallback.iter())
            .filter_map(|id| {
                if !seen.insert(id.clone()) {
                    return None;
                }
                result_by_id.get(id).cloned()
            })
            .collect();
    } else {
        // No local sequence edit: retain the on-disk order after content
        // additions/removals have been applied, then keep any local edit
        // that LocalWins restored after it disappeared from disk.
        let mut seen = HashSet::new();
        result.expansion = onto_ids
            .iter()
            .chain(candidate_ids.iter())
            .filter_map(|id| {
                if !seen.insert(id.clone()) {
                    return None;
                }
                result_by_id.get(id).cloned()
            })
            .collect();
    }
    let mut section = |changed: bool, differs_on_disk: bool, name: &str| -> bool {
        if !changed {
            return false;
        }
        if detect && differs_on_disk {
            conflicts.push(name.to_owned());
            return false;
        }
        true
    };
    if section(
        candidate.settings != base.settings,
        onto.settings != base.settings && onto.settings != candidate.settings,
        "settings",
    ) {
        result.settings = candidate.settings.clone();
    }
    if section(
        candidate.hotkey != base.hotkey,
        onto.hotkey != base.hotkey && onto.hotkey != candidate.hotkey,
        "hotkeys",
    ) {
        result.hotkey = candidate.hotkey.clone();
    }
    if section(
        candidate.organization != base.organization,
        onto.organization != base.organization && onto.organization != candidate.organization,
        "organization policy",
    ) {
        result.organization = candidate.organization.clone();
    }
    if conflicts.is_empty() {
        Ok(result)
    } else {
        conflicts.sort();
        conflicts.dedup();
        Err(conflicts)
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
        // Restore by stable ID rather than searching the vector once per
        // changed snippet. Large libraries can retain many modified or
        // deleted entries in one undo record; the map keeps restoration
        // linear in the number of entries.
        let current_order: Vec<String> = restored
            .expansion
            .iter()
            .map(|expansion| expansion.id.clone())
            .collect();
        let current_ids: std::collections::HashSet<_> = current_order.iter().collect();
        let mut by_id: std::collections::HashMap<_, _> = std::mem::take(&mut restored.expansion)
            .into_iter()
            .map(|expansion| (expansion.id.clone(), expansion))
            .collect();
        for previous in &self.prior_expansions {
            by_id.insert(previous.id.clone(), previous.clone());
        }
        if let Some(order) = &self.prior_order {
            restored.expansion = order
                .iter()
                .map(|id| {
                    by_id.get(id).cloned().ok_or_else(|| {
                        "undo history no longer matches the snippet library".to_owned()
                    })
                })
                .collect::<Result<_, _>>()?;
        } else {
            // IDs are unique (validated), so each entry moves out of the map
            // instead of being cloned a second time.
            restored.expansion = current_order
                .iter()
                .map(|id| {
                    by_id.remove(id).ok_or_else(|| {
                        "undo history no longer matches the snippet library".to_owned()
                    })
                })
                .collect::<Result<_, _>>()?;
            // A deleted entry was not present in `current_order`; retain the
            // historical behavior of appending it after the current entries.
            restored.expansion.extend(
                self.prior_expansions
                    .iter()
                    .filter(|expansion| !current_ids.contains(&expansion.id))
                    .filter_map(|expansion| by_id.remove(&expansion.id)),
            );
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

#[cfg(test)]
mod tests {
    use super::{rebase_change, Rebase};
    use wayexpand_core::Config;

    fn library(replacement: &str) -> Config {
        Config::parse(&format!(
            "[[expansion]]\nid = \"11111111-1111-4111-8111-111111111111\"\ntrigger = \":a\"\nreplacement = \"{replacement}\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn the_same_edit_on_both_sides_is_not_a_conflict() {
        let base = library("old");
        let edited = library("new");
        let merged = rebase_change(&base, &edited, &edited, Rebase::DetectConflicts).unwrap();
        assert_eq!(merged.expansion[0].replacement, "new");
    }

    #[test]
    fn different_edits_to_one_snippet_conflict() {
        let base = library("old");
        let error = rebase_change(
            &base,
            &library("mine"),
            &library("theirs"),
            Rebase::DetectConflicts,
        )
        .unwrap_err();
        assert_eq!(error, [":a"]);
    }

    #[test]
    fn pure_reorder_is_replayed_during_rebase() {
        let base = Config::parse(
            r#"
[[expansion]]
id = "11111111-1111-4111-8111-111111111111"
trigger = ":a"
replacement = "A"

[[expansion]]
id = "22222222-2222-4222-8222-222222222222"
trigger = ":b"
replacement = "B"
"#,
        )
        .unwrap();
        let candidate = Config::parse(
            r#"
[[expansion]]
id = "22222222-2222-4222-8222-222222222222"
trigger = ":b"
replacement = "B"

[[expansion]]
id = "11111111-1111-4111-8111-111111111111"
trigger = ":a"
replacement = "A"
"#,
        )
        .unwrap();
        let merged = rebase_change(&base, &candidate, &base, Rebase::DetectConflicts).unwrap();
        let ids: Vec<_> = merged
            .expansion
            .iter()
            .map(|expansion| expansion.id.to_string())
            .collect();
        assert_eq!(
            ids,
            [
                "22222222-2222-4222-8222-222222222222",
                "11111111-1111-4111-8111-111111111111",
            ]
        );
    }

    #[test]
    fn deletion_does_not_look_like_a_local_reorder() {
        let base = Config::parse(
            r#"
[[expansion]]
id = "11111111-1111-4111-8111-111111111111"
trigger = ":a"
replacement = "A"

[[expansion]]
id = "22222222-2222-4222-8222-222222222222"
trigger = ":b"
replacement = "B"
"#,
        )
        .unwrap();
        let candidate = Config::parse(
            r#"
[[expansion]]
id = "22222222-2222-4222-8222-222222222222"
trigger = ":b"
replacement = "B"
"#,
        )
        .unwrap();
        let onto = Config::parse(
            r#"
[[expansion]]
id = "22222222-2222-4222-8222-222222222222"
trigger = ":b"
replacement = "B"

[[expansion]]
id = "11111111-1111-4111-8111-111111111111"
trigger = ":a"
replacement = "A"
"#,
        )
        .unwrap();
        let merged = rebase_change(&base, &candidate, &onto, Rebase::DetectConflicts).unwrap();
        assert_eq!(merged.expansion.len(), 1);
        assert_eq!(merged.expansion[0].trigger, ":b");
    }

    #[test]
    fn local_edit_to_a_snippet_missing_from_onto_survives_local_wins() {
        let base = library("old");
        let onto = Config::parse("").unwrap();
        let merged = rebase_change(&base, &library("new"), &onto, Rebase::LocalWins).unwrap();
        assert_eq!(merged.expansion.len(), 1);
        assert_eq!(merged.expansion[0].replacement, "new");
    }
}
