use super::commands::write_new;
use super::*;
use std::fs;

impl App {
    /// 기다리는 작업에 답할 암호 상자. 확인 = 입력값, 취소 = None. 창이 답 없이 사라지면 보내는 쪽이 끊겨 작업은 취소로 본다.
    pub(super) fn open_password_prompt(&mut self, request: PasswordRequest) {
        let title = self.language_pack.text("RES.TITLE_PASSWORD", "Password");
        let message = self.language_pack.text("RES.MSG_PASSWORD", "Enter a password.");
        let reply = request.reply;
        self.prompt(&title, &message, "", None, true, move |_, value, _| {
            let _ = reply.send(value);
        });
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let ko = self.language == "ko";
        // 원본 FUN_004399b4 = C# RequestArchivePasswordAsync: TfGetText(TITLE_PASSWORD, MSG_PASSWORD, 빈 글, 가림).
        if let Some(request) = self.password_request.take() {
            self.open_password_prompt(request);
        }
        self.file_progress_ui(ctx);
        if let Some(document) = &mut self.text {
            let mut close = false;
            let mut save = false;
            egui::Window::new(&document.title)
                .default_width(900.0)
                .default_height(580.0)
                .show(ctx, |ui| {
                    if document.editable {
                        ui.horizontal(|ui| {
                            if ui.button(if ko { "저장" } else { "Save" }).clicked() {
                                save = true;
                            }
                            ui.label(if document.dirty { "*" } else { "" });
                        });
                    }
                    egui::ScrollArea::both().show(ui, |ui| {
                        let edit = egui::TextEdit::multiline(&mut document.text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(26)
                            .interactive(document.editable);
                        if ui.add(edit).changed() {
                            document.dirty = true;
                        }
                    });
                    if ui.button(if ko { "닫기" } else { "Close" }).clicked() {
                        close = true;
                    }
                });
            if save {
                if let Some(path) = &document.path {
                    match atomic_text_save(path, &document.text) {
                        Ok(()) => document.dirty = false,
                        Err(e) => self.status = e,
                    }
                }
            }
            if close && !document.dirty {
                self.text = None;
            } else if close {
                self.status = if ko {
                    "변경 사항을 저장한 후 닫으세요."
                } else {
                    "Save your changes before closing."
                }
                .into();
            }
        }
    }
}

fn atomic_text_save(path: &Path, text: &str) -> Result<(), String> {
    fsops::ensure_plain_path(path)?;
    let temp = path.with_file_name(format!(
        ".ndir-edit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    write_new(&temp, text.as_bytes())?;
    let result = fs::rename(&temp, path).map_err(|e| e.to_string());
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
