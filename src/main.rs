use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;
use serde_json::Value;
use std::{fs, io, time::Duration};
use tui_input::{backend::crossterm::EventHandler, Input};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the all.json file
    #[arg(short, long)]
    file: Option<String>,

    /// Game version to download (e.g. v0.9.1)
    #[arg(short, long)]
    game: Option<String>,

    /// Force download of game data even if cached
    #[arg(long)]
    force: bool,

    /// List all available game versions
    #[arg(long)]
    game_versions: bool,
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

#[derive(Clone)]
struct CbnItem {
    original_json: Value,
    display_name: String,
    // Fields for filtering
    id: String,
    type_: String,
    category: String,
    abstract_: String,
}

impl CbnItem {
    fn from_json(v: Value) -> Self {
        let id = v
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let abstract_ = v
            .get("abstract")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = v
            .get("name")
            .and_then(|v| {
                if v.is_object() {
                    v.get("str").and_then(|s| s.as_str())
                } else {
                    v.as_str()
                }
            })
            .unwrap_or("")
            .to_string();

        let type_ = v
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let category = v
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let display_name = if !id.is_empty() {
            id.clone()
        } else if !abstract_.is_empty() {
            format!("(abstract) {}", abstract_)
        } else if !name.is_empty() {
            name
        } else {
            "(unknown)".to_string()
        };

        Self {
            original_json: v,
            display_name,
            id,
            type_,
            category,
            abstract_,
        }
    }

    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        for part in query.split_whitespace() {
            let part_lower = part.to_lowercase();

            let match_found = if let Some(val) = part_lower.strip_prefix("id:") {
                self.id.to_lowercase().contains(val)
            } else if let Some(val) = part_lower.strip_prefix("i:") {
                self.id.to_lowercase().contains(val)
            } else if let Some(val) = part_lower.strip_prefix("type:") {
                self.type_.to_lowercase().contains(val)
            } else if let Some(val) = part_lower.strip_prefix("t:") {
                self.type_.to_lowercase().contains(val)
            } else if let Some(val) = part_lower.strip_prefix("category:") {
                self.category.to_lowercase().contains(val)
            } else if let Some(val) = part_lower.strip_prefix("c:") {
                self.category.to_lowercase().contains(val)
            } else {
                self.id.to_lowercase().contains(&part_lower)
                    || self.type_.to_lowercase().contains(&part_lower)
                    || self.abstract_.to_lowercase().contains(&part_lower)
                    || self.category.to_lowercase().contains(&part_lower)
            };

            if !match_found {
                return false;
            }
        }
        true
    }
}

enum InputMode {
    Normal,
    Editing,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum ActivePane {
    List,
    Details,
    Filter,
}

enum Message {
    Quit,
    EnterEdit,
    ExitEdit,
    MoveNext,
    MovePrevious,
    PageUp,
    PageDown,
    NextPane,
    PrevPane,
    FocusPane(ActivePane),
    Input(Event),
}

struct Model {
    items: Vec<CbnItem>,
    filtered_items: Vec<usize>, // Indices into self.items
    list_state: ListState,
    input: Input,
    input_mode: InputMode,
    active_pane: ActivePane,
    details_scroll: u16,
    should_quit: bool,

    // Store areas for mouse interaction
    list_area: Rect,
    details_area: Rect,
    filter_area: Rect,
}

impl Model {
    fn new(items: Vec<CbnItem>) -> Self {
        let indices: Vec<usize> = (0..items.len()).collect();
        let mut state = ListState::default();
        if !indices.is_empty() {
            state.select(Some(0));
        }
        Self {
            items,
            filtered_items: indices,
            list_state: state,
            input: Input::default(),
            input_mode: InputMode::Normal,
            active_pane: ActivePane::List,
            details_scroll: 0,
            should_quit: false,
            list_area: Rect::default(),
            details_area: Rect::default(),
            filter_area: Rect::default(),
        }
    }

    fn update_filter(&mut self) {
        self.filtered_items = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.matches(self.input.value()))
            .map(|(i, _)| i)
            .collect();

        // Reset selection
        if self.filtered_items.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn update(&mut self, msg: Message) {
        match msg {
            Message::Quit => self.should_quit = true,
            Message::EnterEdit => self.input_mode = InputMode::Editing,
            Message::ExitEdit => self.input_mode = InputMode::Normal,
            Message::MoveNext => self.move_next(),
            Message::MovePrevious => self.move_previous(),
            Message::PageUp => self.page_up(),
            Message::PageDown => self.page_down(),
            Message::NextPane => self.next_pane(),
            Message::PrevPane => self.prev_pane(),
            Message::FocusPane(pane) => self.focus_pane(pane),
            Message::Input(event) => {
                self.input.handle_event(&event);
                self.update_filter();
            }
        }
    }

    fn next_pane(&mut self) {
        self.active_pane = match self.active_pane {
            ActivePane::List => ActivePane::Details,
            ActivePane::Details => ActivePane::Filter,
            ActivePane::Filter => ActivePane::List,
        };
        if self.active_pane == ActivePane::Filter {
            self.input_mode = InputMode::Editing;
        } else {
            self.input_mode = InputMode::Normal;
        }
    }

    fn prev_pane(&mut self) {
        self.active_pane = match self.active_pane {
            ActivePane::List => ActivePane::Filter,
            ActivePane::Filter => ActivePane::Details,
            ActivePane::Details => ActivePane::List,
        };
        if self.active_pane == ActivePane::Filter {
            self.input_mode = InputMode::Editing;
        } else {
            self.input_mode = InputMode::Normal;
        }
    }

    fn focus_pane(&mut self, pane: ActivePane) {
        self.active_pane = pane;
        if self.active_pane == ActivePane::Filter {
            self.input_mode = InputMode::Editing;
        } else {
            self.input_mode = InputMode::Normal;
        }
    }

    fn move_next(&mut self) {
        match self.active_pane {
            ActivePane::List => self.select_next(),
            ActivePane::Details => self.details_scroll = self.details_scroll.saturating_add(1),
            ActivePane::Filter => {}
        }
    }

    fn move_previous(&mut self) {
        match self.active_pane {
            ActivePane::List => self.select_previous(),
            ActivePane::Details => self.details_scroll = self.details_scroll.saturating_sub(1),
            ActivePane::Filter => {}
        }
    }

    fn page_up(&mut self) {
        match self.active_pane {
            ActivePane::List => self.select_page_previous(),
            ActivePane::Details => self.details_scroll = self.details_scroll.saturating_sub(10),
            ActivePane::Filter => {}
        }
    }

    fn page_down(&mut self) {
        match self.active_pane {
            ActivePane::List => self.select_page_next(),
            ActivePane::Details => self.details_scroll = self.details_scroll.saturating_add(10),
            ActivePane::Filter => {}
        }
    }

    fn select_next(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.filtered_items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.details_scroll = 0;
    }

    fn select_previous(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered_items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.details_scroll = 0;
    }

    fn select_page_next(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                let next = i + 10; // Page size roughly 10
                if next >= self.filtered_items.len() {
                    self.filtered_items.len() - 1
                } else {
                    next
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn select_page_previous(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i < 10 {
                    0
                } else {
                    i - 10
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.game_versions {
        let project_dirs = directories::ProjectDirs::from("com", "cataclysmbn", "cbn-tui")
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        let cache_dir = project_dirs.cache_dir();
        fs::create_dir_all(cache_dir)?;
        let builds_path = cache_dir.join("builds.json");

        let mut should_download = args.force || !builds_path.exists();
        if !should_download {
            if let Ok(metadata) = fs::metadata(&builds_path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(elapsed) = modified.elapsed() {
                        if elapsed.as_secs() > 3600 {
                            should_download = true;
                        }
                    }
                }
            }
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
            let type_ = if build.prerelease { "Nightly" } else { "Stable" };
            println!("{} ({})", build.build_number, type_);
        }
        return Ok(());
    }

    let file_path = if let Some(game_version) = args.game {
        let project_dirs = directories::ProjectDirs::from("com", "cataclysmbn", "cbn-tui")
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        let version_cache_dir = project_dirs.cache_dir().join(&game_version);
        fs::create_dir_all(&version_cache_dir)?;

        let target_path = version_cache_dir.join("all.json");

        if args.force || !target_path.exists() {
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
    } else if let Some(file) = args.file {
        file
    } else {
        "all.json".to_string()
    };

    // 1. Load Data
    println!("Loading data from {}...", file_path);
    if !std::path::Path::new(&file_path).exists() {
        if file_path == "all.json" {
            anyhow::bail!("Default 'all.json' not found in current directory. Use --file or --game to specify data source.");
        } else {
            anyhow::bail!("File not found: {}", file_path);
        }
    }
    let content = fs::read_to_string(&file_path)?;
    let root: Root = serde_json::from_str(&content)?;
    let mut items: Vec<CbnItem> = root.data.into_iter().map(CbnItem::from_json).collect();

    // Sort items by type then id
    items.sort_by(|a, b| a.type_.cmp(&b.type_).then_with(|| a.id.cmp(&b.id)));

    // 2. Setup Terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 3. Run Loop
    let mut model = Model::new(items);
    let res = run(&mut terminal, &mut model);

    // 4. Teardown
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

fn run<B: Backend>(terminal: &mut Terminal<B>, model: &mut Model) -> io::Result<()> {
    loop {
        terminal.draw(|f| view(f, model))?;

        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            if let Some(msg) = handle_input_event(&model, &event) {
                model.update(msg);
            }
        }

        if model.should_quit {
            return Ok(());
        }
    }
}

fn handle_input_event(model: &Model, event: &Event) -> Option<Message> {
    match event {
        Event::Key(key) => {
            if key.kind != KeyEventKind::Press {
                return None;
            }
            handle_key_event(&model.input_mode, *key)
        }
        Event::Mouse(mouse) => {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                if model.list_area.contains(Position::new(mouse.column, mouse.row)) {
                    return Some(Message::FocusPane(ActivePane::List));
                }
                if model.details_area.contains(Position::new(mouse.column, mouse.row)) {
                    return Some(Message::FocusPane(ActivePane::Details));
                }
                if model.filter_area.contains(Position::new(mouse.column, mouse.row)) {
                    return Some(Message::FocusPane(ActivePane::Filter));
                }
            }
            None
        }
        _ => None,
    }
}

fn handle_key_event(input_mode: &InputMode, key: event::KeyEvent) -> Option<Message> {
    match key.code {
        KeyCode::Tab => return Some(Message::NextPane),
        KeyCode::BackTab => return Some(Message::PrevPane),
        KeyCode::Esc => return Some(Message::Quit),
        _ => {}
    }

    match input_mode {
        InputMode::Normal => match key.code {
            KeyCode::Char('q') => Some(Message::Quit),
            KeyCode::Char('/') => Some(Message::EnterEdit),
            KeyCode::Down | KeyCode::Char('j') => Some(Message::MoveNext),
            KeyCode::Up | KeyCode::Char('k') => Some(Message::MovePrevious),
            KeyCode::PageDown => Some(Message::PageDown),
            KeyCode::PageUp => Some(Message::PageUp),
            _ => None,
        },
        InputMode::Editing => match key.code {
            KeyCode::Enter => Some(Message::ExitEdit),
            KeyCode::PageDown => Some(Message::PageDown),
            KeyCode::PageUp => Some(Message::PageUp),
            _ => Some(Message::Input(Event::Key(key))),
        },
    }
}

fn view(f: &mut Frame, model: &mut Model) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // Main View
            Constraint::Length(3), // Filter Input
        ])
        .split(f.area());

    model.filter_area = chunks[1];

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(chunks[0]);

    model.list_area = main_chunks[0];
    model.details_area = main_chunks[1];

    // render list
    let list_items: Vec<ListItem> = model
        .filtered_items
        .iter()
        .map(|&idx| {
            let item = &model.items[idx];
            // Format: [TYPE] DisplayName
            let content = format!("[{}] {}", item.type_, item.display_name);
            ListItem::new(content)
        })
        .collect();

    let list = List::new(list_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Items")
                .border_style(if model.active_pane == ActivePane::List {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, main_chunks[0], &mut model.list_state);

    // render details
    let details_lines = if let Some(idx) = model.list_state.selected() {
        if idx < model.filtered_items.len() {
            let real_idx = model.filtered_items[idx];
            let item = &model.items[real_idx];
            match serde_json::to_string_pretty(&item.original_json) {
                Ok(s) => highlight_json(&s),
                Err(_) => Text::raw("Error parsing JSON"),
            }
        } else {
            Text::raw("No item selected")
        }
    } else {
        Text::raw("No item selected")
    };

    let paragraph = Paragraph::new(details_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Details")
                .border_style(if model.active_pane == ActivePane::Details {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        )
        .wrap(Wrap { trim: false })
        .scroll((model.details_scroll, 0));

    f.render_widget(paragraph, main_chunks[1]);
    // render input
    let input_block_title = match model.input_mode {
        InputMode::Normal => "Filter (Press '/' to edit, Enter/Esc to stop)",
        InputMode::Editing => "Filter (Editing...)",
    };

    let width = chunks[1].width.max(3) - 3; // keep 2 for borders and 1 for cursor
    let scroll = model.input.visual_scroll(width as usize);
    let input = Paragraph::new(model.input.value())
        .style(match model.active_pane {
            ActivePane::Filter => Style::default().fg(Color::Yellow),
            _ => Style::default(),
        })
        .scroll((0, scroll as u16))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(input_block_title)
                .border_style(if model.active_pane == ActivePane::Filter {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
    f.render_widget(input, chunks[1]);
    match model.input_mode {
        InputMode::Normal =>
            // Hide the cursor. `Frame` does this by default, so we don't need to do anything here
            {}

        InputMode::Editing => {
            // Make the cursor visible and ask ratatui to put it at the specified coordinates after
            // rendering
            f.set_cursor_position((
                // Draft the area of the block
                chunks[1].x + ((model.input.visual_cursor().max(scroll) - scroll) as u16) + 1,
                chunks[1].y + 1,
            ))
        }
    }
}

fn highlight_json(json: &str) -> Text<'static> {
    let mut lines = Vec::new();
    for line in json.lines() {
        let mut spans = Vec::new();
        let mut remaining = line;

        while !remaining.is_empty() {
            if let Some(pos) = remaining.find('"') {
                // Add prefix before quotes
                let prefix = &remaining[..pos];
                if !prefix.is_empty() {
                    spans.push(Span::styled(
                        prefix.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                let rest = &remaining[pos + 1..];
                if let Some(end_pos) = rest.find('"') {
                    let quoted = &rest[..end_pos];
                    let is_key = rest[end_pos + 1..].trim_start().starts_with(':');

                    if is_key {
                        spans.push(Span::styled(
                            format!("\"{}\"", quoted),
                            Style::default().fg(Color::Cyan),
                        ));
                    } else {
                        spans.push(Span::styled(
                            format!("\"{}\"", quoted),
                            Style::default().fg(Color::Green),
                        ));
                    }
                    remaining = &rest[end_pos + 1..];
                } else {
                    spans.push(Span::styled(
                        remaining.to_string(),
                        Style::default().fg(Color::Green),
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
                        spans.push(Span::styled(
                            remaining_processed[..start_offset].to_string(),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }

                    if trimmed.is_empty() {
                        break;
                    }

                    // Find end of token (space, comma, brace, bracket, colon)
                    let token_end = trimmed
                        .find(|c: char| c.is_whitespace() || c == ',' || c == '}' || c == ']' || c == ':')
                        .map(|pos| if pos == 0 { 1 } else { pos })
                        .unwrap_or(trimmed.len());
                    let token = &trimmed[..token_end];
                    let rest = &trimmed[token_end..];

                    if token == "true" || token == "false" || token == "null" {
                        spans.push(Span::styled(token.to_string(), Style::default().fg(Color::Red)));
                    } else if token
                        .chars()
                        .all(|c| c.is_numeric() || c == '.' || c == '-')
                        && !token.is_empty()
                    {
                        spans.push(Span::styled(
                            token.to_string(),
                            Style::default().fg(Color::Magenta),
                        ));
                    } else if token == "{"
                        || token == "}"
                        || token == "["
                        || token == "]"
                        || token == ":"
                        || token == ","
                    {
                        spans.push(Span::styled(token.to_string(), Style::default().fg(Color::Gray)));
                    } else {
                        spans.push(Span::styled(
                            token.to_string(),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    remaining_processed = rest;
                }
                remaining = "";
            }
        }
        lines.push(Line::from(spans));
    }
    Text::from(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_matches() {
        let item = CbnItem::from_json(json!({
            "id": "test_id",
            "type": "MONSTER",
            "category": "creatures",
            "name": "Test Name"
        }));

        // Generic search
        assert!(item.matches("test"));
        assert!(item.matches("monster"));
        assert!(item.matches("creatures"));
        assert!(!item.matches("food"));

        // ID search
        assert!(item.matches("id:test_id"));
        assert!(item.matches("i:test"));
        assert!(!item.matches("id:monster"));

        // Type search
        assert!(item.matches("type:MONSTER"));
        assert!(item.matches("t:monster"));
        assert!(!item.matches("t:test"));

        // Category search
        assert!(item.matches("category:creatures"));
        assert!(item.matches("c:creatures"));
        assert!(!item.matches("c:monster"));

        // Combined search (AND logic)
        assert!(item.matches("t:monster c:creatures"));
        assert!(item.matches("i:test t:monster"));
        assert!(!item.matches("i:test t:item"));
    }

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
        let highlighted = highlight_json(json);
        // Basic check that we have lines and at least some spans
        assert!(!highlighted.lines.is_empty());
        
        let mut has_cyan = false;
        let mut has_green = false;
        let mut has_magenta = false;
        let mut has_red = false;
        
        for line in &highlighted.lines {
            for span in &line.spans {
                if span.style.fg == Some(Color::Cyan) { has_cyan = true; }
                if span.style.fg == Some(Color::Green) { has_green = true; }
                if span.style.fg == Some(Color::Magenta) { has_magenta = true; }
                if span.style.fg == Some(Color::Red) { has_red = true; }
            }
        }
        
        assert!(has_cyan, "Should have cyan for keys");
        assert!(has_green, "Should have green for strings");
        assert!(has_magenta, "Should have magenta for numbers");
        assert!(has_red, "Should have red for booleans");
    }

    #[test]
    fn test_sorting() {
        let mut items = vec![
            CbnItem::from_json(json!({"id": "z_id", "type": "A_TYPE"})),
            CbnItem::from_json(json!({"id": "a_id", "type": "B_TYPE"})),
            CbnItem::from_json(json!({"id": "a_id", "type": "A_TYPE"})),
        ];

        items.sort_by(|a, b| a.type_.cmp(&b.type_).then_with(|| a.id.cmp(&b.id)));

        assert_eq!(items[0].type_, "A_TYPE");
        assert_eq!(items[0].id, "a_id");
        assert_eq!(items[1].type_, "A_TYPE");
        assert_eq!(items[1].id, "z_id");
        assert_eq!(items[2].type_, "B_TYPE");
        assert_eq!(items[2].id, "a_id");
    }
}
