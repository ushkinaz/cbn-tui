//! Runtime-agnostic input event types.
//!
//! Both the native (crossterm) and web (ratzilla) runtimes convert their
//! platform-specific events into these types before calling the shared reducer.

/// Runtime-agnostic key codes used by the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKeyCode {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Enter,
    Esc,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
}

/// A runtime-agnostic keyboard event.
#[derive(Debug, Clone, Copy)]
pub struct AppKeyEvent {
    pub code: AppKeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// `true` when the key was released (ignored by the reducer).
    pub is_release: bool,
}

impl AppKeyEvent {
    pub fn new(code: AppKeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
            is_release: false,
        }
    }
}

/// The kind of a runtime-agnostic mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMouseKind {
    Move,
    LeftDown,
    ScrollUp,
    ScrollDown,
}

/// A runtime-agnostic mouse event with pre-converted cell coordinates.
///
/// Coordinate conversion (pixel → cell) is always the responsibility of the
/// runtime adapter layer, never of the shared reducer.
#[derive(Debug, Clone, Copy)]
pub struct AppMouseEvent {
    pub kind: AppMouseKind,
    /// Column in terminal cell coordinates.
    pub column: u16,
    /// Row in terminal cell coordinates.
    pub row: u16,
    pub ctrl: bool,
}
