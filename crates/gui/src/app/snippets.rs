//! Snippet lifecycle actions: create, test snippet, duplicate, delete, enable toggle, and save.

use crate::*;

impl GuiApp {
    pub(crate) fn save_selected(&mut self) {
        let Some(draft) = self.draft.as_ref().cloned() else {
            self.status = Status::warning(self.strings.no_selection());
            return;
        };
        let is_new = self.new_draft;
        let Some(index) = self
            .selected_index()
            .or_else(|| is_new.then_some(self.config.expansion.len()))
        else {
            self.status = Status::warning(self.strings.no_selection());
            return;
        };
        let mut candidate = self.config.clone();
        let command = match draft.command_config() {
            Ok(command) => command,
            Err(error) => {
                self.status =
                    Status::error(self.strings.status_command_invalid(&error.to_string()));
                return;
            }
        };
        if is_new {
            if draft.replacement.is_empty() {
                self.status = Status::error(
                    self.strings
                        .status_save_rejected(self.strings.new_snippet_replacement_required()),
                );
                return;
            }
            candidate.expansion.push(ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: draft.trigger.clone(),
                replacement: draft.replacement.clone(),
                description: draft.description.clone(),
                tags: draft.committed_tags(),
                category: draft.category.clone(),
                app_filter: draft.committed_app_filter(),
                match_mode: draft.match_mode,
                command,
                enabled: draft.enabled,
                propagate_case: draft.propagate_case,
                aliases: draft.committed_aliases(),
            });
        } else {
            candidate.expansion[index].trigger = draft.trigger.clone();
            candidate.expansion[index].aliases = draft.committed_aliases();
            candidate.expansion[index].description = draft.description.clone();
            candidate.expansion[index].tags = draft.committed_tags();
            candidate.expansion[index].category = draft.category.clone();
            candidate.expansion[index].app_filter = draft.committed_app_filter();
            candidate.expansion[index].replacement = draft.replacement.clone();
            candidate.expansion[index].enabled = draft.enabled;
            candidate.expansion[index].match_mode = draft.match_mode;
            candidate.expansion[index].propagate_case = draft.propagate_case;
            candidate.expansion[index].command = command;
        }
        if let Err(error) = candidate.validate() {
            self.status = Status::error(self.strings.status_save_rejected(&error.safe_summary()));
            return;
        }
        self.queue_save_config(candidate, SaveIntent::Snippet { is_new, index });
    }

    /// Ctrl+S: the same rule as the Save button. `draft_is_dirty` is also
    /// true for a new, not yet saved snippet, which an earlier `selected`
    /// check skipped, so Ctrl+S did nothing while writing a new snippet; a
    /// clean draft is reported instead of being rewritten to disk.
    pub(crate) fn save_shortcut(&mut self) {
        if self.draft_is_dirty() {
            self.save_selected();
        } else if self.draft.is_some() {
            self.status = Status::info(self.strings.no_changes_to_save());
        }
    }

    pub(crate) fn create_new_snippet(&mut self) {
        let mut trigger = ":new".to_owned();
        let mut suffix = 2;
        while self
            .config
            .expansion
            .iter()
            .any(|item| item.answers_to(&trigger))
        {
            trigger = format!(":new-{suffix}");
            suffix += 1;
        }
        let draft_expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger,
            replacement: String::new(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        };
        self.new_draft_origin = self.selected_id.clone();
        self.new_draft = true;
        self.set_selected_index(None);
        self.draft = Some(Draft::from_expansion(&draft_expansion));
        self.preview_input = draft_expansion.trigger;
        self.clear_command_preview();
        self.status = Status::info(self.strings.new_snippet_draft());
    }

    pub(crate) fn create_test_snippet(&mut self) {
        if !self.config.expansion.is_empty() {
            self.status = Status::warning(self.strings.no_selection());
            return;
        }
        let mut candidate = self.config.clone();
        candidate.expansion.push(ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":wayexpand-test".into(),
            replacement: "WayExpand is working!".into(),
            description: self.strings.onboarding_sample_description().into(),
            tags: vec!["tutorial".into()],
            category: self.strings.onboarding_sample_category().into(),
            app_filter: Vec::new(),
            match_mode: MatchMode::WordBoundary,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        });
        self.queue_save_config(candidate, SaveIntent::Created);
    }

    pub(crate) fn duplicate_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            self.status = Status::warning(self.strings.no_selection());
            return;
        };
        let mut duplicate = self.config.expansion[index].clone();
        let base = format!("{}-copy", duplicate.trigger);
        let mut trigger = base.clone();
        let mut suffix = 2;
        while self
            .config
            .expansion
            .iter()
            .any(|item| item.answers_to(&trigger))
        {
            trigger = format!("{base}-{suffix}");
            suffix += 1;
        }
        duplicate.trigger = trigger;
        duplicate.id = ExpansionConfig::new_id();
        if !duplicate.description.is_empty() {
            duplicate.description.push_str(" (copy)");
        }
        let mut candidate = self.config.clone();
        candidate.expansion.push(duplicate);
        self.queue_save_config(candidate, SaveIntent::Duplicated);
    }

    pub(crate) fn perform_delete_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            self.status = Status::warning(self.strings.no_selection());
            return;
        };
        let mut candidate = self.config.clone();
        let trigger = candidate.expansion[index].trigger.clone();
        candidate.expansion.remove(index);
        self.queue_save_config(candidate, SaveIntent::Deleted { index, trigger });
    }

    /// Flips a snippet's enabled flag directly from the sidebar dot and
    /// saves immediately, independent of selection or any in-progress
    /// unsaved draft. If the toggled row is the one currently being edited,
    /// only its `enabled` field is synced so other unsaved edits survive.
    pub(crate) fn toggle_enabled(&mut self, index: usize) {
        if index >= self.config.expansion.len() {
            self.status = Status::warning(self.strings.no_selection());
            return;
        }
        let mut candidate = self.config.clone();
        candidate.expansion[index].enabled = !candidate.expansion[index].enabled;
        let now_enabled = candidate.expansion[index].enabled;
        let trigger = candidate.expansion[index].trigger.clone();
        self.queue_save_config(
            candidate,
            SaveIntent::Toggled {
                index,
                enabled: now_enabled,
                trigger,
            },
        );
    }
}
