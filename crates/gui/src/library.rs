use std::collections::BTreeSet;

use wayexpand_core::Config;
#[cfg(test)]
use wayexpand_core::ExpansionConfig;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchFields {
    pub triggers: bool,
    pub descriptions: bool,
    pub tags: bool,
    pub replacements: bool,
}

impl Default for SearchFields {
    fn default() -> Self {
        Self {
            triggers: true,
            descriptions: true,
            tags: true,
            replacements: false,
        }
    }
}

#[derive(Debug, Clone)]
struct SearchEntry {
    trigger: String,
    description: String,
    category: String,
    tags: Vec<String>,
    replacement: String,
}

/// Normalized once per committed library edit; frame-time filtering only
/// lowercases the query and searches the selected fields.
#[derive(Debug, Clone, Default)]
pub(crate) struct SearchIndex {
    entries: Vec<SearchEntry>,
}

impl SearchIndex {
    pub fn new(config: &Config) -> Self {
        Self {
            entries: config
                .expansion
                .iter()
                .map(|item| SearchEntry {
                    trigger: item.trigger.to_lowercase(),
                    description: item.description.to_lowercase(),
                    category: item.category.to_lowercase(),
                    tags: item.tags.iter().map(|tag| tag.to_lowercase()).collect(),
                    replacement: item.replacement.to_lowercase(),
                })
                .collect(),
        }
    }

    pub fn visible_indices(
        &self,
        config: &Config,
        filter: &str,
        category_filter: Option<&str>,
        fields: SearchFields,
    ) -> Vec<usize> {
        let query = filter.to_lowercase();
        let category = category_filter.map(str::to_lowercase);
        config
            .expansion
            .iter()
            .enumerate()
            .filter_map(|(index, expansion)| {
                if category
                    .as_ref()
                    .is_some_and(|category| expansion.category.to_lowercase() != *category)
                {
                    return None;
                }
                let entry = self.entries.get(index)?;
                let matches = query.is_empty()
                    || (fields.triggers && entry.trigger.contains(&query))
                    || (fields.descriptions
                        && (entry.description.contains(&query) || entry.category.contains(&query)))
                    || (fields.tags && entry.tags.iter().any(|tag| tag.contains(&query)))
                    || (fields.replacements && entry.replacement.contains(&query));
                matches.then_some(index)
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) fn visible_indices(
    config: &Config,
    filter: &str,
    category_filter: Option<&str>,
) -> Vec<usize> {
    SearchIndex::new(config).visible_indices(
        config,
        filter,
        category_filter,
        SearchFields::default(),
    )
}

/// `query` must already be lowercase.
///
/// Fields are searched one at a time and the first hit wins. Concatenating
/// them into a single lowercased haystack instead built three throwaway
/// strings -- the joined tags, the concatenation, and its lowercased copy --
/// for every snippet in the library, on every frame that redraws the sidebar.
#[cfg(test)]
fn matches_query(expansion: &ExpansionConfig, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    field_matches(&expansion.trigger, query)
        || field_matches(&expansion.description, query)
        || field_matches(&expansion.category, query)
        || expansion.tags.iter().any(|tag| field_matches(tag, query))
}

#[cfg(test)]
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
            id: ExpansionConfig::new_id(),
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
    fn replacement_search_is_opt_in_and_uses_the_normalized_index() {
        let mut item = expansion(":incident", "Incident response", "work", &["sre"]);
        item.replacement = "Restart the production database".into();
        let config = Config {
            expansion: vec![item],
            hotkey: Vec::new(),
            settings: wayexpand_core::Settings::default(),
            organization: wayexpand_core::OrganizationPolicy::default(),
        };
        let index = SearchIndex::new(&config);
        assert!(index
            .visible_indices(&config, "production", None, SearchFields::default())
            .is_empty());
        let fields = SearchFields {
            replacements: true,
            ..SearchFields::default()
        };
        assert_eq!(
            index.visible_indices(&config, "PRODUCTION", None, fields),
            vec![0]
        );
    }

    #[test]
    fn indexed_search_finds_exact_matches_in_a_ten_thousand_snippet_library() {
        let expansions = (0..10_000)
            .map(|index| {
                let mut item = expansion(
                    &format!(":snippet-{index}"),
                    &format!("Description {index}"),
                    "Bulk",
                    &["generated"],
                );
                item.replacement = format!("replacement body {index}");
                item
            })
            .collect();
        let config = Config {
            expansion: expansions,
            hotkey: Vec::new(),
            settings: wayexpand_core::Settings::default(),
            organization: wayexpand_core::OrganizationPolicy::default(),
        };
        let index = SearchIndex::new(&config);
        let matches = index.visible_indices(
            &config,
            "replacement body 9876",
            None,
            SearchFields {
                replacements: true,
                ..SearchFields::default()
            },
        );

        assert_eq!(matches, vec![9876]);
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
