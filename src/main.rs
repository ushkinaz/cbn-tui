use anyhow::Result;
use clap::Parser;
use cursive::Cursive;
use cursive::align::HAlign;
use cursive::theme::{Color, ColorStyle, PaletteColor, Theme};
use cursive::traits::*;
use cursive::utils::markup::StyledString;
use cursive::views::{
    EditView, LinearLayout, Panel, ResizedView, ScrollView, SelectView, TextView,
};
use serde::Deserialize;
use serde_json::Value;
use std::fs;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the all.json file
    #[arg(short, long)]
    file: Option<String>,

    /// Game version to download (e.g., v0.9.1, stable, nightly)
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

/// Represents a single item from the Cataclysm:BN JSON data.
/// Stores both the original JSON and extracted fields for filtering and display.
#[derive(Clone)]
struct CbnItem {
    /// The original JSON value for this item
    original_json: Value,
    /// Display name derived from id, abstract, or name field
    display_name: String,
    /// Pre-lowercased item ID for filtering
    id_lower: String,
    /// Original item ID for sorting/display
    id: String,
    /// Pre-lowercased item type for filtering
    type_lower: String,
    /// Original item type for sorting
    type_: String,
    /// Pre-lowercased item category for filtering
    category_lower: String,
    /// Pre-lowercased abstract identifier for filtering
    abstract_lower: String,
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

        let id_lower = id.to_lowercase();
        let abstract_lower = abstract_.to_lowercase();
        let type_lower = type_.to_lowercase();
        let category_lower = category.to_lowercase();

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
            id_lower,
            id,
            type_lower,
            type_,
            category_lower,
            abstract_lower,
        }
    }

    /// Checks if the item matches the given search query.
    /// Supports prefixes:
    /// - `id:` or `i:` for filtering by ID
    /// - `type:` or `t:` for filtering by type
    /// - `category:` or `c:` for filtering by category
    ///   Multiple terms are combined using AND logic.
    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        for part in query.split_whitespace() {
            let part_lower = part.to_lowercase();

            let match_found = if let Some(val) = part_lower.strip_prefix("id:") {
                self.id_lower.contains(val)
            } else if let Some(val) = part_lower.strip_prefix("i:") {
                self.id_lower.contains(val)
            } else if let Some(val) = part_lower.strip_prefix("type:") {
                self.type_lower.contains(val)
            } else if let Some(val) = part_lower.strip_prefix("t:") {
                self.type_lower.contains(val)
            } else if let Some(val) = part_lower.strip_prefix("category:") {
                self.category_lower.contains(val)
            } else if let Some(val) = part_lower.strip_prefix("c:") {
                self.category_lower.contains(val)
            } else {
                self.id_lower.contains(&part_lower)
                    || self.type_lower.contains(&part_lower)
                    || self.abstract_lower.contains(&part_lower)
                    || self.category_lower.contains(&part_lower)
            };

            if !match_found {
                return false;
            }
        }
        true
    }
}

/// Application state stored in Cursive's user data.
/// Contains all items and the current filtered subset.
struct AppState {
    /// All loaded items from the JSON file
    all_items: Vec<CbnItem>,
    /// Indices into all_items that match the current filter
    filtered_indices: Vec<usize>,
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
            let type_ = if build.prerelease {
                "Nightly"
            } else {
                "Stable"
            };
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

        let mut should_download = args.force || !target_path.exists();
        if !should_download {
            let expiration = match game_version.as_str() {
                "nightly" => Some(std::time::Duration::from_secs(12 * 3600)),
                "stable" => Some(std::time::Duration::from_secs(30 * 24 * 3600)),
                _ => None,
            };

            if let Some(exp) = expiration {
                if let Ok(metadata) = fs::metadata(&target_path) {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(elapsed) = modified.elapsed() {
                            if elapsed > exp {
                                should_download = true;
                            }
                        }
                    }
                }
            }
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
    } else if let Some(file) = args.file {
        file
    } else {
        "all.json".to_string()
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
    let mut items: Vec<CbnItem> = root.data.into_iter().map(CbnItem::from_json).collect();

    // Sort items by type then id
    items.sort_by(|a, b| a.type_.cmp(&b.type_).then_with(|| a.id.cmp(&b.id)));

    // Create Cursive app
    let mut siv = cursive::default();
    siv.set_theme(solarized_dark());

    // Initialize state
    let filtered_indices: Vec<usize> = (0..items.len()).collect();
    let state = AppState {
        all_items: items,
        filtered_indices,
    };
    siv.set_user_data(state);

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
    if let Some(item) = state.all_items.get(item_idx) {
        if let Ok(json_str) = serde_json::to_string_pretty(&item.original_json) {
            let highlighted = highlight_json(&json_str);
            siv.call_on_name("details", |view: &mut ScrollView<TextView>| {
                view.get_inner_mut().set_content(highlighted);
            });
        }
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
fn on_filter_edit(siv: &mut Cursive, query: &str, _cursor: usize) {
    // Update filtered indices
    let new_filtered: Vec<usize> = {
        let state = siv.user_data::<AppState>().unwrap();
        state
            .all_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.matches(query))
            .map(|(i, _)| i)
            .collect()
    };

    // Update state
    if let Some(state) = siv.user_data::<AppState>() {
        state.filtered_indices = new_filtered;
    }

    // Repopulate a list
    repopulate_list(siv);
}

/// Returns a Cursive theme based on the Solarized Dark color palette.
fn solarized_dark() -> Theme {
    let mut theme = Theme::default();

    // Solarized Dark palette
    let base03 = Color::Rgb(0, 43, 54);
    let base02 = Color::Rgb(7, 54, 66);
    let base01 = Color::Rgb(88, 110, 117);
    let base00 = Color::Rgb(101, 123, 131);
    let base0 = Color::Rgb(131, 148, 150);
    let base3 = Color::Rgb(253, 246, 227);
    let yellow = Color::Rgb(181, 137, 0);
    let blue = Color::Rgb(38, 139, 210);

    {
        let palette = &mut theme.palette;

        palette[PaletteColor::Background] = base03;
        palette[PaletteColor::View] = base02;
        palette[PaletteColor::Shadow] = Color::Rgb(0, 0, 0); // Pure black for shadows

        palette[PaletteColor::Primary] = base0;
        palette[PaletteColor::Secondary] = base01;
        palette[PaletteColor::Tertiary] = base00;

        palette[PaletteColor::TitlePrimary] = blue;
        palette[PaletteColor::TitleSecondary] = yellow;

        palette[PaletteColor::Highlight] = blue;
        palette[PaletteColor::HighlightInactive] = base01;
        palette[PaletteColor::HighlightText] = base3;

        // Custom borders/panels
        // Cursive uses Yellow (for active) and White (for inactive) by default for Panel borders,
        // but it's better to keep it consistent with the palette.
    }

    // Border style
    theme.borders = cursive::theme::BorderStyle::Simple;

    theme
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
                state.all_items.get(idx).map(|item| {
                    (
                        format!("[{}] {}", item.type_, item.display_name),
                        item.type_.clone(),
                        item.display_name.clone(),
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
fn highlight_json(json: &str) -> StyledString {
    let mut result = StyledString::new();

    // Use palette roles for the foundation styles
    // This ensures highlighting background matches the View background exactly.
    let style_default = ColorStyle::new(PaletteColor::Primary, PaletteColor::View);
    let style_key = ColorStyle::new(PaletteColor::TitlePrimary, PaletteColor::View);
    let style_punct = ColorStyle::new(PaletteColor::Secondary, PaletteColor::View);

    // Accent colors for values (Solarized palette accents)
    let col_string = Color::Rgb(133, 153, 0); // Green
    let col_num = Color::Rgb(211, 54, 130); // Magenta
    let col_bool = Color::Rgb(220, 50, 47); // Red

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
    fn test_sorting() {
        let mut items = [
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
