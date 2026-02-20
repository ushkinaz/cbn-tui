## 2026-02-20 - [Ratatui Text Caching Anti-Pattern] 
**Learning:** Caching `ratatui::text::Text<'static>` inside the application state and cloning its `Span` components on interactive events (like hover) causes extreme memory allocation overhead, as `Span<'static>` owns its entire string content via `Cow::Owned`. 
**Action:** Always store the raw data and pre-computed boundary/style markers in application state, and generate `Text<'a>` on the fly during the `render` pass using `Cow::Borrowed` slices referencing the original buffers.
