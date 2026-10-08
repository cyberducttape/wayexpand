use super::*;

impl ExpansionEngine {
    /// The currently known focused window, if any. Used to carry window
    /// context across a config reload: reload replaces the whole engine
    /// (parse-then-swap), which would otherwise silently forget the last
    /// known window until the next real focus change -- wrongly
    /// fail-closing `app_filter`-scoped expansions for a window the user
    /// never actually left.
    pub fn current_window(&self) -> Option<&WindowContext> {
        self.current_window.as_ref()
    }

    /// Restores window context captured via `current_window` before this
    /// engine replaced a previous one. Does not clear the match buffer,
    /// matching `InputEvent::WindowChanged`'s own behavior (see its
    /// handler for why that clear is unnecessary).
    pub fn set_current_window(&mut self, window: Option<WindowContext>) {
        self.set_window_context(window);
    }

    pub(super) fn set_window_context(&mut self, window: Option<WindowContext>) {
        self.normalized_window = window.as_ref().map(|window| NormalizedWindowContext {
            app_id: window.app_id.as_ref().map(|value| value.to_lowercase()),
            title: window.title.as_ref().map(|value| value.to_lowercase()),
            instance_id: window.instance_id.clone(),
        });
        self.current_window = window;
    }

    /// Returns whether the user has manually paused text expansion.
    /// This must be preserved across config reloads to maintain pause state.
    pub fn is_user_paused(&self) -> bool {
        self.user_paused
    }

    /// Restores user pause state from a previous engine instance.
    /// Critical for config reloads to preserve pause state.
    pub fn set_user_paused(&mut self, paused: bool) {
        self.user_paused = paused;
    }

    /// Returns whether the focused field is sensitive (password, OTP, etc).
    /// This must be preserved across config reloads to maintain input protection.
    pub fn is_sensitive_focus(&self) -> bool {
        self.sensitive_focus
    }

    /// Returns whether an input method or keyboard compose sequence owns the
    /// current text stream. Expansion remains disabled until the backend
    /// reports that composition has ended.
    pub fn is_composition_active(&self) -> bool {
        self.composition_active
    }

    /// Restores composition state from a previous engine instance. Critical
    /// for config reloads so matching cannot resume during an active preedit.
    pub fn set_composition_active(&mut self, active: bool) {
        self.composition_active = active;
    }

    /// Restores sensitive field focus state from a previous engine instance.
    /// Critical for config reloads to preserve password-field protection.
    pub fn set_sensitive_focus(&mut self, sensitive: bool) {
        self.sensitive_focus = sensitive;
    }

    /// Whether text expansion capture is currently enabled.
    /// Both user pause and sensitive field focus independently disable capture.
    pub(super) fn is_capture_enabled(&self) -> bool {
        !self.user_paused && !self.sensitive_focus && !self.composition_active && !self.form_active
    }

    #[cfg(any(test, feature = "fuzzing"))]
    pub(crate) fn buffer_len_for_checks(&self) -> usize {
        self.buffer.len()
    }

    #[cfg(any(test, feature = "fuzzing"))]
    pub(crate) fn max_buffer_chars_for_checks(&self) -> usize {
        self.max_buffer_chars
    }

    /// Carry runtime state across a configuration reload, from the engine
    /// this one replaces: pause, sensitive-field and composition gates,
    /// window identity, command and title-matching restrictions, terminator
    /// handling, and host hooks. Every host that reloads uses this one list,
    /// so a protection cannot be dropped by one host and kept by another.
    /// Asynchronous workers are not carried; hosts restart them.
    pub fn inherit_runtime_state(&mut self, previous: &ExpansionEngine) {
        self.set_user_paused(previous.is_user_paused());
        self.set_sensitive_focus(previous.is_sensitive_focus());
        self.set_composition_active(previous.is_composition_active());
        self.set_current_window(previous.current_window().cloned());
        self.set_commands_disabled(previous.commands_disabled());
        self.set_direct_commands_disabled(previous.direct_commands_disabled());
        self.set_title_matching_disabled(previous.title_matching_disabled());
        self.set_reinsert_terminators(previous.reinserts_terminators());
        self.set_completion_notifier(previous.completion_notifier());
        self.set_clipboard_reader(previous.clipboard_reader());
        self.set_clipboard_prefetch(previous.clipboard_prefetch());
        self.set_form_replacement_guarantee(previous.form_replacement_guarantee());
    }

    /// Whether a snippet form is open (capture is suspended meanwhile).
    pub fn is_form_open(&self) -> bool {
        self.form_active
    }

    /// Invalidate pending asynchronous expansions by incrementing the generation
    /// counter. Called when any key event arrives, before processing the hotkey
    /// action. This ensures running commands are discarded when the user presses
    /// another key, preventing output injection at the wrong cursor position.
    ///
    /// Note: does not clear `last_expansion` (the undo transaction) because
    /// the undo chord itself arrives as a Key event, and clearing it would make
    /// undo impossible to trigger. Undo validity is checked separately.
    pub fn note_key_event(&mut self) {
        self.input_generation = self.input_generation.wrapping_add(1);
        self.shared_input_generation
            .store(self.input_generation, Ordering::Release);
    }

    /// Check if a key chord is the configured undo chord. Used to preserve
    /// undo validity across the chord that triggers it.
    pub fn is_undo_chord(&self, chord: &KeyChord) -> bool {
        self.undo_chord
            .as_ref()
            .is_some_and(|undo| undo.matches(chord))
    }
}
