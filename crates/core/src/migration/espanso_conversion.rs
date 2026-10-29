//! Translation of supported Espanso variables and forms into WayExpand templates.

use super::{
    EspansoChoiceValues, EspansoFormField, EspansoFormValues, EspansoOption, EspansoVariable,
};
use std::collections::BTreeMap;

pub(super) fn convert_espanso_variables<'a>(
    template: &str,
    variables: impl Iterator<Item = &'a EspansoVariable>,
) -> Result<(String, Vec<String>), String> {
    let referenced = crate::template_variables(template);
    let mut substitutions = BTreeMap::new();
    let mut details = Vec::new();
    for variable in variables {
        if !referenced
            .iter()
            .any(|name| name.trim() == variable.name.as_str())
        {
            continue;
        }
        if matches!(variable.kind.as_str(), "shell" | "script") {
            return Err(format!(
                "{} variable `{}` executes external code and was not imported",
                variable.kind, variable.name
            ));
        }
        if !valid_variable_name(&variable.name) {
            return Err(format!(
                "dynamic variable name {:?} cannot be represented safely",
                variable.name
            ));
        }
        if !variable.extra.is_empty() {
            return Err(format!(
                "dynamic variable `{}` has unmapped options: {}",
                variable.name,
                variable
                    .extra
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !variable.params.extra.is_empty() {
            return Err(format!(
                "{} variable `{}` uses unsupported parameters: {}",
                variable.kind,
                variable.name,
                variable
                    .params
                    .extra
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let value = match variable.kind.as_str() {
            "echo" => {
                let value = variable.params.echo.as_deref().ok_or_else(|| {
                    format!("echo variable `{}` has no `echo` value", variable.name)
                })?;
                escape_template_literal(value)
            }
            "date" => {
                let offset = variable.params.offset.unwrap_or(0);
                let suffix = if offset == 0 {
                    String::new()
                } else {
                    let (magnitude, unit) = if offset % 86_400 == 0 {
                        (offset / 86_400, 'd')
                    } else if offset % 3_600 == 0 {
                        (offset / 3_600, 'h')
                    } else if offset % 60 == 0 {
                        (offset / 60, 'm')
                    } else {
                        return Err(format!(
                            "date variable `{}` has an offset that is not an exact minute, hour, or day",
                            variable.name
                        ));
                    };
                    format!("{}{magnitude}{unit}", if magnitude < 0 { "" } else { "+" })
                };
                let format = variable.params.format.as_deref();
                let marker = match format {
                    Some(format) => format!("{{{{local_date{suffix}:{format}}}}}"),
                    None if suffix.is_empty() => "{{local_date}}".to_owned(),
                    None => format!("{{{{local_date{suffix}:%Y-%m-%d}}}}"),
                };
                crate::render_template(&marker, &crate::TemplateContext::default()).map_err(
                    |_| {
                        format!(
                            "date variable `{}` uses an unsupported format",
                            variable.name
                        )
                    },
                )?;
                marker
            }
            "choice" => {
                let choices = variable.params.values.as_ref().ok_or_else(|| {
                    format!("choice variable `{}` has no `values` list", variable.name)
                })?;
                let EspansoChoiceValues::List(choices) = choices else {
                    return Err(format!(
                        "choice variable `{}` uses structured values that need manual conversion",
                        variable.name
                    ));
                };
                let values = option_strings(choices, &variable.name)?;
                if !(2..=32).contains(&values.len()) {
                    return Err(format!(
                        "choice variable `{}` needs between 2 and 32 string values",
                        variable.name
                    ));
                }
                format!("{{{{choice:{}={}}}}}", variable.name, values.join("|"))
            }
            "random" => {
                let choices = variable.params.choices.as_ref().ok_or_else(|| {
                    format!("random variable `{}` has no `choices` list", variable.name)
                })?;
                let EspansoChoiceValues::List(choices) = choices else {
                    return Err(format!(
                        "random variable `{}` uses structured choices that need manual conversion",
                        variable.name
                    ));
                };
                let values = option_strings(choices, &variable.name)?;
                if values.is_empty() || values.len() > 64 {
                    return Err(format!(
                        "random variable `{}` needs between 1 and 64 string values",
                        variable.name
                    ));
                }
                format!("{{{{random:{}}}}}", values.join("|"))
            }
            "match" => {
                let trigger = variable.params.trigger.as_deref().ok_or_else(|| {
                    format!("nested match variable `{}` has no trigger", variable.name)
                })?;
                if trigger.is_empty()
                    || trigger.contains(['{', '}'])
                    || trigger.chars().any(char::is_control)
                {
                    return Err(format!(
                        "nested match variable `{}` has a trigger that cannot be represented safely",
                        variable.name
                    ));
                }
                format!("{{{{snippet:{trigger}}}}}")
            }
            "clipboard" => {
                return Err(format!(
                    "clipboard variable `{}` was not imported because WayExpand requires explicit clipboard enablement",
                    variable.name
                ));
            }
            other => {
                return Err(format!(
                    "dynamic variable type `{other}` for `{}` has no safe automatic conversion",
                    variable.name
                ));
            }
        };
        if variable.kind == "date" && variable.params.offset.is_some() {
            details.push(format!(
                "date variable `{}` uses WayExpand local-time formatting; verify timezone and daylight-saving behavior",
                variable.name
            ));
        }
        substitutions.insert(variable.name.clone(), value);
    }
    Ok((
        substitute_template_variables(template, &substitutions),
        details,
    ))
}

fn option_strings(options: &[EspansoOption], name: &str) -> Result<Vec<String>, String> {
    let values = options
        .iter()
        .map(|option| match option {
            EspansoOption::Text(value) => Ok(value.clone()),
            EspansoOption::Unsupported => Err(format!(
                "dynamic variable `{name}` uses structured or non-string choices that need manual conversion"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.iter().any(|value| {
        value.trim() != value
            || value.contains(['|', ';'])
            || value.contains("{{")
            || value.contains("}}")
            || value.contains("$|$")
    }) {
        return Err(format!(
            "dynamic variable `{name}` has choices that cannot be represented without changing their values"
        ));
    }
    Ok(values)
}

fn valid_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn substitute_template_variables(
    template: &str,
    substitutions: &BTreeMap<String, String>,
) -> String {
    let mut output = String::with_capacity(template.len());
    let mut cursor = 0;
    while cursor < template.len() {
        let remaining = &template[cursor..];
        if remaining.starts_with("{{{{") || remaining.starts_with("}}}}") {
            output.push_str(&remaining[..4]);
            cursor += 4;
            continue;
        }
        let Some(start) = remaining.find("{{") else {
            output.push_str(remaining);
            break;
        };
        output.push_str(&remaining[..start]);
        let marker_start = cursor + start;
        let value_start = marker_start + 2;
        let Some(end) = template[value_start..].find("}}") else {
            output.push_str(&template[marker_start..]);
            break;
        };
        let marker_end = value_start + end + 2;
        let name = template[value_start..value_start + end].trim();
        if let Some(value) = substitutions.get(name) {
            output.push_str(value);
        } else {
            output.push_str(&template[marker_start..marker_end]);
        }
        cursor = marker_end;
    }
    output
}

fn escape_template_literal(value: &str) -> String {
    value.replace("{{", "{{{{").replace("}}", "}}}}")
}

pub(super) fn convert_espanso_cursor_marker(template: &str) -> Result<String, String> {
    let count = template.matches("$|$").count();
    if count > 1 {
        return Err("match contains more than one Espanso cursor marker".into());
    }
    Ok(if count == 1 {
        template.replacen("$|$", "{{cursor}}", 1)
    } else {
        template.to_owned()
    })
}

pub(super) fn convert_espanso_form(
    layout: &str,
    fields: &BTreeMap<String, EspansoFormField>,
) -> Result<(String, Vec<String>), String> {
    let mut output = String::with_capacity(layout.len());
    let mut details = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    let mut cursor = 0;
    while cursor < layout.len() {
        let remaining = &layout[cursor..];
        let Some(start) = remaining.find("[[") else {
            output.push_str(&escape_template_literal(remaining));
            break;
        };
        output.push_str(&escape_template_literal(&remaining[..start]));
        let marker_start = cursor + start;
        let content_start = marker_start + 2;
        let Some(end) = layout[content_start..].find("]]") else {
            return Err("form layout contains an unclosed `[[field]]` marker".into());
        };
        let name = layout[content_start..content_start + end].trim();
        if !valid_form_field_name(name) {
            return Err(format!(
                "form field name {name:?} cannot be represented safely"
            ));
        }
        used.insert(name.to_owned());
        let field = fields.get(name);
        let marker = match field.and_then(|field| field.kind.as_deref()) {
            None | Some("text") => {
                if let Some(field) = field {
                    if field.multiline == Some(true) {
                        details.push(format!(
                            "multiline form field `{name}` became a single-line text field"
                        ));
                    }
                    if !field.extra.is_empty() {
                        details.push(format!(
                            "form field `{name}` has unmapped options: {}",
                            field.extra.keys().cloned().collect::<Vec<_>>().join(", ")
                        ));
                    }
                    let default = field.default.as_deref().unwrap_or_default();
                    if !default.is_empty()
                        && (default.contains(['|', ';'])
                            || default.contains("{{")
                            || default.contains("}}")
                            || default.contains("$|$"))
                    {
                        return Err(format!("form field `{name}` default cannot be represented without changing its value"));
                    }
                    if default.is_empty() {
                        format!("{{{{field:{name}}}}}")
                    } else {
                        format!("{{{{field:{name}={default}}}}}")
                    }
                } else {
                    format!("{{{{field:{name}}}}}")
                }
            }
            Some("choice") => {
                let values = field
                    .and_then(|field| field.values.as_ref())
                    .ok_or_else(|| format!("choice form field `{name}` has no values"))?;
                let values = match values {
                    EspansoFormValues::List(values) => values.clone(),
                    EspansoFormValues::Lines(values) => values
                        .lines()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .collect(),
                };
                if !(2..=32).contains(&values.len())
                    || values.iter().any(|value| {
                        value.trim() != value
                            || value.contains(['|', ';'])
                            || value.contains("{{")
                            || value.contains("}}")
                            || value.contains("$|$")
                    })
                {
                    return Err(format!(
                        "choice form field `{name}` has values that cannot be represented safely"
                    ));
                }
                let default = field.and_then(|field| field.default.as_deref());
                if default.is_some_and(|default| !values.iter().any(|value| value == default)) {
                    return Err(format!(
                        "choice form field `{name}` default is not one of its options"
                    ));
                }
                let default = default
                    .map(|value| format!(";default={value}"))
                    .unwrap_or_default();
                if let Some(field) = field {
                    if !field.extra.is_empty() {
                        details.push(format!(
                            "form field `{name}` has unmapped options: {}",
                            field.extra.keys().cloned().collect::<Vec<_>>().join(", ")
                        ));
                    }
                }
                format!("{{{{choice:{name}={}{default}}}}}", values.join("|"))
            }
            Some(other) => {
                return Err(format!(
                    "form field `{name}` uses unsupported control type `{other}`"
                ))
            }
        };
        output.push_str(&marker);
        cursor = content_start + end + 2;
    }
    if fields.keys().any(|name| !used.contains(name)) {
        return Err("form_fields contains definitions not referenced by the form layout".into());
    }
    Ok((output, details))
}

fn valid_form_field_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && !name.contains(['=', '|', ';', '{', '}'])
        && !name.chars().any(char::is_control)
}
