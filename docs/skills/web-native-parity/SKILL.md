---
name: web-native-parity
description: Maintain behavior parity between the native TUI runtime and the web runtime in this repository. Use when native changes are made (input handling, data loading, rendering, state flow, key bindings, progress, version switching, or dependency wiring) and equivalent web updates and verification are needed.
---

# Web Native Parity

Keep native and web behavior aligned by default.

## Architecture Contract

- Keep domain logic shared in `src/app_core/`, `src/matcher.rs`, `src/search_index.rs`, `src/ui.rs`, `src/model.rs`.
- Keep runtime adapters thin:
- `src/runtime/native/main.rs`
- `src/runtime/web/main.rs`
- Keep runtime-only data access isolated:
- `src/runtime/native/data.rs`
- `src/runtime/web/data.rs`
- Prefer moving logic from runtime files into shared modules instead of duplicating logic in both runtimes.

## When Native Changes

For every native change, classify it first:

1. Shared logic change: reducer/state/filtering/indexing/matcher/search/highlighting/layout math.
2. Runtime adapter change: key mapping, mouse mapping, filesystem/history, progress/timers, async/sync event handling.
3. Platform data change: HTTP/cache/source loading/build list parsing.

Then apply this rule:

- If type `1`, implement in shared code first, then only minimal adapter changes.
- If type `2` or `3`, implement native change and mirror behavior in web adapter/data path.

## Mandatory Diff Review

After native edits, inspect these files and confirm parity intent:

- `src/runtime/native/main.rs`
- `src/runtime/web/main.rs`
- `src/app_core/reducer.rs`
- `src/app_core/state.rs`
- `src/app_core/input.rs`
- `src/app_core/indexing.rs`
- `src/ui.rs`
- `Cargo.toml`

Use focused search to catch drift:

```bash
rg -n "handle_key_event|handle_mouse_event|pending_action|update_filter|progress|version|reload|history" src/runtime src/app_core src/ui.rs
```

## Parity Checklist

For each changed behavior, verify:

1. Key mapping parity:
- Native and web map equivalent keys/modifiers into `AppKeyEvent`.

2. Mouse parity:
- Both map coordinates to terminal cells before reducer call.
- Hover/click/scroll behavior is equivalent within platform limits.

3. Action flow parity:
- `pending_action` is consumed after input handling in both runtimes.
- Version picker and reload flows match expected behavior.

4. State transition parity:
- Focus/input mode transitions are identical for the same reducer events.
- Selection/filter/history behavior is consistent.

5. Data and progress parity:
- Progress stages/labels and completion timing are equivalent.
- Version label resolution and dataset apply flow are equivalent.

6. Dependency/config parity:
- Runtime-specific deps remain target-gated in `Cargo.toml`.
- Bin names/paths stay accurate (`cbn-tui`, `cbn-web`).

## Verification Commands

Run all of these before finishing:

```bash
cargo fmt --all
cargo test --offline
cargo check --offline --features web --bin cbn-web --target wasm32-unknown-unknown
```

If behavior changed and tests do not cover it, add tests in shared modules first:

- `src/app_core/reducer.rs` tests for input/state behavior.
- `src/app_core/web_mouse.rs` tests for web coordinate math.
- `src/ui.rs` tests for hit-testing/cursor/layout helpers.

## Non-Negotiables

- Do not duplicate business logic in runtime files if shared module can own it.
- Do not introduce native-only assumptions into shared modules.
- Do not ship parity-sensitive changes without running native + wasm checks.
- Do not remove tests to make parity issues pass.

## PR Summary Template

Use this in change summaries:

```text
Parity scope:
- Native change:
- Web mirror:
- Shared refactor:

Validation:
- cargo test --offline
- cargo check --offline --features web --bin cbn-web --target wasm32-unknown-unknown

Known platform differences:
- (example: ratzilla wheel events unavailable in on_mouse_event)
```
