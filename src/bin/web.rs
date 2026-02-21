#![cfg_attr(
    not(target_arch = "wasm32"),
    allow(dead_code, unused_imports, unused_variables)
)]

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("cbn-tui-web must be built for wasm32-unknown-unknown");
}

#[cfg(target_arch = "wasm32")]
use anyhow::Result;
#[cfg(target_arch = "wasm32")]
use foldhash::{HashMap, HashSet};
#[cfg(target_arch = "wasm32")]
use js_sys::Promise;
#[cfg(target_arch = "wasm32")]
use ratatui::{Terminal, widgets::ListState};
#[cfg(target_arch = "wasm32")]
use ratzilla::web_sys::wasm_bindgen::{JsCast, JsValue};
#[cfg(target_arch = "wasm32")]
use ratzilla::{
    DomBackend, WebRenderer,
    event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
};
#[cfg(target_arch = "wasm32")]
use serde_json::Value;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::str::FromStr;
#[cfg(target_arch = "wasm32")]
use tui_scrollview::ScrollViewState;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::{JsFuture, spawn_local};

#[cfg(target_arch = "wasm32")]
#[path = "../web_data.rs"]
mod data;
#[cfg(target_arch = "wasm32")]
#[path = "../matcher.rs"]
mod matcher;
#[cfg(target_arch = "wasm32")]
#[path = "../search_index.rs"]
mod search_index;
#[cfg(target_arch = "wasm32")]
#[path = "../theme.rs"]
mod theme;
#[cfg(target_arch = "wasm32")]
#[path = "../ui.rs"]
mod ui;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filtering,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    List,
    Details,
    Filter,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone)]
pub struct VersionEntry {
    pub label: String,
    pub version: String,
    pub detail: Option<String>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone)]
pub struct ProgressStage {
    pub label: String,
    pub ratio: f64,
    pub done: bool,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone)]
enum AppAction {
    OpenVersionPicker,
    SwitchVersion(String),
    ReloadSource,
}

#[cfg(target_arch = "wasm32")]
pub struct AppState {
    pub indexed_items: Vec<data::IndexedItem>,
    pub search_index: search_index::SearchIndex,
    pub id_set: foldhash::HashSet<String>,
    pub filtered_indices: Vec<usize>,
    pub list_state: ListState,
    pub filter_text: String,
    pub filter_cursor: usize,
    pub input_mode: InputMode,
    pub focused_pane: FocusPane,
    pub theme: theme::ThemeConfig,
    pub game_version: String,
    pub game_version_key: String,
    pub app_version: String,
    pub force_download: bool,
    pub total_items: usize,
    pub index_time_ms: f64,
    pub details_scroll_state: ScrollViewState,
    pub details_annotated: Vec<Vec<ui::AnnotatedSpan>>,
    pub details_wrapped_annotated: Vec<Vec<ui::AnnotatedSpan>>,
    pub details_wrapped_width: u16,
    pub hovered_span_id: Option<usize>,
    pub details_content_area: Option<ratatui::layout::Rect>,
    pub list_area: Option<ratatui::layout::Rect>,
    pub list_content_area: Option<ratatui::layout::Rect>,
    pub details_area: Option<ratatui::layout::Rect>,
    pub filter_area: Option<ratatui::layout::Rect>,
    pub filter_input_area: Option<ratatui::layout::Rect>,
    pub should_quit: bool,
    pub show_help: bool,
    pub show_version_picker: bool,
    pub version_entries: Vec<VersionEntry>,
    pub version_list_state: ListState,
    pub show_progress: bool,
    pub progress_title: String,
    pub progress_stages: Vec<ProgressStage>,
    pub filter_history: Vec<String>,
    pub history_index: Option<usize>,
    pub stashed_input: String,
    pending_action: Option<AppAction>,
    pub source_dir: Option<String>,
    pub source_warnings: Vec<String>,
    cached_details_item_idx: Option<usize>,
    pub cached_display: Vec<(String, String)>,
    cached_separator: (u16, String),
}

#[cfg(target_arch = "wasm32")]
impl AppState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        indexed_items: Vec<data::IndexedItem>,
        search_index: search_index::SearchIndex,
        theme: theme::ThemeConfig,
        game_version: String,
        game_version_key: String,
        app_version: String,
        force_download: bool,
        total_items: usize,
        index_time_ms: f64,
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
            pending_action: None,
            source_dir,
            source_warnings: Vec::new(),
            cached_details_item_idx: None,
            cached_display: Vec::new(),
            cached_separator: (0, String::new()),
        };
        app.load_history();
        app.refresh_details();
        app
    }

    fn load_history(&mut self) {
        // Keep history in-memory for web runtime.
    }

    fn save_history(&self) {
        // Keep history in-memory for web runtime.
    }

    pub fn get_separator(&mut self, width: u16) -> &str {
        if self.cached_separator.0 != width {
            let separator = format!("├{}┤", "─".repeat(width as usize));
            self.cached_separator = (width, separator);
        }
        &self.cached_separator.1
    }

    fn refresh_details(&mut self) {
        let selected_item_idx = self
            .list_state
            .selected()
            .and_then(|sel| self.filtered_indices.get(sel).copied());

        self.details_scroll_state = ScrollViewState::default();

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
                    self.details_annotated = vec![vec![ui::AnnotatedSpan {
                        span: ratatui::text::Span::raw("Error formatting JSON"),
                        kind: ui::JsonSpanKind::Whitespace,
                        key_context: None,
                        span_id: None,
                    }]];
                }
            }
        } else {
            self.details_annotated = vec![vec![ui::AnnotatedSpan {
                span: ratatui::text::Span::raw("Select an item to view details"),
                kind: ui::JsonSpanKind::Whitespace,
                key_context: None,
                span_id: None,
            }]];
        }

        self.details_wrapped_width = 0;
        self.details_wrapped_annotated.clear();
    }

    fn clamp_selection(&mut self) {
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

    fn move_selection(&mut self, direction: i32) {
        if direction < 0 {
            self.list_state.select_previous();
        } else {
            self.list_state.select_next();
        }
        self.clamp_selection();
        self.refresh_details();
    }

    pub fn get_selected_item(&self) -> Option<&data::IndexedItem> {
        self.list_state
            .selected()
            .and_then(|idx| self.filtered_indices.get(idx))
            .and_then(|&idx| self.indexed_items.get(idx))
    }

    fn scroll_details_up(&mut self) {
        self.details_scroll_state.scroll_up();
    }

    fn scroll_details_down(&mut self) {
        self.details_scroll_state.scroll_down();
    }

    fn filter_add_char(&mut self, c: char) {
        let byte_idx = self
            .filter_text
            .char_indices()
            .nth(self.filter_cursor)
            .map(|(idx, _)| idx)
            .unwrap_or(self.filter_text.len());
        self.filter_text.insert(byte_idx, c);
        self.filter_cursor += 1;
    }

    fn filter_backspace(&mut self) {
        if self.filter_cursor > 0 {
            self.filter_cursor -= 1;
            if let Some((byte_idx, _)) = self.filter_text.char_indices().nth(self.filter_cursor) {
                self.filter_text.remove(byte_idx);
            }
        }
    }

    fn filter_delete(&mut self) {
        let char_count = self.filter_text.chars().count();
        if self.filter_cursor < char_count
            && let Some((byte_idx, _)) = self.filter_text.char_indices().nth(self.filter_cursor)
        {
            self.filter_text.remove(byte_idx);
        }
    }

    fn filter_move_cursor_left(&mut self) {
        if self.filter_cursor > 0 {
            self.filter_cursor -= 1;
        }
    }

    fn filter_move_cursor_right(&mut self) {
        let char_count = self.filter_text.chars().count();
        if self.filter_cursor < char_count {
            self.filter_cursor += 1;
        }
    }

    fn filter_move_to_start(&mut self) {
        self.filter_cursor = 0;
    }

    fn filter_move_to_end(&mut self) {
        self.filter_cursor = self.filter_text.chars().count();
    }

    fn filter_clear(&mut self) {
        self.filter_text.clear();
        self.filter_cursor = 0;
    }

    fn filter_delete_word(&mut self) {
        if self.filter_cursor == 0 {
            return;
        }

        let chars: Vec<char> = self.filter_text.chars().collect();
        let mut i = self.filter_cursor;

        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }

        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }

        let new_cursor = i;

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

    fn focus_pane(&mut self, pane: FocusPane) {
        self.focused_pane = pane;
        self.input_mode = if pane == FocusPane::Filter {
            InputMode::Filtering
        } else {
            InputMode::Normal
        };
    }

    fn focus_next_pane(&mut self) {
        let next = match self.focused_pane {
            FocusPane::Filter => FocusPane::List,
            FocusPane::List => FocusPane::Details,
            FocusPane::Details => FocusPane::Filter,
        };
        self.focus_pane(next);
    }

    fn focus_prev_pane(&mut self) {
        let prev = match self.focused_pane {
            FocusPane::Filter => FocusPane::Details,
            FocusPane::List => FocusPane::Filter,
            FocusPane::Details => FocusPane::List,
        };
        self.focus_pane(prev);
    }

    fn update_filter(&mut self) {
        let new_filtered =
            matcher::find_matches(&self.filter_text, &self.indexed_items, &self.search_index);
        self.filtered_indices = new_filtered;
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        self.rebuild_display_cache();
        self.refresh_details();
    }

    fn rebuild_display_cache(&mut self) {
        self.cached_display = self
            .filtered_indices
            .iter()
            .map(|&idx| {
                let item = &self.indexed_items[idx];
                let display = ui::display_name_for_item(&item.value, &item.id, &item.item_type);
                let type_prefix = format!("{} ", item.item_type);
                (display, type_prefix)
            })
            .collect();
    }

    fn apply_new_dataset(
        &mut self,
        indexed_items: Vec<data::IndexedItem>,
        search_index: search_index::SearchIndex,
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
        self.cached_details_item_idx = None;
        self.index_time_ms = index_time_ms;
        self.game_version = game_version;
        self.game_version_key = game_version_key;
        self.filter_text = filter_text;
        self.filter_cursor = filter_cursor;
        self.update_filter();
    }

    fn start_progress(&mut self, title: impl Into<String>, stages: &[&str]) {
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

    fn update_stage(&mut self, label: &str, ratio: f64) {
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

    fn finish_stage(&mut self, label: &str) {
        self.update_stage(label, 1.0);
    }

    fn clear_progress(&mut self) {
        self.show_progress = false;
        self.progress_title.clear();
        self.progress_stages.clear();
    }
}

#[cfg(target_arch = "wasm32")]
fn main() -> Result<()> {
    console_error_panic_hook::set_once();

    let app_version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let theme_name = "dracula";
    let theme_enum = theme::Theme::from_str(theme_name).map_err(anyhow::Error::msg)?;
    let theme = theme_enum.config();

    let app = Rc::new(RefCell::new(AppState::new(
        Vec::new(),
        search_index::SearchIndex::new(),
        theme,
        "loading".to_string(),
        "nightly".to_string(),
        app_version,
        false,
        0,
        0.0,
        None,
    )));

    start_load_version(app.clone(), "nightly".to_string());

    let backend = DomBackend::new_by_id("grid").map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let terminal = Terminal::new(backend)?;

    terminal.on_key_event({
        let event_state = app.clone();
        move |key_event| {
            let pending_action = {
                let mut state = event_state.borrow_mut();
                handle_key_event(&mut state, key_event);
                state.pending_action.take()
            };

            if let Some(action) = pending_action {
                handle_action(event_state.clone(), action);
            }
        }
    });

    terminal.on_mouse_event({
        let event_state = app.clone();
        move |mouse_event| {
            let pending_action = {
                let mut state = event_state.borrow_mut();
                let _ = handle_mouse_event(&mut state, mouse_event);
                state.pending_action.take()
            };

            if let Some(action) = pending_action {
                handle_action(event_state.clone(), action);
            }
        }
    });

    terminal.draw_web(move |f| {
        let mut state = app.borrow_mut();
        ui::ui(f, &mut state);
    });

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn handle_action(app_state: Rc<RefCell<AppState>>, action: AppAction) {
    match action {
        AppAction::OpenVersionPicker => {
            let mut app = app_state.borrow_mut();
            app.version_entries = build_version_entries(&app.game_version_key);
            let selected = app
                .version_entries
                .iter()
                .position(|entry| entry.version == app.game_version_key)
                .unwrap_or(0);
            let has_entries = !app.version_entries.is_empty();
            app.version_list_state
                .select(if has_entries { Some(selected) } else { None });
            app.show_version_picker = true;
        }
        AppAction::SwitchVersion(version) => {
            start_load_version(app_state, version);
        }
        AppAction::ReloadSource => {
            let version = app_state.borrow().game_version_key.clone();
            start_load_version(app_state, version);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_load_version(app_state: Rc<RefCell<AppState>>, version: String) {
    {
        let mut app = app_state.borrow_mut();
        app.show_version_picker = false;
        app.start_progress("Loading data", &["Downloading", "Parsing", "Indexing"]);
        app.source_warnings.clear();
    }

    spawn_local(async move {
        let load_result = load_game_data(&app_state, &version).await;

        if let Err(err) = load_result {
            let mut app = app_state.borrow_mut();
            app.clear_progress();
            app.source_warnings = vec![format!("Web load failed: {err}")];
            app.details_annotated = vec![vec![ui::AnnotatedSpan {
                span: ratatui::text::Span::raw(format!("Failed to load data: {err}")),
                kind: ui::JsonSpanKind::Whitespace,
                key_context: None,
                span_id: None,
            }]];
            app.details_wrapped_width = 0;
            app.details_wrapped_annotated.clear();
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn load_game_data(app_state: &Rc<RefCell<AppState>>, version: &str) -> Result<()> {
    {
        let mut app = app_state.borrow_mut();
        app.update_stage("Downloading", 0.05);
    }

    let root = data::fetch_game_root(version).await?;

    {
        let mut app = app_state.borrow_mut();
        app.finish_stage("Downloading");
        app.finish_stage("Parsing");
        app.update_stage("Indexing", 0.01);
    }

    let game_version_label = resolve_game_version_label(version, &root);
    let total_items = root.data.len();
    let (indexed_items, search_index, index_time_ms) =
        build_index_with_progress(root.data, |ratio| {
            let mut app = app_state.borrow_mut();
            app.update_stage("Indexing", ratio);
        })
        .await;

    {
        let mut app = app_state.borrow_mut();
        app.apply_new_dataset(
            indexed_items,
            search_index,
            total_items,
            index_time_ms,
            game_version_label,
            version.to_string(),
        );
        app.finish_stage("Indexing");
        app.clear_progress();
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn yield_to_browser() {
    let promise = Promise::new(&mut |resolve, _reject| {
        if let Some(window) = ratzilla::web_sys::window() {
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0);
        } else {
            let _ = resolve.call0(&JsValue::NULL);
        }
    });

    let _ = JsFuture::from(promise).await;
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    if let Some(window) = ratzilla::web_sys::window()
        && let Some(performance) = window.performance()
    {
        return performance.now();
    }
    js_sys::Date::now()
}

#[cfg(target_arch = "wasm32")]
async fn build_index_with_progress<F>(
    data: Vec<Value>,
    mut on_progress: F,
) -> (Vec<data::IndexedItem>, search_index::SearchIndex, f64)
where
    F: FnMut(f64),
{
    let total = data.len();
    let start_ms = now_ms();
    let mut indexed_items: Vec<data::IndexedItem> = Vec::with_capacity(total);

    for (idx, v) in data.into_iter().enumerate() {
        let id = v
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let type_ = v
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        indexed_items.push(data::IndexedItem {
            value: v,
            id,
            item_type: type_,
        });

        if total > 0 && (idx % 500 == 0 || idx + 1 == total) {
            let ratio = (idx + 1) as f64 / total as f64 * 0.4;
            on_progress(ratio);
            yield_to_browser().await;
        }
    }

    indexed_items.sort_by(|a, b| a.item_type.cmp(&b.item_type).then_with(|| a.id.cmp(&b.id)));
    on_progress(0.4);
    yield_to_browser().await;

    let search_index =
        build_search_index_with_progress(&indexed_items, |processed, total_items| {
            let ratio = if total_items > 0 {
                0.4 + 0.6 * (processed as f64 / total_items as f64)
            } else {
                1.0
            };
            on_progress(ratio);
        })
        .await;

    let index_time_ms = (now_ms() - start_ms).max(0.0);
    (indexed_items, search_index, index_time_ms)
}

#[cfg(target_arch = "wasm32")]
async fn build_search_index_with_progress<F>(
    items: &[data::IndexedItem],
    mut on_progress: F,
) -> search_index::SearchIndex
where
    F: FnMut(usize, usize),
{
    let mut index = search_index::SearchIndex::new();
    let total = items.len();

    for (idx, item) in items.iter().enumerate() {
        let json = &item.value;
        let id = &item.id;
        let type_ = &item.item_type;

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

        index_value_recursive(&mut index.word_index, json, idx);

        if idx % 250 == 0 || idx + 1 == total {
            on_progress(idx + 1, total);
        }
        if idx % 1000 == 0 || idx + 1 == total {
            yield_to_browser().await;
        }
    }

    index
}

#[cfg(target_arch = "wasm32")]
fn index_value_recursive(
    word_index: &mut HashMap<String, HashSet<usize>>,
    value: &Value,
    idx: usize,
) {
    match value {
        Value::String(s) => {
            index_words(word_index, s, idx);
        }
        Value::Array(arr) => {
            for item in arr {
                index_value_recursive(word_index, item, idx);
            }
        }
        Value::Object(obj) => {
            for val in obj.values() {
                index_value_recursive(word_index, val, idx);
            }
        }
        _ => {}
    }
}

#[cfg(target_arch = "wasm32")]
fn index_words(word_index: &mut HashMap<String, HashSet<usize>>, text: &str, idx: usize) {
    for word in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if word.len() < 2 {
            continue;
        }

        let is_lowercase = word.chars().all(|c| !c.is_uppercase());
        if is_lowercase && let Some(set) = word_index.get_mut(word) {
            set.insert(idx);
            continue;
        }

        let word_lower = word.to_lowercase();
        if word_lower.len() >= 2 {
            word_index.entry(word_lower).or_default().insert(idx);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn resolve_game_version_label(version: &str, root: &data::Root) -> String {
    if !version.is_empty() && version != root.build.build_number && version != root.build.tag_name {
        format!("{}:{}", version, root.build.tag_name)
    } else {
        root.build.tag_name.clone()
    }
}

#[cfg(target_arch = "wasm32")]
fn build_version_entries(current: &str) -> Vec<VersionEntry> {
    let mut entries = vec![
        VersionEntry {
            label: "stable".to_string(),
            version: "stable".to_string(),
            detail: None,
        },
        VersionEntry {
            label: "nightly".to_string(),
            version: "nightly".to_string(),
            detail: None,
        },
    ];

    if current != "stable" && current != "nightly" {
        entries.push(VersionEntry {
            label: current.to_string(),
            version: current.to_string(),
            detail: Some("current".to_string()),
        });
    }

    entries
}

#[cfg(target_arch = "wasm32")]
fn handle_key_event(app: &mut AppState, key_event: KeyEvent) {
    fn apply_filter_edit(app: &mut AppState, edit: impl FnOnce(&mut AppState)) {
        edit(app);
        app.update_filter();
    }

    let code = key_event.code;
    let ctrl = key_event.ctrl;
    let alt = key_event.alt;
    let shift = key_event.shift;

    if ctrl && code == KeyCode::Char('g') {
        app.show_help = false;
        app.show_version_picker = false;
        app.focus_pane(FocusPane::List);
        app.history_index = None;
        app.pending_action = Some(AppAction::OpenVersionPicker);
        return;
    }

    if ctrl && code == KeyCode::Char('r') {
        app.pending_action = Some(AppAction::ReloadSource);
        return;
    }

    if code == KeyCode::Tab {
        if shift {
            app.focus_prev_pane();
        } else {
            app.focus_next_pane();
        }
        return;
    }

    if app.show_help {
        if matches!(code, KeyCode::Char('?') | KeyCode::Esc) {
            app.show_help = false;
        }
        return;
    }

    if app.show_version_picker {
        match code {
            KeyCode::Esc => app.show_version_picker = false,
            KeyCode::Up => app.version_list_state.select_previous(),
            KeyCode::Down => app.version_list_state.select_next(),
            KeyCode::Enter => {
                if let Some(idx) = app.version_list_state.selected()
                    && let Some(entry) = app.version_entries.get(idx)
                {
                    app.pending_action = Some(AppAction::SwitchVersion(entry.version.clone()));
                }
            }
            _ => {}
        }
        return;
    }

    match app.input_mode {
        InputMode::Normal => match code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Char('/') => app.focus_pane(FocusPane::Filter),
            KeyCode::Char('?') => app.show_help = true,
            KeyCode::Up if !ctrl => {
                if app.focused_pane == FocusPane::Details {
                    app.scroll_details_up();
                } else {
                    app.move_selection(-1);
                }
            }
            KeyCode::Down if !ctrl => {
                if app.focused_pane == FocusPane::Details {
                    app.scroll_details_down();
                } else {
                    app.move_selection(1);
                }
            }
            KeyCode::Home => {
                if app.focused_pane == FocusPane::Details {
                    app.details_scroll_state = ScrollViewState::default();
                } else {
                    app.list_state.select(Some(0));
                    app.refresh_details();
                }
            }
            KeyCode::End => {
                if app.focused_pane == FocusPane::Details {
                    app.details_scroll_state.scroll_to_bottom();
                } else {
                    let len = app.filtered_indices.len();
                    if len > 0 {
                        app.list_state.select(Some(len - 1));
                        app.refresh_details();
                    }
                }
            }
            KeyCode::PageUp => {
                if app.focused_pane == FocusPane::Details {
                    app.details_scroll_state.scroll_page_up();
                } else {
                    let page_size = app.list_area.map(|a| a.height).unwrap_or(10) as i32;
                    let current = app.list_state.selected().unwrap_or(0);
                    let new_sel = current.saturating_sub(page_size as usize);
                    app.list_state.select(Some(new_sel));
                    app.refresh_details();
                }
            }
            KeyCode::PageDown => {
                if app.focused_pane == FocusPane::Details {
                    app.details_scroll_state.scroll_page_down();
                } else {
                    let page_size = app.list_area.map(|a| a.height).unwrap_or(10) as i32;
                    let current = app.list_state.selected().unwrap_or(0);
                    let len = app.filtered_indices.len();
                    if len > 0 {
                        let new_sel = (current + page_size as usize).min(len - 1);
                        app.list_state.select(Some(new_sel));
                        app.refresh_details();
                    }
                }
            }
            KeyCode::Char('r') if ctrl => {
                app.pending_action = Some(AppAction::ReloadSource);
            }
            KeyCode::Char(c) if c.is_alphanumeric() && !ctrl && !alt => {
                app.focus_pane(FocusPane::Filter);
                app.filter_move_to_end();
                apply_filter_edit(app, |app| app.filter_add_char(c));
            }
            _ => {}
        },
        InputMode::Filtering => match code {
            KeyCode::Enter => {
                if !app.filter_text.trim().is_empty()
                    && app.filter_history.last() != Some(&app.filter_text)
                {
                    app.filter_history.push(app.filter_text.clone());
                    app.save_history();
                }
                app.history_index = None;
                app.focus_pane(FocusPane::List);
            }
            KeyCode::Esc => {
                app.history_index = None;
                app.focus_pane(FocusPane::List);
            }
            KeyCode::Char('u') if ctrl => {
                apply_filter_edit(app, AppState::filter_clear);
            }
            KeyCode::Char('w') if ctrl => {
                apply_filter_edit(app, AppState::filter_delete_word);
            }
            KeyCode::Char('a') if ctrl => {
                app.filter_move_to_start();
            }
            KeyCode::Char('e') if ctrl => {
                app.filter_move_to_end();
            }
            KeyCode::Char(c) if !ctrl => {
                app.history_index = None;
                apply_filter_edit(app, |app| app.filter_add_char(c));
            }
            KeyCode::Backspace => {
                app.history_index = None;
                apply_filter_edit(app, AppState::filter_backspace);
            }
            KeyCode::Delete => {
                app.history_index = None;
                apply_filter_edit(app, AppState::filter_delete);
            }
            KeyCode::Up => {
                if !app.filter_history.is_empty() {
                    match app.history_index {
                        None => {
                            app.stashed_input = app.filter_text.clone();
                            app.history_index = Some(app.filter_history.len() - 1);
                        }
                        Some(idx) if idx > 0 => {
                            app.history_index = Some(idx - 1);
                        }
                        _ => {}
                    }
                    if let Some(idx) = app.history_index {
                        app.filter_text = app.filter_history[idx].clone();
                        app.filter_move_to_end();
                        app.update_filter();
                    }
                }
            }
            KeyCode::Down => {
                if let Some(idx) = app.history_index {
                    if idx < app.filter_history.len() - 1 {
                        app.history_index = Some(idx + 1);
                        app.filter_text = app.filter_history[idx + 1].clone();
                    } else {
                        app.history_index = None;
                        app.filter_text = app.stashed_input.clone();
                    }
                    app.filter_move_to_end();
                    app.update_filter();
                }
            }
            KeyCode::Left => app.filter_move_cursor_left(),
            KeyCode::Right => app.filter_move_cursor_right(),
            KeyCode::Home => app.filter_move_to_start(),
            KeyCode::End => app.filter_move_to_end(),
            _ => {}
        },
    }
}

#[cfg(target_arch = "wasm32")]
const EXCLUDED_FIELDS: &[&str] = &[
    "id",
    "abstract",
    "description",
    "name",
    "__filename",
    "//",
    "//2",
    "rows",
];

#[cfg(target_arch = "wasm32")]
fn pane_at(app: &AppState, column: u16, row: u16) -> Option<FocusPane> {
    if let Some(area) = app.filter_area
        && area.contains((column, row).into())
    {
        return Some(FocusPane::Filter);
    }
    if let Some(area) = app.list_area
        && area.contains((column, row).into())
    {
        return Some(FocusPane::List);
    }
    if let Some(area) = app.details_area
        && area.contains((column, row).into())
    {
        return Some(FocusPane::Details);
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn app_terminal_size(app: &AppState) -> Option<(u16, u16)> {
    let mut max_right = 0u16;
    let mut max_bottom = 0u16;

    for area in [app.list_area, app.details_area, app.filter_area] {
        if let Some(area) = area {
            max_right = max_right.max(area.x.saturating_add(area.width));
            max_bottom = max_bottom.max(area.y.saturating_add(area.height));
        }
    }

    if max_right == 0 || max_bottom == 0 {
        None
    } else {
        Some((max_right, max_bottom))
    }
}

#[cfg(target_arch = "wasm32")]
fn mouse_to_cell_position(app: &AppState, mouse: &MouseEvent) -> Option<(u16, u16)> {
    let fallback_size = app_terminal_size(app);
    let window = ratzilla::web_sys::window()?;
    let document = window.document()?;
    let grid: ratzilla::web_sys::HtmlElement = document
        .get_element_by_id("grid_ratzilla_grid")?
        .dyn_into()
        .ok()?;
    let rect = grid.get_bounding_client_rect();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }

    let local_x = mouse.x as f64 - rect.left();
    let local_y = mouse.y as f64 - rect.top();
    if local_x < 0.0 || local_y < 0.0 || local_x >= rect.width() || local_y >= rect.height() {
        return None;
    }

    let mut cell_width = 0.0;
    let mut cell_height = 0.0;
    if let Some(first_row) = grid.first_element_child()
        && let Ok(first_row) = first_row.dyn_into::<ratzilla::web_sys::HtmlElement>()
    {
        let row_rect = first_row.get_bounding_client_rect();
        cell_height = row_rect.height();
        if let Some(first_cell) = first_row.first_element_child()
            && let Ok(first_cell) = first_cell.dyn_into::<ratzilla::web_sys::HtmlElement>()
        {
            let cell_rect = first_cell.get_bounding_client_rect();
            cell_width = cell_rect.width();
        }
    }

    if cell_width <= 0.0 || cell_height <= 0.0 {
        if let Some((cols, rows)) = fallback_size {
            let col = ((local_x / rect.width()) * cols as f64).floor() as u16;
            let row = ((local_y / rect.height()) * rows as f64).floor() as u16;
            return Some((
                col.min(cols.saturating_sub(1)),
                row.min(rows.saturating_sub(1)),
            ));
        }
        return None;
    }

    let mut col = (local_x / cell_width).floor() as u16;
    let mut row = (local_y / cell_height).floor() as u16;
    if let Some((cols, rows)) = fallback_size {
        col = col.min(cols.saturating_sub(1));
        row = row.min(rows.saturating_sub(1));
    }

    Some((col, row))
}

#[cfg(target_arch = "wasm32")]
fn handle_mouse_event(app: &mut AppState, mouse: MouseEvent) -> bool {
    let Some((column, row)) = mouse_to_cell_position(app, &mouse) else {
        if matches!(mouse.event, MouseEventKind::Moved) && app.hovered_span_id.is_some() {
            app.hovered_span_id = None;
            return true;
        }
        return false;
    };
    let hovered_pane = pane_at(app, column, row);
    let mut is_valid_target = false;
    let mut new_hover_id = None;
    let mut target_path = String::new();
    let mut target_id = None;

    if let Some(span) = ui::hit_test_details(app, column, row)
        && let Some(path) = &span.key_context
    {
        let path_str = path.as_ref();
        let first_part = path_str.split('.').next().unwrap_or("");
        if !EXCLUDED_FIELDS.contains(&first_part) && span.span_id.is_some() {
            is_valid_target = true;
            new_hover_id = span.span_id;
            target_path = path_str.to_string();
            target_id = span.span_id;
        }
    }

    let mut transitioned = false;

    if matches!(mouse.event, MouseEventKind::Moved) && app.hovered_span_id != new_hover_id {
        app.hovered_span_id = new_hover_id;
        transitioned = true;
    }

    if mouse.event == MouseEventKind::Pressed && mouse.button == MouseButton::Left {
        if let Some(pane) = hovered_pane {
            let previous_focus = app.focused_pane;
            let previous_mode = app.input_mode;
            app.focus_pane(pane);
            if app.focused_pane != previous_focus || app.input_mode != previous_mode {
                transitioned = true;
            }
        }

        if hovered_pane == Some(FocusPane::List)
            && let Some(content_area) = app.list_content_area
            && content_area.contains((column, row).into())
            && !app.filtered_indices.is_empty()
        {
            let list_row = row.saturating_sub(content_area.y) as usize;
            if list_row < content_area.height as usize {
                let top_index = app.list_state.offset();
                let clicked = (top_index + list_row).min(app.filtered_indices.len() - 1);
                if app.list_state.selected() != Some(clicked) {
                    app.list_state.select(Some(clicked));
                    app.refresh_details();
                    transitioned = true;
                }
            }
        }

        if hovered_pane == Some(FocusPane::Filter)
            && let Some(input_area) = app.filter_input_area
            && input_area.contains((column, row).into())
        {
            let horizontal_scroll =
                ui::filter_horizontal_scroll(&app.filter_text, app.filter_cursor, input_area.width);
            let local_x = column.saturating_sub(input_area.x);
            let target_column = horizontal_scroll + local_x;
            let new_cursor = ui::filter_cursor_for_column(&app.filter_text, target_column);
            if new_cursor != app.filter_cursor {
                app.filter_cursor = new_cursor;
                transitioned = true;
            }
        }

        if is_valid_target {
            let mut full_value = String::new();
            if let Some(id) = target_id {
                for line in &app.details_annotated {
                    for span in line {
                        if span.span_id == Some(id) {
                            full_value.push_str(&span.span.content);
                        }
                    }
                }
            }

            let clean_val = full_value.trim();
            let mut unescaped_val = clean_val.to_string();
            if clean_val.starts_with('"') && clean_val.ends_with('"') && clean_val.len() >= 2 {
                if let Ok(s) = serde_json::from_str::<String>(clean_val) {
                    unescaped_val = s;
                } else {
                    unescaped_val = clean_val[1..clean_val.len() - 1].to_string();
                }
            }

            let escaped = unescaped_val.replace('\\', "\\\\").replace('\'', "\\'");
            let final_val = format!("'{}'", escaped);

            if mouse.ctrl {
                app.filter_text = format!("i:{}", final_val);
                app.filter_cursor = app.filter_text.chars().count();
                app.update_filter();
                app.focus_pane(FocusPane::Details);
            } else {
                let filter_addition = format!("{}:{}", target_path, final_val);
                let current = app.filter_text.trim();
                if current.is_empty() {
                    app.filter_text = filter_addition;
                } else {
                    app.filter_text = format!("{} {}", current, filter_addition);
                }
                app.filter_cursor = app.filter_text.chars().count();
                app.update_filter();
                app.focus_pane(FocusPane::Filter);
            }

            transitioned = true;
        }
    }

    transitioned
}
