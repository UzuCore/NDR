//! Original Advanced Select / Advanced Deselect (FUN_00438034, miAdvSelection): one TfGetText input box takes the filter.
use super::*;

/// Selection in original FUN_00438034: split the text on ';', match every list item name against each filter with PathMatchSpecW,
/// and select (or deselect) only matching files (kind 3). The previous selection is kept. An empty filter matches nothing.
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
    /// Original FUN_00438034: title TITLE_ADVSELECT (select) / TITLE_ADVDESELECT (deselect), message MSG_ADVSELECT, empty initial text.
    /// Selects only on OK with non-empty text.
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

    /// Marks or unmarks files matching the filter (native::path_match_spec = PathMatchSpecW). Keeps the current selection for Restore Selection.
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
