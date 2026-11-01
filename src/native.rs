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

/// Original TNXShellChangeNotify thread (ResetThread 0x7b37b8 → FUN_007b3060): watches a single folder (not subfolders, SetSubFolder default 0)
/// with filter 0x0B (file name / folder name / size changes, constructor 0x7b3304) and calls `on_change` when something changes. Stops when `stop` is set.
/// If the watch cannot be started (missing folder, etc.) it does nothing. The handle is created on the calling thread, so changes after it returns are not missed.
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

/// VCL TCanvas.TextHeight of a font (GetTextExtentPoint32 height = TEXTMETRIC tmHeight) for `face` at `pixels` em height
/// (TFont.Height = -pixels), `style` bit 1 bold and bit 2 italic. GDI rounds the ascent and the descent separately (Tahoma 16 px: 16 + 3 = 19),
/// while egui's line height keeps the fraction (19.3). Cached per font; None when GDI cannot create the font.
pub fn gdi_text_height(face: &str, pixels: i32, style: u8) -> Option<i32> {
    use std::{collections::HashMap, sync::Mutex};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateFontW, DeleteObject, GetDC, GetTextMetricsW, ReleaseDC, SelectObject, DEFAULT_CHARSET, TEXTMETRICW,
    };
    static CACHE: Mutex<Option<HashMap<(String, i32, u8), Option<i32>>>> = Mutex::new(None);
    let key = (face.to_owned(), pixels, style & 3);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(known) = cache.get_or_insert_with(HashMap::new).get(&key) {
        return *known;
    }
    let name = wide(face);
    let height = (!face.contains('\0') && pixels > 0).then(|| {
        // SAFETY: NUL-terminated face name; every GDI object and the screen DC are released before returning.
        unsafe {
            let font = CreateFontW(-pixels, 0, 0, 0, if style & 1 != 0 { 700 } else { 400 }, (style & 2 != 0) as u32, 0, 0, DEFAULT_CHARSET as u32, 0, 0, 0, 0, name.as_ptr());
            if font.is_null() {
                return None;
            }
            let dc = GetDC(ptr::null_mut());
            let old = SelectObject(dc, font);
            let mut metrics: TEXTMETRICW = std::mem::zeroed();
            let ok = GetTextMetricsW(dc, &mut metrics) != 0;
            SelectObject(dc, old);
            ReleaseDC(ptr::null_mut(), dc);
            DeleteObject(font);
            (ok && metrics.tmHeight > 0).then_some(metrics.tmHeight)
        }
    }).flatten();
    cache.get_or_insert_with(HashMap::new).insert(key, height);
    height
}

/// VCL TCanvas.TextWidth (GetTextExtentPoint32W, with the font's system font links for Korean and other scripts) of each text, for the font `face`
/// at `pixels` em height and `style` (bit 1 bold, bit 2 italic). None when GDI cannot create the font.
/// GDI text extents do not kern, so a text's width is the sum of its characters' widths; those are measured once per font and character and
/// cached (measuring every string took about 24 µs each with font fallback, 2.4 s for 100,000 names).
/// The original list layout (GetFieldMaxWidth, GetAdjustedNameWidth, column counts) is built on these widths.
pub fn gdi_text_widths<S: AsRef<str>>(face: &str, pixels: i32, style: u8, texts: &[S]) -> Option<Vec<i32>> {
    use std::{collections::HashMap, sync::Mutex};
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::{CreateFontW, DeleteObject, GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject, DEFAULT_CHARSET, HDC, HFONT};
    type Widths = HashMap<char, i32>;
    static CACHE: Mutex<Option<HashMap<(String, i32, u8), Option<Widths>>>> = Mutex::new(None);
    if face.contains('\0') || pixels <= 0 {
        return None;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let fonts = cache.get_or_insert_with(HashMap::new);
    let widths = fonts.entry((face.to_owned(), pixels, style & 3)).or_insert_with(|| Some(HashMap::new())).as_mut()?;
    // The screen DC with the font selected, created only when a character is missing from the cache.
    let mut gdi: Option<(HDC, HFONT, *mut core::ffi::c_void)> = None;
    let mut failed = false;
    let name = wide(face);
    let mut result = Vec::with_capacity(texts.len());
    for text in texts {
        let mut total = 0;
        for ch in text.as_ref().chars() {
            if let Some(width) = widths.get(&ch) {
                total += width;
                continue;
            }
            // SAFETY: NUL-terminated face name; the font and the screen DC are released after the loop; the character is passed with its UTF-16 length.
            let width = unsafe {
                if gdi.is_none() && !failed {
                    let font = CreateFontW(-pixels, 0, 0, 0, if style & 1 != 0 { 700 } else { 400 }, (style & 2 != 0) as u32, 0, 0, DEFAULT_CHARSET as u32, 0, 0, 0, 0, name.as_ptr());
                    if font.is_null() {
                        failed = true;
                    } else {
                        let dc = GetDC(ptr::null_mut());
                        let old = SelectObject(dc, font);
                        gdi = Some((dc, font, old));
                    }
                }
                match gdi {
                    Some((dc, _, _)) => {
                        let mut units = [0u16; 2];
                        let units = ch.encode_utf16(&mut units);
                        let mut size = SIZE { cx: 0, cy: 0 };
                        if GetTextExtentPoint32W(dc, units.as_ptr(), units.len() as i32, &mut size) == 0 { 0 } else { size.cx }
                    }
                    None => 0,
                }
            };
            if failed {
                break;
            }
            widths.insert(ch, width);
            total += width;
        }
        if failed {
            break;
        }
        result.push(total);
    }
    if let Some((dc, font, old)) = gdi {
        // SAFETY: restores the DC's previous font, then releases the DC and deletes the font created above.
        unsafe {
            SelectObject(dc, old);
            ReleaseDC(ptr::null_mut(), dc);
            DeleteObject(font);
        }
    }
    if failed {
        *fonts.get_mut(&(face.to_owned(), pixels, style & 3)).unwrap() = None;
        return None;
    }
    Some(result)
}

/// The ".." entry the file system reports in FindFirstFileW(folder\*), as original GetList (0x45a34c) adds it: its dwFileAttributes and
/// ftLastWriteTime (NTFS reports the parent folder there, so below a drive root it carries the root's hidden and system bits).
/// Stops at that entry. None when the listing has no ".." (the original then adds its own ".." with attribute 0x10 and GetFolderTime).
pub fn dot_dot_entry(folder: &Path) -> Option<(u32, Option<std::time::SystemTime>)> {
    let pattern = folder.join("*");
    reject_nul(&pattern.to_string_lossy()).ok()?;
    let dot_dot: Vec<u16> = "..".encode_utf16().collect();
    // SAFETY: NUL-terminated pattern; the find handle is closed before returning.
    unsafe {
        let mut data: WIN32_FIND_DATAW = std::mem::zeroed();
        let handle = FindFirstFileW(wide(&pattern).as_ptr(), &mut data);
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut found = None;
        loop {
            let length = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
            if data.cFileName[..length] == dot_dot[..] {
                let ticks = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32) | data.ftLastWriteTime.dwLowDateTime as u64;
                let time = ticks
                    .checked_sub(116_444_736_000_000_000)
                    .map(|since| std::time::UNIX_EPOCH + std::time::Duration::from_nanos(since.saturating_mul(100)));
                found = Some((data.dwFileAttributes, time));
                break;
            }
            if FindNextFileW(handle, &mut data) == 0 {
                break;
            }
        }
        FindClose(handle);
        found
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

/// xilesShell_ExecuteFile 0x7b4394 (called by original FUN_00431dd4): ShellExecuteExW(file, arguments, working folder, SW_SHOWNORMAL), verb NULL
/// (default verb), fMask SEE_MASK_UNICODE (0x4000; with the wait flag also SEE_MASK_NOCLOSEPROCESS and a message loop until the process ends).
/// Without SEE_MASK_FLAG_NO_UI, the shell reports failures itself.
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

/// Original PlaySoundW(path, 0, 0x22003 = SND_FILENAME|SND_NOWAIT|SND_NODEFAULT|SND_ASYNC). None stops the current sound like the original PlaySoundW(NULL, …).
/// Returns false if the file sound cannot be played.
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

/// Delphi Mouse.WheelScrollLines (SPI_GETWHEELSCROLLLINES). Falls back to the Windows default of 3 if it cannot be read.
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
/// Original FUN_0040d354 (copy name/path/description and so on): OpenClipboard, EmptyClipboard, a GMEM_MOVEABLE copy as CF_UNICODETEXT.
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
        put_clipboard(format, &effect.to_ne_bytes())?;
        // Original TNXDropFileSource::DoGetData 0x7b0d48 (set on the clipboard by CutOrCopyToClipboard 0x7b1440) also offers
        // CFSTR_SHELLIDLIST, built by FUN_007b09f4 from the first item's folder and each name. Optional: skipped when it cannot be built.
        if let Some(list) = shell_id_list(paths) {
            let format = RegisterClipboardFormatW(wide("Shell IDList Array").as_ptr());
            if format != 0 {
                let _ = put_clipboard(format, &list);
            }
        }
        Ok(())
    }
}

/// CIDA for "Shell IDList Array": the folder's absolute item id list, then one relative id list per item. Only when every item is in
/// the same folder (the original took the first item's folder for all of them, which named the wrong items otherwise).
fn shell_id_list(paths: &[PathBuf]) -> Option<Vec<u8>> {
    use windows_sys::Win32::UI::Shell::{Common::ITEMIDLIST, ILFindLastID, ILFree, ILGetSize, SHParseDisplayName};
    let parent = paths.first()?.parent()?;
    if paths.iter().any(|p| p.parent() != Some(parent)) {
        return None;
    }
    // SAFETY: SHParseDisplayName gets a NUL-terminated name; every returned id list is freed once with ILFree below.
    let parse = |path: &Path| -> Option<*mut ITEMIDLIST> {
        let name = wide(path);
        let mut pidl: *mut ITEMIDLIST = ptr::null_mut();
        let ok = unsafe { SHParseDisplayName(name.as_ptr(), ptr::null_mut(), &mut pidl, 0, ptr::null_mut()) } == 0;
        (ok && !pidl.is_null()).then_some(pidl)
    };
    let mut owned = Vec::new();
    let result = (|| {
        let folder = parse(parent)?;
        owned.push(folder);
        let mut blocks = Vec::with_capacity(paths.len() + 1);
        // SAFETY: the id lists are valid until freed; ILGetSize counts the terminating zero id.
        unsafe {
            blocks.push(std::slice::from_raw_parts(folder as *const u8, ILGetSize(folder) as usize).to_vec());
            for path in paths {
                let item = parse(path)?;
                owned.push(item);
                let last = ILFindLastID(item);
                blocks.push(std::slice::from_raw_parts(last as *const u8, ILGetSize(last) as usize).to_vec());
            }
        }
        let mut offset = 4 * (blocks.len() + 1);
        let mut data = (paths.len() as u32).to_ne_bytes().to_vec();
        for block in &blocks {
            data.extend((offset as u32).to_ne_bytes());
            offset += block.len();
        }
        for block in blocks {
            data.extend(block);
        }
        Some(data)
    })();
    for pidl in owned {
        // SAFETY: allocated by SHParseDisplayName above and freed once.
        unsafe { ILFree(pidl) };
    }
    result
}
/// Lets NDR detect when another application replaces an archive clipboard.
pub fn clipboard_sequence_number() -> u32 {
    // SAFETY: this Win32 query takes no arguments or pointers.
    unsafe { GetClipboardSequenceNumber() }
}

/// Paste reads CF_HDROP and Preferred DropEffect. The original drop target (TNXDropFileTarget DoGetData 0x7b2920) takes CF_HDROP first and then
/// NUL-separated Unicode and ANSI name lists from its other formats; window drops arrive here through winit as CF_HDROP paths.
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
/// Stands in for the original SHChangeNotify handling (FUN_00442bc8: SHCNE_DRIVEADD → TNXDriveList.Add, SHCNE_DRIVEREMOVED → Delete, media insert/eject → Update,
/// after which FUN_00442d80 redraws each pane's drive bar, buttons and info bar): a separate thread checks the GetLogicalDrives bits every second and
/// calls `on_change` when they change (the UI only wakes on a change). Stops when `stop` is set.
/// Add (0x7b7948) and Update (0x7b7ad8) both add an unknown root whose GetDriveTypeW is not 0/1 and is in the list's type set, then re-sort the
/// list (comparator 0x7b7924); a known root is refreshed (TNXDriveInfo.Refresh). Re-reading the whole list gives the same result.
pub fn spawn_drive_watch(stop: std::sync::Arc<std::sync::atomic::AtomicBool>, on_change: impl Fn() + Send + 'static) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        // SAFETY: GetLogicalDrives takes no arguments and only reads system state.
        let mut last = unsafe { GetLogicalDrives() };
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            // SAFETY: as above.
            let now = unsafe { GetLogicalDrives() };
            if now != last {
                last = now;
                on_change();
            }
        }
    });
}
/// C# DriveInfo.GetDrives().Where(IsReady): only drives whose volume information can be read.
pub fn ready_drives() -> Vec<PathBuf> {
    drives().into_iter().filter(|root| volume_label(root).is_some()).collect()
}
/// Original TNXDriveList.Refresh (0x7b7df0): among A~Z from GetLogicalDrives, those whose GetDriveTypeW is not 0 (unknown) or 1 (no root).
/// CD and card readers without media are included too.
pub fn listed_drives() -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    drives()
        .into_iter()
        // SAFETY: NUL-terminated root string.
        .filter(|root| unsafe { GetDriveTypeW(wide(root.as_os_str()).as_ptr()) } > 1)
        .collect()
}
/// Original FUN_004351cc: SetVolumeLabelW(root, new name). Returns GetLastError on failure.
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

/// C# RecycleBinService.Empty: all drives, with no confirmation, progress window or sound (SHERB_NOCONFIRMATION | NOPROGRESSUI | NOSOUND).
pub fn empty_recycle_bin() -> Result<(), String> {
    // SAFETY: null owner and root mean every drive; no pointers are kept.
    let code = unsafe { SHEmptyRecycleBinW(ptr::null_mut(), ptr::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) };
    if code < 0 {
        Err(format!("Recycle bin error: 0x{code:08x}"))
    } else {
        Ok(())
    }
}

/// Original miEmptyRecycleBinClick (0x433094): SHEmptyRecycleBinW(window, NULL, 0) — the shell shows the confirmation/progress window.
pub fn empty_recycle_bin_with_shell_ui() -> Result<(), String> {
    // SAFETY: null owner and root mean every drive; no pointers are kept.
    let code = unsafe { SHEmptyRecycleBinW(ptr::null_mut(), ptr::null(), 0) };
    if code < 0 {
        Err(format!("Recycle bin error: 0x{code:08x}"))
    } else {
        Ok(())
    }
}

/// C# WNetAddConnection2(Type 1, LocalName, RemoteName, password, user, flags 0). Failure is (Windows error code, system message).
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

/// Those with DriveType.Network from DriveInfo.GetDrives() (in "Z:" form, trailing '\' removed).
pub fn network_drives() -> Vec<String> {
    drives().into_iter().filter(|d| is_network_drive(d)).map(|d| d.to_string_lossy().trim_end_matches('\\').to_owned()).collect()
}

const EXPLORER_FOLDER_KEY: &str = "Software\\Classes\\Directory\\shell\\NDR";
const EXPLORER_BACKGROUND_KEY: &str = "Software\\Classes\\Directory\\Background\\shell\\NDR";
/// Keys used by older Rust builds (removed alongside when disabling).
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

/// Default value of HKEY_CLASSES_ROOT\`key` (REG_SZ / REG_EXPAND_SZ, not expanded).
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

/// Whether HKEY_CLASSES_ROOT\`key` exists.
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

/// Whether the shell icon differs per file: DefaultIcon is "%1" (with the quotes) or a shellex\IconHandler exists.
pub fn per_file_icon_rule(default_icon: Option<&str>, icon_handler: bool) -> bool {
    icon_handler || default_icon.is_some_and(|icon| icon.trim().trim_matches('"') == "%1")
}

/// The original image thread (FUN_007dd51c) asks the shell for every local item's icon by its actual path (SHGFI_SYSICONINDEX | SHGFI_ICON |
/// SHGFI_OVERLAYINDEX); until it arrives DrawListItem draws GetIconIndex (0x7b8d54) of the bare name (SHGFI_USEFILEATTRIBUTES). Without overlays,
/// NDR reads by actual path only the formats whose result varies per file: any of HKCR\.ext, its ProgID, or SystemFileAssociations\.ext matching
/// per_file_icon_rule.
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

/// ExplorerIntegrationService.IsEnabled: both keys have a non-empty command default value.
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

/// ExplorerIntegrationService.Enable/Disable (HKCU): folder "Browse with NDR" → "\"exe\" \"/dir:%1\"", folder background "Open folder in NDR" →
/// "\"exe\" \"/dir:%V\"", icon is the executable. Disabling removes both keys (and the older Rust-build keys).
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
/// Original image thread (TNXCustomFileListView StartImageThread 0x7defc8, Execute FUN_007de870): IExtractImage through the parent folder's
/// GetUIObjectOf (FUN_007de3b4, IEIFLAG_OFFLINE | IEIFLAG_SCREEN | IEIFLAG_ORIGSIZE, plus IEIFLAG_QUALITY above 120 px), larger results scaled
/// down with GDI+ (FUN_007dd994), 32-bit bitmaps without any alpha made opaque, and on failure the item's shell icon drawn into the cell
/// (FUN_007dd51c, FUN_007ddc60). Files outside .exe .lnk .ico .cur .ani .scr .jpg .jpeg .png .bmp .gif .ttc .ttf .otf (FUN_007de83c) that fell
/// back to the icon share it per extension; here every file asks the shell, which gives the same icon for those.
/// IShellItemImageFactory (SIIGBF_RESIZETOFIT) covers both steps.
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
    // One extraction at a time, like the original's single image thread (FUN_007de870): concurrent GetImage calls from the per-item workers
    // returned E_PENDING (0x8000000A) for executables, documents and drive roots, which then showed "[FILE]"/"[DIR]" in the thumbnail view.
    // E_PENDING is also retried a few times.
    static SHELL_THUMBNAILS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = SHELL_THUMBNAILS.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let image = (|| -> Result<_, String> {
            let factory: IShellItemImageFactory =
                SHCreateItemFromParsingName(PCWSTR(wide(path).as_ptr()), None)
                    .map_err(|e| e.to_string())?;
            let mut tries = 0;
            loop {
                match factory.GetImage(SIZE { cx: side, cy: side }, SIIGBF_BIGGERSIZEOK | SIIGBF_RESIZETOFIT) {
                    Err(e) if e.code().0 as u32 == 0x8000_000A && tries < 20 => {
                        tries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    result => break result.map_err(|e| e.to_string()),
                }
            }
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

/// Like Image1 in the original TfMsgBox, reads a system icon with LoadIconW(NULL, id) and returns it as 32x32 RGBA.
/// id is IDI_HAND 0x7f01, IDI_QUESTION 0x7f02, IDI_EXCLAMATION 0x7f03, IDI_ASTERISK 0x7f04.
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

/// C# FileVisualConverter.LoadIcon: the small icon from SHGetFileInfoW(path, or "folder"/"." if missing, folder/normal attributes,
/// SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES) as 16x16 RGBA. The file is not read (by extension/attributes only).
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

/// Original FUN_00422314 (TfFileExists imgOrg/imgNew): the large icon from SHGetFileInfoW(path, 0, SHGFI_ICON, plus | SHGFI_USEFILEATTRIBUTES for a missing path)
/// as 32x32 RGBA.
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

/// Whether this is "X:\" (a drive root).
fn is_drive_root(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let chars: Vec<char> = text.chars().collect();
    chars.len() == 3 && chars[0].is_ascii_alphabetic() && chars[1] == ':' && chars[2] == '\\'
}

/// Independent fallback for the generic folder class. Unlike a shell lookup
/// for an arbitrary path, the stock SIID_FOLDER id cannot accidentally return
/// a drive/virtual-folder icon or trigger a third-party icon handler.
pub fn shell_stock_folder_icon() -> Option<(usize, usize, Vec<u8>)> {
    use windows_sys::Win32::UI::Shell::{SHGSI_ICON, SHGSI_SMALLICON, SHGetStockIconInfo, SHSTOCKICONINFO, SIID_FOLDER};
    let mut info: SHSTOCKICONINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHSTOCKICONINFO>() as u32;
    unsafe {
        if SHGetStockIconInfo(SIID_FOLDER, SHGSI_ICON | SHGSI_SMALLICON, &mut info) < 0 || info.hIcon.is_null() {
            return None;
        }
        let pixels = icon_rgba(info.hIcon, 16).ok();
        DestroyIcon(info.hIcon);
        pixels
    }
}

/// The Windows default "network drive" small icon (SHGetStockIconInfo SIID_DRIVENET).
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

/// C# FileVisualConverter.LoadShellOverlayIcon: SHGetFileInfoW(actual path, 0, SHGFI_ICON | SHGFI_SMALLICON | SHGFI_ADDOVERLAYS).
/// The shell actually inspects the item (shell extensions included). Used on a separate thread that called com_sta_init.
pub fn shell_overlay_icon(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    // Fix: the shell gives a "disconnected" icon to network drives whose volume can be read (WSL 9P, etc.), so use the Windows network-drive icon instead.
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

/// File item icon by actual path: TNXShellImageList.GetIconIndex (0x7b8d54) with an absolute path is SHGetFileInfoW(path, attributes,
/// SHGFI_SYSICONINDEX) (a relative name adds SHGFI_USEFILEATTRIBUTES), the file's own icon (executable, shortcut, etc.). Overlays are not added.
/// Used on a separate thread that called com_sta_init.
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

/// Shell item icon for a pane folder tree (TNXShellTreeView) entry: converts the key (empty key = desktop, "::{CLSID}" virtual folder, or a path)
/// into an item id list with SHParseDisplayName, then SHGetFileInfoW(SHGFI_PIDL | SHGFI_ICON | SHGFI_SMALLICON). Used on a thread that called com_sta_init.
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

/// Memory-maps a file that is only read until the program exits, such as a font file (CreateFileMappingW / MapViewOfFile), and returns it.
/// Fix (memory): previously a font file was read whole (Vec) and egui copied it again, so Malgun Gothic (12.8MB) and Bold (12MB) each existed twice
/// in private memory. A mapped file is shared with the OS file cache (only the needed parts are read). The same file is mapped only once.
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

/// Makes this thread an STA for the shell (the ApartmentState.STA of the C# overlay worker thread). Leaves it that way until the thread ends.
pub fn com_sta_init() {
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    // SAFETY: plain COM apartment initialisation for the calling thread.
    unsafe {
        CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }
}

/// Original miAddToFavoritesClick (0x43371d): runs DrawTextW(path, rect 0,0,width,30, DT_PATH_ELLIPSIS | DT_MODIFYSTRING | DT_NOPREFIX | DT_SINGLELINE = 0x14820)
/// with the bold form font, then uses the modified text (with "..." in the middle). On failure returns it unchanged.
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

/// Draws an HICON at side x side into straight (non-premultiplied) RGBA.
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

/// System random bytes (BCryptGenRandom, system default RNG). Same source as C# RandomNumberGenerator.
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

/// Environment.ExpandEnvironmentVariables: ExpandEnvironmentStringsW (%NAME% form).
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

/// DriveInfo.DriveType name (GetDriveTypeW): Unknown, NoRootDirectory, Removable, Fixed, Network, CDRom, Ram.
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

/// DriveInfo.VolumeLabel (GetVolumeInformationW). None if not ready.
pub fn volume_label(root: &Path) -> Option<String> {
    let mut name = vec![0u16; 261];
    // SAFETY: NUL-terminated root and a buffer of the stated length; the other outputs are optional.
    let ok = unsafe {
        GetVolumeInformationW(wide(root.as_os_str()).as_ptr(), name.as_mut_ptr(), name.len() as u32, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), 0)
    } != 0;
    ok.then(|| String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]))
}

/// Original FUN_004266ac: SHFileOperationW(FO_DELETE, pFrom = path, FOF_ALLOWUNDO 0x40 for the recycle bin, else 0) — the shell shows confirmation/progress. 0 = success.
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

/// Original xilesShell GetVolumeSerial: the volume serial number from GetVolumeInformationW (used in the tree file name FUN_007e3e7c). None if it cannot be read.
pub fn volume_serial(root: &Path) -> Option<u32> {
    let mut serial = 0u32;
    // SAFETY: NUL-terminated root; only the serial output is requested.
    let ok = unsafe { GetVolumeInformationW(wide(root.as_os_str()).as_ptr(), ptr::null_mut(), 0, &mut serial, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), 0) } != 0;
    ok.then_some(serial)
}

/// Whether InputLanguageManager.CurrentInputLanguage is English (the primary language of GetKeyboardLayout(0) = LANG_ENGLISH).
pub fn english_input_language() -> bool {
    // SAFETY: GetKeyboardLayout(0) reads the calling thread's layout.
    let layout = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout(0) } as usize;
    (layout & 0x3ff) == 0x09
}

/// shlwapi StrToIntExW(text, 0 = STIF_DEFAULT): the value if it parses as a decimal.
pub fn str_to_int_ex(text: &str) -> Option<i32> {
    let mut value = 0i32;
    // SAFETY: NUL-terminated input and a valid output pointer.
    (unsafe { StrToIntExW(wide(text).as_ptr(), 0, &mut value) } != 0).then_some(value)
}

/// DriveInfo.DriveType == Fixed: GetDriveTypeW(root) == DRIVE_FIXED.
pub fn is_fixed_drive(root: &Path) -> bool {
    // SAFETY: NUL-terminated root path.
    unsafe { GetDriveTypeW(wide(root.as_os_str()).as_ptr()) == 3 }
}

/// Environment.GetFolderPath(SpecialFolder.InternetCache / Cookies): SHGetKnownFolderPath(flags 0, None if absent).
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

/// DriveInfo.DriveType == Network: GetDriveTypeW(root) == DRIVE_REMOTE.
pub fn is_network_drive(root: &Path) -> bool {
    // SAFETY: NUL-terminated root path.
    unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide(root.as_os_str()).as_ptr()) == 4 }
}

/// C# FileVersionInfo: ProductMajorPart (FileMajorPart if 0). None if there is no version info.
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

/// The DriveInfo of C# UpdateDiskCapacityAsync: only "X:\" drives, None for network and not-ready drives. (usable free, total).
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

/// The name of a connected network drive: the connection's target UNC path as-is ("\\server\share\sub", trailing '\' removed).
/// Used instead for connections without a volume label (9P, etc.) that the shell shows as a "disconnected network drive". User request: previously it was "sub (\\server\share)" like Explorer.
pub fn network_drive_name(target: &str) -> Option<String> {
    let trimmed = target.trim_end_matches('\\');
    let rest = trimmed.strip_prefix("\\\\")?;
    let parts: Vec<&str> = rest.split('\\').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    Some(format!("\\\\{}", parts.join("\\")))
}

/// The target UNC of a drive letter: turns QueryDosDeviceW("X:")'s "\??\UNC\server\share…" or
/// "\Device\<redirector>\;X:<number>\server\share…" into "\\server\share…". None if it is not a network connection.
pub fn dos_device_unc(device: &str) -> Option<String> {
    if let Some(rest) = device.strip_prefix("\\??\\UNC\\") {
        return Some(format!("\\\\{rest}"));
    }
    let after = device.strip_prefix("\\Device\\")?;
    let (_, mapped) = after.split_once('\\')?;
    let rest = mapped.strip_prefix(';').and_then(|m| m.split_once('\\')).map_or(mapped, |(_, rest)| rest);
    (!rest.is_empty()).then(|| format!("\\\\{rest}"))
}

/// The network connection target (dos_device_unc) of a drive letter (root "X:\").
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

/// Whether this is a connected (volume-readable) network drive: GetDriveTypeW == DRIVE_REMOTE and GetVolumeInformationW succeeds.
pub fn connected_network_drive(root: &Path) -> bool {
    is_network_drive(root) && volume_label(root).is_some()
}

/// Original xilesShell GetDriveName (0x7b4a74): if not removable and ready, the volume label (GetVolumeInformationW);
/// if empty or removable, the shell display name (SHGetFileInfoW SHGFI_DISPLAYNAME) truncated at one character before the last '('.
/// Fix: for a network drive whose volume can be read, the name built from the connection target instead of the shell name ("disconnected network drive").
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

/// The TNXDriveInfo (read size = 0) of an original This PC drive item: (GetDriveName, GetDriveTypeW, (free, total) for a fixed disk (3)).
/// Refresh (0x7b7554) does not read the size if the type is 0 or 1, and reads only fixed disks when there is no display.
pub fn drive_item_info(root: &Path) -> Option<(String, u32, Option<(u64, u64)>)> {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};
    let name = wide(root.as_os_str());
    // SAFETY: NUL-terminated root string.
    let kind = unsafe { GetDriveTypeW(name.as_ptr()) };
    let (mut free, mut total) = (0u64, 0u64);
    // Fix (user request): the original reads the size only for fixed disks, but connected (volume-readable) network drives are read too.
    // Disconnected network drives can be slow, so they are not read, like the original.
    let label = drive_name(root);
    let connected = kind == 4 && connected_network_drive(root);
    // SAFETY: NUL-terminated root string; out pointers refer to live locals.
    let sizes = ((kind == 3 || connected) && unsafe { GetDiskFreeSpaceExW(name.as_ptr(), &mut free, &mut total, ptr::null_mut()) } != 0).then_some((free, total));
    Some((label, kind, sizes))
}

/// Original PathStripToRootW: "X:\" or "\\server\share\" (and the drive letter).
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

/// Material for the original info bar's (FUN_0048b500) TNXDriveInfo (root, read size = 1): (drive text, free, total).
/// TNXDriveInfo.Refresh (0x7b7554): GetDiskFreeSpaceExW if the type (GetDriveTypeW) is not 0 or 1. The text is "name " for network (4),
/// otherwise "name (C:) " (name = GetDriveName). If the size cannot be read it is 0 like the original (TNXDriveInfo starts at 0).
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

/// HTTPS GET (WinHTTP, 15-second timeout, blocks HTTPS → HTTP redirects, TooLarge if `max_bytes` is exceeded). Stands in for C# UpdateHttpClient.
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

/// Win32Exception(code).Message (without the trailing " (os error N)").
pub fn win32_message(code: u32) -> String {
    let text = std::io::Error::from_raw_os_error(code as i32).to_string();
    match text.rfind(" (os error ") {
        Some(at) => text[..at].trim_end().to_owned(),
        None => text,
    }
}

/// C# GetNetworkPathErrorCode: GetLastError if a UNC path cannot be read, otherwise 0.
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

/// C# PromptAndConnectNetworkShare: temporary connection to "\\server\share" via the Windows credential dialog (CONNECT_TEMPORARY | INTERACTIVE | PROMPT).
/// 67 (ERROR_BAD_NET_NAME) if there is no share name. The result is the Windows error number (0 = success, 1223 = cancelled).
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

/// Whether this window is currently the foreground window (GetForegroundWindow).
pub fn is_foreground(hwnd: isize) -> bool {
    // SAFETY: plain system call without pointers.
    unsafe { GetForegroundWindow() as isize == hwnd }
}

/// C# WindowOpacity: window alpha = clamp(WindowAlpha/255, 0.35, 1.0). Converts the stored alpha (0~255) into actual opacity (0.35~1.0).
pub fn window_opacity(window_alpha: f64) -> f32 {
    (window_alpha / 255.0).clamp(0.35, 1.0) as f32
}

/// Windows double-click thresholds: GetDoubleClickTime (ms) and SM_CXDOUBLECLK / SM_CYDOUBLECLK (pixels, a rectangle centred on the first click).
/// The original Delphi controls receive the WM_LBUTTONDBLCLK (ssDouble) that Windows sends by these thresholds. The defaults are 500ms and 4×4.
pub fn double_click_metrics() -> (u32, i32, i32) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK};
    // SAFETY: plain system queries without pointers.
    let (ms, cx, cy) = unsafe { (GetDoubleClickTime(), GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)) };
    (if ms == 0 { 500 } else { ms }, if cx <= 0 { 4 } else { cx }, if cy <= 0 { 4 } else { cy })
}

/// C# SystemParameters.VirtualScreen* (DIP): the combined screen of all monitors (x, y, width, height) in egui points.
/// Divides the GetSystemMetrics pixels by the system DPI (LOGPIXELSX / 96).
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

/// The work area (excluding the taskbar) of the monitor at (x, y) (egui points), in egui points (x, y, width, height).
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

/// Maximizes the window and sets the restore size (outer x, y, width, height, egui points) (SetWindowPlacement SW_SHOWMAXIMIZED).
pub fn maximize_with_restore(hwnd: isize, restore: (f32, f32, f32, f32)) -> bool {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPlacement, SW_SHOWMAXIMIZED, WINDOWPLACEMENT};
    if hwnd == 0 {
        return false;
    }
    let scale = system_dpi_scale();
    let (x, y, w, h) = restore;
    // rcNormalPosition is in work-area-relative coordinates.
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

/// The original TForm.Width/Height and C# Window.Width/Height (WindowWidth/WindowHeight) are the outer size including the border and title bar.
/// Measures a normal window's (WS_OVERLAPPEDWINDOW) outer size − inner (content) size with AdjustWindowRectEx and returns it in egui points (system DPI).
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

/// C# ApplyWindowBoundsFromSettings: Left = clamp(stored, screen left − width + 96, screen right − 96),
/// Top = clamp(stored, screen top − height + 64, screen bottom − 64). Keeps part of the window visible so it does not disappear off-screen.
pub fn clamp_window_position(left: f32, top: f32, width: f32, height: f32, screen: (f32, f32, f32, f32)) -> (f32, f32) {
    let (sx, sy, sw, sh) = screen;
    let clamp = |value: f32, low: f32, high: f32| if low > high { low } else { value.clamp(low, high) };
    (clamp(left, sx - width + 96.0, sx + sw - 96.0), clamp(top, sy - height + 64.0, sy + sh - 64.0))
}

/// C# WindowOpacity: applies the window alpha (0.0~1.0) directly (SetLayeredWindowAttributes). egui has no window-opacity command, so this
/// turns on WS_EX_LAYERED via the native HWND and sets the alpha. 1.0 stays fully opaque (255).
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


/// Original TColorDialog.Execute (the preferences color side): the Windows color picker (ChooseColorW, initial color = initial). COLORREF (0x00BBGGRR).
/// The 16 custom-color slots are remembered while the program runs (stands in for Delphi TColorDialog.CustomColors). None on cancel. It is modal.
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
    // Original DFM: ColorDialog Options={cdFullOpen}.
    choose.Flags = CC_RGBINIT | CC_ANYCOLOR | CC_FULLOPEN;
    // SAFETY: the struct and the 16-entry custom colour array outlive the modal call.
    (unsafe { ChooseColorW(&mut choose) } != 0).then_some(choose.rgbResult & 0x00FF_FFFF)
}

/// Original btnFontClick (TFontDialog.Execute, initial Options = fdEffects): the Windows font dialog (ChooseFontW).
/// Initial value = (name, points, original TFontStyles byte: 1 bold, 2 italic, 4 underline, 8 strikeout). On OK the same set. It is modal.
pub fn choose_font(name: &str, points: f32, style: u8) -> Option<(String, f32, u8)> {
    use windows_sys::Win32::Graphics::Gdi::LOGFONTW;
    use windows_sys::Win32::UI::Controls::Dialogs::{CF_EFFECTS, CF_FORCEFONTEXIST, CF_INITTOLOGFONTSTRUCT, CF_LIMITSIZE, CF_NOVERTFONTS, CF_SCREENFONTS, CHOOSEFONTW, ChooseFontW};
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
    // Original btnFontClick (0x44e668): MinFontSize 6, MaxFontSize 30, Options + fdForceFontExist + fdLimitSize (CF_FORCEFONTEXIST, CF_LIMITSIZE).
    // Previously any size could be chosen.
    choose.nSizeMin = 6;
    choose.nSizeMax = 30;
    choose.Flags = CF_INITTOLOGFONTSTRUCT | CF_SCREENFONTS | CF_NOVERTFONTS | CF_EFFECTS | CF_FORCEFONTEXIST | CF_LIMITSIZE;
    // SAFETY: the LOGFONTW lives across the modal call.
    if unsafe { ChooseFontW(&mut choose) } == 0 {
        return None;
    }
    let length = font.lfFaceName.iter().position(|c| *c == 0).unwrap_or(font.lfFaceName.len());
    let style = u8::from(font.lfWeight >= 700) | u8::from(font.lfItalic != 0) << 1 | u8::from(font.lfUnderline != 0) << 2 | u8::from(font.lfStrikeOut != 0) << 3;
    Some((String::from_utf16_lossy(&font.lfFaceName[..length]), choose.iPointSize as f32 / 10.0, style))
}

/// Font registry name candidates matching the original TFontStyles (1 bold, 2 italic): from the styled name ("Segoe UI Bold Italic") toward nearer styles, ending with the regular face.
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

/// The mouse position in screen coordinates (physical pixels). Used because the egui pointer position is stale when dropping from a shell drag / Explorer.
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: the out pointer refers to a live local.
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

/// GetKeyState(VK_CAPITAL) != 0 from the original WMChar (0x7d5a18) (Caps Lock is on or held).
pub fn caps_lock_state() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
    // SAFETY: plain system call without pointers.
    unsafe { GetKeyState(VK_CAPITAL as i32) != 0 }
}

/// The Ctrl/Shift state at drop time (C# DragEventArgs.KeyStates). (ctrl, shift).
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

/// C# FileList_PreviewMouseMove's DoDragDrop(FileDrop, Copy | Move | Link): drags out of NDR (to Explorer, etc.) via a shell data object.
/// The drop target does the copy/move/shortcut. Returns the resulting effect (DROPEFFECT bits, 0 = cancelled).
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

/// C# OriginalShortcutService.ExistingTargetFolder (original FUN_00441af4): SHGetSpecialFolderPathW(CSIDL_DESKTOPDIRECTORY 0x10)
/// or CSIDL_APPDATA (0x1a) + "Microsoft\Internet Explorer\Quick Launch". Only when the folder exists (does not create it).
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

/// C# OriginalShortcutService.LinkPath: PathAppendW(folder, PathStripPathW(target)) + ".lnk" — keeps the target's extension.
pub fn original_link_path(target: &Path, folder: &Path) -> PathBuf {
    let name = crate::names::find_file_name(&target.to_string_lossy());
    let mut link = folder.join(name).into_os_string();
    link.push(".lnk");
    PathBuf::from(link)
}

/// GetLongPathNameW (short name "PROGRA~1" to long name). None on failure.
pub fn long_path(path: &Path) -> Option<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let short = wide(path);
    let mut long = vec![0u16; 0x400];
    // SAFETY: NUL-terminated input; the output length is passed.
    let length = unsafe { GetLongPathNameW(short.as_ptr(), long.as_mut_ptr(), long.len() as u32) } as usize;
    (length > 0 && length < long.len()).then(|| PathBuf::from(String::from_utf16_lossy(&long[..length])))
}

/// Original GetShortcutTarget(shortcut, &WIN32_FIND_DATAW): reads an IShellLinkW (IPersistFile::Load) and returns the target path (GetPath) and
/// whether that find data's attributes are a folder (FILE_ATTRIBUTE_DIRECTORY). None if it cannot be read.
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

/// C# OriginalShortcutService.Create (original @CreateShortcut 0x7b3d04): SetPath(target), SetArguments(""),
/// SetWorkingDirectory(PathRemoveFileSpecW(target)), SetIconLocation(NULL, 1), SetHotkey(0), SetShowCmd(0),
/// IPersistFile::Save(link, TRUE) — silently replaces an existing link. The original caller does not check the result.
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

/// Environment.GetFolderPath(Desktop/MyDocuments/MyPictures/MyMusic/MyVideos) = the corresponding known folder.
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

/// A Windows system color (GetSysColor, e.g. 13 COLOR_HIGHLIGHT, 14 COLOR_HIGHLIGHTTEXT) as (R, G, B).
pub fn system_color(index: i32) -> (u8, u8, u8) {
    // SAFETY: GetSysColor only reads a colour table entry.
    let value = unsafe { windows_sys::Win32::Graphics::Gdi::GetSysColor(index as _) };
    ((value & 0xff) as u8, ((value >> 8) & 0xff) as u8, ((value >> 16) & 0xff) as u8)
}

/// C# FileInformationService.GetShellTypeName: SHGetFileInfoW("folder" | "file" | "file.ext", folder/normal attributes,
/// SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES). Empty string on failure.
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

/// C# IsWindowsLightTheme: HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize's AppsUseLightTheme (1 if absent) != 0.
/// false if it cannot be read. (C#'s high-contrast branch was not ported.)
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
        2 => true, // ERROR_FILE_NOT_FOUND: GetValue default is 1
        _ => false,
    }
}

/// Original pmEditPopup: IsClipboardFormatAvailable(CF_HDROP = 15). Does not open the clipboard.
pub fn clipboard_has_files() -> bool {
    // SAFETY: no pointers; only queries the clipboard format list.
    unsafe { IsClipboardFormatAvailable(15) != 0 }
}

/// Original miEmptyTempClick: GetTempPathW(0x400) → GetLongPathNameW(0x400). None if either one fails.
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

/// StrFormatByteSizeW(size, 0x80 slots) — the shell format the original uses for size text ("1.50 MB", "512 bytes").
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

/// PathMatchSpecW(file name, single filter) == 1 from the original advanced selection (FUN_00438034).
pub fn path_match_spec(file: &str, spec: &str) -> bool {
    let (file, spec) = (wide(file), wide(spec));
    // SAFETY: both arguments are NUL-terminated UTF-16 buffers that outlive the call.
    unsafe { PathMatchSpecW(file.as_ptr(), spec.as_ptr()) == 1 }
}

/// .NET FileAttributes.ToString(): known flag names from smallest value, joined with ", ". If unknown bits remain, the number; "0" if 0.
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

/// From a font registry value name ("Segoe UI (TrueType)", "Cambria & Cambria Math (TrueType)"), picks the regular face of `family`.
/// If `family` appears verbatim in the list split by " & ", returns (value name, its index = TTC number). Other style names like bold/italic do not match.
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

/// The font file for the original/C# list font (ListFont) name: from the values of HKCU/HKLM "Software\Microsoft\Windows NT\CurrentVersion\Fonts",
/// the file picked by match_font_entry (relative names resolve to %WINDIR%\Fonts) and the TTC number.
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

    /// Original DoGetData 0x7b0d48 CFSTR_SHELLIDLIST: count, count + 1 offsets, the folder id list, then one relative id list per item;
    /// items from different folders give none.
    /// VCL Canvas.TextHeight = GDI tmHeight: Tahoma at 16 px is 19 (rows of the captured original are 19 + 2 = 21 px apart).
    #[test]
    fn gdi_text_height_matches_the_original_row_pitch() {
        assert_eq!(super::gdi_text_height("Tahoma", 16, 0), Some(19));
        assert_eq!(super::gdi_text_height("Tahoma", 16, 0), Some(19), "cached");
        assert_eq!(super::gdi_text_height("Tahoma", 0, 0), None);
    }
    /// Canvas.TextWidth: GDI does not kern, so a text is the sum of its characters; Korean comes from the linked font; empty is 0.
    #[test]
    fn gdi_text_widths_sum_the_characters() {
        let widths = super::gdi_text_widths("Tahoma", 16, 0, &["W", "WWWWW", "", "한글", "한"]).unwrap();
        assert_eq!(widths[1], widths[0] * 5);
        assert_eq!(widths[2], 0);
        assert!(widths[4] > 0 && widths[3] > widths[4], "{widths:?}");
        assert_eq!(super::gdi_text_widths("Tahoma", 0, 0, &["W"]), None);
    }
    /// Original GetList (0x45a34c): ".." comes from FindFirstFileW(folder\*); on NTFS it describes the parent folder.
    #[test]
    fn dot_dot_entry_reports_the_parent_folder() {
        let root = std::env::temp_dir().join(format!("ndr-dotdot-{}", std::process::id()));
        let child = root.join("child");
        std::fs::create_dir_all(&child).unwrap();
        let (attributes, time) = super::dot_dot_entry(&child).expect("NTFS lists ..");
        assert_ne!(attributes & 0x10, 0);
        let parent_time = std::fs::metadata(&root).unwrap().modified().unwrap();
        let gap = time.unwrap().duration_since(parent_time).unwrap_or_else(|e| e.duration());
        assert!(gap.as_secs() < 2, "{gap:?}");
        let _ = std::fs::remove_dir_all(root);
    }
    /// A generic folder icon must be available independently of file-system
    /// paths. This is the last-resort fallback for an archive/FTP pane when
    /// the shell handler temporarily fails to load an extension icon.
    #[test]
    fn windows_stock_folder_icon_is_visible_and_has_valid_alpha() {
        // GitHub-hosted service runners have no interactive Explorer desktop;
        // SHGetStockIconInfo may return no bitmap there. Keep this visual
        // integration assertion active on normal Windows developer machines.
        if std::env::var_os("GITHUB_ACTIONS").is_some() {
            return;
        }
        let (width, height, rgba) = super::shell_stock_folder_icon().expect("stock folder shell icon");
        assert_eq!((width, height), (16, 16));
        assert_eq!(rgba.len(), width * height * 4);
        assert!(rgba.chunks_exact(4).any(|p| p[3] >= 128), "stock icon cannot be fully transparent");
    }

    /// Thumbnails asked from several workers at once all arrive (the shell answered E_PENDING to concurrent GetImage calls before they were serialized).
    #[test]
    fn concurrent_thumbnails_all_arrive() {
        let root = std::env::temp_dir().join(format!("ndr-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder")).unwrap();
        let mut paths = vec![root.join("folder"), std::path::PathBuf::from("C:\\")];
        for name in ["a.txt", "b.json", "c.ini", "d.csv", "e.log", "f.xml"] {
            std::fs::write(root.join(name), b"x").unwrap();
            paths.push(root.join(name));
        }
        let handles: Vec<_> = paths.into_iter().map(|p| std::thread::spawn(move || (super::thumbnail(&p, 80).map(|(w, h, _)| (w, h)), p))).collect();
        for h in handles {
            let (result, path) = h.join().unwrap();
            assert!(result.is_ok(), "{path:?}: {result:?}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn shell_id_list_follows_the_cida_layout() {
        use std::path::PathBuf;
        let root = std::env::temp_dir().join(format!("ndr-cida-{}", std::process::id()));
        let other = root.join("sub");
        std::fs::create_dir_all(&other).unwrap();
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let list = super::shell_id_list(&[a.clone(), b.clone()]).expect("a CIDA");
        let word = |at: usize| u32::from_ne_bytes(list[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(word(0), 2);
        let offsets = [word(4), word(8), word(12)];
        assert_eq!(offsets[0], 16, "after cidl and three offsets");
        assert!(offsets[0] < offsets[1] && offsets[1] < offsets[2] && offsets[2] < list.len(), "{offsets:?}");
        // Each relative id list is one id (a size word, data) plus the terminating zero id.
        let child = &list[offsets[1]..offsets[2]];
        let first = u16::from_ne_bytes([child[0], child[1]]) as usize;
        assert_eq!(&child[first..first + 2], &[0, 0]);
        assert_eq!(super::shell_id_list(&[a, other.join("missing.txt"), PathBuf::from(r"C:\")]), None);
        let _ = std::fs::remove_dir_all(root);
    }

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
        // Clamps to 0.35 at the low end (never fully transparent).
        assert_eq!(window_opacity(0.0), 0.35);
        assert_eq!(window_opacity(80.0), 0.35);
        // Clamps to 1.0 at the high end.
        assert_eq!(window_opacity(300.0), 1.0);
    }

    #[test]
    fn connected_network_drives_get_a_name_from_their_target() {
        // "\??\UNC\…" form (e.g. 9P/WebDAV connections) and the SMB (LanmanRedirector) form.
        assert_eq!(dos_device_unc(r"\??\UNC\host\share\folder").as_deref(), Some(r"\\host\share\folder"));
        assert_eq!(dos_device_unc(r"\Device\LanmanRedirector\;Z:0000000000012345\server\share").as_deref(), Some(r"\\server\share"));
        assert_eq!(dos_device_unc(r"\Device\HarddiskVolume3"), None);
        // Name = the connection's target UNC path (trailing '\' removed). A server alone is not a name.
        assert_eq!(network_drive_name(r"\\host\share\folder").as_deref(), Some(r"\\host\share\folder"));
        assert_eq!(network_drive_name(r"\\server\share\").as_deref(), Some(r"\\server\share"));
        assert_eq!(network_drive_name(r"\\server"), None);
    }

    #[test]
    fn window_position_is_clamped_onto_the_screen_like_csharp() {
        let screen = (0.0, 0.0, 1920.0, 1080.0);
        // Unchanged if on-screen.
        assert_eq!(clamp_window_position(10.0, 10.0, 1450.0, 900.0, screen), (10.0, 10.0));
        // If off the right/bottom, keep 96×64 visible.
        assert_eq!(clamp_window_position(5000.0, 5000.0, 1450.0, 900.0, screen), (1824.0, 1016.0));
        // If off the left/top, keep 96 on the right / 64 on the bottom visible.
        assert_eq!(clamp_window_position(-5000.0, -5000.0, 1450.0, 900.0, screen), (-1354.0, -836.0));
        // A left monitor (negative coordinates) is unchanged if inside the virtual screen.
        assert_eq!(clamp_window_position(-1500.0, 100.0, 1450.0, 900.0, (-1920.0, 0.0, 3840.0, 1080.0)), (-1500.0, 100.0));
    }
}
