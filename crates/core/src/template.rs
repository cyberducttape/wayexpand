use crate::SensitiveText;
use std::{
    collections::{BTreeMap, HashMap},
    env,
    sync::{Arc, OnceLock},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

mod form;
#[cfg(test)]
use form::local_date_at;
use form::parse_form_field;
pub use form::{
    form_fields, today_local_date, DateError, FormField, FormFieldKind, FormFieldValidation,
    FormValueError, FormValues,
};

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
    pub fields: Arc<FormValues>,
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
    render(template, context, false, MAX_RENDERED_BYTES).map(|(rendered, _)| rendered.into_string())
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
    .map(|(rendered, _, _)| rendered.into_string())
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
/// One-shot rendering streams over the source: compiling first would copy
/// every literal once more for a template that is rendered only once.
fn render(
    template: &str,
    context: &TemplateContext,
    allow_cursor: bool,
    maximum_bytes: usize,
) -> Result<(SensitiveText, Option<usize>), TemplateError> {
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
    named: &mut HashMap<String, SensitiveText>,
) -> Result<(SensitiveText, Option<usize>, bool), TemplateError> {
    // Preview callers may pass an arbitrarily large valid template. Do not
    // reserve its full size before the output limit has a chance to reject it.
    let mut rendered = SensitiveText::new(String::with_capacity(template.len().min(maximum_bytes)));
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
        // Only a preview stops at a newline; a full render must not pay for
        // a second pass over the template.
        let relative_newline = preview.then(|| inspected.find('\n')).flatten();

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
        let value = match parse_variable(name)? {
            Variable::Snippet(trigger) => {
                let include_budget = if preview {
                    maximum_bytes.saturating_sub(rendered.len())
                } else {
                    maximum_bytes
                };
                let (rendered, stopped) =
                    render_include(&trigger, context, depth, include_budget, preview, named)?;
                included_stopped = stopped;
                RenderValue::Owned(rendered)
            }
            Variable::Clipboard if preview => return Err(TemplateError::ClipboardDisabled),
            variable => evaluate(&variable, name, context, named)?,
        };
        let stopped = push_bounded(&mut rendered, value.as_str(), maximum_bytes, preview)?;
        if stopped || included_stopped {
            return Ok((rendered, cursor_position, true));
        }
        cursor = end + 2;
    }
    Ok((rendered, cursor_position, false))
}

/// Render an included snippet one level below `depth`.
fn render_include(
    trigger: &str,
    context: &TemplateContext,
    depth: usize,
    budget: usize,
    preview: bool,
    named: &mut HashMap<String, SensitiveText>,
) -> Result<(SensitiveText, bool), TemplateError> {
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
    let (rendered, _, stopped) =
        render_nested(included, context, false, depth + 1, budget, preview, named)?;
    Ok((rendered, stopped))
}

/// One `{{...}}` variable, parsed. Parsing checks everything that does not
/// depend on the render context, so a validated template never fails here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Variable {
    Clock(String),
    ClockOffset {
        base: String,
        offset: i64,
    },
    FormattedClock {
        base: String,
        offset: i64,
        format: String,
    },
    Username,
    Hostname,
    UnixTimestamp,
    Newline,
    Tab,
    Clipboard,
    Cursor,
    Field(FormField),
    ShellQuotedField(FormField),
    Env(String),
    Snippet(String),
    Uuid,
    NamedUuid(String),
    Random(String),
}

fn is_clock_base(name: &str) -> bool {
    matches!(
        name,
        "date" | "time" | "datetime" | "local_date" | "local_time" | "local_datetime"
    )
}

fn parse_variable(name: &str) -> Result<Variable, TemplateError> {
    Ok(match name {
        clock if is_clock_base(clock) => Variable::Clock(clock.to_owned()),
        "username" => Variable::Username,
        "hostname" => Variable::Hostname,
        "unix_timestamp" => Variable::UnixTimestamp,
        "newline" => Variable::Newline,
        "tab" => Variable::Tab,
        "clipboard" => Variable::Clipboard,
        "cursor" => Variable::Cursor,
        other
            if other.starts_with("field:")
                || other.starts_with("prompt:")
                || other.starts_with("choice:")
                || other.starts_with("datefield:") =>
        {
            Variable::Field(parse_form_field(other).expect("prefix checked")?)
        }
        other if other.starts_with("shellq:") => {
            let field = format!("field:{}", &other["shellq:".len()..]);
            Variable::ShellQuotedField(
                parse_form_field(&field).expect("the shellq field prefix is present")?,
            )
        }
        other if other.starts_with("env:") => {
            Variable::Env(other["env:".len()..].trim().to_owned())
        }
        other if other.starts_with("snippet:") => {
            Variable::Snippet(other["snippet:".len()..].trim().to_owned())
        }
        "uuid" => Variable::Uuid,
        other if other.starts_with("uuid:") => {
            let label = other["uuid:".len()..].trim();
            if label.is_empty() || label.chars().count() > 64 {
                return Err(TemplateError::InvalidUuidName);
            }
            Variable::NamedUuid(label.to_owned())
        }
        other if other.starts_with("random:") => {
            Variable::Random(other["random:".len()..].to_owned())
        }
        other if formatted_clock_spec(other).is_some() => {
            let (base, offset, format) = formatted_clock_spec(other).expect("checked");
            Variable::FormattedClock {
                base: base.to_owned(),
                offset,
                format: format.to_owned(),
            }
        }
        other => match parse_offset_variable(other) {
            Some((base, offset)) if is_clock_base(base) => Variable::ClockOffset {
                base: base.to_owned(),
                offset,
            },
            _ => {
                return Err(TemplateError::UnknownVariable {
                    name: other.to_owned(),
                })
            }
        },
    })
}

/// Quote one form value as a single POSIX shell word. This only makes the
/// value safe to place in a command line; it does not authorize execution.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The value of one variable. `name` is its source text, for errors.
/// Includes and `{{cursor}}` depend on where they appear, so callers handle
/// them; reaching them here means they are not allowed in this position.
fn evaluate<'a>(
    variable: &Variable,
    name: &str,
    context: &'a TemplateContext,
    named: &mut HashMap<String, SensitiveText>,
) -> Result<RenderValue<'a>, TemplateError> {
    let overflow = || TemplateError::DateArithmeticOverflow {
        name: name.to_owned(),
    };
    Ok(match variable {
        Variable::Clock(base) => {
            RenderValue::owned(format_clock(base, context.unix_timestamp).ok_or_else(overflow)?)
        }
        Variable::ClockOffset { base, offset } => {
            let adjusted = context
                .unix_timestamp
                .checked_add_signed(*offset)
                .ok_or_else(overflow)?;
            RenderValue::owned(format_clock(base, adjusted).ok_or_else(overflow)?)
        }
        Variable::FormattedClock {
            base,
            offset,
            format,
        } => {
            let timestamp = context
                .unix_timestamp
                .checked_add_signed(*offset)
                .ok_or_else(overflow)?;
            let utc_offset = if base.starts_with("local_") {
                Some(local_utc_offset(timestamp).ok_or_else(overflow)?)
            } else {
                None
            };
            RenderValue::owned(crate::template_dynamic::format_date(
                format,
                i64::try_from(timestamp).map_err(|_| overflow())?,
                utc_offset,
            )?)
        }
        Variable::Username => RenderValue::Borrowed(context.username.as_str()),
        Variable::Hostname => RenderValue::Borrowed(context.hostname.as_str()),
        Variable::UnixTimestamp => RenderValue::owned(context.unix_timestamp.to_string()),
        Variable::Newline => RenderValue::Borrowed("\n"),
        Variable::Tab => RenderValue::Borrowed("\t"),
        Variable::Clipboard => match &context.clipboard {
            None => return Err(TemplateError::ClipboardDisabled),
            Some(_) if context.validating => RenderValue::Borrowed(""),
            Some(reader) => {
                RenderValue::owned((reader.0)().ok_or(TemplateError::ClipboardUnavailable)?)
            }
        },
        Variable::Field(field) => match context.fields.get(&field.key) {
            Some(value) => {
                field
                    .validate_value(value)
                    .map_err(|_| TemplateError::InvalidField)?;
                RenderValue::Borrowed(value)
            }
            None if context.validating => RenderValue::Borrowed(""),
            None => return Err(TemplateError::FieldValueMissing),
        },
        Variable::ShellQuotedField(field) => match context.fields.get(&field.key) {
            Some(value) => {
                field
                    .validate_value(value)
                    .map_err(|_| TemplateError::InvalidField)?;
                if value.contains('\n') || value.contains('\t') {
                    return Err(TemplateError::InvalidField);
                }
                RenderValue::owned(shell_quote(value))
            }
            None if context.validating => RenderValue::Borrowed("''"),
            None => return Err(TemplateError::FieldValueMissing),
        },
        Variable::Env(variable) => {
            RenderValue::Borrowed(context.env.get(variable).map(String::as_str).ok_or_else(
                || TemplateError::EnvNotAllowed {
                    name: variable.clone(),
                },
            )?)
        }
        Variable::Uuid => RenderValue::owned(crate::template_dynamic::uuid_v4()?),
        Variable::NamedUuid(label) => match named.get(label) {
            Some(value) => RenderValue::Owned(value.clone()),
            None => {
                let value = SensitiveText::from(crate::template_dynamic::uuid_v4()?);
                named.insert(label.clone(), value.clone());
                RenderValue::Owned(value)
            }
        },
        Variable::Random(spec) => RenderValue::owned(crate::template_dynamic::random_value(spec)?),
        Variable::Cursor | Variable::Snippet(_) => {
            return Err(TemplateError::UnknownVariable {
                name: name.to_owned(),
            })
        }
    })
}

enum RenderValue<'a> {
    Borrowed(&'a str),
    Owned(SensitiveText),
}

impl<'a> RenderValue<'a> {
    fn owned(value: String) -> Self {
        Self::Owned(SensitiveText::from(value))
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Variable {
        name: String,
        variable: Variable,
    },
    /// `{{cursor}}`, with its source offset for error reports.
    Cursor {
        offset: usize,
    },
}

/// A template parsed once into literal text and variables. Rendering it
/// gives exactly what [`render_template_with_cursor`] gives for the source
/// text, without scanning or parsing it again; validation compiles each
/// replacement and the engine renders the compiled form. Both share one
/// variable parser and evaluator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTemplate {
    segments: Vec<Segment>,
    /// Total literal bytes, to size the output once.
    literal_bytes: usize,
}

impl CompiledTemplate {
    pub fn compile(template: &str) -> Result<Self, TemplateError> {
        let mut segments = Vec::new();
        let mut literal = String::new();
        let mut cursor = 0;
        while let Some(relative_start) = template[cursor..].find("{{") {
            let start = cursor + relative_start;
            literal.push_str(&literal_text(&template[cursor..start]));
            if template[start..].starts_with(LITERAL_OPEN) {
                literal.push_str("{{");
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
            if !literal.is_empty() {
                segments.push(Segment::Literal(std::mem::take(&mut literal)));
            }
            segments.push(match parse_variable(name)? {
                Variable::Cursor => Segment::Cursor { offset: start },
                variable => Segment::Variable {
                    name: name.to_owned(),
                    variable,
                },
            });
            cursor = end + 2;
        }
        literal.push_str(&literal_text(&template[cursor..]));
        if !literal.is_empty() {
            segments.push(Segment::Literal(literal));
        }
        let literal_bytes = segments
            .iter()
            .map(|segment| match segment {
                Segment::Literal(text) => text.len(),
                _ => 0,
            })
            .sum();
        Ok(Self {
            segments,
            literal_bytes,
        })
    }

    /// Render with `{{cursor}}` support; see [`render_template_with_cursor`].
    pub fn render_with_cursor(
        &self,
        context: &TemplateContext,
    ) -> Result<(String, Option<usize>), TemplateError> {
        self.render_sensitive_with_cursor(context)
            .map(|(rendered, position)| (rendered.into_string(), position))
    }

    pub(crate) fn render_sensitive_with_cursor(
        &self,
        context: &TemplateContext,
    ) -> Result<(SensitiveText, Option<usize>), TemplateError> {
        let (rendered, position) = self.render(context, true, MAX_RENDERED_BYTES)?;
        let cursor_offset = position.map(|position| rendered[position..].graphemes(true).count());
        Ok((rendered, cursor_offset))
    }

    fn render(
        &self,
        context: &TemplateContext,
        allow_cursor: bool,
        maximum_bytes: usize,
    ) -> Result<(SensitiveText, Option<usize>), TemplateError> {
        let mut rendered =
            SensitiveText::new(String::with_capacity(self.literal_bytes.min(maximum_bytes)));
        let mut cursor_position = None;
        let mut named = HashMap::<String, SensitiveText>::new();
        for segment in &self.segments {
            let value = match segment {
                Segment::Literal(text) => RenderValue::Borrowed(text.as_str()),
                Segment::Cursor { offset } if allow_cursor => {
                    if cursor_position.is_some() {
                        return Err(TemplateError::MultipleCursors { offset: *offset });
                    }
                    cursor_position = Some(rendered.len());
                    continue;
                }
                Segment::Cursor { .. } => {
                    return Err(TemplateError::UnknownVariable {
                        name: "cursor".to_owned(),
                    })
                }
                Segment::Variable {
                    variable: Variable::Snippet(trigger),
                    ..
                } => RenderValue::Owned(
                    render_include(trigger, context, 0, maximum_bytes, false, &mut named)?.0,
                ),
                Segment::Variable { name, variable } => {
                    evaluate(variable, name, context, &mut named)?
                }
            };
            push_bounded(&mut rendered, value.as_str(), maximum_bytes, false)?;
        }
        Ok((rendered, cursor_position))
    }
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
    render_template_with_cursor_sensitive(template, context)
        .map(|(rendered, offset)| (rendered.into_string(), offset))
}

pub(crate) fn render_template_with_cursor_sensitive(
    template: &str,
    context: &TemplateContext,
) -> Result<(SensitiveText, Option<usize>), TemplateError> {
    let (rendered, position) = render(template, context, true, MAX_RENDERED_BYTES)?;
    let cursor_offset = position.map(|position| rendered[position..].graphemes(true).count());
    Ok((rendered, cursor_offset))
}

fn push_bounded(
    output: &mut SensitiveText,
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
mod tests;

#[cfg(test)]
mod form_values_tests {
    use super::FormValues;

    #[test]
    fn form_values_redact_debug_and_report_replaced_keys() {
        let mut values = FormValues::default();
        assert!(!values.insert("field:token".into(), "synthetic-form-secret".into()));
        assert!(values.insert("field:token".into(), "replacement".into()));
        assert_eq!(
            values.get("field:token").map(String::as_str),
            Some("replacement")
        );
        let debug = format!("{values:?}");
        assert!(debug.contains("field:token"));
        assert!(!debug.contains("replacement"));
        assert!(!debug.contains("synthetic-form-secret"));
    }

    #[test]
    fn form_values_deserialize_from_a_json_object() {
        let values: FormValues = serde_json::from_str(r#"{"field:name":"Ada"}"#).unwrap();
        assert_eq!(values.get("field:name").map(String::as_str), Some("Ada"));
    }
}
