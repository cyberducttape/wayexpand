//! "Matcher preview": an in-process preview of saved snippet matching.
//!
//! Each typed character is fed through the same core matcher the daemon uses,
//! with commands disabled. It does not exercise desktop capture, focus,
//! portals, keyboard layouts, pass-through, sensitive fields, or real client
//! insertion.

use wayexpand_core::{
    form_fields, Config, ExpansionEngine, ExpansionResult, InputEvent, WindowContext,
};

/// What the field should become after an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rewrite {
    pub text: String,
    /// Caret position in characters, when an expansion moved it.
    pub caret: Option<usize>,
}

#[derive(Default)]
pub(crate) struct Playground {
    pub text: String,
    /// The text as the engine last saw it; the next edit is diffed against
    /// this to recover the keystrokes.
    seen: String,
    engine: Option<ExpansionEngine>,
    pub expansions: usize,
    /// Characters the user did not have to type (replacement minus trigger).
    pub keystrokes_saved: usize,
    pub last_trigger: Option<String>,
    /// True when the current text ends with a form trigger. Forms need the
    /// deferred form overlay and cannot be simulated by this text-only field.
    pub form_detected: bool,
}

impl Playground {
    /// Forget the compiled library; it is rebuilt from the next saved config.
    pub fn invalidate(&mut self) {
        self.engine = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.seen.clear();
        self.form_detected = false;
        if let Some(engine) = self.engine.as_mut() {
            engine.process(InputEvent::Reset);
        }
    }

    /// Apply the field's new contents. `caret_at_end` is false when the user
    /// edited somewhere other than the end, which resets matching the same
    /// way cursor navigation does in a real application.
    pub fn edit(
        &mut self,
        config: &Config,
        app: &str,
        new_text: &str,
        caret_at_end: bool,
    ) -> Option<Rewrite> {
        self.form_detected = config.expansion.iter().any(|expansion| {
            expansion.enabled
                && form_fields(&expansion.replacement).is_ok_and(|fields| !fields.is_empty())
                && std::iter::once(&expansion.trigger)
                    .chain(&expansion.aliases)
                    .any(|trigger| new_text.ends_with(trigger))
        });
        if self.engine.is_none() {
            self.engine = build_engine(config);
        }
        let engine = self.engine.as_mut()?;
        let window = (!app.trim().is_empty()).then(|| WindowContext {
            app_id: Some(app.trim().to_owned()),
            title: None,
            instance_id: None,
        });
        if engine.current_window() != window.as_ref() {
            engine.set_current_window(window);
        }

        let appended = new_text
            .strip_prefix(self.seen.as_str())
            .filter(|_| caret_at_end);
        let Some(appended) = appended else {
            let removed_at_end = self
                .seen
                .strip_prefix(new_text)
                .filter(|_| caret_at_end)
                .map(|removed| removed.chars().count());
            match removed_at_end {
                Some(count) => {
                    for _ in 0..count {
                        engine.process(InputEvent::Backspace);
                    }
                }
                None => {
                    engine.process(InputEvent::Reset);
                }
            }
            self.seen = new_text.to_owned();
            return None;
        };

        let mut text = self.seen.clone();
        let mut caret = None;
        let mut rewritten = false;
        for character in appended.chars() {
            text.push(character);
            caret = None;
            for result in engine.process(InputEvent::Text(character.to_string())) {
                if let Some(moved) = apply(&mut text, &result) {
                    rewritten = true;
                    caret = moved;
                    self.expansions += 1;
                    self.keystrokes_saved += result
                        .insert
                        .chars()
                        .count()
                        .saturating_sub(result.matched_text.chars().count());
                    self.last_trigger = Some(result.trigger.clone());
                    engine.commit_applied_expansion(&result);
                }
            }
        }
        self.seen = text.clone();
        rewritten.then_some(Rewrite { text, caret })
    }
}

/// The library as the daemon would run it, minus anything with side
/// effects: command-backed snippets never run from a text field.
fn build_engine(config: &Config) -> Option<ExpansionEngine> {
    let mut engine = ExpansionEngine::new(config.clone()).ok()?;
    engine.set_commands_disabled(true);
    Some(engine)
}

/// Replace the matched suffix of `text` with the result, returning the new
/// caret position when the replacement placed one with `{{cursor}}`. The
/// suffix is checked first; a result that does not match the text is
/// ignored rather than erasing unrelated characters.
fn apply(text: &mut String, result: &ExpansionResult) -> Option<Option<usize>> {
    let mut erase = result.matched_text.clone();
    let mut insert = result.insert.clone();
    if let Some(character) = result.reinsert_after {
        erase.push(character);
        insert.push(character);
    }
    if !text.ends_with(&erase) {
        return None;
    }
    text.truncate(text.len() - erase.len());
    text.push_str(&insert);
    let back = result
        .cursor_offset
        .filter(|offset| *offset > 0)
        .map(|offset| offset + usize::from(result.reinsert_after.is_some()));
    Some(back.map(|back| text.chars().count().saturating_sub(back)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::parse(
            r#"
            [[expansion]]
            trigger = ";sig"
            replacement = "Best regards"

            [[expansion]]
            trigger = "brb"
            replacement = "be right back"
            match_mode = "word-boundary"

            [[expansion]]
            trigger = ";tag"
            replacement = "v{{cursor}}!"

            [[expansion]]
            trigger = ";cmd"
            replacement = ""
            [expansion.command]
            program = "/bin/echo"
            args = ["side effect"]
            "#,
        )
        .unwrap()
    }

    /// Type `input` one keystroke at a time, as egui reports it.
    fn type_text(playground: &mut Playground, config: &Config, input: &str) -> Option<usize> {
        let mut caret = None;
        for character in input.chars() {
            let mut next = playground.text.clone();
            next.push(character);
            playground.text = next.clone();
            if let Some(rewrite) = playground.edit(config, "", &next, true) {
                playground.text = rewrite.text;
                caret = rewrite.caret;
            }
        }
        caret
    }

    #[test]
    fn a_trigger_expands_in_place_and_counts_saved_keystrokes() {
        let config = config();
        let mut playground = Playground::default();
        type_text(&mut playground, &config, "Thanks! ;sig");
        assert_eq!(playground.text, "Thanks! Best regards");
        assert_eq!(playground.expansions, 1);
        assert_eq!(
            playground.keystrokes_saved,
            "Best regards".len() - ";sig".len()
        );
        assert_eq!(playground.last_trigger.as_deref(), Some(";sig"));
    }

    #[test]
    fn word_boundary_triggers_keep_the_delimiter_that_completed_them() {
        let config = config();
        let mut playground = Playground::default();
        type_text(&mut playground, &config, "brbx brb ");
        assert_eq!(playground.text, "brbx be right back ");
    }

    #[test]
    fn cursor_markers_place_the_caret() {
        let config = config();
        let mut playground = Playground::default();
        let caret = type_text(&mut playground, &config, ";tag");
        assert_eq!(playground.text, "v!");
        assert_eq!(caret, Some(1));
    }

    #[test]
    fn command_snippets_are_never_run() {
        let config = config();
        let mut playground = Playground::default();
        type_text(&mut playground, &config, ";cmd ");
        assert_eq!(playground.text, ";cmd ");
        assert_eq!(playground.expansions, 0);
    }

    #[test]
    fn backspace_and_mid_text_edits_keep_the_matcher_in_step() {
        let config = config();
        let mut playground = Playground::default();
        type_text(&mut playground, &config, ";six");
        // Backspace twice, then finish the trigger.
        playground.text = ";s".into();
        assert!(playground.edit(&config, "", ";s", true).is_none());
        type_text(&mut playground, &config, "ig");
        assert_eq!(playground.text, "Best regards");

        // An edit away from the end resets matching instead of expanding.
        playground.clear();
        type_text(&mut playground, &config, ";si");
        assert!(playground.edit(&config, "", "X;si", false).is_none());
        playground.text = "X;si".into();
        type_text(&mut playground, &config, "g");
        assert_eq!(playground.text, "X;sig");
    }
}
