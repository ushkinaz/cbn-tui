//! Shared indexing helpers for building the item index from raw JSON data.
//!
//! This module is synchronous and has no runtime-specific dependencies.
//! Runtimes wrap these helpers with their own progress-reporting and
//! timing mechanisms (native: `std::time::Instant`; web: `now_ms()` + yields).

use crate::app_core::state::VersionEntry;
use crate::model::{BuildInfo, IndexedItem, Root};
use serde_json::Value;

/// Resolves the human-readable game version label, accounting for various
/// combinations of version key, build number, and tag name.
///
/// `file_path` is `Some` when data was loaded from a local file (native only).
pub fn resolve_game_version_label(version: &str, file_path: Option<&str>, root: &Root) -> String {
    if file_path.is_some() && version == "nightly" {
        root.build.tag_name.clone()
    } else if !version.is_empty()
        && version != root.build.build_number
        && version != root.build.tag_name
    {
        format!("{}:{}", version, root.build.tag_name)
    } else {
        root.build.tag_name.clone()
    }
}

/// Builds the version picker entries from a list of fetched builds (native runtime).
pub fn build_version_entries_from_builds(builds: Vec<BuildInfo>) -> Vec<VersionEntry> {
    let mut entries = Vec::new();
    entries.push(VersionEntry {
        label: "stable".to_string(),
        version: "stable".to_string(),
        detail: None,
    });
    entries.push(VersionEntry {
        label: "nightly".to_string(),
        version: "nightly".to_string(),
        detail: None,
    });

    for build in builds {
        if build.build_number == "stable" || build.build_number == "nightly" {
            continue;
        }
        entries.push(VersionEntry {
            label: build.build_number.clone(),
            version: build.build_number,
            detail: None,
        });
    }

    entries
}

/// Fraction of overall indexing progress budget spent building the item list.
/// The remaining `1.0 - ITEMS_PROGRESS_WEIGHT` is spent on search-index construction.
pub const ITEMS_PROGRESS_WEIGHT: f64 = 0.4;

/// Core item indexing loop — converts raw JSON values into `IndexedItem`s.
///
/// `on_progress` receives a value in `[0.0, ITEMS_PROGRESS_WEIGHT]`.
/// The remaining portion is reported by the search-index build step.
///
/// This function does **not** sort the result; the caller is responsible for
/// sorting by `(item_type, id)` after this call completes.
pub fn build_indexed_items<F>(data: Vec<Value>, mut on_progress: F) -> Vec<IndexedItem>
where
    F: FnMut(f64),
{
    let total = data.len();
    let mut indexed_items: Vec<IndexedItem> = Vec::with_capacity(total);

    for (idx, v) in data.into_iter().enumerate() {
        indexed_items.push(IndexedItem::from_value(v));

        if total > 0 && (idx % 500 == 0 || idx + 1 == total) {
            let ratio = (idx + 1) as f64 / total as f64 * ITEMS_PROGRESS_WEIGHT;
            on_progress(ratio);
        }
    }

    indexed_items
}

/// Converts a download progress value into a `[0.0, 1.0]` ratio.
///
/// When total size is unknown, uses a hyperbolic curve that approaches 1.0
/// as `downloaded` grows.
pub fn progress_ratio(downloaded: u64, total: Option<u64>) -> f64 {
    if let Some(t) = total
        && t > 0
    {
        return downloaded as f64 / t as f64;
    }

    let d = downloaded as f64;
    d / (d + 1_000_000.0)
}
