#![cfg_attr(
    not(target_arch = "wasm32"),
    allow(dead_code, unused_imports, unused_variables)
)]

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("cbn-web must be built for wasm32-unknown-unknown");
}

// ---------------------------------------------------------------------------
// Web target (wasm32) — everything below is only compiled for the browser.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
use anyhow::Result;
#[cfg(target_arch = "wasm32")]
use cbn_tui::app_core::indexing::{
    ITEMS_PROGRESS_WEIGHT, build_indexed_items, resolve_game_version_label,
};
#[cfg(target_arch = "wasm32")]
use cbn_tui::app_core::input::{AppKeyCode, AppKeyEvent, AppMouseEvent, AppMouseKind};
#[cfg(target_arch = "wasm32")]
use cbn_tui::app_core::reducer;
#[cfg(target_arch = "wasm32")]
use cbn_tui::app_core::state::{AppAction, AppState, VersionEntry};
#[cfg(target_arch = "wasm32")]
use cbn_tui::app_core::web_mouse::{PixelRect, mouse_pixels_to_cell};
#[cfg(target_arch = "wasm32")]
use cbn_tui::model::{IndexedItem, Root};
#[cfg(target_arch = "wasm32")]
use cbn_tui::runtime::web::data;
#[cfg(target_arch = "wasm32")]
use cbn_tui::search_index::SearchIndex;
#[cfg(target_arch = "wasm32")]
use cbn_tui::ui;
#[cfg(target_arch = "wasm32")]
use js_sys::Promise;
#[cfg(target_arch = "wasm32")]
use ratatui::Terminal;
#[cfg(target_arch = "wasm32")]
use ratzilla::web_sys::wasm_bindgen::{JsCast, JsValue};
#[cfg(target_arch = "wasm32")]
use ratzilla::{
    DomBackend, WebRenderer,
    event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
};
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::str::FromStr;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::{JsFuture, spawn_local};

// ---------------------------------------------------------------------------
// Ratzilla → shared input type adapters
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn ratzilla_to_app_key_event(key: KeyEvent) -> Option<AppKeyEvent> {
    let key_code = match key.code {
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
        KeyCode::Tab => {
            // Ratzilla sends Tab; shift-tab arrives as Tab with shift modifier
            if key.shift {
                AppKeyCode::BackTab
            } else {
                AppKeyCode::Tab
            }
        }
        _ => return None,
    };
    Some(AppKeyEvent {
        code: key_code,
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
        is_release: false,
    })
}

#[cfg(target_arch = "wasm32")]
fn ratzilla_to_app_mouse_event(column: u16, row: u16, mouse: &MouseEvent) -> AppMouseEvent {
    // Note: ratzilla's MouseEventKind only has Pressed/Released/Moved/Unidentified.
    // Scroll (wheel) events are not delivered through on_mouse_event in ratzilla 0.3.
    let kind = match mouse.event {
        MouseEventKind::Pressed if mouse.button == MouseButton::Left => AppMouseKind::LeftDown,
        MouseEventKind::Moved => AppMouseKind::Move,
        _ => AppMouseKind::Move,
    };
    AppMouseEvent {
        kind,
        column,
        row,
        ctrl: mouse.ctrl,
    }
}

// ---------------------------------------------------------------------------
// Web-specific helpers
// ---------------------------------------------------------------------------

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

/// Converts a ratzilla mouse pixel position to terminal cell coordinates.
/// This is the web-specific coordinate translation that lives here (not in the shared reducer).
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

    let cell_size = if cell_width > 0.0 && cell_height > 0.0 {
        Some((cell_width, cell_height))
    } else {
        None
    };

    mouse_pixels_to_cell(
        mouse.x as f64,
        mouse.y as f64,
        PixelRect {
            left: rect.left(),
            top: rect.top(),
            width: rect.width(),
            height: rect.height(),
        },
        cell_size,
        fallback_size,
    )
}

// ---------------------------------------------------------------------------
// Web event handlers (thin adapters over shared reducer)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn handle_key_event(app: &mut AppState, key_event: KeyEvent) {
    if let Some(event) = ratzilla_to_app_key_event(key_event) {
        reducer::handle_key_event(app, event);
    }
}

#[cfg(target_arch = "wasm32")]
fn handle_mouse_event(app: &mut AppState, mouse: MouseEvent) -> bool {
    let Some((column, row)) = mouse_to_cell_position(app, &mouse) else {
        // Mouse left the grid entirely: clear hover state
        if matches!(mouse.event, MouseEventKind::Moved) && app.hovered_span_id.is_some() {
            app.hovered_span_id = None;
            return true;
        }
        return false;
    };
    let app_event = ratzilla_to_app_mouse_event(column, row, &mouse);
    reducer::handle_mouse_event(app, app_event)
}

// ---------------------------------------------------------------------------
// Async indexing (web version: yields to browser between batches)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
async fn build_index_with_progress<F>(
    data: Vec<serde_json::Value>,
    mut on_progress: F,
) -> (Vec<IndexedItem>, SearchIndex, f64)
where
    F: FnMut(f64),
{
    let start_ms = now_ms();

    // Use the shared indexing helper for item construction (same batch cadence:
    // every 500 items). A single yield lets the browser breathe after this phase.
    let mut indexed_items = build_indexed_items(data, |ratio| on_progress(ratio));
    yield_to_browser().await;

    indexed_items.sort_by(|a, b| a.item_type.cmp(&b.item_type).then_with(|| a.id.cmp(&b.id)));
    on_progress(ITEMS_PROGRESS_WEIGHT);
    yield_to_browser().await;

    // Build search index with browser yields every 1000 items.
    let mut search_index = SearchIndex::new();
    let index_total = indexed_items.len();
    for (idx, item) in indexed_items.iter().enumerate() {
        search_index.index_item(idx, item);

        if idx % 250 == 0 || idx + 1 == index_total {
            let ratio = if index_total > 0 {
                ITEMS_PROGRESS_WEIGHT
                    + (1.0 - ITEMS_PROGRESS_WEIGHT) * ((idx + 1) as f64 / index_total as f64)
            } else {
                1.0
            };
            on_progress(ratio);
        }
        if idx % 1000 == 0 || idx + 1 == index_total {
            yield_to_browser().await;
        }
    }

    let index_time_ms = (now_ms() - start_ms).max(0.0);
    (indexed_items, search_index, index_time_ms)
}

// ---------------------------------------------------------------------------
// Version picker entries (web only shows stable/nightly + current if custom)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn build_version_entries_web(current: &str) -> Vec<VersionEntry> {
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

// ---------------------------------------------------------------------------
// Game data loading (async, web-specific)
// ---------------------------------------------------------------------------

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

    let root: Root = data::fetch_game_root(version).await?;

    {
        let mut app = app_state.borrow_mut();
        app.finish_stage("Downloading");
        app.finish_stage("Parsing");
        app.update_stage("Indexing", 0.01);
    }

    let game_version_label = resolve_game_version_label(version, None, &root);
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

// ---------------------------------------------------------------------------
// Action handling
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn handle_action(app_state: Rc<RefCell<AppState>>, action: AppAction) {
    match action {
        AppAction::OpenVersionPicker => {
            let mut app = app_state.borrow_mut();
            let current = app.game_version_key.clone();
            app.version_entries = build_version_entries_web(&current);
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

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn main() -> Result<()> {
    console_error_panic_hook::set_once();

    let app_version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let theme_name = "dracula";
    use cbn_tui::theme;
    let theme_enum = theme::Theme::from_str(theme_name).map_err(anyhow::Error::msg)?;
    let theme = theme_enum.config();

    let app = Rc::new(RefCell::new(AppState::new(
        Vec::new(),
        SearchIndex::new(),
        theme,
        "loading".to_string(),
        "nightly".to_string(),
        app_version,
        false,
        0,
        0.0,
        std::path::PathBuf::new(), // web has no filesystem history
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
