//! # cbn-tui
//!
//! A terminal user interface (TUI) for browsing Cataclysm: Bright Nights game data.

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, text::Text, widgets::ListState};
use serde_json::Value;
use std::fs;
use std::io;
use std::str::FromStr;
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
}

/// Current input mode for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Normal navigation mode
    Normal,
    /// Mode for entering filter text
    Filtering,
}

/// Application state for the Ratatui app.
pub struct AppState {
    /// All loaded items in indexed format (json, id, type)
    pub indexed_items: Vec<(Value, String, String)>,
    /// Search index for fast lookups
    pub search_index: search_index::SearchIndex,
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
    /// App version string
    pub app_version: String,
    /// Number of items in the full dataset
    pub total_items: usize,
    /// Time taken to build the index
    pub index_time_ms: f64,
    /// Scroll state for details pane
    pub details_scroll_state: ScrollViewState,
    /// Cached highlighted JSON text for the current selection
    pub details_text: Text<'static>,
    /// Number of lines in the current details_text
    pub details_line_count: usize,
    /// Flag to quit app
    pub should_quit: bool,
    /// Whether help overlay is visible
    pub show_help: bool,
    /// Previous search expressions
    pub filter_history: Vec<String>,
    /// Current index in history during navigation
    pub history_index: Option<usize>,
    /// Saved input when starting history navigation
    pub stashed_input: String,
    /// Path to history file
    pub history_path: std::path::PathBuf,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        indexed_items: Vec<(Value, String, String)>,
        search_index: search_index::SearchIndex,
        theme: theme::ThemeConfig,
        game_version: String,
        app_version: String,
        total_items: usize,
        index_time_ms: f64,
        history_path: std::path::PathBuf,
    ) -> Self {
        let filtered_indices: Vec<usize> = (0..indexed_items.len()).collect();
        let mut list_state = ListState::default();
        if filtered_indices.is_empty() {
            list_state.select(None);
        } else {
            list_state.select(Some(0));
        }

        let mut app = Self {
            indexed_items,
            search_index,
            filtered_indices,
            list_state,
            filter_text: String::new(),
            filter_cursor: 0,
            input_mode: InputMode::Normal,
            theme,
            game_version,
            app_version,
            total_items,
            index_time_ms,
            details_scroll_state: ScrollViewState::default(),
            details_text: Text::default(),
            details_line_count: 0,
            should_quit: false,
            show_help: false,
            filter_history: Vec::new(),
            history_index: None,
            stashed_input: String::new(),
            history_path,
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
                    self.details_text = ui::highlight_json(&json_str, &self.theme.json_style);
                    self.details_line_count = self.details_text.lines.len();
                }
                Err(_) => {
                    self.details_text = Text::from("Error formatting JSON");
                    self.details_line_count = 1;
                }
            }
        } else {
            self.details_text = Text::from("Select an item to view details");
            self.details_line_count = 1;
        }
        self.details_scroll_state = ScrollViewState::default();
    }

    /// Clamps the current list selection to valid bounds.
    fn clamp_selection(&mut self) {
        let len = self.filtered_indices.len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }

        if let Some(selected) = self.list_state.selected()
            && selected >= len {
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
            && let Some((byte_idx, _)) = self.filter_text.char_indices().nth(self.filter_cursor) {
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

    let file_path = if let Some(file) = args.file.as_ref() {
        file.clone()
    } else {
        data::fetch_game_data(&args.game, args.force)?
            .to_string_lossy()
            .to_string()
    };

    println!("Loading data from {}...", file_path);
    let root = data::load_root(&file_path)?;
    let total_items = root.data.len();

    // Determine version label
    let game_version_label = if args.file.is_some() && args.game == "nightly" {
        root.build.tag_name.clone()
    } else {
        let requested = &args.game;
        if !requested.is_empty()
            && requested != &root.build.build_number
            && requested != &root.build.tag_name
        {
            format!("{}:{}", requested, root.build.tag_name)
        } else {
            root.build.tag_name.clone()
        }
    };

    println!("Building search index for {} items...", total_items);
    let start = std::time::Instant::now();

    // Convert to indexed format (json, id, type)
    let mut indexed_items: Vec<(Value, String, String)> = root
        .data
        .into_iter()
        .map(|v| {
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
            (v, id, type_)
        })
        .collect();

    indexed_items.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));

    let search_index = search_index::SearchIndex::build(&indexed_items);
    let index_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("Index built in {:.2}ms", index_time_ms);

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = AppState::new(
        indexed_items,
        search_index,
        theme,
        game_version_label,
        app_version,
        total_items,
        index_time_ms,
        history_path,
    );

    let res = run_app(&mut terminal, &mut app);

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
                handle_key_event(app, key.code, key.modifiers);
                terminal.draw(|f| ui::ui(f, app))?;
            }
            Event::Resize(_, _) => {
                terminal.draw(|f| ui::ui(f, app))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_key_event(app: &mut AppState, code: KeyCode, modifiers: KeyModifiers) {
    fn apply_filter_edit(app: &mut AppState, edit: impl FnOnce(&mut AppState)) {
        edit(app);
        app.update_filter();
    }

    if app.show_help {
        if matches!(code, KeyCode::Char('?') | KeyCode::Esc) {
            app.show_help = false;
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
                    && app.filter_history.last() != Some(&app.filter_text) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_highlight_json() {
        let json_str = r#"{"id": "test", "val": 123, "active": true}"#;
        let style = theme::Theme::Dracula.config().json_style;
        let highlighted = ui::highlight_json(json_str, &style);

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
            2,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
        );

        assert_eq!(app.list_state.selected(), Some(0));
        handle_key_event(&mut app, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.list_state.selected(), Some(1));
        handle_key_event(&mut app, KeyCode::Up, KeyModifiers::NONE);
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
            2,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
        );

        handle_key_event(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
        assert_eq!(app.input_mode, InputMode::Filtering);

        handle_key_event(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(app.filter_text, "a");
        assert_eq!(app.filtered_indices.len(), 2);

        handle_key_event(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
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
            1,
            0.0,
            std::path::PathBuf::from("/tmp/history.txt"),
        );

        handle_key_event(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
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
            1,
            0.0,
            history_path.clone(),
        );

        app.input_mode = InputMode::Filtering;
        app.filter_text = "test_query".to_string();
        handle_key_event(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(app.filter_history.len(), 1);
        assert_eq!(app.filter_history[0], "test_query");

        app.input_mode = InputMode::Filtering;
        app.filter_text = "".to_string();
        handle_key_event(&mut app, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.filter_text, "test_query");

        let _ = fs::remove_file(&history_path);
    }
}
