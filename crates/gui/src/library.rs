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

fn matches_query(expansion: &ExpansionConfig, query: &str) -> bool {
    query.is_empty()
        || format!(
            "{} {} {} {}",
            expansion.trigger,
            expansion.description,
            expansion.tags.join(" "),
            expansion.category
        )
        .to_lowercase()
        .contains(query)
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
