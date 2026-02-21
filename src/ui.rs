use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect, Size},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, LineGauge, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};
use serde_json::Value;
use std::rc::Rc;
use tui_scrollview::{ScrollView, ScrollbarVisibility};

use crate::theme;
use crate::{AppState, FocusPane, InputMode};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Semantic role of a span in the rendered JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonSpanKind {
    Key,          // e.g. "range"
    StringValue,  // e.g. "base_furniture"
    NumberValue,  // e.g. 60
    BooleanValue, // true / false / null
    Punctuation,  // { } [ ] , :
    Whitespace,   // indentation
}

#[derive(Debug, Clone)]
pub struct AnnotatedSpan {
    pub span: Span<'static>,
    pub kind: JsonSpanKind,
    /// The JSON key this value belongs to if the span is a value.
    /// For keys themselves this is the key's own text.
    pub key_context: Option<Rc<str>>,
    pub span_id: Option<usize>,
}

/// Main UI entry point that renders the entire application layout.
pub fn ui(f: &mut Frame, app: &mut AppState) {
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

    app.list_area = Some(main_chunks[0]);
    app.details_area = Some(main_chunks[1]);
    app.details_content_area = compute_details_content_area(app, main_chunks[1]);
    app.filter_area = Some(chunks[1]);

    // Render item list
    render_item_list(f, app, main_chunks[0]);

    // Render details pane
    render_details(f, app, main_chunks[1]);

    // Render filter input
    render_filter(f, app, chunks[1]);

    // Render status bar
    render_status_bar(f, app, chunks[2]);

    if app.show_progress {
        render_progress_modal(f, app);
    } else if app.show_version_picker {
        render_version_picker(f, app);
    } else if app.show_help {
        render_help_overlay(f, app);
    }
}

fn compute_details_content_area(app: &AppState, area: Rect) -> Option<Rect> {
    let inner_area = area.inner(Margin::new(1, 1));
    if inner_area.width == 0 || inner_area.height == 0 {
        return None;
    }

    let constraints = if app.get_selected_item().is_some() {
        vec![
            Constraint::Length(2), // Metadata header
            Constraint::Length(1), // Separator
            Constraint::Min(0),    // Content
        ]
    } else {
        vec![Constraint::Min(0)]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner_area);

    let content_area = if app.get_selected_item().is_some() {
        chunks[2]
    } else {
        chunks[0]
    };

    if content_area.width > 0 && content_area.height > 0 {
        Some(content_area)
    } else {
        None
    }
}

/// Renders the scrollable list of game items.
fn render_item_list(f: &mut Frame, app: &mut AppState, area: Rect) {
    // Borrow pre-computed display strings — no JSON traversal or String allocation per frame.
    let items: Vec<ListItem> = app
        .cached_display
        .iter()
        .map(|(display, type_prefix)| {
            let type_label = Line::from(vec![
                Span::styled(type_prefix.as_str(), app.theme.title),
                Span::raw(display.as_str()),
            ]);
            ListItem::new(type_label)
        })
        .collect();

    let is_focused = app.focused_pane == FocusPane::List;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if is_focused {
            app.theme.border_selected
        } else {
            app.theme.border
        })
        .title_style(app.theme.title)
        .title(format!(" Objects ({}) ", app.filtered_indices.len()))
        .title_bottom(if is_focused {
            Line::from(" ↑/↓ move • Tab cycle ").right_aligned()
        } else {
            Line::from("").right_aligned()
        })
        .title_alignment(Alignment::Left)
        .style(app.theme.list_normal);

    app.list_content_area = Some(block.inner(area));

    let list = List::new(items)
        .block(block)
        .style(app.theme.list_normal)
        .scroll_padding(2)
        .highlight_style(app.theme.list_selected);

    f.render_stateful_widget(list, area, &mut app.list_state);

    // Render scrollbar
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    let mut scrollbar_state = ScrollbarState::new(app.filtered_indices.len())
        .position(app.list_state.selected().unwrap_or(0));

    f.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

/// Renders the details pane showing syntax-highlighted JSON data.
fn render_details(f: &mut Frame, app: &mut AppState, area: Rect) {
    let is_focused = app.focused_pane == FocusPane::Details;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if is_focused {
            app.theme.border_selected
        } else {
            app.theme.border
        })
        .style(app.theme.text)
        .title(" JSON ")
        .title_alignment(Alignment::Left)
        .title_style(app.theme.title)
        .title_bottom(if is_focused {
            Line::from(" ↑/↓ scroll • Tab cycle • Esc back ").right_aligned()
        } else {
            Line::from("").right_aligned()
        });

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    if inner_area.width > 0 && inner_area.height > 0 {
        let horizontal_padding = 1;
        let mut content_area = inner_area;

        let header_height = render_metadata_header(f, app, inner_area);

        if header_height > 0 {
            // Render a horizontal separator line that merges with borders
            let separator_y = inner_area.y + header_height;
            if separator_y < area.y + area.height - 1 {
                let border_style = app.theme.border;
                let separator_line = app.get_separator(inner_area.width);
                f.render_widget(
                    Paragraph::new(separator_line).style(border_style),
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
            // Re-wrap if width changed
            if app.details_wrapped_width != content_width {
                app.details_wrapped_annotated =
                    wrap_annotated_lines(&app.details_annotated, content_width);
                app.details_wrapped_width = content_width;
            }

            let content_height = app.details_wrapped_annotated.len() as u16;

            let mut scroll_view = ScrollView::new(Size::new(content_width, content_height))
                .vertical_scrollbar_visibility(ScrollbarVisibility::Automatic)
                .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);

            // Match the background of the scroll view buffer to the theme
            let scroll_area = scroll_view.area();
            scroll_view.buf_mut().set_style(scroll_area, app.theme.text);

            let content_rect = Rect::new(0, 0, content_width, content_height);
            let text = annotated_to_text(&app.details_wrapped_annotated, app.hovered_span_id);
            scroll_view.render_widget(Paragraph::new(text).style(app.theme.text), content_rect);

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

/// Renders the metadata header (ID, Name, Type, Category) for the selected item.
/// Uses a two-column layout with 50% width each.
/// Returns the height occupied by the header (always 2).
fn render_metadata_header(f: &mut Frame, app: &mut AppState, area: Rect) -> u16 {
    let Some(item) = app.get_selected_item() else {
        return 0;
    };
    let json = &item.value;
    let id = &item.id;
    let type_ = &item.item_type;

    let id_val = if !id.is_empty() {
        id.as_str()
    } else {
        json.get("abstract").and_then(|v| v.as_str()).unwrap_or("")
    };
    let type_val = if !type_.is_empty() {
        type_.as_str()
    } else {
        json.get("type").and_then(|v| v.as_str()).unwrap_or("")
    };
    let name_val = json
        .get("name")
        .and_then(name_value)
        .or_else(|| fallback_display_name(json, id, type_))
        .unwrap_or_default();
    let cat_val = json.get("category").and_then(|v| v.as_str()).unwrap_or("");

    let id_val = if id_val.is_empty() { " " } else { id_val };
    let name_val = if name_val.is_empty() { " " } else { &name_val };
    let type_val = if type_val.is_empty() { " " } else { type_val };
    let cat_val = if cat_val.is_empty() { " " } else { cat_val };

    let horizontal_padding = 1;
    let header_area = Rect::new(
        area.x + horizontal_padding,
        area.y,
        area.width.saturating_sub(horizontal_padding * 2),
        2,
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(header_area);

    for (i, row_area) in rows.iter().enumerate() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(*row_area);

        if i == 0 {
            f.render_widget(Paragraph::new(id_val).style(app.theme.text), cols[0]);
            f.render_widget(Paragraph::new(name_val).style(app.theme.text), cols[1]);
        } else {
            f.render_widget(Paragraph::new(type_val).style(app.theme.text), cols[0]);
            f.render_widget(Paragraph::new(cat_val).style(app.theme.text), cols[1]);
        }
    }

    2 // height
}

/// Renders the interactive filter input box.
fn render_filter(f: &mut Frame, app: &mut AppState, area: Rect) {
    let is_focused = app.focused_pane == FocusPane::Filter;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if is_focused {
            app.theme.border_selected
        } else {
            app.theme.border
        })
        .title(" Filter (/) ")
        .title_style(app.theme.title)
        .title_bottom(if is_focused {
            Line::from(" ↑/↓ history • Tab cycle • Esc clear ").right_aligned()
        } else {
            Line::from("")
        });

    let inner = block.inner(area);
    app.filter_input_area = Some(inner);
    let horizontal_scroll =
        filter_horizontal_scroll(&app.filter_text, app.filter_cursor, inner.width);

    let content = if app.filter_text.is_empty() && app.input_mode != InputMode::Filtering {
        Text::from(Line::from(Span::styled(
            "t:gun ammo:rpg",
            app.theme.text.add_modifier(Modifier::DIM).italic(),
        )))
    } else {
        Text::from(app.filter_text.as_str())
    };

    let paragraph = Paragraph::new(content)
        .block(block)
        .style(app.theme.text)
        .scroll((0, horizontal_scroll));

    f.render_widget(paragraph, area);

    if app.input_mode == InputMode::Filtering && inner.width > 0 && inner.height > 0 {
        let cursor_offset = filter_cursor_offset(&app.filter_text, app.filter_cursor);
        let max_x = inner.width.saturating_sub(1);
        let visible_cursor_offset = cursor_offset.saturating_sub(horizontal_scroll);
        let cursor_x = inner.x + visible_cursor_offset.min(max_x);
        let cursor_y = inner.y;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Renders the multisection status bar at the bottom.
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
        Span::styled("Ctrl+G ", key_style),
        Span::raw("versions  "),
        Span::styled("? ", key_style),
        Span::raw("help  "),
        Span::styled("q ", key_style),
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
    let mut spans = vec![Span::raw(format!(
        "Objects: {}",
        app.total_items
    ))];
    if !app.source_warnings.is_empty() {
        spans.push(Span::raw(" |"));
        spans.push(Span::styled(
            " *",
            Style::default()
                .fg(ratatui::style::Color::Red)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let status = Line::from(spans);

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
        "Game: {}",
        app.game_version
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
        .style(app.theme.text)
        .title(" Help ")
        .border_type(ratatui::widgets::BorderType::Double)
        .title_style(app.theme.title);

    let inner_area = block.inner(popup_rect);
    f.render_widget(block, popup_rect);

    let key_style = app.theme.title;
    let desc_style = app.theme.text;
    let header_style = key_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    let format_section = |title: &str, items: Vec<(&str, &str)>| -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(Span::styled(title.to_string(), header_style))];
        for (key, desc) in items {
            lines.push(Line::from(vec![
                Span::styled(format!("{: <18}", key), key_style),
                Span::styled(desc.to_string(), desc_style),
            ]));
        }
        lines
    };

    let nav_lines = format_section(
        "Navigation",
        vec![
            ("/", "filter items"),
            ("Mouse Click", "filter by property"),
            ("Ctrl+Click", "jump to ID"),
            ("Ctrl+R", "reload local source"),
            ("Ctrl+G", "version switcher"),
            ("q", "quit"),
        ],
    );
    let nav_height = nav_lines.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(nav_height), // Dynamic height based on items
            Constraint::Length(1),          // Spacer
            Constraint::Min(0),             // Search Syntax / Input
        ])
        .margin(1)
        .split(inner_area);

    f.render_widget(Paragraph::new(nav_lines), chunks[0]);

    let mut combined_lines = format_section(
        "Filter",
        vec![
            ("Up | Down", "history"),
            ("Ctrl+U", "clear filter"),
            ("Ctrl+W", "delete word"),
            ("Ctrl+A | E", "start | end of line"),
        ],
    );

    combined_lines.push(Line::from(""));
    combined_lines.extend(format_section(
        "Search Syntax",
        vec![
            ("zombie", "- generic search in all fields"),
            ("t:gun", "- filter by type (i:id, t:type, c:cat)"),
            ("bash.str_min:30", "- filter by nested field"),
            ("'shot'", "- exact match"),
            ("zombie mom", "- AND logic"),
        ],
    ));

    combined_lines.push(Line::from(""));
    combined_lines.push(Line::from(vec![
        Span::styled("Example: ", key_style.add_modifier(Modifier::BOLD)),
        Span::styled("t:gun ammo:rpg", desc_style),
    ]));

    f.render_widget(Paragraph::new(combined_lines), chunks[2]);
}

fn render_version_picker(f: &mut Frame, app: &mut AppState) {
    let area = f.area();
    let popup_width = area.width.min(64).saturating_sub(4);
    let popup_height = area.height.min(18).saturating_sub(2);
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
        .style(app.theme.text)
        .title(" Game Versions ")
        .title_style(app.theme.title);

    let inner_area = block.inner(popup_rect);
    f.render_widget(block, popup_rect);

    let items: Vec<ListItem> = app
        .version_entries
        .iter()
        .map(|entry| {
            let mut spans = vec![Span::styled(&entry.label, app.theme.text)];
            if let Some(detail) = &entry.detail {
                spans.push(Span::styled(
                    format!(" ({})", detail),
                    app.theme.text.add_modifier(Modifier::DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default())
        .style(app.theme.list_normal)
        .highlight_style(app.theme.list_selected);

    f.render_stateful_widget(list, inner_area, &mut app.version_list_state);
}

fn render_progress_modal(f: &mut Frame, app: &mut AppState) {
    let area = f.area();
    let stages_len = app.progress_stages.len().max(1) as u16;
    let popup_width = area.width.min(68).saturating_sub(4);
    let popup_height = area.height.saturating_sub(2).min(stages_len + 4);
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
        .style(app.theme.text)
        .title(format!(" {} ", app.progress_title))
        .title_style(app.theme.title);

    let inner_area = block.inner(popup_rect);
    f.render_widget(block, popup_rect);

    let padding_x = 1;
    let padding_y = 1;
    let content_area = Rect::new(
        inner_area.x + padding_x,
        inner_area.y + padding_y,
        inner_area.width.saturating_sub(padding_x * 2),
        inner_area.height.saturating_sub(padding_y * 2),
    );
    if content_area.width == 0 || content_area.height == 0 {
        return;
    }

    let labels: Vec<String> = app
        .progress_stages
        .iter()
        .map(|stage| stage.label.clone())
        .collect();
    let mut label_width = labels.iter().map(|label| label.width()).max().unwrap_or(0) as u16;
    let min_gauge_width = 10u16;
    let percent_width = 4u16;
    if content_area.width <= min_gauge_width {
        label_width = 0;
    } else {
        let max_label = content_area
            .width
            .saturating_sub(min_gauge_width + percent_width + 2);
        label_width = label_width.min(max_label);
    }
    let gap = if label_width > 0 { 1 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); stages_len as usize])
        .split(content_area);

    for (idx, area) in chunks.iter().enumerate() {
        let stage = app
            .progress_stages
            .get(idx)
            .cloned()
            .unwrap_or_else(|| crate::ProgressStage {
                label: "Working".to_string(),
                ratio: 0.0,
                done: false,
            });
        let ratio = stage.ratio.clamp(0.0, 1.0);
        let percent_label = format!("{:.0}%", ratio * 100.0);
        let row_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(label_width),
                Constraint::Length(gap),
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(percent_width),
            ])
            .split(*area);

        if label_width > 0 {
            let label = labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| "Working 0%".to_string());
            f.render_widget(Paragraph::new(label).style(app.theme.text), row_chunks[0]);
        }

        let gauge = LineGauge::default()
            .filled_style(app.theme.title)
            .unfilled_style(app.theme.border)
            .ratio(ratio)
            .label("");
        f.render_widget(gauge, row_chunks[2]);

        f.render_widget(
            Paragraph::new(percent_label)
                .style(app.theme.text)
                .alignment(Alignment::Right),
            row_chunks[4],
        );
    }
}

pub(crate) fn display_name_for_item(json: &Value, id: &str, type_: &str) -> String {
    if !id.is_empty() {
        return id.to_string();
    }

    if let Some(abstract_) = json.get("abstract").and_then(|v| v.as_str()) {
        return format!("(abs) {}", abstract_);
    }

    if let Some(name) = json.get("name").and_then(name_value)
        && !name.is_empty()
    {
        return name;
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
            if let Some(result) = json.get("result").and_then(|v| v.as_str())
                && !result.is_empty()
            {
                return Some(format!("result: {}", result));
            }
            None
        }
        "profession_item_substitutions" => {
            if let Some(trait_) = json.get("trait").and_then(|v| v.as_str())
                && !trait_.is_empty()
            {
                return Some(format!("trait: {}", trait_));
            }
            if let Some(item) = json.get("item").and_then(|v| v.as_str())
                && !item.is_empty()
            {
                return Some(format!("item: {}", item));
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
/// Converts a matrix of AnnotatedSpans into a ratatui Text object.
/// Takes a borrow so callers avoid an expensive clone of the full buffer.
pub fn annotated_to_text(
    annotated: &'_ [Vec<AnnotatedSpan>],
    hovered_span_id: Option<usize>,
) -> Text<'_> {
    Text::from(
        annotated
            .iter()
            .map(|line| {
                Line::from(
                    line.iter()
                        .map(|as_| {
                            let mut style = as_.span.style;
                            if hovered_span_id.is_some() && as_.span_id == hovered_span_id {
                                style = style.add_modifier(Modifier::UNDERLINED);
                            }
                            Span::styled(as_.span.content.as_ref(), style)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
    )
}

/// Wraps a matrix of AnnotatedSpans into lines that fit within the given width.
/// Performs simple character-level wrapping.
pub fn wrap_annotated_lines(lines: &[Vec<AnnotatedSpan>], width: u16) -> Vec<Vec<AnnotatedSpan>> {
    let mut wrapped = Vec::new();
    let width = width as usize;
    if width == 0 {
        return Vec::new();
    }

    for line in lines {
        if line.is_empty() {
            wrapped.push(Vec::new());
            continue;
        }

        let mut current_wrapped_line = Vec::new();
        let mut current_width = 0;

        for annotated in line {
            let mut content = &annotated.span.content[..];
            while !content.is_empty() {
                let remaining_width = width.saturating_sub(current_width);
                if remaining_width == 0 {
                    wrapped.push(current_wrapped_line);
                    current_wrapped_line = Vec::new();
                    current_width = 0;
                    continue;
                }

                let mut fit_len = 0;
                let mut fit_width = 0;
                for c in content.chars() {
                    let w = UnicodeWidthChar::width(c).unwrap_or(0);
                    if fit_width + w > remaining_width {
                        break;
                    }
                    fit_len += c.len_utf8();
                    fit_width += w;
                }

                if fit_len > 0 {
                    let part = &content[..fit_len];
                    current_wrapped_line.push(AnnotatedSpan {
                        span: Span::styled(part.to_string(), annotated.span.style),
                        kind: annotated.kind,
                        key_context: annotated.key_context.clone(),
                        span_id: annotated.span_id,
                    });
                    current_width += fit_width;
                    content = &content[fit_len..];
                } else {
                    // Even one character doesn't fit? This should only happen if the width is tiny.
                    // Push the current line and start a new one.
                    if !current_wrapped_line.is_empty() {
                        wrapped.push(current_wrapped_line);
                        current_wrapped_line = Vec::new();
                        current_width = 0;
                    } else {
                        // Width is so small not even one char fits. Force-fit one char to avoid infinite loop.
                        let first_char = content.chars().next().unwrap();
                        let first_len = first_char.len_utf8();
                        current_wrapped_line.push(AnnotatedSpan {
                            span: Span::styled(
                                content[..first_len].to_string(),
                                annotated.span.style,
                            ),
                            kind: annotated.kind,
                            key_context: annotated.key_context.clone(),
                            span_id: annotated.span_id,
                        });
                        wrapped.push(current_wrapped_line);
                        current_wrapped_line = Vec::new();
                        current_width = 0;
                        content = &content[first_len..];
                    }
                }
            }
        }
        if !current_wrapped_line.is_empty() {
            wrapped.push(current_wrapped_line);
        }
    }
    wrapped
}

#[derive(Debug, Default)]
struct JsonParserState {
    /// Each level holds the current key name at that nesting depth (None for array slots).
    stack: Vec<Option<String>>,
    /// Eagerly maintained dot-joined path cached as a Rc<str>.
    /// current_key() just clones this — O(1) with no heap allocation.
    /// Rebuilt (O(depth)) only on push/pop/update_key, which are rare vs. per-span calls.
    current_path_rc: Option<Rc<str>>,
    next_span_id: usize,
}

impl JsonParserState {
    fn new() -> Self {
        Self {
            stack: vec![None],
            current_path_rc: None,
            next_span_id: 1,
        }
    }

    fn next_id(&mut self) -> usize {
        let id = self.next_span_id;
        self.next_span_id += 1;
        id
    }

    /// Returns the cached context path. Cheap Rc clone — no allocation.
    fn current_key(&self) -> Option<Rc<str>> {
        self.current_path_rc.clone()
    }

    /// Rebuilds current_path_rc from the stack. Called only on structural changes.
    fn rebuild_path(&mut self) {
        let mut path = String::new();
        for entry in &self.stack {
            if let Some(k) = entry
                && !k.is_empty() {
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(k);
                }
        }
        self.current_path_rc = if path.is_empty() {
            None
        } else {
            Some(Rc::from(path.as_str()))
        };
    }

    fn update_key(&mut self, key: &str) {
        if let Some(top) = self.stack.last_mut() {
            *top = Some(key.to_string());
        }
        self.rebuild_path();
    }

    fn push_object(&mut self) {
        self.stack.push(None); // new object level; key will be set by update_key
    }

    fn push_array(&mut self) {
        self.stack.push(None); // arrays do not add a key for their items
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
            self.rebuild_path();
        }
    }
}

/// Refactored version of highlight_json that also returns semantic metadata for each span.
pub fn highlight_json_annotated(
    json: &str,
    json_style: &theme::JsonStyle,
) -> Vec<Vec<AnnotatedSpan>> {
    let mut lines = Vec::new();
    let mut state = JsonParserState::new();

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
                        spans.push(AnnotatedSpan {
                            span: Span::raw(prefix.to_string()),
                            kind: JsonSpanKind::StringValue,
                            key_context: state.current_key(),
                            span_id: Some(state.next_id()),
                        });
                    }
                    remaining = &remaining[pos + 1..];
                    continue;
                }

                // Add prefix before quotes
                let prefix = &remaining[..pos];
                if !prefix.is_empty() {
                    process_non_quoted(prefix, json_style, &mut spans, &mut state);
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

                    if is_key {
                        state.update_key(quoted);
                        spans.push(AnnotatedSpan {
                            span: Span::styled(
                                format!("\"{}\"", quoted),
                                Style::default()
                                    .fg(json_style.key)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            kind: JsonSpanKind::Key,
                            key_context: state.current_key(),
                            span_id: None,
                        });
                    } else {
                        spans.push(AnnotatedSpan {
                            span: Span::styled(
                                format!("\"{}\"", quoted),
                                Style::default().fg(json_style.string),
                            ),
                            kind: JsonSpanKind::StringValue,
                            key_context: state.current_key(),
                            span_id: Some(state.next_id()),
                        });
                    }
                    remaining = &rest[ep + 1..];
                } else {
                    spans.push(AnnotatedSpan {
                        span: Span::styled(
                            remaining.to_string(),
                            Style::default().fg(json_style.string),
                        ),
                        kind: JsonSpanKind::StringValue,
                        key_context: state.current_key(),
                        span_id: Some(state.next_id()),
                    });
                    remaining = "";
                }
            } else {
                process_non_quoted(remaining, json_style, &mut spans, &mut state);
                remaining = "";
            }
        }
        lines.push(spans);
    }

    lines
}

fn process_non_quoted(
    content: &str,
    json_style: &theme::JsonStyle,
    spans: &mut Vec<AnnotatedSpan>,
    state: &mut JsonParserState,
) {
    let mut remaining = content;
    while !remaining.is_empty() {
        let trimmed = remaining.trim_start();
        let start_offset = remaining.len() - trimmed.len();
        if start_offset > 0 {
            spans.push(AnnotatedSpan {
                span: Span::raw(remaining[..start_offset].to_string()),
                kind: JsonSpanKind::Whitespace,
                key_context: None,
                span_id: None,
            });
        }

        if trimmed.is_empty() {
            break;
        }

        let token_end = trimmed
            .find(|c: char| {
                c.is_whitespace()
                    || c == ','
                    || c == '}'
                    || c == ']'
                    || c == '{'
                    || c == '['
                    || c == ':'
            })
            .map(|pos| if pos == 0 { 1 } else { pos })
            .unwrap_or(trimmed.len());
        let token = &trimmed[..token_end];
        let rest = &trimmed[token_end..];

        let (styled, kind) = if token == "true" || token == "false" || token == "null" {
            (
                Span::styled(token.to_string(), Style::default().fg(json_style.boolean)),
                JsonSpanKind::BooleanValue,
            )
        } else if (token
            .chars()
            .all(|c| c.is_numeric() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+'))
            && !token.is_empty()
            && token.chars().any(|c| c.is_numeric())
        {
            (
                Span::styled(token.to_string(), Style::default().fg(json_style.number)),
                JsonSpanKind::NumberValue,
            )
        } else if token == ":"
            || token == ","
            || token == "{"
            || token == "}"
            || token == "["
            || token == "]"
        {
            if token == "{" {
                state.push_object();
            } else if token == "[" {
                state.push_array();
            } else if token == "}" || token == "]" {
                state.pop();
            }
            (Span::raw(token.to_string()), JsonSpanKind::Punctuation)
        } else {
            (
                Span::raw(token.to_string()),
                if token.trim().is_empty() {
                    JsonSpanKind::Whitespace
                } else {
                    JsonSpanKind::Punctuation
                },
            )
        };

        let key_context = match kind {
            JsonSpanKind::Key
            | JsonSpanKind::StringValue
            | JsonSpanKind::NumberValue
            | JsonSpanKind::BooleanValue => state.current_key(),
            _ => None,
        };

        let span_id = match kind {
            JsonSpanKind::StringValue | JsonSpanKind::NumberValue | JsonSpanKind::BooleanValue => {
                Some(state.next_id())
            }
            _ => None,
        };

        spans.push(AnnotatedSpan {
            span: styled,
            kind,
            key_context,
            span_id,
        });
        remaining = rest;
    }
}

/// Given a click at (column, row), resolves the annotated span under the cursor.
/// Returns None if the click is outside the details pane.
pub fn hit_test_details(app: &AppState, column: u16, row: u16) -> Option<&AnnotatedSpan> {
    let area = app.details_content_area?;
    let horizontal_padding = 1;

    // Strictly check bounds, excluding the horizontal gutters
    let content_x_start = area.x + horizontal_padding;
    let content_x_end = area.x + area.width - horizontal_padding;
    if column < content_x_start
        || column >= content_x_end
        || row < area.y
        || row >= area.y + area.height
    {
        return None;
    }

    // Translate screen global coordinates to details content area relative coordinates
    let rel_x = column.saturating_sub(content_x_start);
    // Ensure rel_y is within [0, area.height) relative to the content area
    let rel_y = row.saturating_sub(area.y);

    // Account for scroll offset
    let scroll_offset = app.details_scroll_state.offset();
    let content_y = (rel_y + scroll_offset.y) as usize;

    // Details pane now uses pre-wrapped lines
    if let Some(line) = app.details_wrapped_annotated.get(content_y) {
        let mut current_x = 0;
        for annotated in line {
            let span_width = annotated.span.width() as u16;
            if rel_x >= current_x && rel_x < current_x + span_width {
                return Some(annotated);
            }
            current_x += span_width;
        }
    }

    None
}

/// Calculates the terminal cell width offset for a given character index.
/// Uses `unicode-width` to correctly handle multibyte and multi-cell characters.
pub fn filter_cursor_offset(text: &str, cursor: usize) -> u16 {
    text.chars()
        .take(cursor)
        .filter_map(|c| c.width())
        .map(|w| w as u16)
        .sum::<u16>()
}

/// Calculates horizontal viewport offset so the cursor stays visible in the input.
fn filter_viewport_offset(text: &str, cursor: usize, visible_width: u16) -> u16 {
    if visible_width == 0 {
        return 0;
    }

    let cursor_offset = filter_cursor_offset(text, cursor);
    cursor_offset.saturating_sub(visible_width.saturating_sub(1))
}

pub fn filter_horizontal_scroll(text: &str, cursor: usize, visible_width: u16) -> u16 {
    filter_viewport_offset(text, cursor, visible_width)
}

pub fn filter_cursor_for_column(text: &str, target_column: u16) -> usize {
    let mut width = 0u16;
    for (idx, ch) in text.chars().enumerate() {
        let char_width = ch.width().unwrap_or(0) as u16;
        if width + char_width > target_column {
            return idx;
        }
        width += char_width;
    }
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotated_spans_key_value_pair() {
        let json_str = r#"  "range": 60"#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        assert_eq!(annotated.len(), 1);
        let line = &annotated[0];

        // Whitespace, Key("range"), Punctuation(:), Whitespace, NumberValue(60)
        assert_eq!(line.len(), 5);
        assert_eq!(line[0].kind, JsonSpanKind::Whitespace);
        assert_eq!(line[1].kind, JsonSpanKind::Key);
        assert_eq!(line[1].span.content, "\"range\"");
        assert_eq!(line[2].kind, JsonSpanKind::Punctuation);
        assert_eq!(line[2].span.content, ":");
        assert_eq!(line[4].kind, JsonSpanKind::NumberValue);
        assert_eq!(line[4].span.content, "60");
        assert_eq!(line[4].key_context, Some(Rc::from("range")));
    }

    #[test]
    fn test_annotated_spans_string_value() {
        let json_str = r#""copy-from": "base_rifle""#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        // Key, Punctuation, Whitespace, StringValue
        assert_eq!(line[0].kind, JsonSpanKind::Key);
        assert_eq!(line[3].kind, JsonSpanKind::StringValue);
        assert_eq!(line[3].span.content, "\"base_rifle\"");
        assert_eq!(line[3].key_context, Some(Rc::from("copy-from")));
    }

    #[test]
    fn test_annotated_spans_boolean() {
        let json_str = r#""active": true"#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        assert_eq!(line[3].kind, JsonSpanKind::BooleanValue);
        assert_eq!(line[3].span.content, "true");
        assert_eq!(line[3].key_context, Some(Rc::from("active")));
    }

    #[test]
    fn test_annotated_spans_nested_object() {
        let json_str = "{ \"outer\": { \"inner\": 1 } }";
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        // Find "inner" key and "1" value
        let inner_key = line.iter().find(|s| s.span.content == "\"inner\"").unwrap();
        let one_value = line.iter().find(|s| s.span.content == "1").unwrap();

        assert_eq!(inner_key.kind, JsonSpanKind::Key);
        assert_eq!(one_value.kind, JsonSpanKind::NumberValue);
        assert_eq!(one_value.key_context, Some(Rc::from("outer.inner")));
    }

    #[test]
    fn test_annotated_spans_array() {
        let json_str = r#""tags": ["a", "b"]"#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        let val_a = line.iter().find(|s| s.span.content == "\"a\"").unwrap();
        let val_b = line.iter().find(|s| s.span.content == "\"b\"").unwrap();

        assert_eq!(val_a.kind, JsonSpanKind::StringValue);
        assert_eq!(val_a.key_context, Some(Rc::from("tags")));
        assert_eq!(val_b.kind, JsonSpanKind::StringValue);
        assert_eq!(val_b.key_context, Some(Rc::from("tags")));
    }

    #[test]
    fn test_to_text_preserves_rendering() {
        let json_str = r#"{"id": "test", "num": 123}"#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);
        let text = annotated_to_text(&annotated, None);

        // Verification: ensure it still has some styled spans
        let mut has_styles = false;
        for line in &text.lines {
            for span in &line.spans {
                if span.style != Style::default() {
                    has_styles = true;
                }
            }
        }
        assert!(has_styles);
    }

    #[test]
    fn test_annotated_spans_escaped_quotes() {
        let json_str = r#""text": "he said \"hello\"""#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        let val = line
            .iter()
            .find(|s| s.kind == JsonSpanKind::StringValue && s.span.content.contains("hello"))
            .unwrap();
        assert_eq!(val.span.content, "\"he said \\\"hello\\\"\"");
    }

    #[test]
    fn test_annotated_spans_nested_array_context() {
        let json_str = r#"{ "arr": [{"id": 1}, "x"] }"#;
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let mut val_1 = None;
        let mut val_x = None;

        for line in &annotated {
            for span in line {
                if span.span.content == "1" {
                    val_1 = Some(span.clone());
                }
                if span.span.content == "\"x\"" {
                    val_x = Some(span.clone());
                }
            }
        }

        let val_1 = val_1.unwrap();
        let val_x = val_x.unwrap();

        assert_eq!(val_1.key_context, Some(Rc::from("arr.id")));
        assert_eq!(val_x.key_context, Some(Rc::from("arr")));
    }

    #[test]
    fn test_hit_test_outside_area_returns_none() {
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(r#"{"id": 1}"#, &style);
        let wrapped = wrap_annotated_lines(&annotated, 80);

        let mut app = create_test_app();
        app.details_wrapped_annotated = wrapped;
        app.details_content_area = Some(Rect::new(10, 10, 40, 10));

        // The outside area (above)
        assert!(hit_test_details(&app, 15, 5).is_none());
        // The outside area (left)
        assert!(hit_test_details(&app, 5, 15).is_none());
        // In gutter (horizontal padding = 1)
        assert!(hit_test_details(&app, 10, 15).is_none());
    }

    #[test]
    fn test_hit_test_on_key_span() {
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(r#""id": 1"#, &style);
        let wrapped = wrap_annotated_lines(&annotated, 80);

        let mut app = create_test_app();
        app.details_wrapped_annotated = wrapped;
        app.details_content_area = Some(Rect::new(0, 0, 80, 20));

        // Click on "id" (starts at x=1 because of horizontal padding)
        let span = hit_test_details(&app, 2, 0).unwrap();
        assert_eq!(span.kind, JsonSpanKind::Key);
        assert_eq!(span.span.content, "\"id\"");
    }

    #[test]
    fn test_hit_test_on_value_span() {
        let style = theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(r#""id": 1"#, &style);
        let wrapped = wrap_annotated_lines(&annotated, 80);

        let mut app = create_test_app();
        app.details_wrapped_annotated = wrapped;
        app.details_content_area = Some(Rect::new(0, 0, 80, 20));

        // Click on "1"
        // "id": 1 is 4+2+1+1 = 8 chars
        // "i" is at x=2, "d" at x=3, ":" at x=5, " " at x=6, "1" at x=7
        let span = hit_test_details(&app, 7, 0).unwrap();
        assert_eq!(span.kind, JsonSpanKind::NumberValue);
        assert_eq!(span.span.content, "1");
        assert_eq!(span.key_context, Some(Rc::from("id")));
    }

    fn create_test_app() -> AppState {
        use serde_json::json;
        let indexed_items = vec![crate::data::IndexedItem {
            value: json!({"id": "1"}),
            id: "1".to_string(),
            item_type: "t".to_string(),
        }];
        let search_index = crate::search_index::SearchIndex::build(&indexed_items);
        let theme = theme::Theme::Dracula.config();
        AppState::new(
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
        )
    }

    #[test]
    fn test_filter_viewport_offset_keeps_cursor_visible() {
        let text = "abcdefghijklmnopqrstuvwxyz";

        assert_eq!(filter_viewport_offset(text, 0, 10), 0);
        assert_eq!(filter_viewport_offset(text, 9, 10), 0);
        assert_eq!(filter_viewport_offset(text, 10, 10), 1);
        assert_eq!(filter_viewport_offset(text, 15, 10), 6);
    }

    #[test]
    fn test_filter_viewport_offset_handles_wide_characters() {
        let text = "🦀rust";

        assert_eq!(filter_viewport_offset(text, 1, 2), 1);
        assert_eq!(filter_viewport_offset(text, 2, 3), 1);
        assert_eq!(filter_viewport_offset(text, 5, 4), 3);
    }

    #[test]
    fn test_filter_cursor_for_column_clamps_to_end() {
        assert_eq!(filter_cursor_for_column("abc", 0), 0);
        assert_eq!(filter_cursor_for_column("abc", 2), 2);
        assert_eq!(filter_cursor_for_column("abc", 50), 3);
    }

    #[test]
    fn test_filter_cursor_for_column_handles_wide_characters() {
        assert_eq!(filter_cursor_for_column("🦀a", 0), 0);
        assert_eq!(filter_cursor_for_column("🦀a", 1), 0);
        assert_eq!(filter_cursor_for_column("🦀a", 2), 1);
        assert_eq!(filter_cursor_for_column("🦀a", 3), 2);
    }
}
