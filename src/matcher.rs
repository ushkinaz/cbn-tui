use crate::model::IndexedItem;
use serde_json::Value;

/// Represents a parsed search term with an optional classifier and exact match flag.
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

        // Check if the value is quoted (exact match)
        if value_part.starts_with('\'') && value_part.ends_with('\'') && value_part.len() >= 2 {
            SearchTerm {
                classifier: Some(classifier),
                pattern: unescape_exact_pattern(&value_part[1..value_part.len() - 1]),
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
                pattern: unescape_exact_pattern(&term[1..term.len() - 1]),
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

fn unescape_exact_pattern(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                if next == '\'' || next == '\\' {
                    out.push(next);
                } else {
                    out.push('\\');
                    out.push(next);
                }
            } else {
                out.push('\\');
            }
        } else {
            out.push(ch);
        }
    }

    out
}

/// Splits a query string into terms while preserving quoted segments.
///
/// Whitespace delimits terms unless it's inside a single-quoted segment.
/// Quotes only begin an exact segment at token start (or right after `:`),
/// so apostrophes in normal words are preserved.
fn split_query_terms(query: &str) -> Vec<String> {
    fn is_escaped(input: &str, byte_idx: usize) -> bool {
        let bytes = input.as_bytes();
        let mut i = byte_idx;
        let mut backslashes = 0;
        while i > 0 && bytes[i - 1] == b'\\' {
            backslashes += 1;
            i -= 1;
        }
        backslashes % 2 == 1
    }

    let mut terms = Vec::new();
    let mut start: Option<usize> = None;
    let mut in_single_quotes = false;
    let mut chars = query.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if ch.is_whitespace() && !in_single_quotes {
            if let Some(token_start) = start.take() {
                terms.push(query[token_start..idx].to_string());
            }
            continue;
        }

        if start.is_none() {
            start = Some(idx);
        }

        if ch == '\'' && !is_escaped(query, idx) {
            if !in_single_quotes {
                if let Some(token_start) = start {
                    let quote_starts_exact =
                        idx == token_start || query[token_start..idx].ends_with(':');
                    if quote_starts_exact {
                        in_single_quotes = true;
                    }
                }
            } else {
                let next_is_delimiter = match chars.peek() {
                    None => true,
                    Some((_, next)) => next.is_whitespace(),
                };
                if next_is_delimiter {
                    in_single_quotes = false;
                }
            }
        }
    }

    if let Some(token_start) = start {
        terms.push(query[token_start..].to_string());
    }

    terms
}

/// Recursively checks if a JSON value matches the search criteria.
///
/// For exact matches (`exact: true`), the value (converted to string) must be identical to the pattern.
/// For pattern matches (`exact: false`), the value must contain the pattern as a substring (case-insensitive for strings).
///
/// **Optimization Note:** If `exact` is false, `pattern` MUST be passed in lowercase.
pub(crate) fn matches_value(value: &Value, pattern: &str, exact: bool) -> bool {
    match value {
        Value::String(s) => {
            if exact {
                s == pattern
            } else {
                // pattern is already lowercased by caller
                s.to_lowercase().contains(pattern)
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
                // pattern is already lowercased by caller
                b_str.to_lowercase().contains(pattern)
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
                // pattern is already lowercased by caller
                "null".contains(pattern)
            }
        }
    }
}

/// Navigates to a specific field in the JSON (supporting dot-notation like "bash.str_min")
/// and checks if any value found at that path matches the criteria.
///
/// **Optimization Note:** If `exact` is false, `pattern` MUST be passed in lowercase.
#[allow(dead_code)]
pub(crate) fn matches_field(json: &Value, field_name: &str, pattern: &str, exact: bool) -> bool {
    // Split once here; recursive calls use matches_field_parts to avoid re-splitting.
    let parts: Vec<&str> = field_name.split('.').collect();
    matches_field_parts(json, &parts, pattern, exact)
}

/// Inner implementation that operates on a pre-split path slice, avoiding repeated
/// split().collect() allocations when called across many items in the slow search path.
fn matches_field_parts(json: &Value, parts: &[&str], pattern: &str, exact: bool) -> bool {
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
                // Pass the remaining slice directly — no re-join/re-split needed.
                let remaining = &parts[i..];
                return arr
                    .iter()
                    .any(|item| matches_field_parts(item, remaining, pattern, exact));
            }
            _ => {
                // The current value is not an object or array, can't traverse further
                return false;
            }
        }
    }

    false
}

/// Fast indexed search for items
/// Uses inverted index for common fields, falls back to recursive for nested fields
/// Returns indices of matching items
pub fn find_matches(
    query: &str,
    items: &[IndexedItem],
    search_index: &crate::search_index::SearchIndex,
) -> Vec<usize> {
    use foldhash::HashSet;

    if query.is_empty() {
        return collect_all_indices(items);
    }

    // Parse all search terms at once (not per item)
    let terms: Vec<SearchTerm> = split_query_terms(query)
        .iter()
        .map(|term| parse_search_term(term))
        .collect();

    // Start with all items, then intersect with results from each term
    let mut results: Option<HashSet<usize>> = None;

    for term in terms {
        let matches = if let Some(classifier) = &term.classifier {
            // Classifier-based search
            match classifier.as_str() {
                "id" | "abstract" | "i" => {
                    // Fast path - use id index (includes abstract)
                    // Support both "id:" and shortcut "i:"
                    search_index.lookup_field(&search_index.by_id, &term.pattern, term.exact)
                }
                "type" | "t" => {
                    // Fast path - use type index
                    // Support both "type:" and shortcut "t:"
                    search_index.lookup_field(&search_index.by_type, &term.pattern, term.exact)
                }
                "category" | "c" => {
                    // Fast path - use category index
                    // Support both "category:" and shortcut "c:"
                    search_index.lookup_field(&search_index.by_category, &term.pattern, term.exact)
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
                search_index.search_words(&term.pattern)
            }
        };

        // Intersect with AND logic
        results = Some(match results {
            None => matches,
            Some(mut prev) => {
                // Optimization: Always iterate over the smaller set
                // and reuse the allocation if possible.
                if prev.len() < matches.len() {
                    // prev is smaller: iterate prev and keep only elements in matches
                    prev.retain(|k| matches.contains(k));
                    prev
                } else {
                    // matches are smaller (or equal): iterate matches and keep only elements in prev
                    // We can reuse matches' allocation since we own it
                    let mut m = matches;
                    m.retain(|k| prev.contains(k));
                    m
                }
            }
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
    items: &[IndexedItem],
    classifier: &str,
    pattern: &str,
    exact: bool,
) -> foldhash::HashSet<usize> {
    // Pre-lowercase the pattern once (avoids repeated work per item).
    let pattern_owned = if exact {
        pattern.to_string()
    } else {
        pattern.to_lowercase()
    };

    // Pre-split the field path once outside the per-item loop.
    // Previously matches_field split on every item visit.
    let parts: Vec<&str> = classifier.split('.').collect();

    items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches_field_parts(&item.value, &parts, &pattern_owned, exact))
        .map(|(idx, _)| idx)
        .collect()
}

/// Slow path: recursive search without classifier
fn slow_search_no_classifier(
    items: &[IndexedItem],
    pattern: &str,
    exact: bool,
) -> foldhash::HashSet<usize> {
    // Optimization: Pre-calculate the pattern to match against.
    // If not exact, we lowercase it once here instead of for every value check.
    let pattern_owned = if exact {
        pattern.to_string()
    } else {
        pattern.to_lowercase()
    };

    items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches_value(&item.value, &pattern_owned, exact))
        .map(|(idx, _)| idx)
        .collect()
}

fn collect_all_indices(items: &[IndexedItem]) -> Vec<usize> {
    (0..items.len()).collect()
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
    fn test_parse_classifier_quoted_escaped_apostrophe() {
        let term = parse_search_term("snippet:'You wouldn\\'t buy'");
        assert_eq!(
            term,
            SearchTerm {
                classifier: Some("snippet".to_string()),
                pattern: "You wouldn't buy".to_string(),
                exact: true
            }
        );
    }

    #[test]
    fn test_split_query_terms_preserves_quoted_spaces() {
        let terms = split_query_terms("id:test snippet:'exact phrase match'");
        assert_eq!(terms, vec!["id:test", "snippet:'exact phrase match'"]);
    }

    #[test]
    fn test_split_query_terms_preserves_apostrophe_inside_quotes() {
        let terms = split_query_terms("snippet:'You wouldn't buy a car'");
        assert_eq!(terms, vec!["snippet:'You wouldn't buy a car'"]);
    }

    #[test]
    fn test_split_query_terms_keeps_unquoted_apostrophe() {
        let terms = split_query_terms("id:wouldn't");
        assert_eq!(terms, vec!["id:wouldn't"]);
    }

    #[test]
    fn test_matches_value_string_pattern() {
        // When exact=false, pattern must be lowercase
        assert!(matches_value(&json!("EMITTER"), "emit", false));
        assert!(matches_value(&json!("EMITTER"), "itter", false));
        assert!(matches_value(&json!("EMITTER"), "emitter", false));
        assert!(!matches_value(&json!("EMITTER"), "transmit", false));
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
        // Pattern must be lowercase for non-exact match
        assert!(matches_value(&arr, "emitter", false));
        assert!(matches_value(&arr, "emit", false));
        assert!(matches_value(&arr, "itter", false));
        assert!(!matches_value(&arr, "transmit", false));
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

    // ========== Original matcher tests (refactored to use find_matches) ==========

    #[test]
    fn test_search_simple_pattern() {
        let items = vec![IndexedItem {
            value: json!({"id": "f_alien_gasper", "flags": ["TRANSPARENT", "EMITTER", "MINEABLE"]}),
            id: "f_alien_gasper".to_string(),
            item_type: "furniture".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!find_matches("EMITTER", &items, &index).is_empty());
        assert!(!find_matches("EMITT", &items, &index).is_empty());
        assert!(!find_matches("ITTER", &items, &index).is_empty());
    }

    #[test]
    fn test_search_exact_match() {
        let items = vec![IndexedItem {
            value: json!({"id": "f_alien_gasper", "flags": ["TRANSPARENT", "EMITTER", "MINEABLE"]}),
            id: "f_alien_gasper".to_string(),
            item_type: "furniture".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(
            find_matches("'EMITT'", &items, &index).is_empty(),
            "'EMITT' should not match"
        );
        assert!(
            !find_matches("'EMITTER'", &items, &index).is_empty(),
            "'EMITTER' should match"
        );
    }

    #[test]
    fn test_search_classifier_exact_with_spaces() {
        let items = vec![IndexedItem {
            value: json!({"snippet": "exact phrase match"}),
            id: "test".to_string(),
            item_type: "item".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(
            !find_matches("snippet:'exact phrase match'", &items, &index).is_empty(),
            "Exact classifier query with spaces should match"
        );
    }

    #[test]
    fn test_search_classifier_exact_with_apostrophe() {
        let items = vec![IndexedItem {
            value: json!({"snippet": "You wouldn't buy a car"}),
            id: "test".to_string(),
            item_type: "item".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(
            !find_matches("snippet:'You wouldn't buy a car'", &items, &index).is_empty(),
            "Exact classifier query with apostrophe should match"
        );
    }

    #[test]
    fn test_search_classifier_exact_with_escaped_apostrophe() {
        let items = vec![IndexedItem {
            value: json!({"snippet": "You wouldn't buy a car"}),
            id: "test".to_string(),
            item_type: "item".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(
            !find_matches("snippet:'You wouldn\\'t buy a car'", &items, &index).is_empty(),
            "Escaped apostrophe exact query should match"
        );
    }

    #[test]
    fn test_search_classifier() {
        let items = vec![IndexedItem {
            value: json!({"id": "f_alien_gasper", "type": "furniture"}),
            id: "f_alien_gasper".to_string(),
            item_type: "furniture".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!find_matches("id:f_alien", &items, &index).is_empty());
        assert!(!find_matches("id:alien", &items, &index).is_empty());
        assert!(
            find_matches("id:'f_alien'", &items, &index).is_empty(),
            "Exact 'f_alien' should not match 'f_alien_gasper'"
        );
        assert!(!find_matches("type:furniture", &items, &index).is_empty());
    }

    #[test]
    fn test_search_nested_field() {
        let items = vec![IndexedItem {
            value: json!({"bash": {"str_min": 30, "str_max": 60}}),
            id: "test".to_string(),
            item_type: "furniture".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!find_matches("bash.str_min:30", &items, &index).is_empty());
        assert!(
            !find_matches("bash.str_min:3", &items, &index).is_empty(),
            "Pattern match should work"
        );
        // Exact match - number converts to string "30"
        assert!(!find_matches("bash.str_min:'30'", &items, &index).is_empty());
        assert!(
            find_matches("bash.str_min:'3'", &items, &index).is_empty(),
            "Exact '3' should not match '30'"
        );
    }

    #[test]
    fn test_search_invalid_classifier() {
        let items = vec![IndexedItem {
            value: json!({"bash": {"str_min": 30}}),
            id: "test".to_string(),
            item_type: "furniture".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(
            find_matches("str_:30", &items, &index).is_empty(),
            "Invalid classifier should not match"
        );
    }

    #[test]
    fn test_search_and_logic() {
        let items = vec![IndexedItem {
            value: json!({"id": "f_alien_gasper", "flags": ["EMITTER"]}),
            id: "f_alien_gasper".to_string(),
            item_type: "furniture".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        assert!(!find_matches("id:f_alien EMITTER", &items, &index).is_empty());
        assert!(find_matches("id:f_alien TRANSMIT", &items, &index).is_empty());
    }

    /// Test all examples from the original user request
    #[test]
    fn test_all_user_examples() {
        let items = vec![IndexedItem {
            value: json!({
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
            id: "f_alien_gasper".to_string(),
            item_type: "furniture".to_string(),
        }];
        let index = crate::search_index::SearchIndex::build(&items);

        // Should match
        assert!(
            !find_matches("id:f_alien", &items, &index).is_empty(),
            "id:f_alien should match"
        );
        assert!(
            !find_matches("id:f_alien EMITTER", &items, &index).is_empty(),
            "id:f_alien EMITTER should match"
        );
        assert!(
            !find_matches("EMITTER", &items, &index).is_empty(),
            "EMITTER should match"
        );
        assert!(
            !find_matches("EMITT", &items, &index).is_empty(),
            "EMITT should match"
        );
        assert!(
            !find_matches("ITTER", &items, &index).is_empty(),
            "ITTER should match"
        );
        assert!(
            !find_matches("bash.str_min:30", &items, &index).is_empty(),
            "str_min:30 should match"
        );
        assert!(
            !find_matches("bash.str_min:3", &items, &index).is_empty(),
            "str_min:3 should match"
        );
        assert!(
            !find_matches("bash.items.count:15", &items, &index).is_empty(),
            "count:15 should match"
        );
        assert!(
            !find_matches("emissions:migo", &items, &index).is_empty(),
            "emissions:migo should match"
        );
        assert!(
            !find_matches("bash.str_min:'30'", &items, &index).is_empty(),
            "str_min:'30' should match (number to string)"
        );

        // Should NOT match
        assert!(
            find_matches("id:'f_alien'", &items, &index).is_empty(),
            "id:'f_alien' should NOT match"
        );
        assert!(
            find_matches("'EMITT'", &items, &index).is_empty(),
            "'EMITT' should NOT match"
        );
        assert!(
            find_matches("bash.str_min:'3'", &items, &index).is_empty(),
            "str_min:'3' should NOT match (exact '3' != '30')"
        );
        assert!(
            find_matches("str_:30", &items, &index).is_empty(),
            "str_:30 should NOT match"
        );
        assert!(
            find_matches("bash.items.count:16", &items, &index).is_empty(),
            "count:16 should NOT match"
        );
    }

    // ========== New tests for identified issues ==========

    #[test]
    fn test_search_with_index_shortcuts() {
        // Tests for issue #2: shortcuts i:, t:, c: should work
        let items = vec![IndexedItem {
            value: json!({"id": "test_item", "type": "TOOL", "category": "weapons"}),
            id: "test_item".to_string(),
            item_type: "TOOL".to_string(),
        }];

        let index = crate::search_index::SearchIndex::build(&items);

        // Shortcuts should work like full names
        let results = find_matches("i:test", &items, &index);
        assert!(!results.is_empty(), "i:test shortcut should work");

        let results = find_matches("t:tool", &items, &index);
        assert!(!results.is_empty(), "t:tool shortcut should work");

        let results = find_matches("c:weapons", &items, &index);
        assert!(!results.is_empty(), "c:weapons shortcut should work");
    }

    #[test]
    fn test_search_with_index_array_elements() {
        // Tests for issue #3: array elements should be indexed
        let items = vec![IndexedItem {
            value: json!({"id": "test", "flags": ["EMITTER", "DANGEROUS"]}),
            id: "test".to_string(),
            item_type: "item".to_string(),
        }];

        let index = crate::search_index::SearchIndex::build(&items);

        // Should find items with "EMITTER" in the flags array
        let results = find_matches("EMITTER", &items, &index);
        assert!(!results.is_empty(), "Should find EMITTER in array");

        let results = find_matches("dangerous", &items, &index);
        assert!(
            !results.is_empty(),
            "Should find DANGEROUS in array (case insensitive)"
        );
    }

    #[test]
    fn test_search_with_index_nested_fields() {
        // Tests for issue #4 & #5: nested fields should be searchable
        let items = vec![
            IndexedItem {
                value: json!({
                    "id": "f_alien_gasper",
                    "bash": {
                        "str_min": 30,
                        "items": [{"item": "alien_resin", "count": 15}]
                    }
                }),
                id: "f_alien_gasper".to_string(),
                item_type: "furniture".to_string(),
            },
            IndexedItem {
                value: json!({"id": "apple", "color": "red"}),
                id: "apple".to_string(),
                item_type: "fruit".to_string(),
            },
            IndexedItem {
                value: json!({"id": "banana", "color": "yellow"}),
                id: "banana".to_string(),
                item_type: "fruit".to_string(),
            },
        ];
        let index = crate::search_index::SearchIndex::build(&items);

        // Nested field with classifier should work
        let results = find_matches("bash.str_min:30", &items, &index);
        assert!(!results.is_empty(), "bash.str_min:30 should match");

        // Nested array element
        let results = find_matches("bash.items.count:15", &items, &index);
        assert!(!results.is_empty(), "bash.items.count:15 should match");

        // Generic search should find nested values
        let results = find_matches("alien_resin", &items, &index);
        assert!(
            !results.is_empty(),
            "Should find 'alien_resin' in nested object"
        );
    }

    #[test]
    fn test_search_intersection_optimization() {
        use serde_json::json;
        // This test ensures that the intersection logic works correctly
        // even when we optimize it to iterate the smaller set.
        let mut items = Vec::new();
        // Create 100 items
        for i in 0..100 {
            let id = format!("item_{}", i);
            let type_ = if i < 10 { "rare" } else { "common" };
            let cat = if i % 2 == 0 { "even" } else { "odd" };
            items.push(IndexedItem {
                value: json!({"id": id, "type": type_, "category": cat}),
                id,
                item_type: type_.to_string(),
            });
        }

        let index = crate::search_index::SearchIndex::build(&items);

        // Search for "common even"
        // "common" matches 90 items (10..99)
        // "even" matches 50 items (0, 2, ... 98)
        // Intersection should be 45 items (10, 12, ... 98)
        let results = find_matches("common even", &items, &index);
        assert_eq!(results.len(), 45);

        // Verify contents
        for idx in results {
            let i = idx; // items are indexed 0..99
            assert!(i >= 10); // common
            assert_eq!(i % 2, 0); // even
        }

        // Search for "even common" (reverse order)
        let results2 = find_matches("even common", &items, &index);
        assert_eq!(results2.len(), 45);

        // Search for "rare even"
        // "rare" matches 10 items (0..9)
        // "even" matches 50 items
        // Intersection should be 5 items (0, 2, 4, 6, 8)
        let results3 = find_matches("rare even", &items, &index);
        assert_eq!(results3.len(), 5);
        for idx in results3 {
            assert!(idx < 10);
            assert_eq!(idx % 2, 0);
        }
    }

    #[test]
    #[ignore]
    fn test_slow_search_performance() {
        use serde_json::json;
        use std::time::Instant;

        // Generate 10,000 items with nested structure
        let mut items = Vec::new();
        for i in 0..10000 {
            items.push(IndexedItem {
                value: json!({
                    "id": format!("item_{}", i),
                    "description": "This is a test description with some random words like zombie, alien, and robot.",
                    "nested": {
                        "level1": {
                            "level2": {
                                "value": format!("value_{}", i)
                            }
                        }
                    },
                    "array": ["one", "two", "three", "four", "five"]
                }),
                id: format!("item_{}", i),
                item_type: "item".to_string()
            });
        }

        let start = Instant::now();
        // Run 100 searches
        // "description:zombie" will force a scan of all items checking the "description" field.
        // This exercises matches_field -> matches_value recursion.
        for _ in 0..100 {
            let _ = slow_search_classifier(&items, "description", "zombie", false);
        }
        let duration = start.elapsed();
        println!("Performance test time: {:?}", duration);
    }
}
