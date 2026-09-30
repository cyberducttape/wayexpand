//! Lossless TOML document handling for the GUI configuration editor.

use anyhow::Result;
use std::str::FromStr;

pub(crate) fn read_config_document(text: &str) -> Result<toml_edit::DocumentMut> {
    toml_edit::DocumentMut::from_str(text)
        .map_err(|error| anyhow::anyhow!("could not parse configuration document: {error}"))
}

/// Merge a canonical configuration into the loaded document while preserving
/// comments, whitespace, and quoting on values that remain present.
pub(crate) fn merge_config_document(
    mut original: toml_edit::DocumentMut,
    replacement: toml_edit::DocumentMut,
) -> toml_edit::DocumentMut {
    merge_toml_item(original.as_item_mut(), replacement.into_item());
    original
}

fn merge_toml_item(old: &mut toml_edit::Item, replacement: toml_edit::Item) {
    match replacement {
        toml_edit::Item::Table(new_table) => {
            if let toml_edit::Item::Table(old_table) = old {
                merge_toml_table(old_table, new_table);
            } else {
                *old = toml_edit::Item::Table(new_table);
            }
        }
        toml_edit::Item::ArrayOfTables(new_array) => {
            if let toml_edit::Item::ArrayOfTables(old_array) = old {
                let mut merged = Vec::new();
                let mut used = vec![false; old_array.len()];
                let mut by_id: std::collections::HashMap<&str, std::collections::VecDeque<usize>> =
                    std::collections::HashMap::new();
                let mut by_trigger: std::collections::HashMap<
                    &str,
                    std::collections::VecDeque<usize>,
                > = std::collections::HashMap::new();
                for (old_index, old_table) in old_array.iter().enumerate() {
                    if let Some(id) = old_table
                        .get("id")
                        .and_then(toml_edit::Item::as_value)
                        .and_then(toml_edit::Value::as_str)
                    {
                        by_id.entry(id).or_default().push_back(old_index);
                    }
                    if let Some(trigger) = old_table
                        .get("trigger")
                        .and_then(toml_edit::Item::as_value)
                        .and_then(toml_edit::Value::as_str)
                    {
                        by_trigger.entry(trigger).or_default().push_back(old_index);
                    }
                }
                for (index, new_table) in new_array.iter().enumerate() {
                    let id_match = new_table
                        .get("id")
                        .and_then(toml_edit::Item::as_value)
                        .and_then(toml_edit::Value::as_str)
                        .and_then(|id| take_unused(by_id.get_mut(id)?, &used));
                    let matching_index = id_match
                        .or_else(|| {
                            new_table
                                .get("trigger")
                                .and_then(toml_edit::Item::as_value)
                                .and_then(toml_edit::Value::as_str)
                                .and_then(|trigger| {
                                    take_unused(by_trigger.get_mut(trigger)?, &used)
                                })
                        })
                        .or_else(|| (index < old_array.len() && !used[index]).then_some(index));
                    let mut table = matching_index
                        .and_then(|old_index| {
                            used[old_index] = true;
                            old_array.get(old_index).cloned()
                        })
                        .unwrap_or_else(|| new_table.clone());
                    table.set_position(None);
                    merge_toml_table(&mut table, new_table.clone());
                    merged.push(table);
                }
                old_array.clear();
                for table in merged {
                    old_array.push(table);
                }
            } else {
                *old = toml_edit::Item::ArrayOfTables(new_array);
            }
        }
        toml_edit::Item::Value(mut new_value) => {
            if let toml_edit::Item::Value(old_value) = old {
                *new_value.decor_mut() = old_value.decor().clone();
            }
            *old = toml_edit::Item::Value(new_value);
        }
        toml_edit::Item::None => *old = toml_edit::Item::None,
    }
}

fn take_unused(candidates: &mut std::collections::VecDeque<usize>, used: &[bool]) -> Option<usize> {
    while let Some(candidate) = candidates.pop_front() {
        if !used[candidate] {
            return Some(candidate);
        }
    }
    None
}

fn merge_toml_table(old: &mut toml_edit::Table, replacement: toml_edit::Table) {
    let replacement_keys: Vec<String> = replacement.iter().map(|(key, _)| key.to_owned()).collect();
    let replacement_key_set: std::collections::HashSet<&str> =
        replacement_keys.iter().map(String::as_str).collect();
    let old_keys: Vec<String> = old.iter().map(|(key, _)| key.to_owned()).collect();
    for key in old_keys {
        if !replacement_key_set.contains(key.as_str()) {
            old.remove(&key);
        }
    }
    for key in replacement_keys {
        if let Some(item) = replacement.get(&key).cloned() {
            if let Some(existing) = old.get_mut(&key) {
                merge_toml_item(existing, item);
            } else {
                old.insert(&key, item);
            }
        }
    }
}
