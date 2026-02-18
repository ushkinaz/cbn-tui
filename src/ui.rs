use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect, Size},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, LineGauge, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use serde_json::Value;
use tui_scrollview::{ScrollView, ScrollbarVisibility};

use crate::theme;
use crate::{AppState, InputMode};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Semantic role of a span in the rendered JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonSpanKind {
    Key,           // e.g. "range"
    StringValue,   // e.g. "base_furniture"
    NumberValue,   // e.g. 60
    BooleanValue,  // true / false / null
    Punctuation,   // { } [ ] , :
    Whitespace,    // indentation
}

#[derive(Debug, Clone)]
pub struct AnnotatedSpan {
    pub span: Span<'static>,
    pub kind: JsonSpanKind,
    /// The JSON key this value belongs to, if the span is a value.
    /// For keys themselves this is the key's own text.
    pub key_context: Option<String>,
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

/// Renders the scrollable list of game items.
fn render_item_list(f: &mut Frame, app: &mut AppState, area: Rect) {
    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .map(|&idx| {
            let (json, id, type_) = &app.indexed_items[idx];
            let display_name = display_name_for_item(json, id, type_);

            let type_label = Line::from(vec![
                Span::styled(format!("{} ", type_), app.theme.title),
                Span::raw(display_name),
            ]);
            ListItem::new(type_label)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border_selected)
        .title_style(app.theme.title)
        .title(format!(" Items ({}) ", app.filtered_indices.len()))
        .title_bottom(Line::from(" up / down ").right_aligned())
        .title_alignment(Alignment::Left)
        .style(app.theme.list_normal);

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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border.bg(app.theme.background))
        .style(app.theme.border.bg(app.theme.background))
        .title(" JSON ")
        .title_alignment(Alignment::Left)
        .title_style(app.theme.title)
        .title_bottom(Line::from(" pg-up / pg-down ").right_aligned());

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
            app.details_content_area = Some(content_area);
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

/// Renders the metadata header (ID, Name, Type, Category) for the selected item.
/// Uses a two-column layout with 50% width each.
/// Returns the height occupied by the header (always 2).
fn render_metadata_header(f: &mut Frame, app: &mut AppState, area: Rect) -> u16 {
    let Some((json, id, type_)) = app.get_selected_item() else {
        return 0;
    };

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
        Span::styled("/ ", key_style),
        Span::raw("filter  "),
        Span::styled("Ctrl+G ", key_style),
        Span::raw("versions  "),
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
            Constraint::Length(12), // Navigation
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
        ("Home", "selection to start"),
        ("End", "selection to end"),
        ("PgUp | Ctrl+k", "scroll JSON up"),
        ("PgDown | Ctrl+j", "scroll JSON down"),
        ("Ctrl+G", "version switcher"),
        ("?", "this help"),
        ("Esc", "back / quit"),
        ("q", "quit"),
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
        .style(app.theme.border_selected.bg(app.theme.background))
        .title(" Game Versions ")
        .title_style(app.theme.title)
        .title_bottom(Line::from(" enter select / esc close ").right_aligned());

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
        .style(app.theme.border_selected.bg(app.theme.background))
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

fn display_name_for_item(json: &Value, id: &str, type_: &str) -> String {
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
pub fn highlight_json(json: &str, json_style: &theme::JsonStyle) -> Text<'static> {
    let annotated = highlight_json_annotated(json, json_style);
    annotated_to_text(annotated)
}

/// Converts a matrix of AnnotatedSpans into a ratatui Text object.
pub fn annotated_to_text(annotated: Vec<Vec<AnnotatedSpan>>) -> Text<'static> {
    Text::from(
        annotated
            .into_iter()
            .map(|line| Line::from(line.into_iter().map(|as_| as_.span).collect::<Vec<_>>()))
            .collect::<Vec<_>>(),
    )
}

/// Refactored version of highlight_json that also returns semantic metadata for each span.
pub fn highlight_json_annotated(
    json: &str,
    json_style: &theme::JsonStyle,
) -> Vec<Vec<AnnotatedSpan>> {
    let mut lines = Vec::new();
    let mut current_key: Option<String> = None;

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
                            key_context: current_key.clone(),
                        });
                    }
                    remaining = &remaining[pos + 1..];
                    continue;
                }

                // Add prefix before quotes
                let prefix = &remaining[..pos];
                if !prefix.is_empty() {
                    process_non_quoted(prefix, json_style, &mut spans, &current_key);
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
                        current_key = Some(quoted.to_string());
                        spans.push(AnnotatedSpan {
                            span: Span::styled(
                                format!("\"{}\"", quoted),
                                Style::default()
                                    .fg(json_style.key)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            kind: JsonSpanKind::Key,
                            key_context: current_key.clone(),
                        });
                    } else {
                        spans.push(AnnotatedSpan {
                            span: Span::styled(
                                format!("\"{}\"", quoted),
                                Style::default().fg(json_style.string),
                            ),
                            kind: JsonSpanKind::StringValue,
                            key_context: current_key.clone(),
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
                        key_context: current_key.clone(),
                    });
                    remaining = "";
                }
            } else {
                process_non_quoted(remaining, json_style, &mut spans, &current_key);
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
    key_context: &Option<String>,
) {
    let mut remaining = content;
    while !remaining.is_empty() {
        let trimmed = remaining.trim_start();
        let start_offset = remaining.len() - trimmed.len();
        if start_offset > 0 {
            spans.push(AnnotatedSpan {
                span: Span::raw(remaining[..start_offset].to_string()),
                kind: JsonSpanKind::Whitespace,
                key_context: key_context.clone(),
            });
        }

        if trimmed.is_empty() {
            break;
        }

        let token_end = trimmed
            .find(|c: char| c.is_whitespace() || c == ',' || c == '}' || c == ']' || c == ':')
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
        } else if token == ":" || token == "," || token == "{" || token == "}" || token == "[" || token == "]" {
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

        spans.push(AnnotatedSpan {
            span: styled,
            kind,
            key_context: key_context.clone(),
        });
        remaining = rest;
    }
}

/// Given a click at (column, row), resolves the annotated span under the cursor.
/// Returns None if the click is outside the details pane.
pub fn hit_test_details(app: &AppState, column: u16, row: u16) -> Option<&AnnotatedSpan> {
    let area = app.details_content_area?;
    let horizontal_padding = 1;
    let content_width = area.width.saturating_sub(horizontal_padding * 2);

    if content_width == 0 {
        return None;
    }

    // Strictly check bounds, excluding the horizontal gutters
    let content_x_start = area.x + horizontal_padding;
    let content_x_end = content_x_start + content_width;
    if column < content_x_start || column >= content_x_end || row < area.y || row >= area.y + area.height {
        return None;
    }

    // Translate screen global coordinates to details content area relative coordinates
    let rel_x = column - content_x_start;
    let rel_y = row - area.y;

    // Account for scroll offset
    let scroll_offset = app.details_scroll_state.offset();
    let content_y = rel_y + scroll_offset.y;
    // Note: horizontal scroll is disabled in render_details (ScrollbarVisibility::Never)

    // Details pane uses wrapping. We need to find which original line AND which wrap-line was clicked.
    let mut current_wrapped_row = 0;
    for line_spans in &app.details_annotated {
        let line_width = line_spans
            .iter()
            .map(|s| s.span.width())
            .sum::<usize>() as u16;

        let wraps = if line_width == 0 {
            1
        } else {
            line_width.div_ceil(content_width)
        };

        if content_y >= current_wrapped_row && content_y < current_wrapped_row + wraps {
            // Click is on this original JSON line (possibly wrapped)
            let wrap_index = content_y - current_wrapped_row;
            let click_x_in_line = wrap_index * content_width + rel_x;

            let mut current_x = 0;
            for annotated in line_spans {
                let span_width = annotated.span.width() as u16;
                if click_x_in_line >= current_x && click_x_in_line < current_x + span_width {
                    return Some(annotated);
                }
                current_x += span_width;
            }
            return None;
        }
        current_wrapped_row += wraps;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotated_spans_key_value_pair() {
        let json_str = r#"  "range": 60"#;
        let style = crate::theme::Theme::Dracula.config().json_style;
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
        assert_eq!(line[4].key_context, Some("range".to_string()));
    }

    #[test]
    fn test_annotated_spans_string_value() {
        let json_str = r#""copy-from": "base_rifle""#;
        let style = crate::theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        // Key, Punctuation, Whitespace, StringValue
        assert_eq!(line[0].kind, JsonSpanKind::Key);
        assert_eq!(line[3].kind, JsonSpanKind::StringValue);
        assert_eq!(line[3].span.content, "\"base_rifle\"");
        assert_eq!(line[3].key_context, Some("copy-from".to_string()));
    }

    #[test]
    fn test_annotated_spans_boolean() {
        let json_str = r#""active": true"#;
        let style = crate::theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        assert_eq!(line[3].kind, JsonSpanKind::BooleanValue);
        assert_eq!(line[3].span.content, "true");
        assert_eq!(line[3].key_context, Some("active".to_string()));
    }

    #[test]
    fn test_annotated_spans_nested_object() {
        let json_str = "{ \"outer\": { \"inner\": 1 } }";
        let style = crate::theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        // Find "inner" key and "1" value
        let inner_key = line.iter().find(|s| s.span.content == "\"inner\"").unwrap();
        let one_value = line.iter().find(|s| s.span.content == "1").unwrap();

        assert_eq!(inner_key.kind, JsonSpanKind::Key);
        assert_eq!(one_value.kind, JsonSpanKind::NumberValue);
        assert_eq!(one_value.key_context, Some("inner".to_string()));
    }

    #[test]
    fn test_annotated_spans_array() {
        let json_str = r#""tags": ["a", "b"]"#;
        let style = crate::theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        let val_a = line.iter().find(|s| s.span.content == "\"a\"").unwrap();
        let val_b = line.iter().find(|s| s.span.content == "\"b\"").unwrap();

        assert_eq!(val_a.kind, JsonSpanKind::StringValue);
        assert_eq!(val_a.key_context, Some("tags".to_string()));
        assert_eq!(val_b.kind, JsonSpanKind::StringValue);
        assert_eq!(val_b.key_context, Some("tags".to_string()));
    }

    #[test]
    fn test_to_text_preserves_rendering() {
        let json_str = r#"{"id": "test", "num": 123}"#;
        let style = crate::theme::Theme::Dracula.config().json_style;
        let text = highlight_json(json_str, &style);

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
        let style = crate::theme::Theme::Dracula.config().json_style;
        let annotated = highlight_json_annotated(json_str, &style);

        let line = &annotated[0];
        let val = line.iter().find(|s| s.kind == JsonSpanKind::StringValue && s.span.content.contains("hello")).unwrap();
        assert_eq!(val.span.content, "\"he said \\\"hello\\\"\"");
    }
}
