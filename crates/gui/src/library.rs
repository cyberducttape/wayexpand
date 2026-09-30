use std::collections::BTreeSet;

use wayexpand_core::{Config, ExpansionConfig};

pub(crate) fn visible_indices(
    config: &Config,
    filter: &str,
    category_filter: Option<&str>,
) -> Vec<usize> {
    let query = filter.to_lowercase();
    config
        .expansion
        .iter()
        .enumerate()
        .filter(|(_, expansion)| {
            category_filter.is_none_or(|category| expansion.category == category)
        })
        .filter(|(_, expansion)| matches_query(expansion, &query))
        .map(|(index, _)| index)
        .collect()
}

/// `query` must already be lowercase.
///
/// Fields are searched one at a time and the first hit wins. Concatenating
/// them into a single lowercased haystack instead built three throwaway
/// strings -- the joined tags, the concatenation, and its lowercased copy --
/// for every snippet in the library, on every frame that redraws the sidebar.
fn matches_query(expansion: &ExpansionConfig, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    field_matches(&expansion.trigger, query)
        || field_matches(&expansion.description, query)
        || field_matches(&expansion.category, query)
        || expansion.tags.iter().any(|tag| field_matches(tag, query))
}

fn field_matches(field: &str, query: &str) -> bool {
    // No byte-length fast path: lowercasing can lengthen a string (`İ` becomes
    // `i` plus a combining dot), so a field shorter than the query can still
    // contain it once folded.
    field.to_lowercase().contains(query)
}

pub(crate) fn categories(config: &Config) -> Vec<String> {
    config
        .expansion
        .iter()
        .map(|expansion| expansion.category.clone())
        .filter(|category| !category.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayexpand_core::MatchMode;

    fn expansion(
        trigger: &str,
        description: &str,
        category: &str,
        tags: &[&str],
    ) -> ExpansionConfig {
        ExpansionConfig {
            trigger: trigger.into(),
            replacement: "never searched".into(),
            description: description.into(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            category: category.into(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        }
    }

    #[test]
    fn a_query_matches_any_searchable_field_case_insensitively() {
        let item = expansion(":Sig", "Email Signature", "Work", &["Mail", "personal"]);

        for query in [":sig", "signature", "work", "mail", "personal"] {
            assert!(
                matches_query(&item, query),
                "expected {query:?} to match the snippet"
            );
        }
        assert!(!matches_query(&item, "absent"));
        // The replacement is deliberately not part of the search surface.
        assert!(!matches_query(&item, "never searched"));
        assert!(matches_query(&item, ""));
    }

    #[test]
    fn folding_that_lengthens_a_field_still_matches() {
        // `İ` (U+0130) lowercases to `i` plus a combining dot, which is longer
        // than the original. A byte-length shortcut would wrongly reject this.
        let item = expansion("İ", "", "", &[]);
        assert!(matches_query(&item, "i"));
    }

    #[test]
    fn filtering_combines_the_category_chip_with_the_search_text() {
        let config = Config {
            expansion: vec![
                expansion(":sig", "signature", "email", &[]),
                expansion(":addr", "address", "personal", &[]),
                expansion(":sig2", "second signature", "personal", &[]),
            ],
            hotkey: Vec::new(),
            settings: wayexpand_core::Settings::default(),
            organization: wayexpand_core::OrganizationPolicy::default(),
        };

        assert_eq!(visible_indices(&config, "", None), vec![0, 1, 2]);
        assert_eq!(visible_indices(&config, "", Some("personal")), vec![1, 2]);
        assert_eq!(visible_indices(&config, "sig", Some("personal")), vec![2]);
        assert_eq!(visible_indices(&config, "sig", Some("email")), vec![0]);
        assert!(visible_indices(&config, "absent", None).is_empty());
    }

    #[test]
    fn categories_are_sorted_deduplicated_and_skip_the_uncategorized() {
        let config = Config {
            expansion: vec![
                expansion(":a", "", "work", &[]),
                expansion(":b", "", "", &[]),
                expansion(":c", "", "email", &[]),
                expansion(":d", "", "work", &[]),
            ],
            hotkey: Vec::new(),
            settings: wayexpand_core::Settings::default(),
            organization: wayexpand_core::OrganizationPolicy::default(),
        };

        assert_eq!(categories(&config), vec!["email", "work"]);
    }
}
