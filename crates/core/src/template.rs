use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    sync::{Arc, OnceLock},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

const MAX_RENDERED_BYTES: usize = 1024 * 1024;
/// How deeply `{{snippet:...}}` includes may nest.
const MAX_INCLUDE_DEPTH: usize = 8;
static SYSTEM_USERNAME: OnceLock<String> = OnceLock::new();
static SYSTEM_HOSTNAME: OnceLock<String> = OnceLock::new();

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
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldKind {
    Text { default: String },
    Choice { options: Vec<String> },
}

/// Parse one form marker body (`field:name`, `field:name=default`,
/// `prompt:name`, or `choice:A|B|C`). Returns `None` for other variables.
fn parse_form_field(name: &str) -> Option<Result<FormField, TemplateError>> {
    if let Some(spec) = name
        .strip_prefix("field:")
        .or_else(|| name.strip_prefix("prompt:"))
    {
        let (label, default) = spec.split_once('=').unwrap_or((spec, ""));
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 64 {
            return Some(Err(TemplateError::InvalidField));
        }
        return Some(Ok(FormField {
            key: format!("field:{label}"),
            label: label.to_owned(),
            kind: FormFieldKind::Text {
                default: default.to_owned(),
            },
        }));
    }
    let spec = name.strip_prefix("choice:")?;
    let options: Vec<String> = spec
        .split('|')
        .map(|option| option.trim().to_owned())
        .collect();
    if options.len() < 2 || options.len() > 32 || options.iter().any(String::is_empty) {
        return Some(Err(TemplateError::InvalidField));
    }
    Some(Ok(FormField {
        key: format!("choice:{}", options.join("|")),
        label: "Choice".to_owned(),
        kind: FormFieldKind::Choice { options },
    }))
}

/// The form fields a template asks for, in order of first appearance and
/// without duplicates. An invalid marker is an error.
pub fn form_fields(template: &str) -> Result<Vec<FormField>, TemplateError> {
    let mut fields: Vec<FormField> = Vec::new();
    for name in template_variables(template) {
        if let Some(field) = parse_form_field(name) {
            let field = field?;
            if !fields.iter().any(|existing| existing.key == field.key) {
                fields.push(field);
            }
        }
    }
    if fields.len() > 32 {
        return Err(TemplateError::InvalidField);
    }
    Ok(fields)
}

impl TemplateContext {
    /// Build the deliberately small, deterministic set of built-in values.
    /// External commands are not executed while rendering a snippet.
    pub fn system() -> Self {
        let username = SYSTEM_USERNAME
            .get_or_init(|| env::var("USER").unwrap_or_default())
            .clone();
        let hostname = SYSTEM_HOSTNAME
            .get_or_init(|| {
                fs::read_to_string("/etc/hostname")
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            })
            .clone();
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
        "the clipboard variable is disabled (settings.allow_clipboard, or organization policy)"
    )]
    ClipboardDisabled,
    #[error("the clipboard could not be read")]
    ClipboardUnavailable,
    #[error("a form field is malformed (field:name, field:name=default, or choice:A|B with 2-32 options)")]
    InvalidField,
    #[error("a form field has no value")]
    FieldValueMissing,
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
    render_nested(template, context, true, 0, maximum_bytes, true).map(|(rendered, _, _)| rendered)
}

/// Every `{{...}}` variable name in a template, in order. Unclosed markers
/// end the scan; rendering reports them as errors.
pub fn template_variables(template: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut cursor = 0;
    while let Some(start) = template[cursor..].find("{{") {
        let variable_start = cursor + start + 2;
        let Some(end) = template[variable_start..].find("}}") else {
            break;
        };
        names.push(template[variable_start..variable_start + end].trim());
        cursor = variable_start + end + 2;
    }
    names
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
    render_nested(template, context, allow_cursor, 0, maximum_bytes, false)
        .map(|(rendered, cursor, _)| (rendered, cursor))
}

fn render_nested(
    template: &str,
    context: &TemplateContext,
    allow_cursor: bool,
    depth: usize,
    maximum_bytes: usize,
    preview: bool,
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
                rendered.push_str(&remaining_template[..newline]);
                return Ok((rendered, cursor_position, true));
            }
        }

        let Some(relative_start) =
            relative_start.filter(|start| !preview || *start < remaining_output)
        else {
            let stopped = push_bounded(&mut rendered, remaining_template, maximum_bytes, preview)?;
            if stopped {
                return Ok((rendered, cursor_position, true));
            }
            break;
        };
        let start = cursor + relative_start;
        let stopped = push_bounded(
            &mut rendered,
            &template[cursor..start],
            maximum_bytes,
            preview,
        )?;
        if stopped {
            return Ok((rendered, cursor_position, true));
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
            "date" => format_date(context.unix_timestamp),
            "time" => format_time(context.unix_timestamp),
            "datetime" => format!(
                "{}T{}Z",
                format_date(context.unix_timestamp),
                format_time(context.unix_timestamp)
            ),
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
                    || other.starts_with("choice:") =>
            {
                let field = parse_form_field(other).expect("prefix checked")?;
                match context.fields.get(&field.key) {
                    Some(value) => value.clone(),
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
                let (rendered, _, stopped) =
                    render_nested(included, context, false, depth + 1, include_budget, preview)?;
                included_stopped = stopped;
                rendered
            }
            other => match parse_offset_variable(other) {
                Some((base, offset)) => {
                    let adjusted = context
                        .unix_timestamp
                        .checked_add_signed(offset)
                        .ok_or_else(|| TemplateError::DateArithmeticOverflow {
                            name: other.to_owned(),
                        })?;
                    match base {
                        "date" => format_date(adjusted),
                        "time" => format_time(adjusted),
                        "datetime" => {
                            format!("{}T{}Z", format_date(adjusted), format_time(adjusted))
                        }
                        _ => {
                            return Err(TemplateError::UnknownVariable {
                                name: other.to_owned(),
                            })
                        }
                    }
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

fn format_time(timestamp: u64) -> String {
    let seconds = timestamp % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

pub(crate) fn format_date(timestamp: u64) -> String {
    let days = (timestamp / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

// Gregorian UTC conversion, adapted from the public-domain civil_from_days
// algorithm. Keeping this local avoids a runtime dependency or shell command.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
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
}
