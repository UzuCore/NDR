//! Windows common file dialogs (IFileOpenDialog / IFileSaveDialog). In place of C# OpenFolderDialog, OpenFileDialog, SaveFileDialog.
//! Modal, so they block the calling thread until closed. Cancel or failure gives None (an empty list).
use std::path::{Path, PathBuf};

/// Save dialog function shape (title, initial folder, name, list of (type name, pattern), default extension). The app swaps it in tests.
pub type SaveFileFn = fn(&str, Option<&Path>, &str, &[(&str, &str)], &str, bool) -> Option<PathBuf>;
use windows::{
    Win32::{
        System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
            SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
        },
    },
    core::{HSTRING, Interface, PCWSTR},
};

/// CoUninitialize only when CoInitializeEx succeeded.
struct Com(bool);
impl Com {
    fn new() -> Self {
        // SAFETY: plain COM apartment initialisation for this thread.
        Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balanced with the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
    }
}

fn item_path(item: &IShellItem) -> Option<PathBuf> {
    // SAFETY: the returned string is CoTaskMemAlloc'd and freed after copying.
    unsafe {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as _));
        path.map(PathBuf::from)
    }
}

fn prepare(dialog: &IFileDialog, title: &str, folder: Option<&Path>) {
    // SAFETY: COM calls on a live dialog object; failures only leave defaults.
    unsafe {
        // An empty title uses the Windows default title (a box without Title, like the original TSaveDialog).
        if !title.is_empty() {
            let _ = dialog.SetTitle(&HSTRING::from(title));
        }
        if let Some(folder) = folder.filter(|f| f.is_dir()) {
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(folder.as_os_str()), None) {
                let _ = dialog.SetFolder(&item);
            }
        }
    }
}

/// Original folder picker FUN_0040cc50 (Add folder in Find, BrowseStartPath / BrowseSystemPath in Preferences): SHBrowseForFolderW with
/// flags 0x49 (only file system folders, new dialog style) and, when the start folder exists, callback FUN_0040ca60 selecting it on
/// BFFM_INITIALIZED. Previously the newer folder version of the open dialog was shown.
pub fn browse_folder(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    shell_browse(title, initial, None, 0x49)
}

/// SHBrowseForFolderW under `root` (a CSIDL, or the desktop) with `flags`; `initial` is selected when it is an existing folder.
/// The chosen file system path; None on cancel or when the choice has no path.
fn shell_browse(title: &str, initial: Option<&Path>, root: Option<i32>, flags: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::{
            Shell::{BFFM_INITIALIZED, BFFM_SETSELECTIONW, BROWSEINFOW, Common::ITEMIDLIST, SHBrowseForFolderW, SHGetFolderLocation, SHGetPathFromIDListW},
            WindowsAndMessaging::SendMessageW,
        },
    };
    unsafe extern "system" fn select_initial(hwnd: HWND, message: u32, _lparam: LPARAM, data: LPARAM) -> i32 {
        if message == BFFM_INITIALIZED && data != 0 {
            // SAFETY: data points to the NUL-terminated wide path that shell_browse keeps alive while the dialog runs.
            unsafe { SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, data) };
        }
        0
    }
    let wide = |text: &std::ffi::OsStr| text.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let title = wide(std::ffi::OsStr::new(title));
    let initial = initial.filter(|p| p.is_dir()).map(|p| wide(p.as_os_str()));
    let mut display = [0u16; 260];
    let mut chosen_path = [0u16; 260];
    let _com = Com::new();
    // SAFETY: every buffer passed to the shell outlives the modal call; the ID lists are freed with CoTaskMemFree.
    unsafe {
        let mut root_list: *mut ITEMIDLIST = std::ptr::null_mut();
        if let Some(csidl) = root
            && SHGetFolderLocation(std::ptr::null_mut(), csidl, std::ptr::null_mut(), 0, &mut root_list) != 0
        {
            return None;
        }
        let info = BROWSEINFOW {
            hwndOwner: std::ptr::null_mut(),
            pidlRoot: root_list,
            pszDisplayName: display.as_mut_ptr(),
            lpszTitle: title.as_ptr(),
            ulFlags: flags,
            lpfn: initial.is_some().then_some(select_initial as unsafe extern "system" fn(HWND, u32, LPARAM, LPARAM) -> i32),
            lParam: initial.as_ref().map_or(0, |w| w.as_ptr() as LPARAM),
            iImage: 0,
        };
        let chosen = SHBrowseForFolderW(&info);
        if !root_list.is_null() {
            CoTaskMemFree(Some(root_list as *const _));
        }
        if chosen.is_null() {
            return None;
        }
        let found = SHGetPathFromIDListW(chosen, chosen_path.as_mut_ptr()) != 0;
        CoTaskMemFree(Some(chosen as *const _));
        let length = chosen_path.iter().position(|&c| c == 0).unwrap_or(chosen_path.len());
        (found && length > 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&chosen_path[..length])))
    }
}

/// C# OpenFileDialog(Multiselect, CheckFileExists).
pub fn open_files(title: &str, initial: Option<&Path>) -> Vec<PathBuf> {
    let _com = Com::new();
    // SAFETY: as above.
    let result = unsafe {
        (|| {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            dialog.SetOptions(dialog.GetOptions().ok()? | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM).ok()?;
            prepare(&dialog.cast().ok()?, title, initial);
            dialog.Show(None).ok()?;
            let items = dialog.GetResults().ok()?;
            let count = items.GetCount().ok()?;
            Some((0..count).filter_map(|i| items.GetItemAt(i).ok().and_then(|item| item_path(&item))).collect::<Vec<_>>())
        })()
    };
    result.unwrap_or_default()
}

/// C# OpenFileDialog (one file, type list, initial name).
pub fn open_file(title: &str, initial: Option<&Path>, file_name: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    open_one(title, initial, file_name, filters, true)
}

/// VCL TOpenDialog with ofFileMustExist off (original TfFileJoin btnBrowseClick 0x423444): an open box that also accepts names that do not exist.
pub fn open_any_file(title: &str, initial: Option<&Path>, file_name: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    open_one(title, initial, file_name, filters, false)
}

fn open_one(title: &str, initial: Option<&Path>, file_name: &str, filters: &[(&str, &str)], must_exist: bool) -> Option<PathBuf> {
    let _com = Com::new();
    // SAFETY: as above; the filter strings outlive SetFileTypes.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()? | FOS_FORCEFILESYSTEM;
        dialog.SetOptions(if must_exist { options } else { options & !FOS_FILEMUSTEXIST }).ok()?;
        prepare(&dialog.cast().ok()?, title, initial);
        let names: Vec<(HSTRING, HSTRING)> = filters.iter().map(|(name, spec)| (HSTRING::from(*name), HSTRING::from(*spec))).collect();
        let specs: Vec<COMDLG_FILTERSPEC> = names.iter().map(|(name, spec)| COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) }).collect();
        if !specs.is_empty() {
            let _ = dialog.SetFileTypes(&specs);
            let _ = dialog.SetFileTypeIndex(1);
        }
        let _ = dialog.SetFileName(&HSTRING::from(file_name));
        dialog.Show(None).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

/// Open dialog function shape (title, initial folder, name, type list).
pub type OpenFileFn = fn(&str, Option<&Path>, &str, &[(&str, &str)]) -> Option<PathBuf>;
/// C# SaveFileDialog: the chosen full path. overwrite_prompt = OverwritePrompt. filters are a list of (name, pattern) with the first item initially chosen;
/// with default_extension, a name typed without an extension gets the chosen type's extension (AddExtension, DefaultExt).
pub fn save_file(title: &str, initial: Option<&Path>, file_name: &str, filters: &[(&str, &str)], default_extension: &str, overwrite_prompt: bool) -> Option<PathBuf> {
    let _com = Com::new();
    // SAFETY: as above. The filter strings outlive the SetFileTypes call.
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()? | FOS_FORCEFILESYSTEM;
        // C# OverwritePrompt (false when adding to an archive: choosing an existing file does not ask).
        dialog.SetOptions(if overwrite_prompt { options | FOS_OVERWRITEPROMPT } else { options & !FOS_OVERWRITEPROMPT }).ok()?;
        prepare(&dialog.cast().ok()?, title, initial);
        let names: Vec<(HSTRING, HSTRING)> = filters.iter().map(|(name, spec)| (HSTRING::from(*name), HSTRING::from(*spec))).collect();
        let specs: Vec<COMDLG_FILTERSPEC> = names.iter().map(|(name, spec)| COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) }).collect();
        if !specs.is_empty() {
            let _ = dialog.SetFileTypes(&specs);
            let _ = dialog.SetFileTypeIndex(1);
        }
        if !default_extension.is_empty() {
            let _ = dialog.SetDefaultExtension(&HSTRING::from(default_extension.trim_start_matches('.')));
        }
        let _ = dialog.SetFileName(&HSTRING::from(file_name));
        dialog.Show(None).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}


/// Original miNetwork (0x4416ec -> FUN_0040d040): SHBrowseForFolderW (root SHGetFolderLocation(CSIDL_NETWORK 0x12), title, 0x249,
/// callback FUN_0040cf90 selects the initial folder on BFFM_INITIALIZED). The chosen file system path; None on cancel or when there is no path.
pub fn browse_network(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    shell_browse(title, initial, Some(windows_sys::Win32::UI::Shell::CSIDL_NETWORK as i32), 0x249)
}
