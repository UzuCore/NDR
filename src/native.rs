//! Windows shell integration. All buffers passed to Win32 live through the call.
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    ffi::OsStr,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::Cryptography::*,
    Storage::FileSystem::*,
    System::{DataExchange::*, Memory::*},
    UI::{Shell::*, WindowsAndMessaging::*},
};

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

/// 원본 TNXShellChangeNotify 스레드(ResetThread 0x7b37b8 → FUN_007b3060): 폴더 하나(하위 폴더 아님, SetSubFolder 처음 값 0)를
/// 거르개 0x0B(파일 이름·폴더 이름·크기 바뀜, 생성자 0x7b3304)로 지켜보다가 바뀌면 `on_change`를 부른다. `stop`이 켜지면 끝낸다.
/// 감시를 시작하지 못하면(없는 폴더 등) 아무것도 하지 않는다. 핸들은 부른 스레드에서 만들므로 돌아온 뒤의 바뀜은 놓치지 않는다.
pub fn spawn_folder_watch(path: PathBuf, stop: std::sync::Arc<std::sync::atomic::AtomicBool>, on_change: impl Fn() + Send + 'static) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    let path = wide(path.as_os_str());
    // SAFETY: the path buffer lives through the call; the returned change handle is moved to the watch thread and closed there.
    let handle = unsafe { FindFirstChangeNotificationW(path.as_ptr(), 0, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE) };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return;
    }
    let raw = handle as usize;
    std::thread::spawn(move || {
        let handle = raw as windows_sys::Win32::Foundation::HANDLE;
        while !stop.load(Ordering::Relaxed) {
            // SAFETY: handle is a live change-notification handle.
            match unsafe { WaitForSingleObject(handle, 250) } {
                WAIT_OBJECT_0 => {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    on_change();
                    // SAFETY: as above.
                    if unsafe { FindNextChangeNotification(handle) } == 0 {
                        break;
                    }
                }
                WAIT_TIMEOUT => {}
                _ => break,
            }
        }
        // SAFETY: handle came from FindFirstChangeNotificationW and is closed once.
        unsafe { FindCloseChangeNotification(handle) };
    });
}
pub fn reject_nul(text: &str) -> Result<(), String> {
    if text.contains('\0') {
        Err("Embedded NUL is not allowed".into())
    } else {
        Ok(())
    }
}

/// Windows CommandLineToArgvW quoting, including backslashes preceding quotes/end.
pub fn quote_arg(arg: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            result.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            result.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        result.push(c);
    }
    result.push_str(&"\\".repeat(slashes * 2));
    result.push('"');
    result
}

pub fn protect_secret(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: text.len().try_into().map_err(|_| "Secret too long")?,
        pbData: text.as_ptr().cast_mut(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let success = unsafe {
        CryptProtectData(
            &mut input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let encoded = STANDARD
        .encode(unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) });
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(encoded)
}
pub fn unprotect_secret(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let mut data = STANDARD.decode(text).map_err(|e| e.to_string())?;
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into().map_err(|_| "Secret too long")?,
        pbData: data.as_mut_ptr(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let success = unsafe {
        CryptUnprotectData(
            &mut input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let result = String::from_utf8(
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec(),
    )
    .map_err(|e| e.to_string());
    unsafe {
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData.cast());
    }
    data.fill(0);
    result
}

pub fn shell_open(path: &Path, args: Option<&str>, verb: &str) -> Result<(), String> {
    reject_nul(&path.to_string_lossy())?;
    reject_nul(verb)?;
    reject_nul(args.unwrap_or(""))?;
    let filename = wide(path.as_os_str());
    let verb = wide(verb);
    let arguments = args.map(wide);
    let directory = path.parent().map(|p| wide(p.as_os_str()));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_INVOKEIDLIST;
    info.lpVerb = verb.as_ptr();
    info.lpFile = filename.as_ptr();
    info.lpParameters = arguments.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    info.lpDirectory = directory.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    info.nShow = SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

/// xilesShell_ExecuteFile(원본 FUN_00431dd4가 부름): ShellExecuteExW(파일, 인수, 작업 폴더, SW_SHOWNORMAL), 동사 NULL(기본 동사).
/// SEE_MASK_FLAG_NO_UI가 없으므로 실패는 셸이 직접 알린다.
pub fn shell_execute(path: &Path, parameters: &str, directory: &Path) -> Result<(), String> {
    reject_nul(&path.to_string_lossy())?;
    reject_nul(parameters)?;
    let filename = wide(path.as_os_str());
    let parameters = wide(parameters);
    let directory = (!directory.as_os_str().is_empty() && directory.is_dir()).then(|| wide(directory.as_os_str()));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.lpFile = filename.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.lpDirectory = directory.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    info.nShow = SW_SHOWNORMAL;
    // SAFETY: every string pointer refers to a NUL-terminated buffer that outlives the call.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

/// C# PlayOperationCompletionSound: SystemSounds.Asterisk = MessageBeep(MB_ICONASTERISK).
pub fn completion_sound() {
    // SAFETY: plain system call without pointers.
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONASTERISK);
    }
}

/// 원본 PlaySoundW(경로, 0, 0x22003 = SND_FILENAME|SND_NOWAIT|SND_NODEFAULT|SND_ASYNC). None이면 원본 PlaySoundW(NULL, …)처럼 지금 소리를 멈춘다.
/// 파일 소리를 낼 수 없으면 false.
pub fn play_wave(path: Option<&Path>) -> bool {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT, SND_NOWAIT};
    let flags = SND_FILENAME | SND_NOWAIT | SND_NODEFAULT | SND_ASYNC;
    let Some(path) = path else {
        // SAFETY: a null sound name only stops the current sound.
        return unsafe { PlaySoundW(ptr::null(), ptr::null_mut(), flags) } != 0;
    };
    let wide = wide(path.as_os_str());
    // SAFETY: wide is NUL-terminated and outlives the call; SND_ASYNC copies the name before returning.
    unsafe { PlaySoundW(wide.as_ptr(), ptr::null_mut(), flags) != 0 }
}

/// Delphi Mouse.WheelScrollLines(SPI_GETWHEELSCROLLLINES). 읽지 못하면 Windows 처음 값 3.
pub fn wheel_scroll_lines() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETWHEELSCROLLLINES, SystemParametersInfoW};
    let mut lines: u32 = 3;
    // SAFETY: SPI_GETWHEELSCROLLLINES writes one UINT into the supplied buffer.
    let ok = unsafe { SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, (&mut lines as *mut u32).cast(), 0) };
    if ok == 0 { 3 } else { lines }
}

struct Clipboard;
impl Clipboard {
    fn open() -> Result<Self, String> {
        if unsafe { OpenClipboard(ptr::null_mut()) } == 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(Self)
        }
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
unsafe fn put_clipboard(format: u32, bytes: &[u8]) -> Result<(), String> {
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
    if memory.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let target = unsafe { GlobalLock(memory) };
    if target.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return Err(std::io::Error::last_os_error().to_string());
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        GlobalUnlock(memory);
    }
    if unsafe { SetClipboardData(format, memory) }.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
pub fn clipboard_text(text: &str) -> Result<(), String> {
    reject_nul(text)?;
    let data = wide(text);
    let _clipboard = Clipboard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    unsafe {
        put_clipboard(
            13,
            std::slice::from_raw_parts(data.as_ptr().cast(), data.len() * 2),
        )
    }
}
pub fn clipboard_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    let mut names = Vec::<u16>::new();
    for path in paths {
        reject_nul(&path.to_string_lossy())?;
        names.extend(wide(path));
    }
    names.push(0);
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: 0,
        fWide: 1,
    };
    let mut data = unsafe {
        std::slice::from_raw_parts(
            (&header as *const DROPFILES).cast::<u8>(),
            size_of::<DROPFILES>(),
        )
    }
    .to_vec();
    data.extend_from_slice(unsafe {
        std::slice::from_raw_parts(names.as_ptr().cast::<u8>(), names.len() * 2)
    });
    let effect = if cut { 2u32 } else { 1u32 };
    let registered = wide("Preferred DropEffect");
    let _clipboard = Clipboard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    unsafe {
        put_clipboard(15, &data)?;
        let format = RegisterClipboardFormatW(registered.as_ptr());
        if format == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        put_clipboard(format, &effect.to_ne_bytes())
    }
}
pub fn read_clipboard_files() -> Result<(Vec<PathBuf>, bool), String> {
    let _clipboard = Clipboard::open()?;
    let handle = unsafe { GetClipboardData(15) };
    if handle.is_null() {
        return Ok((vec![], false));
    }
    let count = unsafe { DragQueryFileW(handle, 0xffffffff, ptr::null_mut(), 0) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let length = unsafe { DragQueryFileW(handle, index, ptr::null_mut(), 0) };
        let mut buffer = vec![0u16; length as usize + 1];
        unsafe {
            DragQueryFileW(handle, index, buffer.as_mut_ptr(), buffer.len() as u32);
        }
        paths.push(PathBuf::from(String::from_utf16_lossy(
            &buffer[..length as usize],
        )));
    }
    let format = unsafe { RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr()) };
    let effect = unsafe { GetClipboardData(format) };
    let mut cut = false;
    if !effect.is_null() && unsafe { GlobalSize(effect) } >= 4 {
        let p = unsafe { GlobalLock(effect) };
        if !p.is_null() {
            cut = unsafe { ptr::read_unaligned(p.cast::<u32>()) } & 2 != 0;
            unsafe {
                GlobalUnlock(effect);
            }
        }
    }
    Ok((paths, cut))
}

pub fn drives() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26)
        .filter(|n| mask & (1 << n) != 0)
        .map(|n| PathBuf::from(format!("{}:\\", char::from(b'A' + n))))
        .collect()
}
/// C# DriveInfo.GetDrives().Where(IsReady): 볼륨 정보를 읽을 수 있는 드라이브만.
pub fn ready_drives() -> Vec<PathBuf> {
    drives().into_iter().filter(|root| volume_label(root).is_some()).collect()
}
/// 원본 TNXDriveList.Refresh(0x7b7df0): GetLogicalDrives의 A~Z 가운데 GetDriveTypeW가 0(알 수 없음)·1(루트 없음)이 아닌 것.
/// 미디어가 없는 CD·카드 리더도 들어간다.
pub fn listed_drives() -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    drives()
        .into_iter()
        // SAFETY: NUL-terminated root string.
        .filter(|root| unsafe { GetDriveTypeW(wide(root.as_os_str()).as_ptr()) } > 1)
        .collect()
}
/// 원본 FUN_004351cc: SetVolumeLabelW(루트, 새 이름). 실패하면 GetLastError.
pub fn set_volume_label(root: &Path, label: &str) -> Result<(), u32> {
    // SAFETY: both arguments are NUL-terminated UTF-16 buffers that outlive the call.
    if unsafe { SetVolumeLabelW(wide(root.as_os_str()).as_ptr(), wide(label).as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(unsafe { GetLastError() })
    }
}
pub fn attributes(path: &Path) -> Result<u32, String> {
    let value = unsafe { GetFileAttributesW(wide(path).as_ptr()) };
    if value == INVALID_FILE_ATTRIBUTES {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(value)
    }
}
pub fn set_attributes(path: &Path, read_only: bool, hidden: bool) -> Result<(), String> {
    let mut value = attributes(path)?;
    value = if read_only {
        value | FILE_ATTRIBUTE_READONLY
    } else {
        value & !FILE_ATTRIBUTE_READONLY
    };
    value = if hidden {
        value | FILE_ATTRIBUTE_HIDDEN
    } else {
        value & !FILE_ATTRIBUTE_HIDDEN
    };
    if unsafe { SetFileAttributesW(wide(path).as_ptr(), value) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

pub fn shortcut(source: &Path, output: &Path) -> Result<(), String> {
    use windows::{
        Win32::{
            System::Com::*,
            UI::Shell::{IShellLinkW, ShellLink},
        },
        core::{Interface, PCWSTR},
    };
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<()> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(PCWSTR(wide(source).as_ptr()))?;
            if let Some(parent) = source.parent() {
                link.SetWorkingDirectory(PCWSTR(wide(parent).as_ptr()))?;
            }
            let persist: IPersistFile = link.cast()?;
            persist.Save(PCWSTR(wide(output).as_ptr()), true)?;
            Ok(())
        })();
        if initialized {
            CoUninitialize();
        }
        result.map_err(|e| e.to_string())
    }
}

/// C# RecycleBinService.Empty: 모든 드라이브, 확인·진행 창·소리 없이(SHERB_NOCONFIRMATION | NOPROGRESSUI | NOSOUND).
pub fn empty_recycle_bin() -> Result<(), String> {
    // SAFETY: null owner and root mean every drive; no pointers are kept.
    let code = unsafe { SHEmptyRecycleBinW(ptr::null_mut(), ptr::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) };
    if code < 0 {
        Err(format!("Recycle bin error: 0x{code:08x}"))
    } else {
        Ok(())
    }
}

/// 원본 miEmptyRecycleBinClick(0x433094): SHEmptyRecycleBinW(창, NULL, 0) — 셸이 확인·진행 창을 띄운다.
pub fn empty_recycle_bin_with_shell_ui() -> Result<(), String> {
    // SAFETY: null owner and root mean every drive; no pointers are kept.
    let code = unsafe { SHEmptyRecycleBinW(ptr::null_mut(), ptr::null(), 0) };
    if code < 0 {
        Err(format!("Recycle bin error: 0x{code:08x}"))
    } else {
        Ok(())
    }
}

/// C# WNetAddConnection2(Type 1, LocalName, RemoteName, password, user, flags 0). 실패는 (Windows 오류 코드, 시스템 글).
pub fn wnet_add_connection(local: &str, remote: &str, user: Option<&str>, password: Option<&str>) -> Result<(), (u32, String)> {
    use windows_sys::Win32::NetworkManagement::WNet::*;
    if [local, remote, user.unwrap_or(""), password.unwrap_or("")].iter().any(|v| v.contains('\0')) {
        return Err((87, std::io::Error::from_raw_os_error(87).to_string()));
    }
    let (local, remote) = (wide(local), wide(remote));
    let (user, password) = (user.map(wide), password.map(wide));
    let mut resource: NETRESOURCEW = unsafe { std::mem::zeroed() };
    resource.dwType = RESOURCETYPE_DISK;
    resource.lpLocalName = local.as_ptr().cast_mut();
    resource.lpRemoteName = remote.as_ptr().cast_mut();
    // SAFETY: every string is NUL-terminated and outlives the call.
    let code = unsafe { WNetAddConnection2W(&resource, password.as_ref().map_or(ptr::null(), |p| p.as_ptr()), user.as_ref().map_or(ptr::null(), |u| u.as_ptr()), 0) };
    if code == 0 { Ok(()) } else { Err((code, std::io::Error::from_raw_os_error(code as i32).to_string())) }
}

/// C# WNetCancelConnection2(name, 0, false).
pub fn wnet_cancel_connection(name: &str) -> Result<(), (u32, String)> {
    use windows_sys::Win32::NetworkManagement::WNet::*;
    // SAFETY: NUL-terminated name.
    let code = unsafe { WNetCancelConnection2W(wide(name).as_ptr(), 0, 0) };
    if code == 0 { Ok(()) } else { Err((code, std::io::Error::from_raw_os_error(code as i32).to_string())) }
}

/// DriveInfo.GetDrives()에서 DriveType.Network인 것("Z:" 모양, 끝 '\' 뺌).
pub fn network_drives() -> Vec<String> {
    drives().into_iter().filter(|d| is_network_drive(d)).map(|d| d.to_string_lossy().trim_end_matches('\\').to_owned()).collect()
}

const EXPLORER_FOLDER_KEY: &str = "Software\\Classes\\Directory\\shell\\NDR";
const EXPLORER_BACKGROUND_KEY: &str = "Software\\Classes\\Directory\\Background\\shell\\NDR";
/// 예전 Rust 판이 쓰던 키(지울 때 같이 지운다).
const LEGACY_EXPLORER_KEYS: [&str; 2] = ["Software\\Classes\\Directory\\shell\\NDR.Rust", "Software\\Classes\\Directory\\Background\\shell\\NDR.Rust"];

fn registry_default_value(key: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::*;
    let mut size = 0u32;
    let subkey = wide(&format!("{key}\\command"));
    // SAFETY: first call asks for the size only.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, subkey.as_ptr(), ptr::null(), RRF_RT_REG_SZ, ptr::null_mut(), ptr::null_mut(), &mut size) };
    if status != 0 || size < 2 {
        return None;
    }
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `bytes` bytes.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, subkey.as_ptr(), ptr::null(), RRF_RT_REG_SZ, ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut bytes) };
    (status == 0).then(|| String::from_utf16_lossy(&buffer[..buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len())]))
}

/// HKEY_CLASSES_ROOT\`key`의 기본값(REG_SZ·REG_EXPAND_SZ, 펼치지 않음).
fn classes_default(key: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::*;
    let subkey = wide(key);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut size = 0u32;
    // SAFETY: first call asks for the size only.
    let status = unsafe { RegGetValueW(HKEY_CLASSES_ROOT, subkey.as_ptr(), ptr::null(), flags, ptr::null_mut(), ptr::null_mut(), &mut size) };
    if status != 0 || size < 2 {
        return None;
    }
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `bytes` bytes.
    let status = unsafe { RegGetValueW(HKEY_CLASSES_ROOT, subkey.as_ptr(), ptr::null(), flags, ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut bytes) };
    (status == 0).then(|| String::from_utf16_lossy(&buffer[..buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len())]))
}

/// HKEY_CLASSES_ROOT\`key`가 있는가.
fn classes_key_exists(key: &str) -> bool {
    use windows_sys::Win32::System::Registry::*;
    let subkey = wide(key);
    let mut handle = ptr::null_mut();
    // SAFETY: NUL-terminated key; the handle is closed below.
    if unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, subkey.as_ptr(), 0, KEY_READ, &mut handle) } != 0 {
        return false;
    }
    // SAFETY: handle came from RegOpenKeyExW.
    unsafe { RegCloseKey(handle) };
    true
}

/// 셸 아이콘이 파일마다 다른 형식인가: DefaultIcon이 "%1"(따옴표 포함)이거나 shellex\IconHandler가 있다.
pub fn per_file_icon_rule(default_icon: Option<&str>, icon_handler: bool) -> bool {
    icon_handler || default_icon.is_some_and(|icon| icon.trim().trim_matches('"') == "%1")
}

/// 원본 GetIconIndex(0x7b8d54)는 파일마다 실제 경로로 셸 아이콘을 묻는다. 결과가 파일마다 달라지는 형식만 골라 실제 경로로 읽기 위해,
/// HKCR\.확장자, 그 ProgID, SystemFileAssociations\.확장자 가운데 하나라도 per_file_icon_rule에 맞는지 본다.
pub fn extension_has_per_file_icon(extension: &str) -> bool {
    let extension = extension.trim().trim_start_matches('.');
    if extension.is_empty() || extension.contains(['\\', '/']) {
        return false;
    }
    let dotted = format!(".{extension}");
    let mut keys = vec![dotted.clone(), format!("SystemFileAssociations\\{dotted}")];
    if let Some(prog_id) = classes_default(&dotted).map(|p| p.trim().to_owned()).filter(|p| !p.is_empty() && !p.contains(['\\', '/'])) {
        keys.insert(0, prog_id);
    }
    keys.iter().any(|key| per_file_icon_rule(classes_default(&format!("{key}\\DefaultIcon")).as_deref(), classes_key_exists(&format!("{key}\\shellex\\IconHandler"))))
}

/// ExplorerIntegrationService.IsEnabled: 두 키 모두 command 기본값이 비어 있지 않다.
pub fn explorer_integration_enabled() -> bool {
    [EXPLORER_FOLDER_KEY, EXPLORER_BACKGROUND_KEY].iter().all(|key| registry_default_value(key).is_some_and(|v| !v.trim().is_empty()))
}

fn set_registry_string(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::*;
    let mut handle = ptr::null_mut();
    // SAFETY: NUL-terminated key; the handle is closed below.
    let status = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, wide(key).as_ptr(), 0, ptr::null(), 0, KEY_SET_VALUE, ptr::null(), &mut handle, ptr::null_mut()) };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32).to_string());
    }
    let data = wide(value);
    let name = name.map(wide);
    // SAFETY: data and name are NUL-terminated and alive.
    let status = unsafe { RegSetValueExW(handle, name.as_ref().map_or(ptr::null(), |n| n.as_ptr()), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
    // SAFETY: handle came from RegCreateKeyExW.
    unsafe { RegCloseKey(handle) };
    if status != 0 { Err(std::io::Error::from_raw_os_error(status as i32).to_string()) } else { Ok(()) }
}

/// ExplorerIntegrationService.Enable/Disable(HKCU): 폴더 "Browse with NDR" → "\"exe\" \"/dir:%1\"", 폴더 배경 "Open folder in NDR" →
/// "\"exe\" \"/dir:%V\"", 아이콘은 실행 파일. 끄면 두 키(와 예전 Rust 판 키)를 지운다.
pub fn set_explorer_integration(enable: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::*;
    if enable {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let quoted = format!("\"{}\"", executable.replace('"', "\\\""));
        for (key, verb, argument) in [(EXPLORER_FOLDER_KEY, "Browse with NDR", "%1"), (EXPLORER_BACKGROUND_KEY, "Open folder in NDR", "%V")] {
            set_registry_string(key, Some("MUIVerb"), verb)?;
            set_registry_string(key, Some("Icon"), &executable)?;
            set_registry_string(&format!("{key}\\command"), None, &format!("{quoted} \"/dir:{argument}\""))?;
        }
    } else {
        for key in [EXPLORER_FOLDER_KEY, EXPLORER_BACKGROUND_KEY].into_iter().chain(LEGACY_EXPLORER_KEYS) {
            // SAFETY: NUL-terminated key.
            let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(key).as_ptr()) };
            if status != 0 && status != 2 {
                return Err(std::io::Error::from_raw_os_error(status as i32).to_string());
            }
        }
    }
    // SAFETY: notification without pointers.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, ptr::null(), ptr::null()) };
    Ok(())
}
/// Shell thumbnails/icons; called on workers and copied before releasing GDI handles.
pub fn thumbnail(path: &Path, side: u32) -> Result<(usize, usize, Vec<u8>), String> {
    use windows::{
        Win32::{
            Foundation::SIZE,
            System::Com::*,
            UI::Shell::{
                IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
                SIIGBF_RESIZETOFIT,
            },
        },
        core::PCWSTR,
    };
    use windows_sys::Win32::Graphics::Gdi::*;
    reject_nul(&path.to_string_lossy())?;
    let side = side.clamp(16, 256) as i32;
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let image = (|| -> Result<_, String> {
            let factory: IShellItemImageFactory =
                SHCreateItemFromParsingName(PCWSTR(wide(path).as_ptr()), None)
                    .map_err(|e| e.to_string())?;
            factory
                .GetImage(
                    SIZE { cx: side, cy: side },
                    SIIGBF_BIGGERSIZEOK | SIIGBF_RESIZETOFIT,
                )
                .map_err(|e| e.to_string())
        })();
        if initialized {
            CoUninitialize();
        }
        let bitmap = image?;
        let handle = bitmap.0;
        let mut data: BITMAP = std::mem::zeroed();
        if GetObjectW(
            handle,
            size_of::<BITMAP>() as i32,
            (&mut data as *mut BITMAP).cast(),
        ) == 0
        {
            DeleteObject(handle);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let (width, height) = (
            data.bmWidth.unsigned_abs() as usize,
            data.bmHeight.unsigned_abs() as usize,
        );
        if width == 0 || height == 0 || width > 1024 || height > 1024 {
            DeleteObject(handle);
            return Err("Invalid thumbnail dimensions".into());
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width as i32;
        info.bmiHeader.biHeight = -(height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            DeleteObject(handle);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut pixels = vec![0u8; width * height * 4];
        let copied = GetDIBits(
            dc,
            handle,
            0,
            height as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );
        DeleteDC(dc);
        DeleteObject(handle);
        if copied == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            if !has_alpha {
                pixel[3] = 255;
            }
        }
        Ok((width, height, pixels))
    }
}

/// 원본 TfMsgBox의 Image1처럼 LoadIconW(NULL, id)로 시스템 아이콘을 읽어 32x32 RGBA로 돌려준다.
/// id는 IDI_HAND 0x7f01, IDI_QUESTION 0x7f02, IDI_EXCLAMATION 0x7f03, IDI_ASTERISK 0x7f04.
pub fn system_icon_rgba(id: u16) -> Result<(usize, usize, Vec<u8>), String> {
    if !(0x7f00..=0x7f08).contains(&id) {
        return Err("Unknown system icon".into());
    }
    // SAFETY: shared system icon (never destroyed); the helper only draws it.
    unsafe {
        let icon = LoadIconW(ptr::null_mut(), id as usize as *const u16);
        if icon.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        icon_rgba(icon, 32)
    }
}

/// C# FileVisualConverter.LoadIcon: SHGetFileInfoW(경로, 없으면 "folder"/".", 폴더/보통 속성,
/// SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES)의 작은 아이콘을 16x16 RGBA로. 파일은 읽지 않는다(확장자·속성으로만).
pub fn shell_small_icon(path: &Path, directory: bool) -> Option<(usize, usize, Vec<u8>)> {
    let text = path.to_string_lossy();
    let candidate = if text.trim().is_empty() { if directory { "folder".to_owned() } else { ".".to_owned() } } else { text.into_owned() };
    let name = wide(&candidate);
    let attributes = if directory { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    // SAFETY: the path is NUL-terminated and the structure size is passed; the returned icon is destroyed below.
    unsafe {
        let result = SHGetFileInfoW(name.as_ptr(), attributes, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES);
        if result == 0 || info.hIcon.is_null() {
            return None;
        }
        let pixels = icon_rgba(info.hIcon, 16).ok();
        DestroyIcon(info.hIcon);
        pixels
    }
}

/// 원본 FUN_00422314(TfFileExists imgOrg·imgNew): SHGetFileInfoW(경로, 0, SHGFI_ICON, 없는 경로면 | SHGFI_USEFILEATTRIBUTES)의
/// 큰 아이콘을 32x32 RGBA로.
pub fn shell_large_icon(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let name = wide(path);
    let mut flags = SHGFI_ICON;
    if !path.exists() {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    // SAFETY: the path is NUL-terminated and the structure size is passed; the returned icon is destroyed below.
    unsafe {
        let result = SHGetFileInfoW(name.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, flags);
        if result == 0 || info.hIcon.is_null() {
            return None;
        }
        let pixels = icon_rgba(info.hIcon, 32).ok();
        DestroyIcon(info.hIcon);
        pixels
    }
}

/// "X:\"(드라이브 루트)인가.
fn is_drive_root(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let chars: Vec<char> = text.chars().collect();
    chars.len() == 3 && chars[0].is_ascii_alphabetic() && chars[1] == ':' && chars[2] == '\\'
}

/// Windows 기본 "네트워크 드라이브" 작은 아이콘(SHGetStockIconInfo SIID_DRIVENET).
fn stock_network_drive_icon() -> Option<(usize, usize, Vec<u8>)> {
    use windows_sys::Win32::UI::Shell::{SHGSI_ICON, SHGSI_SMALLICON, SHGetStockIconInfo, SHSTOCKICONINFO, SIID_DRIVENET};
    let mut info: SHSTOCKICONINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHSTOCKICONINFO>() as u32;
    // SAFETY: the structure size is set; the returned icon is destroyed below.
    unsafe {
        if SHGetStockIconInfo(SIID_DRIVENET, SHGSI_ICON | SHGSI_SMALLICON, &mut info) < 0 || info.hIcon.is_null() {
            return None;
        }
        let pixels = icon_rgba(info.hIcon, 16).ok();
        DestroyIcon(info.hIcon);
        pixels
    }
}

/// C# FileVisualConverter.LoadShellOverlayIcon: SHGetFileInfoW(실제 경로, 0, SHGFI_ICON | SHGFI_SMALLICON | SHGFI_ADDOVERLAYS).
/// 셸이 그 항목을 실제로 본다(셸 확장 포함). com_sta_init을 부른 따로 된 스레드에서 쓴다.
pub fn shell_overlay_icon(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    // 고친 것: 볼륨을 읽을 수 있는 네트워크 드라이브(WSL 9P 등)에 셸이 "연결 끊김" 아이콘을 주므로 Windows 네트워크 드라이브 아이콘을 쓴다.
    if is_drive_root(path) && connected_network_drive(path)
        && let Some(icon) = stock_network_drive_icon()
    {
        return Some(icon);
    }
    let name = wide(path);
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    // SAFETY: the path is NUL-terminated and the structure size is passed; the returned icon is destroyed below.
    unsafe {
        let result = SHGetFileInfoW(name.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_SMALLICON | SHGFI_ADDOVERLAYS);
        if result == 0 || info.hIcon.is_null() {
            return None;
        }
        let pixels = icon_rgba(info.hIcon, 16).ok();
        DestroyIcon(info.hIcon);
        pixels
    }
}

/// 원본 TNXShellImageList.GetIconIndex(0x7b8d54) 파일 항목: SHGetFileInfoW(실제 경로, 속성, SHGFI_SYSICONINDEX)로
/// 그 파일 자신의 아이콘(실행 파일·바로 가기 등). 오버레이는 넣지 않는다. com_sta_init을 부른 따로 된 스레드에서 쓴다.
pub fn shell_path_icon(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let name = wide(path);
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    // SAFETY: the path is NUL-terminated and the structure size is passed; the returned icon is destroyed below.
    unsafe {
        let result = SHGetFileInfoW(name.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_SMALLICON);
        if result == 0 || info.hIcon.is_null() {
            return None;
        }
        let pixels = icon_rgba(info.hIcon, 16).ok();
        DestroyIcon(info.hIcon);
        pixels
    }
}

/// 패널 폴더 트리(TNXShellTreeView)의 셸 항목 아이콘: 열쇠(빈 열쇠 = 바탕 화면, "::{CLSID}" 가상 폴더, 경로)를
/// SHParseDisplayName으로 항목 목록으로 바꿔 SHGetFileInfoW(SHGFI_PIDL | SHGFI_ICON | SHGFI_SMALLICON). com_sta_init을 부른 스레드에서 쓴다.
pub fn shell_key_icon(key: &Path) -> Option<(usize, usize, Vec<u8>)> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{SHGFI_PIDL, SHGetSpecialFolderLocation, SHParseDisplayName};
    let mut pidl: *mut windows_sys::Win32::UI::Shell::Common::ITEMIDLIST = ptr::null_mut();
    // SAFETY: NUL-terminated name; the identifier list is freed below; the returned icon is destroyed below.
    unsafe {
        let ok = if key.as_os_str().is_empty() {
            SHGetSpecialFolderLocation(ptr::null_mut(), 0, &mut pidl) == 0
        } else {
            let name = wide(key);
            SHParseDisplayName(name.as_ptr(), ptr::null_mut(), &mut pidl, 0, ptr::null_mut()) == 0
        };
        if !ok || pidl.is_null() {
            return None;
        }
        let mut info: SHFILEINFOW = std::mem::zeroed();
        let result = SHGetFileInfoW(pidl.cast_const().cast(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_PIDL | SHGFI_ICON | SHGFI_SMALLICON);
        CoTaskMemFree(pidl.cast_const().cast());
        if result == 0 || info.hIcon.is_null() {
            return None;
        }
        let pixels = icon_rgba(info.hIcon, 16).ok();
        DestroyIcon(info.hIcon);
        pixels
    }
}

/// 글꼴 파일처럼 프로그램이 끝날 때까지 읽기만 하는 파일을 메모리에 사상(CreateFileMappingW·MapViewOfFile)해 돌려준다.
/// 고친 것(메모리): 전에는 글꼴 파일을 통째로 읽고(Vec), egui가 그것을 한 번 더 복사해 맑은 고딕(12.8MB)·굵게(12MB)가 두 벌씩
/// 개인 메모리에 있었다. 사상한 파일은 운영 체제 파일 캐시와 함께 쓰는 쪽(필요한 쪽만 읽음)이 된다. 같은 파일은 한 번만 사상한다.
pub fn mapped_file(path: &Path) -> Option<&'static [u8]> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, GetFileSizeEx, OPEN_EXISTING};
    use windows_sys::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY};
    static MAPPED: OnceLock<Mutex<HashMap<PathBuf, (usize, usize)>>> = OnceLock::new();
    let cache = MAPPED.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    let key = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(&(address, length)) = cache.get(&key) {
        // SAFETY: the view below is never unmapped, so the slice stays valid for the whole process.
        return Some(unsafe { std::slice::from_raw_parts(address as *const u8, length) });
    }
    let name = wide(key.as_os_str());
    // SAFETY: NUL-terminated path; handles are closed after mapping, the view keeps the mapping alive.
    unsafe {
        let file = CreateFileW(name.as_ptr(), GENERIC_READ, FILE_SHARE_READ, ptr::null(), OPEN_EXISTING, 0, ptr::null_mut());
        if file == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut size = 0i64;
        if GetFileSizeEx(file, &mut size) == 0 || size <= 0 || size > 512 * 1024 * 1024 {
            CloseHandle(file);
            return None;
        }
        let mapping = CreateFileMappingW(file, ptr::null(), PAGE_READONLY, 0, 0, ptr::null());
        CloseHandle(file);
        if mapping.is_null() {
            return None;
        }
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        CloseHandle(mapping);
        if view.Value.is_null() {
            return None;
        }
        let (address, length) = (view.Value as usize, size as usize);
        cache.insert(key, (address, length));
        Some(std::slice::from_raw_parts(address as *const u8, length))
    }
}

/// 이 스레드를 셸용 STA로(C# 오버레이 작업 스레드의 ApartmentState.STA). 스레드가 끝날 때까지 그대로 둔다.
pub fn com_sta_init() {
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    // SAFETY: plain COM apartment initialisation for the calling thread.
    unsafe {
        CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }
}

/// 원본 miAddToFavoritesClick(0x43371d): 굵은 폼 글꼴로 DrawTextW(경로, 사각형 0,0,width,30,
/// DT_PATH_ELLIPSIS | DT_MODIFYSTRING | DT_NOPREFIX | DT_SINGLELINE = 0x14820)한 뒤 바뀐 글(가운데 "...")을 쓴다. 실패하면 그대로.
pub fn compact_path(path: &str, width: i32) -> String {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::*;
    let mut buffer: Vec<u16> = path.encode_utf16().chain(std::iter::repeat_n(0, 8)).collect();
    let length = path.encode_utf16().count() as i32;
    // SAFETY: a private memory DC and font are created, used and released here; the buffer has room for DT_MODIFYSTRING.
    unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            return path.to_owned();
        }
        let mut font: LOGFONTW = std::mem::zeroed();
        let stock = GetStockObject(DEFAULT_GUI_FONT);
        GetObjectW(stock, size_of::<LOGFONTW>() as i32, (&mut font as *mut LOGFONTW).cast());
        font.lfWeight = 700;
        let bold = CreateFontIndirectW(&font);
        let previous = if bold.is_null() { ptr::null_mut() } else { SelectObject(dc, bold) };
        let mut rect = RECT { left: 0, top: 0, right: width, bottom: 30 };
        DrawTextW(dc, buffer.as_mut_ptr(), length, &mut rect, 0x14820);
        if !bold.is_null() {
            SelectObject(dc, previous);
            DeleteObject(bold);
        }
        DeleteDC(dc);
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// HICON을 side x side로 그려 직선(비곱셈) RGBA로.
/// SAFETY: icon must be a valid HICON for the duration of the call.
unsafe fn icon_rgba(icon: HICON, side: i32) -> Result<(usize, usize, Vec<u8>), String> {
    use windows_sys::Win32::Graphics::Gdi::*;
    unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = side;
        info.bmiHeader.biHeight = -side;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let previous = SelectObject(dc, bitmap);
        let drawn = DrawIconEx(dc, 0, 0, icon, side, side, 0, ptr::null_mut(), DI_NORMAL);
        GdiFlush();
        let count = (side * side * 4) as usize;
        let mut pixels = std::slice::from_raw_parts(bits.cast::<u8>(), count).to_vec();
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        if drawn == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // BGRA (premultiplied by DrawIconEx) -> straight RGBA.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            let alpha = pixel[3] as u32;
            if alpha > 0 && alpha < 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        Ok((side as usize, side as usize, pixels))
    }
}

/// 시스템 난수(BCryptGenRandom, 시스템 기본 RNG). C# RandomNumberGenerator와 같은 출처.
pub fn random_bytes(buffer: &mut [u8]) -> Result<(), String> {
    for chunk in buffer.chunks_mut(u32::MAX as usize) {
        // SAFETY: the buffer is valid for chunk.len() bytes.
        let status = unsafe { BCryptGenRandom(ptr::null_mut(), chunk.as_mut_ptr(), chunk.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if status != 0 {
            return Err(format!("BCryptGenRandom failed: {status:#x}"));
        }
    }
    Ok(())
}

/// Environment.ExpandEnvironmentVariables: ExpandEnvironmentStringsW(%NAME% 형식).
pub fn expand_environment(text: &str) -> String {
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    let source = wide(text);
    // SAFETY: the first call measures, the second fills a buffer of that size.
    unsafe {
        let size = ExpandEnvironmentStringsW(source.as_ptr(), ptr::null_mut(), 0);
        if size == 0 {
            return text.to_owned();
        }
        let mut buffer = vec![0u16; size as usize];
        let written = ExpandEnvironmentStringsW(source.as_ptr(), buffer.as_mut_ptr(), size);
        if written == 0 || written > size {
            return text.to_owned();
        }
        String::from_utf16_lossy(&buffer[..written as usize - 1])
    }
}

/// DriveInfo.DriveType 이름(GetDriveTypeW): Unknown, NoRootDirectory, Removable, Fixed, Network, CDRom, Ram.
pub fn drive_type_name(root: &Path) -> &'static str {
    // SAFETY: NUL-terminated root path.
    match unsafe { GetDriveTypeW(wide(root.as_os_str()).as_ptr()) } {
        1 => "NoRootDirectory",
        2 => "Removable",
        3 => "Fixed",
        4 => "Network",
        5 => "CDRom",
        6 => "Ram",
        _ => "Unknown",
    }
}

/// DriveInfo.VolumeLabel(GetVolumeInformationW). 준비되지 않았으면 None.
pub fn volume_label(root: &Path) -> Option<String> {
    let mut name = vec![0u16; 261];
    // SAFETY: NUL-terminated root and a buffer of the stated length; the other outputs are optional.
    let ok = unsafe {
        GetVolumeInformationW(wide(root.as_os_str()).as_ptr(), name.as_mut_ptr(), name.len() as u32, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), 0)
    } != 0;
    ok.then(|| String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]))
}

/// 원본 FUN_004266ac: SHFileOperationW(FO_DELETE, pFrom = 경로, 휴지통이면 FOF_ALLOWUNDO 0x40, 아니면 0) — 확인·진행은 셸 창. 0 = 성공.
pub fn shell_delete(path: &Path, allow_undo: bool) -> i32 {
    use windows_sys::Win32::UI::Shell::{SHFILEOPSTRUCTW, SHFileOperationW};
    let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = 3;
    op.pFrom = from.as_ptr();
    op.fFlags = if allow_undo { 0x40 } else { 0 };
    // SAFETY: pFrom is double-NUL terminated and outlives the call; the other fields are zeroed.
    let result = unsafe { SHFileOperationW(&mut op) };
    if op.fAnyOperationsAborted != 0 && result == 0 { 1223 } else { result }
}

/// 원본 xilesShell GetVolumeSerial: GetVolumeInformationW의 볼륨 일련번호(트리 파일 이름 FUN_007e3e7c에 쓴다). 읽을 수 없으면 None.
pub fn volume_serial(root: &Path) -> Option<u32> {
    let mut serial = 0u32;
    // SAFETY: NUL-terminated root; only the serial output is requested.
    let ok = unsafe { GetVolumeInformationW(wide(root.as_os_str()).as_ptr(), ptr::null_mut(), 0, &mut serial, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), 0) } != 0;
    ok.then_some(serial)
}

/// InputLanguageManager.CurrentInputLanguage가 영어인가(GetKeyboardLayout(0)의 주 언어 = LANG_ENGLISH).
pub fn english_input_language() -> bool {
    // SAFETY: GetKeyboardLayout(0) reads the calling thread's layout.
    let layout = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout(0) } as usize;
    (layout & 0x3ff) == 0x09
}

/// shlwapi StrToIntExW(글, 0 = STIF_DEFAULT): 10진수로 읽히면 그 값.
pub fn str_to_int_ex(text: &str) -> Option<i32> {
    let mut value = 0i32;
    // SAFETY: NUL-terminated input and a valid output pointer.
    (unsafe { StrToIntExW(wide(text).as_ptr(), 0, &mut value) } != 0).then_some(value)
}

/// DriveInfo.DriveType == Fixed: GetDriveTypeW(루트) == DRIVE_FIXED.
pub fn is_fixed_drive(root: &Path) -> bool {
    // SAFETY: NUL-terminated root path.
    unsafe { GetDriveTypeW(wide(root.as_os_str()).as_ptr()) == 3 }
}

/// Environment.GetFolderPath(SpecialFolder.InternetCache / Cookies): SHGetKnownFolderPath(플래그 0, 없으면 None).
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    let mut raw: windows_sys::core::PWSTR = ptr::null_mut();
    // SAFETY: the out pointer receives a CoTaskMemAlloc string that is copied and freed here.
    unsafe {
        let ok = SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut raw) >= 0 && !raw.is_null();
        let path = ok.then(|| {
            let length = (0..).take_while(|&i| *raw.add(i) != 0).count();
            PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(raw, length)))
        });
        windows_sys::Win32::System::Com::CoTaskMemFree(raw as _);
        path.filter(|p| !p.as_os_str().is_empty())
    }
}
/// Environment.GetFolderPath(SpecialFolder.MyDocuments) = FOLDERID_Documents.
pub fn documents_folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_Documents)
}

/// DriveInfo.DriveType == Network: GetDriveTypeW(루트) == DRIVE_REMOTE.
pub fn is_network_drive(root: &Path) -> bool {
    // SAFETY: NUL-terminated root path.
    unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide(root.as_os_str()).as_ptr()) == 4 }
}

/// C# FileVersionInfo: ProductMajorPart(0이면 FileMajorPart). 버전 정보가 없으면 None.
pub fn file_major_version(path: &Path) -> Option<u32> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW};
    let name = wide(path.as_os_str());
    let mut handle = 0u32;
    // SAFETY: NUL-terminated name; the buffer is sized by the first call and outlives VerQueryValueW's pointer use.
    unsafe {
        let size = GetFileVersionInfoSizeW(name.as_ptr(), &mut handle);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(name.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let root = wide("\\");
        let mut info: *mut std::ffi::c_void = ptr::null_mut();
        let mut length = 0u32;
        if VerQueryValueW(data.as_ptr().cast(), root.as_ptr(), &mut info, &mut length) == 0 || info.is_null() || (length as usize) < size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }
        let fixed = &*(info as *const VS_FIXEDFILEINFO);
        let product = fixed.dwProductVersionMS >> 16;
        Some(if product > 0 { product } else { fixed.dwFileVersionMS >> 16 })
    }
}

/// C# UpdateDiskCapacityAsync의 DriveInfo: "X:\" 드라이브만, 네트워크 드라이브와 준비 안 된 드라이브는 None. (쓸 수 있는 여유, 전체).
pub fn disk_capacity(path: &Path) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};
    let text = path.to_string_lossy();
    let chars: Vec<char> = text.chars().take(3).collect();
    if chars.len() < 2 || chars[1] != ':' || !chars[0].is_ascii_alphabetic() {
        return None;
    }
    let root = wide(format!("{}:\\", chars[0]));
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: NUL-terminated root string; out pointers refer to live locals.
    unsafe {
        if GetDriveTypeW(root.as_ptr()) == 4 {
            return None;
        }
        if GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, ptr::null_mut()) == 0 {
            return None;
        }
    }
    Some((free, total))
}

/// 연결된 네트워크 드라이브의 이름: 연결 대상 UNC 경로 그대로("\\서버\공유\아래", 끝 '\' 뺌).
/// 볼륨 이름이 없는 연결(9P 등)은 셸이 "연결 끊긴 네트워크 드라이브"로 보이므로 대신 쓴다. 사용자 요청: 전에는 탐색기처럼 "아래 (\\서버\공유)"였다.
pub fn network_drive_name(target: &str) -> Option<String> {
    let trimmed = target.trim_end_matches('\\');
    let rest = trimmed.strip_prefix("\\\\")?;
    let parts: Vec<&str> = rest.split('\\').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    Some(format!("\\\\{}", parts.join("\\")))
}

/// 드라이브 글자의 연결 대상 UNC: QueryDosDeviceW("X:")의 "\??\UNC\서버\공유…" 또는
/// "\Device\<리디렉터>\;X:<번호>\서버\공유…"를 "\\서버\공유…"로. 네트워크 연결이 아니면 None.
pub fn dos_device_unc(device: &str) -> Option<String> {
    if let Some(rest) = device.strip_prefix("\\??\\UNC\\") {
        return Some(format!("\\\\{rest}"));
    }
    let after = device.strip_prefix("\\Device\\")?;
    let (_, mapped) = after.split_once('\\')?;
    let rest = mapped.strip_prefix(';').and_then(|m| m.split_once('\\')).map_or(mapped, |(_, rest)| rest);
    (!rest.is_empty()).then(|| format!("\\\\{rest}"))
}

/// 드라이브 글자(루트 "X:\")의 네트워크 연결 대상(dos_device_unc).
fn drive_unc_target(root: &Path) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;
    let text = root.to_string_lossy();
    let letter = text.chars().next().filter(|c| c.is_ascii_alphabetic() && text[1..].starts_with(':'))?;
    let mut buffer = vec![0u16; 1024];
    // SAFETY: NUL-terminated device name and a buffer of the stated length.
    let count = unsafe { QueryDosDeviceW(wide(format!("{letter}:")).as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
    if count == 0 {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    dos_device_unc(&String::from_utf16_lossy(&buffer[..end]))
}

/// 연결된(볼륨을 읽을 수 있는) 네트워크 드라이브인가: GetDriveTypeW == DRIVE_REMOTE이고 GetVolumeInformationW가 된다.
pub fn connected_network_drive(root: &Path) -> bool {
    is_network_drive(root) && volume_label(root).is_some()
}

/// 원본 xilesShell GetDriveName(0x7b4a74): 이동식이 아니고 준비됐으면 볼륨 이름(GetVolumeInformationW),
/// 비었거나 이동식이면 셸 표시 이름(SHGetFileInfoW SHGFI_DISPLAYNAME)에서 마지막 '(' 앞 한 글자부터 자른 것.
/// 고친 것: 볼륨을 읽을 수 있는 네트워크 드라이브는 셸 이름("연결 끊긴 네트워크 드라이브") 대신 연결 대상에서 만든 이름.
pub fn drive_name(root: &Path) -> String {
    let removable = drive_type_name(root) == "Removable";
    if let Some(label) = (!removable).then(|| volume_label(root)).flatten().filter(|l| !l.is_empty()) {
        return label;
    }
    if connected_network_drive(root)
        && let Some(name) = drive_unc_target(root).as_deref().and_then(network_drive_name)
    {
        return name;
    }
    let name = wide(root.as_os_str());
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    // SAFETY: the path is NUL-terminated and the structure size is passed.
    unsafe { SHGetFileInfoW(name.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, 0x200) };
    let length = info.szDisplayName.iter().position(|c| *c == 0).unwrap_or(info.szDisplayName.len());
    let mut shown = String::from_utf16_lossy(&info.szDisplayName[..length]);
    if let Some(open) = shown.rfind('(').filter(|i| *i > 0) {
        shown.truncate(open);
        shown.pop();
    }
    shown
}

/// 원본 내 PC 드라이브 항목의 TNXDriveInfo(크기 읽기 = 0): (GetDriveName, GetDriveTypeW, 고정 디스크(3)면 (여유, 전체)).
/// Refresh(0x7b7554)는 종류가 0·1이면 크기를 읽지 않고, 표시가 없으면 고정 디스크만 읽는다.
pub fn drive_item_info(root: &Path) -> Option<(String, u32, Option<(u64, u64)>)> {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};
    let name = wide(root.as_os_str());
    // SAFETY: NUL-terminated root string.
    let kind = unsafe { GetDriveTypeW(name.as_ptr()) };
    let (mut free, mut total) = (0u64, 0u64);
    // 고친 것(사용자 요청): 원본은 고정 디스크만 크기를 읽지만, 연결된(볼륨을 읽을 수 있는) 네트워크 드라이브도 읽는다.
    // 끊긴 네트워크 드라이브는 오래 걸릴 수 있어 원본처럼 읽지 않는다.
    let label = drive_name(root);
    let connected = kind == 4 && connected_network_drive(root);
    // SAFETY: NUL-terminated root string; out pointers refer to live locals.
    let sizes = ((kind == 3 || connected) && unsafe { GetDiskFreeSpaceExW(name.as_ptr(), &mut free, &mut total, ptr::null_mut()) } != 0).then_some((free, total));
    Some((label, kind, sizes))
}

/// 원본 PathStripToRootW: "X:\" 또는 "\\서버\공유\"(그리고 드라이브 글자).
fn volume_root(path: &Path) -> Option<(PathBuf, Option<char>)> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else { return None };
    match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            let letter = (letter as char).to_ascii_uppercase();
            Some((PathBuf::from(format!("{letter}:\\")), Some(letter)))
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some((PathBuf::from(format!("\\\\{}\\{}\\", server.to_string_lossy(), share.to_string_lossy())), None)),
        _ => None,
    }
}

/// 원본 정보 표시줄(FUN_0048b500)의 TNXDriveInfo(루트, 크기 읽기 = 1) 재료: (드라이브 글, 여유, 전체).
/// TNXDriveInfo.Refresh(0x7b7554): 종류(GetDriveTypeW)가 0·1이 아니면 GetDiskFreeSpaceExW. 글은 네트워크(4)면 "이름 ",
/// 아니면 "이름 (C:) "(이름 = GetDriveName). 크기를 읽지 못하면 원본처럼 0(TNXDriveInfo는 0으로 시작한다).
pub fn pane_disk_info(path: &Path) -> Option<(String, u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};
    let (root, letter) = volume_root(path)?;
    let name = wide(root.as_os_str());
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: NUL-terminated root string; out pointers refer to live locals.
    let kind = unsafe { GetDriveTypeW(name.as_ptr()) };
    if kind < 2 {
        return None;
    }
    // SAFETY: as above.
    if unsafe { GetDiskFreeSpaceExW(name.as_ptr(), &mut free, &mut total, ptr::null_mut()) } == 0 {
        (free, total) = (0, 0);
    }
    let drive = drive_name(&root);
    let text = match letter {
        Some(letter) if kind != 4 => format!("{drive} ({letter}:) "),
        _ => format!("{drive} "),
    };
    Some((text, free, total))
}

/// HTTPS GET(WinHTTP, 15초 시간 제한, HTTPS → HTTP 넘겨주기 막음, `max_bytes` 넘으면 TooLarge). C# UpdateHttpClient 자리.
pub fn https_get(url: &str, max_bytes: usize) -> Result<crate::update::HttpResponse, crate::update::HttpError> {
    use crate::update::{HttpError, HttpResponse};
    use windows_sys::Win32::Networking::WinHttp::*;
    struct Handle(*mut std::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: a handle returned by WinHTTP, closed once.
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }
    let fail = || {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == 12002 { HttpError::Timeout } else { HttpError::Failed(format!("WinHTTP error {code}")) }
    };
    let wurl = wide(url);
    // SAFETY: every buffer passed to WinHTTP is NUL-terminated and outlives the calls; the URL_COMPONENTS pointers point into `wurl`.
    unsafe {
        let mut parts: URL_COMPONENTS = std::mem::zeroed();
        parts.dwStructSize = size_of::<URL_COMPONENTS>() as u32;
        parts.dwHostNameLength = 1;
        parts.dwUrlPathLength = 1;
        parts.dwExtraInfoLength = 1;
        if WinHttpCrackUrl(wurl.as_ptr(), 0, 0, &mut parts) == 0 {
            return Err(HttpError::Failed("Invalid URL".into()));
        }
        let slice = |p: *mut u16, n: u32| if p.is_null() { String::new() } else { String::from_utf16_lossy(std::slice::from_raw_parts(p, n as usize)) };
        let host = wide(slice(parts.lpszHostName, parts.dwHostNameLength));
        let mut target = slice(parts.lpszUrlPath, parts.dwUrlPathLength) + &slice(parts.lpszExtraInfo, parts.dwExtraInfoLength);
        if target.is_empty() {
            target.push('/');
        }
        let target = wide(target);
        let agent = wide("NDR");
        let session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, ptr::null(), ptr::null(), 0));
        if session.0.is_null() {
            return Err(fail());
        }
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 15_000, 15_000);
        let connection = Handle(WinHttpConnect(session.0, host.as_ptr(), parts.nPort, 0));
        if connection.0.is_null() {
            return Err(fail());
        }
        let verb = wide("GET");
        let request = Handle(WinHttpOpenRequest(connection.0, verb.as_ptr(), target.as_ptr(), ptr::null(), ptr::null(), ptr::null(), WINHTTP_FLAG_SECURE));
        if request.0.is_null() {
            return Err(fail());
        }
        if WinHttpSendRequest(request.0, ptr::null(), 0, ptr::null(), 0, 0, 0) == 0 || WinHttpReceiveResponse(request.0, ptr::null_mut()) == 0 {
            return Err(fail());
        }
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, ptr::null(), (&mut status as *mut u32).cast(), &mut size, ptr::null_mut());
        let mut length = 0u32;
        let mut size = size_of::<u32>() as u32;
        if WinHttpQueryHeaders(request.0, WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER, ptr::null(), (&mut length as *mut u32).cast(), &mut size, ptr::null_mut()) != 0 && length as usize > max_bytes {
            return Err(HttpError::TooLarge);
        }
        let mut final_url = vec![0u16; 4096];
        let mut bytes = (final_url.len() * 2) as u32;
        let secure = WinHttpQueryOption(request.0, WINHTTP_OPTION_URL, final_url.as_mut_ptr().cast(), &mut bytes) != 0 && String::from_utf16_lossy(&final_url[..(bytes as usize / 2).min(final_url.len())]).to_ascii_lowercase().starts_with("https://");
        let mut body = Vec::new();
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(request.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut read) == 0 {
                return Err(fail());
            }
            if read == 0 {
                break;
            }
            if body.len() + read as usize > max_bytes {
                return Err(HttpError::TooLarge);
            }
            body.extend_from_slice(&buffer[..read as usize]);
        }
        Ok(HttpResponse { status, secure, body })
    }
}

/// Win32Exception(code).Message(끝의 " (os error N)" 없이).
pub fn win32_message(code: u32) -> String {
    let text = std::io::Error::from_raw_os_error(code as i32).to_string();
    match text.rfind(" (os error ") {
        Some(at) => text[..at].trim_end().to_owned(),
        None => text,
    }
}

/// C# GetNetworkPathErrorCode: UNC 경로를 읽지 못하면 GetLastError, 아니면 0.
pub fn network_path_error(path: &Path) -> u32 {
    use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    if !path.to_string_lossy().starts_with("\\\\") {
        return 0;
    }
    // SAFETY: NUL-terminated path.
    unsafe {
        if GetFileAttributesW(wide(path.as_os_str()).as_ptr()) == INVALID_FILE_ATTRIBUTES {
            windows_sys::Win32::Foundation::GetLastError()
        } else {
            0
        }
    }
}

/// C# PromptAndConnectNetworkShare: "\\서버\공유"에 Windows 자격 증명 창으로 임시 연결(CONNECT_TEMPORARY | INTERACTIVE | PROMPT).
/// 공유 이름이 없으면 67(ERROR_BAD_NET_NAME). 결과는 Windows 오류 번호(0 = 성공, 1223 = 취소).
pub fn connect_share_interactive(path: &str) -> u32 {
    use windows_sys::Win32::NetworkManagement::WNet::*;
    let normalized = match path.get(..8) {
        Some(head) if head.eq_ignore_ascii_case("\\\\?\\UNC\\") => format!("\\\\{}", &path[8..]),
        _ => path.to_owned(),
    };
    let parts: Vec<&str> = normalized.trim_end_matches('\\').split('\\').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 || !normalized.starts_with("\\\\") || normalized.contains('\0') {
        return 67;
    }
    let remote = wide(format!("\\\\{}\\{}", parts[0], parts[1]));
    let mut resource: NETRESOURCEW = unsafe { std::mem::zeroed() };
    resource.dwType = RESOURCETYPE_DISK;
    resource.lpRemoteName = remote.as_ptr().cast_mut();
    // SAFETY: the remote name is NUL-terminated and outlives the call; no user name or password.
    unsafe { WNetAddConnection2W(&resource, ptr::null(), ptr::null(), 0x4 | 0x8 | 0x10) }
}

/// 이 창이 지금 앞 창(GetForegroundWindow)인지.
pub fn is_foreground(hwnd: isize) -> bool {
    // SAFETY: plain system call without pointers.
    unsafe { GetForegroundWindow() as isize == hwnd }
}

/// C# WindowOpacity: 창 글 = clamp(WindowAlpha/255, 0.35, 1.0). 저장 알파(0~255)를 실제 불투명도(0.35~1.0)로.
pub fn window_opacity(window_alpha: f64) -> f32 {
    (window_alpha / 255.0).clamp(0.35, 1.0) as f32
}

/// Windows 두 번 누르기 기준: GetDoubleClickTime(ms)과 SM_CXDOUBLECLK·SM_CYDOUBLECLK(화소, 첫 누름을 가운데로 한 사각).
/// 원본 Delphi 컨트롤은 Windows가 이 기준으로 보낸 WM_LBUTTONDBLCLK(ssDouble)을 받는다. 처음 값은 500ms, 4×4.
pub fn double_click_metrics() -> (u32, i32, i32) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK};
    // SAFETY: plain system queries without pointers.
    let (ms, cx, cy) = unsafe { (GetDoubleClickTime(), GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)) };
    (if ms == 0 { 500 } else { ms }, if cx <= 0 { 4 } else { cx }, if cy <= 0 { 4 } else { cy })
}

/// C# SystemParameters.VirtualScreen*(DIP): 모든 모니터를 합친 화면(x, y, 너비, 높이)을 egui 점 단위로.
/// GetSystemMetrics 화소를 시스템 DPI(LOGPIXELSX / 96)로 나눈다.
pub fn virtual_screen_points() -> Option<(f32, f32, f32, f32)> {
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, LOGPIXELSX, ReleaseDC};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};
    // SAFETY: plain system calls; the screen DC is released before returning.
    unsafe {
        let (x, y, w, h) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        if w <= 0 || h <= 0 {
            return None;
        }
        let dc = GetDC(ptr::null_mut());
        let dpi = if dc.is_null() { 96 } else { GetDeviceCaps(dc, LOGPIXELSX as _) };
        if !dc.is_null() {
            ReleaseDC(ptr::null_mut(), dc);
        }
        let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
        Some((x as f32 / scale, y as f32 / scale, w as f32 / scale, h as f32 / scale))
    }
}

fn system_dpi_scale() -> f32 {
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, LOGPIXELSX, ReleaseDC};
    // SAFETY: plain system calls; the screen DC is released before returning.
    unsafe {
        let dc = GetDC(ptr::null_mut());
        let dpi = if dc.is_null() { 96 } else { GetDeviceCaps(dc, LOGPIXELSX as _) };
        if !dc.is_null() {
            ReleaseDC(ptr::null_mut(), dc);
        }
        if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 }
    }
}

/// (x, y)(egui 점) 자리 모니터의 작업 영역(작업 표시줄을 뺀 곳)을 egui 점 단위로(x, y, 너비, 높이).
pub fn work_area_points(x: f32, y: f32) -> Option<(f32, f32, f32, f32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    let scale = system_dpi_scale();
    // SAFETY: plain system calls with a live local MONITORINFO.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: (x * scale) as i32, y: (y * scale) as i32 }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let r = info.rcWork;
        Some((r.left as f32 / scale, r.top as f32 / scale, (r.right - r.left) as f32 / scale, (r.bottom - r.top) as f32 / scale))
    }
}

/// 창을 최대화하고 되돌릴 크기(바깥 x, y, 너비, 높이, egui 점)를 정한다(SetWindowPlacement SW_SHOWMAXIMIZED).
pub fn maximize_with_restore(hwnd: isize, restore: (f32, f32, f32, f32)) -> bool {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPlacement, SW_SHOWMAXIMIZED, WINDOWPLACEMENT};
    if hwnd == 0 {
        return false;
    }
    let scale = system_dpi_scale();
    let (x, y, w, h) = restore;
    // rcNormalPosition은 작업 영역 기준 좌표다.
    let (wx, wy) = work_area_points(x, y).map_or((0.0, 0.0), |(l, t, _, _)| (l, t));
    let placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        flags: 0,
        showCmd: SW_SHOWMAXIMIZED as u32,
        ptMinPosition: POINT { x: -1, y: -1 },
        ptMaxPosition: POINT { x: -1, y: -1 },
        rcNormalPosition: RECT { left: ((x - wx) * scale) as i32, top: ((y - wy) * scale) as i32, right: ((x - wx + w) * scale) as i32, bottom: ((y - wy + h) * scale) as i32 },
    };
    // SAFETY: hwnd is this process's main window; the placement is a live local.
    unsafe { SetWindowPlacement(hwnd as _, &placement) != 0 }
}

/// 원본 TForm.Width·Height와 C# Window.Width·Height(WindowWidth·WindowHeight)는 테두리·제목 줄을 포함한 바깥 크기다.
/// 보통 창(WS_OVERLAPPEDWINDOW)의 바깥 크기 − 안쪽(내용) 크기를 AdjustWindowRectEx로 재서 egui 점 단위(시스템 DPI)로 돌려준다.
pub fn window_frame_points() -> Option<(f32, f32)> {
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, LOGPIXELSX, ReleaseDC};
    use windows_sys::Win32::UI::WindowsAndMessaging::{AdjustWindowRectEx, WS_OVERLAPPEDWINDOW};
    let mut rect = windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: 100, bottom: 100 };
    // SAFETY: the RECT is a live local; the screen DC is released before returning.
    unsafe {
        if AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0) == 0 {
            return None;
        }
        let dc = GetDC(ptr::null_mut());
        let dpi = if dc.is_null() { 96 } else { GetDeviceCaps(dc, LOGPIXELSX as _) };
        if !dc.is_null() {
            ReleaseDC(ptr::null_mut(), dc);
        }
        let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
        let (w, h) = ((rect.right - rect.left - 100) as f32 / scale, (rect.bottom - rect.top - 100) as f32 / scale);
        (w >= 0.0 && h >= 0.0).then_some((w, h))
    }
}

/// C# ApplyWindowBoundsFromSettings: Left = clamp(저장, 화면 왼쪽 − 너비 + 96, 화면 오른쪽 − 96),
/// Top = clamp(저장, 화면 위 − 높이 + 64, 화면 아래 − 64). 창이 화면 밖으로 사라지지 않게 일부는 보이게 둔다.
pub fn clamp_window_position(left: f32, top: f32, width: f32, height: f32, screen: (f32, f32, f32, f32)) -> (f32, f32) {
    let (sx, sy, sw, sh) = screen;
    let clamp = |value: f32, low: f32, high: f32| if low > high { low } else { value.clamp(low, high) };
    (clamp(left, sx - width + 96.0, sx + sw - 96.0), clamp(top, sy - height + 64.0, sy + sh - 64.0))
}

/// C# WindowOpacity: 창 알파(0.0~1.0)를 바로 적용한다(SetLayeredWindowAttributes). egui에는 창 불투명도 명령이 없어
/// 네이티브 HWND로 WS_EX_LAYERED를 켜고 알파를 넣는다. 1.0이면 그대로 불투명(255).
pub fn set_window_opacity(hwnd: isize, alpha: f32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, SetLayeredWindowAttributes, GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED};
    if hwnd == 0 {
        return;
    }
    let value = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    // SAFETY: plain system calls on a window handle we own; the style read/write is a standard layered-window setup.
    unsafe {
        let style = GetWindowLongPtrW(hwnd as _, GWL_EXSTYLE);
        if style & WS_EX_LAYERED as isize == 0 {
            SetWindowLongPtrW(hwnd as _, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
        }
        SetLayeredWindowAttributes(hwnd as _, 0, value, LWA_ALPHA);
    }
}


/// 원본 TColorDialog.Execute(환경 설정 색 쪽): Windows 색 고르기 상자(ChooseColorW, 처음 색 = initial). COLORREF(0x00BBGGRR).
/// 사용자 지정 색 16칸은 프로그램이 도는 동안 기억한다(Delphi TColorDialog.CustomColors 자리). 취소면 None. 모달이다.
pub fn choose_color(initial: u32) -> Option<u32> {
    use std::sync::Mutex;
    use windows_sys::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
    static CUSTOM: Mutex<[u32; 16]> = Mutex::new([0x00FF_FFFF; 16]);
    let mut custom = CUSTOM.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: CHOOSECOLORW is plain data; zeroed is its documented Default.
    let mut choose: CHOOSECOLORW = unsafe { std::mem::zeroed() };
    choose.lStructSize = std::mem::size_of::<CHOOSECOLORW>() as u32;
    choose.rgbResult = initial & 0x00FF_FFFF;
    choose.lpCustColors = custom.as_mut_ptr();
    // 원본 DFM: ColorDialog Options={cdFullOpen}.
    choose.Flags = CC_RGBINIT | CC_ANYCOLOR | CC_FULLOPEN;
    // SAFETY: the struct and the 16-entry custom colour array outlive the modal call.
    (unsafe { ChooseColorW(&mut choose) } != 0).then_some(choose.rgbResult & 0x00FF_FFFF)
}

/// 원본 btnFontClick(TFontDialog.Execute, 처음 Options = fdEffects): Windows 글꼴 상자(ChooseFontW).
/// 처음 값 = (이름, 포인트, 원본 TFontStyles 바이트: 1 굵게, 2 기울임, 4 밑줄, 8 취소선). 확인이면 같은 셋. 모달이다.
pub fn choose_font(name: &str, points: f32, style: u8) -> Option<(String, f32, u8)> {
    use windows_sys::Win32::Graphics::Gdi::LOGFONTW;
    use windows_sys::Win32::UI::Controls::Dialogs::{CF_EFFECTS, CF_INITTOLOGFONTSTRUCT, CF_NOVERTFONTS, CF_SCREENFONTS, CHOOSEFONTW, ChooseFontW};
    // SAFETY: both structs are plain data; zeroed is their documented Default.
    let mut font: LOGFONTW = unsafe { std::mem::zeroed() };
    for (slot, unit) in font.lfFaceName.iter_mut().zip(name.encode_utf16().take(31)) {
        *slot = unit;
    }
    let points = if points > 0.0 { points } else { 9.0 };
    font.lfHeight = -((points * 96.0 / 72.0).round() as i32);
    font.lfWeight = if style & 1 != 0 { 700 } else { 400 };
    font.lfItalic = u8::from(style & 2 != 0);
    font.lfUnderline = u8::from(style & 4 != 0);
    font.lfStrikeOut = u8::from(style & 8 != 0);
    let mut choose: CHOOSEFONTW = unsafe { std::mem::zeroed() };
    choose.lStructSize = std::mem::size_of::<CHOOSEFONTW>() as u32;
    choose.lpLogFont = &mut font;
    choose.iPointSize = (points * 10.0).round() as i32;
    choose.Flags = CF_INITTOLOGFONTSTRUCT | CF_SCREENFONTS | CF_NOVERTFONTS | CF_EFFECTS;
    // SAFETY: the LOGFONTW lives across the modal call.
    if unsafe { ChooseFontW(&mut choose) } == 0 {
        return None;
    }
    let length = font.lfFaceName.iter().position(|c| *c == 0).unwrap_or(font.lfFaceName.len());
    let style = u8::from(font.lfWeight >= 700) | u8::from(font.lfItalic != 0) << 1 | u8::from(font.lfUnderline != 0) << 2 | u8::from(font.lfStrikeOut != 0) << 3;
    Some((String::from_utf16_lossy(&font.lfFaceName[..length]), choose.iPointSize as f32 / 10.0, style))
}

/// 원본 TFontStyles(1 굵게, 2 기울임)에 맞는 글꼴 레지스트리 이름 후보: 그 꼴 이름("Segoe UI Bold Italic")부터 가까운 꼴, 마지막은 보통 꼴.
pub fn styled_font_names(family: &str, style: u8) -> Vec<String> {
    let family = family.trim();
    let suffixes: &[&str] = match style & 3 {
        3 => &[" Bold Italic", " Bold", " Italic"],
        1 => &[" Bold"],
        2 => &[" Italic"],
        _ => &[],
    };
    suffixes.iter().map(|suffix| format!("{family}{suffix}")).chain(std::iter::once(family.to_owned())).collect()
}

/// 화면 좌표의 마우스 위치(물리 화소). 셸 끌기·탐색기에서 놓을 때 egui 포인터 위치가 낡아 있으므로 이것을 쓴다.
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: the out pointer refers to a live local.
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

/// 원본 WMChar(0x7d5a18)의 GetKeyState(VK_CAPITAL) != 0(Caps Lock이 켜졌거나 눌린 상태).
pub fn caps_lock_state() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
    // SAFETY: plain system call without pointers.
    unsafe { GetKeyState(VK_CAPITAL as i32) != 0 }
}

/// 놓을 때의 Ctrl/Shift(C# DragEventArgs.KeyStates). (ctrl, shift).
pub fn drop_modifiers() -> (bool, bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
    // SAFETY: plain system calls without pointers.
    unsafe { ((GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0, (GetKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0) }
}

#[windows::core::implement(windows::Win32::System::Ole::IDropSource)]
struct FileDropSource;

#[allow(non_snake_case)]
impl windows::Win32::System::Ole::IDropSource_Impl for FileDropSource_Impl {
    fn QueryContinueDrag(&self, escape: windows::core::BOOL, keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS) -> windows::core::HRESULT {
        use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP};
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & windows::Win32::System::SystemServices::MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            windows::core::HRESULT(0)
        }
    }
    fn GiveFeedback(&self, _effect: windows::Win32::System::Ole::DROPEFFECT) -> windows::core::HRESULT {
        windows::Win32::Foundation::DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// C# FileList_PreviewMouseMove의 DoDragDrop(FileDrop, Copy | Move | Link): 셸 데이터 개체로 NDR 밖(탐색기 등)에 끌어 놓는다.
/// 놓은 쪽이 복사·이동·바로 가기를 한다. 결과 효과(DROPEFFECT 비트, 0 = 취소).
pub fn drag_files_out(paths: &[PathBuf]) -> Result<u32, String> {
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop, IDropSource};
    use windows::Win32::UI::Shell::{BHID_DataObject, ILCreateFromPathW, ILFree, IShellItemArray, SHCreateShellItemArrayFromIDLists};
    use windows::core::PCWSTR;
    if paths.is_empty() {
        return Ok(0);
    }
    // SAFETY: every PIDL comes from ILCreateFromPathW and is freed once; COM objects are released by their wrappers.
    unsafe {
        let pidls: Vec<_> = paths.iter().map(|p| ILCreateFromPathW(PCWSTR(wide(p.as_os_str()).as_ptr()))).collect();
        let result = (|| -> Result<u32, String> {
            if pidls.iter().any(|p| p.is_null()) {
                return Err("Could not resolve a dragged item".into());
            }
            let list: Vec<*const _> = pidls.iter().map(|p| *p as *const _).collect();
            let array: IShellItemArray = SHCreateShellItemArrayFromIDLists(&list).map_err(|e| e.to_string())?;
            let data: IDataObject = array.BindToHandler(None, &BHID_DataObject).map_err(|e| e.to_string())?;
            let source: IDropSource = FileDropSource.into();
            let mut effect = DROPEFFECT(0);
            let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect);
            Ok(effect.0)
        })();
        for pidl in pidls {
            if !pidl.is_null() {
                ILFree(Some(pidl));
            }
        }
        result
    }
}

/// C# OriginalShortcutService.ExistingTargetFolder (원본 FUN_00441af4): SHGetSpecialFolderPathW(CSIDL_DESKTOPDIRECTORY 0x10)
/// 또는 CSIDL_APPDATA(0x1a) + "Microsoft\Internet Explorer\Quick Launch". 폴더가 있을 때만(만들지 않는다).
pub fn shortcut_target_folder(quick_launch: bool) -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    // SAFETY: the buffer is larger than MAX_PATH as SHGetSpecialFolderPathW requires.
    let found = unsafe { SHGetSpecialFolderPathW(ptr::null_mut(), buffer.as_mut_ptr(), if quick_launch { 0x1a } else { 0x10 }, 0) } != 0;
    if !found {
        return None;
    }
    let length = buffer.iter().position(|c| *c == 0).unwrap_or(0);
    let mut folder = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    if quick_launch {
        folder.push(r"Microsoft\Internet Explorer\Quick Launch");
    }
    (!folder.as_os_str().is_empty() && folder.exists()).then_some(folder)
}

/// C# OriginalShortcutService.LinkPath: PathAppendW(folder, PathStripPathW(target)) + ".lnk" — 대상 확장자는 남긴다.
pub fn original_link_path(target: &Path, folder: &Path) -> PathBuf {
    let name = crate::names::find_file_name(&target.to_string_lossy());
    let mut link = folder.join(name).into_os_string();
    link.push(".lnk");
    PathBuf::from(link)
}

/// GetLongPathNameW(짧은 이름 "PROGRA~1"을 긴 이름으로). 실패하면 None.
pub fn long_path(path: &Path) -> Option<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let short = wide(path);
    let mut long = vec![0u16; 0x400];
    // SAFETY: NUL-terminated input; the output length is passed.
    let length = unsafe { GetLongPathNameW(short.as_ptr(), long.as_mut_ptr(), long.len() as u32) } as usize;
    (length > 0 && length < long.len()).then(|| PathBuf::from(String::from_utf16_lossy(&long[..length])))
}

/// 원본 GetShortcutTarget(바로 가기, &WIN32_FIND_DATAW): IShellLinkW를 읽어(IPersistFile::Load) 대상 경로(GetPath)와 그 찾기 자료의
/// 속성이 폴더(FILE_ATTRIBUTE_DIRECTORY)인지. 읽지 못하면 None.
pub fn shortcut_target(link: &Path) -> Option<(PathBuf, bool)> {
    use windows::{
        Win32::{
            Storage::FileSystem::WIN32_FIND_DATAW,
            System::Com::*,
            UI::Shell::{IShellLinkW, ShellLink},
        },
        core::{Interface, PCWSTR},
    };
    // SAFETY: COM is initialised for this call; buffers outlive the calls that use them.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<(PathBuf, bool)> {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let persist: IPersistFile = shell_link.cast()?;
            let name = wide(link);
            persist.Load(PCWSTR(name.as_ptr()), STGM_READ)?;
            let mut buffer = [0u16; 1024];
            let mut data = WIN32_FIND_DATAW::default();
            shell_link.GetPath(&mut buffer, &mut data, 0)?;
            let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            Ok((PathBuf::from(String::from_utf16_lossy(&buffer[..length])), data.dwFileAttributes & 0x10 != 0))
        })();
        if initialized {
            CoUninitialize();
        }
        result.ok().filter(|(path, _)| !path.as_os_str().is_empty())
    }
}

/// C# OriginalShortcutService.Create (원본 @CreateShortcut 0x7b3d04): SetPath(대상), SetArguments(""),
/// SetWorkingDirectory(PathRemoveFileSpecW(대상)), SetIconLocation(NULL, 1), SetHotkey(0), SetShowCmd(0),
/// IPersistFile::Save(링크, TRUE) — 있는 링크는 조용히 바꾼다. 원본 호출부는 결과를 보지 않는다.
pub fn create_original_shortcut(target: &Path, folder: &Path) -> Result<(), String> {
    use windows::{
        Win32::{
            System::Com::*,
            UI::{
                Shell::{IShellLinkW, ShellLink},
                WindowsAndMessaging::SHOW_WINDOW_CMD,
            },
        },
        core::{Interface, PCWSTR},
    };
    let directory = crate::names::remove_file_spec(&target.to_string_lossy());
    let link_path = original_link_path(target, folder);
    // SAFETY: COM is initialised for this call; every string buffer outlives the call that uses it.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<()> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let (path, empty, directory, output) = (wide(target), wide(""), wide(&directory), wide(&link_path));
            link.SetPath(PCWSTR(path.as_ptr()))?;
            link.SetArguments(PCWSTR(empty.as_ptr()))?;
            link.SetWorkingDirectory(PCWSTR(directory.as_ptr()))?;
            link.SetIconLocation(PCWSTR::null(), 1)?;
            link.SetHotkey(0)?;
            link.SetShowCmd(SHOW_WINDOW_CMD(0))?;
            let persist: IPersistFile = link.cast()?;
            persist.Save(PCWSTR(output.as_ptr()), true)?;
            Ok(())
        })();
        if initialized {
            CoUninitialize();
        }
        result.map_err(|e| e.to_string())
    }
}

/// Environment.GetFolderPath(Desktop/MyDocuments/MyPictures/MyMusic/MyVideos) = 해당 알려진 폴더.
pub fn special_folder(name: &str) -> Option<PathBuf> {
    let id = match name {
        "desktop" => &FOLDERID_Desktop,
        "documents" => &FOLDERID_Documents,
        "pictures" => &FOLDERID_Pictures,
        "music" => &FOLDERID_Music,
        "videos" => &FOLDERID_Videos,
        _ => return None,
    };
    known_folder(id)
}

/// Windows 시스템 색(GetSysColor, 예: 13 COLOR_HIGHLIGHT, 14 COLOR_HIGHLIGHTTEXT)을 (R, G, B)로.
pub fn system_color(index: i32) -> (u8, u8, u8) {
    // SAFETY: GetSysColor only reads a colour table entry.
    let value = unsafe { windows_sys::Win32::Graphics::Gdi::GetSysColor(index as _) };
    ((value & 0xff) as u8, ((value >> 8) & 0xff) as u8, ((value >> 16) & 0xff) as u8)
}

/// C# FileInformationService.GetShellTypeName: SHGetFileInfoW("folder" | "file" | "file.ext", 폴더/보통 속성,
/// SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES). 실패면 빈 글.
pub fn shell_type_name(extension: &str, is_dir: bool) -> String {
    let lookup = if is_dir { "folder".to_owned() } else if extension.is_empty() { "file".to_owned() } else { format!("file{extension}") };
    let name = wide(&lookup);
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let attributes = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    // SAFETY: the path is NUL-terminated and the structure size is passed.
    let result = unsafe { SHGetFileInfoW(name.as_ptr(), attributes, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES) };
    if result == 0 {
        return String::new();
    }
    let length = info.szTypeName.iter().position(|c| *c == 0).unwrap_or(info.szTypeName.len());
    String::from_utf16_lossy(&info.szTypeName[..length]).trim().to_owned()
}

/// C# IsWindowsLightTheme: HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize의 AppsUseLightTheme(없으면 1) != 0.
/// 읽지 못하면 false. (C#의 고대비 분기는 옮기지 않았다.)
pub fn windows_light_theme() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    let mut data: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: all strings are NUL-terminated; data and size point to valid locals.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, ptr::null_mut(), (&mut data as *mut u32).cast(), &mut size) };
    match status {
        0 => data != 0,
        2 => true, // ERROR_FILE_NOT_FOUND: GetValue 기본값 1
        _ => false,
    }
}

/// 원본 pmEditPopup: IsClipboardFormatAvailable(CF_HDROP = 15). 클립보드를 열지 않는다.
pub fn clipboard_has_files() -> bool {
    // SAFETY: no pointers; only queries the clipboard format list.
    unsafe { IsClipboardFormatAvailable(15) != 0 }
}

/// 원본 miEmptyTempClick: GetTempPathW(0x400) → GetLongPathNameW(0x400). 둘 중 하나라도 실패하면 None.
pub fn temp_folder() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let mut short = [0u16; 0x400];
    let mut long = [0u16; 0x400];
    // SAFETY: both buffers hold 0x400 UTF-16 units; the APIs write at most that many and NUL-terminate.
    let length = unsafe { GetTempPathW(0x400, short.as_mut_ptr()) } as usize;
    if length == 0 || length >= short.len() {
        return None;
    }
    let length = unsafe { GetLongPathNameW(short.as_ptr(), long.as_mut_ptr(), 0x400) } as usize;
    if length == 0 || length >= long.len() {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(&long[..length])))
}

/// StrFormatByteSizeW(크기, 0x80 칸) — 원본이 크기 글에 쓰는 셸 형식("1.50 MB", "512 bytes").
pub fn format_byte_size(bytes: u64) -> String {
    let mut buffer = [0u16; 0x80];
    // SAFETY: the buffer holds 0x80 UTF-16 units and the length passed matches it.
    let result = unsafe { StrFormatByteSizeW(bytes.min(i64::MAX as u64) as i64, buffer.as_mut_ptr(), 0x80) };
    if result.is_null() {
        return bytes.to_string();
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// 원본 고급 선택(FUN_00438034)의 PathMatchSpecW(파일 이름, 필터 하나) == 1.
pub fn path_match_spec(file: &str, spec: &str) -> bool {
    let (file, spec) = (wide(file), wide(spec));
    // SAFETY: both arguments are NUL-terminated UTF-16 buffers that outlive the call.
    unsafe { PathMatchSpecW(file.as_ptr(), spec.as_ptr()) == 1 }
}

/// .NET FileAttributes.ToString(): 알려진 플래그 이름을 값이 작은 것부터 ", "로. 모르는 비트가 남으면 숫자, 0이면 "0".
pub fn dotnet_attributes_text(attributes: u32) -> String {
    const NAMES: [(u32, &str); 16] = [
        (0x1, "ReadOnly"), (0x2, "Hidden"), (0x4, "System"), (0x10, "Directory"), (0x20, "Archive"), (0x40, "Device"),
        (0x80, "Normal"), (0x100, "Temporary"), (0x200, "SparseFile"), (0x400, "ReparsePoint"), (0x800, "Compressed"),
        (0x1000, "Offline"), (0x2000, "NotContentIndexed"), (0x4000, "Encrypted"), (0x8000, "IntegrityStream"), (0x20000, "NoScrubData"),
    ];
    if attributes == 0 {
        return "0".into();
    }
    let known: u32 = NAMES.iter().map(|(bit, _)| bit).sum();
    if attributes & !known != 0 {
        return attributes.to_string();
    }
    NAMES.iter().filter(|(bit, _)| attributes & bit != 0).map(|(_, name)| *name).collect::<Vec<_>>().join(", ")
}

/// 글꼴 레지스트리 값 이름("Segoe UI (TrueType)", "Cambria & Cambria Math (TrueType)")에서 family의 보통 꼴을 고른다.
/// 이름 부분을 " & "로 나눈 목록에 family가 그대로 있으면 (값 이름, 그 차례 = TTC 번호). 굵게·기울임 등 다른 꼴 이름은 맞지 않는다.
pub fn match_font_entry<'a>(names: impl IntoIterator<Item = &'a str>, family: &str) -> Option<(&'a str, u32)> {
    let family = family.trim();
    if family.is_empty() {
        return None;
    }
    names.into_iter().find_map(|name| {
        let base = name.rsplit_once(" (").map_or(name, |(b, _)| b);
        base.split(" & ").position(|part| part.trim().eq_ignore_ascii_case(family)).map(|index| (name, index as u32))
    })
}

/// 원본·C# 판 목록 글꼴(ListFont) 이름의 글꼴 파일: HKCU·HKLM "Software\Microsoft\Windows NT\CurrentVersion\Fonts"의 값에서
/// match_font_entry로 고른 파일(상대 이름이면 %WINDIR%\Fonts)과 TTC 번호.
pub fn font_file(family: &str) -> Option<(PathBuf, u32)> {
    use windows_sys::Win32::System::Registry::*;
    let subkey = wide("Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts");
    let fonts_dir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from).join("Fonts");
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut handle = ptr::null_mut();
        // SAFETY: NUL-terminated key; the handle is closed below.
        if unsafe { RegOpenKeyExW(root, subkey.as_ptr(), 0, KEY_READ, &mut handle) } != 0 {
            continue;
        }
        let mut entries: Vec<(String, String)> = Vec::new();
        for index in 0u32.. {
            let (mut name, mut data) = (vec![0u16; 512], vec![0u16; 1024]);
            let (mut name_len, mut data_len, mut kind) = (name.len() as u32, (data.len() * 2) as u32, 0u32);
            // SAFETY: buffers hold the advertised sizes; the handle is open.
            let status = unsafe { RegEnumValueW(handle, index, name.as_mut_ptr(), &mut name_len, ptr::null(), &mut kind, data.as_mut_ptr().cast(), &mut data_len) };
            if status != 0 {
                break;
            }
            if kind == REG_SZ {
                let value = String::from_utf16_lossy(&data[..(data_len as usize / 2)]).trim_end_matches('\0').to_owned();
                entries.push((String::from_utf16_lossy(&name[..name_len as usize]), value));
            }
        }
        // SAFETY: handle came from RegOpenKeyExW.
        unsafe { RegCloseKey(handle) };
        if let Some((name, index)) = match_font_entry(entries.iter().map(|(n, _)| n.as_str()), family) {
            let file = entries.iter().find(|(n, _)| n == name).map(|(_, f)| PathBuf::from(f))?;
            let path = if file.is_absolute() { file } else { fonts_dir.join(file) };
            if path.is_file() {
                return Some((path, index));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{clamp_window_position, dos_device_unc, dotnet_attributes_text, match_font_entry, network_drive_name, original_link_path, volume_root, window_opacity};

    #[test]
    fn font_entries_match_the_regular_face_of_a_family() {
        let names = ["Segoe UI Bold (TrueType)", "Segoe UI (TrueType)", "Cambria & Cambria Math (TrueType)", "Malgun Gothic (TrueType)"];
        assert_eq!(match_font_entry(names, "Segoe UI"), Some(("Segoe UI (TrueType)", 0)), "not the bold face");
        assert_eq!(match_font_entry(names, "cambria math"), Some(("Cambria & Cambria Math (TrueType)", 1)), "TTC index");
        assert_eq!(match_font_entry(names, "Tahoma"), None);
        assert_eq!(match_font_entry(names, " "), None);
    }

    #[test]
    fn volume_root_follows_path_strip_to_root() {
        use std::path::{Path, PathBuf};
        assert_eq!(volume_root(Path::new(r"c:\Data\x")), Some((PathBuf::from(r"C:\"), Some('C'))));
        assert_eq!(volume_root(Path::new(r"\\server\share\dir")), Some((PathBuf::from(r"\\server\share\"), None)));
        assert_eq!(volume_root(Path::new("relative")), None);
    }

    #[test]
    fn original_link_keeps_the_target_extension_and_attributes_follow_dotnet() {
        // OriginalShortcutService.LinkPath: "name.ext.lnk" in the target folder.
        let link = original_link_path(std::path::Path::new(r"C:\Data\report.txt"), std::path::Path::new(r"D:\Desk"));
        assert_eq!(link, std::path::PathBuf::from(r"D:\Desk\report.txt.lnk"));
        let folder = original_link_path(std::path::Path::new(r"C:\Data\Photos"), std::path::Path::new(r"D:\Desk"));
        assert_eq!(folder, std::path::PathBuf::from(r"D:\Desk\Photos.lnk"));
        // FileAttributes.ToString(): ascending flag names, unknown bits as the number.
        assert_eq!(dotnet_attributes_text(0x21), "ReadOnly, Archive");
        assert_eq!(dotnet_attributes_text(0x10), "Directory");
        assert_eq!(dotnet_attributes_text(0), "0");
        assert_eq!(dotnet_attributes_text(0x80020), (0x80020u32).to_string());
    }

    #[test]
    fn window_opacity_matches_csharp_clamp() {
        // C#: clamp(WindowAlpha/255, 0.35, 1.0).
        assert_eq!(window_opacity(255.0), 1.0);
        assert_eq!(window_opacity(210.0), (210.0f32 / 255.0));
        // 아래로는 0.35에서 멈춘다(완전히 투명해지지 않음).
        assert_eq!(window_opacity(0.0), 0.35);
        assert_eq!(window_opacity(80.0), 0.35);
        // 위로는 1.0에서 멈춘다.
        assert_eq!(window_opacity(300.0), 1.0);
    }

    #[test]
    fn connected_network_drives_get_a_name_from_their_target() {
        // "\??\UNC\…" 꼴(예: 9P·WebDAV 연결)과 SMB(LanmanRedirector) 꼴.
        assert_eq!(dos_device_unc(r"\??\UNC\host\share\folder").as_deref(), Some(r"\\host\share\folder"));
        assert_eq!(dos_device_unc(r"\Device\LanmanRedirector\;Z:0000000000012345\server\share").as_deref(), Some(r"\\server\share"));
        assert_eq!(dos_device_unc(r"\Device\HarddiskVolume3"), None);
        // 이름 = 연결 대상 UNC 경로(끝 '\' 뺌). 서버만 있으면 이름이 아니다.
        assert_eq!(network_drive_name(r"\\host\share\folder").as_deref(), Some(r"\\host\share\folder"));
        assert_eq!(network_drive_name(r"\\server\share\").as_deref(), Some(r"\\server\share"));
        assert_eq!(network_drive_name(r"\\server"), None);
    }

    #[test]
    fn window_position_is_clamped_onto_the_screen_like_csharp() {
        let screen = (0.0, 0.0, 1920.0, 1080.0);
        // 화면 안이면 그대로.
        assert_eq!(clamp_window_position(10.0, 10.0, 1450.0, 900.0, screen), (10.0, 10.0));
        // 오른쪽·아래로 벗어나면 96×64는 보이게.
        assert_eq!(clamp_window_position(5000.0, 5000.0, 1450.0, 900.0, screen), (1824.0, 1016.0));
        // 왼쪽·위로 벗어나면 오른쪽 96·아래 64가 보이게.
        assert_eq!(clamp_window_position(-5000.0, -5000.0, 1450.0, 900.0, screen), (-1354.0, -836.0));
        // 왼쪽 모니터(음수 좌표)도 가상 화면 안이면 그대로.
        assert_eq!(clamp_window_position(-1500.0, 100.0, 1450.0, 900.0, (-1920.0, 0.0, 3840.0, 1080.0)), (-1500.0, 100.0));
    }
}
