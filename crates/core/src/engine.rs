use crate::{
    render_template, CommandConfig, Config, ConfigError, HotkeyConfig, KeyChord, MatchMode,
    Matcher, TemplateContext, TextInjector,
};
use std::{
    collections::VecDeque,
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const MAX_RESULTS_PER_EVENT: usize = 1024;
const MAX_RESULT_BYTES_PER_EVENT: usize = 4 * 1024 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Text(String),
    Backspace,
    Boundary,
    /// Changes capture policy for the focused surface. Sensitive fields must
    /// disable matching and clear any text already buffered.
    FocusChanged {
        sensitive: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionResult {
    pub trigger: String,
    pub erase_chars: usize,
    pub insert: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyResult {
    pub chord: KeyChord,
    pub description: String,
    pub command: CommandConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ExpansionError {
    #[error("injection failed: {0}")]
    Injection(#[from] crate::InjectorError),
}

pub struct ExpansionEngine {
    config: Config,
    matcher: Matcher,
    matcher_indices: Vec<usize>,
    buffer: VecDeque<char>,
    max_buffer_chars: usize,
    capture_enabled: bool,
    command_cache: Vec<Option<CommandCacheEntry>>,
    hotkeys: Vec<(KeyChord, usize)>,
}

#[derive(Debug, Clone)]
struct CommandCacheEntry {
    expires_at: Instant,
    value: String,
}

impl ExpansionEngine {
    pub fn new(config: Config) -> Result<Self, ConfigError> {
        config.validate()?;
        let enabled: Vec<(usize, String)> = config
            .expansion
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.enabled)
            .map(|(index, entry)| (index, entry.trigger.clone()))
            .collect();
        let matcher_indices = enabled.iter().map(|(index, _)| *index).collect();
        let matcher = Matcher::new(enabled.into_iter().map(|(_, trigger)| trigger));
        let max_buffer_chars = config.settings.max_buffer_chars;
        let command_cache = vec![None; config.expansion.len()];
        let hotkeys = config
            .hotkey
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.enabled)
            .filter_map(|(index, binding)| {
                KeyChord::parse(&binding.chord)
                    .ok()
                    .map(|chord| (chord, index))
            })
            .collect();
        Ok(Self {
            config,
            matcher,
            matcher_indices,
            buffer: VecDeque::new(),
            max_buffer_chars,
            capture_enabled: true,
            command_cache,
            hotkeys,
        })
    }

    /// Resolve a normalized key chord into configured actions. This method is
    /// side-effect free; the daemon or script runtime owns execution policy,
    /// cancellation, and capability checks.
    pub fn process_key(&self, chord: &KeyChord) -> Vec<HotkeyResult> {
        if !self.capture_enabled {
            return Vec::new();
        }
        self.hotkeys
            .iter()
            .filter(|(configured, _)| configured.matches(chord))
            .filter_map(|(_, index)| {
                let binding: &HotkeyConfig = self.config.hotkey.get(*index)?;
                Some(HotkeyResult {
                    chord: chord.clone(),
                    description: binding.description.clone(),
                    command: binding.command.clone(),
                })
            })
            .collect()
    }

    /// Process an event stream. A text event may contain multiple Unicode
    /// scalar values; matching is performed after each one.
    pub fn process(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        match event {
            InputEvent::Text(text) => {
                let mut results = Vec::new();
                let mut result_bytes = 0usize;
                if !self.capture_enabled {
                    return results;
                }
                for character in text.chars() {
                    let pending = self
                        .matcher
                        .find_suffix(&self.buffer.iter().collect::<String>());
                    if let Some((index, length)) = pending {
                        if let Some(config_index) = self.matcher_indices.get(index).copied() {
                            let trigger = &self.config.expansion[config_index].trigger;
                            if !self.matcher.can_continue(trigger, character) {
                                let trailing_word_character = self.config.expansion[config_index]
                                    .match_mode
                                    == MatchMode::WordBoundary
                                    && is_word_character(character);
                                if !trailing_word_character {
                                    if let Some(result) = self.take_match(config_index, length) {
                                        let bytes = result
                                            .trigger
                                            .len()
                                            .saturating_add(result.insert.len());
                                        if results.len() >= MAX_RESULTS_PER_EVENT
                                            || result_bytes.saturating_add(bytes)
                                                > MAX_RESULT_BYTES_PER_EVENT
                                        {
                                            self.buffer.clear();
                                            break;
                                        }
                                        result_bytes = result_bytes.saturating_add(bytes);
                                        results.push(result);
                                    }
                                }
                            }
                        }
                    }
                    if results.len() >= MAX_RESULTS_PER_EVENT {
                        self.buffer.clear();
                        break;
                    }
                    self.buffer.push_back(character);
                    while self.buffer.len() > self.max_buffer_chars {
                        self.buffer.pop_front();
                    }
                    let buffer: String = self.buffer.iter().collect();
                    if let Some((index, length)) = self.matcher.find_suffix(&buffer) {
                        let config_index = self.matcher_indices.get(index).copied();
                        let Some(config_index) = config_index else {
                            self.buffer.clear();
                            continue;
                        };
                        let (trigger, match_mode) = {
                            let expansion = &self.config.expansion[config_index];
                            (expansion.trigger.clone(), expansion.match_mode)
                        };
                        if self.matcher.has_continuation(&trigger)
                            || match_mode == MatchMode::WordBoundary
                        {
                            continue;
                        }
                        if !self.match_allowed(config_index, length) {
                            continue;
                        }
                        let Ok(insert) = self.render_expansion(config_index) else {
                            self.buffer.clear();
                            continue;
                        };
                        let expansion_bytes = trigger.len().saturating_add(insert.len());
                        if result_bytes.saturating_add(expansion_bytes) > MAX_RESULT_BYTES_PER_EVENT
                        {
                            self.buffer.clear();
                            break;
                        }
                        result_bytes = result_bytes.saturating_add(expansion_bytes);
                        results.push(ExpansionResult {
                            trigger,
                            erase_chars: length,
                            insert,
                        });
                        // Do not allow a replacement to combine with the
                        // next typed text and accidentally trigger again.
                        self.buffer.clear();
                    }
                }
                results
            }
            InputEvent::Backspace => {
                self.buffer.pop_back();
                Vec::new()
            }
            InputEvent::Boundary => {
                let result = self
                    .matcher
                    .find_suffix(&self.buffer.iter().collect::<String>())
                    .and_then(|(index, length)| {
                        self.matcher_indices
                            .get(index)
                            .copied()
                            .and_then(|config_index| self.take_match(config_index, length))
                    });
                self.buffer.clear();
                result.into_iter().collect()
            }
            InputEvent::FocusChanged { sensitive } => {
                self.capture_enabled = !sensitive;
                self.buffer.clear();
                Vec::new()
            }
        }
    }

    fn take_match(&mut self, config_index: usize, length: usize) -> Option<ExpansionResult> {
        if !self.match_allowed(config_index, length) {
            return None;
        }
        let trigger = self.config.expansion[config_index].trigger.clone();
        let insert = self.render_expansion(config_index).ok()?;
        for _ in 0..length {
            self.buffer.pop_back();
        }
        Some(ExpansionResult {
            trigger,
            erase_chars: length,
            insert,
        })
    }

    fn render_expansion(&mut self, config_index: usize) -> Result<String, ()> {
        let expansion = &self.config.expansion[config_index];
        let Some(command) = &expansion.command else {
            return render_template(&expansion.replacement, &TemplateContext::system())
                .map_err(|_| ());
        };
        if command.cache_ms > 0 {
            if let Some(entry) = self.command_cache[config_index].as_ref() {
                if entry.expires_at > Instant::now() {
                    return Ok(entry.value.clone());
                }
            }
        }
        let value = run_command(command)?;
        if command.cache_ms > 0 {
            self.command_cache[config_index] = Some(CommandCacheEntry {
                expires_at: Instant::now() + Duration::from_millis(command.cache_ms),
                value: value.clone(),
            });
        }
        Ok(value)
    }

    fn match_allowed(&self, config_index: usize, length: usize) -> bool {
        if self.config.expansion[config_index].match_mode != MatchMode::WordBoundary {
            return true;
        }
        let preceding = self.buffer.iter().rev().nth(length);
        preceding.is_none_or(|character| !is_word_character(*character))
    }

    /// Apply an expansion. Backends may perform the replacement atomically;
    /// simple backends use the safe erase-then-insert default.
    pub fn apply<I: TextInjector + ?Sized>(
        injector: &mut I,
        result: &ExpansionResult,
    ) -> Result<(), ExpansionError> {
        injector.replace(&result.trigger, &result.insert)?;
        Ok(())
    }
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn run_command(command: &CommandConfig) -> Result<String, ()> {
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ())?;
    let stdout = child.stdout.take().ok_or(())?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take((MAX_COMMAND_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });

    let deadline = Instant::now() + Duration::from_millis(command.timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(());
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(());
            }
        }
    };
    if !status.success() {
        return Err(());
    }
    let bytes = receiver
        .recv_timeout(Duration::from_millis(100))
        .map_err(|_| ())?
        .map_err(|_| ())?;
    if bytes.len() > MAX_COMMAND_OUTPUT_BYTES {
        return Err(());
    }
    let output = String::from_utf8(bytes).map_err(|_| ())?;
    Ok(output.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn engine() -> ExpansionEngine {
        ExpansionEngine::new(
            Config::parse(
                r#"
            [[expansion]]
            trigger = ":hello"
            replacement = "Hello from Wayland!"

            [[expansion]]
            trigger = ":cafe"
            replacement = "café ☕"
        "#,
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn expands_unicode_replacement() {
        let mut engine = engine();
        let result = engine
            .process(InputEvent::Text(":cafe".into()))
            .pop()
            .unwrap();
        assert_eq!(result.erase_chars, 5);
        assert_eq!(result.insert, "café ☕");
    }

    #[test]
    fn dispatches_enabled_hotkey_actions_without_side_effects() {
        let config = Config::parse(
            r#"
            [[hotkey]]
            chord = "Ctrl+Alt+M"
            description = "Open meeting helper"
            [hotkey.command]
            program = "/bin/true"
            timeout_ms = 100
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let chord = KeyChord::parse("control+option+m").unwrap();
        let result = engine.process_key(&chord);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chord.to_string(), "Ctrl+Alt+M");
        assert_eq!(result[0].description, "Open meeting helper");
        assert_eq!(result[0].command.program, "/bin/true");

        engine.process(InputEvent::FocusChanged { sensitive: true });
        assert!(engine.process_key(&chord).is_empty());
    }

    #[test]
    fn command_expansion_uses_direct_program_output() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":kernel"
            replacement = ""
            [expansion.command]
            program = "/bin/printf"
            args = ["kernel-6.1"]
            timeout_ms = 500
        "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let result = engine
            .process(InputEvent::Text(":kernel".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "kernel-6.1");
    }

    #[test]
    fn command_expansion_times_out_without_blocking_forever() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":slow"
            replacement = ""
            [expansion.command]
            program = "/bin/sleep"
            args = ["1"]
            timeout_ms = 10
        "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.process(InputEvent::Text(":slow".into())).is_empty());
    }

    #[test]
    fn command_expansion_cache_reuses_recent_output() {
        let counter =
            std::env::temp_dir().join(format!("wayexpand-command-cache-{}", std::process::id()));
        std::fs::write(&counter, "0").unwrap();
        let script = format!(
            "n=$(cat '{}'); n=$((n+1)); printf '%s' \"$n\" > '{}'; printf '%s' \"$n\"",
            counter.display(),
            counter.display()
        );
        let config = format!(
            "[[expansion]]\ntrigger = \":count\"\nreplacement = \"\"\n[expansion.command]\nprogram = \"/bin/sh\"\nargs = [\"-c\", {script:?}]\ncache_ms = 1000\n"
        );
        let mut engine = ExpansionEngine::new(Config::parse(&config).unwrap()).unwrap();
        let results = engine.process(InputEvent::Text(":count:count".into()));
        let _ = std::fs::remove_file(&counter);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].insert, "1");
        assert_eq!(results[1].insert, "1");
    }

    #[test]
    fn command_expansion_limits_are_validated_before_activation() {
        let config = r#"
            [[expansion]]
            trigger = ":bad-command"
            replacement = ""
            [expansion.command]
            program = "printf"
            timeout_ms = 5001
        "#;
        assert!(matches!(
            Config::parse(config),
            Err(ConfigError::InvalidCommand { .. })
        ));
    }

    #[test]
    fn malformed_config_is_an_error() {
        assert!(Config::parse("[[expansion]]\ntrigger = \":x\"\n").is_err());
    }

    #[test]
    fn engine_rejects_manually_constructed_invalid_config() {
        let config = Config {
            expansion: vec![crate::ExpansionConfig {
                trigger: String::new(),
                replacement: "value".into(),
                description: String::new(),
                tags: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
            }],
            hotkey: Vec::new(),
            settings: crate::Settings::default(),
        };
        assert!(matches!(
            ExpansionEngine::new(config),
            Err(crate::ConfigError::EmptyTrigger { index: 0 })
        ));
    }

    #[test]
    fn nul_characters_are_rejected() {
        let trigger = r#"[[expansion]]
trigger = "a\u0000b"
replacement = "ok""#;
        assert!(matches!(
            Config::parse(trigger),
            Err(crate::ConfigError::NulCharacter {
                field: "trigger",
                ..
            })
        ));
        let replacement = r#"[[expansion]]
trigger = ":x"
replacement = "bad\u0000value""#;
        assert!(matches!(
            Config::parse(replacement),
            Err(crate::ConfigError::NulCharacter {
                field: "replacement",
                ..
            })
        ));
    }

    #[test]
    fn processes_each_character_in_a_chunk() {
        let mut engine = engine();
        let results = engine.process(InputEvent::Text("prefix :hello suffix".into()));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].trigger, ":hello");
    }

    #[test]
    fn expansion_results_are_bounded_per_text_event() {
        let config =
            Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"").unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let results = engine.process(InputEvent::Text(":x".repeat(MAX_RESULTS_PER_EVENT + 1)));
        assert_eq!(results.len(), MAX_RESULTS_PER_EVENT);

        // Hitting the budget must not leave a partial trigger in the matcher.
        assert_eq!(engine.process(InputEvent::Text(":x".into())).len(), 1);
    }

    #[test]
    fn duplicate_triggers_are_rejected() {
        let config = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"a\"\n[[expansion]]\ntrigger = \":x\"\nreplacement = \"b\"";
        assert!(matches!(
            Config::parse(config),
            Err(crate::ConfigError::DuplicateTrigger { .. })
        ));
    }

    #[test]
    fn prefix_triggers_use_the_longest_match() {
        let config = "[[expansion]]\ntrigger = \":h\"\nreplacement = \"a\"\n[[expansion]]\ntrigger = \":hello\"\nreplacement = \"b\"";
        let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
        let result = engine.process(InputEvent::Text(":hello".into()));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].insert, "b");
    }

    #[test]
    fn short_prefix_waits_for_more_input_or_a_boundary() {
        let config = "[[expansion]]\ntrigger = \":h\"\nreplacement = \"short\"\n[[expansion]]\ntrigger = \":hello\"\nreplacement = \"long\"";
        let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
        assert!(engine.process(InputEvent::Text(":h".into())).is_empty());
        let result = engine.process(InputEvent::Text("x".into()));
        assert_eq!(result[0].insert, "short");

        assert!(engine.process(InputEvent::Text(":h".into())).is_empty());
        let result = engine.process(InputEvent::Boundary);
        assert_eq!(result[0].insert, "short");
    }

    #[test]
    fn word_boundary_mode_rejects_embedded_trigger() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.process(InputEvent::Text("x:sig".into())).is_empty());
        assert_eq!(
            engine.process(InputEvent::Text(" :sig ".into()))[0].insert,
            "signature"
        );
    }

    #[test]
    fn word_boundary_mode_is_unicode_aware() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.process(InputEvent::Text("é:sig".into())).is_empty());
        assert_eq!(
            engine.process(InputEvent::Text(" :sig ".into()))[0].insert,
            "signature"
        );
    }

    #[test]
    fn word_boundary_mode_waits_for_a_trailing_boundary() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.process(InputEvent::Text(":sig".into())).is_empty());
        assert!(engine.process(InputEvent::Text("X".into())).is_empty());
        assert!(engine.process(InputEvent::Text(" :sig".into())).is_empty());
        assert_eq!(engine.process(InputEvent::Boundary)[0].insert, "signature");
    }

    struct RecordingInjector {
        calls: Vec<String>,
    }

    impl crate::TextInjector for RecordingInjector {
        fn name(&self) -> &'static str {
            "test"
        }

        fn erase(&mut self, trigger: &str) -> Result<(), crate::InjectorError> {
            self.calls.push(format!("erase:{trigger}"));
            Ok(())
        }

        fn insert(&mut self, text: &str) -> Result<(), crate::InjectorError> {
            self.calls.push(format!("insert:{text}"));
            Ok(())
        }
    }

    #[test]
    fn apply_erases_before_inserting() {
        let result = ExpansionResult {
            trigger: ":x".into(),
            erase_chars: 2,
            insert: "value".into(),
        };
        let mut injector = RecordingInjector { calls: Vec::new() };
        ExpansionEngine::apply(&mut injector, &result).unwrap();
        assert_eq!(injector.calls, ["erase::x", "insert:value"]);
    }

    struct AtomicInjector {
        calls: Vec<String>,
    }

    impl crate::TextInjector for AtomicInjector {
        fn name(&self) -> &'static str {
            "atomic-test"
        }

        fn erase(&mut self, _: &str) -> Result<(), crate::InjectorError> {
            panic!("atomic backend should not use the default erase path")
        }

        fn insert(&mut self, _: &str) -> Result<(), crate::InjectorError> {
            panic!("atomic backend should not use the default insert path")
        }

        fn replace(&mut self, trigger: &str, text: &str) -> Result<(), crate::InjectorError> {
            self.calls.push(format!("replace:{trigger}:{text}"));
            Ok(())
        }
    }

    #[test]
    fn apply_uses_atomic_backend_operation_when_available() {
        let result = ExpansionResult {
            trigger: ":x".into(),
            erase_chars: 2,
            insert: "value".into(),
        };
        let mut injector = AtomicInjector { calls: Vec::new() };
        ExpansionEngine::apply(&mut injector, &result).unwrap();
        assert_eq!(injector.calls, ["replace::x:value"]);
    }

    #[test]
    fn sensitive_focus_disables_matching_and_clears_buffer() {
        let mut engine = engine();
        assert!(engine.process(InputEvent::Text(":hel".into())).is_empty());
        engine.process(InputEvent::FocusChanged { sensitive: true });
        assert!(engine.process(InputEvent::Text("lo".into())).is_empty());
        engine.process(InputEvent::FocusChanged { sensitive: false });
        assert!(engine.process(InputEvent::Text(":hello".into())).len() == 1);
    }

    #[test]
    fn disabled_expansions_do_not_match() {
        let config = "[[expansion]]\ntrigger = \":off\"\nreplacement = \"secret\"\nenabled = false";
        let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
        assert!(engine.process(InputEvent::Text(":off".into())).is_empty());
    }

    #[test]
    fn disabled_prefix_does_not_block_enabled_expansion() {
        let config = r#"
            [[expansion]]
            trigger = ":x"
            replacement = "disabled"
            enabled = false

            [[expansion]]
            trigger = ":xyz"
            replacement = "enabled"
        "#;
        let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
        let result = engine.process(InputEvent::Text(":xyz".into()));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].insert, "enabled");
    }

    #[test]
    fn disabled_duplicate_does_not_block_enabled_expansion() {
        let config = r#"
            [[expansion]]
            trigger = ":x"
            replacement = "disabled"
            enabled = false

            [[expansion]]
            trigger = ":x"
            replacement = "enabled"
        "#;
        let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
        let result = engine.process(InputEvent::Text(":x".into()));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].insert, "enabled");
    }

    #[test]
    fn unknown_config_fields_are_rejected() {
        let config = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\nwat = true";
        assert!(Config::parse(config).is_err());
    }

    #[test]
    fn invalid_template_is_rejected_before_activation() {
        let config = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"{{unknown}}\"";
        assert!(matches!(
            Config::parse(config),
            Err(crate::ConfigError::InvalidTemplate { .. })
        ));
    }

    #[test]
    fn oversized_replacements_are_rejected() {
        let config = format!(
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"{}\"",
            "a".repeat(1024 * 1024 + 1)
        );
        assert!(matches!(
            Config::parse(&config),
            Err(crate::ConfigError::ReplacementTooLarge { .. })
        ));
    }

    #[test]
    fn excessive_expansion_count_is_rejected() {
        let mut config = String::new();
        for index in 0..10_001 {
            config.push_str(&format!(
                "[[expansion]]\ntrigger = \":{index}\"\nreplacement = \"x\"\n"
            ));
        }
        assert!(matches!(
            Config::parse(&config),
            Err(crate::ConfigError::TooManyExpansions { .. })
        ));
    }

    #[test]
    fn oversized_configuration_is_rejected_before_parsing() {
        let config = "x".repeat(16 * 1024 * 1024 + 1);
        assert!(matches!(
            Config::parse(&config),
            Err(crate::ConfigError::ConfigTooLarge { .. })
        ));
    }

    #[test]
    fn non_regular_configuration_path_is_rejected() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-config-directory-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(crate::ConfigError::NotRegular { .. })
        ));
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn symlinked_configuration_validates_the_resolved_parent() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-config-symlink-{}", std::process::id()));
        let target = root.join("target.toml");
        let link = root.join("link.toml");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            &target,
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(Config::load(&link).is_ok());
        std::fs::remove_file(link).unwrap();
        std::fs::remove_file(target).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn fifo_configuration_path_is_rejected_without_blocking() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-config-fifo-{}", std::process::id()));
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &path,
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(crate::ConfigError::NotRegular { .. })
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn group_writable_configuration_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-config-permissions-{}",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(crate::ConfigError::InsecurePermissions { .. })
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn writable_non_sticky_configuration_parent_is_rejected() {
        let parent = std::env::temp_dir().join(format!(
            "wayexpand-config-parent-permissions-{}",
            std::process::id()
        ));
        let path = parent.join("expansions.toml");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(
            &path,
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(crate::ConfigError::InsecureParent { .. })
        ));
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn excessive_enabled_trigger_data_is_rejected() {
        let mut config = String::new();
        for index in 0..=crate::config::MAX_TOTAL_TRIGGER_CHARS / 128 {
            config.push_str(&format!(
                "[[expansion]]\ntrigger = \":{index:08x}{}\"\nreplacement = \"x\"\n",
                "a".repeat(119)
            ));
        }
        assert!(matches!(
            Config::parse(&config),
            Err(crate::ConfigError::TriggerDataTooLarge { .. })
        ));
    }

    #[test]
    fn buffer_limit_is_bounded() {
        let config = "[settings]\nmax_buffer_chars = 0";
        assert!(matches!(
            Config::parse(config),
            Err(crate::ConfigError::InvalidBufferLimit)
        ));
    }

    #[test]
    fn buffer_limit_can_be_configured() {
        let config = "[settings]\nmax_buffer_chars = 2\n[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"";
        let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
        assert_eq!(engine.process(InputEvent::Text("abc:x".into())).len(), 1);
    }
}
