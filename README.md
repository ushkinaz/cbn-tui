# cbn-tui 🚀

Terminal User Interface (TUI) browser for **Cataclysm: Bright Nights** JSON data.

![cbn-tui screenshot](./screenshot.png)

## ✨ Features

- **Blazing Fast**: Instantly browse and search through thousands of game items, monsters, and definitions.
- **Advanced Search Syntax**: Powerful filtering with support for specific fields and combined logic:
  - `id:zombie` or `i:zombie` - Filter by ID.
  - `type:MONSTER` or `t:MONSTER` - Filter by record type.
  - `category:weapon` or `c:weapon` - Filter by category.
  - `query1 query2` - AND logic for multiple terms.
- **Syntax Highlighting**: Beautifully formatted JSON details with syntax coloring for keys, strings, numbers, and booleans.
- **Deep Navigation**: Full keyboard and mouse support for seamless browsing.
- **Pane Management**: Switch focus between Item List, Details, and Filter bar using Tab.

## ⌨️ Desktop Controls

| Key                    | Action                                       |
|------------------------|----------------------------------------------|
| `q` / `Esc`            | Quit Application                             |
| `/`                    | Enter Filter Mode                            |
| `Enter`                | Exit Filter Mode                             |
| `Tab` / `Shift-Tab`    | Cycle Focused Pane (List / Details / Filter) |
| `j` / `k` or `↑` / `↓` | Move selection or scroll details             |
| `PageUp` / `PageDown`  | Scroll faster (10 items at a time)           |
| `Click`                | Focus pane or select item                    |

## 🚀 Getting Started

### Prerequisites
 
- No life

### Running the Application

By default, the application looks for data in `all.json`. You can specify a different file using the `--file` flag:
```bash
cargo run -- --file path/to/your/data.json
```

## 🛠️ Development

- **Build**: `cargo build`
- **Test**: `cargo test`
- **Run with custom data**: `cargo run -- --file data/all.json`

## 📄 License
Distributed under the MIT License. See `LICENSE` for more information.
