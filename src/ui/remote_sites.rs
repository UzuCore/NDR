//! C# RemoteSitesWindow(FTP 사이트 관리자, 780×490, [formFTPSiteManager]) + RemoteSiteEditorWindow(원격 사이트, 620×650)
//! + ManageRemoteSitesAsync + RunRemoteCommandAsync/SelectRemoteSite/ExecuteSshCommand.
//! 관리자: 왼쪽 이름 목록, 오른쪽 이름·주소·사용자·비밀번호·원격 폴더·포트·서버 종류(FTP/SFTP/FTPS)·수동 모드. 다른 줄로 옮기면 지금 줄을
//! 확인하고(이름·주소가 비면 TfMsgBox FTPSITE_INVALIDINFO) 적는다(종류·포트·주소가 바뀌거나 SFTP가 아니면 저장한 서버 키를 지운다).
//! 추가(TfGetText FTPSITE_ADD, 같은 이름이면 FTPSITE_EXISTS), 삭제(FTPSITE_DELCONFIRM 예/아니요), 추가 설정(편집 창), 연결, 확인, 취소.
use super::original_dialogs::{DialogKind, DialogResult};
use super::remote_window::{SiteProfile, load_sites, parse_port, prepare_host, store_sites, t};
use super::*;
use ndr::msgbox::{IDYES, MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_ICONQUESTION, MB_OK, MB_YESNO};
use ndr::remote::Config;
use serde_json::Value;

/// lstrcmpiW(NexusFileFtpSitesService.NamesEqual).
fn names_equal(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

/// 관리자 서버 종류 목록 순서.
pub(super) const MANAGER_PROTOCOLS: [&str; 3] = ["FTP", "SFTP", "FTPS"];
/// 편집 창 프로토콜 목록 순서.
pub(super) const EDITOR_PROTOCOLS: [&str; 3] = ["FTP", "FTPS", "SFTP"];

pub(super) struct SitesForm {
    pub sites: Vec<SiteProfile>,
    pub selected: Option<usize>,
    /// C# _editingSite(지금 칸에 보이는 사이트).
    pub editing: Option<usize>,
    pub name: String,
    pub host: String,
    pub user: String,
    pub password: String,
    pub remote_directory: String,
    pub port: String,
    pub protocol: String,
    pub passive: bool,
}

impl SitesForm {
    pub(super) fn new(sites: Vec<SiteProfile>) -> Self {
        let mut form = Self { sites, selected: None, editing: None, name: String::new(), host: String::new(), user: String::new(), password: String::new(), remote_directory: String::new(), port: String::new(), protocol: "FTP".into(), passive: true };
        let first = (!form.sites.is_empty()).then_some(0);
        form.selected = first;
        form.load(first);
        form
    }

    /// LoadSelection.
    pub(super) fn load(&mut self, index: Option<usize>) {
        self.editing = index;
        let site = index.and_then(|i| self.sites.get(i)).cloned();
        self.name = site.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        self.host = site.as_ref().map(|s| s.host.clone()).unwrap_or_default();
        self.user = site.as_ref().map(|s| s.user.clone()).unwrap_or_default();
        self.password = site.as_ref().map(|s| native::unprotect_secret(&s.encrypted_password).unwrap_or_default()).unwrap_or_default();
        self.remote_directory = site.as_ref().map(|s| s.remote_directory.clone()).unwrap_or_default();
        self.port = site.as_ref().map(|s| s.port.to_string()).unwrap_or_default();
        self.protocol = site.as_ref().map(|s| s.protocol.clone()).unwrap_or_else(|| "FTP".into());
        self.passive = site.as_ref().is_none_or(|s| s.passive);
    }

    /// CommitSelection: 빈 이름·주소면 false(부르는 쪽이 FTPSITE_INVALIDINFO 상자).
    pub(super) fn commit(&mut self) -> bool {
        let Some(index) = self.editing else { return true };
        if self.name.is_empty() || self.host.is_empty() {
            return false;
        }
        let protocol = if MANAGER_PROTOCOLS.contains(&self.protocol.as_str()) { self.protocol.clone() } else { "FTP".into() };
        let port = parse_port(&self.port);
        let site = &mut self.sites[index];
        if !protocol.eq_ignore_ascii_case(&site.protocol) || protocol != "SFTP" || port != site.port || prepare_host(&self.host).to_lowercase() != prepare_host(&site.host).to_lowercase() {
            site.known_host_fingerprint.clear();
        }
        site.name = self.name.clone();
        site.host = self.host.clone();
        site.user = self.user.clone();
        site.encrypted_password = native::protect_secret(&self.password).unwrap_or_default();
        site.remote_directory = self.remote_directory.clone();
        site.port = port;
        site.protocol = protocol;
        site.passive = self.passive;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SitesAction {
    Select(usize),
    Add,
    Delete,
    Advanced,
    Connect,
    Save,
    Cancel,
}

/// 원본 TfFTPSiteManager(ClientWidth 472 × ClientHeight 297): lbSites(16,25 153×206), lName(176,28) eName(281,25 176),
/// lAddress(176,56) eHost(281,53 176), lUser(176,85) eUser(281,82 121), lPassword(176,114) ePassword(281,111 121),
/// lRemoteDir(176,142) eRemoteDir(281,139 176), lPort(175,171) ePort(280,168 49 가운데), lServerType(175,200) cbServerType(280,197 145),
/// chkPassive(281,228), btnAdd(16,264)·btnDelete(94,264)·btnOk(298,264)·btnCancel(382,264) 75×25.
/// C# 판에만 있는 연결 단추는 삭제와 확인 사이(215,264), 추가 설정은 수동 모드 오른쪽(382,226)에 둔다.
pub(super) fn sites_ui(ui: &mut egui::Ui, form: &mut SitesForm, language: &compat::Language, enter: bool) -> Option<SitesAction> {
    use super::original_dialogs::put_left;
    let text = |key: &str, fallback: &str| language.text(&format!("formFTPSiteManager.{key}"), fallback).replace('&', "");
    let mut action = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(472.0, 297.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let list = at(16.0, 25.0, 153.0, 206.0);
    ui.painter().rect(list, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(2.0)), |ui| {
        egui::ScrollArea::vertical().id_salt("remote-sites-list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, site) in form.sites.iter().enumerate() {
                let response = ui.add(egui::Button::selectable(form.selected == Some(index), &site.name).truncate());
                let response = response.on_hover_text(format!("{} · {}:{} · {}", site.protocol, site.host, site.port, site.remote_directory));
                if response.clicked() && form.selected != Some(index) {
                    action = Some(SitesAction::Select(index));
                }
            }
        });
    });
    let enabled = form.editing.is_some();
    ui.add_enabled_ui(enabled, |ui| {
        // VCL TLabel FocusControl: 단축 문자는 옆 칸에 초점을 준다.
        let raw = |key: &str, fallback: &str| language.raw(&format!("formFTPSiteManager.{key}")).unwrap_or_else(|| fallback.to_owned());
        let label = |ui: &mut egui::Ui, x: f32, y: f32, key: &str, fallback: &str, target: &str| {
            super::original_dialogs::access_label(ui, at(x, y, 103.0, 15.0), &raw(key, fallback), egui::Id::new(("ftp-site", target.to_owned())));
        };
        let id = |name: &str| egui::Id::new(("ftp-site", name.to_owned()));
        label(ui, 176.0, 28.0, "Caption.lName", "Site &Name:", "name");
        ui.put(at(281.0, 25.0, 176.0, 21.0), egui::TextEdit::singleline(&mut form.name).id(id("name")));
        label(ui, 176.0, 56.0, "Caption.lAddress", "Address(&L):", "host");
        ui.put(at(281.0, 53.0, 176.0, 21.0), egui::TextEdit::singleline(&mut form.host).id(id("host")));
        label(ui, 176.0, 85.0, "Caption.lUser", "&User:", "user");
        ui.put(at(281.0, 82.0, 121.0, 21.0), egui::TextEdit::singleline(&mut form.user).id(id("user")));
        label(ui, 176.0, 114.0, "Caption.lPassword", "Pass&word:", "password");
        ui.put(at(281.0, 111.0, 121.0, 21.0), egui::TextEdit::singleline(&mut form.password).password(true).id(id("password")));
        label(ui, 176.0, 142.0, "Caption.lRemoteDir", "&Remote Dir:", "remote");
        ui.put(at(281.0, 139.0, 176.0, 21.0), egui::TextEdit::singleline(&mut form.remote_directory).id(id("remote")));
        label(ui, 175.0, 171.0, "Caption.lPort", "&Port:", "port");
        ui.put(at(280.0, 168.0, 49.0, 21.0), egui::TextEdit::singleline(&mut form.port).horizontal_align(egui::Align::Center).id(id("port")));
        put_left(ui, at(175.0, 200.0, 103.0, 15.0), egui::Label::new(text("Caption.lServerType", "&Server Type:")).truncate());
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(280.0, 197.0, 145.0, 21.0)), |ui| {
            egui::ComboBox::from_id_salt("remote-sites-protocol").width(137.0).selected_text(form.protocol.clone()).show_opaque(ui, |ui| {
                for name in MANAGER_PROTOCOLS {
                    if ui.selectable_label(form.protocol == name, name).clicked() && form.protocol != name {
                        form.protocol = name.into();
                        form.port = if name == "SFTP" { "22" } else { "21" }.into();
                    }
                }
            });
        });
        super::original_dialogs::access_checkbox(ui, at(281.0, 228.0, 99.0, 17.0), &mut form.passive, &raw("Caption.chkPassive", "Passi&ve Mode"));
    });
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFTPSiteManager.{key}")).unwrap_or_else(|| fallback.to_owned());
    let buttons = [
        (16.0, 264.0, raw("Caption.btnAdd", "&Add"), SitesAction::Add),
        (94.0, 264.0, raw("Caption.btnDelete", "&Delete"), SitesAction::Delete),
        (215.0, 264.0, language.text("formFTPQuickConnect.Caption.btnOk", "&Connect").replace('&', ""), SitesAction::Connect),
        (382.0, 226.0, language.text("NDRRemoteSites.Advanced", "Additional settings"), SitesAction::Advanced),
        (298.0, 264.0, text("Caption.btnOk", "OK"), SitesAction::Save),
        (382.0, 264.0, text("Caption.btnCancel", "Cancel"), SitesAction::Cancel),
    ];
    for (x, y, caption, act) in buttons {
        if super::original_dialogs::access_button(ui, at(x, y, 75.0, 25.0), &caption) && action.is_none() {
            action = Some(act);
        }
    }
    if enter && action.is_none() && !ui.ctx().wants_keyboard_input() {
        action = Some(SitesAction::Save);
    }
    action
}
/// RemoteSiteEditorWindow.
pub(super) struct SiteEditorForm {
    ko: bool,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub remote_directory: String,
    pub passive: bool,
    pub encoding: String,
    pub key_path: String,
    pub key_passphrase: String,
    pub fingerprint: String,
    original_host: String,
    original_protocol: String,
    original_port: i64,
    existing_names: Vec<String>,
    require_unique: bool,
}

impl SiteEditorForm {
    pub(super) fn new(source: &SiteProfile, existing_names: Vec<String>, ko: bool) -> Self {
        Self {
            ko,
            name: source.name.clone(),
            protocol: source.protocol.clone(),
            host: source.host.clone(),
            port: source.port.to_string(),
            user: source.user.clone(),
            password: native::unprotect_secret(&source.encrypted_password).unwrap_or_default(),
            remote_directory: source.remote_directory.clone(),
            passive: source.passive,
            encoding: source.encoding_name.clone(),
            key_path: source.private_key_path.clone(),
            key_passphrase: native::unprotect_secret(&source.encrypted_key_passphrase).unwrap_or_default(),
            fingerprint: source.known_host_fingerprint.clone(),
            original_host: prepare_host(&source.host),
            original_protocol: source.protocol.trim().to_owned(),
            original_port: source.port,
            existing_names,
            require_unique: false,
        }
    }

    /// Commit 검사와 결과(키 파일이 없으면 Err(알림 글), 빈 값이면 Err(""): FTPSITE_INVALIDINFO).
    pub(super) fn result(&self) -> Result<SiteProfile, (bool, String)> {
        if !EDITOR_PROTOCOLS.contains(&self.protocol.as_str()) || self.host.is_empty() || self.name.is_empty() {
            return Err((true, "FTPSITE_INVALIDINFO".into()));
        }
        let port = parse_port(&self.port);
        if !self.key_path.is_empty() && !Path::new(&self.key_path).is_file() {
            return Err((false, t(self.ko, "개인 키 파일을 찾을 수 없습니다.", "The private key file was not found.").into()));
        }
        if self.require_unique && self.existing_names.iter().any(|n| names_equal(n, &self.name)) {
            return Err((true, "FTPSITE_EXISTS".into()));
        }
        let keep = self.protocol.eq_ignore_ascii_case(&self.original_protocol)
            && self.protocol.eq_ignore_ascii_case("SFTP")
            && prepare_host(&self.host).to_lowercase() == self.original_host.to_lowercase()
            && port == self.original_port;
        Ok(SiteProfile {
            name: self.name.clone(),
            protocol: self.protocol.clone(),
            host: self.host.clone(),
            port,
            user: self.user.clone(),
            encrypted_password: native::protect_secret(&self.password).unwrap_or_default(),
            remote_directory: self.remote_directory.clone(),
            passive: self.passive,
            encoding_name: if self.encoding.trim().is_empty() { "UTF-8".into() } else { self.encoding.trim().to_owned() },
            private_key_path: self.key_path.trim().to_owned(),
            encrypted_key_passphrase: native::protect_secret(&self.key_passphrase).unwrap_or_default(),
            known_host_fingerprint: if keep { self.fingerprint.trim().to_owned() } else { String::new() },
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditorAction {
    BrowseKey,
    Save,
    Cancel,
}

pub(super) fn site_editor_ui(ui: &mut egui::Ui, form: &mut SiteEditorForm, language: &compat::Language, enter: bool) -> Option<EditorAction> {
    let ko = form.ko;
    let mut action = None;
    ui.set_width(584.0);
    egui::ScrollArea::vertical().id_salt("remote-site-editor").max_height(520.0).show(ui, |ui| {
        let label = |ui: &mut egui::Ui, text: &str| {
            ui.add_space(8.0);
            ui.label(text);
        };
        label(ui, t(ko, "프로필 이름", "Profile name"));
        ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY));
        label(ui, t(ko, "프로토콜", "Protocol"));
        egui::ComboBox::from_id_salt("remote-site-editor-protocol").width(200.0).selected_text(form.protocol.clone()).show_opaque(ui, |ui| {
            for name in EDITOR_PROTOCOLS {
                if ui.selectable_label(form.protocol == name, name).clicked() && form.protocol != name {
                    form.protocol = name.into();
                    form.port = if name == "SFTP" { "22" } else { "21" }.into();
                }
            }
        });
        label(ui, t(ko, "호스트", "Host"));
        ui.add(egui::TextEdit::singleline(&mut form.host).desired_width(f32::INFINITY));
        label(ui, t(ko, "포트", "Port"));
        ui.add(egui::TextEdit::singleline(&mut form.port).desired_width(f32::INFINITY));
        label(ui, t(ko, "사용자 이름", "User name"));
        ui.add(egui::TextEdit::singleline(&mut form.user).desired_width(f32::INFINITY));
        label(ui, t(ko, "비밀번호", "Password"));
        ui.add(egui::TextEdit::singleline(&mut form.password).password(true).desired_width(f32::INFINITY));
        label(ui, t(ko, "원격 시작 경로", "Remote start folder"));
        ui.add(egui::TextEdit::singleline(&mut form.remote_directory).desired_width(f32::INFINITY));
        label(ui, t(ko, "목록 문자 인코딩", "Listing character encoding"));
        ui.add(egui::TextEdit::singleline(&mut form.encoding).desired_width(f32::INFINITY));
        ui.checkbox(&mut form.passive, t(ko, "FTP 수동 모드", "FTP passive mode"));
        label(ui, t(ko, "SFTP 개인 키 (선택 사항)", "SFTP private key (optional)"));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut form.key_path).desired_width(440.0));
            if ui.button(t(ko, "키 파일 선택", "Choose key file")).clicked() {
                action = Some(EditorAction::BrowseKey);
            }
        });
        label(ui, t(ko, "개인 키 암호", "Private key passphrase"));
        ui.add(egui::TextEdit::singleline(&mut form.key_passphrase).password(true).desired_width(f32::INFINITY));
        if form.protocol.eq_ignore_ascii_case("SFTP") {
            label(ui, &language.text("NDRSFTPHostKey.StoredHostFingerprint", "Stored SFTP host fingerprint (SHA-256)"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::multiline(&mut form.fingerprint.as_str()).desired_rows(2).desired_width(400.0));
                if ui.button(language.text("NDRSFTPHostKey.ForgetHostKey", "Forget SFTP host key")).clicked() {
                    form.fingerprint.clear();
                }
            });
        }
    });
    ui.add_space(10.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        if ui.add(egui::Button::new(t(ko, "취소", "Cancel")).min_size(egui::vec2(82.0, 26.0))).clicked() {
            action = Some(EditorAction::Cancel);
        }
        if ui.add(egui::Button::new(t(ko, "저장", "Save")).min_size(egui::vec2(82.0, 26.0))).clicked() {
            action = Some(EditorAction::Save);
        }
    });
    if enter && action.is_none() && !ui.ctx().wants_keyboard_input() {
        action = Some(EditorAction::Save);
    }
    action
}

impl App {
    pub(super) fn sites_form(&mut self) -> Option<&mut SitesForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| match &mut d.kind {
            DialogKind::RemoteSites(form) => Some(form.as_mut()),
            _ => None,
        })
    }

    fn site_title(&self) -> String {
        self.language_pack.text("formFTPSiteManager.Form.Caption", "FTP Site Manager")
    }

    /// TfMsgBox(Caption, "", RES 글, MB_OK | MB_ICONASTERISK).
    fn site_message(&mut self, key: &str, fallback: &str) {
        let title = self.site_title();
        let text = self.language_pack.text(&format!("RES.{key}"), fallback);
        self.message_box(&title, "", &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
    }

    /// C# ManageRemoteSitesAsync.
    pub(super) fn open_remote_sites(&mut self) {
        let form = SitesForm::new(load_sites(&self.settings));
        let title = self.site_title();
        self.push_original_dialog(title, DialogKind::RemoteSites(Box::new(form)), |app, result, _| {
            let DialogResult::RemoteSites(Some((sites, connect))) = result else { return };
            store_sites(&mut app.settings, &sites);
            app.save();
            if let Some(profile) = connect.and_then(|i| sites.get(i)).cloned() {
                app.open_remote_window(Some(profile), None);
            }
        });
    }

    pub(super) fn sites_action(&mut self, action: SitesAction, ctx: &egui::Context) {
        let Some(form) = self.sites_form() else { return };
        match action {
            SitesAction::Select(index) => {
                if form.commit() {
                    form.selected = Some(index);
                    form.load(Some(index));
                } else {
                    let keep = form.editing;
                    form.selected = keep;
                    self.site_message("FTPSITE_INVALIDINFO", "Site name or address is empty.");
                }
            }
            SitesAction::Add => {
                if !form.commit() {
                    return self.site_message("FTPSITE_INVALIDINFO", "Site name or address is empty.");
                }
                let title = self.site_title();
                let message = self.language_pack.text("RES.FTPSITE_ADD", "Type a FTP site name.");
                self.prompt(&title, &message, "", None, false, |app, value, _| {
                    let Some(name) = value.filter(|n| !n.is_empty()) else { return };
                    let Some(form) = app.sites_form() else { return };
                    if form.sites.iter().any(|s| names_equal(&s.name, &name)) {
                        return app.site_message("FTPSITE_EXISTS", "A site has same name already exists.");
                    }
                    form.sites.push(SiteProfile { name, remote_directory: String::new(), host: String::new(), port: 21, passive: true, protocol: "FTP".into(), ..Default::default() });
                    let index = form.sites.len() - 1;
                    form.selected = Some(index);
                    form.load(Some(index));
                });
            }
            SitesAction::Delete => {
                let Some(index) = form.selected else { return };
                let title = self.site_title();
                let text = self.language_pack.text("RES.FTPSITE_DELCONFIRM", "Are you sure you want to delete a selected site?");
                self.message_box(&title, "", &text, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
                    if answer != IDYES {
                        return;
                    }
                    let Some(form) = app.sites_form() else { return };
                    // 지우기 전에 편집 중 참조를 놓는다(원본은 낡은 위치로 다음 줄을 덮어쓴다).
                    form.editing = None;
                    form.sites.remove(index);
                    let next = (!form.sites.is_empty()).then(|| index.min(form.sites.len() - 1));
                    form.selected = next;
                    form.load(next);
                });
            }
            SitesAction::Advanced => {
                if !form.commit() {
                    return self.site_message("FTPSITE_INVALIDINFO", "Site name or address is empty.");
                }
                let Some(index) = form.editing else { return };
                let current = form.sites[index].clone();
                let others = form.sites.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, s)| s.name.clone()).collect();
                let editor = SiteEditorForm::new(&current, others, self.language == "ko");
                let title = self.tr("원격 사이트", "Remote site").into_owned();
                self.push_original_dialog(title, DialogKind::SiteEditor(Box::new(editor)), move |app, result, _| {
                    let DialogResult::SiteEditor(Some(site)) = result else { return };
                    let Some(form) = app.sites_form() else { return };
                    if index < form.sites.len() {
                        form.sites[index] = site;
                        form.selected = Some(index);
                        form.load(Some(index));
                    }
                });
            }
            SitesAction::Connect | SitesAction::Save => {
                if !form.commit() {
                    return self.site_message("FTPSITE_INVALIDINFO", "Site name or address is empty.");
                }
                let connect = if action == SitesAction::Connect {
                    match form.editing {
                        Some(index) => Some(index),
                        None => return,
                    }
                } else {
                    None
                };
                let sites = form.sites.clone();
                self.answer_dialog(DialogResult::RemoteSites(Some((sites, connect))), ctx);
            }
            SitesAction::Cancel => self.answer_dialog(DialogResult::RemoteSites(None), ctx),
        }
    }

    pub(super) fn site_editor_action(&mut self, action: EditorAction, ctx: &egui::Context) {
        let Some(form) = self.original_dialogs.last_mut().and_then(|d| match &mut d.kind {
            DialogKind::SiteEditor(form) => Some(form.as_mut()),
            _ => None,
        }) else {
            return;
        };
        match action {
            EditorAction::BrowseKey => {
                let title = t(form.ko, "SFTP 개인 키 선택", "Select SFTP private key");
                if let Some(path) = (self.open_dialog)(title, None, "", &[]) {
                    if let Some(DialogKind::SiteEditor(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
                        form.key_path = path.to_string_lossy().into_owned();
                    }
                }
            }
            EditorAction::Save => match form.result() {
                Ok(site) => self.answer_dialog(DialogResult::SiteEditor(Some(site)), ctx),
                Err((true, key)) => {
                    let fallback = if key == "FTPSITE_EXISTS" { "A site has same name already exists." } else { "Site name or address is empty." };
                    self.site_message(&key, fallback);
                }
                Err((false, text)) => self.system_box("", &text, MB_OK, |_, _, _| {}),
            },
            EditorAction::Cancel => self.answer_dialog(DialogResult::SiteEditor(None), ctx),
        }
    }

    /// C# RunRemoteCommandAsync: 사이트 고르기 → 명령 입력(처음 값 pwd) → SFTP는 SSH 명령, FTP는 제어 명령 → "원격 명령 결과" 글 창.
    pub(super) fn run_remote_command(&mut self) {
        let sites = load_sites(&self.settings);
        if sites.is_empty() {
            return self.warn("먼저 네트워크 메뉴에서 원격 사이트를 저장하세요.", "Save a remote site from the Network menu first.");
        }
        let labels: Vec<String> = sites.iter().map(|s| format!("{} — {} {}:{}", s.name, s.protocol, s.host, s.port)).collect();
        let ko = self.language == "ko";
        self.choice_dialog(t(ko, "원격 사이트 선택", "Select remote site"), t(ko, "대상 사이트를 선택하세요.", "Select the target site."), labels, 0, move |app, index, _| {
            let Some(index) = index else { return };
            let Some(site) = sites.get(index).cloned() else { return };
            let message = if ko { format!("{} ({})에서 실행할 명령", site.name, site.protocol) } else { format!("Command to run on {} ({})", site.name, site.protocol) };
            app.prompt(t(ko, "원격 명령", "Remote command"), &message, "pwd", None, false, move |app, command, _| {
                let Some(command) = command.filter(|c| !c.trim().is_empty()) else { return };
                app.execute_remote_command(index, command, None);
            });
        });
    }

    fn execute_remote_command(&mut self, index: usize, command: String, accepted: Option<String>) {
        let Some(site) = load_sites(&self.settings).get(index).cloned() else { return };
        let sftp = site.protocol.eq_ignore_ascii_case("SFTP");
        let private_key = PathBuf::from(&site.private_key_path);
        let has_key = !site.private_key_path.trim().is_empty() && private_key.is_file();
        let config = Config {
            name: site.name.clone(),
            protocol: site.protocol.clone(),
            host: site.host.clone(),
            port: site.port as u16,
            user: site.user.clone(),
            password: native::unprotect_secret(&site.encrypted_password).unwrap_or_default(),
            private_key: if has_key { private_key } else { PathBuf::new() },
            key_passphrase: if has_key { native::unprotect_secret(&site.encrypted_key_passphrase).unwrap_or_default() } else { String::new() },
            fingerprint: accepted.unwrap_or_else(|| site.known_host_fingerprint.clone()),
            passive: site.passive,
            encoding_name: site.encoding_name.clone(),
        };
        let connect = self.remote_hooks.connect;
        let directory = site.remote_directory.clone();
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let result = connect(&config).and_then(|mut client| {
                let output = if sftp {
                    client.exec(&command)
                } else {
                    if !directory.trim().is_empty() && directory != "/" {
                        client.command(&format!("CWD {directory}"))?;
                    }
                    client.command(&command)
                };
                let _ = client.close();
                output
            });
            let _ = tx.send(AsyncResult::RemoteCommand { index, command, host: format!("{}:{}", config.host, config.port), result });
        });
    }

    /// 원격 명령 결과: 서버 키 확인이면 묻고 다시, 아니면 글 창 또는 경고 상자.
    pub(super) fn remote_command_done(&mut self, index: usize, command: String, host: String, result: Result<String, String>) {
        let ko = self.language == "ko";
        match result {
            Ok(output) => {
                self.text = Some(TextDocument { path: None, title: t(ko, "원격 명령 결과", "Remote command result").into(), text: output, editable: false, dirty: false });
            }
            Err(error) => {
                let challenge = serde_json::from_str::<Value>(&error).ok().filter(|v| matches!(v.get("kind").and_then(Value::as_str), Some("unknown_host_key" | "host_key_mismatch")));
                let Some(challenge) = challenge else {
                    return self.system_box("NDR", &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                };
                let changed = challenge["kind"] == "host_key_mismatch";
                let candidate = challenge["candidate"].as_str().unwrap_or_default().to_owned();
                let title = self.language_pack.text(if changed { "NDRSFTPHostKey.ChangedTitle" } else { "NDRSFTPHostKey.FirstUseTitle" }, if changed { "SFTP server key changed" } else { "SFTP server verification" });
                let prompt = format!(
                    "{}: {host}\r\n{}: {candidate}\r\n\r\n{}",
                    self.language_pack.text("NDRSFTPHostKey.HostLabel", "Host"),
                    self.language_pack.text("NDRSFTPHostKey.FingerprintLabel", "SHA-256"),
                    self.language_pack.text("NDRSFTPHostKey.TrustQuestion", "Trust this key and continue?")
                );
                self.system_box(&title, &prompt, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| {
                    if answer != IDYES {
                        return app.system_box("NDR", "Key exchange negotiation failed.", MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                    }
                    // C#: 사이트의 KnownHostFingerprint가 바뀌면 저장한다.
                    let mut sites = load_sites(&app.settings);
                    if let Some(site) = sites.get_mut(index) {
                        site.known_host_fingerprint = candidate.clone();
                        store_sites(&mut app.settings, &sites);
                        app.save();
                    }
                    app.execute_remote_command(index, command, Some(candidate));
                });
            }
        }
    }
}

