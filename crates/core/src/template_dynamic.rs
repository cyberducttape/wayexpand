//! Formatted dates, UUIDs, and random values for templates.
//!
//! Formatting is a fixed strftime subset with English month and day names,
//! so a snippet renders the same on every machine and never depends on the
//! process locale. Random values come from the operating system's CSPRNG;
//! if it is unavailable the snippet does not render, rather than falling
//! back to something predictable that might be used as a password.

use crate::template::TemplateError;

/// Longest date format accepted, in bytes.
const MAX_DATE_FORMAT_BYTES: usize = 128;
/// Longest random string, in characters.
pub(crate) const MAX_RANDOM_CHARS: usize = 256;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Format the instant `timestamp` (seconds since the epoch, UTC) shifted by
/// `utc_offset` seconds, with `format`. `utc_offset` is `None` for UTC.
pub(crate) fn format_date(
    format: &str,
    timestamp: i64,
    utc_offset: Option<i64>,
) -> Result<String, TemplateError> {
    let invalid = || TemplateError::InvalidDateFormat {
        format: format.to_owned(),
    };
    if format.is_empty() || format.len() > MAX_DATE_FORMAT_BYTES {
        return Err(invalid());
    }
    let offset = utc_offset.unwrap_or(0);
    let local = timestamp.checked_add(offset).ok_or_else(invalid)?;
    let days = local.div_euclid(86_400);
    let seconds = local.rem_euclid(86_400);
    let (year, month, day) = crate::template::civil_from_days(days);
    let (hour, minute, second) = (seconds / 3_600, (seconds % 3_600) / 60, seconds % 60);
    // 1970-01-01 was a Thursday; index 0 is Monday.
    let weekday = (days + 3).rem_euclid(7) as usize;
    let day_of_year = day_of_year(year, month, day);
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };

    let mut out = String::with_capacity(format.len() * 2);
    let mut chars = format.chars();
    while let Some(character) = chars.next() {
        if character != '%' {
            out.push(character);
            continue;
        }
        // `%-d` drops the padding, as GNU date does.
        let mut spec = chars.next().ok_or_else(invalid)?;
        let unpadded = spec == '-';
        if unpadded {
            spec = chars.next().ok_or_else(invalid)?;
        }
        let number = |value: i64, width: usize| {
            if unpadded {
                value.to_string()
            } else {
                format!("{value:0width$}")
            }
        };
        let text = match spec {
            'Y' => year.to_string(),
            'y' => number(year.rem_euclid(100), 2),
            'm' => number(month, 2),
            'd' => number(day, 2),
            'e' => {
                if unpadded {
                    day.to_string()
                } else {
                    format!("{day:2}")
                }
            }
            'H' => number(hour, 2),
            'I' => number(hour12, 2),
            'M' => number(minute, 2),
            'S' => number(second, 2),
            'p' => (if hour < 12 { "AM" } else { "PM" }).to_owned(),
            'A' => WEEKDAYS[weekday].to_owned(),
            'a' => WEEKDAYS[weekday][..3].to_owned(),
            'B' => MONTHS[(month - 1) as usize].to_owned(),
            'b' | 'h' => MONTHS[(month - 1) as usize][..3].to_owned(),
            'j' => number(day_of_year, 3),
            'u' => (weekday + 1).to_string(),
            'w' => ((weekday + 1) % 7).to_string(),
            'F' => format!("{year:04}-{month:02}-{day:02}"),
            'T' => format!("{hour:02}:{minute:02}:{second:02}"),
            'R' => format!("{hour:02}:{minute:02}"),
            'D' => format!("{month:02}/{day:02}/{:02}", year.rem_euclid(100)),
            's' => timestamp.to_string(),
            'z' if !unpadded => zone(offset, false),
            ':' if !unpadded => {
                if chars.next() != Some('z') {
                    return Err(invalid());
                }
                zone(offset, true)
            }
            'Z' if !unpadded => match utc_offset {
                None => "UTC".to_owned(),
                Some(offset) => zone(offset, true),
            },
            '%' if !unpadded => "%".to_owned(),
            'n' if !unpadded => "\n".to_owned(),
            't' if !unpadded => "\t".to_owned(),
            _ => return Err(invalid()),
        };
        out.push_str(&text);
    }
    Ok(out)
}

fn zone(offset: i64, colon: bool) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.unsigned_abs() / 60;
    if colon {
        format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
    } else {
        format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
    }
}

fn day_of_year(year: i64, month: i64, day: i64) -> i64 {
    const BEFORE: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    BEFORE[(month - 1) as usize] + day + i64::from(leap && month > 2)
}

/// Fill `bytes` from the operating system's CSPRNG, failing closed.
fn random_bytes(bytes: &mut [u8]) -> Result<(), TemplateError> {
    getrandom::fill(bytes).map_err(|_| TemplateError::RandomUnavailable)
}

/// A random RFC 4122 version-4 UUID.
pub(crate) fn uuid_v4() -> Result<String, TemplateError> {
    let mut bytes = [0_u8; 16];
    random_bytes(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            text.push('-');
        }
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(text)
}

/// CSPRNG bytes drawn in batches, so a long random string costs a few
/// `getrandom` calls instead of one per character.
struct RandomPool {
    bytes: [u8; 64],
    next: usize,
}

impl RandomPool {
    fn new() -> Self {
        Self {
            bytes: [0; 64],
            next: 64,
        }
    }

    fn byte(&mut self) -> Result<u8, TemplateError> {
        if self.next == self.bytes.len() {
            random_bytes(&mut self.bytes)?;
            self.next = 0;
        }
        let byte = self.bytes[self.next];
        self.next += 1;
        Ok(byte)
    }

    /// A uniformly random index below `bound` (at most 256), by rejection
    /// so no value is more likely than another.
    fn index(&mut self, bound: usize) -> Result<usize, TemplateError> {
        debug_assert!((1..=256).contains(&bound));
        let limit = 256 - 256 % bound;
        loop {
            let byte = usize::from(self.byte()?);
            if byte < limit {
                return Ok(byte % bound);
            }
        }
    }
}

/// The characters a `{{random:KIND:LENGTH}}` kind draws from.
fn random_alphabet(kind: &str) -> Option<&'static [u8]> {
    Some(match kind {
        "hex" => b"0123456789abcdef",
        "digits" => b"0123456789",
        "lower" => b"abcdefghijklmnopqrstuvwxyz0123456789",
        "alpha" => b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
        "alnum" => b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
        // No look-alikes (l, I, O, 0, 1), and only symbols that URLs, YAML
        // values and shells take without escaping.
        "password" => b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789-_.",
        _ => return None,
    })
}

/// Render the body of a `{{random:...}}` marker: `A|B|C` picks one option,
/// `KIND:LENGTH` draws LENGTH characters of a kind.
pub(crate) fn random_value(spec: &str) -> Result<String, TemplateError> {
    let invalid = || TemplateError::InvalidRandom {
        spec: spec.to_owned(),
    };
    if spec.contains('|') {
        let options: Vec<&str> = spec.split('|').map(str::trim).collect();
        if options.len() < 2 || options.len() > 32 || options.iter().any(|option| option.is_empty())
        {
            return Err(invalid());
        }
        return Ok(options[RandomPool::new().index(options.len())?].to_owned());
    }
    let (kind, length) = spec.split_once(':').ok_or_else(invalid)?;
    let alphabet = random_alphabet(kind.trim()).ok_or_else(invalid)?;
    let length: usize = length.trim().parse().map_err(|_| invalid())?;
    if length == 0 || length > MAX_RANDOM_CHARS {
        return Err(invalid());
    }
    let mut pool = RandomPool::new();
    (0..length)
        .map(|_| {
            pool.index(alphabet.len())
                .map(|index| char::from(alphabet[index]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-09 14:05:07 UTC, a Friday.
    const FRIDAY: i64 = 1_791_554_707;

    #[test]
    fn formats_dates_with_a_strftime_subset() {
        let utc = |format| format_date(format, FRIDAY, None).unwrap();
        assert_eq!(utc("%Y-%m-%d %H:%M:%S"), "2026-10-09 14:05:07");
        assert_eq!(utc("%A %-d %B %Y"), "Friday 9 October 2026");
        assert_eq!(utc("%a, %d %b %y"), "Fri, 09 Oct 26");
        assert_eq!(utc("%-m/%-d/%Y %-I:%M %p"), "10/9/2026 2:05 PM");
        assert_eq!(utc("%e|%j|%u|%w"), " 9|282|5|5");
        assert_eq!(utc("%F %T %R %D"), "2026-10-09 14:05:07 14:05 10/09/26");
        assert_eq!(utc("%Z %z %:z %s %%"), "UTC +0000 +00:00 1791554707 %");
        assert_eq!(utc("week day%n"), "week day\n");
        let mountain = format_date("%F %H:%M %Z %z", FRIDAY, Some(-6 * 3_600)).unwrap();
        assert_eq!(mountain, "2026-10-09 08:05 -06:00 -0600");
        // Midnight and noon on a 12-hour clock.
        assert_eq!(format_date("%I %p", 0, None).unwrap(), "12 AM");
        assert_eq!(format_date("%I %p", 12 * 3_600, None).unwrap(), "12 PM");
        // Leap-year day of year.
        let leap = 951_782_400; // 2000-02-29
        assert_eq!(
            format_date("%j %F", leap + 86_400, None).unwrap(),
            "061 2000-03-01"
        );
    }

    #[test]
    fn rejects_unknown_or_truncated_date_formats() {
        for format in [
            "",
            "%",
            "%Q",
            "%-",
            "%-%",
            "%:",
            "%:x",
            "%E",
            &"x".repeat(129),
        ] {
            assert!(
                matches!(
                    format_date(format, FRIDAY, None),
                    Err(TemplateError::InvalidDateFormat { .. })
                ),
                "{format:?}"
            );
        }
    }

    #[test]
    fn uuids_are_version_4_and_distinct() {
        let first = uuid_v4().unwrap();
        let second = uuid_v4().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.len(), 36);
        let parts: Vec<&str> = first.split('-').collect();
        assert_eq!(
            parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!(matches!(
            parts[3].chars().next(),
            Some('8' | '9' | 'a' | 'b')
        ));
    }

    #[test]
    fn long_random_values_span_several_pool_refills() {
        let spec = format!("digits:{MAX_RANDOM_CHARS}");
        let value = random_value(&spec).unwrap();
        assert_eq!(value.len(), MAX_RANDOM_CHARS);
        // 256 uniform digits all but certainly include every digit; a stuck
        // or never-refilled pool would repeat a short pattern instead.
        for digit in '0'..='9' {
            assert!(value.contains(digit), "{digit} missing from {value}");
        }
        assert_ne!(value, random_value(&spec).unwrap());
    }

    #[test]
    fn random_values_have_the_requested_shape() {
        for (spec, alphabet) in [
            ("hex:32", "0123456789abcdef"),
            ("digits:6", "0123456789"),
            (
                "alnum:24",
                "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            ),
            (
                "password:20",
                "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789-_.",
            ),
        ] {
            let value = random_value(spec).unwrap();
            let length: usize = spec.split(':').nth(1).unwrap().parse().unwrap();
            assert_eq!(value.chars().count(), length, "{spec}");
            assert!(
                value.chars().all(|c| alphabet.contains(c)),
                "{spec}: {value}"
            );
        }
        assert_ne!(
            random_value("hex:32").unwrap(),
            random_value("hex:32").unwrap()
        );
        // Every option of a pick can come up, and nothing else does.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..400 {
            let value = random_value("red | green | blue").unwrap();
            assert!(["red", "green", "blue"].contains(&value.as_str()));
            seen.insert(value);
        }
        assert_eq!(seen.len(), 3);
        for spec in [
            "", "hex", "hex:0", "hex:257", "hex:x", "nope:4", "a|", "only",
        ] {
            assert!(
                matches!(random_value(spec), Err(TemplateError::InvalidRandom { .. })),
                "{spec:?}"
            );
        }
    }
}
