use std::{env, fs};
use thiserror::Error;

const MAX_RENDERED_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub username: String,
    pub hostname: String,
    pub unix_timestamp: u64,
}

impl TemplateContext {
    /// Build the deliberately small, deterministic set of built-in values.
    /// External commands are not executed while rendering a snippet.
    pub fn system() -> Self {
        let username = env::var("USER").unwrap_or_default();
        let hostname = fs::read_to_string("/etc/hostname")
            .unwrap_or_default()
            .trim()
            .to_owned();
        let unix_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        Self {
            username,
            hostname,
            unix_timestamp,
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
}

/// Render built-in variables without invoking a shell or external process.
/// Unknown variables are errors so a typo can never silently reach an editor.
pub fn render_template(template: &str, context: &TemplateContext) -> Result<String, TemplateError> {
    let mut rendered = String::with_capacity(template.len());
    let mut cursor = 0;
    while cursor < template.len() {
        let Some(relative_start) = template[cursor..].find("{{") else {
            push_bounded(&mut rendered, &template[cursor..])?;
            break;
        };
        let start = cursor + relative_start;
        push_bounded(&mut rendered, &template[cursor..start])?;
        let variable_start = start + 2;
        let Some(relative_end) = template[variable_start..].find("}}") else {
            return Err(TemplateError::Unclosed { offset: start });
        };
        let end = variable_start + relative_end;
        let name = template[variable_start..end].trim();
        if name.is_empty() {
            return Err(TemplateError::EmptyVariable { offset: start });
        }
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
            other => {
                return Err(TemplateError::UnknownVariable {
                    name: other.to_owned(),
                })
            }
        };
        push_bounded(&mut rendered, &value)?;
        cursor = end + 2;
    }
    Ok(rendered)
}

fn push_bounded(output: &mut String, value: &str) -> Result<(), TemplateError> {
    if output.len().saturating_add(value.len()) > MAX_RENDERED_BYTES {
        return Err(TemplateError::RenderedTooLarge {
            maximum: MAX_RENDERED_BYTES,
        });
    }
    output.push_str(value);
    Ok(())
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

fn format_date(timestamp: u64) -> String {
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
    fn renders_builtins_without_shelling_out() {
        let context = TemplateContext {
            username: "ada".into(),
            hostname: "workstation".into(),
            unix_timestamp: 0,
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
}
