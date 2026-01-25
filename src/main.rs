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
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::io;

mod matcher;
mod search_index;
mod theme;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
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

#[derive(Debug, Deserialize)]
struct Root {
    data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct GameBuild {
    build_number: String,
    prerelease: bool,
    created_at: String,
}

/// Application state for ratatui app.
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
    /// Whether filter input has focus
    filter_focused: bool,
    /// Theme configuration
    theme: theme::ThemeConfig,
    /// Scroll offset for details pane
    details_scroll: usize,
    /// Flag to quit app
    should_quit: bool,
}

impl AppState {
    fn new(
        indexed_items: Vec<(Value, String, String)>,
        search_index: search_index::SearchIndex,
        theme: theme::ThemeConfig,
    ) -> Self {
        let filtered_indices: Vec<usize> = (0..indexed_items.len()).collect();
        let mut list_state = ListState::default();
        if filtered_indices.is_empty() {
            list_state.select(None);
        } else {
            list_state.select(Some(0));
        }

        Self {
            indexed_items,
            search_index,
            filtered_indices,
            list_state,
            filter_text: String::new(),
            filter_cursor: 0,
            filter_focused: false,
            theme,
            details_scroll: 0,
            should_quit: false,
        }
    }

    fn get_selected_item(&self) -> Option<&(Value, String, String)> {
        self.list_state
            .selected()
            .and_then(|idx| self.filtered_indices.get(idx))
            .and_then(|&idx| self.indexed_items.get(idx))
    }

    fn scroll_details_up(&mut self) {
        if self.details_scroll > 0 {
            self.details_scroll -= 1;
        }
    }

    fn scroll_details_down(&mut self) {
        self.details_scroll += 1;
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
        // Reset selection to first item
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        self.details_scroll = 0;
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Early theme validation
    if let Some(theme) = &args.theme {
        match theme.as_str() {
            "dracula" | "solarized" | "gruvbox" | "everforest_light" => (),
            _ => anyhow::bail!(
                "Unknown theme: {}. Available: dracula, solarized, gruvbox, everforest_light",
                theme
            ),
        }
    }

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

        let mut builds: Vec<GameBuild> = serde_json::from_str(&content)?;
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

    let file_path = if let Some(file) = args.file {
        file
    } else {
        let game_version = args.game;
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
    let reader = std::io::BufReader::new(file);
    let root: Root = serde_json::from_reader(reader)?;

    println!("Building search index for {} items...", root.data.len());
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
    let index_time = start.elapsed();
    println!("Index built in {:.2}ms", index_time.as_secs_f64() * 1000.0);

    // Choose theme
    let theme_name = args.theme.as_deref().unwrap_or("dracula");
    let theme = match theme_name {
        "dracula" => theme::dracula_theme(),
        "solarized" => theme::solarized_dark(),
        "gruvbox" => theme::gruvbox_theme(),
        "everforest_light" => theme::everforest_light_theme(),
        _ => anyhow::bail!(
            "Unknown theme: {}. Available: dracula, solarized, gruvbox, everforest_light",
            theme_name
        ),
    };

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = AppState::new(indexed_items, search_index, theme);

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
    match code {
        KeyCode::Char('q') | KeyCode::Esc if !app.filter_focused => {
            app.should_quit = true;
        }
        KeyCode::Char('/') if !app.filter_focused => {
            app.filter_focused = true;
        }
        KeyCode::Esc if app.filter_focused => {
            app.filter_focused = false;
        }
        KeyCode::Enter if app.filter_focused => {
            app.filter_focused = false;
        }
        KeyCode::Up if !app.filter_focused => {
            app.list_state.select_previous();
            // Clamp to valid indices
            if let Some(selected) = app.list_state.selected()
                && selected >= app.filtered_indices.len()
                && !app.filtered_indices.is_empty()
            {
                app.list_state.select(Some(app.filtered_indices.len() - 1));
            }
            app.details_scroll = 0;
        }
        KeyCode::Down if !app.filter_focused => {
            app.list_state.select_next();
            // Clamp to valid indices
            if let Some(selected) = app.list_state.selected()
                && selected >= app.filtered_indices.len()
                && !app.filtered_indices.is_empty()
            {
                app.list_state.select(Some(app.filtered_indices.len() - 1));
            }
            app.details_scroll = 0;
        }
        KeyCode::PageUp if !app.filter_focused => {
            for _ in 0..10 {
                app.scroll_details_up();
            }
        }
        KeyCode::PageDown if !app.filter_focused => {
            for _ in 0..10 {
                app.scroll_details_down();
            }
        }
        KeyCode::Char('k') if modifiers.contains(KeyModifiers::CONTROL) && !app.filter_focused => {
            app.scroll_details_up();
        }
        KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) && !app.filter_focused => {
            app.scroll_details_down();
        }
        // Filter input handling
        KeyCode::Char(c) if app.filter_focused => {
            app.filter_add_char(c);
            app.update_filter();
        }
        KeyCode::Backspace if app.filter_focused => {
            app.filter_backspace();
            app.update_filter();
        }
        KeyCode::Delete if app.filter_focused => {
            app.filter_delete();
            app.update_filter();
        }
        KeyCode::Left if app.filter_focused => {
            app.filter_move_cursor_left();
        }
        KeyCode::Right if app.filter_focused => {
            app.filter_move_cursor_right();
        }
        KeyCode::Home if app.filter_focused => {
            app.filter_move_to_start();
        }
        KeyCode::End if app.filter_focused => {
            app.filter_move_to_end();
        }
        _ => {}
    }
}

fn ui(f: &mut Frame, app: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Main area - takes all space
            Constraint::Length(3), // Filter input - fixed 3 lines
        ])
        .split(f.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Render item list
    render_item_list(f, app, main_chunks[0]);

    // Render details pane
    render_details(f, app, main_chunks[1]);

    // Render filter input
    render_filter(f, app, chunks[1]);
}

fn render_item_list(f: &mut Frame, app: &mut AppState, area: Rect) {
    let items = app.filtered_indices.iter().map(|&idx| {
        let (json, id, type_) = &app.indexed_items[idx];
        let display_name = if !id.is_empty() {
            id.clone()
        } else if let Some(abstract_) = json.get("abstract").and_then(|v| v.as_str()) {
            format!("(abstract) {}", abstract_)
        } else if let Some(name) = json.get("name") {
            if let Some(name_str) = name.get("str").and_then(|v| v.as_str()) {
                name_str.to_string()
            } else if let Some(name_str) = name.as_str() {
                name_str.to_string()
            } else {
                "(unknown)".to_string()
            }
        } else {
            "(unknown)".to_string()
        };

        let label = Line::from(vec![
            Span::styled(format!("[{}] ", type_), app.theme.title),
            Span::raw(display_name),
        ]);
        ListItem::new(label)
    });

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border_selected)
                .title_style(app.theme.title)
                .title(format!(" Items ({}) ", app.filtered_indices.len()))
                .title_alignment(ratatui::layout::Alignment::Left),
        )
        .style(app.theme.list_normal)
        .highlight_style(app.theme.list_selected);

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_details(f: &mut Frame, app: &AppState, area: Rect) {
    let content = if let Some((json, _, _)) = app.get_selected_item() {
        match serde_json::to_string_pretty(json) {
            Ok(json_str) => highlight_json(&json_str, &app.theme.json_style),
            Err(_) => Text::from("Error formatting JSON"),
        }
    } else {
        Text::from("Select an item to view details")
    };

    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title("JSON definition")
                .title_style(app.theme.title),
        )
        .style(app.theme.text)
        .wrap(Wrap { trim: false })
        .scroll(((app.details_scroll.min(u16::MAX as usize)) as u16, 0));

    f.render_widget(paragraph, area);
}

fn render_filter(f: &mut Frame, app: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.filter_focused {
            app.theme.border_selected
        } else {
            app.theme.border
        })
        .title("Filter")
        .title_style(app.theme.title);

    let inner = block.inner(area);
    let content = if app.filter_text.is_empty() && !app.filter_focused {
        Text::from(Line::from(Span::styled(
            "Press '/' to focus...",
            app.theme.text.add_modifier(Modifier::DIM),
        )))
    } else {
        Text::from(app.filter_text.as_str())
    };

    let paragraph = Paragraph::new(content)
        .block(block)
        .style(app.theme.text);

    f.render_widget(paragraph, area);

    if app.filter_focused {
        if inner.width > 0 && inner.height > 0 {
            let cursor_offset = filter_cursor_offset(&app.filter_text, app.filter_cursor);
            let max_x = inner.width.saturating_sub(1);
            let cursor_x = inner.x + cursor_offset.min(max_x);
            let cursor_y = inner.y;
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
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
                    // This quote is escaped, treat it as normal text and continue searching
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
                // Find next UNESCAPED quote
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
                            ratatui::style::Style::default()
                                .fg(ratatui::style::Color::Cyan)
                        )
                    } else {
                        Span::styled(
                            format!("\"{}\"", quoted),
                            ratatui::style::Style::default().fg(json_style.string),
                        )
                    };

                    spans.push(styled);
                    remaining = &rest[ep + 1..];
                } else {
                    spans.push(Span::styled(
                        remaining.to_string(),
                        ratatui::style::Style::default().fg(json_style.string),
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
                        Span::styled(
                            token.to_string(),
                            ratatui::style::Style::default().fg(json_style.boolean),
                        )
                    } else if (token.chars().all(|c| {
                        c.is_numeric() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+'
                    })) && !token.is_empty()
                        && token.chars().any(|c| c.is_numeric())
                    {
                        Span::styled(
                            token.to_string(),
                            ratatui::style::Style::default().fg(json_style.number),
                        )
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
}
