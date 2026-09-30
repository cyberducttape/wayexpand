use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct Matcher {
    root: Node,
    forward: Node,
}

#[derive(Debug, Default, Clone)]
struct Node {
    children: HashMap<char, Node>,
    value: Option<usize>,
}

impl Matcher {
    pub fn new<I>(triggers: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let mut matcher = Self::default();
        for (index, trigger) in triggers.into_iter().enumerate() {
            matcher.insert_reversed(&trigger, index);
            matcher.insert_forward(&trigger);
        }
        matcher
    }

    fn insert_reversed(&mut self, trigger: &str, index: usize) {
        let mut node = &mut self.root;
        for character in trigger.chars().rev() {
            node = node.children.entry(character).or_default();
        }
        node.value = Some(index);
    }

    fn insert_forward(&mut self, trigger: &str) {
        let mut node = &mut self.forward;
        for character in trigger.chars() {
            node = node.children.entry(character).or_default();
        }
    }

    /// Walks the forward trie along `trigger`, returning the node it ends on.
    ///
    /// Takes the characters as an iterator so the hot path can walk a slice of
    /// the rolling `VecDeque<char>` buffer directly instead of collecting the
    /// matched suffix into a `String` on every keystroke.
    fn forward_node<I>(&self, trigger: I) -> Option<&Node>
    where
        I: IntoIterator<Item = char>,
    {
        let mut node = &self.forward;
        for character in trigger {
            node = node.children.get(&character)?;
        }
        Some(node)
    }

    /// Whether some configured trigger extends `trigger` with `next`.
    pub fn can_continue<I>(&self, trigger: I, next: char) -> bool
    where
        I: IntoIterator<Item = char>,
    {
        self.forward_node(trigger)
            .is_some_and(|node| node.children.contains_key(&next))
    }

    /// Whether some configured trigger extends `trigger` by any character.
    pub fn has_continuation<I>(&self, trigger: I) -> bool
    where
        I: IntoIterator<Item = char>,
    {
        self.forward_node(trigger)
            .is_some_and(|node| !node.children.is_empty())
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
        let mut node = &self.root;
        let mut length = 0;
        let mut best = None;
        for character in chars_rev {
            let Some(next) = node.children.get(&character) else {
                break;
            };
            node = next;
            length += 1;
            if let Some(index) = node.value {
                // Keep walking so `:address` wins over `:a` when both are
                // configured. The old first-match behavior made useful
                // trigger families impossible to express.
                best = Some((index, length));
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
}
