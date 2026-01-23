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

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the all.json file
    #[arg(short, long, default_value = "../_test/all.json")]
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
        
        // Simple case-insensitive contains check across keys
        let q = query.to_lowercase();
        self.id.to_lowercase().contains(&q) ||
        self.type_.to_lowercase().contains(&q) ||
        self.abstract_.to_lowercase().contains(&q) ||
        self.category.to_lowercase().contains(&q)
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
    input: String,
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
            input: String::new(),
            input_mode: InputMode::Normal,
            should_quit: false,
        }
    }

    fn update_filter(&mut self) {
        self.filtered_items = self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.matches(&self.input))
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
                        KeyCode::Char(c) => {
                            app.input.push(c);
                            app.update_filter();
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                            app.update_filter();
                        }
                        _ => {}
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
    
    let input = Paragraph::new(app.input.as_str())
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default().fg(Color::Yellow),
        })
        .block(Block::default().borders(Borders::ALL).title(input_block_title));
    f.render_widget(input, chunks[1]);
}
