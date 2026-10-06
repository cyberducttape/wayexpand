/// Sentinel for a node that ends no trigger.
const NO_VALUE: u32 = u32::MAX;

/// Suffix and prefix tries over the configured triggers.
///
/// Each trie is an arena: nodes refer to a contiguous, character-sorted run
/// of edges, so a step is a binary search over one small slice instead of a
/// per-node `HashMap`. The structure is built once and never mutated.
#[derive(Debug, Clone)]
pub struct Matcher {
    root: Trie,
    forward: Trie,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new(std::iter::empty())
    }
}

#[derive(Debug, Clone)]
struct Trie {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    first_edge: u32,
    edge_count: u32,
    value: u32,
}

#[derive(Debug, Clone, Copy)]
struct Edge {
    character: char,
    target: u32,
}

/// Mutable trie used only while inserting; frozen into a [`Trie`].
struct TrieBuilder {
    /// Per node: children sorted by character, and the trigger index.
    nodes: Vec<(Vec<(char, u32)>, u32)>,
}

impl TrieBuilder {
    fn new() -> Self {
        Self {
            nodes: vec![(Vec::new(), NO_VALUE)],
        }
    }

    fn insert(&mut self, characters: impl Iterator<Item = char>, value: u32) {
        let mut node = 0usize;
        for character in characters {
            let next = self.nodes.len() as u32;
            let children = &mut self.nodes[node].0;
            node = match children.binary_search_by_key(&character, |&(c, _)| c) {
                Ok(position) => children[position].1 as usize,
                Err(position) => {
                    children.insert(position, (character, next));
                    self.nodes.push((Vec::new(), NO_VALUE));
                    next as usize
                }
            };
        }
        if value != NO_VALUE {
            self.nodes[node].1 = value;
        }
    }

    fn freeze(self) -> Trie {
        let edge_total = self.nodes.iter().map(|(children, _)| children.len()).sum();
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let mut edges = Vec::with_capacity(edge_total);
        for (children, value) in self.nodes {
            nodes.push(Node {
                first_edge: edges.len() as u32,
                edge_count: children.len() as u32,
                value,
            });
            edges.extend(
                children
                    .into_iter()
                    .map(|(character, target)| Edge { character, target }),
            );
        }
        Trie { nodes, edges }
    }
}

impl Trie {
    fn edges(&self, node: u32) -> &[Edge] {
        let node = self.nodes[node as usize];
        let start = node.first_edge as usize;
        &self.edges[start..start + node.edge_count as usize]
    }

    fn child(&self, node: u32, character: char) -> Option<u32> {
        let edges = self.edges(node);
        edges
            .binary_search_by_key(&character, |edge| edge.character)
            .ok()
            .map(|position| edges[position].target)
    }
}

impl Matcher {
    pub fn new<I>(triggers: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let mut root = TrieBuilder::new();
        let mut forward = TrieBuilder::new();
        for (index, trigger) in triggers.into_iter().enumerate() {
            // Effective triggers are capped far below u32::MAX by config
            // validation; refuse rather than wrap if a caller exceeds it.
            let index = u32::try_from(index)
                .ok()
                .filter(|&index| index != NO_VALUE)
                .expect("trigger count fits the matcher's u32 index");
            root.insert(trigger.chars().rev(), index);
            forward.insert(trigger.chars(), NO_VALUE);
        }
        Self {
            root: root.freeze(),
            forward: forward.freeze(),
        }
    }

    /// Walks the forward trie along `trigger`, returning the node it ends on.
    ///
    /// Takes the characters as an iterator so the hot path can walk a slice of
    /// the rolling `VecDeque<char>` buffer directly instead of collecting the
    /// matched suffix into a `String` on every keystroke.
    fn forward_node<I>(&self, trigger: I) -> Option<u32>
    where
        I: IntoIterator<Item = char>,
    {
        let mut node = 0;
        for character in trigger {
            node = self.forward.child(node, character)?;
        }
        Some(node)
    }

    /// Whether some configured trigger extends `trigger` with `next`.
    pub fn can_continue<I>(&self, trigger: I, next: char) -> bool
    where
        I: IntoIterator<Item = char>,
    {
        self.forward_node(trigger)
            .is_some_and(|node| self.forward.child(node, next).is_some())
    }

    /// Whether some configured trigger extends `trigger` by any character.
    pub fn has_continuation<I>(&self, trigger: I) -> bool
    where
        I: IntoIterator<Item = char>,
    {
        self.forward_node(trigger)
            .is_some_and(|node| !self.forward.edges(node).is_empty())
    }

    /// Returns the expansion index when the buffer ends in a trigger.
    ///
    /// Takes the buffer's characters in reverse (last-typed character
    /// first) so callers can walk a `VecDeque<char>` directly instead of
    /// materializing the whole buffer into a `String` on every call.
    pub fn find_suffix<I>(&self, chars_rev: I) -> Option<(usize, usize)>
    where
        I: IntoIterator<Item = char>,
    {
        let mut node = 0;
        let mut length = 0;
        let mut best = None;
        for character in chars_rev {
            let Some(next) = self.root.child(node, character) else {
                break;
            };
            node = next;
            length += 1;
            let value = self.root.nodes[node as usize].value;
            if value != NO_VALUE {
                // Keep walking so `:address` wins over `:a` when both are
                // configured. The old first-match behavior made useful
                // trigger families impossible to express.
                best = Some((value as usize, length));
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::Matcher;

    #[test]
    fn finds_only_a_suffix() {
        let matcher = Matcher::new(["ab".into(), "xyz".into()]);
        assert_eq!(matcher.find_suffix("prefixab".chars().rev()), Some((0, 2)));
        assert_eq!(matcher.find_suffix("prefixx".chars().rev()), None);
    }

    #[test]
    fn matches_unicode_by_scalar_value() {
        let matcher = Matcher::new(["🙂x".into()]);
        assert_eq!(matcher.find_suffix("a🙂x".chars().rev()), Some((0, 2)));
    }

    #[test]
    fn continuation_queries_walk_the_forward_trie() {
        let matcher = Matcher::new([":a".into(), ":address".into()]);

        assert!(matcher.has_continuation(":a".chars()));
        assert!(matcher.can_continue(":a".chars(), 'd'));
        assert!(!matcher.can_continue(":a".chars(), 'x'));
        // A complete trigger that nothing extends.
        assert!(!matcher.has_continuation(":address".chars()));
        // A prefix that leaves the trie entirely answers no rather than
        // reporting the root's children.
        assert!(!matcher.has_continuation("nope".chars()));
        assert!(!matcher.can_continue("nope".chars(), 'x'));
    }

    #[test]
    fn continuation_queries_accept_a_buffer_slice_without_allocating() {
        let matcher = Matcher::new([":ab".into()]);
        let buffer: std::collections::VecDeque<char> = "xx:a".chars().collect();

        let start = buffer.len() - 2;
        assert!(matcher.can_continue(buffer.iter().skip(start).copied(), 'b'));
        assert!(matcher.has_continuation(buffer.iter().skip(start).copied()));
    }

    #[test]
    fn prefers_the_longest_matching_suffix() {
        let matcher = Matcher::new([":a".into(), ":address".into()]);
        assert_eq!(
            matcher.find_suffix("email :address".chars().rev()),
            Some((1, 8))
        );
        assert_eq!(matcher.find_suffix("email :a".chars().rev()), Some((0, 2)));
    }

    #[test]
    fn empty_matcher_matches_nothing() {
        let matcher = Matcher::default();
        assert_eq!(matcher.find_suffix("abc".chars().rev()), None);
        assert!(!matcher.has_continuation("".chars()));
        assert!(!matcher.can_continue("".chars(), 'a'));
    }

    #[test]
    fn agrees_with_a_naive_scan_over_unicode_triggers() {
        let triggers: Vec<String> = [
            ":a",
            ":ab",
            ":abc",
            "b",
            "日本",
            "語日本",
            "é",
            "e\u{301}",
            "🙂",
            "x🙂y",
            ":Ab",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let matcher = Matcher::new(triggers.clone());
        let alphabet: Vec<char> = ":abAxy日本語e\u{301}é🙂".chars().collect();
        // Every buffer up to length 4 over the alphabet.
        let mut buffers = vec![String::new()];
        for _ in 0..4 {
            let next: Vec<String> = buffers
                .iter()
                .flat_map(|buffer| {
                    alphabet
                        .iter()
                        .map(move |character| format!("{buffer}{character}"))
                })
                .collect();
            buffers.extend(next.iter().cloned());
            buffers.dedup();
            if buffers.len() > 30_000 {
                break;
            }
        }
        for buffer in &buffers {
            let expected = triggers
                .iter()
                .enumerate()
                .filter(|(_, trigger)| buffer.ends_with(trigger.as_str()))
                .map(|(index, trigger)| (index, trigger.chars().count()))
                .max_by_key(|&(_, length)| length);
            assert_eq!(
                matcher.find_suffix(buffer.chars().rev()),
                expected,
                "{buffer:?}"
            );
            let has_continuation = triggers.iter().any(|trigger| {
                trigger.len() > buffer.len() && trigger.starts_with(buffer.as_str())
            });
            assert_eq!(
                matcher.has_continuation(buffer.chars()),
                has_continuation,
                "{buffer:?}"
            );
            for &next in &alphabet {
                let extended = format!("{buffer}{next}");
                let can_continue = triggers
                    .iter()
                    .any(|trigger| trigger.starts_with(extended.as_str()));
                assert_eq!(matcher.can_continue(buffer.chars(), next), can_continue);
            }
        }
    }
}
