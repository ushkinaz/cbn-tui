# Agent Guide for cbn-tui

Purpose
- Terminal UI browser for Cataclysm: Bright Nights JSON data.
- Real data for analysis is in `data/all.json`.
- Schema reference lives in `reference/types.ts` (TypeScript typings).

Repository Layout
- `Cargo.toml`: crate metadata and dependencies.
- `src/main.rs`: application entry point, UI, state, and tests.
- `target/`: build artifacts (do not edit).

Build, Run, Lint, Test
- Build: `cargo build`
- Run with nightly (default): `cargo run`
- Run with stable: `cargo run -- --game stable`
- Run with a custom file: `cargo run -- --file /path/to/all.json`
- List versions: `cargo run -- --game-versions`
- Test all: `cargo test`
- Single test: `cargo test test_matches`
- Single test in module: `cargo test tests::test_matches`
- Single test exact: `cargo test test_matches -- --exact`
- Test with output: `cargo test test_matches -- --nocapture`
- List tests: `cargo test -- --list`
- Run ignored tests (if any): `cargo test -- --ignored`
- Format: `cargo fmt --all`
- Format check: `cargo fmt --all -- --check`
- Lint: `cargo clippy --all-targets --all-features`

Testing Expectations
- Never delete tests without explicit permission; keep coverage the same or improved.
- If refactoring breaks a test, fix the test or code instead of removing it.
- For bug fixes, follow TDD: add or update a failing test before the fix.

Data Download and Caching
- Downloaded files live under the OS cache directory from `directories::ProjectDirs`.
- Cached path includes the game version, then `all.json` (per `--game`).
- The build list cache (`builds.json`) expires after one hour.
- Nightly data cache expires after 12 hours; stable cache after 30 days.
- Keep network logic synchronous using `reqwest::blocking`.
- Always check HTTP status before reading response bytes.
- Cache directories are created with `fs::create_dir_all` before writes.
- Use `--force` to bypass age checks and refresh cached content.

Large JSON Workflow (jq)
- `all.json` can be tens of MB; avoid opening in editors.
- Use `jq` for targeted queries and keep commands in docs/notes.
- Primary fields: `id`, `abstract`, `type`, `category`, `name`.
- Filter specific item
  - `jq '.data[] | select(.id=="<id>" and .type=="<type>")' all.json`
- List all IDs of a type
  - `jq -r '.data[] | select(.type=="item") | .id' all.json`
- Find items with a specific property
  - `jq '.data[] | select(.color == "RED")' all.json`
- Show types with counts
  - `jq -r '.data[] | .type' all.json | sort | uniq -c | sort -nr`
- Inheritance: raw JSON uses `copy-from`; check the parent when fields are missing.
- `abstract` entries often define shared defaults; resolve with `copy-from` manually.
- The app is read-only; no mutation of JSON is performed.

Cataclysm:BN JSON Structure
- Top-level file is `{ "data": [ ... ] }`.
- Each record is an object with optional `id`, `abstract`, `type`, `category`.
- `name` can be a string or an object with a `.str` field.
- Missing fields are common; treat them as empty strings for filtering.
- Some values are arrays or nested objects; parsing must stay tolerant.
- IDs are not guaranteed unique across types; include `type` in filters.
- `abstract` objects are templates, not always user-facing entries.
- Name formatting can vary by record type; keep display logic defensive.

Coding Style (Rust)
- Keep rendering logic focused on drawing; state updates happen in event handling.
- Store `AppState` as a mutable struct in the main loop.
- Use helper functions like `highlight_json` for data transformation.
- Formatting: rely on rustfmt defaults; avoid manual alignment.
- Prefer explicit `String` conversions at boundaries.
- Avoid unnecessary clones of `serde_json::Value` or large strings.
- Prefer early returns for guard clauses and error conditions.
- Keep filtering logic in matcher module case-insensitive.
- Tests live in `#[cfg(test)]` module inside respecive files.
- Keep event handlers small and focused; heavy work happens in separate functions.
- Use local variables for repeated lookups to reduce borrow complexity.

Imports and Naming
- Group imports by crate; keep `std` imports last.
- Prefer multi-line `use` blocks for long import lists.
- Avoid glob imports; list the necessary items explicitly.
- Use `snake_case` for functions, locals, and fields.
- Use `PascalCase` for types and enums.
- Suffix reserved words with `_`, e.g., `type_`, `abstract_`.
- Use descriptive names for view IDs (`"item_list"`, `"details"`, `"filter"`).

Error Handling
- `main` returns `anyhow::Result<()>`.
- Use `anyhow::bail!` for user-facing errors (missing files, HTTP failures).
- Prefer `?` for propagation over `unwrap`.
- `unwrap` is acceptable when an invariant is guaranteed (e.g., user data set).
- Keep error messages actionable; include file paths or HTTP status codes.
- Avoid panics outside tests or truly unreachable branches.
- Add context with `anyhow::Context` if an error message needs more detail.

Data and JSON Handling
- Use `serde_json::Value` for flexible parsing.
- Use `serde::Deserialize` for stable structs (`Root`, `GameBuild`).
- `serde_json` is configured with `preserve_order`; keep this if reformatting.
- The `matches` function lowercases once per term; avoid repeated work elsewhere.
- Prefer linear scans to complex indices unless profiling proves needed.
- Keep UI rendering from JSON via `serde_json::to_string_pretty`.
- Extract fields with `Value::get` + `as_str` and fallback to empty strings.
- When parsing `name`, check both string and `{ "str": ... }` forms.
- Avoid storing derived strings that can be computed once per item.

UI and Interaction Conventions
- Use `ratatui` for all rendering and layout.
- `List` widget renders the item list with selection, `Paragraph` shows details.
- Rendering happens in a single render function; state drives the UI.
- Use `Vec<Span>` for syntax highlighting with theme-consistent colors.
- Theme system defined in `theme.rs` returns `ThemeConfig` with ratatui styles.
- Event loop handles keyboard: `q`/`Esc` quit, `/` focuses filter, Up/Down navigate.
- Filter input updates state directly; repopulation triggers on state change.
- Use `Layout` with constraints for responsive panel sizing.
- Keep layout split consistent with Elements (40% left) and details (60% right).

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
