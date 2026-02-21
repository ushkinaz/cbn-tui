//! Shared event reducer: pure-ish handlers for key and mouse events.
//!
//! Both the native and web runtimes call these functions after converting their
//! platform-specific events to [`AppKeyEvent`] / [`AppMouseEvent`].

use crate::app_core::input::{AppKeyCode, AppKeyEvent, AppMouseEvent, AppMouseKind};
use crate::app_core::state::{AppAction, AppState, FocusPane, InputMode};
use crate::ui;

/// Fields that should never trigger clickable navigation.
pub const EXCLUDED_FIELDS: &[&str] = &[
    "id",
    "abstract",
    "description",
    "name",
    "__filename",
    "//",
    "//2",
    "rows",
];

pub const SCROLL_LINES: u16 = 1;

/// Returns the pane that contains the given cell coordinates, if any.
pub fn pane_at(app: &AppState, column: u16, row: u16) -> Option<FocusPane> {
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

/// Handle a runtime-agnostic key event, mutating `app` in place.
///
/// May set `app.pending_action`; the runtime is responsible for acting on it
/// after this function returns.
pub fn handle_key_event(app: &mut AppState, event: AppKeyEvent) {
    fn apply_filter_edit(app: &mut AppState, edit: impl FnOnce(&mut AppState)) {
        edit(app);
        app.update_filter();
    }

    if event.is_release {
        return;
    }

    let code = event.code;
    let ctrl = event.ctrl;
    let alt = event.alt;
    let shift = event.shift;

    if ctrl && code == AppKeyCode::Char('g') {
        app.show_help = false;
        app.show_version_picker = false;
        app.focus_pane(FocusPane::List);
        app.history_index = None;
        app.pending_action = Some(AppAction::OpenVersionPicker);
        return;
    }

    if ctrl && code == AppKeyCode::Char('r') {
        if app.source_dir.is_some() {
            app.pending_action = Some(AppAction::ReloadSource);
        }
        return;
    }

    if code == AppKeyCode::Tab || code == AppKeyCode::BackTab {
        if code == AppKeyCode::BackTab || shift {
            app.focus_prev_pane();
        } else {
            app.focus_next_pane();
        }
        return;
    }

    if app.show_help {
        if matches!(code, AppKeyCode::Char('?') | AppKeyCode::Esc) {
            app.show_help = false;
        }
        return;
    }

    if app.show_version_picker {
        match code {
            AppKeyCode::Esc => app.show_version_picker = false,
            AppKeyCode::Up => app.version_list_state.select_previous(),
            AppKeyCode::Down => app.version_list_state.select_next(),
            AppKeyCode::Enter => {
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
            AppKeyCode::Char('q') => app.should_quit = true,
            AppKeyCode::Char('/') => app.focus_pane(FocusPane::Filter),
            AppKeyCode::Char('?') => app.show_help = true,
            AppKeyCode::Up if !ctrl => {
                if app.focused_pane == FocusPane::Details {
                    app.scroll_details_up();
                } else {
                    app.move_selection(-1);
                }
            }
            AppKeyCode::Down if !ctrl => {
                if app.focused_pane == FocusPane::Details {
                    app.scroll_details_down();
                } else {
                    app.move_selection(1);
                }
            }
            AppKeyCode::Home => {
                if app.focused_pane == FocusPane::Details {
                    app.details_scroll_state = tui_scrollview::ScrollViewState::default();
                } else {
                    app.list_state.select(Some(0));
                    app.refresh_details();
                }
            }
            AppKeyCode::End => {
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
            AppKeyCode::PageUp => {
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
            AppKeyCode::PageDown => {
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
            AppKeyCode::Char('r') if ctrl => {
                if app.source_dir.is_some() {
                    app.pending_action = Some(AppAction::ReloadSource);
                }
            }
            AppKeyCode::Char(c) if c.is_alphanumeric() && !ctrl && !alt => {
                app.focus_pane(FocusPane::Filter);
                app.filter_move_to_end();
                apply_filter_edit(app, |app| app.filter_add_char(c));
            }
            _ => {}
        },
        InputMode::Filtering => match code {
            AppKeyCode::Enter => {
                if !app.filter_text.trim().is_empty()
                    && app.filter_history.last() != Some(&app.filter_text)
                {
                    app.filter_history.push(app.filter_text.clone());
                    app.save_history();
                }
                app.history_index = None;
                app.focus_pane(FocusPane::List);
            }
            AppKeyCode::Esc => {
                app.history_index = None;
                app.focus_pane(FocusPane::List);
            }
            AppKeyCode::Char('u') if ctrl => {
                apply_filter_edit(app, AppState::filter_clear);
            }
            AppKeyCode::Char('w') if ctrl => {
                apply_filter_edit(app, AppState::filter_delete_word);
            }
            AppKeyCode::Char('a') if ctrl => {
                app.filter_move_to_start();
            }
            AppKeyCode::Char('e') if ctrl => {
                app.filter_move_to_end();
            }
            AppKeyCode::Char(c) if !ctrl => {
                app.history_index = None;
                apply_filter_edit(app, |app| app.filter_add_char(c));
            }
            AppKeyCode::Backspace => {
                app.history_index = None;
                apply_filter_edit(app, AppState::filter_backspace);
            }
            AppKeyCode::Delete => {
                app.history_index = None;
                apply_filter_edit(app, AppState::filter_delete);
            }
            AppKeyCode::Up => {
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
            AppKeyCode::Down => {
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
            AppKeyCode::Left => app.filter_move_cursor_left(),
            AppKeyCode::Right => app.filter_move_cursor_right(),
            AppKeyCode::Home => app.filter_move_to_start(),
            AppKeyCode::End => app.filter_move_to_end(),
            _ => {}
        },
    }
}

/// Handle a runtime-agnostic mouse event.
///
/// `event.column` and `event.row` must already be in terminal cell coordinates.
/// Returns `true` if the UI needs to be redrawn.
pub fn handle_mouse_event(app: &mut AppState, event: AppMouseEvent) -> bool {
    let column = event.column;
    let row = event.row;
    let hovered_pane = pane_at(app, column, row);
    let mut is_valid_target = false;
    let mut new_hover_id = None;
    let mut target_path = String::new();
    let mut target_id = None;

    if let Some(span) = ui::hit_test_details(app, column, row)
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

    if event.kind == AppMouseKind::Move && app.hovered_span_id != new_hover_id {
        app.hovered_span_id = new_hover_id;
        transitioned = true;
    }

    if matches!(
        event.kind,
        AppMouseKind::ScrollUp | AppMouseKind::ScrollDown
    ) {
        let scroll_down = event.kind == AppMouseKind::ScrollDown;
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

    if event.kind == AppMouseKind::LeftDown {
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
            && content_area.contains((column, row).into())
            && !app.filtered_indices.is_empty()
        {
            let list_row = row.saturating_sub(content_area.y) as usize;
            if list_row < content_area.height as usize {
                let top_index = app.list_state.offset();
                let clicked = (top_index + list_row).min(app.filtered_indices.len() - 1);
                if app.list_state.selected() != Some(clicked) {
                    app.list_state.select(Some(clicked));
                    app.refresh_details();
                    transitioned = true;
                }
            }
        }

        if hovered_pane == Some(FocusPane::Filter)
            && let Some(input_area) = app.filter_input_area
            && input_area.contains((column, row).into())
        {
            let horizontal_scroll =
                ui::filter_horizontal_scroll(&app.filter_text, app.filter_cursor, input_area.width);
            let local_x = column.saturating_sub(input_area.x);
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
            if event.ctrl {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::input::{AppKeyCode, AppKeyEvent, AppMouseEvent, AppMouseKind};
    use crate::app_core::state::{AppState, FocusPane, InputMode};
    use crate::model::IndexedItem;
    use crate::search_index::SearchIndex;
    use crate::theme;
    use ratatui::layout::Rect;
    use serde_json::json;

    fn make_key(code: AppKeyCode) -> AppKeyEvent {
        AppKeyEvent {
            code,
            ctrl: false,
            alt: false,
            shift: false,
            is_release: false,
        }
    }

    fn make_key_ctrl(code: AppKeyCode) -> AppKeyEvent {
        AppKeyEvent {
            code,
            ctrl: true,
            alt: false,
            shift: false,
            is_release: false,
        }
    }

    fn make_key_shift(code: AppKeyCode) -> AppKeyEvent {
        AppKeyEvent {
            code,
            ctrl: false,
            alt: false,
            shift: true,
            is_release: false,
        }
    }

    fn make_mouse(kind: AppMouseKind, column: u16, row: u16) -> AppMouseEvent {
        AppMouseEvent {
            kind,
            column,
            row,
            ctrl: false,
        }
    }

    fn make_test_app(items: usize) -> AppState {
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
        let search_index = SearchIndex::build(&indexed_items);
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
            std::path::PathBuf::from(""),
            None,
        )
    }

    fn make_app_with_items(items: Vec<IndexedItem>) -> AppState {
        let search_index = SearchIndex::build(&items);
        let count = items.len();
        AppState::new(
            items,
            search_index,
            theme::Theme::Dracula.config(),
            "v1".to_string(),
            "v1".to_string(),
            "v1".to_string(),
            false,
            count,
            0.0,
            std::path::PathBuf::from(""),
            None,
        )
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
        let mut app = make_app_with_items(indexed_items);

        assert_eq!(app.list_state.selected(), Some(0));
        handle_key_event(&mut app, make_key(AppKeyCode::Down));
        assert_eq!(app.list_state.selected(), Some(1));
        handle_key_event(&mut app, make_key(AppKeyCode::Up));
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
        let mut app = make_app_with_items(indexed_items);

        handle_key_event(&mut app, make_key(AppKeyCode::Char('/')));
        assert_eq!(app.input_mode, InputMode::Filtering);

        handle_key_event(&mut app, make_key(AppKeyCode::Char('a')));
        assert_eq!(app.filter_text, "a");
        assert_eq!(app.filtered_indices.len(), 2);

        handle_key_event(&mut app, make_key(AppKeyCode::Char('p')));
        assert_eq!(app.filter_text, "ap");
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.filtered_indices[0], 0);
    }

    #[test]
    fn test_handle_key_event_autofocus_filter() {
        let mut app = make_test_app(1);
        handle_key_event(&mut app, make_key(AppKeyCode::Char('t')));
        assert_eq!(app.input_mode, InputMode::Filtering);
        assert_eq!(app.filter_text, "t");
    }

    #[test]
    fn test_filter_history_in_memory() {
        let mut app = make_test_app(1);
        app.input_mode = InputMode::Filtering;
        app.filter_text = "test_query".to_string();
        handle_key_event(&mut app, make_key(AppKeyCode::Enter));
        assert_eq!(app.filter_history.len(), 1);
        assert_eq!(app.filter_history[0], "test_query");

        app.input_mode = InputMode::Filtering;
        app.filter_text = String::new();
        handle_key_event(&mut app, make_key(AppKeyCode::Up));
        assert_eq!(app.filter_text, "test_query");
    }

    #[test]
    fn test_focus_cycling() {
        let mut app = make_test_app(1);
        assert_eq!(app.focused_pane, FocusPane::List);

        handle_key_event(&mut app, make_key(AppKeyCode::Tab));
        assert_eq!(app.focused_pane, FocusPane::Details);

        handle_key_event(&mut app, make_key(AppKeyCode::Tab));
        assert_eq!(app.focused_pane, FocusPane::Filter);

        handle_key_event(&mut app, make_key(AppKeyCode::Tab));
        assert_eq!(app.focused_pane, FocusPane::List);

        handle_key_event(&mut app, make_key_shift(AppKeyCode::Tab));
        assert_eq!(app.focused_pane, FocusPane::Filter);

        handle_key_event(&mut app, make_key(AppKeyCode::BackTab));
        assert_eq!(app.focused_pane, FocusPane::Details);
    }

    #[test]
    fn test_context_aware_navigation() {
        let mut app = make_test_app(20);
        app.list_area = Some(Rect::new(0, 0, 20, 10));
        app.focused_pane = FocusPane::List;

        handle_key_event(&mut app, make_key(AppKeyCode::PageDown));
        assert_eq!(app.list_state.selected(), Some(10));

        handle_key_event(&mut app, make_key(AppKeyCode::Home));
        assert_eq!(app.list_state.selected(), Some(0));

        app.focused_pane = FocusPane::Details;
        assert_eq!(app.details_scroll_state.offset().y, 0);

        handle_key_event(&mut app, make_key(AppKeyCode::Down));
        assert_eq!(app.details_scroll_state.offset().y, 1);

        handle_key_event(&mut app, make_key(AppKeyCode::Home));
        assert_eq!(app.details_scroll_state.offset().y, 0);
    }

    #[test]
    fn test_input_shortcuts() {
        let mut app = make_test_app(1);
        app.focus_pane(FocusPane::Filter);
        app.filter_text = "hello world".to_string();
        app.filter_cursor = 11;

        handle_key_event(&mut app, make_key_ctrl(AppKeyCode::Char('a')));
        assert_eq!(app.filter_cursor, 0);

        handle_key_event(&mut app, make_key_ctrl(AppKeyCode::Char('e')));
        assert_eq!(app.filter_cursor, 11);

        handle_key_event(&mut app, make_key_ctrl(AppKeyCode::Char('w')));
        assert_eq!(app.filter_text, "hello ");
        assert_eq!(app.filter_cursor, 6);

        handle_key_event(&mut app, make_key_ctrl(AppKeyCode::Char('u')));
        assert_eq!(app.filter_text, "");
        assert_eq!(app.filter_cursor, 0);
    }

    #[test]
    fn test_esc_behavior() {
        let mut app = make_test_app(1);

        app.focus_pane(FocusPane::Filter);
        app.filter_text = "abc".to_string();
        handle_key_event(&mut app, make_key(AppKeyCode::Esc));
        assert_eq!(app.filter_text, "abc");
        assert_eq!(app.focused_pane, FocusPane::List);

        app.focus_pane(FocusPane::List);
        app.should_quit = false;
        handle_key_event(&mut app, make_key(AppKeyCode::Esc));
        assert!(!app.should_quit);
    }

    #[test]
    fn test_quit_behavior() {
        let mut app = make_test_app(1);

        app.focus_pane(FocusPane::List);
        app.should_quit = false;
        handle_key_event(&mut app, make_key(AppKeyCode::Char('q')));
        assert!(app.should_quit);

        app.focus_pane(FocusPane::Filter);
        app.filter_text = String::new();
        app.should_quit = false;
        handle_key_event(&mut app, make_key(AppKeyCode::Char('q')));
        assert!(!app.should_quit);
        assert_eq!(app.filter_text, "q");
    }

    #[test]
    fn test_handle_key_event_ignores_release() {
        let mut app = make_test_app(1);
        let release_event = AppKeyEvent {
            code: AppKeyCode::Char('a'),
            ctrl: false,
            alt: false,
            shift: false,
            is_release: true,
        };
        handle_key_event(&mut app, release_event);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.filter_text.is_empty());
    }

    #[test]
    fn test_mouse_click_list_selects_item_and_focuses_list() {
        let mut app = make_test_app(8);
        app.list_area = Some(Rect::new(0, 0, 20, 8));
        app.list_content_area = Some(Rect::new(1, 1, 18, 6));
        app.details_area = Some(Rect::new(20, 0, 40, 8));
        app.filter_area = Some(Rect::new(0, 8, 60, 3));

        let transitioned = handle_mouse_event(&mut app, make_mouse(AppMouseKind::LeftDown, 3, 3));

        assert!(transitioned);
        assert_eq!(app.focused_pane, FocusPane::List);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.list_state.selected(), Some(2));
    }

    #[test]
    fn test_mouse_click_filter_sets_caret_position() {
        let mut app = make_test_app(1);
        app.filter_text = "abcdef".to_string();
        app.filter_cursor = app.filter_text.chars().count();
        app.list_area = Some(Rect::new(0, 0, 20, 8));
        app.details_area = Some(Rect::new(20, 0, 40, 8));
        app.filter_area = Some(Rect::new(0, 8, 60, 3));
        app.filter_input_area = Some(Rect::new(1, 9, 58, 1));

        let transitioned = handle_mouse_event(&mut app, make_mouse(AppMouseKind::LeftDown, 3, 9));

        assert!(transitioned);
        assert_eq!(app.focused_pane, FocusPane::Filter);
        assert_eq!(app.input_mode, InputMode::Filtering);
        assert_eq!(app.filter_cursor, 2);
    }

    #[test]
    fn test_mouse_click_filter_past_end_clamps_to_end() {
        let mut app = make_test_app(1);
        app.filter_text = "abc".to_string();
        app.filter_cursor = 0;
        app.list_area = Some(Rect::new(0, 0, 20, 8));
        app.details_area = Some(Rect::new(20, 0, 40, 8));
        app.filter_area = Some(Rect::new(0, 8, 30, 3));
        app.filter_input_area = Some(Rect::new(1, 9, 28, 1));

        let transitioned = handle_mouse_event(&mut app, make_mouse(AppMouseKind::LeftDown, 20, 9));

        assert!(transitioned);
        assert_eq!(app.filter_cursor, app.filter_text.chars().count());
    }

    #[test]
    fn test_mouse_scroll_hovered_list_moves_by_constant() {
        let mut app = make_test_app(10);
        app.list_area = Some(Rect::new(0, 0, 20, 10));
        app.list_content_area = Some(Rect::new(1, 1, 18, 8));

        let transitioned = handle_mouse_event(&mut app, make_mouse(AppMouseKind::ScrollDown, 2, 2));

        assert!(transitioned);
        assert_eq!(app.list_state.selected(), Some(SCROLL_LINES as usize));
    }

    #[test]
    fn test_mouse_scroll_hovered_details_moves_by_constant() {
        let mut app = make_test_app(1);
        app.details_area = Some(Rect::new(20, 0, 40, 10));

        let transitioned =
            handle_mouse_event(&mut app, make_mouse(AppMouseKind::ScrollDown, 25, 1));

        assert!(transitioned);
        assert_eq!(app.details_scroll_state.offset().y, SCROLL_LINES);
    }

    #[test]
    fn test_mouse_click_details_focuses_without_link() {
        let mut app = make_test_app(1);
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = crate::ui::highlight_json_annotated(r#""id": 1"#, &style);
        app.details_wrapped_annotated = crate::ui::wrap_annotated_lines(&annotated, 20);
        app.details_area = Some(Rect::new(20, 0, 40, 10));
        app.details_content_area = Some(Rect::new(20, 0, 40, 10));
        app.filter_text = "x".to_string();
        app.filter_cursor = 1;
        app.focused_pane = FocusPane::List;
        app.input_mode = InputMode::Normal;

        let transitioned = handle_mouse_event(&mut app, make_mouse(AppMouseKind::LeftDown, 22, 0));

        assert!(transitioned);
        assert_eq!(app.focused_pane, FocusPane::Details);
        assert_eq!(app.filter_text, "x");
    }
}
