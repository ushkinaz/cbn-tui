use anyhow::Result;
use clap::Parser;
use cursive::Cursive;
use cursive::align::HAlign;
use cursive::theme::{ColorStyle, PaletteColor};
use cursive::traits::*;
use cursive::utils::markup::StyledString;
use cursive::views::{
    EditView, LinearLayout, Panel, ResizedView, ScrollView, SelectView, TextView,
};
use serde::Deserialize;
use serde_json::Value;
use std::fs;

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

/// Application state stored in Cursive's user data.
/// Contains indexed items, search index, and current filtered subset.
struct AppState {
    /// All loaded items in indexed format (json, id, type)
    indexed_items: Vec<(Value, String, String)>,
    /// Search index for fast lookups
    search_index: search_index::SearchIndex,
    /// Indices into indexed_items that match the current filter
    filtered_indices: Vec<usize>,
    /// Track search version for debouncing
    search_version: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Highlighting style for JSON
    json_style: theme::JsonStyle,
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

    // Create Cursive app
    let mut siv = cursive::default();

    // Choose theme
    let theme_name = args.theme.as_deref().unwrap_or("dracula");
    let (theme, json_style) = match theme_name {
        "dracula" => theme::dracula_theme(),
        "solarized" => theme::solarized_dark(),
        "gruvbox" => theme::gruvbox_theme(),
        "everforest_light" => theme::everforest_light_theme(),
        _ => anyhow::bail!("Unknown theme: {}. Available: dracula, solarized, gruvbox, everforest_light", theme_name),
    };
    siv.set_theme(theme);

    // Initialize state with index
    let filtered_indices: Vec<usize> = (0..indexed_items.len()).collect();
    siv.set_user_data(AppState {
        indexed_items,
        search_index,
        filtered_indices,
        search_version: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        json_style,
    });

    // Build UI
    build_ui(&mut siv);

    // Add global keybindings
    siv.add_global_callback('q', |s| s.quit());
    siv.add_global_callback(cursive::event::Key::Esc, |s| s.quit());
    siv.add_global_callback('/', |s| {
        s.focus_name("filter").ok();
    });

    // Run the app
    siv.run();

    Ok(())
}

/// Builds the user interface with three main components:
/// - Item list (left pane, 40% width)
/// - Details pane (right pane, 60% width)
/// - Filter input (bottom, fixed height)
fn build_ui(siv: &mut Cursive) {
    // Create the item list
    let select = SelectView::<usize>::new()
        .h_align(HAlign::Left)
        .on_select(on_item_select)
        .with_name("item_list");

    // Repopulate_list will do the initial population after adding a layer

    // Create a details view
    let details = TextView::new("Select an item to view details")
        .scrollable()
        .with_name("details");

    // Create filter input
    let filter = EditView::new().on_edit(on_filter_edit).with_name("filter");

    // Create the main layout
    let main_layout = LinearLayout::horizontal()
        .child(
            Panel::new(select.scrollable())
                .title("Elements")
                .fixed_width(40),
        )
        .child(Panel::new(details).title("JSON definition").full_width());

    let root = LinearLayout::vertical()
        .child(ResizedView::with_full_screen(main_layout))
        .child(
            Panel::new(filter)
                .title("Filter ('/' to focus)")
                .fixed_height(3),
        );

    siv.add_fullscreen_layer(root);

    // Populate the list initially
    repopulate_list(siv);

    // Update details for the first item
    update_details_for_selected(siv);

    // Set initial focus on a list
    siv.focus_name("item_list").ok();
}

/// Callback triggered when user selects an item in the list.
/// Updates the details pane with highlighted JSON for the selected item.
fn on_item_select(siv: &mut Cursive, item_idx: &usize) {
    update_details_for_item(siv, *item_idx);
}

/// Updates the details pane with the JSON for the specified item index.
fn update_details_for_item(siv: &mut Cursive, item_idx: usize) {
    let state = siv.user_data::<AppState>().unwrap();
    let json_style = state.json_style;
    if let Some((json, _, _)) = state.indexed_items.get(item_idx)
        && let Ok(json_str) = serde_json::to_string_pretty(json)
    {
        let highlighted = highlight_json(&json_str, &json_style);
        siv.call_on_name("details", |view: &mut ScrollView<TextView>| {
            view.get_inner_mut().set_content(highlighted);
        });
    }
}

/// Updates the details pane for the currently selected item in the list.
fn update_details_for_selected(siv: &mut Cursive) {
    let selected_idx = siv
        .call_on_name("item_list", |view: &mut SelectView<usize>| {
            view.selection().map(|rc| *rc)
        })
        .flatten();

    if let Some(idx) = selected_idx {
        update_details_for_item(siv, idx);
    }
}

/// Callback triggered when user edits the filter input.
/// Filters items based on the query and repopulates the list.
/// Implements 150ms debouncing to keep UI responsive.
fn on_filter_edit(siv: &mut Cursive, query: &str, _cursor: usize) {
    let state = siv.user_data::<AppState>().unwrap();
    let version = state.search_version.as_ref();
    let current_version = version.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;

    let query_clone = query.to_string();
    let version_clone = state.search_version.clone();
    let cb_sink = siv.cb_sink().clone();

    std::thread::spawn(move || {
        // Wait for typing to pause
        std::thread::sleep(std::time::Duration::from_millis(150));

        // Only proceed if no newer search request has started
        if version_clone.load(std::sync::atomic::Ordering::SeqCst) == current_version {
            cb_sink
                .send(Box::new(move |s| {
                    // Perform the actual search on the main thread
                    let new_filtered = {
                        let state = s.user_data::<AppState>().unwrap();
                        matcher::search_with_index(
                            &state.search_index,
                            &state.indexed_items,
                            &query_clone,
                        )
                    };

                    if let Some(state) = s.user_data::<AppState>() {
                        state.filtered_indices = new_filtered;
                    }

                    repopulate_list(s);
                }))
                .ok();
        }
    });
}

/// Helper function to repopulate the item list from the current filtered indices.
/// Clears and rebuilds the SelectView with filtered items.
fn repopulate_list(siv: &mut Cursive) {
    let (filtered_indices, items_data): (Vec<usize>, Vec<(String, String, String)>) = {
        let state = siv.user_data::<AppState>().unwrap();
        let indices = state.filtered_indices.clone();
        let data: Vec<(String, String, String)> = indices
            .iter()
            .filter_map(|&idx| {
                state.indexed_items.get(idx).map(|(json, id, type_)| {
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

                    (
                        format!("[{}] {}", type_, display_name),
                        type_.clone(),
                        display_name,
                    )
                })
            })
            .collect();
        (indices, data)
    };

    siv.call_on_name("item_list", move |view: &mut SelectView<usize>| {
        view.clear();
        for (i, &idx) in filtered_indices.iter().enumerate() {
            let label = &items_data[i].0;
            view.add_item(label.clone(), idx);
        }
        // Select first item if available
        if !filtered_indices.is_empty() {
            view.set_selection(0);
        }
    });

    // Update details after repopulating the list
    update_details_for_selected(siv);
}

/// Applies syntax highlighting to JSON text using theme-consistent colors.
fn highlight_json(json: &str, style: &theme::JsonStyle) -> StyledString {
    let mut result = StyledString::new();

    // Use palette roles for the foundation styles
    // This ensures highlighting background matches the View background exactly.
    let style_default = ColorStyle::new(PaletteColor::Primary, PaletteColor::View);
    let style_key = ColorStyle::new(PaletteColor::TitlePrimary, PaletteColor::View);
    let style_punct = ColorStyle::new(PaletteColor::Secondary, PaletteColor::View);

    // Accent colors for values from the provided style
    let col_string = style.string;
    let col_num = style.number;
    let col_bool = style.boolean;

    for line in json.lines() {
        let mut remaining = line;

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
                    result.append_styled(prefix, style_default);
                    remaining = &remaining[pos + 1..];
                    continue;
                }

                // Add prefix before quotes
                let prefix = &remaining[..pos];
                if !prefix.is_empty() {
                    result.append_styled(prefix, style_default);
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

                    let quote_style = if is_key {
                        style_key
                    } else {
                        ColorStyle::new(col_string, PaletteColor::View)
                    };

                    result.append_styled(format!("\"{}\"", quoted), quote_style);
                    remaining = &rest[ep + 1..];
                } else {
                    result
                        .append_styled(remaining, ColorStyle::new(col_string, PaletteColor::View));
                    remaining = "";
                }
            } else {
                // Process numbers, booleans, and null
                let mut remaining_processed = remaining;
                while !remaining_processed.is_empty() {
                    let trimmed = remaining_processed.trim_start();
                    let start_offset = remaining_processed.len() - trimmed.len();
                    if start_offset > 0 {
                        result.append_styled(&remaining_processed[..start_offset], style_default);
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

                    let token_style = if token == "true" || token == "false" || token == "null" {
                        ColorStyle::new(col_bool, PaletteColor::View)
                    } else if (token.chars().all(|c| {
                        c.is_numeric() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+'
                    })) && !token.is_empty()
                        && token.chars().any(|c| c.is_numeric())
                    {
                        ColorStyle::new(col_num, PaletteColor::View)
                    } else if token == "{"
                        || token == "}"
                        || token == "["
                        || token == "]"
                        || token == ":"
                        || token == ","
                    {
                        style_punct
                    } else {
                        style_default
                    };

                    result.append_styled(token, token_style);
                    remaining_processed = rest;
                }
                remaining = "";
            }
        }
        result.append_plain("\n");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use cursive::theme::Color;
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
        let style = theme::JsonStyle {
            string: Color::Rgb(0, 255, 0),
            number: Color::Rgb(0, 0, 255),
            boolean: Color::Rgb(255, 0, 0),
        };
        let highlighted = highlight_json(json, &style);

        // Basic check that we have content - source() returns the underlying text
        let source = highlighted.source();
        assert!(!source.is_empty());
        assert!(source.contains("\"id\""));
        assert!(source.contains("\"test\""));
        assert!(source.contains("123"));
        assert!(source.contains("true"));

        // The function successfully creates a StyledString, which means
        // it processes the JSON without panicking. The actual color verification
        // would require runtime testing in a terminal.
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
