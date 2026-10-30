//! Windows 공용 파일 대화 상자(IFileOpenDialog / IFileSaveDialog). C# OpenFolderDialog, OpenFileDialog, SaveFileDialog 자리.
//! 모달이라 닫힐 때까지 호출한 스레드를 막는다. 취소하거나 실패하면 None(빈 목록).
use std::path::{Path, PathBuf};

/// 저장 대화 상자 함수 모양(제목, 처음 폴더, 이름, (형식 이름, 패턴) 목록, 기본 확장자). 앱은 테스트에서 이것을 바꿔 끼운다.
pub type SaveFileFn = fn(&str, Option<&Path>, &str, &[(&str, &str)], &str, bool) -> Option<PathBuf>;
use windows::{
    Win32::{
        System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
            SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
        },
    },
    core::{HSTRING, Interface, PCWSTR},
};

/// CoInitializeEx가 성공했을 때만 CoUninitialize.
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
        // 빈 제목이면 Windows 기본 제목(원본 TSaveDialog처럼 Title이 없는 상자).
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

/// C# OpenFolderDialog.
pub fn browse_folder(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    let _com = Com::new();
    // SAFETY: COM object lifetime is managed by the windows crate wrappers.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.SetOptions(dialog.GetOptions().ok()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        prepare(&dialog.cast().ok()?, title, initial);
        dialog.Show(None).ok()?;
        item_path(&dialog.GetResult().ok()?)
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

/// C# OpenFileDialog(파일 하나, 형식 목록, 처음 이름).
pub fn open_file(title: &str, initial: Option<&Path>, file_name: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    open_one(title, initial, file_name, filters, true)
}

/// VCL TOpenDialog에서 ofFileMustExist를 끈 것(원본 TfFileJoin btnBrowseClick 0x423444): 열기 상자지만 없는 이름도 입력할 수 있다.
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

/// 열기 대화 상자 함수 모양(제목, 처음 폴더, 이름, 형식 목록).
pub type OpenFileFn = fn(&str, Option<&Path>, &str, &[(&str, &str)]) -> Option<PathBuf>;
/// C# SaveFileDialog: 고른 전체 경로. overwrite_prompt = OverwritePrompt. filters는 (이름, 패턴) 목록으로 첫 항목이 처음 선택이고,
/// default_extension이 있으면 확장자 없이 입력한 이름에 고른 형식의 확장자를 붙인다(AddExtension, DefaultExt).
pub fn save_file(title: &str, initial: Option<&Path>, file_name: &str, filters: &[(&str, &str)], default_extension: &str, overwrite_prompt: bool) -> Option<PathBuf> {
    let _com = Com::new();
    // SAFETY: as above. The filter strings outlive the SetFileTypes call.
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()? | FOS_FORCEFILESYSTEM;
        // C# OverwritePrompt(압축에 더하기는 false: 있는 파일을 골라도 묻지 않는다).
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


/// 원본 miNetwork(0x4416ec → FUN_0040d040): SHBrowseForFolderW(뿌리 SHGetFolderLocation(CSIDL_NETWORK 0x12), 제목, 0x249,
/// 콜백 FUN_0040cf90이 BFFM_INITIALIZED에서 처음 폴더를 고른다). 고른 파일 시스템 경로, 취소하거나 경로가 없으면 None.
pub fn browse_network(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        System::Com::CoTaskMemFree,
        UI::{
            Shell::{BFFM_INITIALIZED, BFFM_SETSELECTIONW, BROWSEINFOW, CSIDL_NETWORK, Common::ITEMIDLIST, SHBrowseForFolderW, SHGetFolderLocation, SHGetPathFromIDListW},
            WindowsAndMessaging::SendMessageW,
        },
    };
    unsafe extern "system" fn select_initial(hwnd: HWND, message: u32, _lparam: LPARAM, data: LPARAM) -> i32 {
        if message == BFFM_INITIALIZED && data != 0 {
            // SAFETY: data points to the NUL-terminated wide path that browse_network keeps alive while the dialog runs.
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
        let mut root: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHGetFolderLocation(std::ptr::null_mut(), CSIDL_NETWORK as i32, std::ptr::null_mut(), 0, &mut root) != 0 {
            return None;
        }
        let info = BROWSEINFOW {
            hwndOwner: std::ptr::null_mut(),
            pidlRoot: root,
            pszDisplayName: display.as_mut_ptr(),
            lpszTitle: title.as_ptr(),
            ulFlags: 0x249,
            lpfn: Some(select_initial),
            lParam: initial.as_ref().map_or(0, |w| w.as_ptr() as LPARAM),
            iImage: 0,
        };
        let chosen = SHBrowseForFolderW(&info);
        CoTaskMemFree(root as *const _);
        if chosen.is_null() {
            return None;
        }
        let found = SHGetPathFromIDListW(chosen, chosen_path.as_mut_ptr()) != 0;
        CoTaskMemFree(chosen as *const _);
        let length = chosen_path.iter().position(|&c| c == 0).unwrap_or(chosen_path.len());
        (found && length > 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&chosen_path[..length])))
    }
}
