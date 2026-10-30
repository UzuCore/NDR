//! 원본 고급 선택 / 고급 해제(FUN_00438034, miAdvSelection): TfGetText 입력 상자 하나로 필터를 받는다.
use super::*;

/// 원본 FUN_00438034의 고르기: 글을 ';'로 나눠 필터마다 목록의 모든 항목 이름을 PathMatchSpecW로 맞추고,
/// 맞으면서 파일(종류 3)인 항목만 고르거나(select) 푼다. 이전 선택은 그대로 둔다. 빈 필터는 아무것도 맞지 않는다.
pub(super) fn advanced_select_rows(entries: &[Entry], filters: &str, matches: impl Fn(&str, &str) -> bool) -> Vec<usize> {
    let mut rows = Vec::new();
    for spec in filters.split(';') {
        for (row, entry) in entries.iter().enumerate() {
            if !entry.is_dir && !entry.drive && matches(&entry.name, spec) && !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    rows
}

impl App {
    /// 원본 FUN_00438034: 제목 TITLE_ADVSELECT(고르기) / TITLE_ADVDESELECT(풀기), 글 MSG_ADVSELECT, 처음 값은 빈 글.
    /// 확인이고 글이 비어 있지 않을 때만 고른다.
    pub(super) fn advanced_select(&mut self, deselect: bool) {
        let i = self.active;
        let title = if deselect {
            self.language_pack.text("RES.TITLE_ADVDESELECT", "Advanced Deselection")
        } else {
            self.language_pack.text("RES.TITLE_ADVSELECT", "Advanced Selection")
        };
        let message = self.language_pack.text("RES.MSG_ADVSELECT", "Type filters you want to use with semi-colon separator.\nex) *.exe;*.txt;today???.xls");
        self.prompt(&title, &message, "", None, false, move |app, value, _| {
            let Some(filters) = value.filter(|v| !v.is_empty()) else { return };
            app.apply_advanced_select(i, &filters, !deselect);
        });
    }

    /// 필터에 맞는 파일을 표시하거나 푼다(native::path_match_spec = PathMatchSpecW). 되돌리기(이전 선택 복원)를 위해 지금 선택을 남긴다.
    pub(super) fn apply_advanced_select(&mut self, i: usize, filters: &str, select: bool) {
        self.last_selection[i] = self.panes[i].targets();
        let pane = &mut self.panes[i];
        for row in advanced_select_rows(&pane.entries, filters, native::path_match_spec) {
            if select {
                pane.selected.insert(row);
            } else {
                pane.selected.remove(&row);
            }
        }
    }
}
