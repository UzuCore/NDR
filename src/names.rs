//! The shlwapi name and string functions the original uses (PathFindFileNameW, PathRemoveFileSpecW, StrStrIW, StrCmpNIW).
//! The same APIs are called directly instead of Rust to_lowercase to get the same case-insensitive rules as the original.
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
use windows_sys::Win32::UI::Shell::{PathFindFileNameW, PathRemoveFileSpecW, StrCmpNIW, StrStrIW};

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn from_wide_ptr(start: *const u16) -> String {
    if start.is_null() {
        return String::new();
    }
    let mut length = 0;
    // SAFETY: start points into a NUL-terminated buffer owned by the caller for the whole call.
    unsafe {
        while *start.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(start, length))
    }
}

/// PathFindFileNameW (original CopyPathToClipboard Tag 1).
pub fn find_file_name(path: &str) -> String {
    let buffer = wide(path);
    // SAFETY: buffer is NUL-terminated and outlives the call; the result points into it.
    from_wide_ptr(unsafe { PathFindFileNameW(buffer.as_ptr()) })
}

/// PathRemoveFileSpecW (original CopyPathToClipboard Tag 2): "C:\dir\file" -> "C:\dir", "C:\file" -> "C:\".
pub fn remove_file_spec(path: &str) -> String {
    let mut buffer = wide(path);
    // SAFETY: the buffer is writable, NUL-terminated and the API only shortens it.
    unsafe {
        PathRemoveFileSpecW(buffer.as_mut_ptr());
    }
    from_wide_ptr(buffer.as_ptr())
}

/// StrStrIW(value, search) != NULL. An empty search string gives NULL (not found).
pub fn contains_ignore_case(value: &str, search: &str) -> bool {
    let (value, search) = (wide(value), wide(search));
    // SAFETY: both buffers are NUL-terminated and live through the call.
    !unsafe { StrStrIW(value.as_ptr(), search.as_ptr()) }.is_null()
}

/// StrCmpNIW(prefix, value, len(prefix)) == 0 (by UTF-16 length).
pub fn starts_with_ignore_case(value: &str, prefix: &str) -> bool {
    let (value, prefix_wide) = (wide(value), wide(prefix));
    let count = (prefix_wide.len() - 1) as i32;
    // SAFETY: both buffers are NUL-terminated and live through the call.
    unsafe { StrCmpNIW(prefix_wide.as_ptr(), value.as_ptr(), count) == 0 }
}

/// CompareStringOrdinal(a, b, TRUE) == CSTR_EQUAL: the same ordinal case-insensitive comparison as the file system (in place of C# OrdinalIgnoreCase).
pub fn equals_ignore_case(a: &str, b: &str) -> bool {
    use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
    let (a, b) = (wide(a), wide(b));
    // SAFETY: explicit lengths without the terminating NUL; both buffers live through the call.
    unsafe { CompareStringOrdinal(a.as_ptr(), a.len() as i32 - 1, b.as_ptr(), b.len() as i32 - 1, 1) == CSTR_EQUAL }
}

fn last_error() -> u32 {
    // SAFETY: plain thread-local error read.
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

/// CreateDirectoryW: no intermediate folders, fails when the name exists. 0 = success, otherwise a Win32 error code.
pub fn create_directory(path: &str) -> u32 {
    let path = wide(path);
    // SAFETY: NUL-terminated path, no security attributes.
    if unsafe { windows_sys::Win32::Storage::FileSystem::CreateDirectoryW(path.as_ptr(), std::ptr::null()) } != 0 { 0 } else { last_error() }
}

/// MoveFileW: does not replace an existing item, but changing only the letter case is allowed. 0 = success.
pub fn move_file(source: &str, destination: &str) -> u32 {
    let (source, destination) = (wide(source), wide(destination));
    // SAFETY: both NUL-terminated.
    if unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(source.as_ptr(), destination.as_ptr()) } != 0 { 0 } else { last_error() }
}

/// Original FUN_00434630: CreateFileW(GENERIC_READ, FILE_SHARE_READ, CREATE_NEW, FILE_ATTRIBUTE_ARCHIVE) then CloseHandle.
/// Never empties an existing item. 0 = success.
pub fn create_empty_file(path: &str) -> u32 {
    use windows_sys::Win32::{Foundation::{CloseHandle, INVALID_HANDLE_VALUE}, Storage::FileSystem::*};
    let path = wide(path);
    // SAFETY: NUL-terminated path; the handle is closed immediately.
    unsafe {
        let handle = CreateFileW(path.as_ptr(), 0x8000_0000, FILE_SHARE_READ, std::ptr::null(), CREATE_NEW, FILE_ATTRIBUTE_ARCHIVE, std::ptr::null_mut());
        if handle == INVALID_HANDLE_VALUE {
            return last_error();
        }
        CloseHandle(handle);
    }
    0
}

/// Original PathCombineW(current folder, typed name): '\' at the start means the current drive root, "x:..." stays as is, "." and ".." are resolved.
/// Beyond MAX_PATH it falls back to a simple join (the same approved fix as C# NDR).
pub fn combine_typed_name(directory: &str, name: &str) -> String {
    let (directory_wide, name_wide) = (wide(directory), wide(name));
    let mut buffer = vec![0u16; 260];
    // SAFETY: the destination holds MAX_PATH characters as the API requires.
    let result = unsafe { windows_sys::Win32::UI::Shell::PathCombineW(buffer.as_mut_ptr(), directory_wide.as_ptr(), name_wide.as_ptr()) };
    if result.is_null() {
        return std::path::Path::new(directory).join(name).to_string_lossy().into_owned();
    }
    from_wide_ptr(buffer.as_ptr())
}

/// PathRemoveExtensionW(name): removes only the last extension (an extension containing a space is not an extension).
pub fn remove_extension(name: &str) -> String {
    let mut buffer = wide(name);
    // SAFETY: NUL-terminated writable buffer; the API only shortens it.
    unsafe {
        windows_sys::Win32::UI::Shell::PathRemoveExtensionW(buffer.as_mut_ptr());
    }
    from_wide_ptr(buffer.as_ptr())
}

/// Field split of original FUN_004351cc. Folders get the whole name and an empty extension. Files get PathRemoveExtensionW(name) and
/// PathFindExtensionW(name) without the dot (only when the extension is longer than one character).
pub fn split_for_rename(file_name: &str, is_directory: bool) -> (String, String) {
    if is_directory {
        return (file_name.into(), String::new());
    }
    let mut stem = wide(file_name);
    let original = wide(file_name);
    // SAFETY: both buffers are NUL-terminated; PathRemoveExtensionW only shortens its buffer.
    let extension = unsafe {
        windows_sys::Win32::UI::Shell::PathRemoveExtensionW(stem.as_mut_ptr());
        from_wide_ptr(windows_sys::Win32::UI::Shell::PathFindExtensionW(original.as_ptr()))
    };
    let extension = if extension.chars().count() > 1 { extension.chars().skip(1).collect() } else { String::new() };
    (from_wide_ptr(stem.as_ptr()), extension)
}

/// Original new folder default name: today's date in the user short date format with '\' and '/' turned into '-'.
pub fn new_folder_default_name() -> String {
    use windows_sys::Win32::Globalization::GetDateFormatW;
    const LOCALE_USER_DEFAULT: u32 = 0x400;
    const DATE_SHORTDATE: u32 = 0x1;
    // SAFETY: the first call asks for the length, the second fills a buffer of that length.
    let text = unsafe {
        let required = GetDateFormatW(LOCALE_USER_DEFAULT, DATE_SHORTDATE, std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), 0);
        if required <= 0 {
            String::new()
        } else {
            let mut buffer = vec![0u16; required as usize];
            let written = GetDateFormatW(LOCALE_USER_DEFAULT, DATE_SHORTDATE, std::ptr::null(), std::ptr::null(), buffer.as_mut_ptr(), required);
            if written > 0 { from_wide_ptr(buffer.as_ptr()) } else { String::new() }
        }
    };
    let text = if text.is_empty() { chrono::Local::now().format("%Y-%m-%d").to_string() } else { text };
    text.replace(['\\', '/'], "-")
}

/// Text of FormatMessageW(FROM_SYSTEM | IGNORE_INSERTS, LOCALE_USER_DEFAULT) (including the trailing line break), None when missing.
pub fn system_message(error: u32) -> Option<String> {
    use windows_sys::Win32::System::Diagnostics::Debug::{FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW};
    let mut buffer: *mut u16 = std::ptr::null_mut();
    // SAFETY: with ALLOCATE_BUFFER the API stores a LocalAlloc'd pointer in `buffer`, freed below.
    unsafe {
        let length = FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            error,
            0x400,
            (&mut buffer as *mut *mut u16).cast(),
            0,
            std::ptr::null(),
        );
        if length == 0 || buffer.is_null() {
            return None;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(buffer, length as usize));
        windows_sys::Win32::Foundation::LocalFree(buffer.cast());
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_name_operations_follow_the_original() {
        let root = std::env::temp_dir().join(format!("ndr-names-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();
        let folder = root.join("새 폴더");
        assert_eq!(create_directory(&folder.to_string_lossy()), 0);
        assert_eq!(create_directory(&folder.to_string_lossy()), 183, "ERROR_ALREADY_EXISTS");
        assert_ne!(create_directory(&root.join("a\\b").to_string_lossy()), 0, "no intermediate folders");
        let file = root.join("empty.txt");
        assert_eq!(create_empty_file(&file.to_string_lossy()), 0);
        std::fs::write(&file, b"keep").unwrap();
        assert_eq!(create_empty_file(&file.to_string_lossy()), 80, "CREATE_NEW never truncates (ERROR_FILE_EXISTS)");
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
        assert_eq!(move_file(&file.to_string_lossy(), &root.join("EMPTY.txt").to_string_lossy()), 0, "case-only rename");
        assert_ne!(move_file(&root.join("EMPTY.txt").to_string_lossy(), &folder.to_string_lossy()), 0, "never replaces");
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(combine_typed_name(r"C:\work\sub", "x.txt"), r"C:\work\sub\x.txt");
        assert_eq!(combine_typed_name(r"C:\work\sub", r"\x.txt"), r"C:\x.txt", "leading backslash: root of the drive");
        assert_eq!(combine_typed_name(r"C:\work\sub", r"..\x"), r"C:\work\x");
        assert_eq!(combine_typed_name(r"C:\work", r"D:\y"), r"D:\y");
        assert_eq!(split_for_rename("archive.tar.gz", false), ("archive.tar".into(), "gz".into()));
        assert_eq!(split_for_rename("name.", false), ("name".into(), "".into()), "a one-character extension (just the dot) is not shown");
        assert_eq!(split_for_rename("folder.v1", true), ("folder.v1".into(), "".into()));
        assert_eq!(split_for_rename(".gitignore", false), ("".into(), "gitignore".into()));
        let default = new_folder_default_name();
        assert!(!default.is_empty() && !default.contains(['\\', '/']));
        assert!(system_message(2).is_some_and(|m| !m.is_empty()));
    }

    #[test]
    fn shlwapi_name_rules() {
        assert_eq!(find_file_name(r"C:\dir\file.txt"), "file.txt");
        assert_eq!(find_file_name(r"C:\dir\sub\"), r"sub\");
        assert_eq!(remove_file_spec(r"C:\dir\file.txt"), r"C:\dir");
        assert_eq!(remove_file_spec(r"C:\file.txt"), r"C:\");
        assert_eq!(remove_file_spec(r"\\server\share\x"), r"\\server\share");
    }

    #[test]
    fn shlwapi_case_insensitive_search() {
        assert!(contains_ignore_case("Alphabet.doc", "PHA"));
        assert!(!contains_ignore_case("abc", ""), "StrStrIW returns NULL for an empty search text");
        assert!(contains_ignore_case("한글 문서.txt", "문서"));
        assert!(starts_with_ignore_case("gamma.log", "G"));
        assert!(!starts_with_ignore_case("beta.txt", "eta"));
        assert!(starts_with_ignore_case("abc", ""));
        assert!(equals_ignore_case("Renamed.TXT", "renamed.txt"));
        assert!(equals_ignore_case("새 폴더", "새 폴더"));
        assert!(!equals_ignore_case("abc", "abcd"));
    }
}
