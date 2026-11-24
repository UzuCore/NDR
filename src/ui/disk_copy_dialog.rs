//! C# DiskCopyWindow (disk copy, 610x630) + DiskImageService call flow.
//! Modes = create an image from a removable drive / restore an image to a removable drive / copy directly to another removable drive (initial choice).
//! "Include partition table and all partitions" uses the whole physical disk instead of a volume. Direct copy + whole disk is "copy with one drive (swap media)"
//! (read into %TEMP%\NDR-DiskCopy-{guid}.img -> confirm the media swap -> write to the same disk number; if writing fails, keep that image for recovery and
//! switch to restore mode). Direct copy has "verify data after copying".
//! Progress is the status text and bar inside the window. Cancel works only for image creation and the read phase of a one-drive copy. The window cannot close during work;
//! trying to close it (x, Esc, closing the main window) only requests cancel for image creation (C# OnClosing). On success the window closes (C# DialogResult = true).
//! Difference (HANDOFF): an English UI shows all window text in English (C# reverse-looks-up ko.lang values, so only part is translated and the rest stays Korean),
//! device errors are the English text of the Rust disk module, and its safety checks (protected disks, image on another physical disk, volume locks) are stricter than C#.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::disk::{PhysicalDisk, RemovableDrive};
use ndr::msgbox::{IDYES, MB_ICONEXCLAMATION, MB_OK, MB_YESNO};

type DiskProgress<'a> = &'a mut dyn FnMut(u64, u64, bool);

/// Functions that touch devices (swapped in tests): listing, image read/write, device-to-device copy, temp folder.
#[derive(Clone, Copy)]
pub(super) struct DiskHooks {
    pub drives: fn() -> Vec<RemovableDrive>,
    pub physical_disks: fn() -> Vec<PhysicalDisk>,
    /// (device, image file, cancel, progress)
    pub read_image: fn(&str, &Path, &AtomicBool, DiskProgress) -> Result<(), String>,
    /// (image file, device, verify, cancel, progress)
    pub write_image: fn(&Path, &str, bool, &AtomicBool, DiskProgress) -> Result<(), String>,
    /// (source device, destination device, verify, cancel, progress)
    pub copy_device: fn(&str, &str, bool, &AtomicBool, DiskProgress) -> Result<(), String>,
    /// Path.GetTempPath().
    pub temp_dir: fn() -> PathBuf,
}

impl Default for DiskHooks {
    fn default() -> Self {
        Self {
            drives: ndr::disk::ready_removable_drives,
            physical_disks: ndr::disk::ready_removable_physical_disks,
            read_image: ndr::disk::read_image,
            write_image: ndr::disk::write_image,
            copy_device: ndr::disk::copy_device,
            temp_dir: std::env::temp_dir,
        }
    }
}

/// The three radio buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiskMode {
    Create,
    Restore,
    Copy,
}

/// Running stage. Swap is while waiting for the media swap box (no worker).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiskStage {
    Create,
    Restore,
    Copy,
    SameRead,
    Swap,
    SameWrite,
}

/// Choices when the start button was pressed (values C# StartCopyAsync reads up front).
#[derive(Clone, Debug)]
struct DiskPlan {
    create: bool,
    restore: bool,
    whole: bool,
    same: bool,
    verify: bool,
    drive: Option<RemovableDrive>,
    destination_drive: Option<RemovableDrive>,
    physical: Option<PhysicalDisk>,
    destination_physical: Option<PhysicalDisk>,
    image: String,
}

impl DiskPlan {
    fn direct(&self) -> bool {
        !self.create && !self.restore
    }
}

enum DiskEvent {
    Progress(u64, u64, bool),
    Done(Result<(), String>),
}

pub(super) struct DiskRun {
    receiver: Receiver<DiskEvent>,
    cancel: Arc<AtomicBool>,
    pub stage: DiskStage,
    plan: DiskPlan,
    /// C# temporaryImagePath (one-drive copy).
    pub temp: Option<PathBuf>,
}

impl DiskRun {
    /// Stages where the cancel button works (C# _cancel.IsEnabled = create || samePhysicalDrive, off after reading).
    pub(super) fn cancellable(&self) -> bool {
        matches!(self.stage, DiskStage::Create | DiskStage::SameRead)
    }
    /// C# _restoring = !create: trying to close the window does not request cancel.
    fn restoring(&self) -> bool {
        !self.plan.create
    }
}

pub(super) struct DiskCopyForm {
    pub mode: DiskMode,
    pub whole_disk: bool,
    pub same_drive: bool,
    pub verify: bool,
    pub drives: Vec<RemovableDrive>,
    pub physical: Vec<PhysicalDisk>,
    pub drive: Option<usize>,
    pub destination_drive: Option<usize>,
    pub physical_disk: Option<usize>,
    pub destination_physical: Option<usize>,
    /// Read-only field (changed only by browsing).
    pub image_path: String,
    pub status: String,
    pub progress: f32,
    pub run: Option<DiskRun>,
    language: compat::Language,
}

/// Window buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiskAction {
    Browse,
    Start,
    Cancel,
    Close,
}

/// .NET "N0" (comma every three digits in both ko-KR and en-US).
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// C# PhysicalDiskChoice.ToString: "PhysicalDrive{n} · {bytes:N0} bytes" (same text in every language).
pub(super) fn physical_text(disk: &PhysicalDisk, language: &compat::Language) -> String {
    super::file_commands::operation_format(language, "NDR.OP_DISK_PHYSICAL_DESCRIPTOR", "PhysicalDrive{0} · {1} bytes", &[&disk.number, &grouped(disk.bytes)])
}

/// Guid.NewGuid().ToString("N").
fn guid_n() -> String {
    let mut bytes = [0u8; 16];
    if native::random_bytes(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl DiskCopyForm {
    /// C# constructor: direct copy, first drive, destination the second when there are two or more (else the first).
    pub(super) fn new(drives: Vec<RemovableDrive>, physical: Vec<PhysicalDisk>, language: compat::Language) -> Self {
        let second = |count: usize| match count {
            0 => None,
            1 => Some(0),
            _ => Some(1),
        };
        Self {
            mode: DiskMode::Copy,
            whole_disk: false,
            same_drive: false,
            verify: false,
            drive: (!drives.is_empty()).then_some(0),
            destination_drive: second(drives.len()),
            physical_disk: (!physical.is_empty()).then_some(0),
            destination_physical: second(physical.len()),
            drives,
            physical,
            image_path: String::new(),
            status: language.text("NDR.OP_DISK_SELECT_A_REMOVABLE_DRIVE_AND_AN_IMAGE_FILE", "Select a removable drive and an image file.").into(),
            progress: 0.0,
            run: None,
            language,
        }
    }

    fn direct(&self) -> bool {
        self.mode == DiskMode::Copy
    }

    fn same(&self) -> bool {
        self.whole_disk && self.direct() && self.same_drive
    }

    /// DriveChoice.Label: volume name, otherwise "removable disk".
    fn label_of(&self, drive: &RemovableDrive) -> String {
        if drive.label.trim().is_empty() { self.language.text("NDR.OP_DISK_REMOVABLE_DISK", "Removable Disk").into() } else { drive.label.clone() }
    }

    /// DriveChoice.ToString: "{Root} {Label} · {FormatSize(Bytes)}".
    pub(super) fn drive_text(&self, drive: &RemovableDrive) -> String {
        format!("{} {} · {}", drive.root, self.label_of(drive), ndr::parity::format_size(drive.bytes, "Auto", true))
    }

    pub(super) fn drive_label(&self) -> String {
        if self.direct() { self.language.text("NDR.OP_DISK_SOURCE_DRIVE", "Source drive") } else { self.language.text("NDR.OP_DISK_REMOVABLE_DRIVE", "Removable drive") }
    }

    pub(super) fn path_label(&self) -> String {
        match (self.mode == DiskMode::Create, self.whole_disk) {
            (true, true) => self.language.text("NDR.OP_DISK_IMAGE_TO_CREATE", "Image to create"),
            (true, false) => self.language.text("NDR.OP_DISK_IMAGE_TO_SAVE", "Image to save"),
            (false, true) => self.language.text("NDR.OP_DISK_SOURCE_WHOLE_IMAGE", "Source image"),
            (false, false) => self.language.text("NDR.OP_DISK_SOURCE_IMAGE", "Source image"),
        }
    }

    pub(super) fn start_label(&self) -> String {
        match (self.mode, self.whole_disk) {
            (DiskMode::Create, true) => self.language.text("NDR.OP_DISK_CREATE_FULL_DISK_IMAGE", "Create full disk image"),
            (DiskMode::Create, false) => self.language.text("NDR.OP_DISK_CREATE_IMAGE", "Create image"),
            (DiskMode::Restore, true) => self.language.text("NDR.OP_DISK_RESTORE_FULL_DISK", "Restore full disk"),
            (DiskMode::Restore, false) => self.language.text("NDR.OP_DISK_RESTORE_DRIVE", "Restore drive"),
            (DiskMode::Copy, true) => self.language.text("NDR.OP_DISK_CLONE_FULL_DISK", "Clone full disk"),
            (DiskMode::Copy, false) => self.language.text("NDR.OP_DISK_COPY_DRIVE", "Copy drive"),
        }
    }

    /// _warning.Text of C# UpdateMode.
    pub(super) fn warning(&self) -> String {
        if self.whole_disk {
            let mut text = if self.direct() {
                if self.same() {
                    self.language.text("NDR.OP_DISK_NDR_SAVES_THE_SOURCE_TO_A_TEMPORARY_IMAGE_ON", "NDR saves the source to a temporary image on another local disk. Swap in destination media with the same capacity; every byte on it will be overwritten.")
                } else {
                    self.language.text("NDR.OP_DISK_THIS_OVERWRITES_THE_TARGET_DISK_S_PARTITION_TABLE_AND", "This overwrites the target disk's partition table and every byte. Disk capacities must match exactly. The operation cannot be cancelled.")
                }
            } else if self.mode == DiskMode::Create {
                self.language.text("NDR.OP_DISK_THE_IMAGE_INCLUDES_THE_PARTITION_TABLE_AND_EVERY_PARTITION", "The image includes the partition table and every partition. Save it on a different local physical disk.")
            } else {
                self.language.text("NDR.OP_DISK_THE_TARGET_DISK_S_PARTITION_TABLE_AND_EVERY_BYTE", "The target disk's partition table and every byte will be replaced. Image and disk capacities must match exactly.")
            };
            if self.physical.is_empty() {
                text.push_str(&self.language.text("NDR.OP_DISK_NO_REMOVABLE_PHYSICAL_DISKS_WERE_FOUND_DISK_ACCESS_MAY", " No removable physical disks were found. Disk access may require administrator privileges."));
            }
            return text;
        }
        match self.mode {
            DiskMode::Copy => self.language.text("NDR.OP_DISK_THIS_OVERWRITES_ALL_EXISTING_DATA_ON_THE_DESTINATION_DRIVE", "This overwrites all existing data on the destination drive. Source and destination capacities must match, and copying cannot be cancelled."),
            DiskMode::Create => self.language.text("NDR.OP_DISK_SAVE_THE_IMAGE_FILE_OUTSIDE_THE_SOURCE_DRIVE", "Save the image file outside the source drive."),
            DiskMode::Restore => self.language.text("NDR.OP_DISK_RESTORING_OVERWRITES_THE_EXISTING_DATA_ON_THE_TARGET_VOLUME", "Restoring overwrites the existing data on the target volume. Image and target capacities must match exactly."),
        }
        .into()
    }

    /// C# UpdateStartAvailability.
    pub(super) fn can_start(&self) -> bool {
        if self.run.is_some() {
            return false;
        }
        if self.whole_disk {
            let Some(source) = self.physical_disk.and_then(|i| self.physical.get(i)) else { return false };
            if self.direct() {
                if self.same_drive {
                    return true;
                }
                return self.destination_physical.and_then(|i| self.physical.get(i)).is_some_and(|d| d.number != source.number && d.bytes == source.bytes);
            }
            return !self.image_path.trim().is_empty();
        }
        if self.direct() {
            let source = self.drive.and_then(|i| self.drives.get(i));
            let destination = self.destination_drive.and_then(|i| self.drives.get(i));
            return matches!((source, destination), (Some(s), Some(d)) if !s.root.eq_ignore_ascii_case(&d.root));
        }
        self.drive.is_some_and(|i| i < self.drives.len()) && !self.image_path.trim().is_empty()
    }

    /// Reading the choices and the empty-choice check at the start of C# StartCopyAsync.
    fn plan(&self) -> Option<DiskPlan> {
        let direct = self.direct();
        let plan = DiskPlan {
            create: self.mode == DiskMode::Create,
            restore: self.mode == DiskMode::Restore,
            whole: self.whole_disk,
            same: self.same(),
            verify: direct && self.verify,
            drive: self.drive.and_then(|i| self.drives.get(i)).cloned(),
            destination_drive: self.destination_drive.and_then(|i| self.drives.get(i)).cloned(),
            physical: self.physical_disk.and_then(|i| self.physical.get(i)).copied(),
            destination_physical: self.destination_physical.and_then(|i| self.physical.get(i)).copied(),
            image: self.image_path.clone(),
        };
        let has_source = if plan.whole { plan.physical.is_some() } else { plan.drive.is_some() };
        if !has_source || (!direct && plan.image.trim().is_empty()) {
            return None;
        }
        if direct && !plan.same && (if plan.whole { plan.destination_physical.is_none() } else { plan.destination_drive.is_none() }) {
            return None;
        }
        Some(plan)
    }

    /// Confirmation box before restore or direct copy (caption, text).
    fn confirmation(&self, plan: &DiskPlan) -> (String, String) {
        let direct = plan.direct();
        let file_name = Path::new(&plan.image).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if plan.whole {
            let source_disk = plan.physical.expect("checked by plan");
            let target = if direct && !plan.same { plan.destination_physical.expect("checked by plan") } else { source_disk };
            let destination = if plan.same { format!("{} ({})", physical_text(&target, &self.language), self.language.text("NDR.OP_DISK_AFTER_MEDIA_SWAP", "after media swap")) } else { physical_text(&target, &self.language) };
            let source = if direct { physical_text(&source_disk, &self.language) } else { file_name };
            let title = match (direct, plan.same) {
                (true, true) => self.language.text("NDR.OP_DISK_SINGLE_DRIVE_DISK_COPY", "Single-drive disk copy"),
                (true, false) => self.language.text("NDR.OP_DISK_CLONE_FULL_PHYSICAL_DISK", "Clone full physical disk"),
                _ => self.language.text("NDR.OP_DISK_RESTORE_FULL_PHYSICAL_DISK", "Restore full physical disk"),
            };
            let message = super::file_commands::operation_format(&self.language, "NDR.OP_DISK_CONFIRM_PHYSICAL", "The entire physical disk, including its partition table and every partition, {0} will be overwritten with {1}.\n\nDo not disconnect the disk or close NDR during the operation. Continue?", &[&destination, &source]);
            return (title, message);
        }
        let drive = plan.drive.as_ref().expect("checked by plan");
        let target = if direct { plan.destination_drive.as_ref().expect("checked by plan") } else { drive };
        let destination = format!("{} ({})", target.root, self.label_of(target));
        let source = if direct { format!("{} ({})", drive.root, self.label_of(drive)) } else { file_name };
        let title = if direct { self.language.text("NDR.OP_DISK_DIRECT_REMOVABLE_DRIVE_COPY", "Direct removable drive copy") } else { self.language.text("NDR.OP_DISK_RESTORE_REMOVABLE_DRIVE", "Restore removable drive") };
        let message = super::file_commands::operation_format(&self.language, "NDR.OP_DISK_CONFIRM_VOLUME", "The existing volume data on {0} will be overwritten with {1}.\n\nDo not disconnect the drive or close NDR during the operation. Continue?", &[&destination, &source]);
        (title, message)
    }

    fn starting_status(&self, plan: &DiskPlan) -> String {
        if plan.same {
            self.language.text("NDR.OP_DISK_SAVING_THE_SOURCE_DISK_TO_A_TEMPORARY_IMAGE", "Saving the source disk to a temporary image…")
        } else if plan.create {
            if plan.whole { self.language.text("NDR.OP_DISK_READING_FULL_PHYSICAL_DISK", "Reading full physical disk…") } else { self.language.text("NDR.OP_DISK_READING_DATA_FROM_THE_DRIVE", "Reading data from the drive…") }
        } else if plan.whole {
            self.language.text("NDR.OP_DISK_LOCKING_PHYSICAL_DISK_VOLUMES", "Locking physical disk volumes…")
        } else {
            self.language.text("NDR.OP_DISK_LOCKING_THE_DRIVE", "Locking the drive…")
        }
    }

    fn success_text(&self, plan: &DiskPlan) -> String {
        if plan.create {
            if plan.whole { self.language.text("NDR.OP_DISK_FULL_DISK_IMAGE_CREATED", "Full disk image created.") } else { self.language.text("NDR.OP_DISK_DISK_IMAGE_CREATED", "Disk image created.") }
        } else if plan.restore {
            if plan.whole { self.language.text("NDR.OP_DISK_IMAGE_RESTORED_TO_THE_FULL_PHYSICAL_DISK", "Image restored to the full physical disk.") } else { self.language.text("NDR.OP_DISK_IMAGE_RESTORED_TO_THE_DRIVE", "Image restored to the drive.") }
        } else if plan.same {
            self.language.text("NDR.OP_DISK_SINGLE_DRIVE_DISK_COPY_COMPLETED", "Single-drive disk copy completed.")
        } else if plan.whole {
            self.language.text("NDR.OP_DISK_FULL_PHYSICAL_DISK_CLONE_COMPLETED", "Full physical disk clone completed.")
        } else {
            self.language.text("NDR.OP_DISK_DRIVE_TO_DRIVE_COPY_COMPLETED", "Drive-to-drive copy completed.")
        }
    }

    /// C# Progress<DiskCopyProgress>: bar = copied/total, text = "[verify · ]N / M bytes (P0)".
    pub(super) fn show_progress(&mut self, copied: u64, total: u64, verifying: bool) {
        let ratio = if total == 0 { 0.0 } else { copied as f64 / total as f64 };
        self.progress = ratio as f32;
        let prefix = if verifying { format!("{} · ", self.language.text("NDR.OP_DISK_VERIFYING", "Verifying")) } else { String::new() };
        self.status = format!("{prefix}{} / {} {} ({}%)", grouped(copied), grouped(total), self.language.text("NDR.OP_DISK_BYTES", "bytes"), (ratio * 100.0).round());
    }
}

/// Name field 120 + choice box 330 (C# DockPanel row).
fn combo_row(ui: &mut egui::Ui, id: &str, label: &str, items: &[String], selected: &mut Option<usize>, enabled: bool) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(120.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(120.0);
            ui.label(label);
        });
        ui.add_enabled_ui(enabled, |ui| {
            let text = selected.and_then(|i| items.get(i)).cloned().unwrap_or_default();
            egui::ComboBox::from_id_salt(id).width(330.0).selected_text(text).show_opaque(ui, |ui| {
                for (index, item) in items.iter().enumerate() {
                    ui.selectable_value(selected, Some(index), item);
                }
            });
        });
    });
    ui.add_space(10.0);
}

/// Draws the window. A pressed button returns its action.
pub(super) fn disk_copy_ui(ui: &mut egui::Ui, form: &mut DiskCopyForm) -> Option<DiskAction> {
    let language = &form.language;
    let running = form.run.is_some();
    if running {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let mut action = None;
    ui.set_width(574.0);
    ui.add(egui::Label::new(language.text("NDR.OP_DISK_SAVE_A_REMOVABLE_VOLUME_AS_AN_IMAGE_FILE_OR", "Save a removable volume as an image file, or restore the contents of an image or another drive.")).wrap());
    ui.add_space(12.0);
    ui.add_enabled_ui(!running, |ui| {
        for (mode, label) in [
            (DiskMode::Create, language.text("NDR.OP_DISK_MODE_CREATE", "Create an image from a removable drive")),
            (DiskMode::Restore, language.text("NDR.OP_DISK_MODE_RESTORE", "Restore an image to a removable drive")),
            (DiskMode::Copy, language.text("NDR.OP_DISK_MODE_COPY", "Copy directly to another removable drive")),
        ] {
            ui.radio_value(&mut form.mode, mode, label);
            ui.add_space(4.0);
        }
        ui.checkbox(&mut form.whole_disk, language.text("NDR.OP_DISK_INCLUDE_PARTITION_TABLE_AND_EVERY_PARTITION", "Include partition table and every partition"));
    });
    ui.add_space(8.0);
    let direct = form.direct();
    let whole = form.whole_disk;
    let same = form.same();
    let drive_items: Vec<String> = form.drives.iter().map(|d| form.drive_text(d)).collect();
    let physical_items: Vec<String> = form.physical.iter().map(|disk| physical_text(disk, language)).collect();
    if !whole {
        let label = form.drive_label();
        combo_row(ui, "disk-copy-drive", &label, &drive_items, &mut form.drive, !running);
    }
    if direct && !whole {
        combo_row(ui, "disk-copy-destination-drive", &language.text("NDR.OP_DISK_DESTINATION_DRIVE", "Destination drive"), &drive_items, &mut form.destination_drive, !running);
    }
    if whole {
        combo_row(ui, "disk-copy-physical", &language.text("NDR.OP_DISK_PHYSICAL_DISK", "Physical disk"), &physical_items, &mut form.physical_disk, !running);
    }
    if whole && direct && !same {
        combo_row(ui, "disk-copy-destination-physical", &language.text("NDR.OP_DISK_TARGET_PHYSICAL_DISK", "Target physical disk"), &physical_items, &mut form.destination_physical, !running);
    }
    ui.add_enabled_ui(!running, |ui| {
        if whole && direct {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.checkbox(&mut form.same_drive, language.text("NDR.OP_DISK_USE_ONE_DRIVE_AND_SWAP_MEDIA", "Use one drive and swap media"));
            });
            ui.add_space(8.0);
        }
        if direct {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.checkbox(&mut form.verify, language.text("NDR.OP_DISK_VERIFY_COPIED_DATA_AFTER_WRITING", "Verify copied data after writing"));
            });
            ui.add_space(8.0);
        }
    });
    if !direct {
        let label = form.path_label();
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(120.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(120.0);
                ui.label(label);
            });
            ui.add_enabled(!running, egui::TextEdit::singleline(&mut form.image_path.as_str()).desired_width(340.0));
            if ui.add_enabled(!running, egui::Button::new(language.text("NDR.OP_DISK_BROWSE", "Browse…")).min_size(egui::vec2(96.0, 22.0))).clicked() {
                action = Some(DiskAction::Browse);
            }
        });
        ui.add_space(8.0);
    }
    ui.add(egui::Label::new(RichText::new(form.warning()).color(Color32::from_rgb(0xFF, 0xA5, 0x00))).wrap());
    ui.add_space(12.0);
    ui.add(egui::Label::new(form.status.as_str()).wrap());
    ui.add_space(12.0);
    ui.add(egui::ProgressBar::new(form.progress).desired_height(9.0).desired_width(574.0));
    ui.add_space(18.0);
    let can_start = form.can_start();
    let can_cancel = form.run.as_ref().is_some_and(DiskRun::cancellable);
    let start_label = form.start_label();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        if ui.add_enabled(!running, egui::Button::new(language.text("NDR.OP_DISK_CLOSE", "Close")).min_size(egui::vec2(90.0, 26.0))).clicked() {
            action = Some(DiskAction::Close);
        }
        if ui.add_enabled(can_cancel, egui::Button::new(language.text("NDR.OP_DISK_CANCEL", "Cancel")).min_size(egui::vec2(90.0, 26.0))).clicked() {
            action = Some(DiskAction::Cancel);
        }
        if ui.add_enabled(can_start, egui::Button::new(start_label).min_size(egui::vec2(100.0, 26.0))).clicked() {
            action = Some(DiskAction::Start);
        }
    });
    action
}

/// Worker thread: reports progress and completion.
fn spawn(job: impl FnOnce(&AtomicBool, DiskProgress) -> Result<(), String> + Send + 'static) -> (Receiver<DiskEvent>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    thread::spawn(move || {
        let progress_tx = tx.clone();
        let mut progress = move |copied, total, verifying| {
            let _ = progress_tx.send(DiskEvent::Progress(copied, total, verifying));
        };
        let result = job(&flag, &mut progress);
        let _ = tx.send(DiskEvent::Done(result));
    });
    (rx, cancel)
}

/// finally: deletes temporary images that should not be kept.
fn discard_temp(run: &DiskRun) {
    if let Some(temp) = &run.temp {
        let _ = std::fs::remove_file(temp);
    }
}

/// What to do afterwards (decided while borrowing the window, done after releasing it).
enum Follow {
    None,
    Close,
    Swap(String),
    Error { restoring: bool, message: String },
}

impl App {
    /// C# ShowDiskCopyWindow: reads the lists and opens the window.
    pub(super) fn open_disk_copy(&mut self) {
        let form = DiskCopyForm::new((self.disk.drives)(), (self.disk.physical_disks)(), self.language_pack.clone());
        let title = self.language_pack.text("NDR.OP_DISK_DISK_COPY", "Disk Copy");
        self.push_original_dialog(title, DialogKind::DiskCopy(Box::new(form)), |_, _, _| {});
    }

    pub(super) fn disk_copy_form(&mut self) -> Option<&mut DiskCopyForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| match &mut d.kind {
            DialogKind::DiskCopy(form) => Some(form.as_mut()),
            _ => None,
        })
    }

    /// When the main window tries to close: blocks it during work (image creation gets a cancel request) and returns true.
    pub(super) fn disk_copy_blocks_close(&mut self) -> bool {
        let Some(run) = self.disk_copy_form().and_then(|f| f.run.as_ref()) else { return false };
        if !run.restoring() {
            run.cancel.store(true, Ordering::Relaxed);
        }
        true
    }

    pub(super) fn disk_copy_action(&mut self, action: DiskAction, ctx: &egui::Context) {
        match action {
            DiskAction::Browse => self.browse_disk_image(),
            DiskAction::Start => self.start_disk_copy(),
            DiskAction::Cancel => {
                if let Some(run) = self.disk_copy_form().and_then(|f| f.run.as_ref()).filter(|r| r.cancellable()) {
                    run.cancel.store(true, Ordering::Relaxed);
                }
            }
            DiskAction::Close => {
                if self.disk_copy_blocks_close() {
                    return;
                }
                self.close_disk_copy(ctx);
            }
        }
    }

    fn close_disk_copy(&mut self, ctx: &egui::Context) {
        if self.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::DiskCopy(_))) {
            self.answer_dialog(DialogResult::DiskCopy, ctx);
        } else {
            self.original_dialogs.retain(|d| !matches!(d.kind, DialogKind::DiskCopy(_)));
        }
    }

    /// C# BrowseForImage: create uses a save dialog (Documents folder, suggested name, overwrite prompt), otherwise an open dialog.
    fn browse_disk_image(&mut self) {
        let language = self.language_pack.clone();
        let Some(form) = self.disk_copy_form() else { return };
        let create = form.mode == DiskMode::Create;
        let name = if form.whole_disk {
            form.physical_disk.and_then(|i| form.physical.get(i)).map_or_else(|| "whole-disk.img".to_owned(), |d| format!("PhysicalDrive{}.img", d.number))
        } else {
            form.drive.and_then(|i| form.drives.get(i)).map_or_else(|| "removable-disk.img".to_owned(), |d| format!("{}.img", d.root.trim_end_matches('\\').replace(':', "-")))
        };
        let raw = language.text("NDR.OP_DISK_RAW_IMAGE_FILTER", "Raw disk image (*.img)");
        let all = language.text("NDR.OP_COMMON_ALL_FILES_FILTER", "All files (*.*)");
        let filters = [(raw.as_str(), "*.img"), (all.as_str(), "*.*")];
        let picked = if create {
            (self.save_dialog)(&language.text("NDR.OP_DISK_SAVE_DISK_IMAGE", "Save Disk Image"), native::documents_folder().as_deref(), &name, &filters, "img", true)
        } else {
            (self.open_dialog)(&language.text("NDR.OP_DISK_SELECT_DISK_IMAGE", "Select Disk Image"), None, "", &filters)
        };
        if let (Some(path), Some(form)) = (picked, self.disk_copy_form()) {
            form.image_path = path.to_string_lossy().into_owned();
        }
    }

    /// C# StartCopyAsync: restore and direct copy start after a warning confirmation (yes/no).
    fn start_disk_copy(&mut self) {
        let Some(form) = self.disk_copy_form() else { return };
        if !form.can_start() {
            return;
        }
        let Some(plan) = form.plan() else { return };
        if plan.create {
            return self.begin_disk_copy(plan);
        }
        let (caption, message) = form.confirmation(&plan);
        self.system_box(&caption, &message, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| {
            if answer == IDYES {
                app.begin_disk_copy(plan);
            }
        });
    }

    fn begin_disk_copy(&mut self, plan: DiskPlan) {
        let hooks = self.disk;
        let Some(form) = self.disk_copy_form() else { return };
        if form.run.is_some() {
            return;
        }
        form.progress = 0.0;
        form.status = form.starting_status(&plan).into();
        let image = PathBuf::from(&plan.image);
        let volume = |drive: &Option<RemovableDrive>| drive.as_ref().map(|d| d.root.clone()).unwrap_or_default();
        let disk = |disk: &Option<PhysicalDisk>| disk.map(|d| d.device()).unwrap_or_default();
        let source = if plan.whole { disk(&plan.physical) } else { volume(&plan.drive) };
        let mut temp = None;
        let (stage, (receiver, cancel)) = if plan.create {
            (DiskStage::Create, spawn(move |cancel, progress| (hooks.read_image)(&source, &image, cancel, progress)))
        } else if plan.restore {
            (DiskStage::Restore, spawn(move |cancel, progress| (hooks.write_image)(&image, &source, false, cancel, progress)))
        } else if plan.same {
            let path = (hooks.temp_dir)().join(format!("NDR-DiskCopy-{}.img", guid_n()));
            temp = Some(path.clone());
            (DiskStage::SameRead, spawn(move |cancel, progress| (hooks.read_image)(&source, &path, cancel, progress)))
        } else {
            let destination = if plan.whole { disk(&plan.destination_physical) } else { volume(&plan.destination_drive) };
            let verify = plan.verify;
            (DiskStage::Copy, spawn(move |cancel, progress| (hooks.copy_device)(&source, &destination, verify, cancel, progress)))
        };
        form.run = Some(DiskRun { receiver, cancel, stage, plan, temp });
    }

    /// Reads worker notifications (every App::poll).
    pub(super) fn poll_disk_copy(&mut self, ctx: &egui::Context) {
        let Some(form) = self.disk_copy_form() else { return };
        let events: Vec<DiskEvent> = form.run.as_ref().map(|r| r.receiver.try_iter().collect()).unwrap_or_default();
        let mut done = None;
        for event in events {
            match event {
                DiskEvent::Progress(copied, total, verifying) => form.show_progress(copied, total, verifying),
                DiskEvent::Done(result) => done = Some(result),
            }
        }
        if let Some(result) = done {
            self.disk_copy_done(result, ctx);
        }
    }

    /// End of C# StartCopyAsync (success, media swap, cancel, error) and finally.
    fn disk_copy_done(&mut self, result: Result<(), String>, ctx: &egui::Context) {
        let title = self.language_pack.text("NDR.OP_DISK_DISK_COPY", "Disk Copy");
        let Some(form) = self.disk_copy_form() else { return };
        let Some(run) = form.run.take() else { return };
        let language = form.language.clone();
        let follow = match (run.stage, result) {
            (DiskStage::SameRead, Ok(())) => {
                form.status = language.text("NDR.OP_DISK_THE_SOURCE_HAS_BEEN_READ_EJECT_IT_INSERT_DESTINATION", "The source has been read. Eject it, insert destination media of the same capacity, then continue.").into();
                let text = format!("{}\n\n{}", form.status, language.text("NDR.OP_DISK_EVERY_BYTE_ON_THE_DESTINATION_MEDIA_WILL_BE_OVERWRITTEN", "Every byte on the destination media will be overwritten."));
                form.run = Some(DiskRun { stage: DiskStage::Swap, ..run });
                Follow::Swap(text)
            }
            (_, Ok(())) => {
                form.status = form.success_text(&run.plan).into();
                discard_temp(&run);
                Follow::Close
            }
            (DiskStage::Create | DiskStage::SameRead, Err(_)) if run.cancel.load(Ordering::Relaxed) => {
                form.status = language.text("NDR.OP_DISK_IMAGE_CREATION_CANCELLED_THE_INCOMPLETE_IMAGE_FILE_WAS_DELETED", "Image creation cancelled. The incomplete image file was deleted.").into();
                discard_temp(&run);
                Follow::None
            }
            (DiskStage::SameWrite, Err(error)) => {
                // Keep the recovery image and switch to restore mode.
                let temp = run.temp.clone().unwrap_or_default();
                form.image_path = temp.to_string_lossy().into_owned();
                form.mode = DiskMode::Restore;
                let kept = language.text("NDR.OP_DISK_THE_RECOVERY_IMAGE_WAS_KEPT_AT", "The recovery image was kept at");
                Follow::Error { restoring: run.restoring(), message: format!("{error}\n\n{kept}: {}", temp.display()) }
            }
            (_, Err(error)) => {
                discard_temp(&run);
                Follow::Error { restoring: run.restoring(), message: error }
            }
        };
        match follow {
            Follow::None => {}
            Follow::Close => {
                self.completion_notice_operation(self.language_pack.text("NDR.OP_DISK_DISK_OPERATION_COMPLETED", "Disk operation completed"),
                    language.text("NDR.OP_DISK_FINISH_DETAIL", "Disk operation finished successfully."));
                self.close_disk_copy(ctx);
            },
            Follow::Swap(text) => self.system_box(&title, &text, MB_YESNO | MB_ICONEXCLAMATION, |app, answer, _| app.disk_swap_answered(answer == IDYES)),
            Follow::Error { restoring, message } => {
                let prefix = if restoring {
                    language.text("NDR.OP_DISK_AN_ERROR_OCCURRED_WHILE_RESTORING_OR_COPYING_THE_DRIVE", "An error occurred while restoring or copying the drive. Data on the target disk may have been partly changed.\n\n")
                } else {
                    language.text("NDR.OP_DISK_THE_IMAGE_COULD_NOT_BE_CREATED_N_N", "The image could not be created.\n\n")
                };
                self.system_box(&title, &format!("{prefix}{message}"), MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
        }
    }

    /// Media swap box: yes writes the temporary image to the same disk number (no cancel), no writes nothing.
    fn disk_swap_answered(&mut self, yes: bool) {
        let hooks = self.disk;
        let Some(form) = self.disk_copy_form() else { return };
        let Some(run) = form.run.take() else { return };
        let language = &form.language;
        if !yes {
            form.status = language.text("NDR.OP_DISK_COPY_CANCELLED_NOTHING_WAS_WRITTEN_TO_DESTINATION_MEDIA", "Copy cancelled. Nothing was written to destination media.").into();
            discard_temp(&run);
            return;
        }
        form.status = language.text("NDR.OP_DISK_WRITING_THE_IMAGE_TO_DESTINATION_MEDIA", "Writing the image to destination media…").into();
        let image = run.temp.clone().unwrap_or_default();
        let device = run.plan.physical.map(|d| d.device()).unwrap_or_default();
        let verify = run.plan.verify;
        let (receiver, cancel) = spawn(move |cancel, progress| (hooks.write_image)(&image, &device, verify, cancel, progress));
        form.run = Some(DiskRun { receiver, cancel, stage: DiskStage::SameWrite, plan: run.plan, temp: run.temp });
    }
}
