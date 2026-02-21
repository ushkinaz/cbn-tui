# Technology Stack

## Core Technologies
- **Programming Language:** Rust (2024 Edition)
- **TUI Framework:** [Ratatui](https://ratatui.rs/) with `crossterm` backend.
- **JSON Serialization/Deserialization:** [Serde](https://serde.rs/) and `serde_json` (configured with `preserve_order`).

## Infrastructure & Libraries
- **Network/HTTP Client:** [Reqwest](https://docs.rs/reqwest/) (using `blocking` feature for synchronous data fetching).
- **CLI Argument Parsing:** [Clap](https://docs.rs/clap/) (using `derive` features).
- **Error Handling:** [Anyhow](https://docs.rs/anyhow/) for idiomatic and flexible error management.
- **Data Caching:** [directories](https://docs.rs/directories/) for OS-compliant cache directory resolution.
- **UI Utilities:** `tui-scrollview` for scrolling large content areas, `unicode-width` for correct character width handling in TUI.
- **Performance:** `foldhash` for fast hashing in internal data structures.

## Build System
- **Package Manager:** `cargo`
