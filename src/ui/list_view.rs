//! C# Details list (ConfigureGrid columns, FileNameDisplayConverter, ExtensionColorConverter, group headers).
//! Column texts are built once when the list is received or rearranged, like C# (date ages use the value at that time).
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DetailColumn {
    Name,
    Extension,
    Kind,
    Size,
    Modified,
    Attributes,
    Description,
}

impl DetailColumn {
    pub(super) fn index(self) -> usize {
        self as usize
    }
    /// Sort mode when a header is clicked (the C# SortMemberPath entries that have a sort mode).
    pub(super) fn sort_mode(self) -> Option<&'static str> {
        match self {
            DetailColumn::Name => Some("Name"),
            DetailColumn::Extension => Some("Extension"),
            DetailColumn::Size => Some("Size"),
            DetailColumn::Modified => Some("Modified"),
            DetailColumn::Attributes => Some("Attributes"),
            DetailColumn::Kind | DetailColumn::Description => None,
        }
    }
}

pub(super) use super::detail_rows::{DetailRow, DetailRows};

/// Immutable local display arrays can be shared by identical pane snapshots.
/// Mutation detaches the array, never the other pane's selection or rows.
/// Sensitive views never enter the explicit twin sharing path.
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct SharedRows<T:Clone>(Arc<Vec<T>>);
impl<T:Clone> Default for SharedRows<T>{
    fn default()->Self{Self(Arc::new(Vec::new()))}
}
impl<T:Clone> std::ops::Deref for SharedRows<T>{
    type Target=Vec<T>;
    fn deref(&self)->&Self::Target{&self.0}
}
impl<T:Clone> std::ops::DerefMut for SharedRows<T>{
    fn deref_mut(&mut self)->&mut Self::Target{Arc::make_mut(&mut self.0)}
}
impl<T:Clone> SharedRows<T>{
    fn reset(&mut self,count:usize){
        if let Some(rows)=Arc::get_mut(&mut self.0){reset_view_buffer(rows,count)}
        else{self.0=Arc::new(Vec::with_capacity(count));}
    }
}
impl<T:Clone> IntoIterator for SharedRows<T>{
    type Item=T;
    type IntoIter=std::vec::IntoIter<T>;
    fn into_iter(self)->Self::IntoIter{Arc::unwrap_or_clone(self.0).into_iter()}
}
impl<'a,T:Clone> IntoIterator for &'a SharedRows<T>{
    type Item=&'a T;
    type IntoIter=std::slice::Iter<'a,T>;
    fn into_iter(self)->Self::IntoIter{self.0.iter()}
}
impl<'a,T:Clone> IntoIterator for &'a mut SharedRows<T>{
    type Item=&'a mut T;
    type IntoIter=std::slice::IterMut<'a,T>;
    fn into_iter(self)->Self::IntoIter{Arc::make_mut(&mut self.0).iter_mut()}
}

/// Per list row the column texts (7 cells, DetailColumn order) and name color, plus the row order with group headers inserted.
#[derive(Default)]
pub(super) struct PaneView {
    // Repeated extension, kind, size, date and attribute texts share
    // immutable allocations. Vault rows deliberately use unique allocations.
    pub cells: SharedRows<[Arc<str>; 7]>,
    pub colors: SharedRows<Option<(u8, u8, u8)>>,
    pub rows: DetailRows,
    sensitive: bool,
}

impl PaneView {
    pub(super) fn clone_local_shared(&self)->Self{
        assert!(!self.sensitive,"Vault cells cannot be shared");
        Self{cells:self.cells.clone(),colors:self.colors.clone(),rows:self.rows.clone(),sensitive:false}
    }
    // Every sensitive cell is uniquely owned. Ordinary lists may share
    // non-sensitive text; they are only released, not overwritten here.
    pub(super) fn wipe_sensitive_cells(&mut self) {
        if !self.sensitive{return;}
        for row in &mut self.cells {
            for cell in row {
                if let Some(text)=Arc::get_mut(cell) {
                    // SAFETY: replacing UTF-8 bytes with NULs preserves UTF-8.
                    use zeroize::Zeroize;
                    unsafe { text.as_bytes_mut().zeroize(); }
                } else {
                    debug_assert!(cell.is_empty(),"shared sensitive cell");
                }
            }
        }
    }
}

/// Reuse row buffers on refresh, but release oversized allocations
/// when navigating away from a large directory.
fn reset_view_buffer<T>(rows: &mut Vec<T>, count: usize) {
    if rows.capacity() > count.saturating_mul(2).max(1024) {
        *rows = Vec::with_capacity(count);
    } else {
        rows.clear();
        rows.reserve(count);
    }
}

/// Adaptive per-rebuild cache. High-cardinality listings should not pay
/// for hashing/cloning unique values. Re-sample periodically so a later
/// repetitive region (e.g. grouped folders) can still benefit.
struct CellCache<K,V=String> {
    entries: HashMap<K, V>,
    samples: usize,
    hits: usize,
    skip: usize,
}

impl<K,V> Default for CellCache<K,V> {
    fn default() -> Self {
        Self { entries: HashMap::new(), samples: 0, hits: 0, skip: 0 }
    }
}

fn cached_cell<K: Eq + std::hash::Hash,V:Clone>(cache: &mut CellCache<K,V>, key: K, format: impl FnOnce() -> V) -> V {
    if cache.skip != 0 {
        cache.skip -= 1;
        return format();
    }
    cache.samples += 1;
    // One hash-table probe on admission instead of get followed by insert.
    let value = if cache.entries.len() < 4096 {
        match cache.entries.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                cache.hits += 1;
                entry.get().clone()
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let value = format();
                entry.insert(value.clone());
                value
            }
        }
    } else if let Some(value) = cache.entries.get(&key) {
        cache.hits += 1;
        value.clone()
    } else { format() };
    if cache.samples == 128 {
        if cache.hits < 8 {
            cache.entries.clear();
            cache.skip = 4096;
        }
        cache.samples = 0;
        cache.hits = 0;
    }
    value
}

fn shared_cell<K:Copy+Eq+std::hash::Hash>(cache:&mut CellCache<K,Arc<str>>,
    last:&mut Option<(K,Arc<str>)>,key:K,format:impl FnOnce()->Arc<str>)->Arc<str>{
    if let Some((old,value))=last.as_ref(){if *old==key{return value.clone();}}
    let value=cached_cell(cache,key,format);
    *last=Some((key,value.clone()));value
}

#[cfg(test)]
#[test]
fn repeated_cells_share_and_sensitive_cells_are_unique(){
    let mut cache=CellCache::<u64,Arc<str>>::default();
    let mut last=None;
    let a=shared_cell(&mut cache,&mut last,1,||Arc::from("same"));
    let b=shared_cell(&mut cache,&mut last,1,||panic!("repeated"));
    assert!(Arc::ptr_eq(&a,&b));
    let mut view=PaneView::default();
    view.cells.push(std::array::from_fn(|_|Arc::from("secret")));
    view.sensitive=true;
    assert!(view.cells.iter().flatten().all(|text|Arc::strong_count(text)==1));
    view.wipe_sensitive_cells();
    assert!(view.cells.iter().flatten().all(|text|text.as_bytes().iter().all(|&b|b==0)));
}

#[cfg(test)]
#[test]
fn local_twin_arrays_share_and_detach_without_copying_unchanged_columns(){
    let mut a=PaneView::default();
    a.rows.reset(4096,true);
    for i in 0..4096{
        a.cells.push(std::array::from_fn(|_|Arc::from(format!("row-{i}"))));
        a.colors.push(Some((10,20,30)));a.rows.push(DetailRow::Item(i));
    }
    let mut b=a.clone_local_shared();
    assert_eq!(a.cells.as_ptr(),b.cells.as_ptr());
    assert_eq!(a.colors.as_ptr(),b.colors.as_ptr());
    assert_eq!(a.rows.explicit_ptr(),b.rows.explicit_ptr());
    // Changing one cell detaches only the changed array. Immutable texts
    // for the unaffected rows are still shared.
    b.cells[0][3]=Arc::from("123");
    assert_ne!(a.cells.as_ptr(),b.cells.as_ptr());
    assert_eq!(a.cells[0][3].as_ref(),"row-0");
    assert_eq!(b.cells[0][3].as_ref(),"123");
    assert!(Arc::ptr_eq(&a.cells[20][0],&b.cells[20][0]));
    assert_eq!(a.colors.as_ptr(),b.colors.as_ptr());
    let old=a.rows.explicit_ptr();b.rows.reset(0,false);
    assert_eq!(a.rows.explicit_ptr(),old);assert_eq!(a.rows.len(),4096);assert_eq!(b.rows.len(),0);
    b.cells.reset(1);assert!(b.cells.is_empty());assert_eq!(a.cells.len(),4096);
}
#[cfg(test)]
#[test]
fn view_cache_and_buffers_are_bounded_and_reusable() {
    let mut rows = vec![String::from("old"); 100_000];
    let ptr = rows.as_ptr();
    reset_view_buffer(&mut rows, 100_000);
    assert!(rows.is_empty());
    assert_eq!(rows.as_ptr(), ptr);
    reset_view_buffer(&mut rows, 3);
    assert!(rows.capacity() <= 1024);
    let mut cache = CellCache::default();
    for i in 0..5000 {
        assert_eq!(cached_cell(&mut cache, i, || i.to_string()), i.to_string());
    }
    assert!(cache.entries.len() <= 128 && cache.skip > 0);
    for _ in 0..cache.skip {
        cached_cell(&mut cache, 17, || "17".to_owned());
    }
    for _ in 0..512 {
        assert_eq!(cached_cell(&mut cache, 17, || "17".to_owned()), "17");
    }
    assert_eq!(cached_cell(&mut cache, 17, || panic!("cache hit after re-probe")), "17");
    // A highly repetitive workload can still admit up to the same cap,
    // without sacrificing the exact text of uncached unique rows.
    for i in 0..5000 {
        assert_eq!(cached_cell(&mut cache, i, || i.to_string()), i.to_string());
        assert_eq!(cached_cell(&mut cache, i, || i.to_string()), i.to_string());
    }
    assert!(cache.entries.len() <= 4096);
}

#[cfg(test)]
#[test]
#[ignore = "explicit Release benchmark of bounded display text caches"]
fn benchmark_display_cell_cache_100k() {
    fn legacy_cell<K: Eq + std::hash::Hash>(cache: &mut CellCache<K>, key: K, format: impl FnOnce() -> String) -> String {
        if cache.skip != 0 { cache.skip -= 1; return format(); }
        cache.samples += 1;
        let value = if let Some(value) = cache.entries.get(&key) {
            cache.hits += 1; value.clone()
        } else {
            let value = format();
            if cache.entries.len() < 4096 { cache.entries.insert(key, value.clone()); }
            value
        };
        if cache.samples == 128 {
            if cache.hits < 8 { cache.entries.clear(); cache.skip = 4096; }
            cache.samples = 0; cache.hits = 0;
        }
        value
    }
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let now = SystemTime::now();
    for spread in ["repeated", "unique", "blocks", "mixed", "date-unique", "size-unique", "abrupt"] {
        let inputs: Vec<_> = (0..100000u64).map(|i| {
            let slot = match spread {
                "repeated" => i % 64,
                "unique" => i,
                "blocks" if i / 10_000 % 2 == 0 => i % 64,
                "mixed" if i % 3 != 0 => i % 64,
                "abrupt" if i / 6000 % 2 == 1 => i % 64,
                _ => i + 100_000,
            };
            let date_slot = if spread == "size-unique" { i % 64 } else { slot };
            let size_slot = if spread == "date-unique" { i % 64 } else { slot };
            (Some(UNIX_EPOCH + Duration::from_secs(1_600_000_000 + date_slot * 3607)), size_slot * 301)
        }).collect();
        let mut before_ms = Vec::new();
        let mut after_ms = Vec::new();
        let mut prior_ms = Vec::new();
        let date = |modified| ndr::listing::list_date_text(modified, "yyyy-MM-dd", "HH:mm", Some(["Today", "Yesterday", "%d days ago"]), now);
        let size = |size| ndr::parity::size_to_str(size, "Auto", true);
        for i in 0..9 {
            let before = || {
                let t=Instant::now();
                let rows: Vec<_>=inputs.iter().map(|&(d,s)| (date(d),size(s))).collect();
                (t.elapsed().as_secs_f64()*1000.0, rows)
            };
            let prior = || {
                let t=Instant::now();
                let (mut dates,mut sizes)=(CellCache::default(),CellCache::default());
                let rows: Vec<_>=inputs.iter().map(|&(d,s)| (
                    legacy_cell(&mut dates,d,||date(d)),
                    legacy_cell(&mut sizes,s,||size(s)),
                )).collect();
                (t.elapsed().as_secs_f64()*1000.0,rows)
            };
            let after = || {
                let t=Instant::now();
                let (mut dates,mut sizes)=(CellCache::default(),CellCache::default());
                let mut formatter=ndr::listing::ListDateFormatter::new("yyyy-MM-dd", "HH:mm", Some(["Today", "Yesterday", "%d days ago"]), now);
                let rows: Vec<_>=inputs.iter().map(|&(d,s)| (
                    cached_cell(&mut dates,d,||formatter.format(d)),
                    cached_cell(&mut sizes,s,||size(s)),
                )).collect();
                (t.elapsed().as_secs_f64()*1000.0,rows)
            };
            let (a,p,b)=match i%3 {0=>(before(),prior(),after()),1=>{let p=prior();let b=after();(before(),p,b)},_=>{let b=after();let a=before();(a,prior(),b)}};
            assert_eq!(a.1,b.1);
            assert_eq!(a.1,p.1);
            if i>0 {before_ms.push(a.0);prior_ms.push(p.0);after_ms.push(b.0);}
        }
        eprintln!("BATCH95 cells100k spread={spread} uncached_ms={before_ms:?} build23_ms={prior_ms:?} new_ms={after_ms:?}");
    }
}

/// CharacterCase of C# FileNameDisplayConverter: 0 as is, 1 lower, 2 upper, 3 first letter upper.
/// SFTP permission bits as list text ("drwxr-xr-x"): original OnGetAttrString (FUN_00485aa0) shows the row's permission text (+0x74) as is on FTP panes,
/// otherwise GetAttrString (width "drwxrwxrwx"). The SFTP adapter builds it in FUN_0045b900: "----------" (0x8ffa42), 'd' for a folder, then
/// r/w/x from bits 0x100..0x1.
pub(super) fn permission_text(is_dir: bool, mode: u32) -> String {
    let mut text = String::from(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        text.push(if bits & 4 != 0 { 'r' } else { '-' });
        text.push(if bits & 2 != 0 { 'w' } else { '-' });
        text.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    text
}

/// Original StrCharCase (0x7db714): 1 = CharLowerW, 2 = CharUpperW, 3 = FUN_007d223c (after CharLowerW, only a-z at the start or after a space become upper).
/// Fixed: previously 3 uppercased only the first character of the whole text and left the rest alone.
pub(super) fn character_case(text: &str, mode: i64) -> String {
    match mode {
        1 => text.to_lowercase(),
        2 => text.to_uppercase(),
        3 => {
            let mut previous = ' ';
            text.to_lowercase()
                .chars()
                .map(|c| {
                    let out = if previous == ' ' && c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c };
                    previous = c;
                    out
                })
                .collect()
        }
        _ => text.into(),
    }
}

#[cfg(test)]
mod character_case_tests {
    #[test]
    fn title_case_follows_fun_007d223c() {
        assert_eq!(super::character_case("my REPORT file", 3), "My Report File");
        assert_eq!(super::character_case("a-b c", 3), "A-b C", "only after a space");
        assert_eq!(super::character_case("éclair été", 3), "éclair été", "a-z only");
        assert_eq!((super::character_case("AbC", 1), super::character_case("AbC", 2)), ("abc".to_owned(), "ABC".to_owned()));
    }
}

impl App {
    /// Visible columns, headers and width ratios (C# star widths 2.4, 0.55, 1.0, 0.8, 1.2, 0.55, 1.4).
    /// Column titles = original FIELD_NAME/EXT/SIZE/DATEMODIFIED/ATTR/DESC (TNXCustomFileListView headers). The "type" column exists only in the C# version.
    /// GetFieldTitle (0x7e2760) takes the field's English name from the table at 0x92d428 (Name, Ext, Location, Size, Date modified, Attr,
    /// Description) and passes it through GetString (0x7e2134, the OnGetString event = the language file); Location stays hidden in panes.
    pub(super) fn detail_columns(&self, _ko: bool) -> Vec<(DetailColumn, String, f32)> {
        let field = |key: &str, fallback: &str| self.language_pack.text(&format!("RES.{key}"), fallback);
        let mut columns = vec![(DetailColumn::Name, field("FIELD_NAME", "Name"), 2.4)];
        if !self.settings.flag("AttachExtensionToName", false) {
            columns.push((DetailColumn::Extension, field("FIELD_EXT", "Ext"), 0.55));
        }
        // The original has no "type" column; with ShowFileType (original showfiletype) it shows the type in an empty description cell.
        columns.extend([
            (DetailColumn::Size, field("FIELD_SIZE", "Size"), 0.8),
            (DetailColumn::Modified, field("FIELD_DATEMODIFIED", "Date modified"), 1.2),
            (DetailColumn::Attributes, field("FIELD_ATTR", "Attr"), 0.55),
            (DetailColumn::Description, field("FIELD_DESC", "Description"), 1.4),
        ]);
        columns
    }

    /// C# GroupHeaderNameConverter. Difference (approved fix): C# Get("Caption", key) cannot find the [formNDRSearch] section of the language file and
    /// shows English names, while Rust uses that section's translation.
    pub(super) fn group_caption(&self, key: &str) -> String {
        match ndr::listing::group_caption_key(key) {
            (label, Some("RES.FOLDERS")) => self.language_pack.text("RES.FOLDERS", label),
            (label, Some(caption)) => self.language_pack.text(&format!("formNDRSearch.Caption.{caption}"), label),
            (label, None) => label.to_owned(),
        }
    }

    /// The same builder handles UI re-sorts and worker-prepared listings.
    pub(super) fn compute_view(&mut self, index: usize) {
        let plan = ViewPlan::capture(self, index);
        let view = std::mem::take(&mut self.panes[index].view);
        let pane = &self.panes[index];
        let sftp_modes = pane.remote.as_ref().filter(|r| r.is_sftp()).and_then(|r| r.modes.lock().ok().map(|m| (r.path.clone(), m.clone())));
        let ftp_permissions = pane.remote.as_ref().filter(|r| !r.is_sftp()).and_then(|r| r.permissions.lock().ok().map(|m| (r.path.clone(), m.clone())));
        let local = pane.archive.is_none() && pane.remote.is_none();
        let view = build_view(&plan, &pane.entries, &pane.descriptions, &pane.drive_infos,
            sftp_modes, ftp_permissions, local, local && pane.vault.is_none(), view, None).expect("uncancellable UI view");
        self.panes[index].view = view;
        self.panes[index].prepared_field_max = None;
        self.panes[index].thumb_content_revision = self.panes[index].thumb_content_revision.wrapping_add(1);
    }

    /// 0x368 of original UpdateDisplay (0x7e01f8): the widest drive item name text, from the item's left edge to the end of that text
    /// (head width + 5 + text, the same margin as `drive_row`). 0 without drive items.
    pub(super) fn drive_text_extent(&self, ui: &egui::Ui, index: usize) -> f32 {
        let pane = &self.panes[index];
        let drives = pane.drive_count();
        if drives == 0 || pane.entries.len() != pane.view.cells.len() {
            return 0.0;
        }
        let icons = self.settings.flag("ShowIcons", true);
        let font = egui::TextStyle::Button.resolve(ui.style());
        let widest = ui.fonts_mut(|fonts| {
            pane.view.cells.iter().rev().take(drives)
                .map(|cells| fonts.layout_no_wrap(cells[0].to_string(), font.clone(), Color32::WHITE).size().x)
                .fold(0.0f32, f32::max)
        });
        App::list_header_width(icons) + 5.0 + widest
    }
}

/// Only presentation-affecting values are copied into a worker snapshot.
/// No user histories, settings file I/O, or egui state is needed there.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ViewPlan {
    attach: bool,
    case: i64,
    date_format: String,
    time_format: String,
    use_age: bool,
    age_labels: [String; 3],
    unit: String,
    thousands: bool,
    folder_size_text: String,
    vault_type: String,
    folder_type: String,
    pub(super) group_mode: String,
    pub(super) category_mode: String,
    show_group_name: bool,
    show_file_type: bool,
    folder: (u8, u8, u8),
    drive: (u8, u8, u8),
    fore: (u8, u8, u8),
    pub(super) extension_colors: Option<serde_json::Map<String, serde_json::Value>>,
    group_labels: HashMap<String, String>,
}
impl ViewPlan {
    pub(super) fn capture(app: &App, index: usize) -> Self {
        let group_mode = app.side_text(index, "GroupMode", "None");
        let show_group_name = app.side_flag(index, "ShowGroupName", true);
        let group_labels = if group_mode != "None" && show_group_name {
            ["Folders","Archives","Documents","Spreadsheets","Presentations","Images","Audio","Video","Programs","Code","Other",
                "No extension","Unknown date","Empty","Under 1 KB","1 KB\u{2013}1 MB","1\u{2013}100 MB","100 MB\u{2013}1 GB","1 GB and larger"]
                .into_iter().map(|label| (label.to_owned(), app.group_caption(label))).collect()
        } else { HashMap::new() };
        let rgb = |key: &str| { let c = app.basic_list_color(key); (c.r(), c.g(), c.b()) };
        Self {
            attach: app.settings.flag("AttachExtensionToName", false),
            case: app.settings.values.get("CharacterCase").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 3),
            date_format: app.settings.text("DateFormat", "yyyy-MM-dd"),
            time_format: app.settings.text("TimeFormat", "HH:mm"),
            use_age: app.settings.flag("UseDateAge", true),
            age_labels: [
                app.language_pack.text("RES.TODAY", "Today"),
                app.language_pack.text("RES.YESTERDAY", "Yesterday"),
                app.language_pack.text("RES.DATEAGE", "%d days ago"),
            ],
            unit: app.settings.text("SizeUnit", "Auto"),
            thousands: app.settings.flag("UseThousandsSeparator", true),
            folder_size_text: app.language_pack.text("RES.DIR", "[DIR]"),
            vault_type: app.language_pack.text("NDRVault.TextEditorTitle", "Encrypted folder"),
            folder_type: app.language_pack.text("RES.FOLDER", "Folder"),
            group_mode, category_mode: app.side_text(index, "FolderCategoryMode", "Auto"),
            show_group_name, show_file_type: app.settings.flag("ShowFileType", true),
            folder: rgb("NXFOLDER"), drive: rgb("NXDRIVE"), fore: rgb("NXFORE"),
            extension_colors: app.color_maps().0.cloned(), group_labels,
        }
    }
}

pub(super) fn build_view(plan: &ViewPlan, entries: &[Entry], descriptions: &HashMap<String, String>,
    drive_infos: &HashMap<PathBuf, (String, u32, Option<(u64, u64)>)>,
    sftp_modes: Option<(String, HashMap<String, u32>)>, ftp_permissions: Option<(String, HashMap<String, String>)>,
    local: bool, detect_vault: bool, mut view: PaneView, cancel: Option<&AtomicBool>) -> Option<PaneView> {
    let ViewPlan { attach, case, date_format, time_format, use_age, age_labels, unit, thousands,
        folder_size_text, vault_type, folder_type, group_mode, category_mode, show_group_name,
        show_file_type, folder, drive, fore, extension_colors, group_labels } = plan.clone();
    let basic = move |key: &str| match key {
        "NXFOLDER" => folder, "NXDRIVE" => drive, _ => fore,
    };
    let sensitive=local && !detect_vault;
    view.wipe_sensitive_cells();
    view.sensitive=sensitive;
    let cell=|mut text:String|->Arc<str>{
        let value=Arc::from(text.as_str());
        if sensitive{use zeroize::Zeroize;text.zeroize();}
        value
    };
    let case_cell=|text:&str|->Arc<str>{
        if case==0{Arc::from(text)}else{cell(character_case(text,case))}
    };
    let empty:Arc<str>=Arc::from("");
    let folder_kind:Arc<str>=Arc::from(folder_type.as_str());
    let vault_kind:Arc<str>=Arc::from(vault_type.as_str());
    let dir_size:Arc<str>=Arc::from(folder_size_text.as_str());
    let owned=|text:&Arc<str>|->Arc<str>{
        if sensitive{Arc::from(text.as_ref())}else{text.clone()}
    };
    let count = entries.len();
    view.cells.reset(count);
    view.colors.reset(count);
    let grouped = group_mode != "None" && show_group_name;
    view.rows.reset(count,grouped);
    let now = std::time::SystemTime::now();
    let mut date_texts = CellCache::<_,Arc<str>>::default();
    let mut size_texts = CellCache::<_,Arc<str>>::default();
    let mut last_size=None;
    let mut last_date=None;
    let mut date_formatter = ndr::listing::ListDateFormatter::new(&date_format, &time_format, use_age.then(|| [age_labels[0].as_str(), age_labels[1].as_str(), age_labels[2].as_str()]), now);
    // One bounded lookup per exact extension, with a consecutive-hit
    // shortcut for sorted/grouped directories.
    type ExtDisplay=(Arc<str>,Arc<str>,(u8,u8,u8));
    let mut extension_cells=HashMap::<&str,ExtDisplay>::new();
    let mut last_extension:Option<(&str,ExtDisplay)>=None;
    let mut attr_cells=HashMap::<u32,Arc<str>>::new();
    let mut previous = None;
    for (row, entry) in entries.iter().enumerate() {
        if row % 128 == 0 && cancel.is_some_and(|c| c.load(Ordering::Relaxed)) { return None; }
        if grouped && !entry.drive {
            let key = ndr::listing::group_key_cow(entry, &group_mode, &category_mode);
            if previous.as_ref() != Some(&key) {
                view.rows.push(DetailRow::Header(group_labels.get(ndr::listing::group_caption_key(&key).0).cloned().unwrap_or_else(|| ndr::listing::group_caption_key(&key).0.to_owned())));
                previous = Some(key);
            }
        }
        if grouped { view.rows.push(DetailRow::Item(row)); }
        if entry.drive {
            // Original DrawListItem kind 4: the drive name (item+8) after "%s:". Other cells (extension, size, date, attributes, description) are not drawn.
            let label = drive_infos.get(&entry.path).map(|(name, _, _)| name.as_str()).unwrap_or("");
            let name = if label.is_empty() { entry.name.clone() } else { format!("{} {label}", entry.name) };
            view.cells.push([cell(name),owned(&empty),owned(&empty),owned(&empty),owned(&empty),owned(&empty),owned(&empty)]);
            view.colors.push(Some(drive));
            continue;
        }
        let extension = if entry.is_dir { "" } else { ndr::parity::list_extension(&entry.name) };
        let stem = if attach { &entry.name } else { &entry.name[..entry.name.len() - extension.len()] };
        let description=if descriptions.is_empty(){None}else{descriptions.get(&entry.name.to_lowercase()).filter(|d|!d.is_empty())};
        // Vault rows never share an Arc, even for identical cells. Do not
        // cache decrypted names, extensions, sizes or dates in this branch.
        let vault_item=detect_vault && entry.is_dir
            && entry.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ndr"))
            && entry.path.is_file();
        let ext_display=|| {
            let ext=case_cell(extension.trim_start_matches('.'));
            let kind=if !local{owned(&empty)}else{cell(native::shell_type_name(&extension.to_lowercase(),false))};
            let color=ndr::listing::original_item_color(entry,extension_colors.as_ref(),basic);
            (ext,kind,color)
        };
        let (extension_text,kind,color)=if entry.is_dir{
            let kind=if vault_item{owned(&vault_kind)}else if !local||entry.name==".."{owned(&empty)}else{owned(&folder_kind)};
            (owned(&empty),kind,folder)
        }else if sensitive{ext_display()}
        else if let Some((_,value))=last_extension.as_ref().filter(|(key,_)|*key==extension){
            value.clone()
        }else{
            let value=if let Some(value)=extension_cells.get(extension){value.clone()}
                else{
                    let value=ext_display();
                    if extension_cells.len()<4096{extension_cells.insert(extension,value.clone());}
                    value
                };
            last_extension=Some((extension,value.clone()));value
        };
        let description=if let Some(d)=description{Arc::from(d.as_str())}
            else if show_file_type{owned(&kind)}else{owned(&empty)};
        let size=if entry.is_dir{owned(&dir_size)}
            else if sensitive{cell(ndr::parity::size_to_str(entry.size,&unit,thousands))}
            else{shared_cell(&mut size_texts,&mut last_size,entry.size,||cell(ndr::parity::size_to_str(entry.size,&unit,thousands)))};
        let date=if sensitive{cell(date_formatter.format(entry.modified))}
            else{shared_cell(&mut date_texts,&mut last_date,entry.modified,||cell(date_formatter.format(entry.modified)))};
        let attr=match (&sftp_modes,&ftp_permissions){
            (Some((directory,modes)),_)=>cell(modes.get(&super::ftp_pane::join_remote(directory,&entry.name)).map(|mode|permission_text(entry.is_dir,*mode)).unwrap_or_default()),
            (None,Some((directory,texts)))=>texts.get(&super::ftp_pane::join_remote(directory,&entry.name)).map(|s|Arc::from(s.as_str())).unwrap_or_else(||owned(&empty)),
            (None,None) if sensitive=>cell(ndr::listing::attr_string(entry.attributes)),
            (None,None)=>attr_cells.entry(entry.attributes&0x27).or_insert_with(||cell(ndr::listing::attr_string(entry.attributes))).clone(),
        };
        view.cells.push([case_cell(stem),extension_text,kind,size,date,attr,description]);
        view.colors.push(Some(color));
    }
    Some(view)
}

/// One row of original DrawListItem kind 4 (left aligned, DT_LEFT): a button-like background when selected, the icon ("▸ " when missing), "C: name",
/// then `paint_drive_space`. `label_right` is from the row's left to the end of the widest drive name (App::drive_text_extent).
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_row(ui: &mut egui::Ui, size: egui::Vec2, icon: Option<&egui::TextureHandle>, text: &str, color: Color32, selected: bool, cursor: bool, label_right: f32, info: Option<(u32, Option<(u64, u64)>)>, language: &compat::Language, style: u8) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected {
            ui.painter().rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        // Original DrawListItem: the icon at left + 12, the "%s:" text from the head width (GetListHeaderWidth) + 5 (same margin as the name cell).
        if let Some(texture) = icon {
            egui::Image::new((texture.id(), egui::vec2(16.0, 16.0))).paint_at(ui, egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, rect.top() + ((rect.height() - 16.0) / 2.0).floor()), egui::vec2(16.0, 16.0)));
        }
        let x = rect.left() + App::list_header_width(icon.is_some()) + 5.0;
        // Original DrawListItem kind 4: the drive name uses DT_END_ELLIPSIS (local_64 | 0x8000), so it shortens with "..." when longer than the cell.
        // Fixed: previously characters were cut in half at the cell end.
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), egui::TextStyle::Button.resolve(ui.style()), color);
        job.wrap = egui::text::TextWrapping { max_width: (rect.right() - 5.0 - x).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let drawn = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley.size());
        painter.galley(drawn.min, galley, color);
        // Underline (4) and strikeout (8) of the original ListFont style.
        if style & 4 != 0 {
            painter.hline(drawn.x_range(), drawn.bottom() - 1.5, egui::Stroke::new(1.0_f32, color));
        }
        if style & 8 != 0 {
            painter.hline(drawn.x_range(), drawn.center().y, egui::Stroke::new(1.0_f32, color));
        }
        if let Some((kind, sizes)) = info {
            paint_drive_space(ui, rect, rect.left() + label_right, kind, sizes, cursor, color, language, style);
        }
    }
    if let Some((3 | 4, Some((free, total)))) = info {
        if total > 0 {
            return response.on_hover_ui(|ui| {
                let (free, total) = (localized_byte_size(language, free), localized_byte_size(language, total));
                let details = ndr::markup::substitute(&language.text("RES.DISKINFO_LONG", "%s Free / %s Total"), &[&free, &total]);
                ui.label(ndr::markup::plain_text(&details));
            });
        }
    }
    response
}

/// Free space of original DrawListItem kind 4 (0x7d8f58): for a fixed disk (type 3) with a known total size, a 70 x min(8, row height) bar drawn
/// 5 px after the end of the drive name cell (`label_right`), its inner 68 px filled by the used ratio. The bar is drawn only when more than 80 px remain on the right,
/// NDR shows just the available size after the bar, without a translated "Free" suffix.
/// Colors (0x7d9770-0x7d98f2): the border uses the item text color for the cursor item (ItemIndex), otherwise FUN_007d7e6c(NXFORE, NXBACK, 0.3);
/// the fill is always (NXFORE, NXBACK, 0.2); the text uses the item text color for the cursor item, otherwise NXSIZE (size column color).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_drive_space(ui: &egui::Ui, rect: egui::Rect, label_right: f32, kind: u32, sizes: Option<(u64, u64)>, cursor: bool, item_color: Color32, language: &compat::Language, style: u8) {
    // The original draws only fixed disks (3). Fixed: connected network drives whose size was read (drive_item_info) are drawn too.
    let Some((free, total)) = sizes.filter(|_| kind == 3 || kind == 4) else { return };
    let room = rect.right() - label_right;
    if total == 0 || room <= 79.0 {
        return;
    }
    let used = total.saturating_sub(free);
    let fill = ((used as u128 * 100 / total as u128) as f32 * 68.0 / 100.0).floor();
    let height = rect.height().min(8.0);
    let left = (label_right + 5.0).round();
    let top = rect.top() + ((rect.height() - height) / 2.0).floor();
    let fore = ui.visuals().text_color();
    let back = ui.visuals().extreme_bg_color;
    let outline = if cursor { item_color } else { blend(fore, back, 0.3) };
    let fill_color = blend(fore, back, 0.2);
    let text_color = if cursor { item_color } else { ui.visuals().weak_text_color() };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let outer = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(70.0, height));
    painter.rect_stroke(outer, 0.0, egui::Stroke::new(1.0_f32, outline), egui::StrokeKind::Inside);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(left + 1.0, top + 1.0), egui::vec2(fill, (height - 2.0).max(0.0))), 0.0, fill_color);
    if room < 100.0 {
        return;
    }
    let size_text = localized_byte_size(language, free);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let size_width = ui.fonts_mut(|f| f.layout_no_wrap(size_text.clone(), font.clone(), text_color).size().x);
    if room >= 91.0 + size_width {
        let text_rect = egui::Rect::from_min_max(egui::pos2(left + 81.0, rect.top()), egui::pos2(rect.right() - 5.0, rect.bottom()));
        let text_painter = painter.with_clip_rect(text_rect.intersect(ui.clip_rect()));
        let drawn = text_painter.text(
            egui::pos2(text_rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            size_text,
            font,
            text_color,
        );
        // Underline (4) and strikeout (8) of the original ListFont style.
        if style & 4 != 0 {
            text_painter.hline(drawn.x_range(), drawn.bottom() - 1.5, egui::Stroke::new(1.0_f32, text_color));
        }
        if style & 8 != 0 {
            text_painter.hline(drawn.x_range(), drawn.center().y, egui::Stroke::new(1.0_f32, text_color));
        }
    }
}

/// Original FUN_007d7e6c: per channel Round(a x t + b x (1 - t)). Delphi Round rounds half to even.
pub(super) fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) * t + (1.0 - t) * f32::from(y)).round_ties_even().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::character_case;

    #[test]
    fn all_locales_draw_only_capacity_beside_drive_bar() {
        use super::*;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("Resources/Languages");
        let free = 64 * 1024 * 1024;
        for &(locale, _) in ndr::compat::BUILTIN_LANGUAGES {
            let language = compat::Language::load(&root, locale);
            let ctx = egui::Context::default();
            let expected = localized_byte_size(&language, free);
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 120.0))), ..Default::default() };
            let output = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(500.0, 24.0), egui::Sense::hover());
                    paint_drive_space(ui, rect, rect.left() + 110.0, 3, Some((free, free * 4)), false, Color32::WHITE, &language, 0);
                });
            });
            let labels: Vec<_> = output.shapes.iter().filter_map(|item| match &item.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            }).collect();
            assert_eq!(labels, vec![expected.as_str()], "{locale}: drive capacity must not have a Free suffix");
        }
    }

    #[test]
    fn drive_bar_blend_follows_fun_007d7e6c() {
        use super::Color32;
        // Default NXFORE C0C0C0, NXBACK 000000: border 0.3 -> 58 (57.6), fill 0.2 -> 38 (38.4).
        let (fore, back) = (Color32::from_rgb(0xC0, 0xC0, 0xC0), Color32::BLACK);
        assert_eq!(super::blend(fore, back, 0.3), Color32::from_rgb(58, 58, 58));
        assert_eq!(super::blend(fore, back, 0.2), Color32::from_rgb(38, 38, 38));
        assert_eq!(super::blend(Color32::WHITE, Color32::BLACK, 0.5), Color32::from_rgb(128, 128, 128));
    }
    #[test]
    fn character_case_follows_filenamedisplayconverter() {
        assert_eq!(character_case("Report Final", 0), "Report Final");
        assert_eq!(character_case("Report Final", 1), "report final");
        assert_eq!(character_case("Report Final", 2), "REPORT FINAL");
        // Original FUN_007d223c: the first a-z of each word becomes upper case (C# FileNameDisplayConverter does only the first character).
        assert_eq!(character_case("report final", 3), "Report Final");
        assert_eq!(character_case("", 3), "");
    }
}
