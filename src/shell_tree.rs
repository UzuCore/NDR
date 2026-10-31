//! 원본 패널 폴더 트리(FUN_0048da38의 TNXShellTreeView)의 셸 이름 공간: 뿌리 = 바탕 화면, 자식 = IShellFolder::EnumObjects(폴더).
//!
//! 항목은 열쇠(key)로 가리킨다. 바탕 화면은 빈 열쇠, 그 밖은 셸 구문 이름(SHGDN_FORPARSING: 파일 시스템 폴더는 전체 경로,
//! 내 PC·제어판·휴지통 같은 가상 폴더는 "::{CLSID}"). 열쇠는 SHParseDisplayName으로 다시 셸 항목이 된다.
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};
use windows::{
    Win32::{
        Foundation::LPARAM,
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{
            Common::{ITEMIDLIST, STRRET},
            IEnumIDList, ILCombine, IShellFolder, SHGDN_FORPARSING, SHGDN_INFOLDER, SHGDN_NORMAL, SHGetDesktopFolder,
            SHGetPathFromIDListW, SHGetSpecialFolderLocation, SHParseDisplayName, StrRetToStrW,
        },
    },
    core::PCWSTR,
};

/// 내 PC(CLSID_MyComputer)의 구문 이름. 파일 시스템 경로는 이 아래(드라이브 → 폴더)에 놓인다.
pub const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
/// 네트워크(CLSID_NetworkPlaces)의 구문 이름. UNC 경로(\\서버\공유)는 이 아래에 놓인다.
pub const NETWORK: &str = "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}";

const SFGAO_STREAM: u32 = 0x0040_0000;
const SFGAO_FOLDER: u32 = 0x2000_0000;
const SFGAO_HASSUBFOLDER: u32 = 0x8000_0000;
const SHCONTF_FOLDERS: u32 = 0x20;
const SHCONTF_INCLUDEHIDDEN: u32 = 0x80;
const CSIDL_CONTROLS: i32 = 3;
const CSIDL_BITBUCKET: i32 = 10;

/// 원본 FUN_0048e320이 따로 다루는 셸 폴더.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    None,
    /// CSIDL_CONTROLS: 고르면 control.exe.
    ControlPanel,
    /// CSIDL_BITBUCKET: 고르면 셸로 휴지통을 연다(FUN_00432fe4).
    RecycleBin,
}

#[derive(Clone, Debug)]
pub struct ShellNode {
    pub key: PathBuf,
    /// 보이는 이름(SHGDN_INFOLDER | SHGDN_NORMAL).
    pub name: String,
    /// 파일 시스템 폴더면 그 경로(SHGetPathFromIDListW).
    pub path: Option<PathBuf>,
    /// SFGAO_HASSUBFOLDER: 펼침 표시를 그린다.
    pub has_children: bool,
    pub special: Special,
}

struct Pidl(*mut ITEMIDLIST);
impl Drop for Pidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: shell-allocated identifier lists are freed once with the COM task allocator.
            unsafe { CoTaskMemFree(Some(self.0.cast_const().cast())) };
        }
    }
}

struct Com(bool);
impl Com {
    fn init() -> Self {
        // SAFETY: balanced with CoUninitialize in Drop when this call initialized the apartment.
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: matches the successful CoInitializeEx above on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn wide(text: &Path) -> Vec<u16> {
    text.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// 열쇠의 절대 항목 목록. 바탕 화면(빈 열쇠)은 빈 목록(null).
fn parse(key: &Path) -> Option<Pidl> {
    if key.as_os_str().is_empty() {
        return Some(Pidl(ptr::null_mut()));
    }
    if key.as_os_str().encode_wide().any(|c| c == 0) {
        return None;
    }
    let name = wide(key);
    let mut pidl = Pidl(ptr::null_mut());
    // SAFETY: NUL-terminated name; the returned list is owned by `pidl`.
    unsafe { SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut pidl.0, 0, None) }.ok()?;
    (!pidl.0.is_null()).then_some(pidl)
}

fn strret(folder: &IShellFolder, pidl: *const ITEMIDLIST, flags: windows::Win32::UI::Shell::SHGDNF) -> Option<String> {
    let mut value = STRRET::default();
    // SAFETY: `pidl` is valid for `folder`; StrRetToStrW frees the STRRET payload and returns a CoTaskMem string we free.
    unsafe {
        folder.GetDisplayNameOf(pidl, flags, &mut value).ok()?;
        let mut text = windows::core::PWSTR::null();
        StrRetToStrW(&mut value, Some(pidl), &mut text).ok()?;
        let result = text.to_string().ok();
        CoTaskMemFree(Some(text.0.cast_const().cast()));
        result
    }
}

fn special_key(desktop: &IShellFolder, csidl: i32) -> Option<String> {
    // SAFETY: the returned list is owned by `pidl`.
    let pidl = Pidl(unsafe { SHGetSpecialFolderLocation(None, csidl) }.ok()?);
    strret(desktop, pidl.0, SHGDN_FORPARSING)
}

fn file_path(pidl: *const ITEMIDLIST) -> Option<PathBuf> {
    let mut buffer = [0u16; 260];
    // SAFETY: MAX_PATH buffer as the API requires.
    if !unsafe { SHGetPathFromIDListW(pidl, &mut buffer) }.as_bool() {
        return None;
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    (length > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

/// 뿌리(바탕 화면).
pub fn desktop() -> Option<ShellNode> {
    let _com = Com::init();
    // SAFETY: plain COM calls on this thread's apartment.
    let desktop = unsafe { SHGetDesktopFolder() }.ok()?;
    // 바탕 화면 자신 = 빈 항목 목록(끝 표시 2바이트).
    let mut empty = [0u16; 1];
    let pidl = empty.as_mut_ptr().cast::<ITEMIDLIST>();
    let name = strret(&desktop, pidl, SHGDN_NORMAL).unwrap_or_default();
    Some(ShellNode { key: PathBuf::new(), name, path: file_path(pidl), has_children: true, special: Special::None })
}

/// TNXShellTreeView 펼치기: `key` 바로 아래의 폴더(숨김은 `show_hidden`일 때만), 셸 순서(IShellFolder::CompareIDs).
/// 압축 파일처럼 폴더이기도 한 스트림(SFGAO_STREAM)은 넣지 않는다.
pub fn children(key: &Path, show_hidden: bool) -> Vec<ShellNode> {
    let _com = Com::init();
    // SAFETY: every identifier list is either borrowed from its owner for the duration of the call or owned by a Pidl.
    unsafe {
        let Ok(desktop) = SHGetDesktopFolder() else { return vec![] };
        let Some(parent) = parse(key) else { return vec![] };
        let folder: IShellFolder = if parent.0.is_null() {
            desktop.clone()
        } else {
            match desktop.BindToObject(parent.0, None) {
                Ok(folder) => folder,
                Err(_) => return vec![],
            }
        };
        let mut flags = SHCONTF_FOLDERS;
        if show_hidden {
            flags |= SHCONTF_INCLUDEHIDDEN;
        }
        let mut list: Option<IEnumIDList> = None;
        if folder.EnumObjects(windows::Win32::Foundation::HWND::default(), flags, &mut list).is_err() {
            return vec![];
        }
        let Some(list) = list else { return vec![] };
        let mut items: Vec<Pidl> = Vec::new();
        loop {
            let mut one = [ptr::null_mut::<ITEMIDLIST>()];
            let mut fetched = 0u32;
            if list.Next(&mut one, Some(&mut fetched)).0 != 0 || fetched == 0 {
                break;
            }
            items.push(Pidl(one[0]));
            if items.len() > 20_000 {
                break;
            }
        }
        items.sort_by(|a, b| {
            let result = folder.CompareIDs(LPARAM(0), a.0, b.0);
            let code = (result.0 & 0xFFFF) as u16 as i16;
            code.cmp(&0)
        });
        let control = special_key(&desktop, CSIDL_CONTROLS);
        let recycle = special_key(&desktop, CSIDL_BITBUCKET);
        let mut nodes = Vec::with_capacity(items.len());
        for item in &items {
            let mut attributes = SFGAO_FOLDER | SFGAO_HASSUBFOLDER | SFGAO_STREAM;
            if folder.GetAttributesOf(&[item.0.cast_const()], &mut attributes).is_err() {
                continue;
            }
            if attributes & SFGAO_FOLDER == 0 || attributes & SFGAO_STREAM != 0 {
                continue;
            }
            let full = Pidl(ILCombine(Some(parent.0.cast_const()).filter(|p| !p.is_null()), Some(item.0.cast_const())));
            if full.0.is_null() {
                continue;
            }
            let Some(parsing) = strret(&desktop, full.0, SHGDN_FORPARSING) else { continue };
            let name = strret(&folder, item.0, windows::Win32::UI::Shell::SHGDNF(SHGDN_INFOLDER.0 | SHGDN_NORMAL.0)).unwrap_or_else(|| parsing.clone());
            let special = if control.as_deref() == Some(parsing.as_str()) {
                Special::ControlPanel
            } else if recycle.as_deref() == Some(parsing.as_str()) {
                Special::RecycleBin
            } else {
                Special::None
            };
            nodes.push(ShellNode { key: PathBuf::from(&parsing), name, path: file_path(full.0), has_children: attributes & SFGAO_HASSUBFOLDER != 0, special });
        }
        nodes
    }
}

/// SetPath(패널 위치)의 조상 열쇠: 바탕 화면(빈 열쇠)부터 `path`의 부모까지. 보통 경로는 내 PC → 드라이브 → 폴더,
/// UNC는 네트워크 → \\서버 → \\서버\공유. 대소문자는 셸이 돌려주는 것과 다를 수 있으니 비교는 대소문자를 무시한다.
pub fn ancestors(path: &Path) -> Vec<PathBuf> {
    let text = path.to_string_lossy();
    let mut keys = vec![PathBuf::new()];
    if text.starts_with(r"\\") {
        keys.push(PathBuf::from(NETWORK));
        let parts: Vec<&str> = text.trim_start_matches('\\').split('\\').filter(|p| !p.is_empty()).collect();
        for n in 1..parts.len() {
            keys.push(PathBuf::from(format!(r"\\{}", parts[..n].join("\\"))));
        }
    } else {
        keys.push(PathBuf::from(THIS_PC));
        let mut list: Vec<PathBuf> = path.ancestors().skip(1).map(Path::to_path_buf).collect();
        list.reverse();
        keys.extend(list);
    }
    keys
}

/// 원본 FUN_0048e320: 셸 경로 이름에 '{'가 있으면(가상 폴더) 패널을 옮기지 않고 셸로 연다.
/// 고친 것: 이름에 '{'가 든 보통 폴더(예: "C:\a\{b}")도 셸로 열리던 것을, 구문 이름이 "::"로 시작할 때만으로 좁혔다.
pub fn is_virtual(key: &Path) -> bool {
    key.to_string_lossy().starts_with("::")
}

/// 대소문자를 무시한 열쇠 비교(Windows 경로).
pub fn same_key(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().trim_end_matches('\\').to_lowercase() == b.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestors_follow_the_shell_namespace() {
        assert_eq!(ancestors(Path::new(r"C:\Users\a")), [PathBuf::new(), PathBuf::from(THIS_PC), PathBuf::from(r"C:\"), PathBuf::from(r"C:\Users")]);
        assert_eq!(ancestors(Path::new(r"\\srv\share\x")), [PathBuf::new(), PathBuf::from(NETWORK), PathBuf::from(r"\\srv"), PathBuf::from(r"\\srv\share")]);
        assert!(is_virtual(Path::new(THIS_PC)) && !is_virtual(Path::new(r"C:\a\{b}")));
        assert!(same_key(Path::new(r"c:\users"), Path::new(r"C:\Users")) && same_key(Path::new(r"C:\"), Path::new("C:")));
    }

    #[test]
    fn desktop_children_include_this_pc_and_drives_list_folders() {
        let Some(root) = desktop() else { return };
        assert!(root.key.as_os_str().is_empty() && !root.name.is_empty());
        let top = children(Path::new(""), false);
        assert!(top.iter().any(|n| same_key(&n.key, Path::new(THIS_PC))), "{:?}", top.iter().map(|n| &n.key).collect::<Vec<_>>());
        let drives = children(Path::new(THIS_PC), false);
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let drive = drives.iter().find(|n| same_key(&n.key, &PathBuf::from(format!("{system}\\")))).expect("system drive under This PC");
        assert!(drive.path.is_some() && drive.has_children);
        let folders = children(&drive.key, false);
        assert!(folders.iter().any(|n| n.name.eq_ignore_ascii_case("Windows") || n.key.ends_with("Windows")));
    }
}
