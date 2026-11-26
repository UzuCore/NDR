//! Original Advanced Select / Advanced Deselect (FUN_00438034, miAdvSelection): one TfGetText input box takes the filter.
use super::*;

/// Selection in original FUN_00438034: split the text on ';', match every list item name against each filter with PathMatchSpecW,
/// and select (or deselect) only matching files (kind 3). The previous selection is kept. An empty filter matches nothing.
pub(super) fn advanced_select_rows(entries: &[Entry], filters: &str, matches: impl Fn(&str, &str) -> bool) -> Vec<usize> {
    let mut rows = Vec::new();
    // Preserve mask-major order and the first match, without an O(N^2)
    // linear scan of the already selected row indices.
    let mut seen = vec![false; entries.len()];
    for spec in filters.split(';') {
        for (row, entry) in entries.iter().enumerate() {
            if !seen[row] && !entry.is_dir && !entry.drive && matches(&entry.name, spec) {
                seen[row] = true;
                rows.push(row);
            }
        }
    }
    rows
}

#[cfg(test)]
fn old_advanced_rows(entries: &[Entry], filters: &str) -> Vec<usize> {
    let mut rows = Vec::new();
    for spec in filters.split(';') {
        for (row,e) in entries.iter().enumerate() {
            if !e.is_dir && !e.drive && native::path_match_spec(&e.name,spec) && !rows.contains(&row) {rows.push(row);}
        }
    }
    rows
}

#[cfg(test)]
fn selection_fixture(count: usize) -> Vec<Entry> {
    (0..count).map(|i| {
        let name=format!("file{i:06}.{}",if i%2==0 {"txt"} else {"log"});
        Entry {path:PathBuf::from(&name),name,is_dir:i%17==0,drive:i%29==0,size:i as u64,modified:None,hidden:false,attributes:0}
    }).collect()
}

#[test]
fn advanced_selection_deduplicates_without_changing_mask_order() {
    let entries=selection_fixture(251);
    for pattern in ["","*.log;*.txt;*","*.txt;file000???.*;*.log","*;*;*.txt","FILE??????.TXT"] {
        assert_eq!(advanced_select_rows(&entries,pattern,native::path_match_spec),old_advanced_rows(&entries,pattern));
    }
}

#[test]
#[ignore = "explicit Release benchmark"]
fn benchmark_advanced_selection_20k() {
    let entries=selection_fixture(20000);
    let (mut old,mut new)=(Vec::new(),Vec::new());
    let pattern="*.log;*.txt;*;*.txt";
    for i in 0..9 {
        let before=||{let t=std::time::Instant::now();let r=old_advanced_rows(&entries,pattern);(t.elapsed().as_secs_f64()*1000.0,r)};
        let after=||{let t=std::time::Instant::now();let r=advanced_select_rows(&entries,pattern,native::path_match_spec);(t.elapsed().as_secs_f64()*1000.0,r)};
        let (a,b)=if i%2==0 {(before(),after())}else{let b=after();(before(),b)};
        assert_eq!(a.1,b.1);
        if i>0 {old.push(a.0);new.push(b.0);}
    }
    eprintln!("BATCH76 select20k old_ms={old:?} new_ms={new:?}");
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
