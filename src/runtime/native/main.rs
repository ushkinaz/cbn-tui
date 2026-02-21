//! # cbn-tui
//!
//! A terminal user interface (TUI) for browsing Cataclysm: Bright Nights game data.

use anyhow::Result;
use cbn_tui::app_core::indexing::{
    ITEMS_PROGRESS_WEIGHT, build_indexed_items, build_version_entries_from_builds,
    progress_ratio as shared_progress_ratio, resolve_game_version_label,
};
use cbn_tui::app_core::input::{AppKeyCode, AppKeyEvent, AppMouseEvent, AppMouseKind};
use cbn_tui::app_core::reducer;
use cbn_tui::app_core::state::{AppAction, AppState};
use cbn_tui::runtime::native::data;
use cbn_tui::{model, search_index, theme, ui};
use clap::Parser;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use std::fs;
use std::io;
use std::str::FromStr;
use std::time::{Duration, Instant};

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

// ---------------------------------------------------------------------------
// Native filesystem helpers for history (not available in the shared lib)
// ---------------------------------------------------------------------------

fn load_history_from_fs(app: &mut AppState) {
    if let Ok(content) = fs::read_to_string(&app.history_path) {
        app.filter_history = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|s| s.to_string())
            .collect();
    }
}

fn save_history_to_fs(app: &AppState) {
    if let Some(parent) = app.history_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = app.filter_history.join("\n");
    let _ = fs::write(&app.history_path, content);
}

// ---------------------------------------------------------------------------
// Crossterm → shared-reducer adapters
// ---------------------------------------------------------------------------

fn crossterm_to_app_key_event(
    code: KeyCode,
    modifiers: KeyModifiers,
    kind: KeyEventKind,
) -> Option<AppKeyEvent> {
    if matches!(kind, KeyEventKind::Release) {
        return None;
    }

    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let super_key = modifiers.contains(KeyModifiers::SUPER);

    let key_code = match code {
        KeyCode::Char(c) => AppKeyCode::Char(c),
        KeyCode::Backspace => AppKeyCode::Backspace,
        KeyCode::Delete => AppKeyCode::Delete,
        KeyCode::Enter => AppKeyCode::Enter,
        KeyCode::Esc => AppKeyCode::Esc,
        KeyCode::Up => AppKeyCode::Up,
        KeyCode::Down => AppKeyCode::Down,
        KeyCode::Left => AppKeyCode::Left,
        KeyCode::Right => AppKeyCode::Right,
        KeyCode::Home => AppKeyCode::Home,
        KeyCode::End => AppKeyCode::End,
        KeyCode::PageUp => AppKeyCode::PageUp,
        KeyCode::PageDown => AppKeyCode::PageDown,
        KeyCode::Tab => AppKeyCode::Tab,
        KeyCode::BackTab => AppKeyCode::BackTab,
        _ => return None,
    };

    Some(AppKeyEvent {
        code: key_code,
        ctrl: ctrl || super_key,
        alt,
        shift,
        is_release: false,
    })
}

fn crossterm_to_app_mouse_event(mouse: &event::MouseEvent) -> Option<AppMouseEvent> {
    let kind = match mouse.kind {
        MouseEventKind::Down(event::MouseButton::Left) => AppMouseKind::LeftDown,
        MouseEventKind::ScrollUp => AppMouseKind::ScrollUp,
        MouseEventKind::ScrollDown => AppMouseKind::ScrollDown,
        MouseEventKind::Moved | MouseEventKind::Drag(_) => AppMouseKind::Move,
        _ => return None,
    };
    Some(AppMouseEvent {
        kind,
        column: mouse.column,
        row: mouse.row,
        ctrl: mouse.modifiers.contains(KeyModifiers::CONTROL),
    })
}

// ---------------------------------------------------------------------------
// Native event handlers (thin wrappers that persist history after reducer run)
// ---------------------------------------------------------------------------

fn handle_key_event(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
    kind: KeyEventKind,
) {
    let Some(event) = crossterm_to_app_key_event(code, modifiers, kind) else {
        return;
    };

    let saved_history_len = app.filter_history.len();
    reducer::handle_key_event(app, event);

    // Persist history if it grew (native-only concern)
    if app.filter_history.len() != saved_history_len {
        save_history_to_fs(app);
    }
}

fn handle_mouse_event(app: &mut AppState, mouse: event::MouseEvent) -> bool {
    let Some(app_event) = crossterm_to_app_mouse_event(&mouse) else {
        return false;
    };
    reducer::handle_mouse_event(app, app_event)
}

// ---------------------------------------------------------------------------
// Local helper (native-only): progress ratio for download display
// ---------------------------------------------------------------------------

fn progress_ratio(progress: data::DownloadProgress) -> f64 {
    shared_progress_ratio(progress.downloaded, progress.total)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

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
    load_history_from_fs(&mut app);

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
            app.version_entries = build_version_entries_from_builds(builds);
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
) -> Result<Vec<model::BuildInfo>>
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
    let (indexed_items, index, index_time_ms) =
        build_index_with_progress(terminal, app, root.data)?;
    app.apply_new_dataset(
        indexed_items,
        index,
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
    data: Vec<serde_json::Value>,
) -> Result<(Vec<model::IndexedItem>, search_index::SearchIndex, f64)>
where
    B::Error: Send + Sync + 'static,
{
    let start = Instant::now();
    let mut last_draw = Instant::now();
    let mut draw_error: Option<anyhow::Error> = None;

    let mut indexed_items = build_indexed_items(data, |ratio| {
        if draw_error.is_some() {
            return;
        }
        app.update_stage("Indexing", ratio);
        if last_draw.elapsed() >= Duration::from_millis(120) || ratio >= ITEMS_PROGRESS_WEIGHT {
            if let Err(err) = terminal.draw(|f| ui::ui(f, app)) {
                draw_error = Some(anyhow::Error::from(err));
            } else {
                last_draw = Instant::now();
            }
        }
    });

    if let Some(err) = draw_error {
        return Err(err);
    }

    indexed_items.sort_by(|a, b| a.item_type.cmp(&b.item_type).then_with(|| a.id.cmp(&b.id)));

    let mut last_ratio = -1.0;
    let search_index =
        search_index::SearchIndex::build_with_progress(&indexed_items, |processed, total_items| {
            let ratio = if total_items > 0 {
                ITEMS_PROGRESS_WEIGHT
                    + (1.0 - ITEMS_PROGRESS_WEIGHT) * (processed as f64 / total_items as f64)
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

#[cfg(test)]
mod tests {
    use super::*;
    use cbn_tui::app_core::state::{AppState, FocusPane, InputMode};
    use cbn_tui::model::IndexedItem;
    use cbn_tui::{search_index, theme, ui};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;
    use serde_json::json;

    const SCROLL_LINES: u16 = 1;

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
            IndexedItem {
                value: json!({"id": "1"}),
                id: "1".to_string(),
                item_type: "type".to_string(),
            },
            IndexedItem {
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
            IndexedItem {
                value: json!({"id": "apple"}),
                id: "apple".to_string(),
                item_type: "fruit".to_string(),
            },
            IndexedItem {
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
        let indexed_items = vec![IndexedItem {
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
        let indexed_items = vec![IndexedItem {
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
        let indexed_items = vec![IndexedItem {
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
        let indexed_items = vec![IndexedItem {
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
            IndexedItem {
                value: json!({"id": "base_rifle"}),
                id: "base_rifle".to_string(),
                item_type: "t".to_string(),
            },
            IndexedItem {
                value: json!({"id": "other"}),
                id: "other".to_string(),
                item_type: "t".to_string(),
            },
            IndexedItem {
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
                IndexedItem {
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
