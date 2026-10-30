//! Guarded Windows removable-volume and removable-physical-disk imaging.
//! Device writes require UI confirmation before calling this module. Every call
//! revalidates removability, protected disks, extents, image location and size.
use crate::fsops::{check_cancel, commit_file, ensure_plain_path, unique_path};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::AtomicBool,
};
/// C# DiskCopyWindow.DriveChoice: 준비된 이동식 드라이브(DriveInfo: Removable·IsReady, VolumeLabel, TotalSize).
/// label은 볼륨 이름 그대로(비어 있으면 화면이 "이동식 디스크"로 보인다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovableDrive {
    pub root: String,
    pub label: String,
    pub bytes: u64,
}
/// C# PhysicalDiskChoice(Number, Bytes): 이동식·핫플러그이고 Windows·NDIR이 없는 물리 디스크.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalDisk {
    pub number: u32,
    pub bytes: u64,
}
impl PhysicalDisk {
    /// 이 모듈의 장치 이름(\\.\PhysicalDriveN).
    pub fn device(&self) -> String {
        format!("\\\\.\\PhysicalDrive{}", self.number)
    }
}
/// 진행 알림(복사한 바이트, 전체 바이트, 읽기 검증 단계인가 = C# DiskCopyProgress.Verifying).
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64, bool);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceId {
    Volume(u8),
    Physical(u32),
}
fn parse_device(text: &str) -> Result<DeviceId, String> {
    let b = text.as_bytes();
    if b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        return Ok(DeviceId::Volume(b[0].to_ascii_uppercase()));
    }
    if let Some(n) = text.strip_prefix("\\\\.\\PhysicalDrive") {
        if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n.len() < 4 {
            let number = n.parse::<u32>().map_err(|e| e.to_string())?;
            if number < 256 && number.to_string() == n {
                return Ok(DeviceId::Physical(number));
            }
        }
    }
    Err("Specify an exact removable root (E:\\) or \\\\.\\PhysicalDriveN (0..255)".into())
}
fn parse_extents(bytes: &[u8]) -> Result<Vec<u32>, String> {
    if bytes.len() < 8 {
        return Err("Truncated disk extents".into());
    }
    let count =
        u32::from_le_bytes(bytes[..4].try_into().map_err(|_| "Invalid extent count")?) as usize;
    if count == 0 || count > (bytes.len() - 8) / 24 {
        return Err("Invalid disk extent count".into());
    }
    let mut disks = BTreeSet::new();
    for i in 0..count {
        disks.insert(u32::from_le_bytes(
            bytes[8 + i * 24..12 + i * 24]
                .try_into()
                .map_err(|_| "Invalid disk extent")?,
        ));
    }
    Ok(disks.into_iter().collect())
}
fn copy_exact(
    input: &mut dyn Read,
    output: &mut dyn Write,
    length: u64,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<(), String> {
    let mut buffer = vec![0; 1024 * 1024];
    let mut copied = 0;
    let mut reported = 0;
    while copied < length {
        check_cancel(cancel)?;
        let amount = (length - copied).min(buffer.len() as u64) as usize;
        let n = input
            .read(&mut buffer[..amount])
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err(format!("Input ended after {copied} of {length} bytes"));
        }
        output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        copied += n as u64;
        if copied == length || copied - reported >= 32 * 1024 * 1024 {
            progress(copied, length, false);
            reported = copied;
        }
    }
    Ok(())
}
fn verify_exact(
    a: &mut dyn Read,
    b: &mut dyn Read,
    length: u64,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<(), String> {
    let mut left = vec![0; 1024 * 1024];
    let mut right = vec![0; 1024 * 1024];
    let mut compared = 0;
    while compared < length {
        check_cancel(cancel)?;
        let size = (length - compared).min(left.len() as u64) as usize;
        a.read_exact(&mut left[..size]).map_err(|e| e.to_string())?;
        b.read_exact(&mut right[..size])
            .map_err(|e| e.to_string())?;
        if left[..size] != right[..size] {
            let offset = left[..size]
                .iter()
                .zip(&right[..size])
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            return Err(format!(
                "Verification differs at byte {}",
                compared + offset as u64
            ));
        }
        compared += size as u64;
        if compared == length || compared % (32 * 1024 * 1024) == 0 {
            progress(compared, length, true);
        }
    }
    Ok(())
}
#[cfg(windows)]
mod win {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    };
    type Handle = *mut c_void;
    use windows_sys::Win32::Storage::FileSystem::IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS as GET_EXTENTS;
    use windows_sys::Win32::System::Ioctl::{
        FSCTL_DISMOUNT_VOLUME as DISMOUNT, FSCTL_LOCK_VOLUME as LOCK,
        FSCTL_UNLOCK_VOLUME as UNLOCK, IOCTL_DISK_GET_LENGTH_INFO as GET_LENGTH,
        IOCTL_DISK_UPDATE_PROPERTIES as UPDATE, IOCTL_STORAGE_GET_HOTPLUG_INFO as GET_HOTPLUG,
    };
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            device: Handle,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetLogicalDrives() -> u32;
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
        fn GetVolumePathNameW(path: *const u16, buffer: *mut u16, size: u32) -> i32;
        fn GetVolumeNameForVolumeMountPointW(path: *const u16, buffer: *mut u16, size: u32) -> i32;
        fn FindFirstVolumeW(buffer: *mut u16, size: u32) -> Handle;
        fn FindNextVolumeW(handle: Handle, buffer: *mut u16, size: u32) -> i32;
        fn FindVolumeClose(handle: Handle) -> i32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_size: u32,
            serial: *mut u32,
            component: *mut u32,
            flags: *mut u32,
            file_system: *mut u16,
            file_system_size: u32,
        ) -> i32;
        fn GetDiskFreeSpaceExW(
            root: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(Some(0)).collect()
    }
    fn from_wide(value: &[u16]) -> String {
        String::from_utf16_lossy(
            &value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())],
        )
    }
    fn error(context: &str) -> String {
        format!("{context}: {}", std::io::Error::last_os_error())
    }
    fn ioctl(file: &File, code: u32, output: &mut [u8]) -> Result<usize, String> {
        let mut returned = 0;
        let pointer = if output.is_empty() {
            std::ptr::null_mut()
        } else {
            output.as_mut_ptr().cast()
        };
        if unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                std::ptr::null(),
                0,
                pointer,
                output.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error(&format!("Device control {code:#x} failed")));
        }
        if returned as usize > output.len() {
            return Err("Invalid device control response length".into());
        }
        Ok(returned as usize)
    }
    fn open(path: &str, write: bool) -> Result<File, String> {
        OpenOptions::new()
            .read(true)
            .write(write)
            .share_mode(3)
            .open(path.trim_end_matches('\\'))
            .map_err(|e| format!("Cannot open device {path}: {e}"))
    }
    fn root(id: DeviceId) -> String {
        match id {
            DeviceId::Volume(letter) => format!("{}:\\", letter as char),
            DeviceId::Physical(n) => format!("\\\\.\\PhysicalDrive{n}"),
        }
    }
    fn device_path(id: DeviceId) -> String {
        match id {
            DeviceId::Volume(letter) => format!("\\\\.\\{}:", letter as char),
            DeviceId::Physical(n) => format!("\\\\.\\PhysicalDrive{n}"),
        }
    }
    fn disks(file: &File) -> Result<Vec<u32>, String> {
        let mut bytes = vec![0; 64 * 1024];
        let count = ioctl(file, GET_EXTENTS, &mut bytes)?;
        parse_extents(&bytes[..count])
    }
    fn length(file: &File) -> Result<u64, String> {
        let mut bytes = [0; 8];
        if ioctl(file, GET_LENGTH, &mut bytes)? != 8 {
            return Err("Invalid device length response".into());
        }
        let length = i64::from_le_bytes(bytes);
        if length <= 0 {
            return Err("Device length is not positive".into());
        }
        Ok(length as u64)
    }
    fn volume_for_path(path: &Path) -> Result<String, String> {
        let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
        let directory = if absolute.is_dir() {
            absolute.as_path()
        } else {
            absolute.parent().ok_or("Image directory is missing")?
        };
        if !directory.is_dir() {
            return Err("Image directory does not exist".into());
        }
        let mut mount = vec![0; 1024];
        let input = wide(directory.as_os_str());
        if unsafe { GetVolumePathNameW(input.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) }
            == 0
        {
            return Err(error("Cannot determine path volume"));
        }
        let mut volume = vec![0; 1024];
        if unsafe {
            GetVolumeNameForVolumeMountPointW(
                mount.as_ptr(),
                volume.as_mut_ptr(),
                volume.len() as u32,
            )
        } == 0
        {
            return Err(error("Only verifiable local image volumes are supported"));
        }
        Ok(from_wide(&volume))
    }
    fn protected_disks() -> Result<BTreeSet<u32>, String> {
        let mut system = vec![0; 32768];
        let n = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) };
        if n == 0 || n as usize >= system.len() {
            return Err(error("Cannot determine Windows system directory"));
        }
        let paths = [
            std::path::PathBuf::from(from_wide(&system)),
            std::env::current_exe().map_err(|e| e.to_string())?,
        ];
        let mut result = BTreeSet::new();
        for path in paths {
            let volume = volume_for_path(&path)?;
            result.extend(disks(&open(&volume, false)?)?);
        }
        if result.is_empty() {
            return Err("System and application disks could not be protected".into());
        }
        Ok(result)
    }
    struct VolumeSearch(Handle);
    impl Drop for VolumeSearch {
        fn drop(&mut self) {
            unsafe {
                FindVolumeClose(self.0);
            }
        }
    }
    fn volume_names() -> Result<Vec<String>, String> {
        let mut buffer = vec![0; 1024];
        let search = unsafe { FindFirstVolumeW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if search.is_null() || search as isize == -1 {
            return Err(error("Cannot enumerate every mounted volume"));
        }
        let search = VolumeSearch(search);
        let mut names = vec![from_wide(&buffer)];
        loop {
            buffer.fill(0);
            if unsafe { FindNextVolumeW(search.0, buffer.as_mut_ptr(), buffer.len() as u32) } == 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() == Some(18) {
                    break;
                }
                return Err(format!("Volume enumeration incomplete: {e}"));
            }
            names.push(from_wide(&buffer));
        }
        names.sort();
        Ok(names)
    }
    /// IOCTL_STORAGE_GET_HOTPLUG_INFO: MediaRemovable 또는 DeviceHotplug.
    fn hotplug(file: &File) -> Result<bool, String> {
        let mut info = [0; 8];
        info[..4].copy_from_slice(&8u32.to_le_bytes());
        let nread = ioctl(file, GET_HOTPLUG, &mut info)?;
        Ok(nread >= 8 && (info[4] != 0 || info[6] != 0))
    }
    struct Validated {
        id: DeviceId,
        file: File,
        bytes: u64,
        disks: Vec<u32>,
    }
    fn validate(id: DeviceId, write: bool) -> Result<Validated, String> {
        if let DeviceId::Volume(_) = id {
            let w = wide(root(id));
            if unsafe { GetDriveTypeW(w.as_ptr()) } != 2 {
                return Err("Only ready removable drive volumes are permitted".into());
            }
        }
        let file = open(&device_path(id), write)?;
        let ids = match id {
            DeviceId::Volume(_) => disks(&file)?,
            DeviceId::Physical(n) => {
                if !hotplug(&file)? {
                    return Err("Physical disk is neither removable-media nor hot-pluggable".into());
                }
                vec![n]
            }
        };
        if ids.len() != 1 {
            return Err("Spanned and multi-device volumes are not permitted".into());
        }
        let protected = protected_disks()?;
        if ids.iter().any(|n| protected.contains(n)) {
            return Err(
                "The selected device contains Windows or the running NDIR application".into(),
            );
        }
        let bytes = length(&file)?;
        Ok(Validated {
            id,
            file,
            bytes,
            disks: ids,
        })
    }
    struct Locked(File);
    impl Drop for Locked {
        fn drop(&mut self) {
            let _ = ioctl(&self.0, UNLOCK, &mut []);
        }
    }
    fn lock(file: File) -> Result<Locked, String> {
        ioctl(&file, LOCK, &mut [])?;
        let locked = Locked(file);
        ioctl(&locked.0, DISMOUNT, &mut [])?;
        Ok(locked)
    }
    fn lock_devices(devices: &[&Validated]) -> Result<Vec<Locked>, String> {
        let mut locked = Vec::new();
        let physical: BTreeSet<_> = devices
            .iter()
            .filter(|d| matches!(d.id, DeviceId::Physical(_)))
            .flat_map(|d| d.disks.iter().copied())
            .collect();
        let mut volumes = BTreeSet::new();
        if !physical.is_empty() {
            for name in volume_names()? {
                let file = open(&name, false)?;
                let ids = disks(&file)?;
                if ids.iter().any(|n| physical.contains(n)) {
                    if ids.len() != 1 {
                        return Err("Selected physical disk shares a spanned volume".into());
                    }
                    volumes.insert(name);
                }
            }
        }
        for device in devices {
            if matches!(device.id, DeviceId::Volume(_)) {
                volumes.insert(device_path(device.id));
            }
        }
        for name in volumes {
            let file = open(&name, true)?;
            locked.push(lock(file)?);
        }
        Ok(locked)
    }
    fn image_on_other_disk(path: &Path, device: &Validated) -> Result<(), String> {
        ensure_plain_path(path)?;
        let volume = volume_for_path(path)?;
        let image_disks = disks(&open(&volume, false)?)?;
        if image_disks.iter().any(|n| device.disks.contains(n)) {
            return Err(
                "Image file must be on a different physical disk from the selected device".into(),
            );
        }
        Ok(())
    }
    /// C# DiskCopyWindow.GetReadyRemovableDrives: DriveType Removable이고 준비된(볼륨 정보를 읽는) 드라이브,
    /// 이름과 전체 크기(TotalSize = GetDiskFreeSpaceExW)를 읽지 못하면 뺀다. 장치는 열지 않는다.
    pub(super) fn ready_drives() -> Vec<RemovableDrive> {
        let letters = unsafe { GetLogicalDrives() };
        let mut result = Vec::new();
        for n in 0..26u8 {
            if letters & (1 << n) == 0 {
                continue;
            }
            let root = format!("{}:\\", (b'A' + n) as char);
            let w = wide(&root);
            if unsafe { GetDriveTypeW(w.as_ptr()) } != 2 {
                continue;
            }
            let mut name = vec![0u16; 261];
            let ready = unsafe {
                GetVolumeInformationW(
                    w.as_ptr(),
                    name.as_mut_ptr(),
                    name.len() as u32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            } != 0;
            if !ready {
                continue;
            }
            let mut total = 0u64;
            if unsafe {
                GetDiskFreeSpaceExW(
                    w.as_ptr(),
                    std::ptr::null_mut(),
                    &mut total,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                continue;
            }
            result.push(RemovableDrive {
                root,
                label: from_wide(&name),
                bytes: total,
            });
        }
        result
    }
    /// C# DiskImageService.GetReadyRemovablePhysicalDisks: 0..63 중 열리고, 이동식·핫플러그이고,
    /// 보호 디스크(Windows·NDIR)가 아니고, 길이가 양수인 디스크. 보호 디스크를 못 구하면 빈 목록.
    pub(super) fn ready_physical() -> Vec<PhysicalDisk> {
        let Ok(protected) = protected_disks() else {
            return Vec::new();
        };
        (0..64u32)
            .filter_map(|number| {
                let file = open(&device_path(DeviceId::Physical(number)), false).ok()?;
                if !hotplug(&file).ok()? || protected.contains(&number) {
                    return None;
                }
                let bytes = length(&file).ok()?;
                Some(PhysicalDisk { number, bytes })
            })
            .collect()
    }
    pub(super) fn read(
        device: &str,
        output: &Path,
        cancel: &AtomicBool,
        progress: Progress,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let mut validated = validate(parse_device(device)?, false)?;
        image_on_other_disk(output, &validated)?;
        if output.is_dir() {
            return Err("Image output is a directory".into());
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let stage = unique_path(parent, "disk-image");
        let result = (|| {
            let locks = lock_devices(&[&validated])?;
            if length(&validated.file)? != validated.bytes {
                return Err("Device length changed before imaging".into());
            }
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)
                .map_err(|e| e.to_string())?;
            copy_exact(
                &mut validated.file,
                &mut out,
                validated.bytes,
                cancel,
                progress,
            )?;
            out.sync_all().map_err(|e| e.to_string())?;
            drop(out);
            drop(locks);
            check_cancel(cancel)?;
            commit_file(&stage, output, true)
        })();
        let _ = fs::remove_file(stage);
        result
    }
    pub(super) fn write(
        image: &Path,
        device: &str,
        verify: bool,
        cancel: &AtomicBool,
        progress: Progress,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        ensure_plain_path(image)?;
        let mut input = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(image)
            .map_err(|e| e.to_string())?;
        let metadata = input.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Image source must be an ordinary file".into());
        }
        let mut validated = validate(parse_device(device)?, true)?;
        image_on_other_disk(image, &validated)?;
        if metadata.len() != validated.bytes {
            return Err(format!(
                "Image length {} must exactly equal device length {}",
                metadata.len(),
                validated.bytes
            ));
        }
        let _locks = lock_devices(&[&validated])?;
        if length(&validated.file)? != validated.bytes {
            return Err("Device length changed before restore".into());
        }
        copy_exact(
            &mut input,
            &mut validated.file,
            validated.bytes,
            cancel,
            progress,
        )
        .map_err(|e| format!("{e}; the raw target may have been partially written"))?;
        validated.file.sync_all().map_err(|e| e.to_string())?;
        // C# RestoreImage/RestoreImageToPhysicalDisk: 읽기 검증은 고른 경우만(한 드라이브 복사의 확인란).
        if verify {
            input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            validated
                .file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            verify_exact(
                &mut input,
                &mut validated.file,
                validated.bytes,
                cancel,
                progress,
            )?;
        }
        if matches!(validated.id, DeviceId::Physical(_)) {
            ioctl(&validated.file, UPDATE, &mut []).map_err(|e| {
                format!("Data written; disk properties refresh failed: {e}")
            })?;
        }
        Ok(())
    }
    pub(super) fn copy(
        source: &str,
        destination: &str,
        verify: bool,
        cancel: &AtomicBool,
        progress: Progress,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let source_id = parse_device(source)?;
        let dest_id = parse_device(destination)?;
        if source_id == dest_id {
            return Err("Source and destination devices are identical".into());
        }
        let mut a = validate(source_id, false)?;
        let mut b = validate(dest_id, true)?;
        if a.disks.iter().any(|id| b.disks.contains(id)) {
            return Err("Source and destination overlap the same physical disk".into());
        }
        if a.bytes != b.bytes {
            return Err("Source and destination device lengths must be exactly equal".into());
        }
        let _locks = lock_devices(&[&a, &b])?;
        if length(&a.file)? != a.bytes || length(&b.file)? != b.bytes {
            return Err("Device length changed before cloning".into());
        }
        copy_exact(&mut a.file, &mut b.file, a.bytes, cancel, progress)
            .map_err(|e| format!("{e}; the raw target may have been partially written"))?;
        b.file.sync_all().map_err(|e| e.to_string())?;
        if verify {
            a.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            b.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            verify_exact(&mut a.file, &mut b.file, a.bytes, cancel, progress)?;
        }
        if matches!(b.id, DeviceId::Physical(_)) {
            ioctl(&b.file, UPDATE, &mut [])
                .map_err(|e| format!("Data written; disk properties refresh failed: {e}"))?;
        }
        Ok(())
    }
}
/// C# DiskCopyWindow.GetReadyRemovableDrives.
pub fn ready_removable_drives() -> Vec<RemovableDrive> {
    #[cfg(windows)]
    {
        win::ready_drives()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
/// C# DiskImageService.GetReadyRemovablePhysicalDisks(장치를 여는 데 관리자 권한이 필요할 수 있다).
pub fn ready_removable_physical_disks() -> Vec<PhysicalDisk> {
    #[cfg(windows)]
    {
        win::ready_physical()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
pub fn read_image(
    device: &str,
    output: &Path,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::read(device, output, cancel, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (device, output, cancel, progress);
        Err("Disk imaging requires Windows".into())
    }
}
/// 이미지를 장치에 쓴다. verify면 쓴 뒤 처음부터 다시 읽어 비교한다(C# verify 인자).
pub fn write_image(
    image: &Path,
    device: &str,
    verify: bool,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::write(image, device, verify, cancel, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (image, device, verify, cancel, progress);
        Err("Disk imaging requires Windows".into())
    }
}
pub fn copy_device(
    source: &str,
    destination: &str,
    verify: bool,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::copy(source, destination, verify, cancel, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (source, destination, verify, cancel, progress);
        Err("Disk imaging requires Windows".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn exact_device_ids_reject_ambiguous_and_injected_paths() {
        assert_eq!(parse_device("E:\\").unwrap(), DeviceId::Volume(b'E'));
        assert_eq!(
            parse_device("\\\\.\\PhysicalDrive3").unwrap(),
            DeviceId::Physical(3)
        );
        for value in [
            "C:",
            "C:\\folder",
            "\\\\.\\C:",
            "\\\\?\\GLOBALROOT\\Device\\Harddisk0",
            "\\\\.\\PhysicalDrive-1",
            "\\\\.\\PhysicalDrive3junk",
            "\\\\server\\share",
            "E:\\..\\C:\\",
        ] {
            assert!(parse_device(value).is_err(), "accepted {value}");
        }
    }
    #[test]
    fn malformed_or_truncated_extents_never_authorize_a_device() {
        let mut bytes = vec![0; 32];
        bytes[..4].copy_from_slice(&1u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(parse_extents(&bytes).unwrap(), vec![7]);
        assert!(parse_extents(&bytes[..31]).is_err());
        bytes[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_extents(&bytes).is_err());
    }
    #[test]
    fn exact_copy_detects_truncation_and_cancellation() {
        let mut output = Vec::new();
        let cancel = AtomicBool::new(false);
        copy_exact(
            &mut Cursor::new(b"image"),
            &mut output,
            5,
            &cancel,
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(output, b"image");
        assert!(
            copy_exact(
                &mut Cursor::new(b"short"),
                &mut Vec::new(),
                6,
                &cancel,
                &mut |_, _, _| {}
            )
            .is_err()
        );
        assert!(
            copy_exact(
                &mut Cursor::new(b"x"),
                &mut Vec::new(),
                1,
                &AtomicBool::new(true),
                &mut |_, _, _| {}
            )
            .is_err()
        );
    }
    #[test]
    fn progress_marks_the_verification_pass_like_csharp_disk_copy_progress() {
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        copy_exact(
            &mut Cursor::new(b"abc"),
            &mut Vec::new(),
            3,
            &cancel,
            &mut |done, total, verifying| seen.push((done, total, verifying)),
        )
        .unwrap();
        verify_exact(
            &mut Cursor::new(b"abc"),
            &mut Cursor::new(b"abc"),
            3,
            &cancel,
            &mut |done, total, verifying| seen.push((done, total, verifying)),
        )
        .unwrap();
        assert_eq!(seen, [(3, 3, false), (3, 3, true)]);
        let error = verify_exact(
            &mut Cursor::new(b"abc"),
            &mut Cursor::new(b"abd"),
            3,
            &cancel,
            &mut |_, _, _| {},
        )
        .unwrap_err();
        assert_eq!(error, "Verification differs at byte 2");
        assert_eq!(
            PhysicalDisk { number: 7, bytes: 1 }.device(),
            "\\\\.\\PhysicalDrive7"
        );
        assert!(parse_device(&PhysicalDisk { number: 7, bytes: 1 }.device()).is_ok());
    }
}
