/** 0-based screen rectangle. A panel's rect includes its 2-line panel header. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type LayoutMode = 'split' | 'stacked' | 'list';

export interface Layout {
  mode: LayoutMode;
  list?: Rect;
  preview?: Rect;
  /** Column of the vertical divider between the panels (split mode with both panels shown). */
  dividerX?: number;
}

/** Header bar + filter pills on top, help bar at the bottom. */
export const TOP_ROWS = 2;
export const BOTTOM_ROWS = 1;
/** Each panel's own title line + underline. */
export const PANEL_HEADER_ROWS = 2;

const SPLIT_MIN_COLS = 80;
const STACKED_MIN_COLS = 50;
const STACKED_LIST_SHARE = 0.4;
const MIN_PANEL_ROWS = PANEL_HEADER_ROWS + 3;

/**
 * Side by side from 80 columns (list takes `sidebarPct` of the width), list above preview from 50,
 * list only below that. Hiding the sidebar leaves the preview alone, except in list-only mode where
 * the list is all there is.
 */
export function computeLayout(cols: number, rows: number, sidebarPct: number, sidebarVisible: boolean): Layout {
  const area: Rect = { x: 0, y: TOP_ROWS, width: cols, height: Math.max(MIN_PANEL_ROWS, rows - TOP_ROWS - BOTTOM_ROWS) };

  if (cols < STACKED_MIN_COLS) {
    return { mode: 'list', list: area };
  }
  if (!sidebarVisible) {
    return { mode: cols < SPLIT_MIN_COLS ? 'stacked' : 'split', preview: area };
  }
  if (cols < SPLIT_MIN_COLS) {
    const listHeight = Math.max(MIN_PANEL_ROWS, Math.floor(area.height * STACKED_LIST_SHARE));
    const previewHeight = Math.max(MIN_PANEL_ROWS, area.height - listHeight);
    return {
      mode: 'stacked',
      list: { ...area, height: listHeight },
      preview: { ...area, y: area.y + listHeight, height: previewHeight },
    };
  }
  const listWidth = Math.round((cols * sidebarPct) / 100);
  const dividerX = listWidth;
  return {
    mode: 'split',
    list: { ...area, width: listWidth },
    preview: { ...area, x: dividerX + 1, width: cols - listWidth - 1 },
    dividerX,
  };
}

/** The size a background agent's PTY should have so the preview shows it without cropping. */
export function ptySizeFor(layout: Layout): { cols: number; rows: number } | undefined {
  const p = layout.preview;
  return p ? { cols: Math.max(20, p.width), rows: Math.max(3, p.height - PANEL_HEADER_ROWS) } : undefined;
}
