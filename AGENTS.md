# Agent Guide for cbn-tui

Purpose
- Terminal UI browser for Cataclysm:BN JSON data.
- Default dataset from this crate is `data/all.json` (relative to `tui/`).
- Schema reference lives in `reference/types.ts` (TypeScript typings).

Repository Layout
- `Cargo.toml`: crate metadata and dependencies.
- `src/main.rs`: application entry point and UI logic.
- `target/`: build artifacts (do not edit).

Build, Run, Lint, Test
- Build: `cargo build`
- Run: `cargo run -- --file data/all.json`
- Run with default file: `cargo run`
- Test: `cargo test`
- Single test: `cargo test <test_name>`
- Single test in module: `cargo test <module>::<test_name>`
- Format (if needed): `cargo fmt --all`
- Lint (if needed): `cargo clippy --all-targets --all-features`

Large JSON Workflow
- Truth: `data/all.json` is the compiled data blob.
- Large file handling: ~30MB, never read the whole file; use `jq`.
- Primary fields: `id`, `abstract`, `type`, `category`.
- Filter specific item
  - `jq '.data[] | select(.id=="<id>" and .type=="<type>")' data/all.json`
- List all IDs of a type
  - `jq '.data[] | select(.type=="item") | .id' -r data/all.json`
- Find items with specific property
  - `jq '.data[] | select(.color == "RED")' data/all.json`
- Show types with counts
  - `jq -r '.data[] | .type' data/all.json | sort | uniq -c | sort -nr`
- Inheritance: raw JSON uses `copy-from`; check the parent when fields are missing.
- Resolution: use `../src/data.ts` (`CBNData` / `_flatten`) for resolved values.

Cataclysm:BN JSON Structure
- Top-level file has `{ data: [...] }`.
- Records often include `id`, `abstract`, `type`, `category`.
- Some fields can be arrays or objects based on type.

Coding Style (Rust)
- Formatting: rely on rustfmt; keep default style.
- Imports grouped by crate, multi-line use blocks are preferred.
- Use `snake_case` for functions/fields and `PascalCase` for types.
- Reserved words use suffixes: `type_`, `abstract_`.
- Prefer small helper methods for logic (see `CbnItem::from_json`).
- Keep UI layout in `ui` and input loop in `run_app`.
- Keep data parsing in `main` or dedicated helpers.

Error Handling
- Use `anyhow::Result` for `main`.
- Propagate errors with `?`.
- Use `io::Result` for terminal loop functions.
- Avoid panic except for truly impossible states.
- Prefer explicit messages for recoverable errors.

Data and JSON Handling
- Use `serde` and `serde_json::Value` for flexible parsing.
- Use `serde::Deserialize` for typed data when possible.
- Avoid cloning large JSON values unless necessary.
- Keep filtering case-insensitive and fast for large inputs.

UI and Interaction Conventions
- Keyboard flow: `q` quits, `/` enters filter input.
- Up/down or `j`/`k` move selection.
- Keep highlight style consistent with ratatui defaults.
- Layout uses vertical split with list/details and filter input.

Performance Notes
- The dataset is large; avoid repeated full scans where possible.
- Filtering should be linear and low-allocation.
- Prefer indexing into `items` via `filtered_items` indices.

Agent Expectations
- Do not edit `target/` or generated files.
- Avoid adding new dependencies unless required.
- Keep changes minimal and localized.
- Follow repository conventions for naming and structure.

Commit Message Rules
- All commits must follow Conventional Commits.
- Format: `type(scope): summary`.
- Examples: `feat(tui): add search highlights`, `fix(parser): handle empty ids`.
