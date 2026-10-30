//! Native Explorer menus for explicitly selected filesystem items.
use std::{
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    ptr,
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::{FreeLibrary, HMODULE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::{
            Com::{
                COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
            LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
            Threading::GetCurrentThreadId,
        },
        UI::{
            Shell::{
                CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFOEX, Common::ITEMIDLIST, IContextMenu,
                IContextMenu2, IContextMenu3, IShellFolder, SHBindToParent, SHParseDisplayName,
                SUBCLASSPROC,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{Interface, PCSTR, PCWSTR},
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn prepare_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Err("Select at least one item for the Explorer menu".into());
    }
    if paths.len() > u32::MAX as usize {
        return Err("Too many selected items".into());
    }
    let mut result = Vec::with_capacity(paths.len());
    let mut parent = None;
    for path in paths {
        if path.as_os_str().encode_wide().any(|unit| unit == 0) {
            return Err("A shell path cannot contain NUL".into());
        }
        let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
        let mut path = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::ParentDir => {
                    path.pop();
                }
                Component::CurDir => {}
                other => path.push(other.as_os_str()),
            }
        }
        if path.file_name().is_none() {
            return Err("Select an item inside a filesystem folder".into());
        }
        std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let folder = path
            .parent()
            .ok_or("The selected item has no containing folder")?
            .to_string_lossy()
            .to_lowercase();
        if let Some(parent) = &parent {
            if parent != &folder {
                return Err("Explorer menu items must share one containing folder".into());
            }
        } else {
            parent = Some(folder);
        }
        result.push(path);
    }
    Ok(result)
}
fn validate_owner_identity(
    owner_pid: u32,
    owner_thread: u32,
    current_pid: u32,
    current_thread: u32,
) -> Result<(), String> {
    if owner_pid == 0 || owner_pid != current_pid {
        return Err("The foreground window does not belong to this NDR process".into());
    }
    if owner_thread == 0 || owner_thread != current_thread {
        return Err("The Explorer menu must run on the foreground window's UI thread".into());
    }
    Ok(())
}
fn owned_foreground() -> Result<HWND, String> {
    let owner = unsafe { GetForegroundWindow() };
    if owner.is_invalid() {
        return Err("There is no foreground owner window".into());
    }
    validate_owner(owner)?;
    Ok(owner)
}
fn validate_owner(owner: HWND) -> Result<(), String> {
    let mut pid = 0;
    let thread = unsafe { GetWindowThreadProcessId(owner, Some(&mut pid)) };
    validate_owner_identity(pid, thread, std::process::id(), unsafe {
        GetCurrentThreadId()
    })
}
struct Apartment;
impl Apartment {
    fn enter() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .ok()
            .map_err(|e| format!("Shell COM initialization: {e}"))?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Pidl(*mut ITEMIDLIST);
impl Pidl {
    fn parse(path: &Path) -> Result<Self, String> {
        let name = wide(path);
        let mut pidl = Self(ptr::null_mut());
        unsafe { SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut pidl.0, 0, None) }
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if pidl.0.is_null() {
            return Err("The shell returned an empty item identifier".into());
        }
        Ok(pidl)
    }
}
impl Drop for Pidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CoTaskMemFree(Some(self.0.cast()));
            }
        }
    }
}
struct Popup(HMENU);
impl Popup {
    fn create() -> Result<Self, String> {
        unsafe { CreatePopupMenu() }
            .map(Self)
            .map_err(|e| e.to_string())
    }
}
impl Drop for Popup {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}
struct HookState {
    api: Rc<SubclassApi>,
    menu3: Option<IContextMenu3>,
    menu2: Option<IContextMenu2>,
    #[cfg(test)]
    forwarded: std::cell::Cell<u32>,
}
type SetSubclass =
    unsafe extern "system" fn(HWND, SUBCLASSPROC, usize, usize) -> windows::core::BOOL;
type RemoveSubclass = unsafe extern "system" fn(HWND, SUBCLASSPROC, usize) -> windows::core::BOOL;
type GetSubclass =
    unsafe extern "system" fn(HWND, SUBCLASSPROC, usize, *mut usize) -> windows::core::BOOL;
type DefaultSubclass = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
struct Library(HMODULE);
impl Drop for Library {
    fn drop(&mut self) {
        let _ = unsafe { FreeLibrary(self.0) };
    }
}
struct SubclassApi {
    _library: Library,
    set: SetSubclass,
    remove: RemoveSubclass,
    get: GetSubclass,
    default: DefaultSubclass,
}
impl SubclassApi {
    fn load() -> Result<Self, String> {
        let library = Library(
            unsafe {
                LoadLibraryExW(
                    windows::core::w!("comctl32.dll"),
                    None,
                    LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
            }
            .map_err(|e| e.to_string())?,
        );
        let resolve = |name: &'static [u8], ordinal: usize| {
            unsafe { GetProcAddress(library.0, PCSTR(name.as_ptr())) }
                .or_else(|| unsafe { GetProcAddress(library.0, PCSTR(ordinal as *const u8)) })
                .ok_or_else(|| {
                    format!(
                        "Required Explorer menu subclass API unavailable: {}",
                        String::from_utf8_lossy(&name[..name.len() - 1])
                    )
                })
        };
        // Old common-controls versions expose GetWindowSubclass by ordinal only.
        let set = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, SetSubclass>(resolve(
                b"SetWindowSubclass\0",
                410,
            )?)
        };
        let get = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, GetSubclass>(resolve(
                b"GetWindowSubclass\0",
                411,
            )?)
        };
        let remove = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, RemoveSubclass>(resolve(
                b"RemoveWindowSubclass\0",
                412,
            )?)
        };
        let default = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, DefaultSubclass>(resolve(
                b"DefSubclassProc\0",
                413,
            )?)
        };
        Ok(Self {
            _library: library,
            set,
            remove,
            get,
            default,
        })
    }
}
struct MenuHook {
    owner: HWND,
    id: usize,
    state: Option<Box<HookState>>,
}
impl MenuHook {
    fn install(owner: HWND, menu: &IContextMenu) -> Result<Self, String> {
        let state = Box::new(HookState {
            api: Rc::new(SubclassApi::load()?),
            menu3: menu.cast().ok(),
            menu2: menu.cast().ok(),
            #[cfg(test)]
            forwarded: std::cell::Cell::new(0),
        });
        let id = (&*state as *const HookState) as usize;
        if !unsafe { (state.api.set)(owner, Some(menu_window_proc), id, id) }.as_bool() {
            return Err("Could not install shell-extension menu message handling".into());
        }
        Ok(Self {
            owner,
            id,
            state: Some(state),
        })
    }
}
impl Drop for MenuHook {
    fn drop(&mut self) {
        if validate_owner(self.owner).is_err() {
            return;
        }
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let _ = unsafe { (state.api.remove)(self.owner, Some(menu_window_proc), self.id) };
        let mut remaining = 0;
        // Avoid freeing callback data if a shell extension prevented subclass removal.
        if unsafe { (state.api.get)(self.owner, Some(menu_window_proc), self.id, &mut remaining) }
            .as_bool()
            && remaining == self.id
            && let Some(state) = self.state.take()
        {
            let _ = Box::leak(state);
        }
    }
}
unsafe extern "system" fn menu_window_proc(
    owner: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let state = unsafe { &*(data as *const HookState) };
    if message == WM_NCDESTROY {
        let _ = unsafe { (state.api.remove)(owner, Some(menu_window_proc), id) };
    } else if matches!(
        message,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) && (!matches!(message, WM_DRAWITEM | WM_MEASUREITEM) || wparam.0 == 0)
    {
        if let Some(menu) = &state.menu3 {
            let mut result = LRESULT(0);
            #[cfg(test)]
            state.forwarded.set(state.forwarded.get() + 1);
            if unsafe { menu.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }.is_ok() {
                return result;
            }
        } else if let Some(menu) = &state.menu2 {
            #[cfg(test)]
            state.forwarded.set(state.forwarded.get() + 1);
            if unsafe { menu.HandleMenuMsg(message, wparam, lparam) }.is_ok() {
                return LRESULT(isize::from(matches!(message, WM_DRAWITEM | WM_MEASUREITEM)));
            }
        }
    }
    unsafe { (state.api.default)(owner, message, wparam, lparam) }
}
struct ShellSelection {
    menu: IContextMenu,
    _folder: IShellFolder,
    _pidls: Vec<Pidl>,
}
impl ShellSelection {
    fn bind(owner: HWND, paths: &[PathBuf]) -> Result<Self, String> {
        let mut pidls = Vec::with_capacity(paths.len());
        let mut folder = None;
        let mut children = Vec::with_capacity(paths.len());
        for path in paths {
            let pidl = Pidl::parse(path)?;
            let mut child = ptr::null_mut();
            let parent: IShellFolder =
                unsafe { SHBindToParent(pidl.0, Some(&mut child)) }.map_err(|e| e.to_string())?;
            if child.is_null() {
                return Err("The selected item has no child shell identifier".into());
            }
            if folder.is_none() {
                folder = Some(parent);
            }
            children.push(child.cast_const());
            pidls.push(pidl);
        }
        let folder = folder.ok_or("Select at least one shell item")?;
        let menu = unsafe { folder.GetUIObjectOf::<IContextMenu>(owner, &children, None) }
            .map_err(|e| e.to_string())?;
        Ok(Self {
            _pidls: pidls,
            _folder: folder,
            menu,
        })
    }
    fn popup(&self) -> Result<Popup, String> {
        let popup = Popup::create()?;
        unsafe { self.menu.QueryContextMenu(popup.0, 0, 1, 0x7fff, 0) }
            .ok()
            .map_err(|e| e.to_string())?;
        Ok(popup)
    }
}

/// Show an Explorer menu on the current NDR foreground/UI thread; cancellation is success.
/// A selected shell command is invoked only after the native menu returns its command identifier.
pub fn show(paths: &[PathBuf]) -> Result<(), String> {
    let paths = prepare_paths(paths)?;
    let owner = owned_foreground()?;
    let _apartment = Apartment::enter()?;
    let selection = ShellSelection::bind(owner, &paths)?;
    let popup = selection.popup()?;
    let hook = MenuHook::install(owner, &selection.menu)?;
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.map_err(|e| e.to_string())?;
    if owned_foreground()? != owner {
        return Err("The foreground owner changed while preparing the shell menu".into());
    }
    let command = unsafe {
        TrackPopupMenuEx(
            popup.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            point.x,
            point.y,
            owner,
            None,
        )
    }
    .0 as u32;
    if validate_owner(owner).is_ok() {
        let _ = unsafe { PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0)) };
    }
    drop(hook);
    if command == 0 {
        return Ok(());
    }
    if !(1..=0x7fff).contains(&command) {
        return Err("The shell returned an invalid menu command".into());
    }
    validate_owner(owner)?;
    let ordinal = (command - 1) as usize;
    let invoke = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: 0x4000 | CMIC_MASK_PTINVOKE,
        hwnd: owner,
        lpVerb: PCSTR(ordinal as *const u8),
        lpVerbW: PCWSTR(ordinal as *const u16),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: point,
        ..Default::default()
    };
    unsafe {
        selection
            .menu
            .InvokeCommand((&invoke as *const CMINVOKECOMMANDINFOEX).cast())
    }
    .map_err(|e| e.to_string())
}

/// 원본 miPropertiesClick → FUN_00458da8: 한 폴더 안의 항목들로 IContextMenu를 얻어 "properties" 동사를 부른다(여러 파일 속성 창).
pub fn properties(paths: &[PathBuf]) -> Result<(), String> {
    let paths = prepare_paths(paths)?;
    let owner = owned_foreground()?;
    let _apartment = Apartment::enter()?;
    let selection = ShellSelection::bind(owner, &paths)?;
    let _popup = selection.popup()?;
    let invoke = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: 0x4000,
        hwnd: owner,
        lpVerb: PCSTR(c"properties".as_ptr().cast()),
        lpVerbW: windows::core::w!("properties"),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { selection.menu.InvokeCommand((&invoke as *const CMINVOKECOMMANDINFOEX).cast()) }.map_err(|e| e.to_string())
}

/// 원본 NexusFile은 Ctrl+V(Shift+Insert)를 단축키로 받는다. egui-winit은 클립보드에 글이 없으면(파일 목록만 있을 때)
/// 붙여넣기 글쇠를 아무 사건으로도 보내지 않으므로, 주 창 하위 클래스로 WM_KEYDOWN을 센다(take_paste_keys).
static PASTE_KEYS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static PASTE_DEFAULT: std::sync::OnceLock<DefaultSubclass> = std::sync::OnceLock::new();
/// 원본 단축키 표의 "Num *"·"Num +"·"Num -"·"Num /"·"Num 0~9"·"Num ." 글쇠. egui에는 숫자판 글쇠가 없으므로(Num *는 사건이 아예 없고,
/// 나머지는 보통 글쇠와 같은 이름) 하위 클래스가 (가상 키, Ctrl, Shift, Alt)를 남긴다(take_numpad_keys).
static NUMPAD_KEYS: std::sync::Mutex<Vec<(u16, bool, bool, bool)>> = std::sync::Mutex::new(Vec::new());
unsafe extern "system" fn paste_key_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_INSERT, VK_MENU, VK_SHIFT};
        let down = |key: u16| unsafe { GetKeyState(key as i32) } < 0;
        let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
        let key = wparam.0;
        if message == WM_KEYDOWN && !alt && ((key == b'V' as usize && ctrl && !shift) || (key == VK_INSERT as usize && shift && !ctrl)) {
            PASTE_KEYS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if (0x60..=0x6f).contains(&key) && key != 0x6c {
            NUMPAD_KEYS.lock().unwrap_or_else(|e| e.into_inner()).push((key as u16, ctrl, shift, alt));
        }
    }
    match PASTE_DEFAULT.get() {
        Some(default) => unsafe { default(hwnd, message, wparam, lparam) },
        None => LRESULT(0),
    }
}
/// 주 창에 붙여넣기 글쇠 하위 클래스를 단다(한 번). comctl32는 프로그램이 끝날 때까지 둔다.
pub fn install_paste_key_hook(hwnd: isize) -> Result<(), String> {
    if PASTE_DEFAULT.get().is_some() {
        return Ok(());
    }
    let api = SubclassApi::load()?;
    let _ = PASTE_DEFAULT.set(api.default);
    let installed = unsafe { (api.set)(HWND(hwnd as *mut core::ffi::c_void), Some(paste_key_proc), 0x4E44_4950, 0) };
    std::mem::forget(api);
    if installed.as_bool() { Ok(()) } else { Err("SetWindowSubclass failed".into()) }
}
/// 지난번 뒤로 센 붙여넣기 글쇠 수.
pub fn take_paste_keys() -> u32 {
    PASTE_KEYS.swap(0, std::sync::atomic::Ordering::Relaxed)
}

/// 지난번 뒤로 눌린 숫자판 글쇠(가상 키, Ctrl, Shift, Alt).
pub fn take_numpad_keys() -> Vec<(u16, bool, bool, bool)> {
    std::mem::take(&mut *NUMPAD_KEYS.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sandbox() -> PathBuf {
        let path = std::env::temp_dir().join(format!("ndr-shell-menu-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn empty_and_embedded_nul_are_rejected_before_shell_access() {
        assert!(prepare_paths(&[]).is_err());
        assert!(prepare_paths(&[PathBuf::from("C:\\bad\0name")]).is_err());
    }
    #[test]
    fn cross_parent_paths_are_rejected_and_unicode_siblings_work() {
        let root = sandbox();
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let first = root.join("문서 one.txt");
        let second = root.join("two.txt");
        let third = other.join("three.txt");
        for path in [&first, &second, &third] {
            std::fs::write(path, "owned fixture").unwrap();
        }
        assert_eq!(prepare_paths(&[first.clone(), second]).unwrap().len(), 2);
        assert!(prepare_paths(&[first, third]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn foreign_process_and_foreign_ui_thread_are_rejected() {
        assert!(validate_owner_identity(7, 9, 7, 9).is_ok());
        assert!(validate_owner_identity(8, 9, 7, 9).is_err());
        assert!(validate_owner_identity(7, 10, 7, 9).is_err());
    }
    #[test]
    #[ignore = "Constructs an Explorer menu for an owned fixture; invokes no commands and shows no popup"]
    fn owned_fixture_native_menu_construction() {
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let root = sandbox();
        let path = root.join("owned menu 문서.txt");
        std::fs::write(&path, "owned menu fixture").unwrap();
        let second = root.join("second.txt");
        std::fs::write(&second, "second owned fixture").unwrap();
        let _apartment = Apartment::enter().unwrap();
        let owner = Window(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    windows::core::w!("STATIC"),
                    windows::core::w!("NDR owned shell-menu fixture"),
                    WS_POPUP,
                    0,
                    0,
                    100,
                    100,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap(),
        );
        let paths = prepare_paths(&[path, second]).unwrap();
        let selection = ShellSelection::bind(owner.0, &paths).unwrap();
        let popup = selection.popup().unwrap();
        let hook = MenuHook::install(owner.0, &selection.menu).unwrap();
        let count = unsafe { GetMenuItemCount(Some(popup.0)) };
        assert!(count > 0, "Explorer returned no menu entries");
        let state = hook.state.as_ref().unwrap();
        let supports_forwarding = state.menu3.is_some() || state.menu2.is_some();
        let _ = unsafe {
            SendMessageW(
                owner.0,
                WM_INITMENUPOPUP,
                Some(WPARAM(popup.0.0 as usize)),
                Some(LPARAM(0)),
            )
        };
        if supports_forwarding {
            assert!(
                state.forwarded.get() > 0,
                "Shell extension menu message was not forwarded"
            );
        }
        let api = state.api.clone();
        let subclass_id = hook.id;
        println!(
            "Constructed Explorer menu with {count} root entries for an owned Unicode file; no command invoked"
        );
        drop(hook);
        let mut data = 0;
        assert!(
            !unsafe { (api.get)(owner.0, Some(menu_window_proc), subclass_id, &mut data) }
                .as_bool(),
            "Shell menu subclass was not removed"
        );
        drop(popup);
        drop(selection);
        drop(owner);
        std::fs::remove_dir_all(root).unwrap();
    }
}
