use super::*;

impl App {
    /// Password box answering a waiting job. OK = typed value, Cancel = None. If the window vanishes without an answer the sender is dropped and the job counts as cancelled.
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
        // Original FUN_004399b4 = C# RequestArchivePasswordAsync: TfGetText(TITLE_PASSWORD, MSG_PASSWORD, empty text, masked).
        if let Some(request) = self.password_request.take() {
            self.open_password_prompt(request);
        }
        self.file_progress_ui(ctx);
        // Result text window (error lists, comparison results, archive properties). Read-only.
        if let Some(document) = &mut self.text {
            let mut close = false;
            egui::Window::new(&document.title)
                .default_width(900.0)
                .default_height(580.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::both().show(ui, |ui| {
                        let edit = egui::TextEdit::multiline(&mut document.text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(26)
                            .interactive(false);
                        ui.add(edit);
                    });
                    if ui.button(if ko { "닫기" } else { "Close" }).clicked() {
                        close = true;
                    }
                });
            if close {
                self.text = None;
            }
        }
    }
}
