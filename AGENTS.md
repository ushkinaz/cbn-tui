# Agent Guide for cbn-tui

Purpose
- Terminal UI browser for Cataclysm: Bright Nights JSON data.
- Real data for analysis is in `data/all.json`.
- Schema reference lives in `reference/types.ts` (TypeScript typings).

Runtime Priority
- TUI/native comes first.
- Web is a parity target and may lag behind native temporarily.
- Concentrated development focus is on TUI; update web after native behavior is stable.

Repository Layout
- `Cargo.toml`: crate metadata and dependencies.
- `src/main.rs`: application entry point, UI, state, and tests.
- `src/`: modules for data loading, matching, search index, theming, and UI rendering.
- `target/`: build artifacts (do not edit).

Build, Run, Lint, Test
- Build: `cargo build`
- Run (nightly default): `cargo run`
- Run with a custom file: `cargo run -- --file /path/to/all.json`
- Test all: `cargo test`
- Single test by name: `cargo test test_matches`
- Single test in module: `cargo test tests::test_matches`
- Single test exact: `cargo test test_matches -- --exact`
- Single test with output: `cargo test test_matches -- --nocapture`
- List tests: `cargo test -- --list`
- Run ignored tests (if any): `cargo test -- --ignored`
- Format: `cargo fmt --all`
- Format check: `cargo fmt --all -- --check`
- Lint: `cargo clippy --all-targets --all-features`

Testing Expectations
- Never delete tests without explicit permission; keep coverage the same or improved.
- If refactoring breaks a test, fix the test or code instead of removing it.
- For bug fixes, follow TDD: add or update a failing test before the fix.

Coding Style (Rust)
- Keep rendering logic focused on drawing; state updates happen in event handling.
- Store `AppState` as a mutable struct in the main loop.
- Keep event handlers small and focused; heavy work happens in separate functions.
- Use helper functions like `highlight_json` for data transformation.
- Formatting: rely on rustfmt defaults; avoid manual alignment.
- Prefer explicit `String` conversions at boundaries.
- Avoid unnecessary clones of `serde_json::Value` or large strings.
- Prefer early returns for guard clauses and error conditions.
- Tests live in `#[cfg(test)]` modules inside the same file.

Imports and Naming
- Group imports by crate; keep `std` imports last.
- Prefer multi-line `use` blocks for long import lists.
- Avoid glob imports; list the necessary items explicitly.
- Use `snake_case` for functions, locals, and fields.
- Use `PascalCase` for types and enums.
- Suffix reserved words with `_`, e.g., `type_`, `abstract_`.
- Use descriptive names for view IDs (`"item_list"`, `"details"`, `"filter"`).

Types and Data
- Use `serde_json::Value` for flexible parsing of raw data.
- Use `serde::Deserialize` for stable structs (`Root`, `BuildInfo`).
- `serde_json` is configured with `preserve_order`; keep this if reformatting.
- Extract fields with `Value::get` + `as_str` and fallback to empty strings.
- When parsing `name`, check both string and `{ "str": ... }` forms.
- Missing fields are common; treat them as empty strings for filtering.
- IDs are not guaranteed unique across types; include `type` in filters.

Error Handling
- Use `anyhow::bail!` for user-facing errors (missing files, HTTP failures).
- Prefer `?` for propagation over `unwrap`.
- `unwrap` is acceptable when an invariant is guaranteed.
- Keep error messages actionable; include file paths or HTTP status codes.
- Avoid panics outside tests or truly unreachable branches.
- Add context with `anyhow::Context` if an error message needs more detail.

UI and Interaction Conventions
- Use `ratatui` for all rendering and layout.
- `List` widget renders the item list; `Paragraph` shows details.
- Rendering happens in a single render function; state drives the UI.
- Use `Vec<Span>` for syntax highlighting with theme-consistent colors.
- Theme system defined in `theme.rs` returns `ThemeConfig` with ratatui styles.
- Event loop handles keyboard; keep navigation and filtering consistent.
- Use `Layout` with constraints for responsive panel sizing.
- Keep layout split consistent with Elements (40% left) and details (60% right).

Filtering and Search
- Filtering logic lives in `matcher` and is case-insensitive.
- `SearchIndex` supports fast lookups; rebuild when dataset changes.
- Update filter via `update_filter()` to refresh list, caches, and details.
- Treat `filter_cursor` as a character index; use `char_indices` for byte offsets.
- Filter history persists to `history.txt` in the data dir.
- Avoid recomputing display strings; rely on `cached_display` after filter changes.

Details Rendering Cache
- Use `cached_details_item_idx` to skip expensive JSON re-rendering.
- Invalidate wrapped cache when content changes by resetting `details_wrapped_width`.
- Reset `details_scroll_state` on selection changes for snappy navigation.

Performance Notes
- Dataset is large; avoid repeated scans when possible.
- Filtering should be O(n) on `all_items`.
- Store filtered indices instead of cloning items.
- Keep string allocations in hot loops to a minimum.
- Avoid repeated `to_string_pretty` calls unless selection changes.
- Keep highlight work scoped to the visible selection.

Agent Expectations
- Do not edit `target/` or cached `all.json` files.
- Avoid adding dependencies unless required by the task.
- Keep changes minimal and localized to `src/main.rs` unless refactoring.
- Preserve existing UI patterns and theme choices.
- Do not edit cached downloads under OS cache directories.
- Avoid running heavy commands on `all.json` without `jq` filters.
- Always use the Context7 MCP tool to look up external library documentation before implementing functionality that relies on those libraries.

Commit Message Rules
- All commits follow Conventional Commits.
- Format: `type(scope): summary`.
- Examples: `feat(tui): add search highlights`, `fix(parser): handle empty ids`.
  
