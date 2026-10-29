use std::{
    collections::{BTreeMap, HashMap},
    env,
    sync::{Arc, OnceLock},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

/// How deeply `{{snippet:...}}` includes may nest.
use crate::limits::MAX_RENDERED_BYTES;

const MAX_INCLUDE_DEPTH: usize = 8;
static SYSTEM_USERNAME: OnceLock<String> = OnceLock::new();

/// Reads the clipboard for `{{clipboard}}`. Called only while rendering a
/// snippet that uses the variable; `None` means it could not be read.
#[derive(Clone)]
pub struct ClipboardReader(pub Arc<dyn Fn() -> Option<String> + Send + Sync>);

impl std::fmt::Debug for ClipboardReader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ClipboardReader")
    }
}

/// Asks the host to start reading the clipboard ahead of time because the
/// typed text can only be completed into a `{{clipboard}}` snippet. The host
/// must keep the value only for that imminent render (see the daemon).
#[derive(Clone)]
pub struct ClipboardPrefetch(pub Arc<dyn Fn() + Send + Sync>);

impl std::fmt::Debug for ClipboardPrefetch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ClipboardPrefetch")
    }
}

/// Static snippet replacements for `{{snippet:TRIGGER}}`, keyed by trigger
/// and alias. All keys of one snippet share a single `Arc<str>`, so aliases
/// cost a pointer rather than another copy of a (possibly large) replacement.
pub type SnippetLibrary = HashMap<String, Arc<str>>;

/// Values a template may read. Variables have different trust levels: the
/// built-ins are always available, `{{env:NAME}}` only reads names the user
/// allowlisted, `{{snippet:...}}` only includes other static snippets, and
/// `{{clipboard}}` only works when the user enabled it and policy allows it.
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub username: String,
    pub hostname: String,
    pub unix_timestamp: u64,
    /// Allowlisted environment variables and their values (empty if unset).
    pub env: Arc<BTreeMap<String, String>>,
    /// Static snippet replacements available to `{{snippet:TRIGGER}}`,
    /// keyed by trigger and alias.
    pub snippets: Arc<SnippetLibrary>,
    /// Present only when the clipboard variable is enabled.
    pub clipboard: Option<ClipboardReader>,
    /// Validation renders: the clipboard is never read and reports as empty.
    pub validating: bool,
    /// Values the user entered in a snippet form, keyed by field key (see
    /// [`FormField::key`]).
    pub fields: Arc<HashMap<String, String>>,
}

/// A value the user fills in when a form snippet expands.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FormField {
    /// Identifies the value; repeated markers with the same key share it.
    pub key: String,
    /// Shown next to the input.
    pub label: String,
    pub kind: FormFieldKind,
    /// Reject empty or whitespace-only submissions.
    #[serde(default)]
    pub required: bool,
    /// Text fields may impose a character-count lower bound.
    #[serde(default)]
    pub min_chars: Option<usize>,
    /// Text fields may impose a character-count upper bound.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldKind {
    Text {
        default: String,
    },
    Choice {
        options: Vec<String>,
        #[serde(default)]
        default: Option<String>,
    },
    /// An ISO 8601 calendar date (`YYYY-MM-DD`) chosen with a date picker.
    Date {
        default: String,
    },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FormValueError {
    #[error("a value is required")]
    Required,
    #[error("value is shorter than {minimum} characters")]
    TooShort { minimum: usize },
    #[error("value is longer than {maximum} characters")]
    TooLong { maximum: usize },
    #[error("value is not one of the available choices")]
    InvalidChoice,
    #[error("date must be a real calendar date in YYYY-MM-DD format")]
    InvalidDate,
    #[error("value contains a control character")]
    ControlCharacter,
    #[error("value exceeds the form size limit")]
    TooLarge,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DateError {
    #[error("the current instant cannot be represented as a local calendar date")]
    Unrepresentable,
}

/// Resolve the form marker's `today` default using the system-local date.
pub fn today_local_date() -> Result<String, DateError> {
    local_date_at(TemplateContext::system().unix_timestamp)
}

fn local_date_at(timestamp: u64) -> Result<String, DateError> {
    format_clock("local_date", timestamp).ok_or(DateError::Unrepresentable)
}

impl FormField {
    /// Apply the same value rules in the GUI, CLI, renderer, and daemon.
    pub fn validate_value(&self, value: &str) -> Result<(), FormValueError> {
        if value.len() > crate::limits::MAX_FORM_VALUE_BYTES {
            return Err(FormValueError::TooLarge);
        }
        if value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
        {
            return Err(FormValueError::ControlCharacter);
        }
        if self.required && value.trim().is_empty() {
            return Err(FormValueError::Required);
        }
        match &self.kind {
            FormFieldKind::Text { .. } => {
                let count = value.chars().count();
                if self.min_chars.is_some_and(|minimum| count < minimum) {
                    return Err(FormValueError::TooShort {
                        minimum: self.min_chars.unwrap_or_default(),
                    });
                }
                if self.max_chars.is_some_and(|maximum| count > maximum) {
                    return Err(FormValueError::TooLong {
                        maximum: self.max_chars.unwrap_or_default(),
                    });
                }
            }
            FormFieldKind::Choice { options, .. } => {
                if !options.iter().any(|option| option == value) {
                    return Err(FormValueError::InvalidChoice);
                }
            }
            FormFieldKind::Date { .. } => {
                if !valid_iso_date(value) {
                    return Err(FormValueError::InvalidDate);
                }
            }
        }
        Ok(())
    }
}

fn valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u32>().ok();
    let month = value[5..7].parse::<u32>().ok();
    let day = value[8..10].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day)) = (year, month, day) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

fn parse_form_attributes(
    attributes: &[&str],
    allow_length: bool,
) -> Result<(bool, Option<usize>, Option<usize>), TemplateError> {
    let mut required = false;
    let mut min_chars = None;
    let mut max_chars = None;
    for attribute in attributes {
        let attribute = attribute.trim();
        if attribute == "required" && !required {
            required = true;
        } else if let Some(value) = attribute.strip_prefix("min=") {
            if !allow_length || min_chars.is_some() {
                return Err(TemplateError::InvalidField);
            }
            min_chars = Some(parse_field_length(value)?);
        } else if let Some(value) = attribute.strip_prefix("max=") {
            if !allow_length || max_chars.is_some() {
                return Err(TemplateError::InvalidField);
            }
            max_chars = Some(parse_field_length(value)?);
        } else {
            return Err(TemplateError::InvalidField);
        }
    }
    if min_chars.zip(max_chars).is_some_and(|(min, max)| min > max) {
        return Err(TemplateError::InvalidField);
    }
    Ok((required, min_chars, max_chars))
}

fn parse_choice_attributes(attributes: &[&str]) -> Result<(bool, Option<String>), TemplateError> {
    let mut required = false;
    let mut default = None;
    for attribute in attributes {
        let attribute = attribute.trim();
        if attribute == "required" && !required {
            required = true;
        } else if let Some(value) = attribute.strip_prefix("default=") {
            if default.is_some() || value.is_empty() {
                return Err(TemplateError::InvalidField);
            }
            default = Some(value.to_owned());
        } else {
            return Err(TemplateError::InvalidField);
        }
    }
    Ok((required, default))
}

fn parse_field_length(value: &str) -> Result<usize, TemplateError> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| TemplateError::InvalidField)?;
    if parsed == 0 || parsed > 65_536 {
        return Err(TemplateError::InvalidField);
    }
    Ok(parsed)
}

/// Parse a form marker. Returns `None` for other template variables.
fn parse_form_field(name: &str) -> Option<Result<FormField, TemplateError>> {
    if let Some(spec) = name
        .strip_prefix("field:")
        .or_else(|| name.strip_prefix("prompt:"))
    {
        let mut parts = spec.split('|');
        let definition = parts.next().unwrap_or_default();
        let attributes = parts.collect::<Vec<_>>();
        let (label, default) = definition.split_once('=').unwrap_or((definition, ""));
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 64 {
            return Some(Err(TemplateError::InvalidField));
        }
        let validation = parse_form_attributes(&attributes, true);
        let (required, min_chars, max_chars) = match validation {
            Ok(validation) => validation,
            Err(error) => return Some(Err(error)),
        };
        return Some(Ok(FormField {
            key: format!("field:{label}"),
            label: label.to_owned(),
            kind: FormFieldKind::Text {
                default: default.to_owned(),
            },
            required,
            min_chars,
            max_chars,
        }));
    }
    if let Some(spec) = name.strip_prefix("datefield:") {
        let mut parts = spec.split('|');
        let definition = parts.next().unwrap_or_default();
        let attributes = parts.collect::<Vec<_>>();
        let (label, default) = definition.split_once('=').unwrap_or((definition, "today"));
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 64 {
            return Some(Err(TemplateError::InvalidField));
        }
        let (required, min_chars, max_chars) = match parse_form_attributes(&attributes, false) {
            Ok(validation) => validation,
            Err(error) => return Some(Err(error)),
        };
        if default != "today" && !valid_iso_date(default) {
            return Some(Err(TemplateError::InvalidField));
        }
        return Some(Ok(FormField {
            key: format!("date:{label}"),
            label: label.to_owned(),
            kind: FormFieldKind::Date {
                default: default.to_owned(),
            },
            required,
            min_chars,
            max_chars,
        }));
    }
    let spec = name.strip_prefix("choice:")?;
    let (spec, attributes) = match spec.split_once(';') {
        Some((spec, modifiers)) => (spec, modifiers.split(';').collect::<Vec<_>>()),
        None => (spec, Vec::new()),
    };
    let (required, preferred_default) = match parse_choice_attributes(&attributes) {
        Ok(validation) => validation,
        Err(error) => return Some(Err(error)),
    };
    // `choice:label=A|B` names the drop-down; the `=` must come before the
    // first option separator.
    let (label, spec) = match spec.split_once('=') {
        Some((label, rest)) if !label.contains('|') => (Some(label.trim()), rest),
        _ => (None, spec),
    };
    let options: Vec<String> = spec
        .split('|')
        .map(|option| option.trim().to_owned())
        .collect();
    if options.len() < 2 || options.len() > 32 || options.iter().any(String::is_empty) {
        return Some(Err(TemplateError::InvalidField));
    }
    if preferred_default
        .as_ref()
        .is_some_and(|value| !options.contains(value))
    {
        return Some(Err(TemplateError::InvalidField));
    }
    let (key, label) = match label {
        Some(label) if label.is_empty() || label.chars().count() > 64 => {
            return Some(Err(TemplateError::InvalidField));
        }
        Some(label) => (format!("choice:{label}"), label.to_owned()),
        None => (format!("choice:{}", options.join("|")), "Choice".to_owned()),
    };
    Some(Ok(FormField {
        key,
        label,
        kind: FormFieldKind::Choice {
            options,
            default: preferred_default,
        },
        required,
        min_chars: None,
        max_chars: None,
    }))
}

/// The form fields a template asks for, in order of first appearance and
/// without duplicates. An invalid marker is an error.
pub fn form_fields(template: &str) -> Result<Vec<FormField>, TemplateError> {
    let mut fields: Vec<FormField> = Vec::new();
    // Whether each field so far comes only from bare references.
    let mut bare_only: Vec<bool> = Vec::new();
    for name in template_variables(template) {
        if let Some(field) = parse_form_field(name) {
            let field = field?;
            let bare = is_bare_form_field_reference(name);
            match fields.iter().position(|existing| existing.key == field.key) {
                None => {
                    fields.push(field);
                    bare_only.push(bare);
                }
                // A bare marker reuses the definition of a named field,
                // wherever that definition appears. Packs and templates
                // commonly declare a default once and interpolate the same
                // value elsewhere without repeating its rules.
                Some(_) if bare => {}
                Some(index) if bare_only[index] => {
                    fields[index] = field;
                    bare_only[index] = false;
                }
                // A repeated key is one input only when all of its rules and
                // explicit defaults agree; otherwise the prompt is ambiguous.
                Some(index) if fields[index] != field => {
                    return Err(TemplateError::InvalidField);
                }
                Some(_) => {}
            }
        }
    }
    if fields.len() > 32 {
        return Err(TemplateError::InvalidField);
    }
    Ok(fields)
}

fn is_bare_form_field_reference(name: &str) -> bool {
    ["field:", "prompt:", "datefield:"]
        .iter()
        .filter_map(|prefix| name.strip_prefix(prefix))
        .any(|definition| !definition.contains('=') && !definition.contains('|'))
}

impl TemplateContext {
    /// Build the deliberately small, deterministic set of built-in values.
    /// External commands are not executed while rendering a snippet.
    pub fn system() -> Self {
        let username = SYSTEM_USERNAME
            .get_or_init(|| env::var("USER").unwrap_or_default())
            .clone();
        let hostname = current_hostname();
        let unix_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        Self {
            username,
            hostname,
            unix_timestamp,
            ..Self::default()
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TemplateError {
    #[error("template has an unclosed '{{{{' at byte {offset}")]
    Unclosed { offset: usize },
    #[error("template contains an empty variable at byte {offset}")]
    EmptyVariable { offset: usize },
    #[error("unknown template variable {name:?}")]
    UnknownVariable { name: String },
    #[error("rendered template exceeds {maximum} bytes")]
    RenderedTooLarge { maximum: usize },
    #[error("date arithmetic overflow for template variable {name:?}")]
    DateArithmeticOverflow { name: String },
    #[error("template has a second {{{{cursor}}}} marker at byte {offset}; only one is supported")]
    MultipleCursors { offset: usize },
    #[error("environment variable {name:?} is not in settings.template_env")]
    EnvNotAllowed { name: String },
    #[error("{{{{snippet:...}}}} names no static snippet")]
    UnknownSnippet,
    #[error(
        "{{{{snippet:...}}}} includes form a cycle or nest more than {MAX_INCLUDE_DEPTH} deep"
    )]
    IncludeTooDeep,
    #[error("an included snippet cannot contain {{{{cursor}}}}")]
    CursorInInclude,
    #[error(
        "the clipboard variable is disabled (settings.allow_clipboard, settings.clipboard_mode, or organization policy)"
    )]
    ClipboardDisabled,
    #[error("the clipboard could not be read")]
    ClipboardUnavailable,
    #[error(
        "a form field is malformed (see docs/TEMPLATES.md for field, choice, and datefield syntax)"
    )]
    InvalidField,
    #[error("a form field has no value")]
    FieldValueMissing,
    #[error("date format {format:?} is not supported (see docs/TEMPLATES.md)")]
    InvalidDateFormat { format: String },
    #[error(
        "{{{{random:{spec}}}}} is malformed (random:hex:16, random:alnum:24, or random:A|B|C)"
    )]
    InvalidRandom { spec: String },
    #[error("{{{{uuid:NAME}}}} needs a name of 1 to 64 characters")]
    InvalidUuidName,
    #[error("the system random number generator is unavailable")]
    RandomUnavailable,
}

impl TemplateError {
    /// Describe template failures without echoing user-provided variable
    /// names, formats, or random-choice values into diagnostics.
    pub fn safe_summary(&self) -> &'static str {
        match self {
            Self::UnknownVariable { .. } => "unknown template variable",
            Self::DateArithmeticOverflow { .. } => "date arithmetic overflow",
            Self::EnvNotAllowed { .. } => "environment variable is not allowlisted",
            Self::InvalidDateFormat { .. } => "unsupported date format",
            Self::InvalidRandom { .. } => "malformed random variable",
            Self::Unclosed { .. } => "unclosed template variable",
            Self::EmptyVariable { .. } => "empty template variable",
            Self::RenderedTooLarge { .. } => "rendered template exceeds the size limit",
            Self::MultipleCursors { .. } => "template has more than one cursor marker",
            Self::UnknownSnippet => "included snippet does not exist",
            Self::IncludeTooDeep => "snippet includes are cyclic or too deeply nested",
            Self::CursorInInclude => "included snippets cannot contain a cursor marker",
            Self::ClipboardDisabled => "clipboard variable is disabled",
            Self::ClipboardUnavailable => "clipboard is unavailable",
            Self::InvalidField => "form field is malformed",
            Self::FieldValueMissing => "form field value is missing",
            Self::InvalidUuidName => "named UUID variable is malformed",
            Self::RandomUnavailable => "system random number generator is unavailable",
        }
    }
}

/// Render built-in variables without invoking a shell or external process.
/// Unknown variables are errors so a typo can never silently reach an editor.
pub fn render_template(template: &str, context: &TemplateContext) -> Result<String, TemplateError> {
    render(template, context, false, MAX_RENDERED_BYTES).map(|(rendered, _)| rendered)
}

/// Render a deliberately bounded preview. This is for interactive UIs only:
/// rendering stops at the first output newline or byte budget, including in
/// included snippets, so a preview cannot materialize a full replacement.
pub fn render_template_preview(
    template: &str,
    context: &TemplateContext,
    maximum_bytes: usize,
) -> Result<String, TemplateError> {
    render_nested(
        template,
        context,
        true,
        0,
        maximum_bytes,
        true,
        &mut HashMap::new(),
    )
    .map(|(rendered, _, _)| rendered)
}

/// Every `{{...}}` variable name in a template, in order. Unclosed markers
/// end the scan; rendering reports them as errors. A doubled `{{{{` is a
/// literal `{{`, not a marker.
pub fn template_variables(template: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut cursor = 0;
    while let Some(start) = template[cursor..].find("{{") {
        if template[cursor + start..].starts_with(LITERAL_OPEN) {
            cursor += start + LITERAL_OPEN.len();
            continue;
        }
        let variable_start = cursor + start + 2;
        let Some(end) = template[variable_start..].find("}}") else {
            break;
        };
        names.push(template[variable_start..variable_start + end].trim());
        cursor = variable_start + end + 2;
    }
    names
}

/// A doubled `{{{{` renders as a literal `{{`, and a doubled `}}}}` outside a
/// marker as a literal `}}`, so Jinja, Go and Helm templates (Ansible
/// playbooks, Compose files, charts) can be written as snippets.
const LITERAL_OPEN: &str = "{{{{";
const LITERAL_CLOSE: &str = "}}}}";

/// Literal template text with doubled closing braces collapsed.
fn literal_text(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains(LITERAL_CLOSE) {
        std::borrow::Cow::Owned(text.replace(LITERAL_CLOSE, "}}"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Shared renderer. With `allow_cursor`, a `{{cursor}}` variable renders to
/// nothing and its byte position in the output is returned; otherwise it is
/// an unknown variable like any other.
fn render(
    template: &str,
    context: &TemplateContext,
    allow_cursor: bool,
    maximum_bytes: usize,
) -> Result<(String, Option<usize>), TemplateError> {
    render_nested(
        template,
        context,
        allow_cursor,
        0,
        maximum_bytes,
        false,
        &mut HashMap::new(),
    )
    .map(|(rendered, cursor, _)| (rendered, cursor))
}

fn render_nested(
    template: &str,
    context: &TemplateContext,
    allow_cursor: bool,
    depth: usize,
    maximum_bytes: usize,
    preview: bool,
    // `{{uuid:NAME}}` values, shared with included snippets so one name is
    // one value throughout a render.
    named: &mut HashMap<String, String>,
) -> Result<(String, Option<usize>, bool), TemplateError> {
    // Preview callers may pass an arbitrarily large valid template. Do not
    // reserve its full size before the output limit has a chance to reject it.
    let mut rendered = String::with_capacity(template.len().min(maximum_bytes));
    let mut cursor_position = None;
    let mut cursor = 0;
    while cursor < template.len() {
        if preview && rendered.len() >= maximum_bytes {
            return Ok((rendered, cursor_position, true));
        }
        let remaining_template = &template[cursor..];
        let remaining_output = maximum_bytes.saturating_sub(rendered.len());
        let mut inspect_len = if preview {
            remaining_template
                .len()
                .min(remaining_output.saturating_add(1))
        } else {
            remaining_template.len()
        };
        while !remaining_template.is_char_boundary(inspect_len) {
            inspect_len -= 1;
        }
        let inspected = &remaining_template[..inspect_len];
        let relative_start = inspected.find("{{");
        let relative_newline = inspected.find('\n');

        // A literal newline before any variable ends a preview immediately;
        // do not search or parse the potentially enormous tail that follows.
        if preview {
            if let Some(newline) = relative_newline
                .filter(|newline| relative_start.is_none_or(|variable| *newline < variable))
            {
                rendered.push_str(&literal_text(&remaining_template[..newline]));
                return Ok((rendered, cursor_position, true));
            }
        }

        let Some(relative_start) =
            relative_start.filter(|start| !preview || *start < remaining_output)
        else {
            let stopped = push_bounded(
                &mut rendered,
                &literal_text(remaining_template),
                maximum_bytes,
                preview,
            )?;
            if stopped {
                return Ok((rendered, cursor_position, true));
            }
            break;
        };
        let start = cursor + relative_start;
        let stopped = push_bounded(
            &mut rendered,
            &literal_text(&template[cursor..start]),
            maximum_bytes,
            preview,
        )?;
        if stopped {
            return Ok((rendered, cursor_position, true));
        }
        if template[start..].starts_with(LITERAL_OPEN) {
            if push_bounded(&mut rendered, "{{", maximum_bytes, preview)? {
                return Ok((rendered, cursor_position, true));
            }
            cursor = start + LITERAL_OPEN.len();
            continue;
        }
        let variable_start = start + 2;
        let Some(relative_end) = template[variable_start..].find("}}") else {
            return Err(TemplateError::Unclosed { offset: start });
        };
        let end = variable_start + relative_end;
        let name = template[variable_start..end].trim();
        if name.is_empty() {
            return Err(TemplateError::EmptyVariable { offset: start });
        }
        if allow_cursor && name == "cursor" {
            if cursor_position.is_some() {
                return Err(TemplateError::MultipleCursors { offset: start });
            }
            cursor_position = Some(rendered.len());
            cursor = end + 2;
            continue;
        }
        let mut included_stopped = false;
        let value = match name {
            "date" | "time" | "datetime" | "local_date" | "local_time" | "local_datetime" => {
                format_clock(name, context.unix_timestamp).ok_or_else(|| {
                    TemplateError::DateArithmeticOverflow {
                        name: name.to_owned(),
                    }
                })?
            }
            "username" => context.username.as_str().to_owned(),
            "hostname" => context.hostname.as_str().to_owned(),
            "unix_timestamp" => context.unix_timestamp.to_string(),
            "newline" => "\n".to_owned(),
            "tab" => "\t".to_owned(),
            "clipboard" if preview => return Err(TemplateError::ClipboardDisabled),
            "clipboard" => match &context.clipboard {
                None => return Err(TemplateError::ClipboardDisabled),
                Some(_) if context.validating => String::new(),
                Some(reader) => (reader.0)().ok_or(TemplateError::ClipboardUnavailable)?,
            },
            other
                if other.starts_with("field:")
                    || other.starts_with("prompt:")
                    || other.starts_with("choice:")
                    || other.starts_with("datefield:") =>
            {
                let field = parse_form_field(other).expect("prefix checked")?;
                match context.fields.get(&field.key) {
                    Some(value) => {
                        field
                            .validate_value(value)
                            .map_err(|_| TemplateError::InvalidField)?;
                        value.clone()
                    }
                    None if context.validating => String::new(),
                    None => return Err(TemplateError::FieldValueMissing),
                }
            }
            other if other.starts_with("env:") => {
                let name = other["env:".len()..].trim();
                context
                    .env
                    .get(name)
                    .cloned()
                    .ok_or_else(|| TemplateError::EnvNotAllowed {
                        name: name.to_owned(),
                    })?
            }
            other if other.starts_with("snippet:") => {
                let trigger = other["snippet:".len()..].trim();
                let included = context
                    .snippets
                    .get(trigger)
                    .ok_or(TemplateError::UnknownSnippet)?;
                if depth >= MAX_INCLUDE_DEPTH {
                    return Err(TemplateError::IncludeTooDeep);
                }
                if !preview && template_variables(included).contains(&"cursor") {
                    return Err(TemplateError::CursorInInclude);
                }
                let include_budget = if preview {
                    maximum_bytes.saturating_sub(rendered.len())
                } else {
                    maximum_bytes
                };
                let (rendered, _, stopped) = render_nested(
                    included,
                    context,
                    false,
                    depth + 1,
                    include_budget,
                    preview,
                    named,
                )?;
                included_stopped = stopped;
                rendered
            }
            "uuid" => crate::template_dynamic::uuid_v4()?,
            other if other.starts_with("uuid:") => {
                let label = other["uuid:".len()..].trim();
                if label.is_empty() || label.chars().count() > 64 {
                    return Err(TemplateError::InvalidUuidName);
                }
                match named.get(label) {
                    Some(value) => value.clone(),
                    None => {
                        let value = crate::template_dynamic::uuid_v4()?;
                        named.insert(label.to_owned(), value.clone());
                        value
                    }
                }
            }
            other if other.starts_with("random:") => {
                crate::template_dynamic::random_value(&other["random:".len()..])?
            }
            other if formatted_clock_spec(other).is_some() => {
                let (base, offset, format) = formatted_clock_spec(other).expect("checked");
                let overflow = || TemplateError::DateArithmeticOverflow {
                    name: other.to_owned(),
                };
                let timestamp = context
                    .unix_timestamp
                    .checked_add_signed(offset)
                    .ok_or_else(overflow)?;
                let utc_offset = if base.starts_with("local_") {
                    Some(local_utc_offset(timestamp).ok_or_else(overflow)?)
                } else {
                    None
                };
                crate::template_dynamic::format_date(
                    format,
                    i64::try_from(timestamp).map_err(|_| overflow())?,
                    utc_offset,
                )?
            }
            other => match parse_offset_variable(other) {
                Some((base, offset)) => {
                    let adjusted = context
                        .unix_timestamp
                        .checked_add_signed(offset)
                        .ok_or_else(|| TemplateError::DateArithmeticOverflow {
                            name: other.to_owned(),
                        })?;
                    if !matches!(
                        base,
                        "date"
                            | "time"
                            | "datetime"
                            | "local_date"
                            | "local_time"
                            | "local_datetime"
                    ) {
                        return Err(TemplateError::UnknownVariable {
                            name: other.to_owned(),
                        });
                    }
                    format_clock(base, adjusted).ok_or_else(|| {
                        TemplateError::DateArithmeticOverflow {
                            name: other.to_owned(),
                        }
                    })?
                }
                None => {
                    return Err(TemplateError::UnknownVariable {
                        name: other.to_owned(),
                    })
                }
            },
        };
        let stopped = push_bounded(&mut rendered, &value, maximum_bytes, preview)?;
        if stopped || included_stopped {
            return Ok((rendered, cursor_position, true));
        }
        cursor = end + 2;
    }
    Ok((rendered, cursor_position, false))
}

/// A formatted clock variable, `date:FORMAT` or `local_date+1d:FORMAT`:
/// its base name, offset in seconds, and format. Every clock base accepts a
/// format; the base only chooses UTC or local time.
fn formatted_clock_spec(name: &str) -> Option<(&str, i64, &str)> {
    let (spec, format) = name.split_once(':')?;
    let spec = spec.trim();
    let (base, offset) = parse_offset_variable(spec).unwrap_or((spec, 0));
    matches!(
        base,
        "date" | "time" | "datetime" | "local_date" | "local_time" | "local_datetime"
    )
    .then_some((base, offset, format))
}

/// Parses a variable name of the form `<base><sign><magnitude><unit>` (e.g.
/// `date+3d`, `datetime-90m`) into the base variable name and a signed
/// offset in seconds. Returns `None` for anything that doesn't match this
/// shape -- including on any arithmetic overflow -- so the caller falls
/// through to the same "unknown variable" error as any other typo, rather
/// than risking a panic on adversarial input (e.g. an imported Espanso
/// library is not a fully trusted source).
fn parse_offset_variable(name: &str) -> Option<(&str, i64)> {
    let sign_position = name.rfind(['+', '-'])?;
    let (base, rest) = name.split_at(sign_position);
    if base.is_empty() {
        return None;
    }
    let negative = rest.starts_with('-');
    let rest = &rest[1..];
    let unit = rest.chars().next_back()?;
    let magnitude_str = &rest[..rest.len() - unit.len_utf8()];
    if magnitude_str.is_empty() {
        return None;
    }
    let magnitude: i64 = magnitude_str.parse().ok()?;
    let unit_seconds: i64 = match unit {
        'd' => 86_400,
        'w' => 7 * 86_400,
        'h' => 3_600,
        'm' => 60,
        _ => return None,
    };
    let offset = magnitude.checked_mul(unit_seconds)?;
    Some((base, if negative { -offset } else { offset }))
}

/// Renders `template` like [`render_template`], additionally accepting one
/// `{{cursor}}` marker (whitespace inside the braces is allowed, as for every
/// other variable). The marker renders to nothing; the returned offset is how
/// many grapheme clusters follow it in the rendered text -- the offset callers
/// use to move the cursor back after typing the result. A second marker is an
/// error rather than silently ignored, since there is only one cursor.
///
/// `{{cursor}}` is deliberately not a variable accepted by `render_template`
/// itself: it only marks a position, which callers without cursor support
/// could not honor.
///
/// Used both by config validation (to accept `{{cursor}}` before
/// activation) and by the engine (to actually render it), so the two
/// agree on what is valid.
pub fn render_template_with_cursor(
    template: &str,
    context: &TemplateContext,
) -> Result<(String, Option<usize>), TemplateError> {
    let (rendered, position) = render(template, context, true, MAX_RENDERED_BYTES)?;
    let cursor_offset = position.map(|position| rendered[position..].graphemes(true).count());
    Ok((rendered, cursor_offset))
}

fn push_bounded(
    output: &mut String,
    value: &str,
    maximum_bytes: usize,
    preview: bool,
) -> Result<bool, TemplateError> {
    if preview {
        if output.len() >= maximum_bytes {
            return Ok(true);
        }
        let remaining = maximum_bytes.saturating_sub(output.len());
        // Scan only far enough to find a newline or prove the byte budget is
        // exhausted. This also keeps a huge first line from requiring a full
        // pass before it can be truncated.
        let inspect_len = value.len().min(remaining.saturating_add(1));
        let line_end = value.as_bytes()[..inspect_len]
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(value.len());
        let mut byte_end = line_end.min(remaining);
        while !value.is_char_boundary(byte_end) {
            byte_end -= 1;
        }
        output.push_str(&value[..byte_end]);
        return Ok(byte_end < value.len() || output.len() >= maximum_bytes);
    }
    if output.len().saturating_add(value.len()) > maximum_bytes {
        return Err(TemplateError::RenderedTooLarge {
            maximum: maximum_bytes,
        });
    }
    output.push_str(value);
    Ok(false)
}

/// Render a clock variable. The plain names are UTC; the `local_` names use
/// the system time zone's offset at that instant, so DST is honored for date
/// offsets too. `None` if the instant cannot be represented.
fn format_clock(name: &str, timestamp: u64) -> Option<String> {
    match name {
        "date" | "time" | "datetime" => format_clock_with_offset(name, timestamp, None),
        "local_date" | "local_time" | "local_datetime" => format_clock_with_offset(
            &name["local_".len()..],
            timestamp,
            Some(local_utc_offset(timestamp)?),
        ),
        _ => None,
    }
}

/// `offset` is seconds east of UTC; `None` renders UTC with a `Z` suffix.
fn format_clock_with_offset(name: &str, timestamp: u64, offset: Option<i64>) -> Option<String> {
    // Signed: a local instant west of UTC may precede the Unix epoch.
    let shifted = i64::try_from(timestamp)
        .ok()?
        .checked_add(offset.unwrap_or(0))?;
    Some(match name {
        "date" => format_signed_date(shifted),
        "time" => format_signed_time(shifted),
        "datetime" => {
            let zone = match offset {
                None => "Z".to_owned(),
                Some(offset) => {
                    let sign = if offset < 0 { '-' } else { '+' };
                    let minutes = offset.unsigned_abs() / 60;
                    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
                }
            };
            format!(
                "{}T{}{zone}",
                format_signed_date(shifted),
                format_signed_time(shifted)
            )
        }
        _ => return None,
    })
}

/// The kernel's current host name. Read on every render rather than cached:
/// a long-running daemon must follow `hostnamectl set-hostname`, and
/// `/etc/hostname` (read before) is only the static name, empty or missing
/// when the name comes from DHCP or a transient setting.
fn current_hostname() -> String {
    rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .trim()
        .to_owned()
}

// POSIX; the `libc` crate does not bind it.
unsafe extern "C" {
    fn tzset();
}

/// Seconds east of UTC for the system time zone at `timestamp`.
fn local_utc_offset(timestamp: u64) -> Option<i64> {
    let time = libc::time_t::try_from(timestamp).ok()?;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    // glibc's localtime_r reads the time zone only once per process, so a
    // long-running daemon kept the zone it started with after the system
    // zone changed (travel, `timedatectl set-timezone`). tzset re-reads TZ
    // and /etc/localtime when they change; glibc serializes it internally.
    // SAFETY: tzset takes no arguments and only updates libc's zone state.
    unsafe { tzset() };
    // SAFETY: both pointers are valid for the call; localtime_r is the
    // reentrant variant and fully initializes `local` when it succeeds.
    let result = unsafe { libc::localtime_r(&time, local.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }
    // SAFETY: localtime_r returned non-null, so `local` is initialized.
    let offset = unsafe { local.assume_init() }.tm_gmtoff;
    // `tm_gmtoff` is a C long: already i64 on 64-bit targets, i32 elsewhere.
    #[allow(clippy::useless_conversion)]
    Some(i64::from(offset))
}

fn format_signed_time(timestamp: i64) -> String {
    let seconds = timestamp.rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

pub(crate) fn format_date(timestamp: u64) -> String {
    format_signed_date(i64::try_from(timestamp).unwrap_or(i64::MAX))
}

fn format_signed_date(timestamp: i64) -> String {
    let (year, month, day) = civil_from_days(timestamp.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

// Gregorian UTC conversion, adapted from the public-domain civil_from_days
// algorithm. Keeping this local avoids a runtime dependency or shell command.
pub(crate) fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_today_reports_an_unrepresentable_timestamp() {
        assert_eq!(local_date_at(u64::MAX), Err(DateError::Unrepresentable));
        assert_ne!(local_date_at(u64::MAX).ok().as_deref(), Some("1970-01-01"));
    }

    #[test]
    fn preview_renderer_stops_at_its_output_budget_for_large_templates() {
        let context = TemplateContext::default();
        assert_eq!(
            render_template_preview(&"x".repeat(1024 * 1024), &context, 512).unwrap(),
            "x".repeat(512)
        );
        assert_eq!(
            render_template_preview(
                &format!("{}{{{{unknown}}}}", "y".repeat(512)),
                &context,
                512
            )
            .unwrap(),
            "y".repeat(512)
        );
        assert_eq!(
            render_template_preview("a🙂z", &context, 2).unwrap(),
            "a",
            "preview truncation must not split a UTF-8 scalar"
        );
        assert_eq!(
            render_template_preview("hello {{username}}", &context, 512).unwrap(),
            format!("hello {}", context.username)
        );
        assert_eq!(
            render_template_preview(
                &format!("first line\n{}{{{{unknown}}}}", "x".repeat(1024 * 1024)),
                &context,
                512
            )
            .unwrap(),
            "first line"
        );
        assert_eq!(
            render_template_preview(
                &format!("{{{{newline}}}}{}", "x".repeat(1024 * 1024)),
                &context,
                512
            )
            .unwrap(),
            ""
        );

        let mut snippets = HashMap::new();
        snippets.insert(
            "signature".into(),
            format!("included line\n{{{{cursor}}}}{}", "x".repeat(1024 * 1024)).into(),
        );
        let context = TemplateContext {
            snippets: Arc::new(snippets),
            ..TemplateContext::default()
        };
        assert_eq!(
            render_template_preview("{{snippet:signature}}", &context, 512).unwrap(),
            "included line"
        );
        assert!(matches!(
            render_template("{{snippet:signature}}", &context),
            Err(TemplateError::CursorInInclude)
        ));
        assert_eq!(
            render_template("first line\nsecond line", &context).unwrap(),
            "first line\nsecond line"
        );
    }

    #[test]
    fn renders_builtins_without_shelling_out() {
        let context = TemplateContext {
            username: "ada".into(),
            hostname: "workstation".into(),
            unix_timestamp: 0,
            ..TemplateContext::default()
        };
        assert_eq!(
            render_template(
                "Hi {{username}} on {{hostname}} {{date}} {{time}}{{newline}}",
                &context
            )
            .unwrap(),
            "Hi ada on workstation 1970-01-01 00:00:00\n"
        );
    }

    #[test]
    fn local_clock_variables_apply_the_utc_offset() {
        // 2024-01-01T00:30:00Z
        let timestamp = 1_704_069_000;
        assert_eq!(
            format_clock_with_offset("datetime", timestamp, None).as_deref(),
            Some("2024-01-01T00:30:00Z")
        );
        // West of UTC the local calendar day is still the previous one.
        assert_eq!(
            format_clock_with_offset("date", timestamp, Some(-5 * 3_600)).as_deref(),
            Some("2023-12-31")
        );
        assert_eq!(
            format_clock_with_offset("datetime", timestamp, Some(-5 * 3_600)).as_deref(),
            Some("2023-12-31T19:30:00-05:00")
        );
        assert_eq!(
            format_clock_with_offset("datetime", timestamp, Some(5 * 3_600 + 45 * 60)).as_deref(),
            Some("2024-01-01T06:15:00+05:45")
        );
        // Validation renders at the epoch; a western zone must still format.
        assert_eq!(
            format_clock_with_offset("datetime", 0, Some(-8 * 3_600)).as_deref(),
            Some("1969-12-31T16:00:00-08:00")
        );
    }

    #[test]
    fn local_clock_variables_render_with_the_system_offset() {
        let context = TemplateContext {
            unix_timestamp: 1_704_069_000,
            ..TemplateContext::default()
        };
        let offset = local_utc_offset(context.unix_timestamp).unwrap();
        assert_eq!(
            render_template("{{local_datetime}}", &context).unwrap(),
            format_clock_with_offset("datetime", context.unix_timestamp, Some(offset)).unwrap()
        );
        let tomorrow = context.unix_timestamp + 86_400;
        assert_eq!(
            render_template("{{local_date+1d}}", &context).unwrap(),
            format_clock_with_offset("date", tomorrow, Some(local_utc_offset(tomorrow).unwrap()))
                .unwrap()
        );
        for template in ["{{local_date}}", "{{local_time}}", "{{local_datetime-1w}}"] {
            render_template(template, &context).unwrap();
        }
    }

    #[test]
    fn rejects_unknown_and_unclosed_variables() {
        let context = TemplateContext::default();
        assert!(matches!(
            render_template("{{secret}}", &context),
            Err(TemplateError::UnknownVariable { .. })
        ));
        assert!(matches!(
            render_template("{{date", &context),
            Err(TemplateError::Unclosed { .. })
        ));
    }

    #[test]
    fn cursor_marker_accepts_whitespace_and_rejects_a_second_marker() {
        let context = TemplateContext::default();
        assert_eq!(
            render_template_with_cursor("a {{ cursor }}bc", &context).unwrap(),
            ("a bc".to_owned(), Some(2))
        );
        assert_eq!(
            render_template_with_cursor("{{cursor}}{{newline}}x", &context).unwrap(),
            ("\nx".to_owned(), Some(2))
        );
        assert!(matches!(
            render_template_with_cursor("v{{cursor}} and v{{cursor}}", &context),
            Err(TemplateError::MultipleCursors { offset: 17 })
        ));
        // Plain rendering has no cursor to place, so the marker stays unknown.
        assert!(matches!(
            render_template("{{cursor}}", &context),
            Err(TemplateError::UnknownVariable { .. })
        ));
    }

    #[test]
    fn uses_long_year_safe_calendar_conversion() {
        let context = TemplateContext {
            unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z
            ..TemplateContext::default()
        };
        assert_eq!(
            render_template("{{datetime}}", &context).unwrap(),
            "2024-01-01T00:00:00Z"
        );
    }

    #[test]
    fn date_math_supports_day_and_week_offsets() {
        let context = TemplateContext {
            unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z (a Monday)
            ..TemplateContext::default()
        };
        assert_eq!(
            render_template("{{date+3d}}", &context).unwrap(),
            "2024-01-04"
        );
        assert_eq!(
            render_template("{{date-1d}}", &context).unwrap(),
            "2023-12-31"
        );
        assert_eq!(
            render_template("{{date+1w}}", &context).unwrap(),
            "2024-01-08"
        );
    }

    #[test]
    fn date_math_supports_hour_and_minute_offsets_on_time_and_datetime() {
        let context = TemplateContext {
            unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z
            ..TemplateContext::default()
        };
        assert_eq!(
            render_template("{{time+5h}}", &context).unwrap(),
            "05:00:00"
        );
        assert_eq!(
            render_template("{{datetime+90m}}", &context).unwrap(),
            "2024-01-01T01:30:00Z"
        );
    }

    #[test]
    fn date_math_offset_crossing_a_year_boundary_is_calendar_correct() {
        let context = TemplateContext {
            unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z
            ..TemplateContext::default()
        };
        assert_eq!(
            render_template("{{date-1d}}", &context).unwrap(),
            "2023-12-31"
        );
    }

    #[test]
    fn date_math_rejects_malformed_or_overflowing_offsets_as_unknown_variables() {
        let context = TemplateContext::default();
        for template in [
            "{{date+3x}}",
            "{{date+}}",
            "{{date+99999999999999999999d}}",
            "{{date+999999999999999999w}}",
        ] {
            assert!(matches!(
                render_template(template, &context),
                Err(TemplateError::UnknownVariable { .. })
            ));
        }
    }

    #[test]
    fn cursor_offset_counts_combining_sequence_as_one_grapheme() {
        let (rendered, offset) =
            render_template_with_cursor("prefix{{cursor}}e\u{301}", &TemplateContext::default())
                .unwrap();

        assert_eq!(rendered, "prefixe\u{301}");
        assert_eq!(offset, Some(1));
    }

    #[test]
    fn cursor_offset_counts_zwj_emoji_as_one_grapheme() {
        let family = "👨‍👩‍👧‍👦";
        let (rendered, offset) = render_template_with_cursor(
            &format!("prefix{{{{cursor}}}}{family}"),
            &TemplateContext::default(),
        )
        .unwrap();

        assert_eq!(rendered, format!("prefix{family}"));
        assert_eq!(offset, Some(1));
    }

    #[test]
    fn cursor_offset_counts_emoji_modifier_as_one_grapheme() {
        let (rendered, offset) =
            render_template_with_cursor("prefix{{cursor}}👍🏽", &TemplateContext::default()).unwrap();

        assert_eq!(rendered, "prefix👍🏽");
        assert_eq!(offset, Some(1));
    }

    #[test]
    fn doubled_braces_render_literal_template_syntax() {
        let context = TemplateContext::default();
        let playbook = r#"name: "{{{{ item }}}}" on {{tab}}{{{{ansible_host}}}}"#;
        assert_eq!(
            render_template(playbook, &context).unwrap(),
            "name: \"{{ item }}\" on \t{{ansible_host}}"
        );
        // An escaped opener is not a marker, so nothing inside it is parsed.
        assert!(template_variables(r"{{{{ cursor }}}} {{tab}}") == vec!["tab"]);
        assert_eq!(
            render_template_with_cursor(r"{{{{ x }}}}{{cursor}}!", &context).unwrap(),
            ("{{ x }}!".to_owned(), Some(1))
        );
        // A lone `}}` in literal text is unchanged, as before.
        assert_eq!(render_template("a }} b", &context).unwrap(), "a }} b");
        assert_eq!(
            render_template_preview(r"{{{{ item }}}}", &context, 3).unwrap(),
            "{{ ",
            "previews stay within their budget"
        );
    }

    #[test]
    fn escaped_braces_mix_with_form_fields() {
        let template = r#"- hosts: {{field:hosts=all}}
  tasks:
    - ansible.builtin.package:
        name: "{{{{ item }}}}"
        state: {{choice:state=present|latest|absent}}
      loop: [{{field:package}}]"#;
        let fields = form_fields(template).unwrap();
        let labels: Vec<&str> = fields.iter().map(|field| field.label.as_str()).collect();
        assert_eq!(labels, ["hosts", "state", "package"]);
        let mut values = HashMap::new();
        values.insert("field:hosts".to_owned(), "web".to_owned());
        values.insert("choice:state".to_owned(), "latest".to_owned());
        values.insert("field:package".to_owned(), "nginx".to_owned());
        let context = TemplateContext {
            fields: Arc::new(values),
            ..TemplateContext::default()
        };
        assert_eq!(
            render_template(template, &context).unwrap(),
            r#"- hosts: web
  tasks:
    - ansible.builtin.package:
        name: "{{ item }}"
        state: latest
      loop: [nginx]"#
        );
    }

    #[test]
    fn named_choices_have_a_label_and_a_stable_key() {
        let fields =
            form_fields("{{choice:env=dev|prod}} {{choice:env=dev|prod}} {{choice:a|b}}").unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].label, "env");
        assert_eq!(fields[0].key, "choice:env");
        assert_eq!(
            fields[0].kind,
            FormFieldKind::Choice {
                options: vec!["dev".to_owned(), "prod".to_owned()],
                default: None,
            }
        );
        assert_eq!(fields[1].label, "Choice");
        assert_eq!(fields[1].key, "choice:a|b");
        // `=` after the first option is part of an option, as before.
        assert_eq!(
            form_fields("{{choice:a|b=c}}").unwrap()[0].kind,
            FormFieldKind::Choice {
                options: vec!["a".to_owned(), "b=c".to_owned()],
                default: None,
            }
        );
        for invalid in [
            "{{choice:=a|b}}",
            "{{choice:env=a}}",
            "{{choice:env=dev|prod}} {{choice:env=dev|test}}",
        ] {
            assert_eq!(
                form_fields(invalid),
                Err(TemplateError::InvalidField),
                "{invalid}"
            );
        }
    }

    #[test]
    fn bare_references_reuse_the_first_named_form_field_definition() {
        let fields = form_fields(
            "{{field:host=example.net|required|min=3}} {{field:host}} {{prompt:host}} {{datefield:reported=2026-10-28|required}} {{datefield:reported}}",
        )
        .unwrap();

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key, "field:host");
        assert_eq!(
            fields[0].kind,
            FormFieldKind::Text {
                default: "example.net".into()
            }
        );
        assert!(fields[0].required);
        assert_eq!(fields[0].min_chars, Some(3));
        assert_eq!(fields[1].key, "date:reported");
        assert_eq!(
            fields[1].kind,
            FormFieldKind::Date {
                default: "2026-10-28".into()
            }
        );
        assert!(fields[1].required);
    }

    #[test]
    fn a_definition_after_bare_references_supplies_the_field_rules() {
        let fields =
            form_fields("Hi {{field:host}}, {{field:host=example.net|required}} {{field:host}}")
                .unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].kind,
            FormFieldKind::Text {
                default: "example.net".into()
            }
        );
        assert!(fields[0].required);
        assert_eq!(
            form_fields("{{field:host}} {{field:host=a}} {{field:host=b}}"),
            Err(TemplateError::InvalidField)
        );
    }

    #[test]
    fn incident_form_parses_defaults_and_validates_values() {
        let template = "{{datefield:reported_at=today|required}} {{field:hostname|required|max=253}} {{choice:severity=Low|Medium|High|Critical;default=High;required}} {{field:summary|required|min=10|max=2000}} {{field:resolution|required|min=3|max=4000}}";
        let fields = form_fields(template).unwrap();
        assert_eq!(fields.len(), 5);
        assert!(
            matches!(fields[0].kind, FormFieldKind::Date { ref default } if default == "today")
        );
        assert_eq!(fields[2].key, "choice:severity");
        assert_eq!(
            fields[2].kind,
            FormFieldKind::Choice {
                options: vec![
                    "Low".into(),
                    "Medium".into(),
                    "High".into(),
                    "Critical".into()
                ],
                default: Some("High".into()),
            }
        );
        assert_eq!(fields[0].validate_value("2024-02-29"), Ok(()));
        assert_eq!(
            fields[0].validate_value("2025-02-29"),
            Err(FormValueError::InvalidDate)
        );
        assert_eq!(
            fields[1].validate_value("  "),
            Err(FormValueError::Required)
        );
        assert_eq!(
            fields[1].validate_value(&"h".repeat(254)),
            Err(FormValueError::TooLong { maximum: 253 })
        );
        assert_eq!(
            fields[3].validate_value("short"),
            Err(FormValueError::TooShort { minimum: 10 })
        );
        assert_eq!(
            fields[2].validate_value("Urgent"),
            Err(FormValueError::InvalidChoice)
        );
    }

    #[test]
    fn invalid_form_rules_and_choice_defaults_are_rejected() {
        for template in [
            "{{datefield:when=2025-02-29}}",
            "{{field:name|min=10|max=2}}",
            "{{choice:severity=Low|High;default=Medium}}",
        ] {
            assert_eq!(
                form_fields(template),
                Err(TemplateError::InvalidField),
                "{template}"
            );
        }
    }

    #[test]
    fn formatted_dates_apply_to_every_clock_variable_and_offset() {
        let context = TemplateContext {
            // 2026-10-09 14:05:07 UTC, a Friday.
            unix_timestamp: 1_791_554_707,
            ..TemplateContext::default()
        };
        let render = |template: &str| render_template(template, &context).unwrap();
        assert_eq!(render("{{date:%A %-d %B}}"), "Friday 9 October");
        assert_eq!(render("{{time:%H:%M}}"), "14:05");
        assert_eq!(render("{{date+1d:%a %F}}"), "Sat 2026-10-10");
        assert_eq!(render("{{datetime-90m:%H:%M %Z}}"), "12:35 UTC");
        assert_eq!(render("{{ date : %Y }}"), " 2026");
        // Local time: the format sees the system offset at that instant.
        let local = render("{{local_date:%F %H:%M %:z}}");
        assert_eq!(local.len(), "2026-10-09 08:05 -06:00".len(), "{local}");
        assert!(matches!(
            render_template("{{date:%Q}}", &context),
            Err(TemplateError::InvalidDateFormat { .. })
        ));
        // Not a clock: still an unknown variable.
        assert!(matches!(
            render_template("{{when:%F}}", &context),
            Err(TemplateError::UnknownVariable { .. })
        ));
    }

    #[test]
    fn uuid_and_random_render_fresh_values_and_named_uuids_repeat() {
        let context = TemplateContext::default();
        let rendered =
            render_template("{{uuid}} {{uuid}} {{uuid:id}} {{uuid:id}}", &context).unwrap();
        let parts: Vec<&str> = rendered.split(' ').collect();
        assert_ne!(parts[0], parts[1]);
        assert_eq!(parts[2], parts[3]);
        assert_ne!(parts[2], parts[0]);
        // A named UUID is shared with included snippets in the same render.
        let mut snippets = HashMap::new();
        snippets.insert(":inner".to_owned(), Arc::<str>::from("{{uuid:id}}"));
        let context = TemplateContext {
            snippets: Arc::new(snippets),
            ..TemplateContext::default()
        };
        let rendered = render_template("{{uuid:id}}={{snippet::inner}}", &context).unwrap();
        let (outer, inner) = rendered.split_once('=').unwrap();
        assert_eq!(outer, inner);
        // ...but each render gets new values.
        assert_ne!(
            rendered,
            render_template("{{uuid:id}}={{snippet::inner}}", &context).unwrap()
        );

        let token = render_template("{{random:hex:16}}", &context).unwrap();
        assert_eq!(token.len(), 16);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        let pick = render_template("{{random:a|b}}", &context).unwrap();
        assert!(pick == "a" || pick == "b");
        for invalid in ["{{random:hex}}", "{{random:x:1}}", "{{uuid:}}"] {
            assert!(render_template(invalid, &context).is_err(), "{invalid}");
        }
    }

    #[test]
    fn malformed_dynamic_markers_fail_configuration_validation() {
        for replacement in ["{{date:%Q}}", "{{random:hex:0}}", "{{uuid: }}"] {
            let text = format!("[[expansion]]\ntrigger = \":x\"\nreplacement = {replacement:?}\n");
            assert!(crate::Config::parse(&text).is_err(), "{replacement}");
        }
        let text = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"{{local_date+1w:%A}} {{uuid}} {{random:password:20}}\"\n";
        assert!(crate::Config::parse(text).is_ok());
    }

    /// A long-running daemon must follow a change of the system time zone.
    /// Changing `TZ` is only safe without other threads, so the check runs
    /// in a child copy of this test binary limited to this one test.
    #[test]
    fn local_offset_follows_a_changed_time_zone() {
        const CHILD: &str = "WAYEXPAND_TZ_CHANGE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            // SAFETY: the parent runs this child with --test-threads=1 and
            // only this test, so no other thread reads the environment.
            unsafe { std::env::set_var("TZ", "AAA+03") };
            let before = super::local_utc_offset(1_800_000_000);
            unsafe { std::env::set_var("TZ", "BBB-05") };
            let after = super::local_utc_offset(1_800_000_000);
            assert_eq!(before, Some(-3 * 3_600));
            assert_eq!(after, Some(5 * 3_600));
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "template::tests::local_offset_follows_a_changed_time_zone",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // The filter must have matched: a renamed test would pass vacuously.
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}
