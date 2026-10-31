//! Original Empty Temp Folder (miEmptyTempClick 0x434344) and Empty Recycle Bin (miEmptyRecycleBinClick 0x433094).
//! Emptying the Recycle Bin goes through `App::shell_recycle_emptier` (tests never touch the real Recycle Bin).
//! Disk Cleanup (TfCleanup, miCleanupDisk) was removed per user request.
use super::*;
use ndr::msgbox::{IDYES, MB_ICONQUESTION, MB_YESNO};

impl App {
    /// Original miEmptyTempClick (0x434344): GetTempPathW -> GetLongPathNameW (stops on failure), then TfMsgBox(TITLE_EMPTYTEMP, "", MSG_EMPTYTEMP, MB_YESNO | MB_ICONQUESTION)
    /// with the folder size (StrFormatByteSizeW) and file count. Yes starts a delete job (mode 2,
    /// flags 0xF0 = contents only, no prompts, errors skipped silently) and rereads both panes.
    pub(super) fn empty_temp_command(&mut self) {
        let Some(folder) = (self.system.temp_folder)() else { return };
        // Counting runs separately (the original counts on the UI thread and stalls briefly on a large temp folder). empty_temp_counted asks when it finishes.
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

    /// Original miEmptyRecycleBinClick (0x433094): SHEmptyRecycleBinW(window, NULL, 0); confirmation and progress are shell windows, no NDR text.
    pub(super) fn empty_recycle_command(&mut self) {
        let emptier = self.shell_recycle_emptier;
        thread::spawn(move || {
            let _ = emptier();
        });
    }
}
