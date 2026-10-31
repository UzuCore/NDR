//! 원본 임시 폴더 비우기(miEmptyTempClick 0x434344)와 휴지통 비우기(miEmptyRecycleBinClick 0x433094).
//! 휴지통 비우기는 `App::shell_recycle_emptier`를 거친다(시험은 실제 휴지통을 건드리지 않는다).
//! 컴퓨터 청소(TfCleanup, miCleanupDisk)는 사용자 요청으로 뺐다.
use super::*;
use ndr::msgbox::{IDYES, MB_ICONQUESTION, MB_YESNO};

impl App {
    /// 원본 miEmptyTempClick(0x434344): GetTempPathW → GetLongPathNameW(실패하면 그만), 폴더 크기(StrFormatByteSizeW)와
    /// 파일 수로 TfMsgBox(TITLE_EMPTYTEMP, "", MSG_EMPTYTEMP, MB_YESNO | MB_ICONQUESTION). 예면 지우기 작업(종류 2,
    /// 플래그 0xF0 = 안의 항목만, 묻지 않고, 오류는 건너뛰고 알리지 않음)을 시작하고 두 패널을 새로 읽는다.
    pub(super) fn empty_temp_command(&mut self) {
        let Some(folder) = (self.system.temp_folder)() else { return };
        // 셈은 따로 돌린다(원본은 UI 스레드에서 세어 큰 임시 폴더면 잠시 멈춘다). 끝나면 empty_temp_counted가 묻는다.
        self.status = self.tr("임시 폴더 크기를 세고 있습니다…", "Counting the temporary folder…").into_owned();
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let summary = ndr::tools::folder_size_summary(std::slice::from_ref(&folder), &std::sync::atomic::AtomicBool::new(false)).unwrap_or_default();
            let _ = tx.send(AsyncResult::TempCounted { folder, files: summary.files, bytes: summary.total_bytes });
        });
    }

    pub(super) fn empty_temp_counted(&mut self, folder: PathBuf, files: u64, bytes: u64) {
        self.status.clear();
        let title = self.language_pack.text("RES.TITLE_EMPTYTEMP", "Empty a temporary folder");
        let text = self
            .language_pack
            .text("RES.MSG_EMPTYTEMP", "Are you sure you want to empty a temporary folder?\n%d files, %s")
            .replacen("%d", &files.to_string(), 1)
            .replacen("%s", &ndr::native::format_byte_size(bytes), 1);
        self.message_box(&title, "", &text, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
            if answer != IDYES {
                return;
            }
            app.start_job("Delete".into(), ndr::fsops::Operation::EmptyFolder { folder }, super::file_operation::JobReport::Delete { recycle: false, skipped: 0 }, None);
            for pane in 0..2 {
                app.refresh(pane);
            }
        });
    }

    /// 원본 miEmptyRecycleBinClick(0x433094): SHEmptyRecycleBinW(창, NULL, 0) — 확인·진행은 셸 창, NDR 글 없음.
    pub(super) fn empty_recycle_command(&mut self) {
        let emptier = self.shell_recycle_emptier;
        thread::spawn(move || {
            let _ = emptier();
        });
    }
}
