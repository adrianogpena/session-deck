//! Port of `layout.ts`: where the list and preview panels go for a terminal size.

use ratatui::layout::Rect;
use sdeck_core::store::deck_config::PanelLayout;
use sdeck_core::store::deck_store::{SIDEBAR_PCT_MAX, SIDEBAR_PCT_MIN};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Split,
    Stacked,
    List,
}

/// 0-based screen rectangles. A panel's rect includes its 2-line panel header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub mode: LayoutMode,
    pub list: Option<Rect>,
    pub preview: Option<Rect>,
    /// Column of the vertical divider between the panels (split mode with both panels shown).
    pub divider_x: Option<u16>,
}

/// Header bar + filter pills on top, help bar at the bottom.
pub const TOP_ROWS: u16 = 2;
pub const BOTTOM_ROWS: u16 = 1;
/// Each panel's own title line + underline.
pub const PANEL_HEADER_ROWS: u16 = 2;

pub const DEFAULT_SIDEBAR_PCT: f64 = 35.0;
pub const DEFAULT_STACKED_LIST_PCT: f64 = 40.0;
pub const SIDEBAR_STEP: f64 = 5.0;

const SPLIT_MIN_COLS: u16 = 80;
const STACKED_MIN_COLS: u16 = 50;
const MIN_PANEL_ROWS: u16 = PANEL_HEADER_ROWS + 3;

/// Smallest terminal the UI draws in: `MIN_ROWS` is `TOP_ROWS + BOTTOM_ROWS + MIN_PANEL_ROWS`.
pub const MIN_COLS: u16 = 30;
pub const MIN_ROWS: u16 = 8;

pub fn too_small(cols: u16, rows: u16) -> bool {
    cols < MIN_COLS || rows < MIN_ROWS
}

/// The panel sizes the user picked: the list's share of the width side by side, and of the height
/// stacked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelPrefs {
    pub arrangement: PanelLayout,
    pub sidebar_pct: f64,
    pub stacked_list_pct: f64,
    pub sidebar_visible: bool,
}

/// Which arrangement applies at `cols` columns, from 50 up (below that only the list shows).
fn panel_mode(arrangement: PanelLayout, cols: u16) -> LayoutMode {
    match arrangement {
        PanelLayout::Auto if cols < SPLIT_MIN_COLS => LayoutMode::Stacked,
        PanelLayout::Auto | PanelLayout::Side => LayoutMode::Split,
        PanelLayout::Stacked => LayoutMode::Stacked,
    }
}

/// `auto`: side by side from 80 columns (list takes `sidebar_pct` of the width), list above preview
/// (`stacked_list_pct` of the height) from 50. `side` / `stacked` keep one arrangement from 50 columns.
/// List only below 50. Hiding the sidebar leaves the preview alone, except in list-only mode where
/// the list is all there is.
pub fn compute_layout(cols: u16, rows: u16, prefs: PanelPrefs) -> Layout {
    let area = Rect::new(
        0,
        TOP_ROWS,
        cols,
        MIN_PANEL_ROWS.max(rows.saturating_sub(TOP_ROWS + BOTTOM_ROWS)),
    );

    if cols < STACKED_MIN_COLS {
        return Layout {
            mode: LayoutMode::List,
            list: Some(area),
            preview: None,
            divider_x: None,
        };
    }
    let mode = panel_mode(prefs.arrangement, cols);
    if !prefs.sidebar_visible {
        return Layout {
            mode,
            list: None,
            preview: Some(area),
            divider_x: None,
        };
    }
    if mode == LayoutMode::Stacked {
        let share = prefs.stacked_list_pct / 100.0;
        let list_height = MIN_PANEL_ROWS.max((f64::from(area.height) * share).floor() as u16);
        let preview_height = MIN_PANEL_ROWS.max(area.height.saturating_sub(list_height));
        return Layout {
            mode: LayoutMode::Stacked,
            list: Some(Rect {
                height: list_height,
                ..area
            }),
            preview: Some(Rect {
                y: area.y + list_height,
                height: preview_height,
                ..area
            }),
            divider_x: None,
        };
    }
    let list_width = (f64::from(cols) * prefs.sidebar_pct / 100.0).round() as u16;
    Layout {
        mode: LayoutMode::Split,
        list: Some(Rect {
            width: list_width,
            ..area
        }),
        preview: Some(Rect {
            x: list_width + 1,
            width: cols - list_width - 1,
            ..area
        }),
        divider_x: Some(list_width),
    }
}

/// The size `(cols, rows)` a background agent's PTY should have so the preview shows it uncropped.
pub fn pty_size_for(layout: &Layout) -> Option<(u16, u16)> {
    layout
        .preview
        .map(|p| (p.width.max(20), p.height.saturating_sub(PANEL_HEADER_ROWS).max(3)))
}

/// `<`/`>`: the sidebar width one step narrower/wider, kept within 15–70%.
pub fn step_sidebar(pct: f64, delta: f64) -> f64 {
    (pct + delta).clamp(SIDEBAR_PCT_MIN, SIDEBAR_PCT_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(arrangement: PanelLayout, sidebar_visible: bool) -> PanelPrefs {
        PanelPrefs {
            arrangement,
            sidebar_pct: 35.0,
            stacked_list_pct: DEFAULT_STACKED_LIST_PCT,
            sidebar_visible,
        }
    }

    fn auto(sidebar_visible: bool) -> PanelPrefs {
        prefs(PanelLayout::Auto, sidebar_visible)
    }

    #[test]
    fn splits_side_by_side_at_80_plus_columns_using_the_sidebar_percentage() {
        let layout = compute_layout(120, 40, auto(true));
        assert_eq!(layout.mode, LayoutMode::Split);
        assert_eq!(layout.list, Some(Rect::new(0, 2, 42, 37)));
        assert_eq!(layout.divider_x, Some(42));
        assert_eq!(layout.preview, Some(Rect::new(43, 2, 77, 37)));
        assert_eq!(pty_size_for(&layout), Some((77, 35)));
    }

    #[test]
    fn stacks_the_list_above_the_preview_between_50_and_79_columns() {
        let layout = compute_layout(70, 40, auto(true));
        assert_eq!(layout.mode, LayoutMode::Stacked);
        assert_eq!(layout.list, Some(Rect::new(0, 2, 70, 14)));
        assert_eq!(layout.preview, Some(Rect::new(0, 16, 70, 23)));
    }

    #[test]
    fn stacked_keeps_the_list_above_the_preview_on_a_wide_terminal_at_its_height_share() {
        let layout = compute_layout(
            200,
            40,
            PanelPrefs {
                stacked_list_pct: 50.0,
                ..prefs(PanelLayout::Stacked, true)
            },
        );
        assert_eq!(layout.mode, LayoutMode::Stacked);
        assert_eq!(layout.list, Some(Rect::new(0, 2, 200, 18)));
        assert_eq!(layout.preview, Some(Rect::new(0, 20, 200, 19)));
        assert_eq!(layout.divider_x, None);
    }

    #[test]
    fn side_keeps_the_panels_side_by_side_down_to_50_columns() {
        let layout = compute_layout(60, 40, prefs(PanelLayout::Side, true));
        assert_eq!(layout.mode, LayoutMode::Split);
        assert_eq!(layout.list, Some(Rect::new(0, 2, 21, 37)));
        assert_eq!(layout.divider_x, Some(21));
        assert_eq!(
            compute_layout(45, 40, prefs(PanelLayout::Side, true)).mode,
            LayoutMode::List
        );
    }

    #[test]
    fn shows_only_the_list_below_50_columns_even_with_the_sidebar_hidden() {
        let layout = compute_layout(45, 30, auto(false));
        assert_eq!(layout.mode, LayoutMode::List);
        assert_eq!(layout.preview, None);
        assert_eq!(pty_size_for(&layout), None);
    }

    #[test]
    fn gives_the_preview_the_whole_area_when_the_sidebar_is_hidden() {
        let layout = compute_layout(120, 40, auto(false));
        assert_eq!(layout.list, None);
        assert_eq!(layout.preview, Some(Rect::new(0, 2, 120, 37)));
        let stacked = compute_layout(120, 40, prefs(PanelLayout::Stacked, false));
        assert_eq!(stacked.mode, LayoutMode::Stacked);
        assert_eq!(stacked.preview, Some(Rect::new(0, 2, 120, 37)));
    }

    #[test]
    fn keeps_minimum_panel_rows_on_a_tiny_terminal() {
        let layout = compute_layout(120, 3, auto(true));
        assert_eq!(layout.list.unwrap().height, 5);
    }

    #[test]
    fn min_rows_is_the_chrome_plus_the_smallest_panel() {
        assert_eq!(MIN_ROWS, TOP_ROWS + BOTTOM_ROWS + MIN_PANEL_ROWS);
    }

    #[test]
    fn too_small_below_30_columns_or_8_rows() {
        assert!(too_small(29, 24));
        assert!(too_small(30, 7));
        assert!(!too_small(30, 8));
    }

    #[test]
    fn step_sidebar_clamps_to_15_and_70_percent() {
        assert_eq!(step_sidebar(35.0, SIDEBAR_STEP), 40.0);
        assert_eq!(step_sidebar(15.0, -SIDEBAR_STEP), 15.0);
        assert_eq!(step_sidebar(70.0, SIDEBAR_STEP), 70.0);
    }
}
