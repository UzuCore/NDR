//! Shell namespace of the original pane folder tree (TNXShellTreeView of FUN_0048da38): root = Desktop, children = IShellFolder::EnumObjects (folders).
//!
//! Items are referred to by key. The Desktop is the empty key, others are shell parsing names (SHGDN_FORPARSING: the full path for file system folders,
//! "::{CLSID}" for virtual folders such as My PC, Control Panel and Recycle Bin). SHParseDisplayName turns a key back into a shell item.
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

/// Parsing name of My PC (CLSID_MyComputer). File system paths sit below it (drive -> folders).
pub const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
/// Parsing name of Network (CLSID_NetworkPlaces). UNC paths (\\server\share) sit below it.
pub const NETWORK: &str = "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}";

const SFGAO_STREAM: u32 = 0x0040_0000;
const SFGAO_GHOSTED: u32 = 0x0000_8000;
const SFGAO_FOLDER: u32 = 0x2000_0000;
const SFGAO_HASSUBFOLDER: u32 = 0x8000_0000;
const SHCONTF_FOLDERS: u32 = 0x20;
const SHCONTF_INCLUDEHIDDEN: u32 = 0x80;
const CSIDL_CONTROLS: i32 = 3;
const CSIDL_BITBUCKET: i32 = 10;

/// Shell folders original FUN_0048e320 handles specially.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    None,
    /// CSIDL_CONTROLS: choosing it runs control.exe.
    ControlPanel,
    /// CSIDL_BITBUCKET: choosing it opens the Recycle Bin in the shell (FUN_00432fe4).
    RecycleBin,
}

#[derive(Clone, Debug)]
pub struct ShellNode {
    pub key: PathBuf,
    /// Display name (SHGDN_INFOLDER | SHGDN_NORMAL).
    pub name: String,
    /// The path for file system folders (SHGetPathFromIDListW).
    pub path: Option<PathBuf>,
    /// SFGAO_HASSUBFOLDER: draws the expand mark.
    pub has_children: bool,
    pub special: Special,
    /// SFGAO_GHOSTED (hidden items): original InitNode (0x7ed85c) sets the node's Cut flag from TNXShellFolder.Properties bit 1
    /// (SHGetItemProperties 0x7b4504 = attribute 0x8000), so the tree draws its icon dimmed.
    pub ghosted: bool,
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

/// Absolute item list of a key. The Desktop (empty key) is an empty list (null).
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

/// Root (Desktop).
/// Original TNXCustomShellTreeView.CreateRoot (0x7edb64): clears the items, adds one root node for the root folder (the Desktop by
/// default), named with SHGetDisplayName(SHGDN_NORMAL), with the shell image and the SubFolders flag, and selects it.
pub fn desktop() -> Option<ShellNode> {
    let _com = Com::init();
    // SAFETY: plain COM calls on this thread's apartment.
    let desktop = unsafe { SHGetDesktopFolder() }.ok()?;
    // The Desktop itself = an empty item list (2-byte terminator).
    let mut empty = [0u16; 1];
    let pidl = empty.as_mut_ptr().cast::<ITEMIDLIST>();
    let name = strret(&desktop, pidl, SHGDN_NORMAL).unwrap_or_default();
    Some(ShellNode { key: PathBuf::new(), name, path: file_path(pidl), has_children: true, special: Special::None, ghosted: false })
}

/// TNXShellTreeView expansion: folders directly under `key` (hidden ones only with `show_hidden`), in shell order (IShellFolder::CompareIDs).
/// Streams that are also folders, like archives (SFGAO_STREAM), are left out.
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
            let mut attributes = SFGAO_FOLDER | SFGAO_HASSUBFOLDER | SFGAO_STREAM | SFGAO_GHOSTED;
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
            nodes.push(ShellNode { key: PathBuf::from(&parsing), name, path: file_path(full.0), has_children: attributes & SFGAO_HASSUBFOLDER != 0, special, ghosted: attributes & SFGAO_GHOSTED != 0 });
        }
        nodes
    }
}

/// Ancestor keys for SetPath (pane location): from the Desktop (empty key) to the parent of `path`. Ordinary paths go My PC -> drive -> folders,
/// UNC goes Network -> \\server -> \\server\share. Letter case may differ from what the shell returns, so comparisons ignore case.
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

/// Original FUN_0048e320: when the shell path name contains '{' (a virtual folder), the shell opens it instead of moving the pane.
/// Fixed: ordinary folders with '{' in the name (e.g. "C:\a\{b}") also opened in the shell; now only parsing names starting with "::" do.
pub fn is_virtual(key: &Path) -> bool {
    key.to_string_lossy().starts_with("::")
}

/// Case-insensitive key comparison (Windows paths).
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
