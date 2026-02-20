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
    pub indexed_items: Vec<(Value, String, String)>,
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
    /// Cached Text object for the current details_wrapped_annotated
    pub details_wrapped_text: ratatui::text::Text<'static>,
    /// Width used for current details_wrapped_annotated
    pub details_wrapped_width: u16,
    /// Currently hovered span ID for tracking click/hover
    pub hovered_span_id: Option<usize>,
    /// Screen region of the JSON content area (set during render)
    pub details_content_area: Option<ratatui::layout::Rect>,
    /// Flag to quit app
    pub should_quit: bool,
    /// Whether help overlay is visible
    pub show_help: bool,
    /// Whether version picker is visible
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
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        indexed_items: Vec<(Value, String, String)>,
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
            .filter(|(_, id, _)| !id.is_empty())
            .map(|(_, id, _)| id.clone())
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
            details_wrapped_text: ratatui::text::Text::default(),
            details_wrapped_width: 0,
            hovered_span_id: None,
            details_content_area: None,
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

    fn refresh_details(&mut self) {
        if let Some((json, _, _)) = self.get_selected_item() {
            match serde_json::to_string_pretty(json) {
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
        self.details_scroll_state = ScrollViewState::default();
        self.details_wrapped_width = 0;
        self.details_wrapped_annotated.clear();
        self.details_wrapped_text = ratatui::text::Text::default();
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

    pub fn get_selected_item(&self) -> Option<&(Value, String, String)> {
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

    fn update_filter(&mut self) {
        let new_filtered =
            matcher::search_with_index(&self.search_index, &self.indexed_items, &self.filter_text);
        self.filtered_indices = new_filtered;
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        self.refresh_details();
    }

    fn apply_new_dataset(
        &mut self,
        indexed_items: Vec<(Value, String, String)>,
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
            .filter(|(_, id, _)| !id.is_empty())
            .map(|(_, id, _)| id.clone())
            .collect();

        self.indexed_items = indexed_items;
        self.search_index = search_index;
        self.id_set = id_set;
        self.total_items = total_items;
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
        app.input_mode = InputMode::Normal;
        app.history_index = None;
        app.pending_action = Some(AppAction::OpenVersionPicker);
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
            KeyCode::Up | KeyCode::Char('k') => app.version_list_state.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => app.version_list_state.select_next(),
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
            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
            KeyCode::Char('/') => app.input_mode = InputMode::Filtering,
            KeyCode::Char('?') => app.show_help = true,
            KeyCode::Up | KeyCode::Char('k') if !modifiers.contains(KeyModifiers::CONTROL) => {
                app.move_selection(-1);
            }
            KeyCode::Down | KeyCode::Char('j') if !modifiers.contains(KeyModifiers::CONTROL) => {
                app.move_selection(1);
            }
            KeyCode::Home => {
                app.list_state.select(Some(0));
                app.refresh_details();
            }
            KeyCode::End => {
                let len = app.filtered_indices.len();
                if len > 0 {
                    app.list_state.select(Some(len - 1));
                    app.refresh_details();
                }
            }
            KeyCode::PageUp => app.details_scroll_state.scroll_page_up(),
            KeyCode::PageDown => app.details_scroll_state.scroll_page_down(),
            KeyCode::Char('k') if modifiers.contains(KeyModifiers::CONTROL) => {
                app.scroll_details_up();
            }
            KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
                app.scroll_details_down();
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
                app.input_mode = InputMode::Filtering;
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
                app.input_mode = InputMode::Normal;
            }
            KeyCode::Esc => {
                app.history_index = None;
                app.input_mode = InputMode::Normal;
            }
            KeyCode::Char(c) => {
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

const ID_LIKE_FIELDS: &[&str] = &[
    "copy-from",
    "abstract",
    "id",
    "result",
    "using",
    "from",
    "to",
    "extends",
    "looks_like",
    "repairs_like",
    "weapon_category",
];

fn handle_mouse_event(app: &mut AppState, mouse: event::MouseEvent) -> bool {
    let mut is_valid_target = false;
    let mut new_hover_id = None;
    let mut target_path = String::new();
    let mut target_id = None;

    if let Some(span) = ui::hit_test_details(app, mouse.column, mouse.row)
        && let Some(path) = &span.key_context
    {
        let path_str = path.as_ref();
        let first_part = path_str.split('.').next().unwrap_or("");
        if !matches!(
            first_part,
            "id" | "abstract" | "description" | "__filename" | "//" | "//2" | "rows" | "name"
        ) && span.span_id.is_some()
        {
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
        app.details_wrapped_text =
            ui::annotated_to_text(app.details_wrapped_annotated.clone(), app.hovered_span_id);
        transitioned = true;
    }

    if matches!(
        mouse.kind,
        event::MouseEventKind::Down(event::MouseButton::Left)
    ) && is_valid_target
    {
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

        let is_id_like =
            ID_LIKE_FIELDS.contains(&target_path.as_str()) || app.id_set.contains(&unescaped_val);

        if is_id_like {
            app.filter_text = format!("i:{}", final_val);
            app.filter_cursor = app.filter_text.chars().count();
            app.update_filter();
            app.input_mode = InputMode::Normal;
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
            app.input_mode = InputMode::Filtering;
        }

        transitioned = true;
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
        app.start_progress("Loading local data", &["Loading files", "Indexing"]);
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
            let ratio = data::DownloadProgress {
                downloaded: progress.downloaded,
                total: progress.total,
            };
            let ratio = if let Some(t) = ratio.total {
                ratio.downloaded as f64 / t as f64
            } else {
                0.0
            };
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
) -> Result<(Vec<(Value, String, String)>, search_index::SearchIndex, f64)>
where
    B::Error: Send + Sync + 'static,
{
    let total = data.len();
    let start = Instant::now();
    let mut last_draw = Instant::now();
    let mut indexed_items: Vec<(Value, String, String)> = Vec::with_capacity(total);

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
        indexed_items.push((v, id, type_));

        if total > 0 && (idx % 500 == 0 || idx + 1 == total) {
            let ratio = (idx + 1) as f64 / total as f64 * 0.4;
            app.update_stage("Indexing", ratio);
            if last_draw.elapsed() >= Duration::from_millis(120) || idx + 1 == total {
                terminal.draw(|f| ui::ui(f, app))?;
                last_draw = Instant::now();
            }
        }
    }

    indexed_items.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));

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
    use serde_json::json;

    #[test]
    fn test_highlight_json() {
        let json_str = r#"{"id": "test", "val": 123, "active": true}"#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = ui::highlight_json_annotated(json_str, &style);
        let highlighted = ui::annotated_to_text(annotated, None);

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
            (json!({"id": "1"}), "1".to_string(), "type".to_string()),
            (json!({"id": "2"}), "2".to_string(), "type".to_string()),
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
            (
                json!({"id": "apple"}),
                "apple".to_string(),
                "fruit".to_string(),
            ),
            (
                json!({"id": "banana"}),
                "banana".to_string(),
                "fruit".to_string(),
            ),
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
        let indexed_items = vec![(json!({"id": "1"}), "1".to_string(), "t".to_string())];
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
        let indexed_items = vec![(json!({"id": "1"}), "1".to_string(), "t".to_string())];
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
    fn test_handle_key_event_ignores_release_kind() {
        let indexed_items = vec![(
            json!({"id": "apple"}),
            "apple".to_string(),
            "fruit".to_string(),
        )];
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
        let indexed_items = vec![(json!({"id": "1"}), "1".to_string(), "t".to_string())];
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
            (
                json!({"id": "base_rifle"}),
                "base_rifle".to_string(),
                "t".to_string(),
            ),
            (json!({"id": "other"}), "other".to_string(), "t".to_string()),
            (json!({"name": "no_id"}), "".to_string(), "t".to_string()),
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
}
