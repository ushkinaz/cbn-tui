use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
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
    #[arg(short, long, default_value = "data/all.json")]
    file: String,
}

#[derive(Debug, Deserialize)]
struct Root {
    data: Vec<Value>,
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
        let id = v.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let abstract_ = v.get("abstract").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = v.get("name").and_then(|v| {
            if v.is_object() {
                v.get("str").and_then(|s| s.as_str())
            } else {
                v.as_str()
            }
        }).unwrap_or("").to_string();
        
        let type_ = v.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let category = v.get("category").and_then(|v| v.as_str()).unwrap_or("").to_string();

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
                 self.id.to_lowercase().contains(&part_lower) ||
                 self.type_.to_lowercase().contains(&part_lower) ||
                 self.abstract_.to_lowercase().contains(&part_lower) ||
                 self.category.to_lowercase().contains(&part_lower)
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

struct App {
    items: Vec<CbnItem>,
    filtered_items: Vec<usize>, // Indices into self.items
    list_state: ListState,
    input: Input,
    input_mode: InputMode,
    should_quit: bool,
}

impl App {
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
            should_quit: false,
        }
    }

    fn update_filter(&mut self) {
        self.filtered_items = self.items
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
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Load Data
    println!("Loading data from {}...", args.file);
    let content = fs::read_to_string(&args.file)?;
    let root: Root = serde_json::from_str(&content)?;
    let items: Vec<CbnItem> = root.data.into_iter().map(CbnItem::from_json).collect();
    
    // 2. Setup Terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 3. Run Loop
    let mut app = App::new(items);
    let res = run_app(&mut terminal, &mut app);

    // 4. Teardown
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => app.should_quit = true,
                        KeyCode::Char('/') => {
                            app.input_mode = InputMode::Editing;
                        }
                        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                        KeyCode::Up | KeyCode::Char('k') => app.select_previous(),
                        _ => {}
                    },
                    InputMode::Editing => match key.code {
                        KeyCode::Enter => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        _ => {
                            app.input.handle_event(&Event::Key(key));
                            app.update_filter();
                        }
                    },
                }
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3), // Main View
            Constraint::Length(3), // Filter Input
        ])
        .split(f.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(70),
        ])
        .split(chunks[0]);

    // render list
    let list_items: Vec<ListItem> = app.filtered_items
        .iter()
        .map(|&idx| {
            let item = &app.items[idx];
            // Format: [TYPE] DisplayName
            let content = format!("[{}] {}", item.type_, item.display_name);
            ListItem::new(content)
        })
        .collect();

    let list = List::new(list_items)
        .block(Block::default().borders(Borders::ALL).title("Items"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, main_chunks[0], &mut app.list_state);

    // render details
    let details_text = if let Some(idx) = app.list_state.selected() {
        if idx < app.filtered_items.len() {
            let real_idx = app.filtered_items[idx];
            let item = &app.items[real_idx];
            match serde_json::to_string_pretty(&item.original_json) {
                Ok(s) => s,
                Err(_) => "Error parsing JSON".to_string(),
            }
        } else {
            "No item selected".to_string()
        }
    } else {
        "No item selected".to_string()
    };

    let paragraph = Paragraph::new(details_text)
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .wrap(Wrap { trim: false });
    
    f.render_widget(paragraph, main_chunks[1]);

    // render input
    let input_block_title = match app.input_mode {
        InputMode::Normal => "Filter (Press '/' to edit, Enter/Esc to stop)",
        InputMode::Editing => "Filter (Editing...)",
    };
    
    let width = chunks[1].width.max(3) - 3; // keep 2 for borders and 1 for cursor
    let scroll = app.input.visual_scroll(width as usize);
    let input = Paragraph::new(app.input.value())
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default().fg(Color::Yellow),
        })
        .scroll((0, scroll as u16))
        .block(Block::default().borders(Borders::ALL).title(input_block_title));
    f.render_widget(input, chunks[1]);
    match app.input_mode {
        InputMode::Normal =>
            // Hide the cursor. `Frame` does this by default, so we don't need to do anything here
            {},

        InputMode::Editing => {
            // Make the cursor visible and ask ratatui to put it at the specified coordinates after
            // rendering
            f.set_cursor_position((
                // Draft the area of the block
                chunks[1].x + ((app.input.visual_cursor().max(scroll) - scroll) as u16) + 1,
                chunks[1].y + 1,
            ))
        }
    }
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
}
