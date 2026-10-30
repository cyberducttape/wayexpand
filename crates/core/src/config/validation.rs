use super::*;

impl Config {
    /// Validate a configuration assembled through the public Rust API.
    /// Parsing is not the only way callers can construct `Config`, so engine
    /// construction and other consumers can enforce the same limits here.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_metadata_with_snippets(None).map(drop)
    }

    /// Validate once and retain an immutable snapshot with the derived data
    /// used by runtime consumers.
    pub fn validate_snapshot(&self) -> Result<crate::ValidatedConfig, ConfigError> {
        self.validate_snapshot_with_snippets(None)
    }

    /// As [`Self::validate_snapshot`], using an external static library for
    /// template includes.
    pub fn validate_snapshot_with_snippets(
        &self,
        snippets: Option<std::sync::Arc<crate::SnippetLibrary>>,
    ) -> Result<crate::ValidatedConfig, ConfigError> {
        let metadata = self.validate_metadata_with_snippets(snippets)?;
        let config = Arc::new(self.clone());
        let compiled_expansions = config
            .expansion
            .iter()
            .map(|expansion| super::CompiledExpansionMetadata {
                id: Arc::from(expansion.id.as_str()),
                trigger: Arc::from(expansion.trigger.as_str()),
                replacement: Arc::from(expansion.replacement.as_str()),
                command: expansion.command.clone().map(Arc::new),
            })
            .collect();
        Ok(crate::ValidatedConfig {
            config,
            compiled_expansions,
            resolved_policy: self.organization.clone(),
            normalized_ids: self
                .expansion
                .iter()
                .map(|expansion| expansion.id.to_ascii_lowercase())
                .collect(),
            effective_triggers: metadata.effective_triggers,
            app_filters: metadata.app_filters,
            form_fields: metadata.form_fields,
            template_variables: metadata.template_variables,
            compiled_templates: metadata.compiled_templates,
            snippets: metadata.snippets,
            undo_chord: metadata.undo_chord,
            hotkeys: metadata.hotkeys,
        })
    }

    fn validate_metadata_with_snippets(
        &self,
        snippets: Option<std::sync::Arc<crate::SnippetLibrary>>,
    ) -> Result<ValidatedMetadata, ConfigError> {
        if self.expansion.len() > MAX_EXPANSIONS {
            return Err(ConfigError::TooManyExpansions {
                count: self.expansion.len(),
                maximum: MAX_EXPANSIONS,
            });
        }
        if !(1..=4096).contains(&self.settings.max_buffer_chars) {
            return Err(ConfigError::InvalidBufferLimit);
        }
        if !(1..=crate::config::MAX_KEYSTROKE_MAX_CHARS)
            .contains(&self.settings.keystroke_max_chars)
        {
            return Err(ConfigError::InvalidKeystrokeLimit {
                maximum: crate::config::MAX_KEYSTROKE_MAX_CHARS,
            });
        }
        let undo_chord = self
            .settings
            .undo_chord
            .as_deref()
            .map(KeyChord::parse)
            .transpose()
            .map_err(|_| ConfigError::InvalidUndoChord)?;
        if self.hotkey.len() > MAX_HOTKEYS {
            return Err(ConfigError::TooManyHotkeys {
                count: self.hotkey.len(),
                maximum: MAX_HOTKEYS,
            });
        }
        let mut hotkeys = Vec::new();
        let mut parsed_hotkeys = Vec::new();
        for (index, binding) in self.hotkey.iter().enumerate() {
            let chord =
                KeyChord::parse(&binding.chord).map_err(|_| ConfigError::InvalidHotkey {
                    index,
                    reason: "chord is empty, ambiguous, or contains an unknown modifier",
                })?;
            if binding.enabled && undo_chord.as_ref().is_some_and(|undo| undo == &chord) {
                return Err(ConfigError::UndoHotkeyCollision {
                    chord: chord.to_string(),
                    index,
                });
            }
            if binding.description.chars().count() > MAX_HOTKEY_DESCRIPTION_CHARS
                || binding.description.contains('\0')
            {
                return Err(ConfigError::InvalidHotkey {
                    index,
                    reason: "description is too long or contains NUL",
                });
            }
            if let Err(reason) = validate_command_config(&binding.command) {
                return Err(ConfigError::InvalidHotkey { index, reason });
            }
            if self
                .organization
                .command_path_is_blocked(&binding.command.program)
                && binding.command.action.is_none()
            {
                return Err(ConfigError::InvalidHotkey {
                    index,
                    reason: "command program must be an absolute path by organization policy",
                });
            }
            let mut argument_chars = 0usize;
            for argument in &binding.command.args {
                if argument.chars().count() > MAX_COMMAND_ARG_CHARS || argument.contains('\0') {
                    return Err(ConfigError::InvalidHotkey {
                        index,
                        reason: "command argument is too long or contains NUL",
                    });
                }
                argument_chars = argument_chars.saturating_add(argument.chars().count());
                if argument_chars > MAX_COMMAND_ARG_DATA_CHARS {
                    return Err(ConfigError::InvalidHotkey {
                        index,
                        reason: "command argument data is too large",
                    });
                }
            }
            if binding.enabled {
                hotkeys.push((index, chord.to_string()));
                parsed_hotkeys.push((chord, index));
            }
        }
        hotkeys.sort_unstable_by(|left, right| {
            left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
        });
        for pair in hotkeys.windows(2) {
            if pair[0].1 == pair[1].1 {
                return Err(ConfigError::DuplicateHotkey {
                    chord: pair[0].1.clone(),
                    first: pair[0].0,
                    second: pair[1].0,
                });
            }
        }
        if self.settings.template_env.len() > MAX_TEMPLATE_ENV {
            return Err(ConfigError::InvalidTemplateEnv {
                reason: "too many template_env names (maximum 32)",
            });
        }
        for name in &self.settings.template_env {
            let mut characters = name.chars();
            let valid = name.len() <= 256
                && characters
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && characters
                    .all(|character| character.is_ascii_alphanumeric() || character == '_');
            if !valid {
                return Err(ConfigError::InvalidTemplateEnv {
                    reason: "template_env names must be ASCII letters, digits, or _, not starting with a digit",
                });
            }
        }
        // An externally supplied library replaces the config's own, so only
        // build the latter when it is actually used.
        let snippets = snippets.unwrap_or_else(|| self.includable_snippets());
        let template_context = self.validation_template_context(Arc::clone(&snippets));
        let mut total_trigger_chars = 0usize;
        let mut expansion_ids = HashMap::with_capacity(self.expansion.len());
        let mut app_filters = Vec::with_capacity(self.expansion.len());
        let mut validated_form_fields = Vec::with_capacity(self.expansion.len());
        let mut template_variables = Vec::with_capacity(self.expansion.len());
        let mut compiled_templates = Vec::with_capacity(self.expansion.len());
        for (index, expansion) in self.expansion.iter().enumerate() {
            if !is_uuid(&expansion.id) {
                return Err(ConfigError::InvalidExpansionId { index });
            }
            if let Some(first) = expansion_ids.insert(expansion.id.to_ascii_lowercase(), index) {
                return Err(ConfigError::DuplicateExpansionId {
                    first,
                    second: index,
                });
            }
            if expansion.trigger.is_empty() {
                return Err(ConfigError::EmptyTrigger { index });
            }
            if expansion.trigger.contains('\0') {
                return Err(ConfigError::NulCharacter {
                    index,
                    field: "trigger",
                });
            }
            if expansion.replacement.contains('\0') {
                return Err(ConfigError::NulCharacter {
                    index,
                    field: "replacement",
                });
            }
            let mut trigger_length = expansion.trigger.chars().count();
            if trigger_length > MAX_TRIGGER_CHARS {
                return Err(ConfigError::TriggerTooLong {
                    index,
                    length: trigger_length,
                    maximum: MAX_TRIGGER_CHARS,
                });
            }
            if expansion.aliases.len() > MAX_ALIASES {
                return Err(ConfigError::TooManyAliases {
                    index,
                    maximum: MAX_ALIASES,
                });
            }
            for alias in &expansion.aliases {
                if alias.is_empty() {
                    return Err(ConfigError::EmptyTrigger { index });
                }
                if alias.contains('\0') {
                    return Err(ConfigError::NulCharacter {
                        index,
                        field: "alias",
                    });
                }
                let length = alias.chars().count();
                if length > MAX_TRIGGER_CHARS {
                    return Err(ConfigError::TriggerTooLong {
                        index,
                        length,
                        maximum: MAX_TRIGGER_CHARS,
                    });
                }
                trigger_length = trigger_length.saturating_add(length);
            }
            if expansion.replacement.len() > MAX_REPLACEMENT_BYTES {
                return Err(ConfigError::ReplacementTooLarge {
                    index,
                    length: expansion.replacement.len(),
                    maximum: MAX_REPLACEMENT_BYTES,
                });
            }
            if expansion.description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(ConfigError::DescriptionTooLong {
                    index,
                    maximum: MAX_DESCRIPTION_CHARS,
                });
            }
            if expansion.description.contains('\0') {
                return Err(ConfigError::NulCharacter {
                    index,
                    field: "description",
                });
            }
            if expansion.tags.len() > MAX_TAGS
                || expansion
                    .tags
                    .iter()
                    .any(|tag| tag.chars().count() > MAX_TAG_CHARS || tag.contains('\0'))
            {
                return Err(ConfigError::InvalidTags { index });
            }
            if expansion.app_filter.len() > MAX_APP_FILTERS
                || expansion.app_filter.iter().any(|filter| {
                    filter.trim().is_empty()
                        || filter.chars().count() > MAX_APP_FILTER_CHARS
                        || filter.contains('\0')
                })
            {
                return Err(ConfigError::InvalidAppFilter { index });
            }
            let parsed_filters: Vec<AppFilter> = expansion
                .app_filter
                .iter()
                .filter_map(|filter| AppFilter::parse(filter))
                .collect();
            if parsed_filters.len() != expansion.app_filter.len() {
                return Err(ConfigError::InvalidAppFilter { index });
            }
            if self.organization.safe_mode
                && !self.organization.allow_weak_app_filters
                && parsed_filters.iter().any(AppFilter::is_weak)
            {
                return Err(ConfigError::InvalidAppFilter { index });
            }
            app_filters.push(parsed_filters);
            if expansion.category.chars().count() > MAX_CATEGORY_CHARS
                || expansion.category.contains('\0')
            {
                return Err(ConfigError::InvalidCategory { index });
            }
            if let Some(output_file) = &expansion.output_file {
                validate_output_file(output_file, expansion.command.is_some())
                    .map_err(|reason| ConfigError::InvalidOutputFile { index, reason })?;
            }
            if let Some(command) = &expansion.command {
                if let Err(reason) = validate_command_config(command) {
                    return Err(ConfigError::InvalidCommand { index, reason });
                }
                if command.action.is_none()
                    && self.organization.command_path_is_blocked(&command.program)
                {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "program must be an absolute path by organization policy",
                    });
                }
            }
            compiled_templates.push(if expansion.command.is_some() {
                None
            } else {
                let compiled = crate::CompiledTemplate::compile(&expansion.replacement)
                    .map_err(|source| ConfigError::InvalidTemplate { index, source })?;
                compiled
                    .render_with_cursor(&template_context)
                    .map_err(|source| ConfigError::InvalidTemplate { index, source })?;
                Some(Arc::new(compiled))
            });
            template_variables.push(
                crate::template_variables(&expansion.replacement)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            );
            let parsed_form_fields = if let Some(command) = &expansion.command {
                match crate::form_fields(&expansion.replacement) {
                    Ok(fields) if !fields.is_empty() => {
                        if command.action.is_none() {
                            return Err(ConfigError::InvalidTemplate {
                                index,
                                source: TemplateError::InvalidField,
                            });
                        }
                        let mut parameter_names = std::collections::HashSet::new();
                        if fields.iter().any(|field| {
                            !field.required
                                || !form_action_parameter_name(&field.key).is_some_and(|name| {
                                    !name.is_empty()
                                        && name.len() <= 32
                                        && name.as_bytes()[0].is_ascii_lowercase()
                                        && name.bytes().all(|byte| {
                                            byte.is_ascii_lowercase()
                                                || byte.is_ascii_digit()
                                                || byte == b'_'
                                        })
                                        && parameter_names.insert(name)
                                })
                        }) {
                            return Err(ConfigError::InvalidTemplate {
                                index,
                                source: TemplateError::InvalidField,
                            });
                        }
                        Some(Arc::new(fields))
                    }
                    _ => None,
                }
            } else {
                let fields = crate::form_fields(&expansion.replacement)
                    .map_err(|source| ConfigError::InvalidTemplate { index, source })?;
                (!fields.is_empty()).then(|| Arc::new(fields))
            };
            validated_form_fields.push(parsed_form_fields);
            if expansion.enabled {
                total_trigger_chars = total_trigger_chars.saturating_add(trigger_length);
                if total_trigger_chars > MAX_TOTAL_TRIGGER_CHARS {
                    return Err(ConfigError::TriggerDataTooLarge {
                        length: total_trigger_chars,
                        maximum: MAX_TOTAL_TRIGGER_CHARS,
                    });
                }
            }
        }

        // Sorting makes duplicate validation O(n log n) instead of comparing
        // every enabled expansion with every other expansion. Prefixes are
        // intentionally allowed; the matcher selects the longest suffix.
        //
        // This checks *effective* triggers (literal trigger, plus any
        // propagate_case-generated variants), not just the literal
        // `trigger` field: the matcher is built from effective triggers
        // (see `ExpansionEngine::new`), so two expansions with distinct
        // configured triggers can still collide once case variants are
        // generated -- and without this, that collision would silently
        // make one expansion unreachable instead of failing validation.
        let mut enabled: Vec<(usize, String)> = self
            .expansion
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.enabled)
            .flat_map(|(index, entry)| {
                entry
                    .effective_triggers()
                    .into_iter()
                    .map(move |trigger| (index, trigger))
            })
            .collect();
        let effective_scalars = enabled
            .iter()
            .map(|(_, trigger)| trigger.chars().count())
            .sum::<usize>();
        if effective_scalars > MAX_EFFECTIVE_TRIGGER_SCALARS
            || enabled.len() > MAX_EFFECTIVE_TRIGGERS
        {
            return Err(ConfigError::EffectiveTriggerDataTooLarge {
                scalars: effective_scalars,
                maximum_scalars: MAX_EFFECTIVE_TRIGGER_SCALARS,
                triggers: enabled.len(),
                maximum_triggers: MAX_EFFECTIVE_TRIGGERS,
            });
        }
        enabled.sort_unstable_by(|left, right| {
            left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
        });
        for pair in enabled.windows(2) {
            let (first_index, first) = &pair[0];
            let (second_index, second) = &pair[1];
            if first == second {
                return Err(ConfigError::DuplicateTrigger {
                    trigger: first.clone(),
                    first: *first_index,
                    second: *second_index,
                });
            }
        }

        // Validate organization policies
        if self.organization.max_replacement_size > 0
            && self.organization.max_replacement_size < 256
        {
            return Err(ConfigError::InvalidPolicyConfig(
                "max_replacement_size must be 0 (unlimited) or at least 256 bytes".to_string(),
            ));
        }

        // Validate that safe_mode doesn't ban all backends
        if self.organization.safe_mode
            && !self.organization.allowed_backends.is_empty()
            && self
                .organization
                .allowed_backends
                .iter()
                .all(|b| b == "none")
        {
            return Err(ConfigError::InvalidPolicyConfig(
                "safe_mode with allowed_backends=['none'] would prevent all expansions".to_string(),
            ));
        }

        Ok(ValidatedMetadata {
            effective_triggers: enabled,
            app_filters,
            form_fields: validated_form_fields,
            template_variables,
            compiled_templates,
            snippets,
            undo_chord,
            hotkeys: parsed_hotkeys,
        })
    }

    /// Apply an active administrator-owned policy and validate the resulting
    /// configuration. User-embedded policy remains available when no external
    /// policy is active; an active external policy is authoritative.
    pub fn apply_administrator_policy(
        &mut self,
        policy: &OrganizationPolicy,
    ) -> Result<(), ConfigError> {
        if policy.is_active() {
            // Validate the administrator input before projecting audit-only
            // values away. Invalid policy files must remain fatal even when
            // safe_mode is disabled.
            let mut candidate = self.clone();
            candidate.organization = policy.clone();
            candidate.validate()?;
            self.organization = policy.effective_enforcement_policy();
        }
        self.validate()
    }
}

fn form_action_parameter_name(key: &str) -> Option<&str> {
    let (kind, name) = key.split_once(':')?;
    matches!(kind, "field" | "date" | "choice").then_some(name)
}

/// `output_file` is where `wayexpand new` writes a snippet: a relative path
/// that stays below the current directory. It is not a template.
fn validate_output_file(output_file: &str, command_backed: bool) -> Result<(), &'static str> {
    use std::path::{Component, Path};
    if command_backed {
        return Err("command-backed snippets cannot be written to a file");
    }
    if output_file.is_empty() || output_file.len() > MAX_OUTPUT_FILE_BYTES {
        return Err("it must be 1 to 255 bytes");
    }
    if output_file.chars().any(char::is_control) {
        return Err("it contains a control character");
    }
    if output_file.ends_with('/') {
        return Err("it names a directory");
    }
    let path = Path::new(output_file);
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        || path.file_name().is_none()
    {
        return Err("it must be a relative path without '..'");
    }
    Ok(())
}

/// Bytes in an `output_file` path.
const MAX_OUTPUT_FILE_BYTES: usize = 255;
