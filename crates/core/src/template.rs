use std::{env, fs};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

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
    #[error("date arithmetic overflow for template variable {name:?}")]
    DateArithmeticOverflow { name: String },
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
        push_bounded(&mut rendered, &value)?;
        cursor = end + 2;
    }
    Ok(rendered)
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

/// Splits `template` on the first literal `{{cursor}}` marker (if any),
/// renders each half independently through [`render_template`], and
/// reports how many grapheme clusters follow the marker in the rendered text --
/// the offset callers use to move the cursor back after typing the result.
/// `{{cursor}}` is deliberately not a variable inside `render_template`
/// itself (it substitutes to nothing; it only marks a position), so
/// splitting around it here keeps that function's "every `{{...}}` is a
/// known variable" guarantee unchanged for everything else.
///
/// Used both by config validation (to accept `{{cursor}}` before
/// activation) and by the engine (to actually render it), so the two
/// agree on what is valid without duplicating the splitting logic.
pub fn render_template_with_cursor(
    template: &str,
    context: &TemplateContext,
) -> Result<(String, Option<usize>), TemplateError> {
    const MARKER: &str = "{{cursor}}";
    let Some(marker_start) = template.find(MARKER) else {
        return render_template(template, context).map(|rendered| (rendered, None));
    };
    let mut rendered = render_template(&template[..marker_start], context)?;
    let after = render_template(&template[marker_start + MARKER.len()..], context)?;
    let cursor_offset = after.graphemes(true).count();
    rendered.push_str(&after);
    Ok((rendered, Some(cursor_offset)))
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
}
