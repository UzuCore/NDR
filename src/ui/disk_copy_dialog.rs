//! C# DiskCopyWindow(디스크 복사, 610×630) + DiskImageService 호출 흐름.
//! 방식 = 이동식 드라이브에서 이미지 만들기 / 이미지를 이동식 드라이브에 복원 / 다른 이동식 드라이브로 직접 복사(처음 선택).
//! "파티션 테이블과 모든 파티션 포함"이면 볼륨 대신 물리 디스크 전체. 직접 복사 + 전체 디스크는 "한 드라이브로 복사(미디어 교체)"
//! (%TEMP%\NDR-DiskCopy-{guid}.img에 읽고 → 미디어 교체 확인 → 같은 디스크 번호에 쓰기, 쓰기가 실패하면 그 이미지를 복구용으로 남기고
//! 복원 방식으로 바꿔 둔다). 직접 복사는 "복사 후 데이터 읽기 검증".
//! 진행은 창 안의 상태 글과 막대다. 취소는 이미지 만들기와 한 드라이브 복사의 읽기 단계만. 작업 중에는 창을 닫을 수 없고
//! 닫으려 하면(×, Esc, 메인 창 닫기) 이미지 만들기만 취소를 요청한다(C# OnClosing). 성공하면 창이 닫힌다(C# DialogResult = true).
//! 차이(HANDOFF): 영어 화면은 창 글이 모두 영어(C#은 ko.lang 값 역검색이라 일부만 번역되고 나머지는 한국어),
//! 장치 오류 글은 Rust 디스크 모듈의 영어 글, Rust 디스크 모듈의 안전 검사(보호 디스크·이미지는 다른 물리 디스크·볼륨 잠금)는 C#보다 엄격하다.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::disk::{PhysicalDisk, RemovableDrive};
use ndr::msgbox::{IDYES, MB_ICONEXCLAMATION, MB_OK, MB_YESNO};

type DiskProgress<'a> = &'a mut dyn FnMut(u64, u64, bool);

/// 장치를 건드리는 함수들(테스트에서 바꿔 끼운다): 목록, 이미지 읽기·쓰기, 장치 간 복사, 임시 폴더.
#[derive(Clone, Copy)]
pub(super) struct DiskHooks {
    pub drives: fn() -> Vec<RemovableDrive>,
    pub physical_disks: fn() -> Vec<PhysicalDisk>,
    /// (장치, 이미지 파일, 취소, 진행)
    pub read_image: fn(&str, &Path, &AtomicBool, DiskProgress) -> Result<(), String>,
    /// (이미지 파일, 장치, 읽기 검증, 취소, 진행)
    pub write_image: fn(&Path, &str, bool, &AtomicBool, DiskProgress) -> Result<(), String>,
    /// (원본 장치, 대상 장치, 읽기 검증, 취소, 진행)
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

/// 세 라디오 단추.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiskMode {
    Create,
    Restore,
    Copy,
}

/// 진행 중인 단계. Swap은 미디어 교체 상자를 기다리는 동안(일꾼 없음).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiskStage {
    Create,
    Restore,
    Copy,
    SameRead,
    Swap,
    SameWrite,
}

/// 시작 단추를 누른 때의 선택(C# StartCopyAsync가 읽어 두는 값).
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
    /// C# temporaryImagePath(한 드라이브 복사).
    pub temp: Option<PathBuf>,
}

impl DiskRun {
    /// 취소 단추를 쓸 수 있는 단계(C# _cancel.IsEnabled = create || samePhysicalDrive, 읽은 뒤에는 끔).
    pub(super) fn cancellable(&self) -> bool {
        matches!(self.stage, DiskStage::Create | DiskStage::SameRead)
    }
    /// C# _restoring = !create: 창을 닫으려 해도 취소를 요청하지 않는다.
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
    /// 읽기 전용 칸(찾아보기로만 바뀐다).
    pub image_path: String,
    pub status: String,
    pub progress: f32,
    pub run: Option<DiskRun>,
    ko: bool,
}

/// 창 단추.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiskAction {
    Browse,
    Start,
    Cancel,
    Close,
}

fn t(ko: bool, korean: &'static str, english: &'static str) -> &'static str {
    if ko { korean } else { english }
}

/// .NET "N0"(ko-KR·en-US 모두 쉼표 세 자리).
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

/// C# PhysicalDiskChoice.ToString: "PhysicalDrive{n} · {bytes:N0} bytes"(언어와 상관없이 같은 글).
pub(super) fn physical_text(disk: &PhysicalDisk) -> String {
    format!("PhysicalDrive{} · {} bytes", disk.number, grouped(disk.bytes))
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
    /// C# 생성자: 직접 복사, 첫 드라이브, 대상은 둘 이상이면 두 번째(하나면 첫 번째).
    pub(super) fn new(drives: Vec<RemovableDrive>, physical: Vec<PhysicalDisk>, ko: bool) -> Self {
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
            status: t(ko, "이동식 드라이브와 이미지 파일을 선택하세요.", "Select a removable drive and an image file.").into(),
            progress: 0.0,
            run: None,
            ko,
        }
    }

    fn direct(&self) -> bool {
        self.mode == DiskMode::Copy
    }

    fn same(&self) -> bool {
        self.whole_disk && self.direct() && self.same_drive
    }

    /// DriveChoice.Label: 볼륨 이름, 없으면 "이동식 디스크".
    fn label_of(&self, drive: &RemovableDrive) -> String {
        if drive.label.trim().is_empty() { t(self.ko, "이동식 디스크", "Removable Disk").into() } else { drive.label.clone() }
    }

    /// DriveChoice.ToString: "{Root} {Label} · {FormatSize(Bytes)}".
    pub(super) fn drive_text(&self, drive: &RemovableDrive) -> String {
        format!("{} {} · {}", drive.root, self.label_of(drive), ndr::parity::format_size(drive.bytes, "Auto", true))
    }

    pub(super) fn drive_label(&self) -> &'static str {
        if self.direct() { t(self.ko, "원본 드라이브", "Source drive") } else { t(self.ko, "이동식 드라이브", "Removable drive") }
    }

    pub(super) fn path_label(&self) -> &'static str {
        match (self.mode == DiskMode::Create, self.whole_disk) {
            (true, true) => t(self.ko, "저장할 전체 디스크 이미지", "Image to create"),
            (true, false) => t(self.ko, "저장할 이미지", "Image to save"),
            (false, true) => t(self.ko, "원본 전체 디스크 이미지", "Source image"),
            (false, false) => t(self.ko, "원본 이미지", "Source image"),
        }
    }

    pub(super) fn start_label(&self) -> &'static str {
        let ko = self.ko;
        match (self.mode, self.whole_disk) {
            (DiskMode::Create, true) => t(ko, "전체 디스크 이미지 만들기", "Create full disk image"),
            (DiskMode::Create, false) => t(ko, "이미지 만들기", "Create image"),
            (DiskMode::Restore, true) => t(ko, "전체 디스크 복원", "Restore full disk"),
            (DiskMode::Restore, false) => t(ko, "드라이브 복원", "Restore drive"),
            (DiskMode::Copy, true) => t(ko, "전체 디스크 복제", "Clone full disk"),
            (DiskMode::Copy, false) => t(ko, "드라이브 복사", "Copy drive"),
        }
    }

    /// C# UpdateMode의 _warning.Text.
    pub(super) fn warning(&self) -> String {
        let ko = self.ko;
        if self.whole_disk {
            let mut text = String::from(if self.direct() {
                if self.same() {
                    t(ko, "원본을 Windows 임시 폴더의 이미지 파일로 저장한 뒤 미디어를 바꿉니다. 같은 용량의 대상 미디어를 넣으면 전체 데이터를 덮어씁니다.",
                        "NDR saves the source to a temporary image on another local disk. Swap in destination media with the same capacity; every byte on it will be overwritten.")
                } else {
                    t(ko, "대상 물리 디스크의 파티션 테이블과 모든 데이터를 덮어씁니다. 디스크 용량이 정확히 같아야 하며, 작업 중에는 취소할 수 없습니다.",
                        "This overwrites the target disk's partition table and every byte. Disk capacities must match exactly. The operation cannot be cancelled.")
                }
            } else if self.mode == DiskMode::Create {
                t(ko, "전체 디스크 이미지에는 파티션 테이블과 모든 파티션이 들어갑니다. 이미지 파일은 다른 로컬 물리 디스크에 저장하세요.",
                    "The image includes the partition table and every partition. Save it on a different local physical disk.")
            } else {
                t(ko, "대상 물리 디스크의 파티션 테이블과 모든 데이터가 이미지로 교체됩니다. 이미지와 디스크 용량이 정확히 같아야 합니다.",
                    "The target disk's partition table and every byte will be replaced. Image and disk capacities must match exactly.")
            });
            if self.physical.is_empty() {
                text.push_str(t(ko, " 이동식 물리 디스크를 찾지 못했습니다. 디스크 복사에는 관리자 권한이 필요할 수 있습니다.",
                    " No removable physical disks were found. Disk access may require administrator privileges."));
            }
            return text;
        }
        match self.mode {
            DiskMode::Copy => t(ko, "대상 드라이브의 모든 기존 데이터를 덮어씁니다. 원본과 대상의 용량이 같아야 하며, 복사 중에는 취소할 수 없습니다.",
                "This overwrites all existing data on the destination drive. Source and destination capacities must match, and copying cannot be cancelled."),
            DiskMode::Create => t(ko, "이미지 파일은 원본 드라이브 바깥에 저장하세요.", "Save the image file outside the source drive."),
            DiskMode::Restore => t(ko, "복원은 대상 볼륨의 기존 데이터를 덮어쓰며, 이미지와 대상 용량이 정확히 같아야 합니다.",
                "Restoring overwrites the existing data on the target volume. Image and target capacities must match exactly."),
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

    /// C# StartCopyAsync 앞부분의 선택 읽기와 빈 선택 검사.
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

    /// 복원·직접 복사 전 확인 상자(캡션, 글).
    fn confirmation(&self, plan: &DiskPlan) -> (String, String) {
        let ko = self.ko;
        let direct = plan.direct();
        let file_name = Path::new(&plan.image).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if plan.whole {
            let source_disk = plan.physical.expect("checked by plan");
            let target = if direct && !plan.same { plan.destination_physical.expect("checked by plan") } else { source_disk };
            let destination = if plan.same { format!("{} ({})", physical_text(&target), t(ko, "미디어 교체 후", "after media swap")) } else { physical_text(&target) };
            let source = if direct { physical_text(&source_disk) } else { file_name };
            let title = match (direct, plan.same) {
                (true, true) => t(ko, "한 드라이브 디스크 복사", "Single-drive disk copy"),
                (true, false) => t(ko, "전체 물리 디스크 복제", "Clone full physical disk"),
                _ => t(ko, "전체 물리 디스크 복원", "Restore full physical disk"),
            };
            let scope = t(ko, "파티션 테이블과 모든 파티션을 포함한 물리 디스크 전체가", "The entire physical disk, including its partition table and every partition,");
            let message = if ko {
                format!("{destination}의 {scope} {source} 내용으로 덮어써집니다.\n\n작업 중에는 디스크를 분리하거나 NDR을 종료하지 마세요. 계속할까요?")
            } else {
                format!("{scope} {destination} will be overwritten with {source}.\n\nDo not disconnect the disk or close NDR during the operation. Continue?")
            };
            return (title.into(), message);
        }
        let drive = plan.drive.as_ref().expect("checked by plan");
        let target = if direct { plan.destination_drive.as_ref().expect("checked by plan") } else { drive };
        let destination = format!("{} ({})", target.root, self.label_of(target));
        let source = if direct { format!("{} ({})", drive.root, self.label_of(drive)) } else { file_name };
        let title = if direct { t(ko, "이동식 드라이브 직접 복사", "Direct removable drive copy") } else { t(ko, "이동식 드라이브 복원", "Restore removable drive") };
        let message = if ko {
            format!("{destination}의 기존 볼륨 데이터가 {source} 내용으로 덮어씁니다.\n\n작업 중에는 드라이브를 분리하거나 NDR을 종료하지 마세요. 계속할까요?")
        } else {
            format!("The existing volume data on {destination} will be overwritten with {source}.\n\nDo not disconnect the drive or close NDR during the operation. Continue?")
        };
        (title.into(), message)
    }

    fn starting_status(&self, plan: &DiskPlan) -> &'static str {
        let ko = self.ko;
        if plan.same {
            t(ko, "원본 디스크를 임시 이미지로 저장하고 있습니다…", "Saving the source disk to a temporary image…")
        } else if plan.create {
            if plan.whole { t(ko, "전체 물리 디스크에서 데이터를 읽고 있습니다…", "Reading full physical disk…") } else { t(ko, "드라이브에서 데이터를 읽고 있습니다…", "Reading data from the drive…") }
        } else if plan.whole {
            t(ko, "물리 디스크 볼륨을 잠그고 있습니다…", "Locking physical disk volumes…")
        } else {
            t(ko, "드라이브를 잠그고 있습니다…", "Locking the drive…")
        }
    }

    fn success_text(&self, plan: &DiskPlan) -> &'static str {
        let ko = self.ko;
        if plan.create {
            if plan.whole { t(ko, "전체 디스크 이미지를 만들었습니다.", "Full disk image created.") } else { t(ko, "디스크 이미지를 만들었습니다.", "Disk image created.") }
        } else if plan.restore {
            if plan.whole { t(ko, "이미지를 전체 물리 디스크에 복원했습니다.", "Image restored to the full physical disk.") } else { t(ko, "이미지를 드라이브에 복원했습니다.", "Image restored to the drive.") }
        } else if plan.same {
            t(ko, "한 드라이브 디스크 복사가 완료되었습니다.", "Single-drive disk copy completed.")
        } else if plan.whole {
            t(ko, "전체 물리 디스크 복제가 완료되었습니다.", "Full physical disk clone completed.")
        } else {
            t(ko, "드라이브 간 복사가 완료되었습니다.", "Drive-to-drive copy completed.")
        }
    }

    /// C# Progress<DiskCopyProgress>: 막대 = 복사/전체, 글 = "[읽기 검증 · ]N / M 바이트 (P0)".
    pub(super) fn show_progress(&mut self, copied: u64, total: u64, verifying: bool) {
        let ratio = if total == 0 { 0.0 } else { copied as f64 / total as f64 };
        self.progress = ratio as f32;
        let prefix = if verifying { format!("{} · ", t(self.ko, "읽기 검증", "Verifying")) } else { String::new() };
        self.status = format!("{prefix}{} / {} {} ({}%)", grouped(copied), grouped(total), t(self.ko, "바이트", "bytes"), (ratio * 100.0).round());
    }
}

/// 이름 칸 120 + 고르기 상자 330(C# DockPanel 줄).
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

/// 창 그리기. 단추를 누르면 그 동작.
pub(super) fn disk_copy_ui(ui: &mut egui::Ui, form: &mut DiskCopyForm) -> Option<DiskAction> {
    let ko = form.ko;
    let running = form.run.is_some();
    if running {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let mut action = None;
    ui.set_width(574.0);
    ui.add(egui::Label::new(t(ko, "이동식 볼륨을 이미지 파일로 저장하거나, 이미지 또는 다른 드라이브의 내용을 복원할 수 있습니다.",
        "Save a removable volume as an image file, or restore the contents of an image or another drive.")).wrap());
    ui.add_space(12.0);
    ui.add_enabled_ui(!running, |ui| {
        for (mode, korean, english) in [
            (DiskMode::Create, "이동식 드라이브에서 이미지 만들기", "Create an image from a removable drive"),
            (DiskMode::Restore, "이미지를 이동식 드라이브에 복원", "Restore an image to a removable drive"),
            (DiskMode::Copy, "다른 이동식 드라이브로 직접 복사", "Copy directly to another removable drive"),
        ] {
            ui.radio_value(&mut form.mode, mode, t(ko, korean, english));
            ui.add_space(4.0);
        }
        ui.checkbox(&mut form.whole_disk, t(ko, "파티션 테이블과 모든 파티션 포함", "Include partition table and every partition"));
    });
    ui.add_space(8.0);
    let direct = form.direct();
    let whole = form.whole_disk;
    let same = form.same();
    let drive_items: Vec<String> = form.drives.iter().map(|d| form.drive_text(d)).collect();
    let physical_items: Vec<String> = form.physical.iter().map(physical_text).collect();
    if !whole {
        let label = form.drive_label();
        combo_row(ui, "disk-copy-drive", label, &drive_items, &mut form.drive, !running);
    }
    if direct && !whole {
        combo_row(ui, "disk-copy-destination-drive", t(ko, "대상 드라이브", "Destination drive"), &drive_items, &mut form.destination_drive, !running);
    }
    if whole {
        combo_row(ui, "disk-copy-physical", t(ko, "전체 물리 디스크", "Physical disk"), &physical_items, &mut form.physical_disk, !running);
    }
    if whole && direct && !same {
        combo_row(ui, "disk-copy-destination-physical", t(ko, "대상 물리 디스크", "Target physical disk"), &physical_items, &mut form.destination_physical, !running);
    }
    ui.add_enabled_ui(!running, |ui| {
        if whole && direct {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.checkbox(&mut form.same_drive, t(ko, "한 드라이브로 복사 (미디어 교체)", "Use one drive and swap media"));
            });
            ui.add_space(8.0);
        }
        if direct {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.checkbox(&mut form.verify, t(ko, "복사 후 데이터 읽기 검증", "Verify copied data after writing"));
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
            if ui.add_enabled(!running, egui::Button::new(t(ko, "찾아보기…", "Browse…")).min_size(egui::vec2(96.0, 22.0))).clicked() {
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
        if ui.add_enabled(!running, egui::Button::new(t(ko, "닫기", "Close")).min_size(egui::vec2(90.0, 26.0))).clicked() {
            action = Some(DiskAction::Close);
        }
        if ui.add_enabled(can_cancel, egui::Button::new(t(ko, "취소", "Cancel")).min_size(egui::vec2(90.0, 26.0))).clicked() {
            action = Some(DiskAction::Cancel);
        }
        if ui.add_enabled(can_start, egui::Button::new(start_label).min_size(egui::vec2(100.0, 26.0))).clicked() {
            action = Some(DiskAction::Start);
        }
    });
    action
}

/// 일꾼 스레드: 진행과 끝을 알린다.
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

/// finally: 남기지 않을 임시 이미지를 지운다.
fn discard_temp(run: &DiskRun) {
    if let Some(temp) = &run.temp {
        let _ = std::fs::remove_file(temp);
    }
}

/// 끝난 뒤 할 일(창을 빌린 동안 정하고, 놓은 뒤 한다).
enum Follow {
    None,
    Close,
    Swap(String),
    Error { restoring: bool, message: String },
}

impl App {
    /// C# ShowDiskCopyWindow: 목록을 읽고 창을 연다.
    pub(super) fn open_disk_copy(&mut self) {
        let ko = self.language == "ko";
        let form = DiskCopyForm::new((self.disk.drives)(), (self.disk.physical_disks)(), ko);
        let title = self.tr("디스크 복사", "Disk Copy").into_owned();
        self.push_original_dialog(title, DialogKind::DiskCopy(Box::new(form)), |_, _, _| {});
    }

    pub(super) fn disk_copy_form(&mut self) -> Option<&mut DiskCopyForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| match &mut d.kind {
            DialogKind::DiskCopy(form) => Some(form.as_mut()),
            _ => None,
        })
    }

    /// 메인 창을 닫으려 할 때: 작업 중이면 막고(이미지 만들기는 취소 요청) true.
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

    /// C# BrowseForImage: 만들기는 저장 대화 상자(문서 폴더, 제안 이름, 덮어쓰기 확인), 아니면 열기 대화 상자.
    fn browse_disk_image(&mut self) {
        let ko = self.language == "ko";
        let Some(form) = self.disk_copy_form() else { return };
        let create = form.mode == DiskMode::Create;
        let name = if form.whole_disk {
            form.physical_disk.and_then(|i| form.physical.get(i)).map_or_else(|| "whole-disk.img".to_owned(), |d| format!("PhysicalDrive{}.img", d.number))
        } else {
            form.drive.and_then(|i| form.drives.get(i)).map_or_else(|| "removable-disk.img".to_owned(), |d| format!("{}.img", d.root.trim_end_matches('\\').replace(':', "-")))
        };
        let filters = [("Raw disk image (*.img)", "*.img"), (t(ko, "모든 파일 (*.*)", "All files (*.*)"), "*.*")];
        let picked = if create {
            (self.save_dialog)(t(ko, "디스크 이미지 저장", "Save Disk Image"), native::documents_folder().as_deref(), &name, &filters, "img", true)
        } else {
            (self.open_dialog)(t(ko, "디스크 이미지 선택", "Select Disk Image"), None, "", &filters)
        };
        if let (Some(path), Some(form)) = (picked, self.disk_copy_form()) {
            form.image_path = path.to_string_lossy().into_owned();
        }
    }

    /// C# StartCopyAsync: 복원·직접 복사는 경고 확인(예/아니요) 뒤에 시작한다.
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

    /// 일꾼 알림을 읽는다(App::poll에서 매번).
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

    /// C# StartCopyAsync의 끝(성공·미디어 교체·취소·오류)과 finally.
    fn disk_copy_done(&mut self, result: Result<(), String>, ctx: &egui::Context) {
        let title = self.tr("디스크 복사", "Disk Copy").into_owned();
        let Some(form) = self.disk_copy_form() else { return };
        let Some(run) = form.run.take() else { return };
        let ko = form.ko;
        let follow = match (run.stage, result) {
            (DiskStage::SameRead, Ok(())) => {
                form.status = t(ko, "원본을 읽었습니다. 원본 미디어를 꺼내고 같은 용량의 대상 미디어를 넣은 뒤 계속을 누르세요.",
                    "The source has been read. Eject it, insert destination media of the same capacity, then continue.").into();
                let text = format!("{}\n\n{}", form.status, t(ko, "대상 미디어의 모든 데이터를 덮어씁니다.", "Every byte on the destination media will be overwritten."));
                form.run = Some(DiskRun { stage: DiskStage::Swap, ..run });
                Follow::Swap(text)
            }
            (_, Ok(())) => {
                form.status = form.success_text(&run.plan).into();
                discard_temp(&run);
                Follow::Close
            }
            (DiskStage::Create | DiskStage::SameRead, Err(_)) if run.cancel.load(Ordering::Relaxed) => {
                form.status = t(ko, "이미지 만들기를 취소했습니다. 불완전한 이미지 파일은 삭제했습니다.", "Image creation cancelled. The incomplete image file was deleted.").into();
                discard_temp(&run);
                Follow::None
            }
            (DiskStage::SameWrite, Err(error)) => {
                // 복구용 이미지를 남기고 복원 방식으로 바꿔 둔다.
                let temp = run.temp.clone().unwrap_or_default();
                form.image_path = temp.to_string_lossy().into_owned();
                form.mode = DiskMode::Restore;
                let kept = t(ko, "복구용 이미지 파일을 보존했습니다", "The recovery image was kept at");
                Follow::Error { restoring: run.restoring(), message: format!("{error}\n\n{kept}: {}", temp.display()) }
            }
            (_, Err(error)) => {
                discard_temp(&run);
                Follow::Error { restoring: run.restoring(), message: error }
            }
        };
        match follow {
            Follow::None => {}
            Follow::Close => self.close_disk_copy(ctx),
            Follow::Swap(text) => self.system_box(&title, &text, MB_YESNO | MB_ICONEXCLAMATION, |app, answer, _| app.disk_swap_answered(answer == IDYES)),
            Follow::Error { restoring, message } => {
                let prefix = if restoring {
                    t(ko, "복원 또는 드라이브 복사 중 오류가 발생했습니다. 대상 디스크의 데이터가 일부 변경되었을 수 있습니다.\n\n",
                        "An error occurred while restoring or copying the drive. Data on the target disk may have been partly changed.\n\n")
                } else {
                    t(ko, "이미지를 만들지 못했습니다.\n\n", "The image could not be created.\n\n")
                };
                self.system_box(&title, &format!("{prefix}{message}"), MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
        }
    }

    /// 미디어 교체 상자: 예면 같은 디스크 번호에 임시 이미지를 쓰고(취소 없음), 아니요면 아무것도 쓰지 않는다.
    fn disk_swap_answered(&mut self, yes: bool) {
        let hooks = self.disk;
        let Some(form) = self.disk_copy_form() else { return };
        let Some(run) = form.run.take() else { return };
        let ko = form.ko;
        if !yes {
            form.status = t(ko, "복사를 취소했습니다. 대상 미디어에는 기록하지 않았습니다.", "Copy cancelled. Nothing was written to destination media.").into();
            discard_temp(&run);
            return;
        }
        form.status = t(ko, "대상 미디어에 이미지를 기록하고 있습니다…", "Writing the image to destination media…").into();
        let image = run.temp.clone().unwrap_or_default();
        let device = run.plan.physical.map(|d| d.device()).unwrap_or_default();
        let verify = run.plan.verify;
        let (receiver, cancel) = spawn(move |cancel, progress| (hooks.write_image)(&image, &device, verify, cancel, progress));
        form.run = Some(DiskRun { receiver, cancel, stage: DiskStage::SameWrite, plan: run.plan, temp: run.temp });
    }
}
