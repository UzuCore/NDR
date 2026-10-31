//! Outer frame of C# MainWindow.xaml. The title banner absent from the original NexusFile (N logo, NDR, description line) is left out. Compare, checksum and remote are reached through menus.
//! Favorites row (BuildFavorites, add current location +, drive buttons), info bars below the panes (UpdatePaneInfoBars, folder/disk, file, extension),
//! AddInfoBarMenu/ToggleInfoBar, UpdateDiskCapacityAsync, bottom row (status text + function key bar).
//! Difference (HANDOFF): an English UI also shows in English the texts C# fixed to Korean.
use super::file_operation::n0;
use super::*;

/// C# PaneState.UpdateSummary (counted once when the list is received).
#[derive(Clone, Debug, Default)]
pub(super) struct PaneSummary {
    pub folders: usize,
    pub files: usize,
    pub bytes: u64,
    /// (extension, count): count descending, ties by name (case-insensitive).
    pub extensions: Vec<(String, usize)>,
}

pub(super) fn summarize(entries: &[Entry], ko: bool) -> PaneSummary {
    let mut summary = PaneSummary::default();
    let mut extensions: Vec<(String, usize)> = Vec::new();
    for entry in entries {
        if entry.drive || entry.is_parent() {
            continue;
        }
        if entry.is_dir {
            summary.folders += 1;
            continue;
        }
        summary.files += 1;
        summary.bytes = summary.bytes.saturating_add(entry.size);
        let extension = match ndr::parity::list_extension(&entry.name) {
            "" => (if ko { "(확장자 없음)" } else { "(no extension)" }).to_owned(),
            e => e.to_owned(),
        };
        match extensions.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&extension)) {
            Some((_, count)) => *count += 1,
            None => extensions.push((extension, 1)),
        }
    }
    extensions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    summary.extensions = extensions;
    summary
}

impl App {
    /// NDR status text in the skin theme (the original has no status bar): overlaid for 4 seconds at the bottom left of the window (right above the function key row, `bottom_inset`)
    /// without shrinking the list area. A new text restarts the count. The text itself (self.status) is kept.
    pub(super) fn skin_status_overlay(&mut self, ctx: &egui::Context, bottom_inset: f32) -> Option<egui::Rect> {
        if self.status.is_empty() {
            self.status_seen = None;
            return None;
        }
        let now = Instant::now();
        let since = match &self.status_seen {
            Some((text, at)) if *text == self.status => *at,
            _ => {
                self.status_seen = Some((self.status.clone(), now));
                now
            }
        };
        let life = Duration::from_secs(4);
        let elapsed = now.saturating_duration_since(since);
        if elapsed >= life {
            return None;
        }
        ctx.request_repaint_after(life - elapsed);
        let visuals = ctx.style().visuals.clone();
        let font = egui::TextStyle::Body.resolve(&ctx.style());
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(self.status.clone(), font, visuals.text_color()));
        let screen = ctx.content_rect();
        let size = galley.size() + egui::vec2(12.0, 6.0);
        let rect = egui::Rect::from_min_size(egui::pos2(screen.left() + 6.0, (screen.bottom() - bottom_inset - size.y - 4.0).round()), egui::vec2(size.x.ceil(), size.y.ceil()));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("ndr-skin-status")));
        painter.rect(rect, 2.0, visuals.window_fill(), visuals.window_stroke(), egui::StrokeKind::Inside);
        painter.galley(rect.min + egui::vec2(6.0, 3.0), galley, visuals.text_color());
        Some(rect)
    }

    /// Tooltip of original FUN_0042f27c work folder buttons: WORKFOLDER_SETHINT when empty, otherwise "path\nWORKFOLDER_UNSETHINT".
    pub(super) fn work_folder_hint(&self, index: usize) -> String {
        let path = self.settings.strings("WorkingFolders").get(index).cloned().unwrap_or_default();
        if path.is_empty() {
            self.language_pack.text("RES.WORKFOLDER_SETHINT", "Click to set a current folder as an work folder.")
        } else {
            format!("{path}\n{}", self.language_pack.text("RES.WORKFOLDER_UNSETHINT", "Click right button to unset an work folder."))
        }
    }

    /// Original FUN_0042fc84: a set slot goes to that folder (FUN_0042fdf0), an empty slot is set to the current folder without asking.
    pub(super) fn work_folder_click(&mut self, index: usize) {
        let path = self.settings.strings("WorkingFolders").get(index).cloned().unwrap_or_default();
        if !path.is_empty() {
            self.prefixed_command(&format!("workOpen:{index}"));
            return;
        }
        let pane = &self.panes[self.active];
        if super::panes::is_drives(&pane.path) {
            return;
        }
        let current = match &pane.archive {
            Some(archive) => archive.file.parent().map(Path::to_path_buf).unwrap_or_default(),
            None => pane.path.clone(),
        };
        self.set_work_folder(index, current.to_string_lossy().into_owned());
    }

    /// Original FUN_0042fe60: the right button clears that slot (without asking).
    pub(super) fn work_folder_clear(&mut self, index: usize) {
        let mut folders = self.settings.strings("WorkingFolders");
        folders.resize(5, String::new());
        folders[index].clear();
        self.settings.set_strings("WorkingFolders", folders);
        self.save();
    }

    /// C# LeftColumn/RightColumn star widths: LeftPaneWidth / (LeftPaneWidth + RightPaneWidth), 20-80%.
    pub(super) fn left_pane_ratio(&self) -> f32 {
        let value = |key: &str| self.settings.values.get(key).and_then(|v| v.as_f64()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(50.0);
        let (left, right) = (value("LeftPaneWidth"), value("RightPaneWidth"));
        ((left / (left + right)) as f32).clamp(0.2, 0.8)
    }

    /// Dragging the divider (C# PaneDivider, original iVertThumb) and double-clicking it (original pmiFrameWidthSame = 50%): left % (20-80) and the rest.
    pub(super) fn set_left_pane_width(&mut self, percent: f64) {
        let left = percent.clamp(20.0, 80.0);
        self.settings.values.insert("LeftPaneWidth".into(), serde_json::json!(left));
        self.settings.values.insert("RightPaneWidth".into(), serde_json::json!(100.0 - left));
    }

    /// Original pVertBar (between the panes, DFM 0xa27870): five work folder buttons (Top 50 + 18*i, i-th cell of the skin workfolders_set/unset.png strip) and
    /// the DFM buttons bSelectAll (157), bClearSelection (179), bCopyToRight (219), bCopyToLeft (240), bSort (289, pmSort),
    /// bColCount (311, pmColCount), bSyncPathToLeft/Right (bSyncAddressClick). Images are cells of the skin vertbarbtns.png strip (cell table of FUN_0042cc2c
    /// [0,1,2,3,4,5,6,8]; buttons were matched by their cell images). Tooltips are [formMain] Hint.b....
    pub(super) fn vertical_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let set = self.toolbar_icon(ctx, "workfolders_set.png");
        let unset = self.toolbar_icon(ctx, "workfolders_unset.png");
        let folder_over = self.toolbar_icon(ctx, "workfolders_over.png");
        let strip = self.toolbar_icon(ctx, "vertbarbtns.png");
        let strip_over = self.toolbar_icon(ctx, "vertbarbtns_over.png");
        let folders = self.settings.strings("WorkingFolders");
        let area = ui.max_rect();
        // Child positions of the original pVertBar (DFM Left/Top relative to the bar's top left). Work folder buttons (FUN_0042f27c) have Left 0.
        let cell_at = |left: f32, y: f32, height: f32| egui::Rect::from_min_size(egui::pos2(area.min.x + left, area.min.y + y), egui::vec2(16.0, height));
        let cell = |y: f32| cell_at(0.0, y, 16.0);
        // Original TNXPngButton SetNormalImage / SetOverImage: the same cell of the over strip while the pointer is over it.
        // (egui Image fits the whole strip's aspect ratio before applying uv, which squashes the cell, so the cell is painted directly.)
        // A disabled button (enabled = false) is drawn at half alpha (>> 1) without a disabled image, like state 2 of original TNXPngButton.Paint, and cannot be clicked.
        let image_button = |ui: &mut egui::Ui, rect: egui::Rect, normal: &Option<egui::TextureHandle>, over: &Option<egui::TextureHandle>, index: usize, count: usize, fallback: &str, enabled: bool| {
            let texture = if enabled && ui.rect_contains_pointer(rect) { over.as_ref().or(normal.as_ref()) } else { normal.as_ref() };
            match texture {
                Some(texture) => {
                    let uv = egui::Rect::from_min_max(egui::pos2(index as f32 / count as f32, 0.0), egui::pos2((index + 1) as f32 / count as f32, 1.0));
                    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
                    let response = ui.interact(rect, ui.id().with(("ndr-vertbar-button", index, count, rect.top() as i32)), sense);
                    ui.painter().image(texture.id(), rect, uv, if enabled { Color32::WHITE } else { Color32::WHITE.gamma_multiply(0.5) });
                    response
                }
                None => super::original_dialogs::put_enabled(ui, rect, enabled, egui::Button::new(RichText::new(fallback).size(10.0)).frame(false)),
            }
        };
        for index in 0..5 {
            let assigned = folders.get(index).is_some_and(|p| !p.is_empty());
            let response = image_button(ui, cell(50.0 + 18.0 * index as f32), if assigned { &set } else { &unset }, &folder_over, index, 5, &(index + 1).to_string(), true);
            let response = response.on_hover_text(self.work_folder_hint(index));
            if response.clicked() {
                self.work_folder_click(index);
            } else if response.secondary_clicked() {
                self.work_folder_clear(index);
            }
        }
        let hint = |app: &App, name: &str, fallback: &str| app.language_pack.text(&format!("formMain.Hint.{name}"), fallback);
        // Original DFM: (name, Left, Top, Height, strip cell). The two sync buttons are 16x12 at the very top of the bar (the image is clipped from the button's top left).
        // Fixed: previously the sync buttons were 16x16 at Top 345 and 363.
        let buttons: [(&str, f32, f32, f32, usize, &str, &str); 8] = [
            ("bSyncPathToRight", 1.0, 6.0, 12.0, 4, "Left Path => Right Path", "▶"),
            ("bSyncPathToLeft", 0.0, 17.0, 12.0, 5, "Left Path <= Right Path", "◀"),
            ("bSelectAll", 0.0, 157.0, 16.0, 2, "Select All", "◆"),
            ("bClearSelection", 0.0, 179.0, 16.0, 3, "Clear Selection", "◇"),
            ("bCopyToRight", 0.0, 219.0, 16.0, 0, "Copy To Right", "→"),
            ("bCopyToLeft", 0.0, 240.0, 16.0, 1, "Copy To Left", "←"),
            ("bSort", 1.0, 289.0, 16.0, 6, "Arrange", "≡"),
            ("bColCount", 1.0, 311.0, 16.0, 8, "Column", "⋮"),
        ];
        // Original FUN_0044532c (when a pane location changes, with two panes): bSyncPathToRight is enabled only when the left pane location is a local folder (1), bSyncPathToLeft for the right pane.
        let local = |index: usize| { let pane = &self.panes[index]; pane.archive.is_none() && pane.remote.is_none() && !super::panes::is_drives(&pane.path) };
        let sync_enabled = [local(0), local(1)];
        for (name, left, y, height, image, fallback, glyph) in buttons {
            if area.min.y + y + height > area.max.y {
                break;
            }
            let tip = hint(self, name, fallback);
            let enabled = match name {
                "bSyncPathToRight" => sync_enabled[0],
                "bSyncPathToLeft" => sync_enabled[1],
                _ => true,
            };
            let rect = cell_at(left, y, height);
            // TNXPngButton draws its image (16x16) at the button's top left and clips outside the button.
            let response = ui.scope(|ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                image_button(ui, cell_at(left, y, 16.0), &strip, &strip_over, image, 10, glyph, enabled)
            }).inner.on_hover_text(tip);
            match name {
                // Original DropDownMenu pmSort / pmColCount: that menu below the button. The skin theme shows the sort and column items directly without submenus,
                // like the original pmSortPopup/pmColCountPopup.
                "bSort" | "bColCount" => {
                    super::skin_pane::menu_popup(&response).show(|ui| {
                        if name == "bSort" {
                            self.original_sort_popup(ui, self.language == "ko");
                        } else {
                            self.original_column_popup(ui, ctx);
                        }
                    });
                }
                _ if response.clicked() => self.vertical_bar_click(name, ctx),
                _ => {}
            }
        }
        // Original iVertThumb (TImage Left 3, Top = height - 25, 12x22, Center, akBottom, crHSplit): draws the skin vertthumb.png at its own size centered in that cell.
        // Like C# PaneDivider the whole bar takes the drag (the cursor is crHSplit over the whole bar too).
        if let Some(thumb) = self.toolbar_icon(ctx, "vertthumb.png") {
            // FUN_0042cc2c: Top = bar height - thumb height (22) - 4, Left 3.
            let frame = egui::Rect::from_min_size(egui::pos2(area.min.x + 3.0, area.max.y - 26.0), egui::vec2(12.0, 22.0));
            let rect = egui::Rect::from_center_size(frame.center(), thumb.size_vec2().min(frame.size()));
            ui.painter().image(thumb.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
    }

    /// Original pVertBar button OnClick: select all/none, copy left<->right (MSG_TOLEFT/TORIGHT confirmation), sync paths (bSyncAddressClick, Tag).
    pub(super) fn vertical_bar_click(&mut self, name: &str, ctx: &egui::Context) {
        match name {
            "bSelectAll" => {
                let shift = ctx.input(|i| i.modifiers.shift);
                self.original_button_select_all(shift);
            }
            "bClearSelection" => self.panes[self.active].selected.clear(),
            // Original bCopyToRightClick (0x42dd08) / bCopyToLeftClick (0x42dd30): holding Shift moves (skin theme).
            "bCopyToRight" | "bCopyToLeft" => {
                let moving = ctx.input(|i| i.modifiers.shift);
                self.original_copy_to_other(if name == "bCopyToRight" { 0 } else { 1 }, moving, ctx);
            }
            "bSyncPathToRight" | "bSyncPathToLeft" => {
                let (from, to) = if name == "bSyncPathToRight" { (0, 1) } else { (1, 0) };
                // Original bSyncAddressClick (0x438458): only when the sending side is in local mode (0). If the receiving side is FTP it goes to the local folder like FUN_0048d6e4 (disconnecting).
                if self.panes[from].archive.is_none() && self.panes[from].remote.is_none() {
                    let path = self.panes[from].path.clone();
                    self.navigate(to, path, true);
                }
            }
            _ => {}
        }
    }

    /// C# ToggleInfoBar. The menu uses the original View menu check items miViewFolderInfoBar and miViewFileInfoBar (instead of the C# AddInfoBarMenu submenu).
    /// Fixed: original miViewExtensionInfoClick (0x434c20) is an empty function, so the extension info bar could not be toggled from the menu -> it toggles ShowExtensionInfoBar.
    pub(super) fn toggle_info_bar(&mut self, key: &str) {
        let default = key != "ShowExtensionInfoBar";
        let value = !self.settings.flag(key, default);
        self.settings.set_flag(key, value);
        if key == "ShowFolderDiskInfoBar" && value {
            self.update_disk_capacity(0);
            self.update_disk_capacity(1);
        }
        self.save();
    }

    /// In place of C# UpdateDiskCapacityAsync: original data of a local drive (not network, ready) (drive text, free, total). Dropped when another listing arrives.
    pub(super) fn update_disk_capacity(&mut self, index: usize) {
        let pane = &mut self.panes[index];
        pane.disk = None;
        // Original FUN_004408f8: FTP mode hides the disk info text (+0x33c, +0x340).
        if pane.archive.is_some() || pane.remote.is_some() || super::panes::is_drives(&pane.path) {
            return;
        }
        let (path, generation, tx, read) = (pane.path.clone(), pane.generation, self.async_tx.clone(), self.system.disk_info);
        thread::spawn(move || {
            let _ = tx.send(AsyncResult::DiskCapacity { pane: index, generation, disk: read(&path) });
        });
    }

    /// Text of the original folder info bar (FUN_0048aa30 + FUN_0048b500 + FUN_00484b24): (FOLDERINFO, drive text, right text, tooltip).
    /// With marked items the right side is SELECTED_INFO (sum of file sizes, "+" when folders are included), otherwise drive text + DISKINFO_SHORT (free), and
    /// the tooltip is the DISKINFO_LONG (free, total) text without tags.
    pub(super) fn folder_info(&self, index: usize) -> (String, String, String, String) {
        let pane = &self.panes[index];
        let summary = pane.summary.clone().unwrap_or_default();
        let res = |key: &str, fallback: &str| self.language_pack.text(&format!("RES.{key}"), fallback);
        let counts = ndr::markup::substitute(
            &res("FOLDERINFO", "<c000000><b>%d</b></c> Folder, <c000000><b>%d</b></c> File (<b>%s</b>)"),
            &[&summary.folders.to_string(), &summary.files.to_string(), &native::format_byte_size(summary.bytes)],
        );
        let marked: Vec<&Entry> = pane.selected.iter().filter_map(|i| pane.entries.get(*i)).filter(|e| !e.drive && !e.is_parent()).collect();
        if !marked.is_empty() {
            let bytes = marked.iter().filter(|e| !e.is_dir).fold(0u64, |total, e| total.saturating_add(e.size));
            let plus = if marked.iter().any(|e| e.is_dir) { "+" } else { "" };
            let selected = ndr::markup::substitute(&res("SELECTED_INFO", "Selected: <b>%d</b> objects (<b>%s</b> bytes)"), &[&marked.len().to_string(), &format!("{}{plus}", n0(bytes as usize))]);
            return (counts, String::new(), selected, String::new());
        }
        match &pane.disk {
            Some((drive, free, total)) if pane.archive.is_none() => {
                let (free, total) = (native::format_byte_size(*free), native::format_byte_size(*total));
                let short = ndr::markup::substitute(&res("DISKINFO_SHORT", "<c000000><b>%s</b></c> Free"), &[&free]);
                let long = ndr::markup::substitute(&res("DISKINFO_LONG", "<c000000><b>%s</b></c> Free (Total <b>%s</b>)"), &[&free, &total]);
                (counts, drive.clone(), short, ndr::markup::plain_text(&long))
            }
            _ => (counts, String::new(), String::new(), String::new()),
        }
    }

    /// Original FUN_0040e138(text, list) -> TfMessage FUN_0044a1d4: shows a black text window (WS_EX_LAYERED, text size + 40) centered on that list and
    /// closes it with Timer1 (3000 ms). A click closes it at once (OnClick).
    pub(super) fn pane_message(&mut self, index: usize, text: impl Into<String>) {
        self.pane_message = Some((text.into(), index.min(1), Instant::now()));
    }

    pub(super) fn pane_message_ui(&mut self, ctx: &egui::Context) {
        let Some((text, index, shown)) = self.pane_message.clone() else { return };
        if shown.elapsed() >= Duration::from_millis(3000) {
            self.pane_message = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(3000).saturating_sub(shown.elapsed()));
        let pane = self.pane_rects[index];
        let center = if pane.is_positive() { pane.center() } else { ctx.content_rect().center() };
        let response = egui::Area::new(egui::Id::new("ndr-pane-message"))
            .order(egui::Order::Foreground)
            .fixed_pos(center)
            .pivot(egui::Align2::CENTER_CENTER)
            .interactable(true)
            .show(ctx, |ui| {
                // Original FUN_004495ec (TfMessage paint, UpdateLayeredWindow): a rounded rectangle (20x20 arcs) filled ARGB 0xBB222222, the text in
                // the form font (Tahoma -19 px = 14 pt) made bold, drawn black and then white one pixel up-left (a 1 px shadow).
                // Previously a darker, less rounded box with regular 17 px text and no shadow.
                egui::Frame::new().fill(Color32::from_rgba_unmultiplied(0x22, 0x22, 0x22, 0xBB)).corner_radius(10.0).inner_margin(egui::Margin::symmetric(20, 20)).show(ui, |ui| {
                    let font = egui::FontId::new(14.0 * 96.0 / 72.0, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()));
                    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font, Color32::WHITE));
                    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                    ui.painter().galley_with_override_text_color(rect.min + egui::vec2(1.0, 1.0), galley.clone(), Color32::BLACK);
                    ui.painter().galley(rect.min, galley, Color32::WHITE);
                })
            })
            .response;
        if response.interact(egui::Sense::click()).clicked() {
            self.pane_message = None;
        }
    }

    /// C# UpdatePaneInfoBars: (folder/disk, file, extension) texts.
    pub(super) fn info_texts(&mut self, index: usize) -> [String; 3] {
        let ko = self.language == "ko";
        let unit = self.settings.text("SizeUnit", "Auto");
        let thousands = self.settings.flag("UseThousandsSeparator", true);
        let size = |bytes: u64| ndr::parity::format_size(bytes, &unit, thousands);
        if self.panes[index].summary.is_none() {
            let summary = summarize(&self.panes[index].entries, ko);
            self.panes[index].summary = Some(summary);
        }
        if self.panes[index].view.cells.len() != self.panes[index].entries.len() {
            self.compute_view(index);
        }
        let pane = &self.panes[index];
        let summary = pane.summary.clone().unwrap_or_default();
        let folder = if super::panes::is_drives(&pane.path) {
            // C# UpdatePaneInfoBars: "::drives::" and N drives (no disk text).
            let count = pane.entries.len();
            format!("{} · {}", pane.path.display(), if ko { format!("드라이브 {count}개") } else { format!("Drives {count}") })
        } else {
            let (counts, drive, right, _) = self.folder_info(index);
            if right.is_empty() { counts } else { format!("{counts}  {drive}{right}") }
        };
        let pane = &self.panes[index];
        // DataGrid selection (marked items, otherwise the cursor item).
        let selected: Vec<usize> = if pane.loading {
            vec![]
        } else if pane.selected.is_empty() {
            pane.entries.get(pane.cursor).map(|_| vec![pane.cursor]).unwrap_or_default()
        } else {
            pane.selected.iter().copied().filter(|i| *i < pane.entries.len()).collect()
        };
        let modified = |row: usize| pane.view.cells.get(row).map(|c| c[DetailColumn::Modified.index()].clone()).unwrap_or_default();
        let original_file = if pane.loading {
            None
        } else {
            match self.file_info(index) {
                Some(text) => Some(text),
                None if !super::panes::is_drives(&pane.path) => Some(String::new()),
                None => None,
            }
        };
        let file = match selected.as_slice() {
            _ if original_file.is_some() => original_file.unwrap_or_default(),
            [] => (if ko { "선택된 항목 없음" } else { "No selection" }).to_owned(),
            [row] => {
                let entry = &pane.entries[*row];
                let description = pane.descriptions.get(&entry.name.to_lowercase()).filter(|d| !d.trim().is_empty()).map(|d| format!(" · {d}")).unwrap_or_default();
                if entry.is_dir {
                    format!("{} · {} · {}{description}", entry.name, if ko { "폴더" } else { "Folder" }, modified(*row))
                } else {
                    format!("{} · {} · {}{description}", entry.name, size(entry.size), modified(*row))
                }
            }
            rows => {
                let files: Vec<&Entry> = rows.iter().map(|r| &pane.entries[*r]).filter(|e| !e.is_dir).collect();
                let bytes = files.iter().fold(0u64, |total, e| total.saturating_add(e.size));
                let folders = rows.len() - files.len();
                if ko {
                    format!("선택 {}개 · 파일 {}개 · 폴더 {}개 · {}", n0(rows.len()), n0(files.len()), n0(folders), size(bytes))
                } else {
                    format!("Selected {} · Files {} · Folders {} · {}", n0(rows.len()), n0(files.len()), n0(folders), size(bytes))
                }
            }
        };
        let count = |n: usize| if ko { format!("{}개", n0(n)) } else { n0(n) };
        let selected_files: Vec<&Entry> = selected.iter().map(|r| &pane.entries[*r]).filter(|e| !e.is_dir).collect();
        let extension = if !selected_files.is_empty() {
            // C# GroupBy (extensions in order of first appearance in the selection) -> the first 4.
            let mut groups: Vec<(String, usize)> = Vec::new();
            for entry in selected_files {
                let key = match ndr::parity::list_extension(&entry.name) {
                    "" => (if ko { "(확장자 없음)" } else { "(no extension)" }).to_owned(),
                    e => e.to_owned(),
                };
                match groups.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&key)) {
                    Some((_, n)) => *n += 1,
                    None => groups.push((key, 1)),
                }
            }
            groups.iter().take(4).map(|(k, n)| format!("{k} {}", count(*n))).collect::<Vec<_>>().join(" · ")
        } else {
            summary.extensions.iter().take(4).map(|(k, n)| format!("{k} {}", count(*n))).collect::<Vec<_>>().join(" · ")
        };
        let extension = if extension.is_empty() { (if ko { "확장자 정보 없음" } else { "No extension information" }).to_owned() } else { extension };
        [folder, file, extension]
    }

    /// C# LeftInfoBars (columns 2* : 1.4* : 1, faint 10pt text, ellipsis when clipped, tooltip = full text).
    pub(super) fn info_bars(&mut self, ui: &mut egui::Ui, index: usize) {
        let shown = [self.settings.flag("ShowFolderDiskInfoBar", true), self.settings.flag("ShowFileInfoBar", true), self.settings.flag("ShowExtensionInfoBar", false)];
        if !shown.iter().any(|s| *s) {
            return;
        }
        let texts = self.info_texts(index);
        let width = ui.available_width() - 6.0;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(3.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            for (column, (text, weight)) in texts.iter().zip([2.0, 1.4, 1.0]).enumerate() {
                let cell = width * weight / 4.4 - if column > 0 { 10.0 } else { 0.0 };
                if column > 0 {
                    ui.add_space(10.0);
                }
                if shown[column] && column == 0 && !super::panes::is_drives(&self.panes[index].path) {
                    self.folder_info_cell(ui, index, cell);
                } else if shown[column] && column == 1 {
                    let mut job = super::original_dialogs::markup_job(text, 10.0, ui.visuals().weak_text_color(), cell);
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    ui.add_sized([cell, 14.0], egui::Label::new(job)).on_hover_text(ndr::markup::plain_text(text));
                } else if shown[column] {
                    ui.add_sized([cell, 14.0], egui::Label::new(RichText::new(text).size(10.0).weak()).truncate()).on_hover_text(text);
                } else {
                    ui.add_space(cell);
                }
            }
        });
    }
    /// Original file info bar (FUN_0048a5cc, label 0x34c): "size  |  date  |  attributes" of the focused item, followed by
    /// "  |  <c6B9FFF>description</c>" when there is a description, otherwise "  |  name". Size via FloatToStrF(ffNumber, 0 digits), FOLDER for folders without a size,
    /// date via FormatFileTime(2) = "%.4d-%.2d-%.2d %.2d:%.2d" (local time), attributes via GetAttrString = A/H/R/S in "____".
    pub(super) fn file_info(&self, index: usize) -> Option<String> {
        let pane = &self.panes[index];
        let entry = pane.entries.get(pane.cursor)?;
        if super::panes::is_drives(&pane.path) || entry.drive {
            // Drive item (+0x3c == 4): a fixed disk shows "%s (%s:)  |  %s  |  %s %s / %s" (DISKFREE), others "%s (%s:)  |  %s".
            // Type names are the original table 0x9128bc as is (not translated). None without info (C# version text).
            let (name, kind, sizes) = pane.drive_infos.get(&entry.path)?;
            let letter = entry.name.chars().next()?.to_ascii_uppercase();
            let kind_name = ["", "", "Removable Disk", "Hard Disk", "Network Drive", "CD/DVD", "RAM Disk"].get(*kind as usize).copied().unwrap_or("");
            return Some(match sizes {
                Some((free, total)) if *kind == 3 => {
                    let label = self.language_pack.text("RES.DISKFREE", "Free");
                    format!("{name} ({letter}:)  |  {kind_name}  |  {} {label} / {}", native::format_byte_size(*free), native::format_byte_size(*total))
                }
                _ => format!("{name} ({letter}:)  |  {kind_name}"),
            });
        }
        let size = if entry.is_dir {
            // Original: a counted folder shows the item size (bytes) via FloatToStrF(ffNumber, 0 digits), the same thousands format as files.
            match pane.folder_bytes.get(&entry.path) {
                Some(bytes) => n0(*bytes as usize),
                None => self.language_pack.text("RES.FOLDER", "Folder"),
            }
        } else {
            n0(entry.size as usize)
        };
        let date = entry.modified.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
        let attributes = ndr::listing::attr_string(entry.attributes);
        let mut text = format!("{size}  |  {date}  |  {attributes}");
        match pane.descriptions.get(&entry.name.to_lowercase()).filter(|d| !d.is_empty()) {
            Some(description) => text.push_str(&format!("  |  <c6B9FFF>{description}</c>")),
            None => text.push_str(&format!("  |  {}", entry.name)),
        }
        Some(text)
    }

    /// Layout of the original folder info bar: FOLDERINFO on the left, free space (or selection info) at the right end, the drive text before it.
    /// Like FUN_0048b500 the drive text is emptied when (cell width - left text - right text - 18) <= drive text width. Tooltip = DISKINFO_LONG.
    fn folder_info_cell(&self, ui: &mut egui::Ui, index: usize, width: f32) {
        let (counts, drive, right, hint) = self.folder_info(index);
        let color = ui.visuals().weak_text_color();
        let font = egui::FontId::proportional(10.0);
        let (left, mut drive, right) = ui.fonts_mut(|fonts| {
            (
                fonts.layout_job(super::original_dialogs::markup_job(&counts, 10.0, color, f32::INFINITY)),
                fonts.layout_no_wrap(drive, font.clone(), color),
                fonts.layout_job(super::original_dialogs::markup_job(&right, 10.0, color, f32::INFINITY)),
            )
        });
        if width - left.size().x - right.size().x - 18.0 <= drive.size().x {
            drive = ui.fonts_mut(|fonts| fonts.layout_no_wrap(String::new(), font, color));
        }
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let top = rect.top() + (rect.height() - left.size().y) / 2.0;
        painter.galley(egui::pos2(rect.left(), top), left.clone(), color);
        let right_x = rect.right() - right.size().x;
        painter.galley(egui::pos2(right_x - drive.size().x, top), drive, color);
        painter.galley(egui::pos2(right_x, top), right, color);
        let tip = if hint.is_empty() { ndr::markup::plain_text(&counts) } else { hint };
        response.on_hover_text(tip);
    }
}

impl App {
    fn update_text(&self, key: &str, fallback: &str) -> String {
        self.language_pack.text(&format!("NDRUpdate.{key}"), fallback)
    }

    /// C# CheckForUpdatesAsync (first part): a notice when there is no feed, an error when it is not HTTPS, otherwise fetch it separately.
    pub(super) fn check_for_updates(&mut self) {
        self.start_update_check(false);
    }

    /// silent = the original quiet check ([+0x54] == 1, at startup): only the result is used, without notice or error boxes.
    pub(super) fn start_update_check(&mut self, silent: bool) {
        let title = self.update_text("Title", "NDR Update");
        let feed = self.settings.text("UpdateManifestUrl", "").trim().to_owned();
        if feed.is_empty() {
            if silent {
                return;
            }
            let message = self.update_text("FeedMissing", "The update manifest URL is not configured. Add an HTTPS JSON feed in Settings.");
            return self.system_box(&title, &message, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONASTERISK, |_, _, _| {});
        }
        // Fixed: feed[..8] panicked when byte 8 fell inside a multi-byte character.
        let https = feed.get(..8).is_some_and(|head| head.eq_ignore_ascii_case("https://")) && feed.get(8..).is_some_and(|rest| !rest.trim_start_matches('/').is_empty());
        if !https {
            if silent {
                return;
            }
            let message = self.update_text("FeedHttpsRequired", "The update feed must use HTTPS.");
            return self.system_box("NDR", &message, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
        }
        let (get, tx) = (self.http_get, self.async_tx.clone());
        thread::spawn(move || {
            let _ = tx.send(AsyncResult::Update(get(&feed, ndr::update::MAX_MANIFEST_BYTES), silent));
        });
    }

    /// C# CheckForUpdatesAsync (second part). Errors use an "NDR" warning box like RunSafely, notices an information box.
    /// For the quiet check (original FUN_00441dd8 0x441e3d), a newer version only shows the menu bar button (tbNewUpdateAvailable) and nothing else happens.
    pub(super) fn update_done(&mut self, result: Result<ndr::update::HttpResponse, ndr::update::HttpError>, silent: bool) {
        use ndr::msgbox::{IDYES, MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_OK, MB_YESNO};
        use ndr::update::{HttpError, Manifest};
        if silent {
            if let Some(response) = result.as_ref().ok().filter(|r| (200..300).contains(&r.status) && r.secure) {
                let current = ndr::update::parse_version(env!("CARGO_PKG_VERSION")).unwrap_or_else(|| vec![0, 0]);
                if let Ok(Manifest::Newer(_, url)) = ndr::update::evaluate(&response.body, &current) {
                    self.update_available = Some(url);
                }
            }
            return;
        }
        let title = self.update_text("Title", "NDR Update");
        let warn = |app: &mut App, text: String| app.system_box("NDR", &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        let response = match result {
            Ok(response) => response,
            Err(HttpError::Timeout) => return warn(self, self.update_text("RequestTimedOut", "The update server did not respond within 15 seconds.")),
            Err(HttpError::TooLarge) => return warn(self, self.update_text("ManifestTooLarge", "The update manifest exceeds the 256 KiB limit.")),
            Err(HttpError::Failed(_)) => return warn(self, self.update_text("RequestFailed", "Could not contact the update server. Check your connection and update feed.")),
        };
        if !(200..300).contains(&response.status) {
            let text = self.update_text("RequestFailedStatus", "The update server returned HTTP status {0}.").replace("{0}", &response.status.to_string());
            return warn(self, text);
        }
        if !response.secure {
            return warn(self, self.update_text("RedirectHttpsRequired", "The update feed did not remain on HTTPS."));
        }
        let current = ndr::update::parse_version(env!("CARGO_PKG_VERSION")).unwrap_or_else(|| vec![0, 0]);
        match ndr::update::evaluate(&response.body, &current) {
            Err(message) => warn(self, message),
            Ok(Manifest::Invalid(key)) => {
                let fallback = if key == "InvalidVersion" { "The update feed does not contain a valid version value." } else { "The update feed does not contain a valid HTTPS download URL." };
                warn(self, self.update_text(key, fallback))
            }
            Ok(Manifest::Legacy(version)) => {
                let text = self.update_text("LegacyFeed", "The configured feed belongs to NexusFile (version {0}). Set an NDR HTTPS JSON feed to check for NDR updates.").replace("{0}", &version);
                self.system_box(&title, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
            }
            Ok(Manifest::UpToDate) => {
                // Original FUN_00441dd8: when up to date, ShowMessage(UPDATE_NOUPDATE), a program-titled box without an icon.
                let text = self.language_pack.text("RES.UPDATE_NOUPDATE", "This application is up-to-date.");
                self.system_box("NDR", &text, MB_OK, |_, _, _| {});
            }
            Ok(Manifest::Newer(_, url)) => {
                // Original: MessageBoxW(UPDATE_NEWUPDATE, title UPDATE, 0x44 = yes/no + information); yes opens the download page.
                let caption = self.language_pack.text("RES.UPDATE", "Update");
                let question = self.language_pack.text("RES.UPDATE_NEWUPDATE", "There is a new update for this application.\nDo you want to go to download?");
                self.system_box(&caption, &question, MB_YESNO | MB_ICONASTERISK, move |app, answer, _| {
                    if answer == IDYES {
                        let _ = (app.shell_runner)(Path::new(&url), "", Path::new(""));
                    }
                });
            }
        }
    }
}
