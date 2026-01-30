use foldhash::{HashMap, HashSet};
use serde_json::Value;

/// Inverted index for fast search across 30k+ items
/// Indexes common fields (id/abstract, type, category) and tokenized words
#[derive(Debug)]
pub struct SearchIndex {
    /// Index for id OR abstract (mutually exclusive in data)
    pub by_id: HashMap<String, HashSet<usize>>,
    /// Index for type field
    pub by_type: HashMap<String, HashSet<usize>>,
    /// Index for category field
    pub by_category: HashMap<String, HashSet<usize>>,
    /// Word index for fast text search (tokenized from id, name, type, category)
    pub word_index: HashMap<String, HashSet<usize>>,
}

impl SearchIndex {
    /// Creates a new empty search index
    pub fn new() -> Self {
        Self {
            by_id: Default::default(),
            by_type: Default::default(),
            by_category: Default::default(),
            word_index: Default::default(),
        }
    }

    /// Builds inverted index from items.
    ///
    /// This implementation is optimized to minimize string allocations by:
    /// 1. Avoiding redundant tokenization of fields already in the raw JSON.
    /// 2. Avoiding `to_lowercase()` for words that are already lowercase.
    /// 3. Using `foldhash` for faster hashing performance.
    pub fn build(items: &[(Value, String, String)]) -> Self {
        let mut index = Self::new();

        for (idx, (json, id, type_)) in items.iter().enumerate() {
            // Index primary search fields
            if !id.is_empty() {
                index
                    .by_id
                    .entry(id.to_lowercase())
                    .or_default()
                    .insert(idx);
            } else if let Some(abstr) = json.get("abstract").and_then(|v| v.as_str()) {
                index
                    .by_id
                    .entry(abstr.to_lowercase())
                    .or_default()
                    .insert(idx);
            }

            if !type_.is_empty() {
                index
                    .by_type
                    .entry(type_.to_lowercase())
                    .or_default()
                    .insert(idx);
            }

            if let Some(category) = json.get("category").and_then(|v| v.as_str()) {
                index
                    .by_category
                    .entry(category.to_lowercase())
                    .or_default()
                    .insert(idx);
            }

            // Recursively index EVERYTHING in the JSON. Note: This covers the fields above,
            // so we don't need to explicitly call index_words for them here.
            Self::index_value_recursive(&mut index.word_index, json, idx);
        }

        index
    }

    pub fn build_with_progress<F>(items: &[(Value, String, String)], mut on_progress: F) -> Self
    where
        F: FnMut(usize, usize),
    {
        let mut index = Self::new();
        let total = items.len();

        for (idx, (json, id, type_)) in items.iter().enumerate() {
            if !id.is_empty() {
                index
                    .by_id
                    .entry(id.to_lowercase())
                    .or_default()
                    .insert(idx);
            } else if let Some(abstr) = json.get("abstract").and_then(|v| v.as_str()) {
                index
                    .by_id
                    .entry(abstr.to_lowercase())
                    .or_default()
                    .insert(idx);
            }

            if !type_.is_empty() {
                index
                    .by_type
                    .entry(type_.to_lowercase())
                    .or_default()
                    .insert(idx);
            }

            if let Some(category) = json.get("category").and_then(|v| v.as_str()) {
                index
                    .by_category
                    .entry(category.to_lowercase())
                    .or_default()
                    .insert(idx);
            }

            Self::index_value_recursive(&mut index.word_index, json, idx);

            if idx % 250 == 0 || idx + 1 == total {
                on_progress(idx + 1, total);
            }
        }

        index
    }

    /// Recursively index all string values in JSON for word search
    fn index_value_recursive(
        word_index: &mut HashMap<String, HashSet<usize>>,
        value: &Value,
        idx: usize,
    ) {
        match value {
            Value::String(s) => {
                Self::index_words(word_index, s, idx);
            }
            Value::Array(arr) => {
                for item in arr {
                    Self::index_value_recursive(word_index, item, idx);
                }
            }
            Value::Object(obj) => {
                for val in obj.values() {
                    Self::index_value_recursive(word_index, val, idx);
                }
            }
            _ => {} // Numbers, booleans, null - skip for word index
        }
    }

    /// Tokenize and index words from a string.
    ///
    /// This is a hot path. We optimize for the common case where words are already lowercase
    /// and potentially already present in the index to avoid expensive allocations.
    fn index_words(word_index: &mut HashMap<String, HashSet<usize>>, text: &str, idx: usize) {
        // Split by non-alphanumeric characters (preserving underscores and hyphens)
        for word in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
            if word.len() < 2 {
                continue;
            }

            // Efficiency check: if the word is already lowercase, we can avoid the allocation
            // for the search key if it's already present in the map.
            let is_lowercase = word.chars().all(|c| !c.is_uppercase());
            if is_lowercase && let Some(set) = word_index.get_mut(word) {
                set.insert(idx);
                continue;
            }

            // Fallback for mixed-case or new words
            let word_lower = word.to_lowercase();
            if word_lower.len() >= 2 {
                word_index.entry(word_lower).or_default().insert(idx);
            }
        }
    }

    /// Fast lookup in specific field index
    /// Returns indices of items matching the pattern
    pub fn lookup_field(
        &self,
        field_index: &HashMap<String, HashSet<usize>>,
        pattern: &str,
        exact: bool,
    ) -> HashSet<usize> {
        let pattern_lower = pattern.to_lowercase();

        if exact {
            // Exact match - direct lookup
            field_index.get(&pattern_lower).cloned().unwrap_or_default()
        } else {
            // Pattern match - check all keys containing pattern
            field_index
                .iter()
                .filter(|(key, _)| key.contains(&pattern_lower))
                .flat_map(|(_, indices)| indices.iter().copied())
                .collect()
        }
    }

    /// Fast word-based text search.
    /// Returns indices of items containing words that match the pattern.
    pub fn search_words(&self, pattern: &str) -> HashSet<usize> {
        let pattern_lower = pattern.to_lowercase();

        // Find all words that contain the pattern
        self.word_index
            .iter()
            .filter(|(word, _)| word.contains(&pattern_lower))
            .flat_map(|(_, indices)| indices.iter().copied())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_index_building() {
        let items = vec![
            (
                json!({"id": "test_item", "type": "TOOL", "category": "weapons"}),
                "test_item".to_string(),
                "TOOL".to_string(),
            ),
            (
                json!({"abstract": "abstract_base", "type": "MONSTER"}),
                "".to_string(),
                "MONSTER".to_string(),
            ),
        ];

        let index = SearchIndex::build(&items);

        // Check id index (includes abstract)
        assert!(index.by_id.contains_key("test_item"));
        assert!(index.by_id.contains_key("abstract_base"));

        // Check type index
        assert!(index.by_type.contains_key("tool"));
        assert!(index.by_type.contains_key("monster"));

        // Check category index
        assert!(index.by_category.contains_key("weapons"));

        // Check word index - recursive indexing should index all strings
        assert!(
            !index.word_index.is_empty(),
            "Word index should not be empty"
        );
        // "weapons" from category should be indexed
        assert!(index.word_index.contains_key("weapons"));
    }

    #[test]
    fn test_lookup_exact() {
        let items = vec![
            (
                json!({"id": "test_item", "type": "TOOL"}),
                "test_item".to_string(),
                "TOOL".to_string(),
            ),
            (
                json!({"id": "test_weapon", "type": "TOOL"}),
                "test_weapon".to_string(),
                "TOOL".to_string(),
            ),
        ];

        let index = SearchIndex::build(&items);

        // Exact match
        let results = index.lookup_field(&index.by_id, "test_item", true);
        assert_eq!(results.len(), 1);
        assert!(results.contains(&0));
    }

    #[test]
    fn test_lookup_pattern() {
        let items = vec![
            (
                json!({"id": "test_item", "type": "TOOL"}),
                "test_item".to_string(),
                "TOOL".to_string(),
            ),
            (
                json!({"id": "test_weapon", "type": "TOOL"}),
                "test_weapon".to_string(),
                "TOOL".to_string(),
            ),
        ];

        let index = SearchIndex::build(&items);

        // Pattern match
        let results = index.lookup_field(&index.by_id, "test", false);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_word_search() {
        let items = vec![(
            json!({"id": "zombie_soldier", "type": "MONSTER", "name": "Zombie Soldier"}),
            "zombie_soldier".to_string(),
            "MONSTER".to_string(),
        )];

        let index = SearchIndex::build(&items);

        // Word search
        let results = index.search_words("zombie");
        assert_eq!(results.len(), 1);

        let results = index.search_words("soldier");
        assert_eq!(results.len(), 1);
    }
}
