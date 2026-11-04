//! Form values, field validation, and marker parsing.

use super::{format_clock, template_variables, TemplateContext, TemplateError};
use std::collections::HashMap;
use thiserror::Error;

/// Values a user typed into a snippet form, keyed by field key. They are
/// wiped from memory when the last owner drops them, and `Debug` never
/// prints them.
#[derive(Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub struct FormValues(HashMap<String, String>);

impl FormValues {
    /// Insert a value, wiping any value it replaces. Returns whether the key
    /// was already present.
    pub fn insert(&mut self, key: String, value: String) -> bool {
        match self.0.insert(key, value) {
            Some(mut previous) => {
                zeroize::Zeroize::zeroize(&mut previous);
                true
            }
            None => false,
        }
    }
}

impl std::ops::Deref for FormValues {
    type Target = HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<HashMap<String, String>> for FormValues {
    fn from(values: HashMap<String, String>) -> Self {
        Self(values)
    }
}

impl FromIterator<(String, String)> for FormValues {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl std::fmt::Debug for FormValues {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_set()
            .entries(self.0.keys())
            .finish_non_exhaustive()
    }
}

impl Drop for FormValues {
    fn drop(&mut self) {
        for value in self.0.values_mut() {
            zeroize::Zeroize::zeroize(value);
        }
    }
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
    /// Optional domain constraint shared by terminal prompts, GUI forms and
    /// template rendering.
    #[serde(default)]
    pub validation: Option<FormFieldValidation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormFieldValidation {
    DnsLabel,
    Hostname,
    HostPort,
    Port,
    SystemdUnit,
    PositiveInteger,
    LinuxUser,
    AnsibleRoleName,
    AbsolutePath,
    SystemdExecutable,
    YamlString,
    AnsibleHostPattern,
    ComposeService,
    ContainerImage,
    SingleLine,
    ShellSingleQuoted,
}

impl FormFieldValidation {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "dns-label" => Self::DnsLabel,
            "hostname" => Self::Hostname,
            "host-port" => Self::HostPort,
            "port" => Self::Port,
            "systemd-unit" => Self::SystemdUnit,
            "positive-integer" => Self::PositiveInteger,
            "linux-user" => Self::LinuxUser,
            "ansible-role-name" => Self::AnsibleRoleName,
            "absolute-path" => Self::AbsolutePath,
            "systemd-executable" => Self::SystemdExecutable,
            "yaml-string" => Self::YamlString,
            "ansible-host-pattern" => Self::AnsibleHostPattern,
            "compose-service" => Self::ComposeService,
            "container-image" => Self::ContainerImage,
            "single-line" => Self::SingleLine,
            "shell-single-quoted" => Self::ShellSingleQuoted,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::DnsLabel => "DNS label",
            Self::Hostname => "hostname or IP address",
            Self::HostPort => "hostname or IP address followed by a valid port",
            Self::Port => "port from 1 to 65535",
            Self::SystemdUnit => "systemd unit name",
            Self::PositiveInteger => "positive integer",
            Self::LinuxUser => "Linux user name",
            Self::AnsibleRoleName => "Ansible role name",
            Self::AbsolutePath => "absolute path without parent traversal",
            Self::SystemdExecutable => "absolute executable path without arguments",
            Self::YamlString => "single-line YAML string without quote or template syntax",
            Self::AnsibleHostPattern => "single-line Ansible host pattern",
            Self::ComposeService => "Docker Compose service name",
            Self::ContainerImage => "container image reference",
            Self::SingleLine => "single-line value",
            Self::ShellSingleQuoted => "single-line value safe inside shell single quotes",
        }
    }

    fn accepts(self, value: &str) -> bool {
        match self {
            Self::DnsLabel => valid_dns_label(value, false),
            Self::Hostname => valid_host(value),
            Self::HostPort => valid_host_port(value),
            Self::Port => valid_port(value),
            Self::SystemdUnit => {
                !value.is_empty()
                    && value.len() <= 255
                    && value.as_bytes()[0].is_ascii_alphanumeric()
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.@:-".contains(&byte))
            }
            Self::PositiveInteger => value.parse::<u32>().is_ok_and(|number| number > 0),
            Self::LinuxUser => {
                !value.is_empty()
                    && value.len() <= 32
                    && (value.as_bytes()[0].is_ascii_alphabetic() || value.as_bytes()[0] == b'_')
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            }
            Self::AnsibleRoleName => {
                !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            }
            Self::AbsolutePath => {
                let path = std::path::Path::new(value);
                path.is_absolute()
                    && !value.chars().any(|character| {
                        character.is_control() || matches!(character, '"' | '\\' | '{' | '}')
                    })
                    && path
                        .components()
                        .all(|component| !matches!(component, std::path::Component::ParentDir))
            }
            Self::SystemdExecutable => {
                value.starts_with('/')
                    && !value.chars().any(char::is_whitespace)
                    && value
                        .chars()
                        .all(|character| !character.is_control() && character != '%')
            }
            Self::YamlString => {
                !value.is_empty()
                    && value.chars().all(|character| {
                        !character.is_control() && !matches!(character, '"' | '\\' | '{' | '}')
                    })
            }
            Self::AnsibleHostPattern => {
                !value.is_empty()
                    && value.len() <= 512
                    && value.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "._-:&!,[]*".contains(character)
                    })
            }
            Self::ComposeService => {
                !value.is_empty()
                    && value.len() <= 63
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            }
            Self::ContainerImage => {
                !value.is_empty()
                    && value.len() <= 512
                    && value.as_bytes()[0].is_ascii_alphanumeric()
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._/:@-".contains(&byte))
                    && !value.contains("..")
            }
            Self::SingleLine => !value.chars().any(char::is_control),
            Self::ShellSingleQuoted => {
                !value.is_empty()
                    && !value
                        .chars()
                        .any(|character| character.is_control() || character == '\'')
            }
        }
    }
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
    #[error("value must be a valid {0}")]
    InvalidFormat(&'static str),
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

pub(super) fn local_date_at(timestamp: u64) -> Result<String, DateError> {
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
                if let Some(validation) = self.validation {
                    if !validation.accepts(value) {
                        return Err(FormValueError::InvalidFormat(validation.label()));
                    }
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

fn valid_dns_label(value: &str, allow_uppercase: bool) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || (allow_uppercase && byte.is_ascii_uppercase())
                || byte.is_ascii_digit()
                || byte == b'-'
        })
}

fn valid_host(value: &str) -> bool {
    if value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let hostname = value.strip_suffix('.').unwrap_or(value);
    !hostname.is_empty()
        && hostname.len() <= 253
        && hostname
            .split('.')
            .all(|label| valid_dns_label(label, true))
}

fn valid_port(value: &str) -> bool {
    value.parse::<u16>().is_ok_and(|port| port > 0)
}

fn valid_host_port(value: &str) -> bool {
    if let Some(bracketed) = value.strip_prefix('[') {
        let Some((address, port)) = bracketed.split_once("]:") else {
            return false;
        };
        return address.parse::<std::net::Ipv6Addr>().is_ok() && valid_port(port);
    }
    value
        .rsplit_once(':')
        .is_some_and(|(host, port)| valid_host(host) && valid_port(port))
}

#[derive(Default)]
struct ParsedFormAttributes {
    required: bool,
    min_chars: Option<usize>,
    max_chars: Option<usize>,
    validation: Option<FormFieldValidation>,
}

fn parse_form_attributes(
    attributes: &[&str],
    allow_length: bool,
) -> Result<ParsedFormAttributes, TemplateError> {
    let mut required = false;
    let mut min_chars = None;
    let mut max_chars = None;
    let mut validation = None;
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
        } else if let Some(value) = attribute.strip_prefix("validate=") {
            if !allow_length || validation.is_some() {
                return Err(TemplateError::InvalidField);
            }
            validation =
                Some(FormFieldValidation::parse(value).ok_or(TemplateError::InvalidField)?);
        } else {
            return Err(TemplateError::InvalidField);
        }
    }
    if min_chars.zip(max_chars).is_some_and(|(min, max)| min > max) {
        return Err(TemplateError::InvalidField);
    }
    Ok(ParsedFormAttributes {
        required,
        min_chars,
        max_chars,
        validation,
    })
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
pub(super) fn parse_form_field(name: &str) -> Option<Result<FormField, TemplateError>> {
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
        let ParsedFormAttributes {
            required,
            min_chars,
            max_chars,
            validation,
        } = match validation {
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
            validation,
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
        let ParsedFormAttributes {
            required,
            min_chars,
            max_chars,
            validation,
        } = match parse_form_attributes(&attributes, false) {
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
            validation,
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
        validation: None,
    }))
}

/// The form fields a template asks for, in order of first appearance and
/// without duplicates. An invalid marker is an error.
pub fn form_fields(template: &str) -> Result<Vec<FormField>, TemplateError> {
    let mut fields: Vec<FormField> = Vec::new();
    // Whether each field so far comes only from bare references.
    let mut bare_only: Vec<bool> = Vec::new();
    for name in template_variables(template) {
        let shellq_field = name
            .strip_prefix("shellq:")
            .map(|spec| format!("field:{spec}"));
        let field_name = shellq_field.as_deref().unwrap_or(name);
        if let Some(field) = parse_form_field(field_name) {
            let field = field?;
            let bare = is_bare_form_field_reference(field_name);
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
    let name = name.strip_prefix("shellq:").unwrap_or(name);
    ["field:", "prompt:", "datefield:"]
        .iter()
        .filter_map(|prefix| name.strip_prefix(prefix))
        .any(|definition| !definition.contains('=') && !definition.contains('|'))
}
