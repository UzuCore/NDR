//! 원본 TNXCustomFileListView::ArrowKey(0x7d69dc)의 커서 이동. 보기 형식(0x38c)마다 다르다:
//! 0 자동·1 이름 = 열로 흐르는 목록(위에서 아래로 한 열을 채우고 다음 열), 2 자세히, 3 미리 보기(왼쪽에서 오른쪽으로 한 줄).
//! 그린 목록이 쪽 정보(`ListPage`)를 남기고, 글쇠 처리가 그것으로 새 커서를 고른다.

/// 목록 그리기 형식.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ListStyle {
    /// 원본 2(자세히). 한 줄에 한 항목.
    #[default]
    Details,
    /// 원본 0·1(자동·이름): 항목 번호 / 한 열의 줄 수 = 열, 나머지 = 줄(GetItemRect).
    Flow,
    /// 원본 3(미리 보기): 한 줄에 칸 여러 개.
    Thumbnails,
}

/// 지난 그림의 쪽 정보(원본 0x338 한 열의 줄 수, 0x33c 보이는 열 수, 0xf1 맨 위 항목).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ListPage {
    pub style: ListStyle,
    /// Flow: 한 열의 줄 수(0x338). Details: 한 쪽에 다 보이는 줄 수(0xce). Thumbnails: 다 보이는 줄 수.
    pub rows: usize,
    /// Flow: 보이는 열 수(0x33c). Thumbnails: 한 줄의 칸 수(0xcf). Details: 1.
    pub columns: usize,
    /// 맨 위(처음) 보이는 항목(0xf1).
    pub top: usize,
    /// Details: 끝까지 다 보이는 마지막 항목.
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

/// 원본 ArrowKey: 지금 커서(cursor)와 항목 수(count)에서 글쇠(key)가 고르는 새 커서. 범위 밖이면 0 ~ count - 1로 자른다.
/// 자세히의 왼쪽·오른쪽은 원본처럼 커서를 옮기지 않는다(원본은 가로 막대가 있을 때 20만큼 가로로 넘긴다).
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
            // 마지막 열에 든 항목 수(0이면 한 열이 꽉 참).
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

/// 원본 ArrowKey(0x7d69dc) 형식 3의 PageUp·PageDown(줄마다 높이가 다른 미리 보기). `tops`·`heights` = 줄마다 위·높이(내용 좌표,
/// 항목 +0x6c·GetThumbnailRowHeight), `offset` = 맨 위 위치(0xdd), `view` = 목록 높이(GetRealClientHeight).
/// - PageDown: 아래 줄(커서 + 열 수)이 다 보이면 다 보이는 동안 한 줄씩 내려간 마지막 줄, 아니면 위가 커서 줄 위에서 목록 높이 안인
///   동안 내려간 마지막 줄. 마지막 항목을 넘으면 마지막 항목. 커서 + 열 수가 마지막 항목 − 1 이상이면 마지막 항목.
/// - PageUp: 커서 − 열 수 < 1이면 0. 위 줄이 맨 위보다 위(가려짐)면 위가 커서 줄 위에서 목록 높이 안인 동안 올라간 줄, 아니면
///   맨 위 아래에 다 보이는 동안 올라간 줄. 고친 것: 원본은 이 비교가 "맨 위보다 큼"이라 맨 위에 딱 붙은 줄(위 = 0xdd)로는 가지 않았다.
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

/// 원본 ArrowKey의 Ctrl(Shift 없이) 글쇠: 항목 종류(TNXFileItem +0x3c: 1 폴더, 3 파일, 4 드라이브, FUN_007d2c54) 수로 건너뛴다.
/// Ctrl+PageUp = 첫 항목(폴더가 있을 때), Ctrl+PageDown = 마지막 폴더, Ctrl+Home = 첫 파일, Ctrl+End = 마지막 파일(파일이 있을 때).
/// 그 밖의 글쇠나 해당 종류가 없으면 None(커서를 옮기지 않는다).
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

/// 원본 ArrowKey의 `[`(0xdb)·`]`(0xdd): 목록 끝 드라이브 항목(종류 4, `drives`개) 사이를 옮긴다.
/// `[`: 드라이브가 있으면 첫 드라이브, 커서가 드라이브이고 마지막 항목이 아니면 다음 항목. 드라이브가 없으면 None.
/// `]`: 커서가 드라이브면 이전 항목(첫 드라이브면 마지막 항목), 아니면 마지막 항목.
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

/// 원본 UpdateDisplayAuto/UpdateDisplayNames의 열 배치: (한 열의 줄 수 0x338, 보이는 열 수 0x33c, 가로 막대가 있는지).
/// `fits`는 한 열 너비로 들어가는 열 수(자동은 이름·칸 너비, 이름은 이름 너비로 셈), `configured`는 열 수 설정(0 = 자동),
/// `rows_full`/`rows_with_bar`는 가로 막대가 없을 때/있을 때 한 열에 들어가는 줄 수. 자동은 다 들어가면 1열(자세히처럼 그린다).
pub(super) fn flow_layout(count: usize, fits: usize, configured: usize, rows_full: usize, rows_with_bar: usize, auto: bool) -> (usize, usize, bool) {
    let (rows_full, rows_with_bar) = (rows_full.max(1), rows_with_bar.max(1));
    let fewer = |rows: usize, columns: usize| {
        // 한 열 덜 써도 쪽 수가 같으면 한 열을 뺀다(열이 넓어진다).
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

/// 열로 흐르는 목록에서 맨 왼쪽에 올 수 있는 마지막 열. 원본 UpdateScrollBar(형식 0·1): 줄 단위면 열 수 − 보이는 열 수,
/// 쪽 단위면 마지막 쪽의 처음 열(쪽 번호 × 보이는 열 수, 오른쪽이 빌 수 있다).
pub(super) fn flow_last_first(total_columns: usize, columns: usize, by_page: bool) -> usize {
    let (total, columns) = (total_columns.max(1), columns.max(1));
    if by_page { ((total - 1) / columns) * columns } else { total.saturating_sub(columns) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 원본 ArrowKey 형식 3 PageUp·PageDown: 아래 줄이 다 보이면 마지막으로 다 보이는 줄까지, 아니면 한 화면(위끼리 목록 높이 안).
    #[test]
    fn thumbnail_pages_follow_the_row_pixels() {
        // 3열, 줄 높이 100·150·100·100·100(위 0, 100, 250, 350, 450), 목록 높이 300, 맨 위 0. 항목 14개(마지막 줄 2개).
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

    /// 쪽 단위는 마지막 쪽(빈 열이 남아도)의 처음 열까지, 줄 단위는 마지막 열이 오른쪽 끝에 오는 데까지.
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

    /// 원본 ArrowKey 0·1(열로 흐르는 목록): 왼쪽·오른쪽은 한 열(한 열의 줄 수)씩, 첫 열 왼쪽은 하나 앞, 마지막 열 오른쪽은 하나 뒤.
    #[test]
    fn flow_keys_follow_the_original_column_list() {
        let page = flow(10, 3, 0);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Down), 4);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Up), 2);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Right), 13);
        assert_eq!(arrow_target(&page, 13, 25, ListKey::Left), 3);
        assert_eq!(arrow_target(&page, 3, 25, ListKey::Left), 2, "first column: one back");
        // 마지막 열(20~24)은 5개: 남은 수가 5 이하이면 하나 뒤로.
        assert_eq!(arrow_target(&page, 21, 25, ListKey::Right), 22);
        assert_eq!(arrow_target(&page, 17, 25, ListKey::Right), 24, "clamped to the last item");
        // 쪽: 맨 위가 아니면 맨 위로, 맨 위면 한 쪽(줄 수 × 보이는 열 수) 앞.
        let page = flow(10, 2, 20);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::PageUp), 20);
        assert_eq!(arrow_target(&page, 20, 60, ListKey::PageUp), 0);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::PageDown), 39);
        assert_eq!(arrow_target(&page, 39, 60, ListKey::PageDown), 59);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::Home), 0);
        assert_eq!(arrow_target(&page, 25, 60, ListKey::End), 59);
    }

    /// 원본 ArrowKey 2(자세히)·3(미리 보기).
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

    /// 원본 ArrowKey `[`·`]`: 폴더 1·파일 2·드라이브 3(5..8) 목록.
    #[test]
    fn brackets_walk_the_drive_items() {
        assert_eq!(bracket_target(0, 6, 3, false, true), Some(3), "[ = first drive");
        assert_eq!(bracket_target(3, 6, 3, true, true), Some(4), "[ on a drive = next drive");
        assert_eq!(bracket_target(5, 6, 3, true, true), Some(3), "[ on the last drive wraps");
        assert_eq!(bracket_target(4, 6, 3, true, false), Some(3), "] on a drive = previous drive");
        assert_eq!(bracket_target(3, 6, 3, true, false), Some(5), "] on the first drive wraps");
        assert_eq!(bracket_target(1, 6, 3, false, false), Some(5), "] elsewhere = last item");
        assert_eq!(bracket_target(1, 3, 0, false, true), None, "[ without drives");
        // Ctrl+End = 드라이브 앞 마지막 파일.
        assert_eq!(ctrl_target(1, 2, 3, 6, ListKey::End), Some(2));
    }

    /// 원본 ArrowKey Ctrl 글쇠: 폴더 3개·파일 5개(드라이브 없음) 목록.
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

    /// 원본 UpdateDisplayAuto/UpdateDisplayNames: 열 수와 한 열의 줄 수.
    #[test]
    fn flow_layout_follows_the_original_column_rules() {
        // 다 들어가면 한 열.
        assert_eq!(flow_layout(8, 4, 0, 10, 9, true), (10, 1, false));
        // 자동: 들어가는 열이 필요한 열보다 많으면 필요한 만큼(가로 막대 없음).
        assert_eq!(flow_layout(25, 4, 0, 10, 9, true), (10, 3, false));
        // 이름: 들어가는 열 수, 한 열 덜 써도 쪽 수가 같으면 한 열 뺌.
        assert_eq!(flow_layout(25, 4, 0, 10, 9, false), (10, 3, false));
        // 넘치면 가로 막대(한 열의 줄 수가 줄어든다).
        assert_eq!(flow_layout(100, 3, 0, 10, 9, true), (9, 3, true));
        // 열 수 설정.
        assert_eq!(flow_layout(100, 3, 2, 10, 9, true), (9, 2, true));
        assert_eq!(flow_layout(15, 3, 2, 10, 9, false), (10, 2, false));
    }
}
