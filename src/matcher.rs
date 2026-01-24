use serde_json::Value;

/// Represents a parsed search term with optional classifier and exact match flag.
/// Used to represent individual components of a space-separated search query.
#[derive(Debug, PartialEq)]
pub(crate) struct SearchTerm {
    /// Optional field classifier (e.g., "id", "str_min").
    pub classifier: Option<String>,
    /// The pattern to match.
    pub pattern: String,
    /// Whether this is an exact match (value surrounded by single quotes).
    pub exact: bool,
}

/// Parses a search string into a `SearchTerm`.
/// Supports "classifier:value", "classifier:'exact_value'", "'exact_value'", and "pattern".
pub(crate) fn parse_search_term(term: &str) -> SearchTerm {
    // Check for classifier (field:value format)
    if let Some(colon_pos) = term.find(':') {
        let classifier = term[..colon_pos].to_string();
        let value_part = &term[colon_pos + 1..];

        // Check if value is quoted (exact match)
        if value_part.starts_with('\'') && value_part.ends_with('\'') && value_part.len() >= 2 {
            SearchTerm {
                classifier: Some(classifier),
                pattern: value_part[1..value_part.len() - 1].to_string(),
                exact: true,
            }
        } else {
            SearchTerm {
                classifier: Some(classifier),
                pattern: value_part.to_string(),
                exact: false,
            }
        }
    } else {
        // No classifier - check if the whole term is quoted
        if term.starts_with('\'') && term.ends_with('\'') && term.len() >= 2 {
            SearchTerm {
                classifier: None,
                pattern: term[1..term.len() - 1].to_string(),
                exact: true,
            }
        } else {
            SearchTerm {
                classifier: None,
                pattern: term.to_string(),
                exact: false,
            }
        }
    }
}

/// Recursively checks if a JSON value matches the search criteria.
///
/// For exact matches (`exact: true`), the value (converted to string) must be identical to the pattern.
/// For pattern matches (`exact: false`), the value must contain the pattern as a substring (case-insensitive for strings).
pub(crate) fn matches_value(value: &Value, pattern: &str, exact: bool) -> bool {
    match value {
        Value::String(s) => {
            if exact {
                s == pattern
            } else {
                s.to_lowercase().contains(&pattern.to_lowercase())
            }
        }
        Value::Number(n) => {
            let n_str = n.to_string();
            if exact {
                // For exact match, convert to string and compare exactly
                // This allows '30' to match the number 30, but not '3'
                n_str == pattern
            } else {
                // For pattern match, allow substring matching
                n_str.contains(pattern)
            }
        }
        Value::Bool(b) => {
            let b_str = b.to_string();
            if exact {
                b_str == pattern
            } else {
                b_str.to_lowercase().contains(&pattern.to_lowercase())
            }
        }
        Value::Array(arr) => {
            // Check if any element in the array matches
            arr.iter().any(|v| matches_value(v, pattern, exact))
        }
        Value::Object(obj) => {
            // Recursively check all values in the object
            obj.values().any(|v| matches_value(v, pattern, exact))
        }
        Value::Null => {
            if exact {
                pattern == "null"
            } else {
                "null".contains(&pattern.to_lowercase())
            }
        }
    }
}

/// Navigates to a specific field in the JSON (supporting dot-notation like "bash.str_min")
/// and checks if any value found at that path matches the criteria.
pub(crate) fn matches_field(json: &Value, field_name: &str, pattern: &str, exact: bool) -> bool {
    // Handle nested field access (e.g., "bash.str_min")
    let parts: Vec<&str> = field_name.split('.').collect();

    let mut current = json;
    for (i, part) in parts.iter().enumerate() {
        match current {
            Value::Object(obj) => {
                if let Some(value) = obj.get(*part) {
                    if i == parts.len() - 1 {
                        // Last part - check the value
                        return matches_value(value, pattern, exact);
                    } else {
                        // Not the last part - continue traversing
                        current = value;
                    }
                } else {
                    // Field not found
                    return false;
                }
            }
            Value::Array(arr) => {
                // If current is an array, try to traverse through each element
                let remaining_parts: Vec<&str> = parts[i..].to_vec();
                let remaining_path = remaining_parts.join(".");
                return arr
                    .iter()
                    .any(|item| matches_field(item, &remaining_path, pattern, exact));
            }
            _ => {
                // Current value is not an object or array, can't traverse further
                return false;
            }
        }
    }

    false
}

/// Fast indexed search for items
/// Uses inverted index for common fields, falls back to recursive for nested fields
/// Returns indices of matching items
pub fn search_with_index(
    index: &crate::search_index::SearchIndex,
    items: &[(Value, String, String)],
    query: &str,
) -> Vec<usize> {
    use std::collections::HashSet;

    if query.is_empty() {
        return (0..items.len()).collect();
    }

    // Parse all search terms once (not per item)
    let terms: Vec<SearchTerm> = query.split_whitespace().map(parse_search_term).collect();

    // Start with all items, then intersect with results from each term
    let mut results: Option<HashSet<usize>> = None;

    for term in terms {
        let matches = if let Some(classifier) = &term.classifier {
            // Classifier-based search
            match classifier.as_str() {
                "id" | "abstract" | "i" => {
                    // Fast path - use id index (includes abstract)
                    // Support both "id:" and shortcut "i:"
                    index.lookup_field(&index.by_id, &term.pattern, term.exact)
                }
                "type" | "t" => {
                    // Fast path - use type index
                    // Support both "type:" and shortcut "t:"
                    index.lookup_field(&index.by_type, &term.pattern, term.exact)
                }
                "category" | "c" => {
                    // Fast path - use category index
                    // Support both "category:" and shortcut "c:"
                    index.lookup_field(&index.by_category, &term.pattern, term.exact)
                }
                _ => {
                    // Nested field - fallback to recursive search
                    slow_search_classifier(items, classifier, &term.pattern, term.exact)
                }
            }
        } else {
            // No classifier - use word index for pattern match
            if term.exact {
                // Exact match without classifier - need recursive search
                slow_search_no_classifier(items, &term.pattern, true)
            } else {
                // Pattern match - use word index
                index.search_words(&term.pattern)
            }
        };

        // Intersect with AND logic
        results = Some(match results {
            None => matches,
            Some(prev) => prev.intersection(&matches).copied().collect(),
        });

        // Early exit if no matches left (optimization)
        if results.as_ref().is_some_and(|r| r.is_empty()) {
            return Vec::new();
        }
    }

    let mut result_vec: Vec<usize> = results.unwrap_or_default().into_iter().collect();
    result_vec.sort_unstable();
    result_vec
}

/// Slow path: recursive search with classifier for nested fields
fn slow_search_classifier(
    items: &[(Value, String, String)],
    classifier: &str,
    pattern: &str,
    exact: bool,
) -> std::collections::HashSet<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, (json, _, _))| matches_field(json, classifier, pattern, exact))
        .map(|(idx, _)| idx)
        .collect()
}

/// Slow path: recursive search without classifier
fn slow_search_no_classifier(
    items: &[(Value, String, String)],
    pattern: &str,
    exact: bool,
) -> std::collections::HashSet<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, (json, _, _))| matches_value(json, pattern, exact))
        .map(|(idx, _)| idx)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_simple_term() {
        let term = parse_search_term("EMITTER");
        assert_eq!(
            term,
            SearchTerm {
                classifier: None,
                pattern: "EMITTER".to_string(),
                exact: false
            }
        );
    }

    #[test]
    fn test_parse_quoted_term() {
        let term = parse_search_term("'EMITT'");
        assert_eq!(
            term,
            SearchTerm {
                classifier: None,
                pattern: "EMITT".to_string(),
                exact: true
            }
        );
    }

    #[test]
    fn test_parse_classifier_term() {
        let term = parse_search_term("id:f_alien");
        assert_eq!(
            term,
            SearchTerm {
                classifier: Some("id".to_string()),
                pattern: "f_alien".to_string(),
                exact: false
            }
        );
    }

    #[test]
    fn test_parse_classifier_quoted() {
        let term = parse_search_term("str_min:'30'");
        assert_eq!(
            term,
            SearchTerm {
                classifier: Some("str_min".to_string()),
                pattern: "30".to_string(),
                exact: true
            }
        );
    }

    #[test]
    fn test_matches_value_string_pattern() {
        assert!(matches_value(&json!("EMITTER"), "EMIT", false));
        assert!(matches_value(&json!("EMITTER"), "ITTER", false));
        assert!(matches_value(&json!("EMITTER"), "emitter", false));
        assert!(!matches_value(&json!("EMITTER"), "TRANSMIT", false));
    }

    #[test]
    fn test_matches_value_string_exact() {
        assert!(matches_value(&json!("EMITTER"), "EMITTER", true));
        assert!(!matches_value(&json!("EMITTER"), "EMIT", true));
        assert!(!matches_value(&json!("EMITTER"), "emitter", true));
    }

    #[test]
    fn test_matches_value_number_pattern() {
        assert!(matches_value(&json!(30), "30", false));
        assert!(matches_value(&json!(30), "3", false));
        assert!(!matches_value(&json!(30), "40", false));
    }

    #[test]
    fn test_matches_value_number_exact() {
        // Exact matches convert numbers to strings for comparison
        assert!(matches_value(&json!(30), "30", true));
        assert!(!matches_value(&json!(30), "3", true));
        assert!(!matches_value(&json!(30), "30.0", true));
    }

    #[test]
    fn test_matches_value_array() {
        let arr = json!(["TRANSPARENT", "EMITTER", "MINEABLE"]);
        assert!(matches_value(&arr, "EMITTER", false));
        assert!(matches_value(&arr, "EMIT", false));
        assert!(matches_value(&arr, "ITTER", false));
        assert!(!matches_value(&arr, "TRANSMIT", false));
    }

    #[test]
    fn test_matches_value_array_exact() {
        let arr = json!(["TRANSPARENT", "EMITTER", "MINEABLE"]);
        assert!(matches_value(&arr, "EMITTER", true));
        assert!(!matches_value(&arr, "EMIT", true));
    }

    #[test]
    fn test_matches_field_simple() {
        let data = json!({
            "id": "f_alien_gasper",
            "type": "furniture"
        });

        assert!(matches_field(&data, "id", "f_alien", false));
        assert!(matches_field(&data, "id", "alien", false));
        assert!(!matches_field(&data, "id", "f_alien", true));
        assert!(!matches_field(&data, "type", "table", false));
    }

    #[test]
    fn test_matches_field_nested() {
        let data = json!({
            "bash": {
                "str_min": 30,
                "str_max": 60
            }
        });

        assert!(matches_field(&data, "bash.str_min", "30", false));
        assert!(matches_field(&data, "bash.str_min", "3", false));
        // Exact match with number - converts to string
        assert!(matches_field(&data, "bash.str_min", "30", true));
        assert!(!matches_field(&data, "bash.str_min", "3", true));
        assert!(!matches_field(&data, "bash.str_min", "60", false));
    }

    #[test]
    fn test_matches_field_nonexistent() {
        let data = json!({
            "id": "test",
            "type": "furniture"
        });

        assert!(!matches_field(&data, "name", "test", false));
        assert!(!matches_field(&data, "str_", "30", false));
    }

    // ========== Original matcher tests (refactored to use search_with_index) ==========

    #[test]
    fn test_search_simple_pattern() {
        let items = vec![(
            json!({"id": "f_alien_gasper", "flags": ["TRANSPARENT", "EMITTER", "MINEABLE"]}),
            "f_alien_gasper".to_string(),
            "furniture".to_string(),
        )];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!search_with_index(&index, &items, "EMITTER").is_empty());
        assert!(!search_with_index(&index, &items, "EMITT").is_empty());
        assert!(!search_with_index(&index, &items, "ITTER").is_empty());
    }

    #[test]
    fn test_search_exact_match() {
        let items = vec![(
            json!({"id": "f_alien_gasper", "flags": ["TRANSPARENT", "EMITTER", "MINEABLE"]}),
            "f_alien_gasper".to_string(),
            "furniture".to_string(),
        )];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(
            search_with_index(&index, &items, "'EMITT'").is_empty(),
            "'EMITT' should not match"
        );
        assert!(
            !search_with_index(&index, &items, "'EMITTER'").is_empty(),
            "'EMITTER' should match"
        );
    }

    #[test]
    fn test_search_classifier() {
        let items = vec![(
            json!({"id": "f_alien_gasper", "type": "furniture"}),
            "f_alien_gasper".to_string(),
            "furniture".to_string(),
        )];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!search_with_index(&index, &items, "id:f_alien").is_empty());
        assert!(!search_with_index(&index, &items, "id:alien").is_empty());
        assert!(
            search_with_index(&index, &items, "id:'f_alien'").is_empty(),
            "Exact 'f_alien' should not match 'f_alien_gasper'"
        );
        assert!(!search_with_index(&index, &items, "type:furniture").is_empty());
    }

    #[test]
    fn test_search_nested_field() {
        let items = vec![(
            json!({"bash": {"str_min": 30, "str_max": 60}}),
            "test".to_string(),
            "furniture".to_string(),
        )];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!search_with_index(&index, &items, "bash.str_min:30").is_empty());
        assert!(
            !search_with_index(&index, &items, "bash.str_min:3").is_empty(),
            "Pattern match should work"
        );
        // Exact match - number converts to string "30"
        assert!(!search_with_index(&index, &items, "bash.str_min:'30'").is_empty());
        assert!(
            search_with_index(&index, &items, "bash.str_min:'3'").is_empty(),
            "Exact '3' should not match '30'"
        );
    }

    #[test]
    fn test_search_invalid_classifier() {
        let items = vec![(
            json!({"bash": {"str_min": 30}}),
            "test".to_string(),
            "furniture".to_string(),
        )];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(
            search_with_index(&index, &items, "str_:30").is_empty(),
            "Invalid classifier should not match"
        );
    }

    #[test]
    fn test_search_and_logic() {
        let items = vec![(
            json!({"id": "f_alien_gasper", "flags": ["EMITTER"]}),
            "f_alien_gasper".to_string(),
            "furniture".to_string(),
        )];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!search_with_index(&index, &items, "id:f_alien EMITTER").is_empty());
        assert!(search_with_index(&index, &items, "id:f_alien TRANSMIT").is_empty());
    }

    /// Test all examples from the original user request
    #[test]
    fn test_all_user_examples() {
        let items = vec![(
            json!({
                "type": "furniture",
                "id": "f_alien_gasper",
                "name": "gasping tube",
                "description": "This is a meaty green stalactite...",
                "symbol": "{",
                "color": "green",
                "move_cost_mod": 6,
                "coverage": 40,
                "required_str": -1,
                "flags": ["TRANSPARENT", "EMITTER", "MINEABLE"],
                "emissions": ["emit_migo_atmosphere", "emit_hot_air_migo_blast"],
                "bash": {
                    "str_min": 30,
                    "str_max": 60,
                    "sound": "splorch!",
                    "sound_fail": "whump!",
                    "furn_set": "f_alien_scar_small",
                    "items": [
                        {
                            "item": "fetid_goop",
                            "count": [15, 25],
                            "prob": 100
                        }
                    ],
                    "ranged": {
                        "reduction": [15, 30],
                        "destroy_threshold": 60,
                        "block_unaimed_chance": "25%"
                    }
                }
            }),
            "f_alien_gasper".to_string(),
            "furniture".to_string(),
        )];
        let index = crate::search_index::SearchIndex::build(&items);

        // Should match
        assert!(
            !search_with_index(&index, &items, "id:f_alien").is_empty(),
            "id:f_alien should match"
        );
        assert!(
            !search_with_index(&index, &items, "id:f_alien EMITTER").is_empty(),
            "id:f_alien EMITTER should match"
        );
        assert!(
            !search_with_index(&index, &items, "EMITTER").is_empty(),
            "EMITTER should match"
        );
        assert!(
            !search_with_index(&index, &items, "EMITT").is_empty(),
            "EMITT should match"
        );
        assert!(
            !search_with_index(&index, &items, "ITTER").is_empty(),
            "ITTER should match"
        );
        assert!(
            !search_with_index(&index, &items, "bash.str_min:30").is_empty(),
            "str_min:30 should match"
        );
        assert!(
            !search_with_index(&index, &items, "bash.str_min:3").is_empty(),
            "str_min:3 should match"
        );
        assert!(
            !search_with_index(&index, &items, "bash.items.count:15").is_empty(),
            "count:15 should match"
        );
        assert!(
            !search_with_index(&index, &items, "emissions:migo").is_empty(),
            "emissions:migo should match"
        );
        assert!(
            !search_with_index(&index, &items, "bash.str_min:'30'").is_empty(),
            "str_min:'30' should match (number to string)"
        );

        // Should NOT match
        assert!(
            search_with_index(&index, &items, "id:'f_alien'").is_empty(),
            "id:'f_alien' should NOT match"
        );
        assert!(
            search_with_index(&index, &items, "'EMITT'").is_empty(),
            "'EMITT' should NOT match"
        );
        assert!(
            search_with_index(&index, &items, "bash.str_min:'3'").is_empty(),
            "str_min:'3' should NOT match (exact '3' != '30')"
        );
        assert!(
            search_with_index(&index, &items, "str_:30").is_empty(),
            "str_:30 should NOT match"
        );
        assert!(
            search_with_index(&index, &items, "bash.items.count:16").is_empty(),
            "count:16 should NOT match"
        );
    }

    // ========== New tests for identified issues ==========

    #[test]
    fn test_search_with_index_shortcuts() {
        // Tests for issue #2: shortcuts i:, t:, c: should work
        let items = vec![(
            json!({"id": "test_item", "type": "TOOL", "category": "weapons"}),
            "test_item".to_string(),
            "TOOL".to_string(),
        )];

        let index = crate::search_index::SearchIndex::build(&items);

        // Shortcuts should work like full names
        let results = search_with_index(&index, &items, "i:test");
        assert!(!results.is_empty(), "i:test shortcut should work");

        let results = search_with_index(&index, &items, "t:tool");
        assert!(!results.is_empty(), "t:tool shortcut should work");

        let results = search_with_index(&index, &items, "c:weapons");
        assert!(!results.is_empty(), "c:weapons shortcut should work");
    }

    #[test]
    fn test_search_with_index_array_elements() {
        // Tests for issue #3: array elements should be indexed
        let items = vec![(
            json!({"id": "test", "flags": ["EMITTER", "DANGEROUS"]}),
            "test".to_string(),
            "item".to_string(),
        )];

        let index = crate::search_index::SearchIndex::build(&items);

        // Should find items with "EMITTER" in flags array
        let results = search_with_index(&index, &items, "EMITTER");
        assert!(!results.is_empty(), "Should find EMITTER in array");

        let results = search_with_index(&index, &items, "dangerous");
        assert!(
            !results.is_empty(),
            "Should find DANGEROUS in array (case insensitive)"
        );
    }

    #[test]
    fn test_search_with_index_nested_fields() {
        // Tests for issue #4 & #5: nested fields should be searchable
        let items = vec![(
            json!({
                "id": "f_alien_gasper",
                "bash": {
                    "str_min": 30,
                    "items": [{"item": "alien_resin", "count": 15}]
                }
            }),
            "f_alien_gasper".to_string(),
            "furniture".to_string(),
        )];

        let index = crate::search_index::SearchIndex::build(&items);

        // Nested field with classifier should work
        let results = search_with_index(&index, &items, "bash.str_min:30");
        assert!(!results.is_empty(), "bash.str_min:30 should match");

        // Nested array element
        let results = search_with_index(&index, &items, "bash.items.count:15");
        assert!(!results.is_empty(), "bash.items.count:15 should match");

        // Generic search should find nested values
        let results = search_with_index(&index, &items, "alien_resin");
        assert!(
            !results.is_empty(),
            "Should find 'alien_resin' in nested object"
        );
    }
}
