//! Background filesystem job lifecycle. The UI never performs file I/O here.
//! Keep per-job conflict state, bounded event polling and completion reports together.
use super::*;

impl App {
    pub(super) fn start_job(
        &mut self,
        title: String,
        operation: Operation,
        report: file_operation::JobReport,
        progress: Option<file_operation::ProgressInfo>,
    ) {
        let reservation = file_operation::TransferReservation::from_operation(&operation);
        let active = self.job.as_ref().is_some_and(|j| !j.finished);
        if (active && reservation.is_none())
            || (reservation.is_none() && !self.background_jobs.is_empty())
            || (active && self.job.as_ref().is_some_and(|j| j.reservation.is_none()))
        {
            self.status = self
                .tr(
                    "진행 중인 작업을 먼저 마치세요.",
                    "Finish the current operation first.",
                )
                .into();
            return;
        }
        if let Some(request) = &reservation {
            let conflicts = self
                .job
                .iter()
                .chain(self.background_jobs.iter())
                .filter(|job| !job.finished)
                .filter_map(|job| job.reservation.as_ref())
                .any(|running| request.overlaps(running));
            if conflicts {
                self.status = self
                    .tr(
                        "동일한 파일을 처리하는 다른 작업이 진행 중입니다.",
                        "Another operation is using the same file paths.",
                    )
                    .into();
                return;
            }
            if self.background_jobs.len() + usize::from(active) >= 16 {
                self.status = self
                    .tr(
                        "동시 파일 작업은 최대 16개까지 가능합니다.",
                        "At most 16 concurrent file operations are allowed.",
                    )
                    .into();
                return;
            }
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let mut job = Job::new(rx, cancel, title);
        self.next_job_serial = self.next_job_serial.wrapping_add(1);
        job.serial = self.next_job_serial;
        job.reservation = reservation;
        // C# ErrorResolver: copy/move/delete ask on each error (wipe asks all at once at the end).
        let options = fsops::OperationOptions {
            remove_read_only: self.settings.flag("RemoveReadOnlyWhenCopying", true),
            use_descriptions: self.settings.flag("UseDescriptionFile", true),

            interactive_errors: matches!(
                operation,
                Operation::Copy { .. }
                    | Operation::Delete { .. }
                    | Operation::DeleteSkipping { .. }
            ),
            prompt_rename: matches!(operation, Operation::Copy { .. }),
        };
        thread::spawn(move || fsops::run_with_options(operation, tx, flag, options));
        if !active {
            self.conflict_all = None;
        }
        job.report = report;
        job.progress = progress;
        if active {
            self.background_jobs.push(job);
        } else {
            self.job = Some(job);
        }
    }
    pub(super) fn poll_active_job_events(&mut self) {
        let mut finished = None;
        let mut close_modal = false;
        // Never drain an unbounded number of filesystem notifications inside
        // one UI frame. Retain the last progress snapshot between important
        // events, but preserve the ordering of conflicts, errors and finish.
        const MAX_EVENTS_PER_JOB_PER_FRAME: usize = 96;
        let mut events: Vec<Event> = Vec::new();
        if let Some(job) = self.job.as_ref() {
            for event in job.receiver.try_iter().take(MAX_EVENTS_PER_JOB_PER_FRAME) {
                if matches!(event, Event::Progress { .. })
                    && matches!(events.last(), Some(Event::Progress { .. }))
                {
                    *events.last_mut().unwrap() = event;
                } else {
                    events.push(event);
                }
            }
        }
        for event in events {
            let Some(job) = self.job.as_mut() else { break };
            match event {
                Event::Progress { path, bytes, files } => {
                    job.path = path.to_string_lossy().into_owned();
                    job.bytes = bytes;
                    job.files = files;
                }
                Event::Conflict {
                    source,
                    destination,
                    allow_resume,
                    reply,
                } => self.ask_file_conflict(source, destination, allow_resume, reply),
                Event::Target {
                    source,
                    destination,
                } => job.target = Some((source, destination)),
                Event::Error { path, message } => job.errors.push(if path.as_os_str().is_empty() {
                    message
                } else {
                    format!("{}: {message}", path.display())
                }),
                Event::Failure {
                    path,
                    message,
                    can_continue,
                    reply,
                } => {
                    job.errors.push(format!("{}: {message}", path.display()));
                    job.interactive += 1;
                    self.ask_file_failure(path, message, can_continue, reply);
                }
                Event::Skipped { .. } => job.skipped += 1,
                Event::Totals { files, bytes } => {
                    if let Some(progress) = job.progress.as_mut() {
                        if files.is_some() {
                            progress.total_files = files;
                        }
                        if bytes.is_some() {
                            progress.total_bytes = bytes;
                        }
                    }
                }
                Event::RenameTarget { name, reply } => self.ask_rename_target(name, reply),
                Event::Fatal(message) => job.fatal = Some(message),
                Event::Password { archive, reply } => {
                    self.password_request = Some(PasswordRequest { archive, reply });
                }
                Event::ChangeVolume { volume, reply } => {
                    // Original 0x439593: wsprintf(ARC_CHANGEVOL, volume name) -> TfMsgBox(TITLE_ERROR, "", text, 0x31). OK (1) retries.
                    let title = self.language_pack.text("RES.TITLE_ERROR", "Error");
                    let text = self
                        .language_pack
                        .text(
                            "RES.ARC_CHANGEVOL",
                            "%s\nPlease, insert a disk containing this file and click 'OK'",
                        )
                        .replacen("%s", &volume, 1);
                    self.message_box(
                        &title,
                        "",
                        &text,
                        ndr::msgbox::MB_OKCANCEL | ndr::msgbox::MB_ICONEXCLAMATION,
                        move |_, answer, _| {
                            let _ = reply.send(answer == ndr::msgbox::IDOK);
                        },
                    );
                }
                Event::Finished {
                    cancelled,
                    files,
                    bytes,
                } => {
                    job.finished = true;
                    if job.modal.is_some() {
                        close_modal = true;
                        continue;
                    }
                    if files > 0 {
                        job.files = files;
                    }
                    if bytes > 0 {
                        job.bytes = bytes;
                    }
                    finished = Some(cancelled);
                }
            }
        }
        if close_modal {
            self.job = None;
        }
        if let Some(cancelled) = finished {
            self.report_job(cancelled);
        }
    }
    pub(super) fn poll_background_jobs(&mut self) {
        let primary = self.job.take();
        let jobs = std::mem::take(&mut self.background_jobs);
        for job in jobs {
            self.job = Some(job);
            self.poll_active_job_events();
            if let Some(job) = self.job.take() {
                if !job.finished {
                    self.background_jobs.push(job);
                }
            }
        }
        self.job = primary;
    }
}
