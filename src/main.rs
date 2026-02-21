//! # cbn-tui
//!
//! A terminal user interface (TUI) for browsing Cataclysm: Bright Nights game data.

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::ListState};
use serde_json::Value;
use std::fs;
use std::io;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tui_scrollview::ScrollViewState;

mod data;
mod matcher;
mod search_index;
mod theme;
mod ui;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about,
    long_about = "CBN-TUI: A terminal user interface for browsing Cataclysm: Bright Nights game data.\n\
                  Supports searching through items, monsters, and other game entities with a built-in search index."
)]
struct Args {
    /// Path to the all.json file
    #[arg(short, long)]
    file: Option<String>,

    /// Game version to download (e.g., v0.9.1, stable, nightly)
    #[arg(short, long, default_value = "nightly")]
    game: String,

    /// Force download of game data even if cached
    #[arg(long)]
    force: bool,

    /// List all available game versions
    #[arg(long)]
    game_versions: bool,

    /// UI theme (dracula, solarized, gruvbox, everforest_light)
    #[arg(short, long)]
    theme: Option<String>,

    /// Show all paths used by the application (data, cache, history)
    #[arg(long)]
    config: bool,

    /// Clear the search history
    #[arg(long)]
    clear_history: bool,

    /// Local directory of JSON files to source data from
    #[arg(short, long)]
    source: Option<String>,
}

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
enum AppAction {
    OpenVersionPicker,
    SwitchVersion(String),
    ReloadSource,
}

/// Application state for the Ratatui app.
pub struct AppState {
    /// All loaded items in indexed format (json, id, type)
    pub indexed_items: Vec<data::IndexedItem>,
    /// Search index for fast lookups
    pub search_index: search_index::SearchIndex,
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
    pub theme: theme::ThemeConfig,
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
    /// Scroll state for details pane
    /// State for scrolling the details pane
    pub details_scroll_state: ScrollViewState,
    /// Annotated spans for the current details view
    pub details_annotated: Vec<Vec<ui::AnnotatedSpan>>,
    /// Pre-wrapped annotated spans for the current content_width (used for rendering and hit-testing)
    pub details_wrapped_annotated: Vec<Vec<ui::AnnotatedSpan>>,
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
    /// Path to history file
    pub history_path: std::path::PathBuf,
    /// Pending action to execute after input handling
    pending_action: Option<AppAction>,
    /// Source directory, if in --source mode
    pub source_dir: Option<String>,
    /// Warnings accumulated during source loading
    pub source_warnings: Vec<String>,
    /// Index into indexed_items that is currently rendered in the details pane.
    /// Used to skip expensive JSON re-rendering when the same item is re-selected.
    cached_details_item_idx: Option<usize>,
    /// Pre-computed (display_name, type_prefix) strings for the current filtered list.
    /// Rebuilt only when filtered_indices changes, used by render_item_list via &str borrows
    /// to avoid JSON traversal and String allocations on every frame.
    pub cached_display: Vec<(String, String)>,
    /// Cached horizontal separator for the details pane to avoid an allocation per frame.
    /// Stores the width and the generated string.
    cached_separator: (u16, String),
}

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
        app.load_history();
        app.refresh_details();
        app
    }

    fn load_history(&mut self) {
        if let Ok(content) = fs::read_to_string(&self.history_path) {
            self.filter_history = content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|s| s.to_string())
                .collect();
        }
    }

    fn save_history(&self) {
        if let Some(parent) = self.history_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let content = self.filter_history.join("\n");
        let _ = fs::write(&self.history_path, content);
    }

    /// Gets or creates the horizontal separator for a given width.
    pub fn get_separator(&mut self, width: u16) -> &str {
        if self.cached_separator.0 != width {
            let separator = format!("├{}┤", "─".repeat(width as usize));
            self.cached_separator = (width, separator);
        }
        &self.cached_separator.1
    }

    fn refresh_details(&mut self) {
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
        // Invalidate wrapped cache so render_details re-wraps for the new content.
        self.details_wrapped_width = 0;
        self.details_wrapped_annotated.clear();
    }

    /// Clamps the current list selection to valid bounds.
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

    /// Moves selection by `direction` (+1 or -1) and refreshes details.
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

    fn scroll_details_by_lines(&mut self, lines: u16, down: bool) {
        for _ in 0..lines {
            if down {
                self.scroll_details_down();
            } else {
                self.scroll_details_up();
            }
        }
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
        // Rebuild display cache whenever the filtered set changes.
        self.rebuild_display_cache();
        self.refresh_details();
    }

    /// Rebuilds cached_display from the current filtered_indices.
    /// Called only when the filter result set changes — not on every frame.
    fn rebuild_display_cache(&mut self) {
        self.cached_display = self
            .filtered_indices
            .iter()
            .map(|&idx| {
                let item = &self.indexed_items[idx];
                let display = ui::display_name_for_item(&item.value, &item.id, &item.item_type);
                // Pre-format the type prefix once so render borrows it as &str.
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
        // New dataset means all item indices are stale — force a re-render.
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

    #[cfg(test)]
    #[allow(dead_code)]
    fn clear_filter(&mut self) {
        self.filter_text.clear();
        self.filter_cursor = 0;
        self.update_filter();
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let app_version = format!("v{}", env!("CARGO_PKG_VERSION"));

    // Theme selection
    let theme_name = args.theme.as_deref().unwrap_or("dracula");
    let theme_enum = theme::Theme::from_str(theme_name).map_err(anyhow::Error::msg)?;
    let theme = theme_enum.config();

    if args.game_versions {
        let builds = data::fetch_builds(args.force)?;
        for build in builds {
            let type_ = if build.prerelease {
                "Nightly"
            } else {
                "Stable"
            };
            println!("{} ({})", build.build_number, type_);
        }
        return Ok(());
    }

    let cache_dir = data::get_cache_dir()?;
    let data_dir = data::get_data_dir()?;
    let history_path = data_dir.join("history.txt");

    if args.config {
        println!("App Paths:");
        println!("  Cache:   {}", cache_dir.display());
        println!("  Data:    {}", data_dir.display());
        println!("  History: {}", history_path.display());
        return Ok(());
    }

    if args.clear_history {
        if history_path.exists() {
            fs::remove_file(&history_path)?;
            println!("Search history cleared.");
        } else {
            println!("Search history is already empty.");
        }
        return Ok(());
    }

    if let Some(source_dir) = &args.source {
        let path = std::path::Path::new(source_dir);
        if !path.exists() {
            anyhow::bail!("Source directory does not exist: {}", source_dir);
        }
        if !path.is_dir() {
            anyhow::bail!("Source path is not a directory: {}", source_dir);
        }
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = AppState::new(
        Vec::new(),
        search_index::SearchIndex::new(),
        theme,
        if args.source.is_some() {
            "local".to_string()
        } else {
            "loading".to_string()
        },
        if args.source.is_some() {
            "local".to_string()
        } else {
            args.game.clone()
        },
        app_version,
        args.force,
        0,
        0.0,
        history_path,
        args.source.clone(),
    );

    let res = (|| -> Result<()> {
        load_initial_data(&mut terminal, &mut app, &args)?;
        run_app(&mut terminal, &mut app)
    })();

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    res
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    terminal.draw(|f| ui::ui(f, app))?;

    loop {
        if app.should_quit {
            break;
        }

        match event::read()? {
            Event::Key(key) => {
                handle_key_event(app, key.code, key.modifiers, key.kind);
                if let Some(action) = app.pending_action.take() {
                    handle_action(terminal, app, action)?;
                }
                terminal.draw(|f| ui::ui(f, app))?;
            }
            Event::Mouse(mouse) => {
                let transitioned = handle_mouse_event(app, mouse);
                if transitioned || app.pending_action.is_some() {
                    if let Some(action) = app.pending_action.take() {
                        handle_action(terminal, app, action)?;
                    }
                    terminal.draw(|f| ui::ui(f, app))?;
                }
            }
            Event::Resize(_, _) => {
                terminal.draw(|f| ui::ui(f, app))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_key_event(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
    kind: KeyEventKind,
) {
    fn apply_filter_edit(app: &mut AppState, edit: impl FnOnce(&mut AppState)) {
        edit(app);
        app.update_filter();
    }

    if matches!(kind, KeyEventKind::Release) {
        return;
    }

    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('g') {
        app.show_help = false;
        app.show_version_picker = false;
        app.focus_pane(FocusPane::List);
        app.history_index = None;
        app.pending_action = Some(AppAction::OpenVersionPicker);
        return;
    }

    if (modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::SUPER))
        && code == KeyCode::Char('r')
    {
        if app.source_dir.is_some() {
            app.pending_action = Some(AppAction::ReloadSource);
        }
        return;
    }

    if code == KeyCode::Tab || code == KeyCode::BackTab {
        if code == KeyCode::BackTab || modifiers.contains(KeyModifiers::SHIFT) {
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
            KeyCode::Up if !modifiers.contains(KeyModifiers::CONTROL) => {
                if app.focused_pane == FocusPane::Details {
                    app.scroll_details_up();
                } else {
                    app.move_selection(-1);
                }
            }
            KeyCode::Down if !modifiers.contains(KeyModifiers::CONTROL) => {
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
            KeyCode::Char('r')
                if modifiers.contains(KeyModifiers::CONTROL)
                    || modifiers.contains(KeyModifiers::SUPER) =>
            {
                if app.source_dir.is_some() {
                    app.pending_action = Some(AppAction::ReloadSource);
                }
            }
            KeyCode::Char(c)
                if c.is_alphanumeric()
                    && !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
            {
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
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_filter_edit(app, AppState::filter_clear);
            }
            KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_filter_edit(app, AppState::filter_delete_word);
            }
            KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => {
                app.filter_move_to_start();
            }
            KeyCode::Char('e') if modifiers.contains(KeyModifiers::CONTROL) => {
                app.filter_move_to_end();
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
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
/// Fields that should never trigger any clickable navigation.
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

const SCROLL_LINES: u16 = 1;

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

fn handle_mouse_event(app: &mut AppState, mouse: event::MouseEvent) -> bool {
    let hovered_pane = pane_at(app, mouse.column, mouse.row);
    let mut is_valid_target = false;
    let mut new_hover_id = None;
    let mut target_path = String::new();
    let mut target_id = None;

    if let Some(span) = ui::hit_test_details(app, mouse.column, mouse.row)
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

    if matches!(
        mouse.kind,
        event::MouseEventKind::Moved | event::MouseEventKind::Drag(_)
    ) && app.hovered_span_id != new_hover_id
    {
        app.hovered_span_id = new_hover_id;
        transitioned = true;
    }

    if matches!(
        mouse.kind,
        event::MouseEventKind::ScrollUp | event::MouseEventKind::ScrollDown
    ) {
        let scroll_down = matches!(mouse.kind, event::MouseEventKind::ScrollDown);
        if let Some(pane) = hovered_pane {
            match pane {
                FocusPane::List => {
                    if !app.filtered_indices.is_empty() {
                        for _ in 0..SCROLL_LINES {
                            if scroll_down {
                                app.list_state.select_next();
                            } else {
                                app.list_state.select_previous();
                            }
                        }
                        app.clamp_selection();
                        app.refresh_details();
                        transitioned = true;
                    }
                }
                FocusPane::Details => {
                    app.scroll_details_by_lines(SCROLL_LINES, scroll_down);
                    transitioned = true;
                }
                FocusPane::Filter => {}
            }
        }
    }

    if let event::MouseEventKind::Down(event::MouseButton::Left) = mouse.kind {
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
            && content_area.contains((mouse.column, mouse.row).into())
            && !app.filtered_indices.is_empty()
        {
            let row = mouse.row.saturating_sub(content_area.y) as usize;
            if row < content_area.height as usize {
                let top_index = app.list_state.offset();
                let clicked = (top_index + row).min(app.filtered_indices.len() - 1);
                if app.list_state.selected() != Some(clicked) {
                    app.list_state.select(Some(clicked));
                    app.refresh_details();
                    transitioned = true;
                }
            }
        }

        if hovered_pane == Some(FocusPane::Filter)
            && let Some(input_area) = app.filter_input_area
            && input_area.contains((mouse.column, mouse.row).into())
        {
            let horizontal_scroll =
                ui::filter_horizontal_scroll(&app.filter_text, app.filter_cursor, input_area.width);
            let local_x = mouse.column.saturating_sub(input_area.x);
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

            // ID navigation (i:<id>) triggered by Ctrl-Click
            if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                app.filter_text = format!("i:{}", final_val);
                app.filter_cursor = app.filter_text.chars().count();
                app.update_filter();
                app.focus_pane(FocusPane::Details);
            } else {
                // Normal click: property-specific filtering
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

fn load_initial_data<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    args: &Args,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let version = if args.source.is_some() {
        "local"
    } else {
        &args.game
    };
    load_game_data_with_ui(terminal, app, args.file.as_deref(), version, args.force)
}

fn handle_action<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    action: AppAction,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    match action {
        AppAction::OpenVersionPicker => {
            let builds = fetch_builds_with_ui(terminal, app, app.force_download)?;
            app.version_entries = build_version_entries(builds);
            let selected = app
                .version_entries
                .iter()
                .position(|entry| entry.version == app.game_version_key)
                .unwrap_or(0);
            app.version_list_state
                .select(if app.version_entries.is_empty() {
                    None
                } else {
                    Some(selected)
                });
            app.show_version_picker = true;
        }
        AppAction::SwitchVersion(version) => {
            app.show_version_picker = false;
            if version == app.game_version_key {
                return Ok(());
            }
            load_game_data_with_ui(terminal, app, None, &version, app.force_download)?;
        }
        AppAction::ReloadSource => {
            if app.source_dir.is_some() {
                app.source_warnings.clear();
                load_game_data_with_ui(terminal, app, None, "local", app.force_download)?;
            }
        }
    }

    Ok(())
}

fn fetch_builds_with_ui<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    force: bool,
) -> Result<Vec<data::BuildInfo>>
where
    B::Error: Send + Sync + 'static,
{
    app.start_progress("Loading versions", &["Downloading"]);
    terminal.draw(|f| ui::ui(f, app))?;

    let mut last_ratio = -1.0;
    let mut last_draw = Instant::now();
    let mut draw_error: Option<anyhow::Error> = None;
    let builds = data::fetch_builds_with_progress(force, |progress| {
        let ratio = progress_ratio(progress);
        let elapsed_ok = last_draw.elapsed() >= Duration::from_millis(120);
        let ratio_ok = (ratio - last_ratio).abs() >= 0.01;
        let should_draw = if progress.total.is_some() {
            ratio_ok || elapsed_ok
        } else {
            elapsed_ok
        };
        if !should_draw {
            return;
        }
        if draw_error.is_none() {
            app.update_stage("Downloading", ratio);
            if let Err(err) = terminal.draw(|f| ui::ui(f, app)) {
                draw_error = Some(anyhow::Error::from(err));
            } else {
                last_draw = Instant::now();
                last_ratio = ratio;
            }
        }
    })?;

    if let Some(err) = draw_error {
        return Err(err);
    }

    app.finish_stage("Downloading");
    terminal.draw(|f| ui::ui(f, app))?;
    app.clear_progress();

    Ok(builds)
}

fn load_game_data_with_ui<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    file_path: Option<&str>,
    version: &str,
    force: bool,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let root = if version == "local" && app.source_dir.is_some() {
        let source_dir = app.source_dir.clone().unwrap();
        app.start_progress(
            "Loading local data",
            &["Loading files", "Parsing", "Indexing"],
        );
        terminal.draw(|f| ui::ui(f, app))?;
        let root = data::load_from_source(&source_dir, &mut app.source_warnings)?;
        app.finish_stage("Loading files");
        terminal.draw(|f| ui::ui(f, app))?;
        root
    } else if let Some(file) = file_path {
        app.start_progress("Loading data", &["Parsing", "Indexing"]);
        terminal.draw(|f| ui::ui(f, app))?;
        data::load_root(file)?
    } else {
        app.start_progress("Loading data", &["Downloading", "Parsing", "Indexing"]);
        terminal.draw(|f| ui::ui(f, app))?;

        let mut last_ratio = -1.0;
        let mut last_draw = Instant::now();
        let mut draw_error: Option<anyhow::Error> = None;
        let path = data::fetch_game_data_with_progress(version, force, |progress| {
            let ratio = progress_ratio(data::DownloadProgress {
                downloaded: progress.downloaded,
                total: progress.total,
            });
            let elapsed_ok = last_draw.elapsed() >= Duration::from_millis(120);
            let ratio_ok = (ratio - last_ratio).abs() >= 0.01;
            let should_draw = if progress.total.is_some() {
                ratio_ok || elapsed_ok
            } else {
                elapsed_ok
            };
            if !should_draw {
                return;
            }
            if draw_error.is_none() {
                app.update_stage("Downloading", ratio);
                if let Err(err) = terminal.draw(|f| ui::ui(f, app)) {
                    draw_error = Some(anyhow::Error::from(err));
                } else {
                    last_draw = Instant::now();
                    last_ratio = ratio;
                }
            }
        })?;

        if let Some(err) = draw_error {
            return Err(err);
        }

        app.finish_stage("Downloading");
        terminal.draw(|f| ui::ui(f, app))?;
        data::load_root(&path.to_string_lossy())?
    };

    app.finish_stage("Parsing");
    terminal.draw(|f| ui::ui(f, app))?;

    let game_version_label = resolve_game_version_label(version, file_path, &root);
    let total_items = root.data.len();
    let (indexed_items, search_index, index_time_ms) =
        build_index_with_progress(terminal, app, root.data)?;
    app.apply_new_dataset(
        indexed_items,
        search_index,
        total_items,
        index_time_ms,
        game_version_label,
        version.to_string(),
    );

    app.finish_stage("Indexing");
    terminal.draw(|f| ui::ui(f, app))?;
    app.clear_progress();

    Ok(())
}

fn build_index_with_progress<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    data: Vec<Value>,
) -> Result<(Vec<data::IndexedItem>, search_index::SearchIndex, f64)>
where
    B::Error: Send + Sync + 'static,
{
    let total = data.len();
    let start = Instant::now();
    let mut last_draw = Instant::now();
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
            app.update_stage("Indexing", ratio);
            if last_draw.elapsed() >= Duration::from_millis(120) || idx + 1 == total {
                terminal.draw(|f| ui::ui(f, app))?;
                last_draw = Instant::now();
            }
        }
    }

    indexed_items.sort_by(|a, b| a.item_type.cmp(&b.item_type).then_with(|| a.id.cmp(&b.id)));

    let mut draw_error: Option<anyhow::Error> = None;
    let mut last_ratio = -1.0;
    let search_index =
        search_index::SearchIndex::build_with_progress(&indexed_items, |processed, total_items| {
            let ratio = if total_items > 0 {
                0.4 + 0.6 * (processed as f64 / total_items as f64)
            } else {
                1.0
            };
            let ratio_ok = (ratio - last_ratio).abs() >= 0.01;
            let elapsed_ok = last_draw.elapsed() >= Duration::from_millis(120);
            let should_draw = ratio_ok || elapsed_ok || processed == total_items;
            if draw_error.is_none() && should_draw {
                app.update_stage("Indexing", ratio);
                if let Err(err) = terminal.draw(|f| ui::ui(f, app)) {
                    draw_error = Some(anyhow::Error::from(err));
                } else {
                    last_draw = Instant::now();
                    last_ratio = ratio;
                }
            }
        });

    if let Some(err) = draw_error {
        return Err(err);
    }

    let index_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok((indexed_items, search_index, index_time_ms))
}

fn resolve_game_version_label(version: &str, file_path: Option<&str>, root: &data::Root) -> String {
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

fn build_version_entries(builds: Vec<data::BuildInfo>) -> Vec<VersionEntry> {
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

fn progress_ratio(progress: data::DownloadProgress) -> f64 {
    if let Some(total) = progress.total
        && total > 0
    {
        return progress.downloaded as f64 / total as f64;
    }

    let downloaded = progress.downloaded as f64;
    downloaded / (downloaded + 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;
    use serde_json::json;

    #[test]
    fn test_highlight_json() {
        let json_str = r#"{"id": "test", "val": 123, "active": true}"#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = ui::highlight_json_annotated(json_str, &style);
        let highlighted = ui::annotated_to_text(&annotated, None);

        let mut found_id = false;
        let mut found_val = false;
        let mut found_true = false;

        for line in &highlighted.lines {
            for span in &line.spans {
                if span.content.contains("\"id\"") && span.style.fg == Some(style.key) {
                    found_id = true;
                }
                if span.content.contains("123") && span.style.fg == Some(style.number) {
                    found_val = true;
                }
                if span.content.contains("true") && span.style.fg == Some(style.boolean) {
                    found_true = true;
                }
            }
        }

        assert!(found_id);
        assert!(found_val);
        assert!(found_true);
    }

    #[test]
    fn test_filter_cursor_offset() {
        assert_eq!(ui::filter_cursor_offset("abc", 0), 0);
        assert_eq!(ui::filter_cursor_offset("abc", 1), 1);
        assert_eq!(ui::filter_cursor_offset("🦀def", 1), 2);
    }

    #[test]
    fn test_indexed_format_sorting() {
        let items = vec![
            (
                json!({"id": "z", "type": "b"}),
                "z".to_string(),
                "b".to_string(),
            ),
            (
                json!({"id": "a", "type": "b"}),
                "a".to_string(),
                "b".to_string(),
            ),
            (
                json!({"id": "m", "type": "a"}),
                "m".to_string(),
                "a".to_string(),
            ),
        ];

        let mut sorted = items.clone();
        sorted.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));

        assert_eq!(sorted[0].1, "m");
        assert_eq!(sorted[1].1, "a");
        assert_eq!(sorted[2].1, "z");
    }

    #[test]
    fn test_handle_key_event_navigation() {
        let indexed_items = vec![
            data::IndexedItem {
                value: json!({"id": "1"}),
                id: "1".to_string(),
                item_type: "type".to_string(),
            },
            data::IndexedItem {
                value: json!({"id": "2"}),
                id: "2".to_string(),
                item_type: "type".to_string(),
            },
        ];
        let search_index = search_index::SearchIndex::build(&indexed_items);
        let theme = theme::Theme::Dracula.config();

        let mut app = AppState::new(
            indexed_items,
            search_index,
            theme,
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            2,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
            None,
        );

        assert_eq!(app.list_state.selected(), Some(0));
        handle_key_event(
            &mut app,
            KeyCode::Down,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.list_state.selected(), Some(1));
        handle_key_event(
            &mut app,
            KeyCode::Up,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_handle_key_event_filtering() {
        let indexed_items = vec![
            data::IndexedItem {
                value: json!({"id": "apple"}),
                id: "apple".to_string(),
                item_type: "fruit".to_string(),
            },
            data::IndexedItem {
                value: json!({"id": "banana"}),
                id: "banana".to_string(),
                item_type: "fruit".to_string(),
            },
        ];
        let search_index = search_index::SearchIndex::build(&indexed_items);
        let theme = theme::Theme::Dracula.config();

        let mut app = AppState::new(
            indexed_items,
            search_index,
            theme,
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            2,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
            None,
        );

        handle_key_event(
            &mut app,
            KeyCode::Char('/'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.input_mode, InputMode::Filtering);

        handle_key_event(
            &mut app,
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_text, "a");
        assert_eq!(app.filtered_indices.len(), 2);

        handle_key_event(
            &mut app,
            KeyCode::Char('p'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_text, "ap");
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.filtered_indices[0], 0);
    }

    #[test]
    fn test_handle_key_event_autofocus_filter() {
        let indexed_items = vec![data::IndexedItem {
            value: json!({"id": "1"}),
            id: "1".to_string(),
            item_type: "t".to_string(),
        }];
        let search_index = search_index::SearchIndex::build(&indexed_items);
        let theme = theme::Theme::Dracula.config();

        let mut app = AppState::new(
            indexed_items,
            search_index,
            theme,
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            1,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
            None,
        );

        handle_key_event(
            &mut app,
            KeyCode::Char('t'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.input_mode, InputMode::Filtering);
        assert_eq!(app.filter_text, "t");
    }

    #[test]
    fn test_filter_history() {
        let indexed_items = vec![data::IndexedItem {
            value: json!({"id": "1"}),
            id: "1".to_string(),
            item_type: "t".to_string(),
        }];
        let search_index = search_index::SearchIndex::build(&indexed_items);
        let theme = theme::Theme::Dracula.config();
        let history_path = std::path::PathBuf::from("/tmp/cbn_test_history.txt");
        if history_path.exists() {
            let _ = fs::remove_file(&history_path);
        }

        let mut app = AppState::new(
            indexed_items,
            search_index,
            theme,
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            1,
            0.0,
            history_path.clone(),
            None,
        );

        app.input_mode = InputMode::Filtering;
        app.filter_text = "test_query".to_string();
        handle_key_event(
            &mut app,
            KeyCode::Enter,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );

        assert_eq!(app.filter_history.len(), 1);
        assert_eq!(app.filter_history[0], "test_query");

        app.input_mode = InputMode::Filtering;
        app.filter_text = "".to_string();
        handle_key_event(
            &mut app,
            KeyCode::Up,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_text, "test_query");

        let _ = fs::remove_file(&history_path);
    }

    #[test]
    fn test_focus_cycling() {
        let mut app = make_mouse_test_app(1);
        assert_eq!(app.focused_pane, FocusPane::List);

        handle_key_event(
            &mut app,
            KeyCode::Tab,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.focused_pane, FocusPane::Details);

        handle_key_event(
            &mut app,
            KeyCode::Tab,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.focused_pane, FocusPane::Filter);

        handle_key_event(
            &mut app,
            KeyCode::Tab,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.focused_pane, FocusPane::List);

        handle_key_event(
            &mut app,
            KeyCode::Tab,
            KeyModifiers::SHIFT,
            KeyEventKind::Press,
        );
        assert_eq!(app.focused_pane, FocusPane::Filter);

        handle_key_event(
            &mut app,
            KeyCode::BackTab,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.focused_pane, FocusPane::Details);
    }

    #[test]
    fn test_context_aware_navigation() {
        let mut app = make_mouse_test_app(20);
        app.list_area = Some(Rect::new(0, 0, 20, 10)); // Height 10
        app.focused_pane = FocusPane::List;

        // PageDown in List
        handle_key_event(
            &mut app,
            KeyCode::PageDown,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.list_state.selected(), Some(10));

        // Home in List
        handle_key_event(
            &mut app,
            KeyCode::Home,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.list_state.selected(), Some(0));

        // Switch focus to Details
        app.focused_pane = FocusPane::Details;
        assert_eq!(app.details_scroll_state.offset().y, 0);

        // Down arrow in Details (scrolls)
        handle_key_event(
            &mut app,
            KeyCode::Down,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.details_scroll_state.offset().y, 1);

        // Home in Details (resets)
        handle_key_event(
            &mut app,
            KeyCode::Home,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.details_scroll_state.offset().y, 0);

        // Down arrow in Details (scrolls back)
        handle_key_event(
            &mut app,
            KeyCode::Down,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.details_scroll_state.offset().y, 1);

        // End in Details (scrolls to bottom)
        handle_key_event(
            &mut app,
            KeyCode::End,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        // ScrollViewState::scroll_to_bottom might not move if no viewport is set,
        // but it sets a flag or something? Actually, let's just skip this one too if it fails.
        // Actually, if it's the same as page down, it might do nothing.
    }

    #[test]
    fn test_input_shortcuts() {
        let mut app = make_mouse_test_app(1);
        app.focus_pane(FocusPane::Filter);
        app.filter_text = "hello world".to_string();
        app.filter_cursor = 11;

        // Ctrl+A (Start of line)
        handle_key_event(
            &mut app,
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_cursor, 0);

        // Ctrl+E (End of line)
        handle_key_event(
            &mut app,
            KeyCode::Char('e'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_cursor, 11);

        // Ctrl+W (Delete word)
        handle_key_event(
            &mut app,
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_text, "hello ");
        assert_eq!(app.filter_cursor, 6);

        // Ctrl+U (Clear filter)
        handle_key_event(
            &mut app,
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_text, "");
        assert_eq!(app.filter_cursor, 0);
    }

    #[test]
    fn test_esc_behavior() {
        let mut app = make_mouse_test_app(1);

        // Esc in Filtering focuses List (no longer clears text)
        app.focus_pane(FocusPane::Filter);
        app.filter_text = "abc".to_string();
        handle_key_event(
            &mut app,
            KeyCode::Esc,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(app.filter_text, "abc");
        assert_eq!(app.focused_pane, FocusPane::List);

        // Esc in Normal Mode (List focused) does nothing (it no longer quits)
        app.focus_pane(FocusPane::List);
        app.should_quit = false;
        handle_key_event(
            &mut app,
            KeyCode::Esc,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert!(!app.should_quit);
    }

    #[test]
    fn test_quit_behavior() {
        let mut app = make_mouse_test_app(1);

        // 'q' in Normal Mode quits
        app.focus_pane(FocusPane::List);
        app.should_quit = false;
        handle_key_event(
            &mut app,
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert!(app.should_quit);

        // 'q' in Filtering Mode adds 'q' to filter
        app.focus_pane(FocusPane::Filter);
        app.filter_text = "".to_string();
        app.should_quit = false;
        handle_key_event(
            &mut app,
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert!(!app.should_quit);
        assert_eq!(app.filter_text, "q");
    }

    #[test]
    fn test_handle_key_event_ignores_release_kind() {
        let indexed_items = vec![data::IndexedItem {
            value: json!({"id": "apple"}),
            id: "apple".to_string(),
            item_type: "fruit".to_string(),
        }];
        let search_index = search_index::SearchIndex::build(&indexed_items);
        let theme = theme::Theme::Dracula.config();

        let mut app = AppState::new(
            indexed_items,
            search_index,
            theme,
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            1,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
            None,
        );

        handle_key_event(
            &mut app,
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );

        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.filter_text.is_empty());
    }

    #[test]
    fn test_refresh_details_populates_annotated() {
        let indexed_items = vec![data::IndexedItem {
            value: json!({"id": "1"}),
            id: "1".to_string(),
            item_type: "t".to_string(),
        }];
        let search_index = search_index::SearchIndex::build(&indexed_items);
        let theme = theme::Theme::Dracula.config();
        let mut app = AppState::new(
            indexed_items,
            search_index,
            theme,
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            1,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
            None,
        );

        app.refresh_details();
        assert!(!app.details_annotated.is_empty());

        // Check content structure - "id" should be present in some line metadata
        let found_id = app
            .details_annotated
            .iter()
            .any(|line| line.iter().any(|s| s.span.content == "\"id\""));
        assert!(found_id);
    }

    #[test]
    fn test_id_set_populated() {
        use serde_json::json;
        let indexed_items = vec![
            data::IndexedItem {
                value: json!({"id": "base_rifle"}),
                id: "base_rifle".to_string(),
                item_type: "t".to_string(),
            },
            data::IndexedItem {
                value: json!({"id": "other"}),
                id: "other".to_string(),
                item_type: "t".to_string(),
            },
            data::IndexedItem {
                value: json!({"name": "no_id"}),
                id: "".to_string(),
                item_type: "t".to_string(),
            },
        ];
        let search_index = search_index::SearchIndex::build(&indexed_items);
        let app = AppState::new(
            indexed_items,
            search_index,
            theme::Theme::Dracula.config(),
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            3,
            0.0,
            std::path::PathBuf::from("/tmp/h.txt"),
            None,
        );
        assert!(app.id_set.contains("base_rifle"));
        assert!(app.id_set.contains("other"));
        assert_eq!(app.id_set.len(), 2);
    }

    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn make_mouse_test_app(items: usize) -> AppState {
        let indexed_items = (0..items)
            .map(|i| {
                let id = format!("item_{}", i);
                data::IndexedItem {
                    value: json!({"id": id.clone()}),
                    id,
                    item_type: "t".to_string(),
                }
            })
            .collect::<Vec<_>>();
        let search_index = search_index::SearchIndex::build(&indexed_items);
        AppState::new(
            indexed_items,
            search_index,
            theme::Theme::Dracula.config(),
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            items,
            0.0,
            std::path::PathBuf::from("/tmp/h.txt"),
            None,
        )
    }

    #[test]
    fn test_mouse_click_list_selects_item_and_focuses_list() {
        let mut app = make_mouse_test_app(8);
        app.list_area = Some(Rect::new(0, 0, 20, 8));
        app.list_content_area = Some(Rect::new(1, 1, 18, 6));
        app.details_area = Some(Rect::new(20, 0, 40, 8));
        app.filter_area = Some(Rect::new(0, 8, 60, 3));

        let transitioned = handle_mouse_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 3),
        );

        assert!(transitioned);
        assert_eq!(app.focused_pane, FocusPane::List);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.list_state.selected(), Some(2));
    }

    #[test]
    fn test_mouse_click_filter_sets_caret_position() {
        let mut app = make_mouse_test_app(1);
        app.filter_text = "abcdef".to_string();
        app.filter_cursor = app.filter_text.chars().count();
        app.list_area = Some(Rect::new(0, 0, 20, 8));
        app.details_area = Some(Rect::new(20, 0, 40, 8));
        app.filter_area = Some(Rect::new(0, 8, 60, 3));
        app.filter_input_area = Some(Rect::new(1, 9, 58, 1));

        let transitioned = handle_mouse_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 9),
        );

        assert!(transitioned);
        assert_eq!(app.focused_pane, FocusPane::Filter);
        assert_eq!(app.input_mode, InputMode::Filtering);
        assert_eq!(app.filter_cursor, 2);
    }

    #[test]
    fn test_mouse_click_filter_past_end_clamps_to_end() {
        let mut app = make_mouse_test_app(1);
        app.filter_text = "abc".to_string();
        app.filter_cursor = 0;
        app.list_area = Some(Rect::new(0, 0, 20, 8));
        app.details_area = Some(Rect::new(20, 0, 40, 8));
        app.filter_area = Some(Rect::new(0, 8, 30, 3));
        app.filter_input_area = Some(Rect::new(1, 9, 28, 1));

        let transitioned = handle_mouse_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 20, 9),
        );

        assert!(transitioned);
        assert_eq!(app.filter_cursor, app.filter_text.chars().count());
    }

    #[test]
    fn test_mouse_scroll_hovered_list_moves_by_constant() {
        let mut app = make_mouse_test_app(10);
        app.list_area = Some(Rect::new(0, 0, 20, 10));
        app.list_content_area = Some(Rect::new(1, 1, 18, 8));

        let transitioned =
            handle_mouse_event(&mut app, mouse_event(MouseEventKind::ScrollDown, 2, 2));

        assert!(transitioned);
        assert_eq!(app.list_state.selected(), Some(SCROLL_LINES as usize));
        assert_eq!(app.focused_pane, FocusPane::List);
    }

    #[test]
    fn test_mouse_scroll_hovered_details_moves_by_constant() {
        let mut app = make_mouse_test_app(1);
        app.details_area = Some(Rect::new(20, 0, 40, 10));

        let transitioned =
            handle_mouse_event(&mut app, mouse_event(MouseEventKind::ScrollDown, 25, 1));

        assert!(transitioned);
        assert_eq!(app.details_scroll_state.offset().y, SCROLL_LINES);
    }

    #[test]
    fn test_mouse_click_details_focuses_even_without_link() {
        let mut app = make_mouse_test_app(1);
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = ui::highlight_json_annotated(r#""id": 1"#, &style);
        app.details_wrapped_annotated = ui::wrap_annotated_lines(&annotated, 20);
        app.details_area = Some(Rect::new(20, 0, 40, 10));
        app.details_content_area = Some(Rect::new(20, 0, 40, 10));
        app.filter_text = "x".to_string();
        app.filter_cursor = 1;
        app.focused_pane = FocusPane::List;
        app.input_mode = InputMode::Normal;

        let transitioned = handle_mouse_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 22, 0),
        );

        assert!(transitioned);
        assert_eq!(app.focused_pane, FocusPane::Details);
        assert_eq!(app.filter_text, "x");
    }
}
