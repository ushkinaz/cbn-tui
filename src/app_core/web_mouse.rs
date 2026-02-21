//! Shared helpers for web mouse coordinate translation.
//!
//! The web runtime receives pixel coordinates from the browser, while the
//! reducer expects terminal cell coordinates. This module keeps the conversion
//! logic pure and testable.

/// Pixel-space rectangle for the rendered terminal grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// Converts browser mouse pixel coordinates to terminal cell coordinates.
///
/// Returns `None` if the point is outside the grid or if dimensions are invalid.
/// Uses `cell_size` when available for precise mapping, and falls back to
/// proportional mapping if only `fallback_size` is available.
pub fn mouse_pixels_to_cell(
    mouse_x: f64,
    mouse_y: f64,
    grid: PixelRect,
    cell_size: Option<(f64, f64)>,
    fallback_size: Option<(u16, u16)>,
) -> Option<(u16, u16)> {
    if grid.width <= 0.0 || grid.height <= 0.0 {
        return None;
    }

    let local_x = mouse_x - grid.left;
    let local_y = mouse_y - grid.top;
    if local_x < 0.0 || local_y < 0.0 || local_x >= grid.width || local_y >= grid.height {
        return None;
    }

    let (fallback_cols, fallback_rows) = fallback_size.unwrap_or((0, 0));
    let has_fallback = fallback_cols > 0 && fallback_rows > 0;

    if let Some((cell_width, cell_height)) = cell_size
        && cell_width > 0.0
        && cell_height > 0.0
    {
        let mut col = (local_x / cell_width).floor() as u16;
        let mut row = (local_y / cell_height).floor() as u16;
        if has_fallback {
            col = col.min(fallback_cols.saturating_sub(1));
            row = row.min(fallback_rows.saturating_sub(1));
        }
        return Some((col, row));
    }

    if has_fallback {
        let col = ((local_x / grid.width) * fallback_cols as f64).floor() as u16;
        let row = ((local_y / grid.height) * fallback_rows as f64).floor() as u16;
        return Some((
            col.min(fallback_cols.saturating_sub(1)),
            row.min(fallback_rows.saturating_sub(1)),
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{PixelRect, mouse_pixels_to_cell};

    #[test]
    fn maps_with_explicit_cell_size() {
        let grid = PixelRect {
            left: 100.0,
            top: 50.0,
            width: 400.0,
            height: 200.0,
        };

        let mapped = mouse_pixels_to_cell(149.9, 89.9, grid, Some((10.0, 20.0)), Some((40, 10)));
        assert_eq!(mapped, Some((4, 1)));
    }

    #[test]
    fn clamps_to_fallback_bounds_with_cell_size() {
        let grid = PixelRect {
            left: 0.0,
            top: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let mapped = mouse_pixels_to_cell(799.0, 599.0, grid, Some((6.0, 6.0)), Some((80, 24)));
        assert_eq!(mapped, Some((79, 23)));
    }

    #[test]
    fn uses_proportional_fallback_when_cell_size_missing() {
        let grid = PixelRect {
            left: 10.0,
            top: 20.0,
            width: 200.0,
            height: 100.0,
        };

        let mapped = mouse_pixels_to_cell(60.0, 45.0, grid, None, Some((20, 10)));
        assert_eq!(mapped, Some((5, 2)));
    }

    #[test]
    fn returns_none_outside_grid() {
        let grid = PixelRect {
            left: 10.0,
            top: 20.0,
            width: 200.0,
            height: 100.0,
        };

        let mapped = mouse_pixels_to_cell(9.0, 20.0, grid, Some((10.0, 10.0)), Some((20, 10)));
        assert_eq!(mapped, None);
    }

    #[test]
    fn returns_none_without_valid_metrics() {
        let grid = PixelRect {
            left: 0.0,
            top: 0.0,
            width: 0.0,
            height: 100.0,
        };

        let mapped = mouse_pixels_to_cell(10.0, 10.0, grid, None, None);
        assert_eq!(mapped, None);
    }
}
