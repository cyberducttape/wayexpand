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

    pub fn can_continue(&self, trigger: &str, next: char) -> bool {
        let mut node = &self.forward;
        for character in trigger.chars() {
            let Some(next_node) = node.children.get(&character) else {
                return false;
            };
            node = next_node;
        }
        node.children.contains_key(&next)
    }

    pub fn has_continuation(&self, trigger: &str) -> bool {
        let mut node = &self.forward;
        for character in trigger.chars() {
            let Some(next_node) = node.children.get(&character) else {
                return false;
            };
            node = next_node;
        }
        !node.children.is_empty()
    }

    /// Returns the expansion index when the buffer ends in a trigger.
    pub fn find_suffix(&self, buffer: &str) -> Option<(usize, usize)> {
        let mut node = &self.root;
        let mut length = 0;
        let mut best = None;
        for character in buffer.chars().rev() {
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
        assert_eq!(matcher.find_suffix("prefixab"), Some((0, 2)));
        assert_eq!(matcher.find_suffix("prefixx"), None);
    }

    #[test]
    fn matches_unicode_by_scalar_value() {
        let matcher = Matcher::new(["🙂x".into()]);
        assert_eq!(matcher.find_suffix("a🙂x"), Some((0, 2)));
    }

    #[test]
    fn prefers_the_longest_matching_suffix() {
        let matcher = Matcher::new([":a".into(), ":address".into()]);
        assert_eq!(matcher.find_suffix("email :address"), Some((1, 8)));
        assert_eq!(matcher.find_suffix("email :a"), Some((0, 2)));
    }
}
