//! C# MainWindow.xaml 바깥 틀. 원본 NexusFile에 없는 제목 배너(N 로고·NDR·설명 줄)는 두지 않는다. 비교/체크섬/원격은 메뉴로 닿는다.
//! 즐겨찾기 줄(BuildFavorites, 현재 위치 추가 +, 드라이브 단추), 패널 아래 정보 표시줄(UpdatePaneInfoBars, 폴더/디스크·파일·확장자),
//! AddInfoBarMenu/ToggleInfoBar, UpdateDiskCapacityAsync, 아래 줄(상태 글 + 기능키 표시줄).
//! 차이(HANDOFF): 영어 화면은 C#이 한국어로 고정한 글도 영어로 보인다.
use super::file_operation::n0;
use super::*;

/// C# PaneState.UpdateSummary(목록을 받을 때 한 번 센다).
#[derive(Clone, Debug, Default)]
pub(super) struct PaneSummary {
    pub folders: usize,
    pub files: usize,
    pub bytes: u64,
    /// (확장자, 개수): 개수 내림차순, 같으면 이름(대소문자 무시) 순.
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
    /// 스킨 테마의 NDR 상태 글(원본에는 상태 줄이 없다): 목록 자리를 줄이지 않고 창 아래(기능키 줄 바로 위, `bottom_inset`) 왼쪽에
    /// 4초 동안 겹쳐 보인다. 글이 바뀌면 다시 센다. 글 자체(self.status)는 그대로 둔다.
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

    /// 원본 FUN_0042f27c 작업 폴더 단추 도움말: 비었으면 WORKFOLDER_SETHINT, 있으면 "경로\nWORKFOLDER_UNSETHINT".
    pub(super) fn work_folder_hint(&self, index: usize) -> String {
        let path = self.settings.strings("WorkingFolders").get(index).cloned().unwrap_or_default();
        if path.is_empty() {
            self.language_pack.text("RES.WORKFOLDER_SETHINT", "Click to set a current folder as an work folder.")
        } else {
            format!("{path}\n{}", self.language_pack.text("RES.WORKFOLDER_UNSETHINT", "Click right button to unset an work folder."))
        }
    }

    /// 원본 FUN_0042fc84: 지정된 칸이면 그 폴더로(FUN_0042fdf0), 빈 칸이면 묻지 않고 지금 폴더로 지정한다.
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

    /// 원본 FUN_0042fe60: 오른쪽 단추면 그 칸을 비운다(묻지 않음).
    pub(super) fn work_folder_clear(&mut self, index: usize) {
        let mut folders = self.settings.strings("WorkingFolders");
        folders.resize(5, String::new());
        folders[index].clear();
        self.settings.set_strings("WorkingFolders", folders);
        self.save();
    }

    /// C# LeftColumn/RightColumn Star 너비: LeftPaneWidth / (LeftPaneWidth + RightPaneWidth), 20~80%.
    pub(super) fn left_pane_ratio(&self) -> f32 {
        let value = |key: &str| self.settings.values.get(key).and_then(|v| v.as_f64()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(50.0);
        let (left, right) = (value("LeftPaneWidth"), value("RightPaneWidth"));
        ((left / (left + right)) as f32).clamp(0.2, 0.8)
    }

    /// 나눔 막대 끌기(C# PaneDivider, 원본 iVertThumb)·두 번 누르기(원본 pmiFrameWidthSame = 50%): 왼쪽 % (20~80)와 나머지.
    pub(super) fn set_left_pane_width(&mut self, percent: f64) {
        let left = percent.clamp(20.0, 80.0);
        self.settings.values.insert("LeftPaneWidth".into(), serde_json::json!(left));
        self.settings.values.insert("RightPaneWidth".into(), serde_json::json!(100.0 - left));
    }

    /// 원본 pVertBar(두 패널 사이, DFM 0xa27870): 작업 폴더 단추 다섯 개(Top 50 + 18·i, 스킨 workfolders_set/unset.png 띠의 i번째 칸)와
    /// DFM 단추들 — bSelectAll(157), bClearSelection(179), bCopyToRight(219), bCopyToLeft(240), bSort(289, pmSort),
    /// bColCount(311, pmColCount), bSyncPathToLeft/Right(bSyncAddressClick). 그림은 스킨 vertbarbtns.png 띠(FUN_0042cc2c의 칸 번호 표
    /// [0,1,2,3,4,5,6,8]; 칸 그림으로 단추를 맞췄다). 도움말은 [formMain] Hint.b….
    pub(super) fn vertical_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let set = self.toolbar_icon(ctx, "workfolders_set.png");
        let unset = self.toolbar_icon(ctx, "workfolders_unset.png");
        let folder_over = self.toolbar_icon(ctx, "workfolders_over.png");
        let strip = self.toolbar_icon(ctx, "vertbarbtns.png");
        let strip_over = self.toolbar_icon(ctx, "vertbarbtns_over.png");
        let folders = self.settings.strings("WorkingFolders");
        let area = ui.max_rect();
        let x = area.center().x - 8.0;
        let cell = |y: f32| egui::Rect::from_min_size(egui::pos2(x, area.min.y + y), egui::vec2(16.0, 16.0));
        // 원본 TNXPngButton SetNormalImage / SetOverImage: 포인터가 올라가 있으면 over 띠의 같은 칸.
        // (egui Image는 띠 전체의 가로세로 비로 맞춘 뒤 uv를 쓰므로 칸이 납작해진다. 칸을 painter로 바로 그린다.)
        let image_button = |ui: &mut egui::Ui, rect: egui::Rect, normal: &Option<egui::TextureHandle>, over: &Option<egui::TextureHandle>, index: usize, count: usize, fallback: &str| {
            let texture = if ui.rect_contains_pointer(rect) { over.as_ref().or(normal.as_ref()) } else { normal.as_ref() };
            match texture {
                Some(texture) => {
                    let uv = egui::Rect::from_min_max(egui::pos2(index as f32 / count as f32, 0.0), egui::pos2((index + 1) as f32 / count as f32, 1.0));
                    let response = ui.interact(rect, ui.id().with(("ndr-vertbar-button", index, count, rect.top() as i32)), egui::Sense::click());
                    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
                    response
                }
                None => ui.put(rect, egui::Button::new(RichText::new(fallback).size(10.0)).frame(false)),
            }
        };
        for index in 0..5 {
            let assigned = folders.get(index).is_some_and(|p| !p.is_empty());
            let response = image_button(ui, cell(50.0 + 18.0 * index as f32), if assigned { &set } else { &unset }, &folder_over, index, 5, &(index + 1).to_string());
            let response = response.on_hover_text(self.work_folder_hint(index));
            if response.clicked() {
                self.work_folder_click(index);
            } else if response.secondary_clicked() {
                self.work_folder_clear(index);
            }
        }
        let hint = |app: &App, name: &str, fallback: &str| app.language_pack.text(&format!("formMain.Hint.{name}"), fallback);
        let buttons: [(&str, f32, usize, &str, &str); 8] = [
            ("bSelectAll", 157.0, 2, "Select All", "◆"),
            ("bClearSelection", 179.0, 3, "Clear Selection", "◇"),
            ("bCopyToRight", 219.0, 0, "Copy To Right", "→"),
            ("bCopyToLeft", 240.0, 1, "Copy To Left", "←"),
            ("bSort", 289.0, 6, "Arrange", "≡"),
            ("bColCount", 311.0, 8, "Column", "⋮"),
            ("bSyncPathToRight", 345.0, 4, "Left Path => Right Path", "▶"),
            ("bSyncPathToLeft", 363.0, 5, "Left Path <= Right Path", "◀"),
        ];
        for (name, y, image, fallback, glyph) in buttons {
            if area.min.y + y + 16.0 > area.max.y {
                break;
            }
            let tip = hint(self, name, fallback);
            let response = image_button(ui, cell(y), &strip, &strip_over, image, 10, glyph).on_hover_text(tip);
            match name {
                // 원본 DropDownMenu pmSort / pmColCount: 단추 아래에 그 메뉴. 스킨 테마는 원본 pmSortPopup/pmColCountPopup처럼
                // 정렬·열 항목을 하위 메뉴 없이 바로 보인다.
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
        // 원본 iVertThumb(TImage Left 3, Top = 높이 − 25, 12×22, Center, akBottom, crHSplit): 스킨 vertthumb.png를 그 칸 가운데에
        // 원래 크기로 그린다. 끌기는 C# PaneDivider처럼 막대 전체가 받는다(커서도 막대 전체가 crHSplit).
        if let Some(thumb) = self.toolbar_icon(ctx, "vertthumb.png") {
            let frame = egui::Rect::from_min_size(egui::pos2(area.center().x - 6.0, area.max.y - 25.0), egui::vec2(12.0, 22.0));
            let rect = egui::Rect::from_center_size(frame.center(), thumb.size_vec2().min(frame.size()));
            ui.painter().image(thumb.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
    }

    /// 원본 pVertBar 단추 OnClick: 모두 선택/해제, 왼쪽↔오른쪽 복사(MSG_TOLEFT/TORIGHT 확인), 경로 맞추기(bSyncAddressClick, Tag).
    pub(super) fn vertical_bar_click(&mut self, name: &str, ctx: &egui::Context) {
        match name {
            "bSelectAll" => {
                let shift = ctx.input(|i| i.modifiers.shift);
                self.original_button_select_all(shift);
            }
            "bClearSelection" => self.panes[self.active].selected.clear(),
            // 원본 bCopyToRightClick(0x42dd08)/bCopyToLeftClick(0x42dd30): Shift를 누르고 있으면 이동(스킨 테마).
            "bCopyToRight" | "bCopyToLeft" => {
                let moving = ctx.input(|i| i.modifiers.shift);
                self.original_copy_to_other(if name == "bCopyToRight" { 0 } else { 1 }, moving, ctx);
            }
            "bSyncPathToRight" | "bSyncPathToLeft" => {
                let (from, to) = if name == "bSyncPathToRight" { (0, 1) } else { (1, 0) };
                // 원본 bSyncAddressClick(0x438458): 보내는 쪽이 로컬 모드(0)일 때만. 받는 쪽이 FTP면 FUN_0048d6e4처럼 로컬 폴더로 간다(끊김).
                if self.panes[from].archive.is_none() && self.panes[from].remote.is_none() {
                    let path = self.panes[from].path.clone();
                    self.navigate(to, path, true);
                }
            }
            _ => {}
        }
    }

    /// C# ToggleInfoBar. 메뉴는 원본 보기 메뉴의 miViewFolderInfoBar·miViewFileInfoBar 체크 항목(C# AddInfoBarMenu 하위 메뉴 대신).
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

    /// C# UpdateDiskCapacityAsync 자리: 로컬 드라이브(네트워크 아님, 준비됨)의 원본 재료(드라이브 글, 여유, 전체). 다른 목록이 오면 버린다.
    pub(super) fn update_disk_capacity(&mut self, index: usize) {
        let pane = &mut self.panes[index];
        pane.disk = None;
        // 원본 FUN_004408f8: FTP 모드에서는 디스크 정보 글(+0x33c, +0x340)을 숨긴다.
        if pane.archive.is_some() || pane.remote.is_some() || super::panes::is_drives(&pane.path) {
            return;
        }
        let (path, generation, tx, read) = (pane.path.clone(), pane.generation, self.async_tx.clone(), self.system.disk_info);
        thread::spawn(move || {
            let _ = tx.send(AsyncResult::DiskCapacity { pane: index, generation, disk: read(&path) });
        });
    }

    /// 원본 폴더 정보 표시줄(FUN_0048aa30 + FUN_0048b500 + FUN_00484b24)의 글: (FOLDERINFO, 드라이브 글, 오른쪽 글, 도움말).
    /// 표시한 항목이 있으면 오른쪽은 SELECTED_INFO(파일 크기 합, 폴더가 끼면 "+"), 없으면 드라이브 글 + DISKINFO_SHORT(여유)이고
    /// 도움말은 DISKINFO_LONG(여유, 전체)의 태그 뺀 글.
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

    /// 원본 FUN_0040e138(글, 목록) → TfMessage FUN_0044a1d4: 그 목록 가운데에 검은 바탕 글 창(WS_EX_LAYERED, 글 크기 + 40)을 띄우고
    /// Timer1(3000ms)로 닫는다. 누르면 바로 닫힌다(OnClick).
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
                egui::Frame::new().fill(Color32::from_rgba_unmultiplied(0, 0, 0, 220)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(20, 20)).show(ui, |ui| {
                    ui.label(RichText::new(&text).color(Color32::WHITE).size(17.0));
                })
            })
            .response;
        if response.interact(egui::Sense::click()).clicked() {
            self.pane_message = None;
        }
    }

    /// C# UpdatePaneInfoBars: (폴더/디스크, 파일, 확장자) 글.
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
            // C# UpdatePaneInfoBars: "::drives::" · 드라이브 N개(디스크 글 없음).
            let count = pane.entries.len();
            format!("{} · {}", pane.path.display(), if ko { format!("드라이브 {count}개") } else { format!("Drives {count}") })
        } else {
            let (counts, drive, right, _) = self.folder_info(index);
            if right.is_empty() { counts } else { format!("{counts}  {drive}{right}") }
        };
        let pane = &self.panes[index];
        // DataGrid 선택(표시한 항목, 없으면 커서 항목).
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
            // C# GroupBy(선택 순서대로 처음 나온 확장자 순) → 앞 4개.
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

    /// C# LeftInfoBars(열 2* : 1.4* : 1, 10pt 흐린 글, 잘리면 …, 도움말 = 전체 글).
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
    /// 원본 파일 정보 표시줄(FUN_0048a5cc, 라벨 0x34c): 초점 항목의 "크기  |  날짜  |  속성" 뒤에 설명이 있으면
    /// "  |  <c6B9FFF>설명</c>", 없으면 "  |  이름". 크기는 FloatToStrF(ffNumber, 0자리), 크기를 모르는 폴더는 FOLDER,
    /// 날짜는 FormatFileTime(2) = "%.4d-%.2d-%.2d %.2d:%.2d"(지역 시각), 속성은 GetAttrString = "____"에 A·H·R·S.
    pub(super) fn file_info(&self, index: usize) -> Option<String> {
        let pane = &self.panes[index];
        let entry = pane.entries.get(pane.cursor)?;
        if super::panes::is_drives(&pane.path) || entry.drive {
            // 드라이브 항목(+0x3c == 4): 고정 디스크 "%s (%s:)  |  %s  |  %s %s / %s"(DISKFREE), 그 밖 "%s (%s:)  |  %s".
            // 종류 이름은 원본 표 0x9128bc 그대로(번역하지 않음). 정보가 없으면 None(C# 판 글).
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
            // 원본: 크기를 센 폴더는 항목 크기(바이트)를 FloatToStrF(ffNumber, 0자리) — 파일과 같은 천 단위 숫자.
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

    /// 원본 폴더 정보 표시줄의 배치: 왼쪽 FOLDERINFO, 오른쪽 끝에 여유(또는 선택 정보), 그 앞에 드라이브 글.
    /// FUN_0048b500처럼 (칸 너비 - 왼쪽 글 - 오른쪽 글 - 18) <= 드라이브 글 너비이면 드라이브 글을 비운다. 도움말 = DISKINFO_LONG.
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

    /// C# CheckForUpdatesAsync(앞부분): 피드가 없으면 안내, HTTPS가 아니면 오류, 아니면 따로 받아 온다.
    pub(super) fn check_for_updates(&mut self) {
        self.start_update_check(false);
    }

    /// silent = 원본의 조용한 확인([+0x54] == 1, 시작할 때): 안내·오류 상자 없이 결과만 본다.
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
        let https = feed.len() > 8 && feed[..8].eq_ignore_ascii_case("https://") && !feed[8..].trim_start_matches('/').is_empty();
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

    /// C# CheckForUpdatesAsync(뒷부분). 오류는 RunSafely처럼 "NDR" 경고 상자, 안내는 정보 상자.
    /// 조용한 확인이면(원본 FUN_00441dd8 0x441e3d) 새 버전일 때 메뉴 줄 단추(tbNewUpdateAvailable)만 보이고 나머지는 아무것도 하지 않는다.
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
                // 원본 FUN_00441dd8: 최신이면 ShowMessage(UPDATE_NOUPDATE) — 아이콘 없는 프로그램 제목 상자.
                let text = self.language_pack.text("RES.UPDATE_NOUPDATE", "This application is up-to-date.");
                self.system_box("NDR", &text, MB_OK, |_, _, _| {});
            }
            Ok(Manifest::Newer(_, url)) => {
                // 원본: MessageBoxW(UPDATE_NEWUPDATE, 제목 UPDATE, 0x44 = 예/아니요 + 정보), 예면 내려받기 쪽을 연다.
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
