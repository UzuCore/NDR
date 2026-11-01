//! In-process SQLCipher vault browser. No plaintext file is created except an
//! explicitly requested export. Passwords are never written to NDR settings.
use super::*;
use ndr::vault::{Vault, VaultItem, SessionPolicy, MAX_FILE_SIZE};
use zeroize::Zeroize;

pub(super) struct VaultDialog {
    path: PathBuf,
    creating: bool,
    password: String,
    confirmation: String,
    vault: Option<Vault>,
    folder: i64,
    folder_stack: Vec<(i64, String)>,
    selected: Option<VaultItem>,
    name: String,
    import_path: String,
    export_path: String,
    text: Option<(i64, String)>,
    error: String,
}

impl Drop for VaultDialog {
    fn drop(&mut self) {
        self.password.zeroize();
        self.confirmation.zeroize();
        if let Some((_, body)) = &mut self.text { body.zeroize(); }
    }
}

impl VaultDialog {
    fn new(path: PathBuf, creating: bool) -> Self {
        Self { path, creating, password: String::new(), confirmation: String::new(), vault: None,
            folder: 1, folder_stack: vec![(1, String::from("/"))], selected: None,
            name: String::new(), import_path: String::new(), export_path: String::new(), text: None, error: String::new() }
    }

    fn authenticate(&mut self) {
        if self.creating && self.password != self.confirmation {
            self.error = "비밀번호가 일치하지 않습니다.".into();
            return;
        }
        let secret = std::mem::take(&mut self.password);
        self.confirmation.zeroize();
        self.confirmation.clear();
        let outcome = if self.creating { Vault::create(&self.path, secret) }
            else { Vault::unlock(&self.path, secret) };
        match outcome {
            Ok(vault) => {
                self.vault = Some(vault);
                self.creating = false;
                self.folder = 1;
                self.folder_stack = vec![(1, "/".into())];
                self.error.clear();
            }
            Err(message) => self.error = message,
        }
    }

    fn lock(&mut self) {
        self.vault = None;
        self.selected = None;
        if let Some((_, body)) = &mut self.text { body.zeroize(); }
        self.text = None;
        self.folder = 1;
        for (_, name) in &mut self.folder_stack { name.zeroize(); }
        self.folder_stack = vec![(1, "/".into())];
        self.import_path.zeroize();
        self.export_path.zeroize();
        self.name.zeroize();
        self.password.zeroize();
        self.password.clear();
        self.confirmation.zeroize();
        self.confirmation.clear();
    }

    fn apply(&mut self, operation: impl FnOnce(&mut Vault) -> Result<(), String>) {
        if let Some(vault) = &mut self.vault {
            match operation(vault) {
                Ok(()) => self.error.clear(),
                Err(message) => self.error = message,
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, policy: SessionPolicy) -> bool {
        if self.vault.as_ref().is_some_and(|vault| vault.expired(policy)) {
            self.lock();
            self.error = "세션이 만료되었습니다. 비밀번호를 다시 입력해야 합니다.".into();
        }
        ui.label(format!("컨테이너: {}", self.path.display()));
        if self.vault.is_none() {
            ui.label(if self.creating { "새 SQLCipher 암호화 컨테이너" } else { "잠긴 암호화 폴더" });
            if self.creating {
                ui.horizontal(|ui| {
                    ui.label("파일명");
                    let mut path = self.path.to_string_lossy().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut path).desired_width(390.0)).changed() {
                        self.path = PathBuf::from(path);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("비밀번호 (16자 이상)");
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
            });
            if self.creating {
                ui.horizontal(|ui| {
                    ui.label("비밀번호 확인");
                    ui.add(egui::TextEdit::singleline(&mut self.confirmation).password(true));
                });
            }
            if ui.button(if self.creating { "암호화 폴더 생성" } else { "잠금 해제" }).clicked() { self.authenticate(); }
            ui.label("비밀번호 복구 기능은 없습니다. 비밀번호를 잊으면 데이터를 복원할 수 없습니다.");
        } else {
            ui.horizontal(|ui| {
                if ui.button("⬅ 상위 폴더").clicked() && self.folder_stack.len() > 1 {
                    if let Some((_, mut name)) = self.folder_stack.pop() { name.zeroize(); }
                    self.folder = self.folder_stack.last().map_or(1, |v| v.0);
                    self.selected = None;
                    self.text = None;
                    if let Some(vault) = &mut self.vault { vault.touch(); }
                }
                ui.label(self.folder_stack.iter().map(|f| f.1.as_str()).collect::<Vec<_>>().join(" / "));
                if ui.button("즉시 잠금").clicked() { self.lock(); }
            });
            if let Some(vault) = &mut self.vault {
                match vault.list(self.folder) {
                    Ok(items) => {
                        egui::ScrollArea::vertical().max_height(225.0).show(ui, |ui| {
                            for item in items {
                                let label = format!("{}  {} {}", if item.folder { "📁" } else { "📄" }, item.name,
                                    if item.folder { String::new() } else { format!("({} B)", item.size) });
                                let chosen = self.selected.as_ref().is_some_and(|v| v.id == item.id);
                                let response = ui.selectable_label(chosen, label);
                                if response.clicked() { vault.touch(); self.selected = Some(item.clone()); self.text = None; }
                                if response.double_clicked() && item.folder {
                                    self.folder = item.id;
                                    self.folder_stack.push((item.id, item.name.clone()));
                                    self.selected = None;
                                    self.text = None;
                                }
                            }
                        });
                    }
                    Err(error) => self.error = error,
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("이름");
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(240.0));
                if ui.button("새 폴더").clicked() {
                    let folder = self.folder; let name = self.name.clone();
                    self.apply(|v| v.add_folder(folder, &name).map(|_| ()));
                }
                if ui.button("새 빈 파일").clicked() {
                    let folder = self.folder; let name = self.name.clone();
                    self.apply(|v| v.add_file(folder, &name, b"").map(|_| ()));
                }
            });
            if let Some(selected) = self.selected.clone() {
                ui.horizontal(|ui| {
                    if ui.button("선택 항목 이름 변경").clicked() {
                        let name = self.name.clone(); self.apply(|v| v.rename(selected.id, &name));
                    }
                    if ui.button("선택 항목 삭제").clicked() {
                        self.apply(|v| v.remove(selected.id));
                        if self.error.is_empty() { self.selected = None; self.text = None; }
                    }
                    if !selected.folder && ui.button("텍스트 편집").clicked() {
                        if let Some(vault) = &mut self.vault {
                            match vault.read_file(selected.id) {
                                Ok(data) if data.len() <= 1024 * 1024 => match String::from_utf8(data) {
                                    Ok(text) => { self.text = Some((selected.id, text)); self.error.clear(); }
                                    Err(_) => self.error = "UTF-8 텍스트가 아닙니다.".into(),
                                },
                                Ok(_) => self.error = "1 MiB 이하 텍스트만 편집할 수 있습니다.".into(),
                                Err(message) => self.error = message,
                            }
                        }
                    }
                });
            }
            if let Some((id, content)) = self.text.as_mut() {
                ui.label("메모리 내 텍스트 편집 (임시 평문 파일 생성 없음)");
                ui.add(egui::TextEdit::multiline(content).desired_rows(5).desired_width(f32::INFINITY));
                if ui.button("암호화 컨테이너에 저장").clicked() {
                    let id = *id; let data = content.as_bytes().to_vec();
                    self.apply(|v| v.update_file(id, &data));
                }
            }
            ui.horizontal(|ui| {
                ui.label("가져올 로컬 파일 경로");
                ui.add(egui::TextEdit::singleline(&mut self.import_path).desired_width(260.0));
                if ui.button("암호화하여 가져오기").clicked() {
                    let source = PathBuf::from(self.import_path.trim());
                    match std::fs::metadata(&source) {
                        Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE_SIZE as u64 => {
                            let folder = self.folder;
                            match (source.file_name().and_then(|n| n.to_str()), std::fs::read(&source)) {
                                (Some(name), Ok(data)) => {
                                    let data = zeroize::Zeroizing::new(data);
                                    self.apply(|v| v.add_file(folder, name, &data).map(|_| ()));
                                }
                                _ => self.error = "가져올 파일을 읽을 수 없습니다.".into(),
                            }
                        }
                        _ => self.error = "파일을 찾지 못했거나 64 MiB 크기 제한을 초과했습니다.".into(),
                    }
                }
            });
            if self.selected.as_ref().is_some_and(|item| !item.folder) {
                ui.horizontal(|ui| {
                    ui.label("명시적 평문 내보내기 경로");
                    ui.add(egui::TextEdit::singleline(&mut self.export_path).desired_width(260.0));
                    if ui.button("평문 파일 내보내기").clicked() {
                        let path = PathBuf::from(self.export_path.trim());
                        let id = self.selected.as_ref().unwrap().id;
                        if path.as_os_str().is_empty() || path.exists() {
                            self.error = "비어 있지 않은 새 경로만 지정할 수 있습니다 (덮어쓰기 금지).".into();
                        } else if let Some(vault) = &mut self.vault {
                            match vault.read_file(id).and_then(|data| {
                                let data = zeroize::Zeroizing::new(data);
                                use std::io::Write;
                                let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
                                out.write_all(&data).map_err(|e| e.to_string())?;
                                out.sync_all().map_err(|e| e.to_string())
                            }) {
                                Ok(()) => self.error = "요청한 경로에 평문을 내보냈습니다. 직접 보안 관리가 필요합니다.".into(),
                                Err(message) => self.error = message,
                            }
                        }
                    }
                });
            }
        }
        if !self.error.is_empty() { ui.colored_label(ui.visuals().error_fg_color, &self.error); }
        ui.separator();
        ui.label("보안: 닫거나 세션이 만료되면 잠금 해제 정보가 폐기됩니다. 잠금 해제 때마다 비밀번호를 다시 입력합니다.");
        ui.button("닫기 (즉시 잠금)").clicked()
    }
}

impl App {
    pub(super) fn open_vault(&mut self, path: PathBuf, creating: bool) {
        self.vault_dialog = Some(VaultDialog::new(path, creating));
    }

    pub(super) fn vault_window_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.vault_dialog.take() else { return };
        if self.settings.flag("VaultLockOnMinimize", false)
            && ctx.input(|input| input.viewport().minimized.unwrap_or(false)) {
            dialog.lock();
        }
        let policy = SessionPolicy::from_settings(&self.settings);
        let mut open = true;
        let mut close = false;
        egui::Window::new("NDR 암호화 폴더 (.ndr)")
            .id(egui::Id::new("ndr-sqlcipher-vault"))
            .open(&mut open).resizable(true).default_width(700.0)
            .show(ctx, |ui| { close = dialog.draw(ui, policy); });
        if open && !close { self.vault_dialog = Some(dialog); }
        // Otherwise the connection and all plaintext/password UI buffers drop.
    }
}
