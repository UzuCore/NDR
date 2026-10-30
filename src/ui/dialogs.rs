use super::*;

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
        // 결과 글 창(오류 목록·비교 결과·압축 파일 속성). 읽기 전용.
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
