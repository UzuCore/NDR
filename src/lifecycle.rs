//! User/profile-scoped instance forwarding and own-window tray/hotkey lifecycle.
use std::{
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
};
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NavigationRequest {
    pub left: Option<PathBuf>,
    pub right: Option<PathBuf>,
}
pub enum InstanceOutcome {
    Primary {
        guard: InstanceGuard,
        requests: Receiver<NavigationRequest>,
    },
    Forwarded,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    Restore,
    Exit,
    /// A volume device or media arrived or left (WM_DEVICECHANGE DBT_DEVICEARRIVAL/DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME).
    /// The original receives it through SHChangeNotify (SHCNE_DRIVEADD/DRIVEREMOVED/MEDIAINSERTED/MEDIAREMOVED) (FUN_00442bc8).
    DevicesChanged,
}

/// Whether a WM_DEVICECHANGE (wparam, device type) is a volume device or media arriving or leaving.
pub fn is_volume_change(event: usize, device_type: u32) -> bool {
    (event == 0x8000 || event == 0x8004) && device_type == 2
}
const MAX_REQUEST: usize = 64 * 1024;
fn parse_hotkey(value: &str) -> Result<Option<u32>, String> {
    let value = value.trim().to_ascii_uppercase();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() {
        return Ok(Some(value.as_bytes()[0] as u32));
    }
    Err("Activation key must be one ASCII letter A-Z or digit 0-9".into())
}
fn validate_request(request: &NavigationRequest) -> Result<(), String> {
    for path in [&request.left, &request.right].into_iter().flatten() {
        let text = path
            .to_str()
            .ok_or("Navigation paths must be valid Unicode")?;
        if text.encode_utf16().count() > 32767 || text.chars().any(|c| c < ' ') {
            return Err("Invalid navigation path".into());
        }
    }
    Ok(())
}
fn decode_request(payload: &[u8]) -> Result<NavigationRequest, String> {
    if payload.is_empty() || payload.len() > MAX_REQUEST {
        return Err("Navigation request exceeds 64 KiB".into());
    }
    let request = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
    validate_request(&request)?;
    Ok(request)
}
fn encode_request(request: &NavigationRequest) -> Result<Vec<u8>, String> {
    validate_request(request)?;
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_REQUEST {
        return Err("Navigation request exceeds 64 KiB".into());
    }
    Ok(bytes)
}
#[cfg(windows)]
pub use win::{InstanceGuard, NativeLifecycle};
#[cfg(windows)]
mod win {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        ffi::c_void,
        fs,
        marker::PhantomData,
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
        rc::Rc,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU64, Ordering},
            mpsc::{self, Sender, SyncSender},
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::{Shell::*, WindowsAndMessaging::*},
    };
    type Handle = *mut c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterHotKey(window: HWND, id: i32, modifiers: u32, key: u32) -> i32;
        fn UnregisterHotKey(window: HWND, id: i32) -> i32;
    }
    #[repr(C)]
    struct SecurityAttributes {
        size: u32,
        descriptor: Handle,
        inherit: i32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CloseHandle(handle: Handle) -> i32;
        fn LocalFree(memory: Handle) -> Handle;
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentProcessId() -> u32;
        fn OpenProcess(access: u32, inherit: i32, id: u32) -> Handle;
        fn CreateMutexW(
            attributes: *const SecurityAttributes,
            owned: i32,
            name: *const u16,
        ) -> Handle;
        fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
        fn ReleaseMutex(handle: Handle) -> i32;
        fn CreateNamedPipeW(
            name: *const u16,
            access: u32,
            mode: u32,
            max_instances: u32,
            out_size: u32,
            in_size: u32,
            timeout: u32,
            attributes: *const SecurityAttributes,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: Handle) -> i32;
        fn DisconnectNamedPipe(pipe: Handle) -> i32;
        fn GetNamedPipeServerProcessId(pipe: Handle, id: *mut u32) -> i32;
        fn SetNamedPipeHandleState(
            pipe: Handle,
            mode: *const u32,
            max: *const u32,
            timeout: *const u32,
        ) -> i32;
        fn CreateFileW(
            path: *const u16,
            access: u32,
            share: u32,
            attributes: Handle,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn ReadFile(
            file: Handle,
            buffer: Handle,
            size: u32,
            read: *mut u32,
            overlapped: Handle,
        ) -> i32;
        fn WriteFile(
            file: Handle,
            buffer: *const c_void,
            size: u32,
            written: *mut u32,
            overlapped: Handle,
        ) -> i32;
        fn GetModuleHandleW(name: *const u16) -> Handle;
        fn GetFinalPathNameByHandleW(
            handle: Handle,
            buffer: *mut u16,
            length: u32,
            flags: u32,
        ) -> u32;
        fn GetFileInformationByHandleEx(
            handle: Handle,
            class: i32,
            buffer: Handle,
            size: u32,
        ) -> i32;
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            class: u32,
            buffer: Handle,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut Handle,
            size: *mut u32,
        ) -> i32;
    }
    fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(Some(0)).collect()
    }
    fn error(context: &str) -> String {
        format!("{context}: {}", std::io::Error::last_os_error())
    }
    struct Owned(Handle);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct Descriptor(Handle);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn user_sid(process: Handle) -> Result<String, String> {
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(process, 8, &mut token) } == 0 {
            return Err(error("Cannot identify process user"));
        }
        let token = Owned(token);
        let mut needed = 0;
        unsafe {
            GetTokenInformation(token.0, 1, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed < std::mem::size_of::<Handle>() as u32 || needed > 65536 {
            return Err("Invalid user token length".into());
        }
        let mut bytes = vec![0u8; needed as usize];
        if unsafe {
            GetTokenInformation(token.0, 1, bytes.as_mut_ptr().cast(), needed, &mut needed)
        } == 0
        {
            return Err(error("Cannot read process user"));
        }
        let sid = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<Handle>()) };
        let mut text = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return Err(error("Cannot format user SID"));
        }
        let mut n = 0;
        unsafe {
            while n < 1024 && *text.add(n) != 0 {
                n += 1
            }
        }
        let result = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(text, n)) };
        unsafe {
            LocalFree(text.cast());
        }
        Ok(result)
    }
    fn descriptor(sid: &str) -> Result<Descriptor, String> {
        let sddl = wide(format!("D:P(A;;GA;;;{sid})"));
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error("Cannot create current-user-only object security"));
        }
        Ok(Descriptor(descriptor))
    }
    fn profile_identity(profile: &Path) -> Result<String, String> {
        let path = std::path::absolute(profile).map_err(|e| e.to_string())?;
        let mut normalized = PathBuf::new();
        for c in path.components() {
            match c {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::CurDir => {}
                _ => normalized.push(c.as_os_str()),
            }
        }
        let mut ancestor = normalized.as_path();
        let mut missing = Vec::new();
        while !ancestor.exists() {
            missing.push(
                ancestor
                    .file_name()
                    .ok_or("Profile has no existing ancestor")?
                    .to_owned(),
            );
            ancestor = ancestor
                .parent()
                .ok_or("Profile has no existing ancestor")?;
        }
        if !ancestor.is_dir() {
            return Err("Profile path has a non-directory ancestor".into());
        }
        let handle = fs::OpenOptions::new()
            .read(true)
            .access_mode(0)
            .custom_flags(0x02000000)
            .open(ancestor)
            .map_err(|e| e.to_string())?;
        let mut info = 0u32;
        let case_sensitive = unsafe {
            GetFileInformationByHandleEx(
                handle.as_raw_handle(),
                23,
                (&mut info as *mut u32).cast(),
                4,
            )
        } != 0
            && info & 1 != 0;
        let mut buffer = vec![0; 32768];
        let count = unsafe {
            GetFinalPathNameByHandleW(
                handle.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                1,
            )
        };
        let mut resolved = if count > 0 && (count as usize) < buffer.len() {
            PathBuf::from(String::from_utf16_lossy(&buffer[..count as usize]))
        } else {
            fs::canonicalize(ancestor).map_err(|e| e.to_string())?
        };
        for segment in missing.into_iter().rev() {
            resolved.push(segment)
        }
        let text = resolved
            .to_str()
            .ok_or("Profile path must be Unicode")?
            .trim_end_matches('\\')
            .to_owned();
        Ok(if case_sensitive {
            text
        } else {
            text.to_uppercase()
        })
    }
    fn pipe_read(
        pipe: Handle,
        buffer: &mut [u8],
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let mut done = 0;
        while done < buffer.len() {
            if cancel.load(Ordering::Relaxed) {
                return Err("Instance listener stopped".into());
            }
            if Instant::now() >= deadline {
                return Err("Navigation pipe read timed out".into());
            }
            let mut count = 0;
            let ok = unsafe {
                ReadFile(
                    pipe,
                    buffer[done..].as_mut_ptr().cast(),
                    (buffer.len() - done) as u32,
                    &mut count,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(232) {
                    return Err(e.to_string());
                }
            }
            done += count as usize;
            if count == 0 {
                thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }
    fn pipe_write(
        pipe: Handle,
        buffer: &[u8],
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let mut done = 0;
        while done < buffer.len() {
            if cancel.load(Ordering::Relaxed) {
                return Err("Instance listener stopped".into());
            }
            if Instant::now() >= deadline {
                return Err("Navigation pipe write timed out".into());
            }
            let mut count = 0;
            let ok = unsafe {
                WriteFile(
                    pipe,
                    buffer[done..].as_ptr().cast(),
                    (buffer.len() - done) as u32,
                    &mut count,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(error("Navigation pipe write failed"));
            }
            done += count as usize;
            if count == 0 {
                thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }
    fn listen(
        name: String,
        sid: String,
        events: SyncSender<NavigationRequest>,
        cancel: Arc<AtomicBool>,
        ready: Sender<Result<(), String>>,
    ) {
        let setup = (|| {
            let descriptor = descriptor(&sid)?;
            let attrs = SecurityAttributes {
                size: std::mem::size_of::<SecurityAttributes>() as u32,
                descriptor: descriptor.0,
                inherit: 0,
            };
            let name = wide(name);
            let pipe = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    3 | 0x80000,
                    1 | 8,
                    1,
                    MAX_REQUEST as u32 + 4,
                    MAX_REQUEST as u32 + 4,
                    650,
                    &attrs,
                )
            };
            if pipe as isize == -1 {
                return Err(error("Cannot create exclusive local navigation pipe"));
            }
            Ok(Owned(pipe))
        })();
        let pipe = match setup {
            Ok(pipe) => {
                let _ = ready.send(Ok(()));
                pipe
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        while !cancel.load(Ordering::Relaxed) {
            let connected = unsafe { ConnectNamedPipe(pipe.0, std::ptr::null_mut()) } != 0;
            let code = std::io::Error::last_os_error().raw_os_error();
            if !connected && code != Some(535) {
                if code != Some(536) {
                    unsafe {
                        DisconnectNamedPipe(pipe.0);
                    }
                }
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            let deadline = Instant::now() + Duration::from_millis(650);
            let accepted = (|| {
                let mut header = [0; 4];
                pipe_read(pipe.0, &mut header, deadline, &cancel)?;
                let length = u32::from_le_bytes(header) as usize;
                if length == 0 || length > MAX_REQUEST {
                    return Err("Navigation frame size rejected".into());
                }
                let mut payload = vec![0; length];
                pipe_read(pipe.0, &mut payload, deadline, &cancel)?;
                let request = decode_request(&payload)?;
                events.try_send(request).map_err(|e| e.to_string())?;
                Ok::<_, String>(())
            })()
            .is_ok();
            let _ = pipe_write(
                pipe.0,
                &[u8::from(accepted)],
                Instant::now() + Duration::from_millis(100),
                &cancel,
            );
            // DisconnectNamedPipe discards unread output. Wait for the client's
            // receipt with a deadline instead of an unbounded FlushFileBuffers.
            let mut receipt = [0];
            let _ = pipe_read(
                pipe.0,
                &mut receipt,
                Instant::now() + Duration::from_millis(200),
                &cancel,
            );
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
        }
    }
    fn forward(name: &str, sid: &str, request: &NavigationRequest) -> Result<(), String> {
        let payload = encode_request(request)?;
        let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
        frame.extend(payload);
        let name = wide(name);
        let mut last = "Primary navigation pipe is unavailable".to_owned();
        for _ in 0..20 {
            let raw = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    0xc0000000,
                    0,
                    std::ptr::null_mut(),
                    3,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if raw as isize == -1 {
                last = error("Cannot connect to primary instance");
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            let pipe = Owned(raw);
            let mut server = 0;
            if unsafe { GetNamedPipeServerProcessId(pipe.0, &mut server) } == 0 {
                return Err(error("Cannot verify primary instance process"));
            }
            let process = unsafe { OpenProcess(0x1000, 0, server) };
            if process.is_null() {
                return Err(error("Cannot verify primary instance user"));
            }
            let process = Owned(process);
            if user_sid(process.0)? != sid {
                return Err("Primary pipe belongs to another Windows user".into());
            }
            let mode = 1u32;
            if unsafe { SetNamedPipeHandleState(pipe.0, &mode, std::ptr::null(), std::ptr::null()) }
                == 0
            {
                return Err(error("Cannot make navigation pipe bounded"));
            }
            let cancel = AtomicBool::new(false);
            let deadline = Instant::now() + Duration::from_millis(1500);
            pipe_write(pipe.0, &frame, deadline, &cancel)?;
            let mut ack = [0];
            pipe_read(pipe.0, &mut ack, deadline, &cancel)?;
            pipe_write(pipe.0, &[0xa5], deadline, &cancel)?;
            if ack[0] != 1 {
                return Err("Primary instance rejected navigation request".into());
            }
            return Ok(());
        }
        Err(last)
    }
    pub struct InstanceGuard {
        mutex: Owned,
        cancel: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        _same_thread: PhantomData<Rc<()>>,
    }
    impl InstanceGuard {
        pub fn acquire(
            profile: &Path,
            request: NavigationRequest,
        ) -> Result<InstanceOutcome, String> {
            encode_request(&request)?;
            Self::open(profile, Some(request)).map(|outcome| outcome.unwrap_or(InstanceOutcome::Forwarded))
        }
        /// C# ApplySingleInstanceModeAsync(true): turning it on while running. None when another NDR already uses this user and profile,
        /// and nothing is sent to it.
        pub fn try_acquire(profile: &Path) -> Result<Option<(Self, Receiver<NavigationRequest>)>, String> {
            match Self::open(profile, None)? {
                Some(InstanceOutcome::Primary { guard, requests }) => Ok(Some((guard, requests))),
                _ => Ok(None),
            }
        }
        /// Primary when the mutex is taken. When it is already held, Forwarded after sending the request there (only when there is one), otherwise None.
        fn open(profile: &Path, request: Option<NavigationRequest>) -> Result<Option<InstanceOutcome>, String> {
            let sid = user_sid(unsafe { GetCurrentProcess() })?;
            let profile = profile_identity(profile)?;
            let identity = format!("{:X}", Sha256::digest(format!("{sid}\0{profile}")));
            let object = format!("NDR-Rust-{}", &identity[..32]);
            let descriptor = descriptor(&sid)?;
            let attrs = SecurityAttributes {
                size: std::mem::size_of::<SecurityAttributes>() as u32,
                descriptor: descriptor.0,
                inherit: 0,
            };
            let mutex_name = wide(format!("Global\\{object}"));
            let raw = unsafe { CreateMutexW(&attrs, 0, mutex_name.as_ptr()) };
            if raw.is_null() {
                return Err(error("Cannot open user/profile instance guard"));
            }
            let mutex = Owned(raw);
            let state = unsafe { WaitForSingleObject(mutex.0, 0) };
            let pipe = format!("\\\\.\\pipe\\{object}");
            match state {
                0 | 0x80 => {
                    let cancel = Arc::new(AtomicBool::new(false));
                    let (tx, requests) = mpsc::sync_channel(16);
                    let (ready_tx, ready_rx) = mpsc::channel();
                    let worker_cancel = cancel.clone();
                    let worker =
                        thread::spawn(move || listen(pipe, sid, tx, worker_cancel, ready_tx));
                    let guard = Self {
                        mutex,
                        cancel,
                        worker: Some(worker),
                        _same_thread: PhantomData,
                    };
                    match ready_rx.recv_timeout(Duration::from_secs(2)) {
                        Ok(Ok(())) => Ok(Some(InstanceOutcome::Primary { guard, requests })),
                        Ok(Err(e)) => Err(e),
                        Err(e) => Err(format!("Navigation listener startup failed: {e}")),
                    }
                }
                0x102 => match request {
                    Some(request) => {
                        forward(&pipe, &sid, &request)?;
                        Ok(Some(InstanceOutcome::Forwarded))
                    }
                    None => Ok(None),
                },
                _ => Err(error("Cannot acquire instance mutex")),
            }
        }
    }
    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            self.cancel.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            unsafe {
                ReleaseMutex(self.mutex.0);
            }
        }
    }
    const COMMAND: u32 = WM_APP + 0x4a0;
    const TRAY: u32 = WM_APP + 0x4a1;
    struct Status {
        tray: AtomicBool,
        fallback: AtomicBool,
        hotkey_error: Mutex<Option<String>>,
    }
    enum UiCommand {
        Hotkey(Option<u32>, Sender<Result<bool, String>>),
        Tray(bool, Sender<Result<(), String>>),
        Labels(String, String, String, Sender<Result<(), String>>),
        Wake(Box<dyn Fn() + Send>),
        Shutdown,
    }
    struct WindowState {
        events: Sender<LifecycleEvent>,
        /// Function that wakes the UI after sending an event (egui request_repaint).
        wake: Option<Box<dyn Fn() + Send>>,
        commands: Receiver<UiCommand>,
        status: Arc<Status>,
        tray_wanted: bool,
        icon: NOTIFYICONDATAW,
        version4: bool,
        taskbar_created: u32,
        key: Option<u32>,
        key_id: i32,
        open_label: String,
        exit_label: String,
    }
    impl WindowState {
        fn tray(&mut self, enabled: bool) -> Result<(), String> {
            self.tray_wanted = enabled;
            if self.status.tray.load(Ordering::Relaxed) {
                if !enabled {
                    unsafe {
                        Shell_NotifyIconW(NIM_DELETE, &self.icon);
                    }
                    self.status.tray.store(false, Ordering::Relaxed);
                }
                return Ok(());
            }
            if !enabled {
                return Ok(());
            }
            if unsafe { Shell_NotifyIconW(NIM_ADD, &self.icon) } == 0 {
                return Err(error("Tray icon is unavailable"));
            }
            self.status.tray.store(true, Ordering::Relaxed);
            self.icon.Anonymous.uVersion = 4;
            self.version4 = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &self.icon) } != 0;
            Ok(())
        }
        fn hotkey(&mut self, hwnd: HWND, key: Option<u32>) -> Result<bool, String> {
            if key == self.key {
                if let Ok(mut error) = self.status.hotkey_error.lock() {
                    *error = None;
                }
                return Ok(self.status.fallback.load(Ordering::Relaxed));
            }
            if let Some(key) = key {
                let next = if self.key_id == 0x4e44 {
                    0x4e45
                } else {
                    0x4e44
                };
                // Original FormShow (0x42a5cc): RegisterHotKey(hWnd, atom, MOD_WIN(8), vk) once. On failure it does not retry with another combination,
                // and there is no MOD_NOREPEAT. Fixed: previously it registered Win+Ctrl instead, silently changing the combination.
                let fallback = false;
                if unsafe { RegisterHotKey(hwnd, next, 8, key) } == 0 {
                    return Err(error("Activation hotkey is already in use"));
                }
                if self.key_id != 0 {
                    unsafe {
                        UnregisterHotKey(hwnd, self.key_id);
                    }
                }
                self.key_id = next;
                self.key = Some(key);
                self.status.fallback.store(fallback, Ordering::Relaxed);
                if let Ok(mut e) = self.status.hotkey_error.lock() {
                    *e = None
                }
                Ok(fallback)
            } else {
                if self.key_id != 0 {
                    unsafe {
                        UnregisterHotKey(hwnd, self.key_id);
                    }
                }
                self.key_id = 0;
                self.key = None;
                self.status.fallback.store(false, Ordering::Relaxed);
                if let Ok(mut e) = self.status.hotkey_error.lock() {
                    *e = None
                }
                Ok(false)
            }
        }
        fn cleanup(&mut self, hwnd: HWND) {
            let _ = self.tray(false);
            if self.key_id != 0 {
                unsafe {
                    UnregisterHotKey(hwnd, self.key_id);
                }
            }
            self.key_id = 0;
        }
    }
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        if message == WM_NCCREATE {
            let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            return 1;
        }
        if message == WM_CLOSE {
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
        if pointer.is_null() {
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        let state = unsafe { &mut *pointer };
        if message == state.taskbar_created && state.taskbar_created != 0 {
            state.status.tray.store(false, Ordering::Relaxed);
            if state.tray_wanted {
                let _ = state.tray(true);
            }
            return 0;
        }
        match message {
            COMMAND => {
                while let Ok(command) = state.commands.try_recv() {
                    match command {
                        UiCommand::Hotkey(key, reply) => {
                            let result = state.hotkey(hwnd, key);
                            if let Err(e) = &result {
                                if let Ok(mut current) = state.status.hotkey_error.lock() {
                                    *current = Some(e.clone())
                                }
                            }
                            let _ = reply.send(result);
                        }
                        UiCommand::Tray(enabled, reply) => {
                            let result = state.tray(enabled);
                            let _ = reply.send(result);
                        }
                        UiCommand::Labels(open, exit, tip, reply) => {
                            state.open_label = open;
                            state.exit_label = exit;
                            state.icon.szTip.fill(0);
                            for (i, c) in tip.encode_utf16().enumerate() {
                                state.icon.szTip[i] = c;
                            }
                            let result = if state.status.tray.load(Ordering::Relaxed)
                                && unsafe { Shell_NotifyIconW(NIM_MODIFY, &state.icon) } == 0
                            {
                                Err(error("Cannot update tray labels"))
                            } else {
                                Ok(())
                            };
                            let _ = reply.send(result);
                        }
                        UiCommand::Wake(wake) => state.wake = Some(wake),
                        UiCommand::Shutdown => {
                            unsafe {
                                PostMessageW(hwnd, WM_CLOSE, 0, 0);
                            }
                            return 0;
                        }
                    }
                }
                0
            }
            WM_HOTKEY if wparam as i32 == state.key_id && state.key_id != 0 => {
                let _ = state.events.send(LifecycleEvent::Restore);
                0
            }
            TRAY => {
                let notification = if state.version4 {
                    lparam as u32 & 0xffff
                } else {
                    lparam as u32
                };
                let id = if state.version4 {
                    (lparam as u32 >> 16) & 0xffff
                } else {
                    wparam as u32
                };
                if id == 1 {
                    if notification == WM_LBUTTONDBLCLK {
                        let _ = state.events.send(LifecycleEvent::Restore);
                    } else if notification == WM_CONTEXTMENU || notification == WM_RBUTTONUP {
                        let menu = unsafe { CreatePopupMenu() };
                        if !menu.is_null() {
                            let open = wide(&state.open_label);
                            let exit = wide(&state.exit_label);
                            unsafe {
                                AppendMenuW(menu, MF_STRING, 1, open.as_ptr());
                                AppendMenuW(menu, MF_STRING, 2, exit.as_ptr());
                            }
                            let mut point = unsafe { std::mem::zeroed() };
                            unsafe {
                                GetCursorPos(&mut point);
                                SetForegroundWindow(hwnd);
                            }
                            let selected = unsafe {
                                TrackPopupMenuEx(
                                    menu,
                                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                                    point.x,
                                    point.y,
                                    hwnd,
                                    std::ptr::null(),
                                )
                            };
                            unsafe {
                                DestroyMenu(menu);
                                PostMessageW(hwnd, WM_NULL, 0, 0);
                            }
                            if selected == 1 {
                                let _ = state.events.send(LifecycleEvent::Restore);
                            } else if selected == 2 {
                                let _ = state.events.send(LifecycleEvent::Exit);
                            }
                        }
                    }
                }
                0
            }
            WM_DESTROY => {
                state.cleanup(hwnd);
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    PostQuitMessage(0);
                }
                0
            }
            WM_DEVICECHANGE => {
                // lparam = DEV_BROADCAST_HDR(dbch_size, dbch_devicetype, ...). Only volumes (2) are considered.
                let device_type = if lparam != 0 { unsafe { *((lparam as *const u32).add(1)) } } else { 0 };
                if is_volume_change(wparam, device_type) {
                    let _ = state.events.send(LifecycleEvent::DevicesChanged);
                    if let Some(wake) = &state.wake {
                        wake();
                    }
                }
                1
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }
    fn own_window(owner: isize) -> Result<HWND, String> {
        let hwnd = owner as HWND;
        let mut process = 0;
        if hwnd.is_null()
            || unsafe { GetWindowThreadProcessId(hwnd, &mut process) } == 0
            || process != unsafe { GetCurrentProcessId() }
        {
            return Err(
                "Lifecycle operation requires a live window owned by this NDR process".into(),
            );
        }
        Ok(hwnd)
    }
    fn run_window(
        commands: Receiver<UiCommand>,
        events: Sender<LifecycleEvent>,
        status: Arc<Status>,
        key: Option<u32>,
        tray: bool,
        ready: Sender<Result<isize, String>>,
    ) {
        static ID: AtomicU64 = AtomicU64::new(0);
        let class_name = wide(format!(
            "NDR.RustLifecycle.{}.{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            let _ = ready.send(Err(error("Cannot register lifecycle message window")));
            return;
        }
        let mut state = Box::new(WindowState {
            events,
            wake: None,
            commands,
            status,
            tray_wanted: false,
            icon: unsafe { std::mem::zeroed() },
            version4: false,
            taskbar_created: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
            key: None,
            key_id: 0,
            open_label: "Open NDR".into(),
            exit_label: "Exit".into(),
        });
        let title = wide("NDR lifecycle");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                (&mut *state as *mut WindowState).cast(),
            )
        };
        if hwnd.is_null() {
            let _ = ready.send(Err(error("Cannot create lifecycle message window")));
            unsafe {
                UnregisterClassW(class_name.as_ptr(), instance);
            }
            return;
        }
        state.icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        state.icon.hWnd = hwnd;
        state.icon.uID = 1;
        state.icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        state.icon.uCallbackMessage = TRAY;
        state.icon.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), 32512usize as *const u16) };
        for (i, c) in "NDR".encode_utf16().enumerate() {
            state.icon.szTip[i] = c
        }
        let _ = state.tray(tray);
        if let Err(error) = state.hotkey(hwnd, key) {
            if let Ok(mut e) = state.status.hotkey_error.lock() {
                *e = Some(error)
            }
        }
        if ready.send(Ok(hwnd as isize)).is_err() {
            unsafe {
                DestroyWindow(hwnd);
            }
        }
        let mut message = unsafe { std::mem::zeroed() };
        loop {
            let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
            if result <= 0 {
                break;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        state.cleanup(hwnd);
        unsafe {
            UnregisterClassW(class_name.as_ptr(), instance);
        }
    }
    pub struct NativeLifecycle {
        owner: isize,
        window: isize,
        commands: Sender<UiCommand>,
        status: Arc<Status>,
        worker: Option<JoinHandle<()>>,
    }
    impl NativeLifecycle {
        pub fn set_tray_labels(&self, open: &str, exit: &str, tip: &str) -> Result<(), String> {
            if [open, exit, tip].iter().any(|s| s.contains('\0'))
                || open.encode_utf16().count() > 256
                || exit.encode_utf16().count() > 256
                || tip.encode_utf16().count() > 127
            {
                return Err("Invalid or oversized tray labels".into());
            }
            let (tx, rx) = mpsc::channel();
            self.commands
                .send(UiCommand::Labels(open.into(), exit.into(), tip.into(), tx))
                .map_err(|e| e.to_string())?;
            self.wake()?;
            rx.recv_timeout(Duration::from_secs(2))
                .map_err(|e| e.to_string())?
        }
        pub fn attach(
            owner: isize,
            key: Option<&str>,
            tray: bool,
        ) -> Result<(Self, Receiver<LifecycleEvent>), String> {
            own_window(owner)?;
            let key = parse_hotkey(key.unwrap_or(""))?;
            let (commands, receiver) = mpsc::channel();
            let (events, event_receiver) = mpsc::channel();
            let (ready, ready_receiver) = mpsc::channel();
            let status = Arc::new(Status {
                tray: AtomicBool::new(false),
                fallback: AtomicBool::new(false),
                hotkey_error: Mutex::new(None),
            });
            let worker_status = status.clone();
            let worker = thread::spawn(move || {
                run_window(receiver, events, worker_status, key, tray, ready)
            });
            match ready_receiver.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(window)) => Ok((
                    Self {
                        owner,
                        window,
                        commands,
                        status,
                        worker: Some(worker),
                    },
                    event_receiver,
                )),
                Ok(Err(e)) => {
                    let _ = worker.join();
                    Err(e)
                }
                Err(e) => Err(format!("Lifecycle worker startup failed: {e}")),
            }
        }
        fn wake(&self) -> Result<(), String> {
            let hwnd = own_window(self.window)?;
            if unsafe { PostMessageW(hwnd, COMMAND, 0, 0) } == 0 {
                Err(error("Lifecycle message queue unavailable"))
            } else {
                Ok(())
            }
        }
        pub fn set_hotkey(&self, key: Option<&str>) -> Result<bool, String> {
            let key = parse_hotkey(key.unwrap_or(""))?;
            let (tx, rx) = mpsc::channel();
            self.commands
                .send(UiCommand::Hotkey(key, tx))
                .map_err(|e| e.to_string())?;
            self.wake()?;
            rx.recv_timeout(Duration::from_secs(2))
                .map_err(|e| e.to_string())?
        }
        pub fn set_tray(&self, enabled: bool) -> Result<(), String> {
            let (tx, rx) = mpsc::channel();
            self.commands
                .send(UiCommand::Tray(enabled, tx))
                .map_err(|e| e.to_string())?;
            self.wake()?;
            rx.recv_timeout(Duration::from_secs(2))
                .map_err(|e| e.to_string())?
        }
        pub fn tray_available(&self) -> bool {
            self.status.tray.load(Ordering::Relaxed)
        }
        /// Wake function called after sending lifecycle events (device changes etc.) (redraws the UI).
        pub fn set_wake(&self, wake: Box<dyn Fn() + Send>) -> Result<(), String> {
            self.commands.send(UiCommand::Wake(wake)).map_err(|_| "Lifecycle worker stopped".to_string())?;
            self.wake()
        }
        #[cfg(test)]
        pub(crate) fn send_device_change(&self, event: usize, header: &[u32]) -> Result<(), String> {
            let hwnd = own_window(self.window)?;
            unsafe {
                SendMessageW(hwnd, WM_DEVICECHANGE, event, header.as_ptr() as isize);
            }
            Ok(())
        }
        pub fn uses_shift_fallback(&self) -> bool {
            self.status.fallback.load(Ordering::Relaxed)
        }
        pub fn hotkey_error(&self) -> Option<String> {
            self.status.hotkey_error.lock().ok().and_then(|s| s.clone())
        }
        pub fn hide_if_minimized(&self) -> Result<bool, String> {
            let owner = own_window(self.owner)?;
            if self.tray_available() && unsafe { IsIconic(owner) } != 0 {
                unsafe {
                    ShowWindow(owner, SW_HIDE);
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
        pub fn show(&self) -> Result<(), String> {
            let owner = own_window(self.owner)?;
            unsafe {
                ShowWindow(owner, SW_RESTORE);
                SetForegroundWindow(owner);
            }
            Ok(())
        }
    }
    impl Drop for NativeLifecycle {
        fn drop(&mut self) {
            let _ = self.commands.send(UiCommand::Shutdown);
            let _ = self.wake();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}
#[cfg(not(windows))]
pub struct InstanceGuard;
#[cfg(not(windows))]
impl InstanceGuard {
    pub fn acquire(_: &Path, _: NavigationRequest) -> Result<InstanceOutcome, String> {
        Err("Native lifecycle requires Windows".into())
    }
}
#[cfg(not(windows))]
pub struct NativeLifecycle;
#[cfg(not(windows))]
impl NativeLifecycle {
    pub fn attach(
        _: isize,
        _: Option<&str>,
        _: bool,
    ) -> Result<(Self, Receiver<LifecycleEvent>), String> {
        Err("Native lifecycle requires Windows".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, SystemTime},
    };
    fn fixture() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "ndr-lifecycle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn hotkey_parser_has_no_os_side_effects() {
        assert_eq!(parse_hotkey(" e ").unwrap(), Some(b'E' as u32));
        assert_eq!(parse_hotkey("").unwrap(), None);
        for value in ["F1", "Ctrl+E", "AB", "\u{D55C}"] {
            assert!(parse_hotkey(value).is_err());
        }
    }
    #[test]
    fn bounded_navigation_wire_accepts_paths_and_rejects_commands() {
        assert_eq!(
            decode_request(br#"{"left":"C:\\owned","right":null}"#)
                .unwrap()
                .left,
            Some(PathBuf::from("C:\\owned"))
        );
        for payload in [
            br#"{"left":"C:\\owned","command":"delete"}"#.as_slice(),
            br#"{"left":"a\u0000b"}"#.as_slice(),
            b"not json".as_slice(),
        ] {
            assert!(decode_request(payload).is_err())
        }
        assert!(decode_request(&vec![b'x'; 65537]).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn own_profile_instance_forwards_exact_navigation() {
        let p = fixture();
        let initial = NavigationRequest::default();
        let InstanceOutcome::Primary { guard, requests } =
            InstanceGuard::acquire(&p, initial).unwrap()
        else {
            panic!("owned fixture already active")
        };
        let q = p.clone();
        let sender = std::thread::spawn(move || {
            matches!(
                InstanceGuard::acquire(
                    &q,
                    NavigationRequest {
                        left: Some(q.join("left")),
                        right: Some(q.join("right"))
                    }
                )
                .unwrap(),
                InstanceOutcome::Forwarded
            )
        });
        assert!(sender.join().unwrap());
        let received = requests.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(received.left, Some(p.join("left")));
        assert_eq!(received.right, Some(p.join("right")));
        drop(guard);
        assert!(matches!(
            InstanceGuard::acquire(&p, NavigationRequest::default()).unwrap(),
            InstanceOutcome::Primary { .. }
        ));
        fs::remove_dir_all(p).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn lifecycle_attaches_only_owned_hwnd_without_registering_hotkeys() {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn CreateWindowExW(
                ex: u32,
                class: *const u16,
                name: *const u16,
                style: u32,
                x: i32,
                y: i32,
                w: i32,
                h: i32,
                parent: *mut std::ffi::c_void,
                menu: *mut std::ffi::c_void,
                instance: *mut std::ffi::c_void,
                param: *mut std::ffi::c_void,
            ) -> *mut std::ffi::c_void;
            fn DestroyWindow(hwnd: *mut std::ffi::c_void) -> i32;
        }
        let class: Vec<_> = "STATIC".encode_utf16().chain(Some(0)).collect();
        let name: Vec<_> = "NDR owned lifecycle test"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                name.as_ptr(),
                0,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert!(!hwnd.is_null());
        let (lifecycle, _events) = NativeLifecycle::attach(hwnd as isize, None, false).unwrap();
        assert!(!lifecycle.tray_available());
        assert!(!lifecycle.uses_shift_fallback());
        assert!(!lifecycle.hide_if_minimized().unwrap());
        assert!(!lifecycle.set_hotkey(None).unwrap());
        assert!(lifecycle.set_hotkey(Some("Ctrl+E")).is_err());
        lifecycle.set_tray(false).unwrap();
        lifecycle
            .set_tray_labels("\u{C5F4}\u{AE30}", "\u{C885}\u{B8CC}", "NDR")
            .unwrap();
        // WM_DEVICECHANGE: a volume (DBT_DEVTYP_VOLUME = 2) arriving or leaving sends DevicesChanged and calls the wake function. Other devices are ignored.
        let woke = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = woke.clone();
        lifecycle.set_wake(Box::new(move || { counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst); })).unwrap();
        let volume: [u32; 5] = [20, 2, 0, 1 << 3, 0x0001];
        let port: [u32; 5] = [20, 3, 0, 0, 0];
        lifecycle.send_device_change(0x8000, &volume).unwrap();
        lifecycle.send_device_change(0x8000, &port).unwrap();
        lifecycle.send_device_change(0x8004, &volume).unwrap();
        // The window thread can be slow while other tests run in parallel: wait up to 5 s for each expected event, then briefly for an unexpected third.
        let mut got: Vec<_> = (0..2).filter_map(|_| _events.recv_timeout(Duration::from_secs(5)).ok()).collect();
        got.extend(_events.recv_timeout(Duration::from_millis(300)).ok());
        assert_eq!(got, vec![LifecycleEvent::DevicesChanged, LifecycleEvent::DevicesChanged]);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while woke.load(std::sync::atomic::Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(woke.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(is_volume_change(0x8000, 2) && is_volume_change(0x8004, 2) && !is_volume_change(0x8000, 3) && !is_volume_change(0x0007, 2));
        drop(lifecycle);
        assert!(NativeLifecycle::attach(0, None, false).is_err());
        assert_ne!(unsafe { DestroyWindow(hwnd) }, 0);
    }
}
