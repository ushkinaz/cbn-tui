//! # cbn-tui
//!
//! A terminal user interface (TUI) for browsing Cataclysm: Bright Nights game data.
//!
//! This application provides an interactive browser for viewing and searching through
//! game JSON data with features including:
//! - Fast inverted-index search with classifiers (id:, type:, category:)
//! - Four beautiful themes (Dracula, Solarized, Gruvbox, Everforest Light)
//! - Syntax-highlighted JSON display
//! - Real-time filtering with UTF-8 support
//! - Automatic data downloading and caching

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect, Size},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::io;
use std::str::FromStr;
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

mod matcher;
mod search_index;
mod theme;

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
}

/// Core metadata for a game build, flattened from various JSON sources.
#[derive(Debug, Clone)]
struct BuildInfo {
    /// The unique build identifier (e.g., "2024-01-01" or "v0.9.1").
    build_number: String,
    /// The human-readable tag name (often matches build_number or is more descriptive).
    tag_name: String,
    /// Whether this is a prerelease/nightly build.
    prerelease: bool,
    /// ISO 8601 creation timestamp.
    created_at: String,
}

/// The root structure of the game data JSON (`all.json`).
#[derive(Debug, Deserialize)]
struct Root {
    /// Flattened build metadata.
    #[serde(flatten)]
    build: BuildInfo,
    /// The actual game data items.
    data: Vec<Value>,
}

impl<'de> Deserialize<'de> for BuildInfo {
    /// Custom deserializer to flatten the potential nesting of `release.tag_name`
    /// from Github-style JSON responses into a flat domain model.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Proxy {
            build_number: String,
            prerelease: Option<bool>,
            created_at: Option<String>,
            release: Option<Value>,
        }

        let proxy = Proxy::deserialize(deserializer)?;

        let mut tag_name = proxy.build_number.clone();
        let mut prerelease = proxy.prerelease.unwrap_or(false);
        let mut created_at = proxy.created_at.unwrap_or_default();

        // Extract flattened fields from the optional nested `release` object
        if let Some(release) = proxy.release {
            if let Some(tag) = release.get("tag_name").and_then(|v| v.as_str()) {
                tag_name = tag.to_string();
            }
            if let Some(pre) = release.get("prerelease").and_then(|v| v.as_bool()) {
                prerelease = pre;
            }
            if let Some(created) = release.get("created_at").and_then(|v| v.as_str()) {
                created_at = created.to_string();
            }
        }

        Ok(BuildInfo {
            build_number: proxy.build_number,
            tag_name,
            prerelease,
            created_at,
        })
    }
}

/// Current input mode for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    /// Normal navigation mode
    Normal,
    /// Mode for entering filter text
    Filtering,
}

/// Application state for the Ratatui app.
struct AppState {
    /// All loaded items in indexed format (json, id, type)
    indexed_items: Vec<(Value, String, String)>,
    /// Search index for fast lookups
    search_index: search_index::SearchIndex,
    /// Indices into indexed_items that match the current filter
    filtered_indices: Vec<usize>,
    /// List selection state managed by ratatui
    list_state: ListState,
    /// Filter input text
    filter_text: String,
    /// Cursor position in filter
    filter_cursor: usize,
    /// Current input mode
    input_mode: InputMode,
    /// Theme configuration
    theme: theme::ThemeConfig,
    /// Resolved game version (from JSON tag_name)
    game_version: String,
    /// App version string
    app_version: String,
    /// Number of items in the full dataset
    total_items: usize,
    /// Time taken to build the index
    index_time_ms: f64,
    /// Scroll state for details pane
    details_scroll_state: ScrollViewState,
    /// Cached highlighted JSON text for the current selection
    details_text: Text<'static>,
    /// Number of lines in the current details_text
    details_line_count: usize,
    /// Flag to quit app
    should_quit: bool,
    /// Whether help overlay is visible
    show_help: bool,
    /// Previous search expressions
    filter_history: Vec<String>,
    /// Current index in history during navigation
    history_index: Option<usize>,
    /// Saved input when starting history navigation
    stashed_input: String,
    /// Path to history file
    history_path: std::path::PathBuf,
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
                    self.details_text = highlight_json(&json_str, &self.theme.json_style);
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

    fn get_selected_item(&self) -> Option<&(Value, String, String)> {
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
        // Convert char index to byte index for safe UTF-8 insertion
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
            // Convert char index to byte index for safe UTF-8 removal
            if let Some((byte_idx, _)) = self.filter_text.char_indices().nth(self.filter_cursor) {
                self.filter_text.remove(byte_idx);
            }
        }
    }

    fn filter_delete(&mut self) {
        let char_count = self.filter_text.chars().count();
        if self.filter_cursor < char_count {
            // Convert char index to byte index for safe UTF-8 removal
            if let Some((byte_idx, _)) = self.filter_text.char_indices().nth(self.filter_cursor) {
                self.filter_text.remove(byte_idx);
            }
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
        // Reset selection to the first item
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        self.refresh_details();
    }

    #[cfg(test)]
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
        let project_dirs = directories::ProjectDirs::from("com", "cataclysmbn", "cbn-tui")
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        let cache_dir = project_dirs.cache_dir();
        fs::create_dir_all(cache_dir)?;
        let builds_path = cache_dir.join("builds.json");

        let mut should_download = args.force || !builds_path.exists();
        if !should_download
            && let Ok(metadata) = fs::metadata(&builds_path)
            && let Ok(modified) = metadata.modified()
            && let Ok(elapsed) = modified.elapsed()
            && elapsed.as_secs() > 3600
        {
            should_download = true;
        }

        let content = if should_download {
            let url = "https://data.cataclysmbn-guide.com/builds.json";
            let response = reqwest::blocking::get(url)?;
            if !response.status().is_success() {
                anyhow::bail!("Failed to download builds list: {}", response.status());
            }
            let bytes = response.bytes()?;
            fs::write(&builds_path, &bytes)?;
            String::from_utf8(bytes.to_vec())?
        } else {
            fs::read_to_string(&builds_path)?
        };

        let mut builds: Vec<BuildInfo> = serde_json::from_str(&content)?;
        // List in order of creation (newest first)
        builds.sort_by(|a, b| b.created_at.cmp(&a.created_at));

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

    let requested_version = args.game.clone();
    let file_path = if let Some(file) = args.file.as_ref() {
        file.clone()
    } else {
        let game_version = requested_version.clone();
        let project_dirs = directories::ProjectDirs::from("com", "cataclysmbn", "cbn-tui")
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        let version_cache_dir = project_dirs.cache_dir().join(&game_version);
        fs::create_dir_all(&version_cache_dir)?;

        let target_path = version_cache_dir.join("all.json");

        let mut should_download = args.force || !target_path.exists();
        let expiration = match game_version.as_str() {
            "nightly" => Some(std::time::Duration::from_secs(12 * 3600)),
            "stable" => Some(std::time::Duration::from_secs(30 * 24 * 3600)),
            _ => None,
        };

        if !should_download
            && let Some(exp) = expiration
            && let Ok(metadata) = fs::metadata(&target_path)
            && let Ok(modified) = metadata.modified()
            && let Ok(elapsed) = modified.elapsed()
            && elapsed > exp
        {
            should_download = true;
        }

        if should_download {
            let url = format!(
                "https://data.cataclysmbn-guide.com/data/{}/all.json",
                game_version
            );
            println!("Downloading data for {} from {}...", game_version, url);
            let response = reqwest::blocking::get(url)?;
            if !response.status().is_success() {
                anyhow::bail!("Failed to download data: {}", response.status());
            }
            let bytes = response.bytes()?;
            fs::write(&target_path, bytes)?;
        }
        target_path.to_string_lossy().to_string()
    };

    // Load Data
    println!("Loading data from {}...", file_path);
    if !std::path::Path::new(&file_path).exists() {
        if file_path == "all.json" {
            anyhow::bail!(
                "Default 'all.json' not found in current directory. Use --file or --game to specify data source."
            );
        } else {
            anyhow::bail!("File not found: {}", file_path);
        }
    }
    let file = fs::File::open(&file_path)?;
    let reader = io::BufReader::new(file);
    let root: Root = serde_json::from_reader(reader)?;

    let total_items = root.data.len();

    // Determine resolved version and version label
    let build_number = &root.build.build_number;
    let resolved_version = &root.build.tag_name;

    let game_version_label = if args.file.is_some() && args.game == "nightly" {
        // If loading from file and game is default, just show the file's version
        resolved_version.clone()
    } else {
        let requested = requested_version;
        if !requested.is_empty() && requested != *build_number && requested != *resolved_version {
            format!("{}:{}", requested, resolved_version)
        } else {
            resolved_version.clone()
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

    // Sort by type then id
    indexed_items.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));

    // Build search index
    let search_index = search_index::SearchIndex::build(&indexed_items);
    let index_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("Index built in {:.2}ms", index_time_ms);

    // Define history path
    let project_dirs = directories::ProjectDirs::from("com", "cataclysmbn", "cbn-tui")
        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;
    let history_path = project_dirs.data_dir().join("history.txt");

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
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

    // Run app
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
    // Initial draw
    terminal.draw(|f| ui(f, app))?;

    loop {
        if app.should_quit {
            break;
        }

        // Block waiting for events (no CPU usage when idle)
        match event::read()? {
            Event::Key(key) => {
                handle_key_event(app, key.code, key.modifiers);
                // Redraw after handling key event
                terminal.draw(|f| ui(f, app))?;
            }
            Event::Resize(_, _) => {
                // Redraw on terminal resize
                terminal.draw(|f| ui(f, app))?;
            }
            _ => {} // Ignore other events
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
        match code {
            KeyCode::Char('?') | KeyCode::Esc => {
                app.show_help = false;
            }
            _ => {}
        }
        return;
    }

    match app.input_mode {
        InputMode::Normal => match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                app.should_quit = true;
            }

            KeyCode::Char('/') => {
                app.input_mode = InputMode::Filtering;
            }

            KeyCode::Char('?') => {
                app.show_help = true;
            }

            KeyCode::Up | KeyCode::Char('k') if !modifiers.contains(KeyModifiers::CONTROL) => {
                app.move_selection(-1);
            }
            KeyCode::Down | KeyCode::Char('j') if !modifiers.contains(KeyModifiers::CONTROL) => {
                app.move_selection(1);
            }

            KeyCode::PageUp => {
                app.details_scroll_state.scroll_page_up();
            }
            KeyCode::PageDown => {
                app.details_scroll_state.scroll_page_down();
            }

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
                if !app.filter_text.trim().is_empty() {
                    // Add to history if not same as last
                    if app.filter_history.last() != Some(&app.filter_text) {
                        app.filter_history.push(app.filter_text.clone());
                        app.save_history();
                    }
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

            KeyCode::Left => {
                app.filter_move_cursor_left();
            }
            KeyCode::Right => {
                app.filter_move_cursor_right();
            }
            KeyCode::Home => {
                app.filter_move_to_start();
            }
            KeyCode::End => {
                app.filter_move_to_end();
            }

            _ => {}
        },
    }
}

fn ui(f: &mut Frame, app: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Main area - takes all space
            Constraint::Length(3), // Filter input - fixed 3 lines
            Constraint::Length(1), // Status bar
        ])
        .split(f.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(chunks[0]);

    // Render item list
    render_item_list(f, app, main_chunks[0]);

    // Render details pane
    render_details(f, app, main_chunks[1]);

    // Render filter input
    render_filter(f, app, chunks[1]);

    // Render status bar
    render_status_bar(f, app, chunks[2]);

    if app.show_help {
        render_help_overlay(f, app);
    }
}

fn render_item_list(f: &mut Frame, app: &mut AppState, area: Rect) {
    let items = app.filtered_indices.iter().map(|&idx| {
        let (json, id, type_) = &app.indexed_items[idx];
        let display_name = display_name_for_item(json, id, type_);

        let type_label = Line::from(vec![
            Span::styled(format!("{} ", type_), app.theme.title),
            Span::raw(display_name),
        ]);
        ListItem::new(type_label)
    });

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border_selected)
                .title_style(app.theme.title)
                .title(format!(" Items ({}) ", app.filtered_indices.len()))
                .title_bottom(Line::from(" up / down ").right_aligned())
                .title_alignment(Alignment::Left),
        )
        .style(app.theme.list_normal)
        .scroll_padding(2)
        .highlight_style(app.theme.list_selected);

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_details(f: &mut Frame, app: &mut AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border.bg(app.theme.background))
        .style(app.theme.border.bg(app.theme.background))
        .title(" JSON ")
        .title_alignment(ratatui::layout::Alignment::Left)
        .title_style(app.theme.title)
        .title_bottom(Line::from(" pg-up / pg-down ").right_aligned());

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    if inner_area.width > 0 && inner_area.height > 0 {
        // Extract metadata for the header
        let mut header_rows = Vec::new();
        if let Some((json, id, type_)) = app.get_selected_item() {
            let id_val = if !id.is_empty() {
                id.clone()
            } else {
                json.get("abstract")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let type_val = if !type_.is_empty() {
                type_.clone()
            } else {
                json.get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let name_val = json
                .get("name")
                .and_then(|v| name_value(v))
                .or_else(|| fallback_display_name(json, id, type_))
                .unwrap_or_default();
            let cat_val = json
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let col1_width = 40;

            // Row 1: id and name
            if !id_val.is_empty() || !name_val.is_empty() {
                header_rows.push(Line::from(vec![
                    Span::styled(
                        format!("{: <width$}", id_val, width = col1_width),
                        app.theme.text,
                    ),
                    Span::styled(name_val, app.theme.text),
                ]));
            }

            // Row 2: type and category
            if !type_val.is_empty() || !cat_val.is_empty() {
                header_rows.push(Line::from(vec![
                    Span::styled(
                        format!("{: <width$}", type_val, width = col1_width),
                        app.theme.text,
                    ),
                    Span::styled(cat_val, app.theme.text),
                ]));
            }
        }

        let mut content_area = inner_area;
        let horizontal_padding = 1;

        if !header_rows.is_empty() {
            let header_height = header_rows.len() as u16;

            // Render header with padding
            let header_render_area = Rect::new(
                inner_area.x + horizontal_padding,
                inner_area.y,
                inner_area.width.saturating_sub(horizontal_padding * 2),
                header_height,
            );
            f.render_widget(Paragraph::new(header_rows), header_render_area);

            // Render horizontal separator line that merges with borders
            let separator_y = inner_area.y + header_height;
            if separator_y < area.y + area.height - 1 {
                let separator_line = format!("├{}┤", "─".repeat(inner_area.width as usize));
                f.render_widget(
                    Paragraph::new(separator_line).style(app.theme.border),
                    Rect::new(area.x, separator_y, area.width, 1),
                );
                content_area = Rect::new(
                    inner_area.x,
                    separator_y + 1,
                    inner_area.width,
                    inner_area.height.saturating_sub(header_height + 1),
                );
            }
        }

        // Apply 1-symbol horizontal padding within the content area
        let content_width = content_area.width.saturating_sub(horizontal_padding * 2);

        if content_width > 0 && content_area.height > 0 {
            // Calculate the height required when text is wrapped to content_width
            let mut wrapped_height = 0;
            for line in &app.details_text.lines {
                let line_width = line.width() as u16;
                if line_width == 0 {
                    wrapped_height += 1;
                } else {
                    wrapped_height += line_width.div_ceil(content_width);
                }
            }
            let content_height = wrapped_height;

            let mut scroll_view = ScrollView::new(Size::new(content_width, content_height))
                .vertical_scrollbar_visibility(ScrollbarVisibility::Automatic)
                .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);

            // Match the background of the scroll view buffer to the theme
            let scroll_area = scroll_view.area();
            scroll_view.buf_mut().set_style(scroll_area, app.theme.text);

            let content_rect = Rect::new(0, 0, content_width, content_height);
            scroll_view.render_widget(
                Paragraph::new(app.details_text.clone())
                    .style(app.theme.text)
                    .wrap(Wrap { trim: false }),
                content_rect,
            );

            // Render ScrollView centered horizontally within content_area using the padding
            let scroll_view_area = Rect::new(
                content_area.x + horizontal_padding,
                content_area.y,
                content_width,
                content_area.height,
            );

            f.render_stateful_widget(scroll_view, scroll_view_area, &mut app.details_scroll_state);
        }
    }
}

fn render_filter(f: &mut Frame, app: &mut AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.input_mode == InputMode::Filtering {
            app.theme.border_selected
        } else {
            app.theme.border
        })
        .title(" Filter (/) ")
        .title_style(app.theme.title);

    let inner = block.inner(area);
    let content = if app.filter_text.is_empty() && app.input_mode != InputMode::Filtering {
        Text::from(Line::from(Span::styled(
            "t:gun ammo:rpg",
            app.theme.text.add_modifier(Modifier::DIM).italic(),
        )))
    } else {
        Text::from(app.filter_text.as_str())
    };

    let paragraph = Paragraph::new(content).block(block).style(app.theme.text);

    f.render_widget(paragraph, area);

    if app.input_mode == InputMode::Filtering && inner.width > 0 && inner.height > 0 {
        let cursor_offset = filter_cursor_offset(&app.filter_text, app.filter_cursor);
        let max_x = inner.width.saturating_sub(1);
        let cursor_x = inner.x + cursor_offset.min(max_x);
        let cursor_y = inner.y;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

fn render_status_bar(f: &mut Frame, app: &mut AppState, area: Rect) {
    let area = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(area);

    render_status_bar_shortcuts(f, app, chunks[0]);
    render_status_bar_operational(f, app, chunks[1]);
    render_status_bar_versions(f, app, chunks[2]);
}

fn render_status_bar_shortcuts(f: &mut Frame, app: &mut AppState, area: Rect) {
    let key_style = app.theme.title;
    let bar_style = app.theme.text.add_modifier(Modifier::DIM);

    let shortcuts = Line::from(vec![
        Span::styled("/ ", key_style),
        Span::raw("filter  "),
        Span::styled("? ", key_style),
        Span::raw("help  "),
        Span::styled("Esc ", key_style),
        Span::raw("quit"),
    ]);

    f.render_widget(
        Paragraph::new(shortcuts)
            .style(bar_style)
            .alignment(Alignment::Left),
        area,
    );
}

fn render_status_bar_operational(f: &mut Frame, app: &mut AppState, area: Rect) {
    let bar_style = app.theme.text.add_modifier(Modifier::DIM);
    let status = Line::from(format!(
        "Items: {} | Index: {:.2}ms",
        app.total_items, app.index_time_ms
    ));

    f.render_widget(
        Paragraph::new(status)
            .style(bar_style)
            .alignment(Alignment::Center),
        area,
    );
}

fn render_status_bar_versions(f: &mut Frame, app: &mut AppState, area: Rect) {
    let bar_style = app.theme.text.add_modifier(Modifier::DIM);
    let versions = Line::from(format!(
        "Game: {}  App: {}",
        app.game_version, app.app_version
    ));

    f.render_widget(
        Paragraph::new(versions)
            .style(bar_style)
            .alignment(Alignment::Right),
        area,
    );
}

fn render_help_overlay(f: &mut Frame, app: &mut AppState) {
    let area = f.area();
    let popup_width = area.width.min(76).saturating_sub(4);
    let popup_height = 24.min(area.height.saturating_sub(2));
    if popup_width == 0 || popup_height == 0 {
        return;
    }
    let popup_rect = Rect::new(
        area.x + (area.width.saturating_sub(popup_width)) / 2,
        area.y + (area.height.saturating_sub(popup_height)) / 2,
        popup_width,
        popup_height,
    );

    f.render_widget(Clear, popup_rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border_selected)
        .style(app.theme.border_selected.bg(app.theme.background))
        .title(" Help ")
        .border_type(ratatui::widgets::BorderType::Double)
        .title_style(app.theme.title);

    let inner_area = block.inner(popup_rect);
    f.render_widget(block, popup_rect);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10), // Navigation
            Constraint::Length(1),  // Spacer
            Constraint::Min(0),     // Search Syntax
        ])
        .margin(1)
        .split(inner_area);

    let key_style = app.theme.title;
    let desc_style = app.theme.text;
    let header_style = key_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    // 1. Navigation Section
    let nav_items = vec![
        ("/", "filter items"),
        ("Up | k", "selection up"),
        ("Down | j", "selection down"),
        ("PgUp | Ctrl+k", "scroll JSON up"),
        ("PgDown | Ctrl+j", "scroll JSON down"),
        ("?", "this help"),
        ("q", "quit"),
        ("Esc", "back / quit"),
    ];

    let mut nav_lines = vec![Line::from(Span::styled("Navigation", header_style))];
    for (key, desc) in nav_items {
        nav_lines.push(Line::from(vec![
            Span::styled(format!("{: <18}", key), key_style),
            Span::styled(desc, desc_style),
        ]));
    }
    f.render_widget(Paragraph::new(nav_lines), chunks[0]);

    // 2. Search Syntax Section
    let syntax_lines = vec![
        Line::from(Span::styled("Search Syntax", header_style)),
        Line::from(vec![
            Span::styled("word", key_style),
            Span::styled(" - generic search in all fields", desc_style),
        ]),
        Line::from(vec![
            Span::styled("t:gun", key_style),
            Span::styled(
                " - filter by type (shortcuts: i:id, t:type, c:cat)",
                desc_style,
            ),
        ]),
        Line::from(vec![
            Span::styled("bash.str_min:30", key_style),
            Span::styled(" - filter by nested field", desc_style),
        ]),
        Line::from(vec![
            Span::styled("'term'", key_style),
            Span::styled(" - exact match (surround with single quotes)", desc_style),
        ]),
        Line::from(vec![
            Span::styled("term1 term2", key_style),
            Span::styled(" - AND logic (matches both terms)", desc_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Example: ", key_style.add_modifier(Modifier::BOLD)),
            Span::styled("t:gun ammo:rpg", desc_style),
        ]),
    ];
    f.render_widget(Paragraph::new(syntax_lines), chunks[2]);
}

fn display_name_for_item(json: &Value, id: &str, type_: &str) -> String {
    if !id.is_empty() {
        return id.to_string();
    }

    if let Some(abstract_) = json.get("abstract").and_then(|v| v.as_str()) {
        return format!("(abs) {}", abstract_);
    }

    if let Some(name) = json.get("name").and_then(name_value) {
        if !name.is_empty() {
            return name;
        }
    }

    if let Some(fallback) = fallback_display_name(json, id, type_) {
        return fallback;
    }

    "(?)".to_string()
}

fn fallback_display_name(json: &Value, id: &str, type_: &str) -> Option<String> {
    if !id.is_empty() {
        return None;
    }

    match type_ {
        "recipe" => {
            let result = json.get("result").and_then(|v| v.as_str()).unwrap_or("");
            if result.is_empty() {
                return None;
            }
            let suffix = json.get("id_suffix").and_then(|v| v.as_str()).unwrap_or("");
            if suffix.is_empty() {
                Some(format!("result: {}", result))
            } else {
                Some(format!("result: {} (suffix: {})", result, suffix))
            }
        }
        "uncraft" => {
            if let Some(result) = json.get("result").and_then(|v| v.as_str()) {
                if !result.is_empty() {
                    return Some(format!("result: {}", result));
                }
            }
            None
        }
        "profession_item_substitutions" => {
            if let Some(trait_) = json.get("trait").and_then(|v| v.as_str()) {
                if !trait_.is_empty() {
                    return Some(format!("trait: {}", trait_));
                }
            }
            if let Some(item) = json.get("item").and_then(|v| v.as_str()) {
                if !item.is_empty() {
                    return Some(format!("item: {}", item));
                }
            }
            None
        }
        _ => None,
    }
}

fn name_value(value: &Value) -> Option<String> {
    if let Some(name_str) = value.as_str() {
        return Some(name_str.to_string());
    }
    if let Some(name_str) = value.get("str").and_then(|v| v.as_str()) {
        return Some(name_str.to_string());
    }
    if let Some(name_str) = value.get("str_sp").and_then(|v| v.as_str()) {
        return Some(name_str.to_string());
    }
    None
}

/// Applies syntax highlighting to JSON text using theme-consistent colors.
/// Returns a Text object for ratatui rendering.
fn highlight_json(json: &str, json_style: &theme::JsonStyle) -> Text<'static> {
    let mut lines = Vec::new();

    for line_str in json.lines() {
        let mut spans = Vec::new();
        let mut remaining = line_str;

        while !remaining.is_empty() {
            if let Some(pos) = remaining.find('"') {
                // Check if this quote is escaped
                let mut is_escaped = false;
                let mut j = pos;
                while j > 0 && remaining.as_bytes()[j - 1] == b'\\' {
                    is_escaped = !is_escaped;
                    j -= 1;
                }

                if is_escaped {
                    // This quote is escaped, treat it as a normal text and continue searching
                    let prefix = &remaining[..pos + 1];
                    if !prefix.is_empty() {
                        spans.push(Span::raw(prefix.to_string()));
                    }
                    remaining = &remaining[pos + 1..];
                    continue;
                }

                // Add prefix before quotes
                let prefix = &remaining[..pos];
                if !prefix.is_empty() {
                    spans.push(Span::raw(prefix.to_string()));
                }

                let rest = &remaining[pos + 1..];
                // Find the next UNESCAPED quote
                let mut end_pos = None;
                let mut search_idx = 0;
                while let Some(q_pos) = rest[search_idx..].find('"') {
                    let actual_q_pos = search_idx + q_pos;
                    let mut is_q_escaped = false;
                    let mut k = actual_q_pos;
                    while k > 0 && rest.as_bytes()[k - 1] == b'\\' {
                        is_q_escaped = !is_q_escaped;
                        k -= 1;
                    }
                    if !is_q_escaped {
                        end_pos = Some(actual_q_pos);
                        break;
                    }
                    search_idx = actual_q_pos + 1;
                }

                if let Some(ep) = end_pos {
                    let quoted = &rest[..ep];
                    let is_key = rest[ep + 1..].trim_start().starts_with(':');

                    let styled = if is_key {
                        Span::styled(
                            format!("\"{}\"", quoted),
                            Style::default()
                                .fg(json_style.key)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::styled(
                            format!("\"{}\"", quoted),
                            Style::default().fg(json_style.string),
                        )
                    };

                    spans.push(styled);
                    remaining = &rest[ep + 1..];
                } else {
                    spans.push(Span::styled(
                        remaining.to_string(),
                        Style::default().fg(json_style.string),
                    ));
                    remaining = "";
                }
            } else {
                // Process numbers, booleans, and null
                let mut remaining_processed = remaining;
                while !remaining_processed.is_empty() {
                    let trimmed = remaining_processed.trim_start();
                    let start_offset = remaining_processed.len() - trimmed.len();
                    if start_offset > 0 {
                        spans.push(Span::raw(remaining_processed[..start_offset].to_string()));
                    }

                    if trimmed.is_empty() {
                        break;
                    }

                    let token_end = trimmed
                        .find(|c: char| {
                            c.is_whitespace() || c == ',' || c == '}' || c == ']' || c == ':'
                        })
                        .map(|pos| if pos == 0 { 1 } else { pos })
                        .unwrap_or(trimmed.len());
                    let token = &trimmed[..token_end];
                    let rest = &trimmed[token_end..];

                    let styled = if token == "true" || token == "false" || token == "null" {
                        Span::styled(token.to_string(), Style::default().fg(json_style.boolean))
                    } else if (token.chars().all(|c| {
                        c.is_numeric() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+'
                    })) && !token.is_empty()
                        && token.chars().any(|c| c.is_numeric())
                    {
                        Span::styled(token.to_string(), Style::default().fg(json_style.number))
                    } else {
                        Span::raw(token.to_string())
                    };

                    spans.push(styled);
                    remaining_processed = rest;
                }
                remaining = "";
            }
        }
        lines.push(Line::from(spans));
    }

    Text::from(lines)
}

fn filter_cursor_offset(text: &str, cursor: usize) -> u16 {
    let char_count = text.chars().count();
    let clamped = cursor.min(char_count);
    let width = text.chars().take(clamped).count();
    width.min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;
    use serde_json::json;

    #[test]
    fn test_highlight_json() {
        let json = r#"{
  "id": "test",
  "count": 123,
  "active": true,
  "nested": {
    "key": "value"
  }
}"#;
        let json_style = theme::JsonStyle {
            key: Color::Rgb(0, 255, 255),
            string: Color::Rgb(0, 255, 0),
            number: Color::Rgb(0, 0, 255),
            boolean: Color::Rgb(255, 0, 0),
        };
        let highlighted = highlight_json(json, &json_style);

        // Basic check that we have content
        assert!(!highlighted.lines.is_empty());

        // Collect all text content
        let full_text: String = highlighted
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert!(full_text.contains("\"id\""));
        assert!(full_text.contains("\"test\""));
        assert!(full_text.contains("123"));
        assert!(full_text.contains("true"));
    }

    #[test]
    fn test_filter_cursor_offset() {
        let text = "hello";
        assert_eq!(filter_cursor_offset(text, 0), 0);
        assert_eq!(filter_cursor_offset(text, 2), 2);
        assert_eq!(filter_cursor_offset(text, 5), 5);
        assert_eq!(filter_cursor_offset(text, 10), 5);
    }

    #[test]
    fn test_indexed_format_sorting() {
        let mut items = [
            (
                json!({"id": "z_id"}),
                "z_id".to_string(),
                "A_TYPE".to_string(),
            ),
            (
                json!({"id": "a_id"}),
                "a_id".to_string(),
                "B_TYPE".to_string(),
            ),
            (
                json!({"id": "a_id"}),
                "a_id".to_string(),
                "A_TYPE".to_string(),
            ),
        ];

        // Sort by type then id
        items.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));

        assert_eq!(items[0].2, "A_TYPE");
        assert_eq!(items[0].1, "a_id");
        assert_eq!(items[1].2, "A_TYPE");
        assert_eq!(items[1].1, "z_id");
        assert_eq!(items[2].2, "B_TYPE");
        assert_eq!(items[2].1, "a_id");
    }

    #[test]
    fn test_handle_key_event_navigation() {
        let items = vec![
            (json!({"id": "a"}), "a".to_string(), "t".to_string()),
            (json!({"id": "b"}), "b".to_string(), "t".to_string()),
        ];
        let search_index = search_index::SearchIndex::build(&items);
        let theme = theme::dracula_theme();
        let mut app = AppState::new(
            items,
            search_index,
            theme,
            "test".to_string(),
            "v0.0.0".to_string(),
            2,
            0.0,
            std::path::PathBuf::from("history.txt"),
        );

        // Initial state
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.list_state.selected(), Some(0));

        // Move down
        handle_key_event(&mut app, KeyCode::Down, KeyModifiers::empty());
        assert_eq!(app.list_state.selected(), Some(1));

        // Move up
        handle_key_event(&mut app, KeyCode::Up, KeyModifiers::empty());
        assert_eq!(app.list_state.selected(), Some(0));

        // Test vim mode (down)
        handle_key_event(&mut app, KeyCode::Char('j'), KeyModifiers::empty());
        assert_eq!(app.list_state.selected(), Some(1));

        // Test vim mode (up)
        handle_key_event(&mut app, KeyCode::Char('k'), KeyModifiers::empty());
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_handle_key_event_filtering() {
        let items = vec![
            (json!({"id": "apple"}), "apple".to_string(), "t".to_string()),
            (
                json!({"id": "banana"}),
                "banana".to_string(),
                "t".to_string(),
            ),
        ];
        let search_index = search_index::SearchIndex::build(&items);
        let theme = theme::dracula_theme();
        let mut app = AppState::new(
            items,
            search_index,
            theme,
            "test".to_string(),
            "v0.0.0".to_string(),
            2,
            0.0,
            std::path::PathBuf::from("history.txt"),
        );

        // Switch to the filtering mode
        handle_key_event(&mut app, KeyCode::Char('/'), KeyModifiers::empty());
        assert_eq!(app.input_mode, InputMode::Filtering);

        // Type 'apple'
        for c in "apple".chars() {
            handle_key_event(&mut app, KeyCode::Char(c), KeyModifiers::empty());
        }
        assert_eq!(app.filter_text, "apple");
        assert_eq!(app.filtered_indices.len(), 1);

        // Backspace
        handle_key_event(&mut app, KeyCode::Backspace, KeyModifiers::empty());
        assert_eq!(app.filter_text, "appl");
        // 'appl' still matches 'apple'
        assert_eq!(app.filtered_indices.len(), 1);

        // Exit filtering mode with Esc
        handle_key_event(&mut app, KeyCode::Esc, KeyModifiers::empty());
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_handle_key_event_autofocus_filter() {
        let items = vec![
            (json!({"id": "apple"}), "apple".to_string(), "t".to_string()),
            (
                json!({"id": "banana"}),
                "banana".to_string(),
                "t".to_string(),
            ),
        ];
        let search_index = search_index::SearchIndex::build(&items);
        let theme = theme::dracula_theme();
        let mut app = AppState::new(
            items,
            search_index,
            theme,
            "test".to_string(),
            "v0.0.0".to_string(),
            2,
            0.0,
            std::path::PathBuf::from("history.txt"),
        );

        handle_key_event(&mut app, KeyCode::Char('a'), KeyModifiers::empty());
        assert_eq!(app.input_mode, InputMode::Filtering);
        assert_eq!(app.filter_text, "a");
        assert_eq!(app.filter_cursor, 1);
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    fn test_filter_history() {
        let items = vec![(json!({"id": "apple"}), "apple".to_string(), "t".to_string())];
        let search_index = search_index::SearchIndex::build(&items);
        let theme = theme::dracula_theme();
        let temp_dir = std::env::temp_dir();
        let history_path = temp_dir.join("cbn_tui_test_history.txt");
        if history_path.exists() {
            let _ = std::fs::remove_file(&history_path);
        }

        let mut app = AppState::new(
            items,
            search_index,
            theme,
            "test".to_string(),
            "v0.0.0".to_string(),
            1,
            0.0,
            history_path.clone(),
        );

        // Enter filter mode and type "apple", then enter
        handle_key_event(&mut app, KeyCode::Char('/'), KeyModifiers::empty());
        for c in "apple".chars() {
            handle_key_event(&mut app, KeyCode::Char(c), KeyModifiers::empty());
        }
        handle_key_event(&mut app, KeyCode::Enter, KeyModifiers::empty());
        assert_eq!(app.filter_history.len(), 1);
        assert_eq!(app.filter_history[0], "apple");

        // Verify it was saved
        let saved = std::fs::read_to_string(&history_path).unwrap();
        assert_eq!(saved.trim(), "apple");

        // Enter filter mode again, type "banana", then enter
        handle_key_event(&mut app, KeyCode::Char('/'), KeyModifiers::empty());
        app.clear_filter();
        for c in "banana".chars() {
            handle_key_event(&mut app, KeyCode::Char(c), KeyModifiers::empty());
        }
        handle_key_event(&mut app, KeyCode::Enter, KeyModifiers::empty());
        assert_eq!(app.filter_history.len(), 2);
        assert_eq!(app.filter_history[1], "banana");

        // Test navigation: Up
        handle_key_event(&mut app, KeyCode::Char('/'), KeyModifiers::empty());
        app.clear_filter();
        handle_key_event(&mut app, KeyCode::Up, KeyModifiers::empty());
        assert_eq!(app.filter_text, "banana");
        handle_key_event(&mut app, KeyCode::Up, KeyModifiers::empty());
        assert_eq!(app.filter_text, "apple");

        // Test navigation: Down
        handle_key_event(&mut app, KeyCode::Down, KeyModifiers::empty());
        assert_eq!(app.filter_text, "banana");
        handle_key_event(&mut app, KeyCode::Down, KeyModifiers::empty());
        assert_eq!(app.filter_text, ""); // Should return to stashed input (empty)

        // Clean up
        let _ = std::fs::remove_file(&history_path);
    }
}
