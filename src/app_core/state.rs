//! Shared application state, types, and state-mutation methods.
//!
//! This module is runtime-agnostic. History persistence (`load_history` /
//! `save_history`) are no-ops here; the native runtime calls the filesystem
//! variants directly after constructing `AppState`.

use crate::model::IndexedItem;
use crate::search_index::SearchIndex;
use crate::theme::ThemeConfig;
use crate::ui::{self, AnnotatedSpan, JsonSpanKind};
use crate::{matcher, ui as ui_mod};
use ratatui::{text::Span, widgets::ListState};
use tui_scrollview::ScrollViewState;

/// Current input mode for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Normal navigation mode
    Normal,
    /// Mode for entering filter text
    Filtering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    List,
    Details,
    Filter,
}

#[derive(Debug, Clone)]
pub struct VersionEntry {
    pub label: String,
    pub version: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProgressStage {
    pub label: String,
    pub ratio: f64,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub enum AppAction {
    OpenVersionPicker,
    SwitchVersion(String),
    ReloadSource,
}

/// Application state for the Ratatui app.
pub struct AppState {
    /// All loaded items in indexed format (json, id, type)
    pub indexed_items: Vec<IndexedItem>,
    /// Search index for fast lookups
    pub search_index: SearchIndex,
    /// Set of purely IDs for O(1) existence checks (used for click navigation)
    pub id_set: foldhash::HashSet<String>,
    /// Indices into indexed_items that match the current filter
    pub filtered_indices: Vec<usize>,
    /// List selection state managed by ratatui
    pub list_state: ListState,
    /// Filter input text
    pub filter_text: String,
    /// Cursor position in filter
    pub filter_cursor: usize,
    /// Current input mode
    pub input_mode: InputMode,
    /// Which pane currently has keyboard focus
    pub focused_pane: FocusPane,
    /// Theme configuration
    pub theme: ThemeConfig,
    /// Resolved game version (from JSON tag_name)
    pub game_version: String,
    /// Requested game version key (stable/nightly/build number)
    pub game_version_key: String,
    /// App version string
    pub app_version: String,
    /// Whether to force downloads when switching
    pub force_download: bool,
    /// Number of items in the full dataset
    pub total_items: usize,
    /// Time taken to build the index
    pub index_time_ms: f64,
    /// State for scrolling the details pane
    pub details_scroll_state: ScrollViewState,
    /// Annotated spans for the current details view
    pub details_annotated: Vec<Vec<AnnotatedSpan>>,
    /// Pre-wrapped annotated spans for the current content_width (used for rendering and hit-testing)
    pub details_wrapped_annotated: Vec<Vec<AnnotatedSpan>>,
    /// Width used for current details_wrapped_annotated
    pub details_wrapped_width: u16,
    /// Currently hovered span ID for tracking click/hover
    pub hovered_span_id: Option<usize>,
    /// Screen region of the JSON content area (set during render)
    pub details_content_area: Option<ratatui::layout::Rect>,
    /// Screen region of the item list pane (including borders)
    pub list_area: Option<ratatui::layout::Rect>,
    /// Screen region of list content (inside borders)
    pub list_content_area: Option<ratatui::layout::Rect>,
    /// Screen region of the details pane (including borders)
    pub details_area: Option<ratatui::layout::Rect>,
    /// Screen region of the filter pane (including borders)
    pub filter_area: Option<ratatui::layout::Rect>,
    /// Screen region of the filter text area (inside borders)
    pub filter_input_area: Option<ratatui::layout::Rect>,
    /// Flag to quit app
    pub should_quit: bool,
    /// Whether help overlay is visible
    pub show_help: bool,
    /// Whether a version picker is visible
    pub show_version_picker: bool,
    /// List of available versions for the picker
    pub version_entries: Vec<VersionEntry>,
    /// Selection state for version picker
    pub version_list_state: ListState,
    /// Whether progress modal is visible
    pub show_progress: bool,
    /// Progress modal title
    pub progress_title: String,
    /// Progress stages for modal display
    pub progress_stages: Vec<ProgressStage>,
    /// Previous search expressions
    pub filter_history: Vec<String>,
    /// Current index in history during navigation
    pub history_index: Option<usize>,
    /// Saved input when starting history navigation
    pub stashed_input: String,
    /// Path to history file (used by native runtime; web passes empty path)
    pub history_path: std::path::PathBuf,
    /// Pending action to execute after input handling
    pub pending_action: Option<AppAction>,
    /// Source directory, if in --source mode
    pub source_dir: Option<String>,
    /// Warnings accumulated during source loading
    pub source_warnings: Vec<String>,
    /// Index into indexed_items that is currently rendered in the details pane.
    /// Used to skip expensive JSON re-rendering when the same item is re-selected.
    pub cached_details_item_idx: Option<usize>,
    /// Pre-computed (display_name, type_prefix) strings for the current filtered list.
    /// Rebuilt only when filtered_indices changes, used by render_item_list via &str borrows
    /// to avoid JSON traversal and String allocations on every frame.
    pub cached_display: Vec<(String, String)>,
    /// Cached horizontal separator for the details pane to avoid an allocation per frame.
    /// Stores the width and the generated string.
    pub cached_separator: (u16, String),
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        indexed_items: Vec<IndexedItem>,
        search_index: SearchIndex,
        theme: ThemeConfig,
        game_version: String,
        game_version_key: String,
        app_version: String,
        force_download: bool,
        total_items: usize,
        index_time_ms: f64,
        history_path: std::path::PathBuf,
        source_dir: Option<String>,
    ) -> Self {
        let filtered_indices: Vec<usize> = (0..indexed_items.len()).collect();
        let id_set = indexed_items
            .iter()
            .filter(|item| !item.id.is_empty())
            .map(|item| item.id.clone())
            .collect();
        let mut list_state = ListState::default();
        if filtered_indices.is_empty() {
            list_state.select(None);
        } else {
            list_state.select(Some(0));
        }

        let mut app = Self {
            indexed_items,
            search_index,
            id_set,
            filtered_indices,
            list_state,
            filter_text: String::new(),
            filter_cursor: 0,
            input_mode: InputMode::Normal,
            focused_pane: FocusPane::List,
            theme,
            game_version,
            game_version_key,
            app_version,
            force_download,
            total_items,
            index_time_ms,
            details_scroll_state: ScrollViewState::default(),
            details_annotated: Vec::new(),
            details_wrapped_annotated: Vec::new(),
            details_wrapped_width: 0,
            hovered_span_id: None,
            details_content_area: None,
            list_area: None,
            list_content_area: None,
            details_area: None,
            filter_area: None,
            filter_input_area: None,
            should_quit: false,
            show_help: false,
            show_version_picker: false,
            version_entries: Vec::new(),
            version_list_state: ListState::default(),
            show_progress: false,
            progress_title: String::new(),
            progress_stages: Vec::new(),
            filter_history: Vec::new(),
            history_index: None,
            stashed_input: String::new(),
            history_path,
            pending_action: None,
            source_dir,
            source_warnings: Vec::new(),
            cached_details_item_idx: None,
            cached_display: Vec::new(),
            cached_separator: (0, String::new()),
        };
        app.refresh_details();
        app
    }

    /// No-op in shared code. The native runtime calls `load_history_from_fs` after `new()`.
    pub fn load_history(&mut self) {}

    /// No-op in shared code. The native runtime calls `save_history_to_fs` directly.
    pub fn save_history(&self) {}

    /// Gets or creates the horizontal separator for a given width.
    pub fn get_separator(&mut self, width: u16) -> &str {
        if self.cached_separator.0 != width {
            let separator = format!("├{}┤", "─".repeat(width as usize));
            self.cached_separator = (width, separator);
        }
        &self.cached_separator.1
    }

    pub fn refresh_details(&mut self) {
        // Resolve the indexed_items index for the current selection.
        let selected_item_idx = self
            .list_state
            .selected()
            .and_then(|sel| self.filtered_indices.get(sel).copied());

        // Always reset the scroll so navigation feels snappy.
        self.details_scroll_state = ScrollViewState::default();

        // Skip the expensive serde_json::to_string_pretty + highlight pass when
        // the same item is already rendered. The wrapped cache is kept intact, so
        // the width-change guard in render_details still triggers a re-wrap on resize.
        if self.cached_details_item_idx == selected_item_idx && selected_item_idx.is_some() {
            return;
        }
        self.cached_details_item_idx = selected_item_idx;

        if let Some(item) = self.get_selected_item() {
            match serde_json::to_string_pretty(&item.value) {
                Ok(json_str) => {
                    self.details_annotated =
                        ui::highlight_json_annotated(&json_str, &self.theme.json_style);
                }
                Err(_) => {
                    self.details_annotated = vec![vec![AnnotatedSpan {
                        span: Span::raw("Error formatting JSON"),
                        kind: JsonSpanKind::Whitespace,
                        key_context: None,
                        span_id: None,
                    }]];
                }
            }
        } else {
            self.details_annotated = vec![vec![AnnotatedSpan {
                span: Span::raw("Select an item to view details"),
                kind: JsonSpanKind::Whitespace,
                key_context: None,
                span_id: None,
            }]];
        }
        // Invalidate wrapped cache so render_details re-wraps for the new content.
        self.details_wrapped_width = 0;
        self.details_wrapped_annotated.clear();
    }

    /// Clamps the current list selection to valid bounds.
    pub fn clamp_selection(&mut self) {
        let len = self.filtered_indices.len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }

        if let Some(selected) = self.list_state.selected()
            && selected >= len
        {
            self.list_state.select(Some(len - 1));
        }
    }

    /// Moves selection by `direction` (+1 or -1) and refreshes details.
    pub fn move_selection(&mut self, direction: i32) {
        if direction < 0 {
            self.list_state.select_previous();
        } else {
            self.list_state.select_next();
        }
        self.clamp_selection();
        self.refresh_details();
    }

    pub fn get_selected_item(&self) -> Option<&IndexedItem> {
        self.list_state
            .selected()
            .and_then(|idx| self.filtered_indices.get(idx))
            .and_then(|&idx| self.indexed_items.get(idx))
    }

    pub fn scroll_details_up(&mut self) {
        self.details_scroll_state.scroll_up();
    }

    pub fn scroll_details_down(&mut self) {
        self.details_scroll_state.scroll_down();
    }

    pub fn scroll_details_by_lines(&mut self, lines: u16, down: bool) {
        for _ in 0..lines {
            if down {
                self.scroll_details_down();
            } else {
                self.scroll_details_up();
            }
        }
    }

    pub fn filter_add_char(&mut self, c: char) {
        let byte_idx = self
            .filter_text
            .char_indices()
            .nth(self.filter_cursor)
            .map(|(idx, _)| idx)
            .unwrap_or(self.filter_text.len());
        self.filter_text.insert(byte_idx, c);
        self.filter_cursor += 1;
    }

    pub fn filter_backspace(&mut self) {
        if self.filter_cursor > 0 {
            self.filter_cursor -= 1;
            if let Some((byte_idx, _)) = self.filter_text.char_indices().nth(self.filter_cursor) {
                self.filter_text.remove(byte_idx);
            }
        }
    }

    pub fn filter_delete(&mut self) {
        let char_count = self.filter_text.chars().count();
        if self.filter_cursor < char_count
            && let Some((byte_idx, _)) = self.filter_text.char_indices().nth(self.filter_cursor)
        {
            self.filter_text.remove(byte_idx);
        }
    }

    pub fn filter_move_cursor_left(&mut self) {
        if self.filter_cursor > 0 {
            self.filter_cursor -= 1;
        }
    }

    pub fn filter_move_cursor_right(&mut self) {
        let char_count = self.filter_text.chars().count();
        if self.filter_cursor < char_count {
            self.filter_cursor += 1;
        }
    }

    pub fn filter_move_to_start(&mut self) {
        self.filter_cursor = 0;
    }

    pub fn filter_move_to_end(&mut self) {
        self.filter_cursor = self.filter_text.chars().count();
    }

    pub fn filter_clear(&mut self) {
        self.filter_text.clear();
        self.filter_cursor = 0;
    }

    pub fn filter_delete_word(&mut self) {
        if self.filter_cursor == 0 {
            return;
        }

        let chars: Vec<char> = self.filter_text.chars().collect();
        let mut i = self.filter_cursor;

        // Skip trailing whitespace
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }

        // Skip non-whitespace (the word)
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }

        let new_cursor = i;
        let _char_count = chars.len();

        let byte_start = self
            .filter_text
            .char_indices()
            .nth(new_cursor)
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        let byte_end = self
            .filter_text
            .char_indices()
            .nth(self.filter_cursor)
            .map(|(idx, _)| idx)
            .unwrap_or(self.filter_text.len());

        self.filter_text.replace_range(byte_start..byte_end, "");
        self.filter_cursor = new_cursor;
    }

    pub fn focus_pane(&mut self, pane: FocusPane) {
        self.focused_pane = pane;
        self.input_mode = if pane == FocusPane::Filter {
            InputMode::Filtering
        } else {
            InputMode::Normal
        };
    }

    pub fn focus_next_pane(&mut self) {
        let next = match self.focused_pane {
            FocusPane::Filter => FocusPane::List,
            FocusPane::List => FocusPane::Details,
            FocusPane::Details => FocusPane::Filter,
        };
        self.focus_pane(next);
    }

    pub fn focus_prev_pane(&mut self) {
        let prev = match self.focused_pane {
            FocusPane::Filter => FocusPane::Details,
            FocusPane::List => FocusPane::Filter,
            FocusPane::Details => FocusPane::List,
        };
        self.focus_pane(prev);
    }

    pub fn update_filter(&mut self) {
        let new_filtered =
            matcher::find_matches(&self.filter_text, &self.indexed_items, &self.search_index);
        self.filtered_indices = new_filtered;
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        // Rebuild display cache whenever the filtered set changes.
        self.rebuild_display_cache();
        self.refresh_details();
    }

    /// Rebuilds cached_display from the current filtered_indices.
    /// Called only when the filter result set changes — not on every frame.
    pub fn rebuild_display_cache(&mut self) {
        self.cached_display = self
            .filtered_indices
            .iter()
            .map(|&idx| {
                let item = &self.indexed_items[idx];
                let display = ui_mod::display_name_for_item(&item.value, &item.id, &item.item_type);
                // Pre-format the type prefix once so render borrows it as &str.
                let type_prefix = format!("{} ", item.item_type);
                (display, type_prefix)
            })
            .collect();
    }

    pub fn apply_new_dataset(
        &mut self,
        indexed_items: Vec<IndexedItem>,
        search_index: SearchIndex,
        total_items: usize,
        index_time_ms: f64,
        game_version: String,
        game_version_key: String,
    ) {
        let filter_text = self.filter_text.clone();
        let filter_cursor = self.filter_cursor.min(filter_text.chars().count());

        let id_set = indexed_items
            .iter()
            .filter(|item| !item.id.is_empty())
            .map(|item| item.id.clone())
            .collect();

        self.indexed_items = indexed_items;
        self.search_index = search_index;
        self.id_set = id_set;
        self.total_items = total_items;
        // New dataset means all item indices are stale — force a re-render.
        self.cached_details_item_idx = None;
        self.index_time_ms = index_time_ms;
        self.game_version = game_version;
        self.game_version_key = game_version_key;
        self.filter_text = filter_text;
        self.filter_cursor = filter_cursor;
        self.update_filter();
    }

    pub fn start_progress(&mut self, title: impl Into<String>, stages: &[&str]) {
        self.show_progress = true;
        self.progress_title = title.into();
        self.progress_stages = stages
            .iter()
            .map(|label| ProgressStage {
                label: (*label).to_string(),
                ratio: 0.0,
                done: false,
            })
            .collect();
    }

    pub fn update_stage(&mut self, label: &str, ratio: f64) {
        if let Some(stage) = self
            .progress_stages
            .iter_mut()
            .find(|stage| stage.label == label)
        {
            stage.ratio = ratio.clamp(0.0, 1.0);
            if stage.ratio >= 1.0 {
                stage.done = true;
            }
        }
    }

    pub fn finish_stage(&mut self, label: &str) {
        self.update_stage(label, 1.0);
    }

    pub fn clear_progress(&mut self) {
        self.show_progress = false;
        self.progress_title.clear();
        self.progress_stages.clear();
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn clear_filter(&mut self) {
        self.filter_text.clear();
        self.filter_cursor = 0;
        self.update_filter();
    }
}
