//! Typed helpers for GNOME's `org.gnome.desktop.input-sources` settings.
//! `sources` is a GVariant `a(ss)` tuple array, not an `as` string list.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSource {
    pub kind: String,
    pub id: String,
}

pub const WAYEXPAND_SOURCE: InputSource = InputSource {
    kind: String::new(),
    id: String::new(),
};

pub fn parse_sources(value: &str) -> Result<Vec<InputSource>, String> {
    let mut parser = Parser {
        chars: value.chars().collect(),
        at: 0,
    };
    parser.ws();
    if parser.take_word("@a(ss)") {
        parser.ws();
    }
    parser.expect('[')?;
    let mut sources = Vec::new();
    parser.ws();
    while parser.peek() != Some(']') {
        parser.expect('(')?;
        let kind = parser.string()?;
        parser.expect(',')?;
        let id = parser.string()?;
        parser.expect(')')?;
        sources.push(InputSource { kind, id });
        parser.ws();
        if parser.peek() == Some(',') {
            parser.at += 1;
            parser.ws();
        } else {
            break;
        }
    }
    parser.expect(']')?;
    parser.ws();
    if parser.peek().is_some() {
        return Err("unexpected trailing data in GNOME input-source array".into());
    }
    Ok(sources)
}

pub fn format_sources(sources: &[InputSource]) -> String {
    let values = sources
        .iter()
        .map(|s| format!("({}, {})", quote(&s.kind), quote(&s.id)))
        .collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

pub fn parse_current(value: &str) -> Result<u32, String> {
    value
        .trim()
        .strip_prefix("uint32 ")
        .unwrap_or(value.trim())
        .parse()
        .map_err(|_| "invalid GNOME input-source current index".into())
}

pub fn wayexpand_index(sources: &[InputSource]) -> Option<usize> {
    sources
        .iter()
        .position(|source| source.kind == "ibus" && source.id == "wayexpand")
}

/// Append WayExpand without disturbing any existing GNOME source or order.
pub fn ensure_wayexpand_source(sources: &mut Vec<InputSource>) -> usize {
    if let Some(index) = wayexpand_index(sources) {
        return index;
    }
    sources.push(InputSource {
        kind: "ibus".into(),
        id: "wayexpand".into(),
    });
    sources.len() - 1
}

fn quote(value: &str) -> String {
    let escaped = value
        .chars()
        .flat_map(|c| match c {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\'' => "\\'".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            c => vec![c],
        })
        .collect::<String>();
    format!("'{escaped}'")
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}
impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
    fn ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }
    fn expect(&mut self, expected: char) -> Result<(), String> {
        self.ws();
        if self.peek() == Some(expected) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {expected:?} in GNOME input-source array"))
        }
    }
    fn take_word(&mut self, word: &str) -> bool {
        let end = self.at + word.chars().count();
        if self
            .chars
            .get(self.at..end)
            .is_some_and(|slice| slice.iter().copied().eq(word.chars()))
        {
            self.at = end;
            true
        } else {
            false
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.ws();
        if self.peek() != Some('\'') {
            return Err("expected a quoted string in GNOME input-source tuple".into());
        }
        self.at += 1;
        let mut result = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string in GNOME input-source tuple".into()),
                Some('\'') => {
                    self.at += 1;
                    return Ok(result);
                }
                Some('\\') => {
                    self.at += 1;
                    let escaped = self
                        .peek()
                        .ok_or("unterminated escape in GNOME input-source tuple")?;
                    self.at += 1;
                    result.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' => '\\',
                        '\'' => '\'',
                        other => other,
                    });
                }
                Some(c) => {
                    self.at += 1;
                    result.push(c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_typed_tuple_array_and_preserves_order() {
        let parsed = parse_sources("@a(ss) [('xkb', 'us'), ('ibus', 'wayexpand')]").unwrap();
        assert_eq!(
            parsed,
            vec![
                InputSource {
                    kind: "xkb".into(),
                    id: "us".into()
                },
                InputSource {
                    kind: "ibus".into(),
                    id: "wayexpand".into()
                }
            ]
        );
        assert_eq!(wayexpand_index(&parsed), Some(1));
    }
    #[test]
    fn parses_empty_array_and_rejects_flat_strings() {
        assert!(parse_sources("@a(ss) []").unwrap().is_empty());
        assert!(parse_sources("['xkb', 'us']").is_err());
    }
    #[test]
    fn formatting_roundtrips_escaped_unicode() {
        let input = vec![InputSource {
            kind: "xkb".into(),
            id: "de'\nü".into(),
        }];
        assert_eq!(parse_sources(&format_sources(&input)).unwrap(), input);
    }
    #[test]
    fn parses_current_index() {
        assert_eq!(parse_current("uint32 2").unwrap(), 2);
    }

    #[test]
    fn ensure_registration_is_idempotent_and_preserves_existing_sources() {
        let keyboard = InputSource {
            kind: "xkb".into(),
            id: "us".into(),
        };
        let mut sources = vec![keyboard.clone()];
        assert_eq!(ensure_wayexpand_source(&mut sources), 1);
        assert_eq!(ensure_wayexpand_source(&mut sources), 1);
        assert_eq!(
            sources,
            vec![
                keyboard,
                InputSource {
                    kind: "ibus".into(),
                    id: "wayexpand".into(),
                }
            ]
        );
    }
}
