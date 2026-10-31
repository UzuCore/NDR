//! Cursor movement of original TNXCustomFileListView::ArrowKey (0x7d69dc). It differs per view style (0x38c):
//! 0 Auto and 1 Names = column-flowing list (fill one column top to bottom, then the next), 2 Details, 3 Thumbnails (one row left to right).
//! The drawn list leaves page info (`ListPage`) and key handling picks the new cursor from it.

/// List drawing style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ListStyle {
    /// Original 2 (Details). One item per row.
    #[default]
    Details,
    /// Original 0 and 1 (Auto, Names): item index / rows per column = column, remainder = row (GetItemRect).
    Flow,
    /// Original 3 (Thumbnails): several cells per row.
    Thumbnails,
}

/// Page info of the last frame (original 0x338 rows per column, 0x33c visible columns, 0xf1 top item).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ListPage {
    pub style: ListStyle,
    /// Flow: rows per column (0x338). Details: rows fully visible on one page (0xce). Thumbnails: fully visible rows.
    pub rows: usize,
    /// Flow: visible columns (0x33c). Thumbnails: cells per row (0xcf). Details: 1.
    pub columns: usize,
    /// Top (first) visible item (0xf1).
    pub top: usize,
    /// Details: the last fully visible item.
    pub last: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListKey {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
}

/// Original ArrowKey: the new cursor chosen by key from the current cursor and the item count. Out-of-range values clamp to 0..count - 1.
/// Left and Right in Details do not move the cursor, like the original (which scrolls horizontally by 20 when there is a horizontal bar).
pub(super) fn arrow_target(page: &ListPage, cursor: usize, count: usize, key: ListKey) -> usize {
    if count == 0 {
        return 0;
    }
    let (current, n) = (cursor as i64, count as i64);
    let (rows, columns, top) = (page.rows.max(1) as i64, page.columns.max(1) as i64, page.top as i64);
    let target = match (page.style, key) {
        (_, ListKey::Home) => 0,
        (_, ListKey::End) => n - 1,
        (ListStyle::Thumbnails, ListKey::Up) => {
            if current < columns {
                current - 1
            } else {
                current - columns
            }
        }
        (_, ListKey::Up) => current - 1,
        (ListStyle::Thumbnails, ListKey::Down) => {
            if n % columns < n - current {
                current + columns
            } else {
                current + 1
            }
        }
        (_, ListKey::Down) => current + 1,
        (ListStyle::Flow, ListKey::Left) => {
            if current < rows {
                current - 1
            } else {
                current - rows
            }
        }
        (ListStyle::Flow, ListKey::Right) => {
            // Items in the last column (0 means a full column).
            let last_column = if n % rows == 0 { rows } else { n % rows };
            if last_column < n - current {
                current + rows
            } else {
                current + 1
            }
        }
        (ListStyle::Thumbnails, ListKey::Left) => current - 1,
        (ListStyle::Thumbnails, ListKey::Right) => current + 1,
        (ListStyle::Details, ListKey::Left | ListKey::Right) => current,
        (ListStyle::Flow, ListKey::PageUp) => {
            if top == current {
                current - rows * columns
            } else {
                top
            }
        }
        (ListStyle::Flow, ListKey::PageDown) => {
            let last = rows * columns + top - 1;
            if last == current {
                current + rows * columns
            } else {
                last
            }
        }
        (ListStyle::Details, ListKey::PageUp) => {
            if top < current {
                top
            } else {
                current - rows
            }
        }
        (ListStyle::Details, ListKey::PageDown) => {
            let last = page.last as i64;
            if last > current {
                last
            } else {
                current + rows
            }
        }
        (ListStyle::Thumbnails, ListKey::PageUp) => {
            if current - columns < 1 {
                0
            } else {
                current - columns * rows
            }
        }
        (ListStyle::Thumbnails, ListKey::PageDown) => {
            if current + columns < n - 1 {
                current + columns * rows
            } else {
                n - 1
            }
        }
    };
    target.clamp(0, n - 1) as usize
}

/// PageUp and PageDown of original ArrowKey (0x7d69dc) style 3 (thumbnails with per-row heights). `tops` and `heights` = per-row top and height (content coordinates,
/// item +0x6c and GetThumbnailRowHeight), `offset` = top position (0xdd), `view` = list height (GetRealClientHeight).
/// - PageDown: if the row below (cursor + columns) is fully visible, the last row reached while rows stay fully visible; otherwise the last row reached while its top stays
///   within the list height above the cursor row. Past the last item, the last item. When cursor + columns is at least last item - 1, the last item.
/// - PageUp: 0 when cursor - columns < 1. If the row above is above the top (hidden), the row reached going up while its top stays within the list height of the cursor row; otherwise
///   the row reached while rows stay fully visible below the top. Fixed: the original compared "greater than the top", so it never went to a row exactly at the top (top = 0xdd).
#[allow(clippy::too_many_arguments)]
pub(super) fn thumbnail_page_target(tops: &[f32], heights: &[f32], columns: usize, offset: f32, view: f32, cursor: usize, count: usize, down: bool) -> usize {
    let columns = columns.max(1);
    if count == 0 {
        return 0;
    }
    let last = count - 1;
    let row_of = |i: usize| (i / columns).min(tops.len().saturating_sub(1));
    let top = |i: usize| tops.get(row_of(i)).copied().unwrap_or(0.0);
    let bottom = |i: usize| top(i) + heights.get(row_of(i)).copied().unwrap_or(0.0);
    let cursor = cursor.min(last);
    if down {
        if cursor + columns >= last {
            return last;
        }
        let next = cursor + columns;
        let mut u = if bottom(next) - offset < view { next } else { cursor };
        let fits = |v: usize, u0: usize| if u0 == next { bottom(v) - offset <= view } else { top(v) - top(cursor) <= view };
        let start = u;
        loop {
            let v = (u + columns).min(last);
            if !fits(v, start) {
                break;
            }
            u = v;
            if v >= last {
                break;
            }
        }
        u
    } else {
        if cursor < columns + 1 {
            return 0;
        }
        let prev = cursor - columns;
        let hidden = top(prev) - offset < 0.0;
        let mut u = if hidden { cursor } else { prev };
        loop {
            let v = u.saturating_sub(columns);
            let fits = if hidden { top(cursor) - top(v) <= view } else { top(v) - offset >= 0.0 };
            if !fits {
                break;
            }
            u = v;
            if v == 0 {
                break;
            }
        }
        u
    }
}

/// Ctrl (without Shift) keys of original ArrowKey: jump by counts of item kinds (TNXFileItem +0x3c: 1 folder, 3 file, 4 drive, FUN_007d2c54).
/// Ctrl+PageUp = first item (when there are folders), Ctrl+PageDown = last folder, Ctrl+Home = first file, Ctrl+End = last file (when there are files).
/// Other keys, or when that kind is missing, return None (the cursor does not move).
pub(super) fn ctrl_target(folders: usize, files: usize, drives: usize, count: usize, key: ListKey) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let target = match key {
        ListKey::PageUp if folders > 0 => 0,
        ListKey::PageDown if folders > 0 => folders - 1,
        ListKey::Home if files > 0 => folders,
        ListKey::End if files > 0 => count.saturating_sub(drives + 1),
        _ => return None,
    };
    Some(target.min(count - 1))
}

/// `[` (0xdb) and `]` (0xdd) of original ArrowKey: move between the drive items at the end of the list (kind 4, `drives` items).
/// `[`: the first drive when there are drives, the next item when the cursor is a drive and not the last item. None without drives.
/// `]`: the previous item when the cursor is a drive (the last item from the first drive), otherwise the last item.
pub(super) fn bracket_target(cursor: usize, count: usize, drives: usize, on_drive: bool, open: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let first_drive = count.saturating_sub(drives);
    let target = if open {
        if drives == 0 {
            return None;
        }
        if on_drive && cursor != count - 1 { cursor + 1 } else { first_drive }
    } else if on_drive {
        if cursor == first_drive { count - 1 } else { cursor.saturating_sub(1) }
    } else {
        count - 1
    };
    Some(target.min(count - 1))
}

/// Column layout of original UpdateDisplayAuto/UpdateDisplayNames: (rows per column 0x338, visible columns 0x33c, whether there is a horizontal bar).
/// `fits` is the number of columns that fit by one column width (Auto counts name and cell widths, Names the name width), `configured` the column count setting (0 = auto),
/// `rows_full`/`rows_with_bar` are rows per column without/with the horizontal bar. Auto uses one column when everything fits (drawn like Details).
pub(super) fn flow_layout(count: usize, fits: usize, configured: usize, rows_full: usize, rows_with_bar: usize, auto: bool) -> (usize, usize, bool) {
    let (rows_full, rows_with_bar) = (rows_full.max(1), rows_with_bar.max(1));
    let fewer = |rows: usize, columns: usize| {
        // If one column fewer gives the same page count, drop a column (columns get wider).
        let total = count.div_ceil(rows);
        if columns > 1 && total.div_ceil(columns - 1) == total.div_ceil(columns) { columns - 1 } else { columns }
    };
    if configured > 0 {
        let rows = if rows_full * configured < count { rows_with_bar } else { rows_full };
        return (rows, configured, rows_full * configured < count);
    }
    let fits = fits.max(1);
    if rows_full >= count || fits == 1 {
        let bar = rows_full < count;
        return (if bar { rows_with_bar } else { rows_full }, 1, bar);
    }
    let needed = count.div_ceil(rows_full);
    if auto && fits >= needed {
        return (rows_full, needed, false);
    }
    let bar = rows_full * fits < count;
    let rows = if bar { rows_with_bar } else { rows_full };
    (rows, fewer(rows, fits), bar)
}

/// The last column that can be leftmost in a column-flowing list. Original UpdateScrollBar (styles 0/1): by line columns - visible columns,
/// by page the first column of the last page (page index x visible columns; the right side may be empty).
pub(super) fn flow_last_first(total_columns: usize, columns: usize, by_page: bool) -> usize {
    let (total, columns) = (total_columns.max(1), columns.max(1));
    if by_page { ((total - 1) / columns) * columns } else { total.saturating_sub(columns) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Original ArrowKey style 3 PageUp/PageDown: to the last fully visible row when the row below is fully visible, otherwise one screen (tops within the list height).
    #[test]
    fn thumbnail_pages_follow_the_row_pixels() {
        // 3 columns, row heights 100, 150, 100, 100, 100 (tops 0, 100, 250, 350, 450), list height 300, top 0. 14 items (2 in the last row).
        let tops = [0.0, 100.0, 250.0, 350.0, 450.0];
        let heights = [100.0, 150.0, 100.0, 100.0, 100.0];
        let page = |cursor, offset, down| thumbnail_page_target(&tops, &heights, 3, offset, 300.0, cursor, 14, down);
        assert_eq!(page(1, 0.0, true), 4, "rows 0..1 fully visible: to the last fully visible row");
        assert_eq!(page(4, 0.0, true), 10, "on the last visible row: tops within one view height (250, 350)");
        assert_eq!(page(12, 0.0, true), 13, "near the end: the last item");
        assert_eq!(page(10, 250.0, false), 7, "row above visible: to the first fully visible row (fixed: >=)");
        assert_eq!(page(7, 250.0, false), 0, "row above hidden: tops within one view height, clamped to the first item");
        assert_eq!(page(2, 0.0, false), 0);
    }

    /// By page up to the first column of the last page (even with empty columns left), by line until the last column reaches the right edge.
    #[test]
    fn the_last_page_of_a_flow_list_is_reachable() {
        assert_eq!(flow_last_first(5, 2, true), 4, "pages start at 0, 2, 4");
        assert_eq!(flow_last_first(5, 2, false), 3);
        assert_eq!(flow_last_first(4, 2, true), 2);
        assert_eq!(flow_last_first(1, 3, true), 0);
        assert_eq!(flow_last_first(2, 3, false), 0);
    }

    fn flow(rows: usize, columns: usize, top: usize) -> ListPage {
        ListPage { style: ListStyle::Flow, rows, columns, top, last: 0 }
    }

    /// Original ArrowKey 0/1 (column-flowing list): Left/Right move by one column (rows per column); left of the first column goes one back, right of the last column one forward.
    #[test]
    fn flow_keys_follow_the_original_column_list() {
        let page = flow(10, 3, 0);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Down), 4);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Up), 2);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Right), 13);
        assert_eq!(arrow_target(&page, 13, 25, ListKey::Left), 3);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Left), 2, "first column: one back");
        // The last column (20-24) has 5 items: with 5 or fewer remaining, move one forward.
        assert_eq!(arrow_target(&page, 21, 25, ListKey::Right), 22);
        assert_eq!(arrow_target(&page, 17, 25, ListKey::Right), 24, "clamped to the last item");
        // Page: to the top when not at the top; at the top, one page back (rows x visible columns).
        let page = flow(10, 2, 20);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::PageUp), 20);
        assert_eq!(arrow_target(&page, 20, 60, ListKey::PageUp), 0);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::PageDown), 39);
        assert_eq!(arrow_target(&page, 39, 60, ListKey::PageDown), 59);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::Home), 0);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::End), 59);
    }

    /// Original ArrowKey 2 (Details) and 3 (Thumbnails).
    #[test]
    fn details_and_thumbnail_keys_follow_the_original() {
        let details = ListPage { style: ListStyle::Details, rows: 20, columns: 1, top: 40, last: 59 };
        assert_eq!(arrow_target(&details, 45, 100, ListKey::PageUp), 40);
        assert_eq!(arrow_target(&details, 40, 100, ListKey::PageUp), 20);
        assert_eq!(arrow_target(&details, 45, 100, ListKey::PageDown), 59);
        assert_eq!(arrow_target(&details, 59, 100, ListKey::PageDown), 79);
        assert_eq!(arrow_target(&details, 45, 100, ListKey::Left), 45);
        let thumbs = ListPage { style: ListStyle::Thumbnails, rows: 3, columns: 4, top: 0, last: 0 };
        assert_eq!(arrow_target(&thumbs, 5, 14, ListKey::Down), 9);
        assert_eq!(arrow_target(&thumbs, 5, 14, ListKey::Up), 1);
        assert_eq!(arrow_target(&thumbs, 2, 14, ListKey::Up), 1, "first row: one back");
        assert_eq!(arrow_target(&thumbs, 12, 14, ListKey::Down), 13, "last row: one forward");
        assert_eq!(arrow_target(&thumbs, 5, 14, ListKey::Right), 6);
    }

    /// Original ArrowKey `[` / `]`: a list of folder 1, file 2 and drives 3 (5..8).
    #[test]
    fn brackets_walk_the_drive_items() {
        assert_eq!(bracket_target(0, 6, 3, false, true), Some(3), "[ = first drive");
        assert_eq!(bracket_target(3, 6, 3, true, true), Some(4), "[ on a drive = next drive");
        assert_eq!(bracket_target(5, 6, 3, true, true), Some(3), "[ on the last drive wraps");
        assert_eq!(bracket_target(4, 6, 3, true, false), Some(3), "] on a drive = previous drive");
        assert_eq!(bracket_target(3, 6, 3, true, false), Some(5), "] on the first drive wraps");
        assert_eq!(bracket_target(1, 6, 3, false, false), Some(5), "] elsewhere = last item");
        assert_eq!(bracket_target(1, 3, 0, false, true), None, "[ without drives");
        // Ctrl+End = the last file before the drives.
        assert_eq!(ctrl_target(1, 2, 3, 6, ListKey::End), Some(2));
    }

    /// Original ArrowKey Ctrl keys: a list of 3 folders and 5 files (no drives).
    #[test]
    fn ctrl_keys_jump_between_folders_and_files() {
        assert_eq!(ctrl_target(3, 5, 0, 8, ListKey::PageUp), Some(0));
        assert_eq!(ctrl_target(3, 5, 0, 8, ListKey::PageDown), Some(2), "last folder");
        assert_eq!(ctrl_target(3, 5, 0, 8, ListKey::Home), Some(3), "first file");
        assert_eq!(ctrl_target(3, 5, 0, 8, ListKey::End), Some(7), "last file");
        assert_eq!(ctrl_target(0, 5, 0, 5, ListKey::PageDown), None, "no folders");
        assert_eq!(ctrl_target(3, 0, 0, 3, ListKey::Home), None, "no files");
        assert_eq!(ctrl_target(3, 5, 0, 8, ListKey::Up), None);
    }

    /// Original UpdateDisplayAuto/UpdateDisplayNames: column count and rows per column.
    #[test]
    fn flow_layout_follows_the_original_column_rules() {
        // Everything fits: one column.
        assert_eq!(flow_layout(8, 4, 0, 10, 9, true), (10, 1, false));
        // Auto: when more columns fit than are needed, only as many as needed (no horizontal bar).
        assert_eq!(flow_layout(25, 4, 0, 10, 9, true), (10, 3, false));
        // Names: the columns that fit, dropping one when one fewer gives the same page count.
        assert_eq!(flow_layout(25, 4, 0, 10, 9, false), (10, 3, false));
        // On overflow, a horizontal bar (rows per column decrease).
        assert_eq!(flow_layout(100, 3, 0, 10, 9, true), (9, 3, true));
        // Column count setting.
        assert_eq!(flow_layout(100, 3, 2, 10, 9, true), (9, 2, true));
        assert_eq!(flow_layout(15, 3, 2, 10, 9, false), (10, 2, false));
    }
}
