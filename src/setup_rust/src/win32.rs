//! Native Win32 helpers: Registry (32/64 view), SetupAPI, Service Control, MSI, Processes, Logging.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::ffi::c_void;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub type HKEY = *mut c_void;
pub type HWND = *mut c_void;
pub type HANDLE = *mut c_void;
pub type SC_HANDLE = *mut c_void;

pub const HKEY_CLASSES_ROOT: HKEY = 0x8000_0000usize as HKEY;
pub const HKEY_CURRENT_USER: HKEY = 0x8000_0001usize as HKEY;
pub const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002usize as HKEY;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const ERROR_SUCCESS: i32 = 0;
const ERROR_NO_MORE_ITEMS: i32 = 259;

const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const KEY_CREATE_SUB_KEY: u32 = 0x0004;
const KEY_ENUMERATE_SUB_KEYS: u32 = 0x0008;
const KEY_READ: u32 = 0x20019;
const KEY_WRITE: u32 = 0x20006;
const KEY_ALL_ACCESS: u32 = 0x000F_003F;
const KEY_WOW64_64KEY: u32 = 0x0100;
const KEY_WOW64_32KEY: u32 = 0x0200;

const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_DWORD: u32 = 4;
const REG_MULTI_SZ: u32 = 7;

const SC_MANAGER_CONNECT: u32 = 0x0001;
const SERVICE_START: u32 = 0x0010;
const SERVICE_STOP: u32 = 0x0020;
const SERVICE_QUERY_STATUS: u32 = 0x0004;
const SERVICE_CONTROL_STOP: u32 = 0x0000_0001;
const SERVICE_STOPPED: u32 = 0x0000_0001;
const SERVICE_STOP_PENDING: u32 = 0x0000_0003;
const SERVICE_RUNNING: u32 = 0x0000_0004;
const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: u32 = 1061;
const SC_STATUS_PROCESS_INFO: u32 = 0;

const DIGCF_PRESENT: u32 = 0x0000_0002;
const DIGCF_ALLCLASSES: u32 = 0x0000_0004;
const SPDRP_DEVICEDESC: u32 = 0x0000_0000;
const SPDRP_HARDWAREID: u32 = 0x0000_0001;
const SPDRP_DRIVER: u32 = 0x0000_0009;
const DIF_REMOVE: u32 = 0x0000_0005;
const DIF_PROPERTYCHANGE: u32 = 0x0000_0012;
const DICS_PROPCHANGE: u32 = 0x0000_0003;
const DICS_FLAG_GLOBAL: u32 = 0x0000_0001;
const CR_NO_SUCH_DEVNODE: u32 = 0x0000_000D;

const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const PROCESS_TERMINATE: u32 = 0x0001;

const HWND_BROADCAST: HWND = 0xFFFFusize as HWND;
const WM_SETTINGCHANGE: u32 = 0x001A;
const SMTO_ABORTIFHUNG: u32 = 0x0002;

pub const CSIDL_DESKTOPDIRECTORY: i32 = 0x0010;
pub const CSIDL_COMMON_PROGRAMS: i32 = 0x0017;
pub const CSIDL_COMMON_DESKTOPDIRECTORY: i32 = 0x0019;
pub const CSIDL_PROGRAMS: i32 = 0x0002;
pub const CSIDL_APPDATA: i32 = 0x001A;
pub const CSIDL_LOCAL_APPDATA: i32 = 0x001C;
pub const CSIDL_COMMON_APPDATA: i32 = 0x0023;

const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
const SHCNE_UPDATEITEM: i32 = 0x0000_2000;
const SHCNF_PATHW: u32 = 0x0005;
const SHCNF_IDLIST: u32 = 0x0000;
const SHCNF_FLUSH: u32 = 0x1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegView {
    Native64,
    Wow32,
}

impl RegView {
    fn sam(self) -> u32 {
        match self {
            RegView::Native64 => KEY_WOW64_64KEY,
            RegView::Wow32 => KEY_WOW64_32KEY,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[repr(C)]
struct SP_DEVINFO_DATA {
    cb_size: u32,
    class_guid: GUID,
    dev_inst: u32,
    reserved: usize,
}

#[repr(C)]
struct SP_CLASSINSTALL_HEADER {
    cb_size: u32,
    install_function: u32,
}

#[repr(C)]
struct SP_PROPCHANGE_PARAMS {
    class_install_header: SP_CLASSINSTALL_HEADER,
    state_change: u32,
    scope: u32,
    hw_profile: u32,
}

#[repr(C)]
struct SERVICE_STATUS_PROCESS {
    dw_service_type: u32,
    dw_current_state: u32,
    dw_controls_accepted: u32,
    dw_win32_exit_code: u32,
    dw_service_specific_exit_code: u32,
    dw_check_point: u32,
    dw_wait_hint: u32,
    dw_process_id: u32,
    dw_service_flags: u32,
}

#[repr(C)]
struct SERVICE_STATUS {
    dw_service_type: u32,
    dw_current_state: u32,
    dw_controls_accepted: u32,
    dw_win32_exit_code: u32,
    dw_service_specific_exit_code: u32,
    dw_check_point: u32,
    dw_wait_hint: u32,
}

#[repr(C)]
struct PROCESSENTRY32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; 260],
}

extern "system" {
    fn RegOpenKeyExW(hKey: HKEY, lpSubKey: *const u16, ulOptions: u32, samDesired: u32, phkResult: *mut HKEY) -> i32;
    fn RegCreateKeyExW(
        hKey: HKEY, lpSubKey: *const u16, Reserved: u32, lpClass: *mut u16, dwOptions: u32,
        samDesired: u32, lpSecurityAttributes: *const c_void, phkResult: *mut HKEY, lpdwDisposition: *mut u32,
    ) -> i32;
    fn RegQueryValueExW(
        hKey: HKEY, lpValueName: *const u16, lpReserved: *mut u32, lpType: *mut u32,
        lpData: *mut u8, lpcbData: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        hKey: HKEY, lpValueName: *const u16, Reserved: u32, dwType: u32, lpData: *const u8, cbData: u32,
    ) -> i32;
    fn RegDeleteValueW(hKey: HKEY, lpValueName: *const u16) -> i32;
    fn RegDeleteTreeW(hKey: HKEY, lpSubKey: *const u16) -> i32;
    fn RegDeleteKeyExW(hKey: HKEY, lpSubKey: *const u16, samDesired: u32, Reserved: u32) -> i32;
    fn RegEnumKeyExW(
        hKey: HKEY, dwIndex: u32, lpName: *mut u16, lpcchName: *mut u32, lpReserved: *mut u32,
        lpClass: *mut u16, lpcchClass: *mut u32, lpftLastWriteTime: *mut c_void,
    ) -> i32;
    fn RegCloseKey(hKey: HKEY) -> i32;

    fn OpenSCManagerW(lpMachineName: *const u16, lpDatabaseName: *const u16, dwDesiredAccess: u32) -> SC_HANDLE;
    fn OpenServiceW(hSCManager: SC_HANDLE, lpServiceName: *const u16, dwDesiredAccess: u32) -> SC_HANDLE;
    fn StartServiceW(hService: SC_HANDLE, dwNumServiceArgs: u32, lpServiceArgVectors: *const *const u16) -> i32;
    fn ControlService(hService: SC_HANDLE, dwControl: u32, lpServiceStatus: *mut SERVICE_STATUS) -> i32;
    fn QueryServiceStatusEx(
        hService: SC_HANDLE, InfoLevel: u32, lpBuffer: *mut u8, cbBufSize: u32, pcbBytesNeeded: *mut u32,
    ) -> i32;
    fn CloseServiceHandle(hSCObject: SC_HANDLE) -> i32;

    fn SetupDiGetClassDevsW(ClassGuid: *const GUID, Enumerator: *const u16, hwndParent: HWND, Flags: u32) -> HANDLE;
    fn SetupDiCreateDeviceInfoList(ClassGuid: *const GUID, hwndParent: HWND) -> HANDLE;
    fn SetupDiOpenDeviceInfoW(
        DeviceInfoSet: HANDLE, DeviceInstanceId: *const u16, hwndParent: HWND, OpenFlags: u32, DeviceInfoData: *mut SP_DEVINFO_DATA,
    ) -> i32;
    fn SetupDiEnumDeviceInfo(DeviceInfoSet: HANDLE, MemberIndex: u32, DeviceInfoData: *mut SP_DEVINFO_DATA) -> i32;
    fn SetupDiGetDeviceRegistryPropertyW(
        DeviceInfoSet: HANDLE, DeviceInfoData: *const SP_DEVINFO_DATA, Property: u32,
        PropertyRegDataType: *mut u32, PropertyBuffer: *mut u8, PropertyBufferSize: u32, RequiredSize: *mut u32,
    ) -> i32;
    fn SetupDiGetDeviceInstanceIdW(
        DeviceInfoSet: HANDLE, DeviceInfoData: *const SP_DEVINFO_DATA,
        DeviceInstanceId: *mut u16, DeviceInstanceIdSize: u32, RequiredSize: *mut u32,
    ) -> i32;
    fn SetupDiSetClassInstallParamsW(
        DeviceInfoSet: HANDLE, DeviceInfoData: *const SP_DEVINFO_DATA,
        ClassInstallParams: *const SP_CLASSINSTALL_HEADER, ClassInstallParamsSize: u32,
    ) -> i32;
    fn SetupDiCallClassInstaller(
        InstallFunction: u32, DeviceInfoSet: HANDLE, DeviceInfoData: *const SP_DEVINFO_DATA,
    ) -> i32;
    fn SetupDiDestroyDeviceInfoList(DeviceInfoSet: HANDLE) -> i32;
    fn SetupGetInfDriverStoreLocationW(
        InfFileName: *const u16, AlternatePlatformInfo: *const c_void, LocaleName: *const u16,
        ReturnBuffer: *mut u16, ReturnBufferSize: u32, RequiredSize: *mut u32,
    ) -> i32;
    fn CM_Get_DevNode_Status(pulStatus: *mut u32, pulProblemNumber: *mut u32, dnDevInst: u32, ulFlags: u32) -> u32;

    fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> HANDLE;
    fn Process32FirstW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> i32;
    fn Process32NextW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> i32;
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> HANDLE;
    fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> i32;
    fn CloseHandle(hObject: HANDLE) -> i32;

    fn MsiSetInternalUI(dwUILevel: u32, phWnd: *mut HWND) -> u32;
    fn MsiEnumRelatedProductsW(lpUpgradeCode: *const u16, dwReserved: u32, iProductIndex: u32, lpProductBuf: *mut u16) -> u32;
    fn MsiConfigureProductExW(szProduct: *const u16, iInstallLevel: i32, eInstallState: i32, szCommandLine: *const u16) -> u32;

    fn SHGetFolderPathW(hwnd: HWND, csidl: i32, hToken: HANDLE, dwFlags: u32, pszPath: *mut u16) -> i32;
    fn SHChangeNotify(wEventId: i32, uFlags: u32, dwItem1: *const c_void, dwItem2: *const c_void);
    fn SendMessageTimeoutW(
        hWnd: HWND, Msg: u32, wParam: usize, lParam: isize, fuFlags: u32, uTimeout: u32, lpdwResult: *mut usize,
    ) -> isize;

    fn GetSystemMetrics(nIndex: i32) -> i32;
    fn OpenProcessToken(ProcessHandle: HANDLE, DesiredAccess: u32, TokenHandle: *mut HANDLE) -> i32;
    fn GetCurrentProcess() -> HANDLE;
    fn LookupPrivilegeValueW(lpSystemName: *const u16, lpName: *const u16, lpLuid: *mut u64) -> i32;
    fn AdjustTokenPrivileges(
        TokenHandle: HANDLE, DisableAllPrivileges: i32, NewState: *const c_void,
        BufferLength: u32, PreviousState: *mut c_void, ReturnLength: *mut u32,
    ) -> i32;
    fn ExitWindowsEx(uFlags: u32, dwReason: u32) -> i32;
    fn GetLastError() -> u32;
    fn GetTickCount64() -> u64;

    fn CreateFileW(
        lpFileName: *const u16, dwDesiredAccess: u32, dwShareMode: u32,
        lpSecurityAttributes: *const SECURITY_ATTRIBUTES, dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32, hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn CreateProcessW(
        lpApplicationName: *const u16, lpCommandLine: *mut u16,
        lpProcessAttributes: *const c_void, lpThreadAttributes: *const c_void,
        bInheritHandles: i32, dwCreationFlags: u32, lpEnvironment: *const c_void,
        lpCurrentDirectory: *const u16, lpStartupInfo: *const STARTUPINFOW,
        lpProcessInformation: *mut PROCESS_INFORMATION,
    ) -> i32;
    fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) -> u32;
    fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut u32) -> i32;
    fn MultiByteToWideChar(
        CodePage: u32, dwFlags: u32, lpMultiByteStr: *const u8, cbMultiByte: i32,
        lpWideCharStr: *mut u16, cchWideChar: i32,
    ) -> i32;

    fn OpenClipboard(hWndNewOwner: HWND) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(uFormat: u32, hMem: HANDLE) -> HANDLE;
    fn CloseClipboard() -> i32;
    fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> HANDLE;
    fn GlobalLock(hMem: HANDLE) -> *mut u8;
    fn GlobalUnlock(hMem: HANDLE) -> i32;
    fn GlobalFree(hMem: HANDLE) -> HANDLE;
}

#[repr(C)]
struct SECURITY_ATTRIBUTES {
    n_length: u32,
    lp_security_descriptor: *mut c_void,
    b_inherit_handle: i32,
}

#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    lp_reserved2: *mut u8,
    h_std_input: HANDLE,
    h_std_output: HANDLE,
    h_std_error: HANDLE,
}

#[repr(C)]
struct PROCESS_INFORMATION {
    h_process: HANDLE,
    h_thread: HANDLE,
    dw_process_id: u32,
    dw_thread_id: u32,
}

static LOG_LOCK: Mutex<()> = Mutex::new(());
static LIVE_LOG: Mutex<String> = Mutex::new(String::new());
static CMD_SEQ: AtomicU64 = AtomicU64::new(1);

pub fn log_file_path() -> PathBuf {
    let dir = PathBuf::from(r"C:\ProgramData\OpenFrankenDriver");
    if fs::create_dir_all(&dir).is_ok() {
        dir.join("setup.log")
    } else {
        std::env::temp_dir().join("OpenFrankenDriver_setup.log")
    }
}

/// Clear the in-memory live log buffer for the current session and write header lines.
pub fn reset_session_log(header_lines: &[String]) {
    {
        let mut live = LIVE_LOG.lock().unwrap_or_else(|e| e.into_inner());
        live.clear();
    }
    for line in header_lines {
        log_line(line);
    }
}

/// Return full live diagnostic log accumulated in the current session right up to this instant.
pub fn get_live_log_text() -> String {
    let live = LIVE_LOG.lock().unwrap_or_else(|e| e.into_inner());
    if !live.is_empty() {
        return live.clone();
    }
    drop(live);
    fs::read_to_string(log_file_path()).unwrap_or_default()
}

/// Copy UTF-8 string to Windows Clipboard as CF_UNICODETEXT (CRLF normalized).
pub fn set_clipboard_unicode(hwnd: HWND, text: &str) -> bool {
    let normalized = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let mut wide: Vec<u16> = normalized.encode_utf16().collect();
    wide.push(0);
    let byte_len = wide.len() * 2;

    const GMEM_MOVEABLE: u32 = 0x0002;
    const GMEM_ZEROINIT: u32 = 0x0040;
    const CF_UNICODETEXT: u32 = 13;

    unsafe {
        if OpenClipboard(hwnd) == 0 && OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let hmem = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, byte_len);
        if hmem.is_null() {
            CloseClipboard();
            return false;
        }
        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            GlobalFree(hmem);
            CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr, byte_len);
        GlobalUnlock(hmem);
        if SetClipboardData(CF_UNICODETEXT, hmem).is_null() {
            GlobalFree(hmem);
            CloseClipboard();
            return false;
        }
        CloseClipboard();
    }
    true
}

pub fn log_line(msg: &str) {
    let _g = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let formatted = format!("[{secs}] {msg}");
    {
        let mut live = LIVE_LOG.lock().unwrap_or_else(|e| e.into_inner());
        live.push_str(&formatted);
        live.push_str("\r\n");
    }
    let path = log_file_path();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{formatted}");
        let _ = f.flush();
    }
}

pub fn tick_count() -> u64 {
    unsafe { GetTickCount64() }
}

pub fn is_safe_mode() -> bool {
    unsafe { GetSystemMetrics(67 /* SM_CLEANBOOT */) != 0 }
}

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn system_directory() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root).join("System32")
}

pub fn syswow_directory() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root).join("SysWOW64")
}

pub fn program_files_64() -> PathBuf {
    std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

pub fn program_files_x86() -> PathBuf {
    std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"))
}

pub fn sh_folder_path(csidl: i32) -> Option<PathBuf> {
    let mut buf = [0u16; 320];
    let hr = unsafe { SHGetFolderPathW(std::ptr::null_mut(), csidl, std::ptr::null_mut(), 0, buf.as_mut_ptr()) };
    if hr == 0 {
        Some(PathBuf::from(from_wide(&buf)))
    } else {
        None
    }
}

pub fn sh_notify_path(path: &Path) {
    let w = to_wide(&path.to_string_lossy());
    unsafe { SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, w.as_ptr() as *const c_void, std::ptr::null()) };
}

pub fn sh_notify_assoc_changed() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST | SHCNF_FLUSH, std::ptr::null(), std::ptr::null()) };
}

pub fn broadcast_env_change() {
    let env_w = to_wide("Environment");
    let mut res: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            env_w.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            3000,
            &mut res,
        );
    }
}

fn quote_win_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| matches!(c, ' ' | '\t' | '\n' | '\x0b' | '"')) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            out.push_str(&"\\".repeat(backslashes * 2 + 1));
            out.push('"');
            backslashes = 0;
        } else {
            out.push_str(&"\\".repeat(backslashes));
            out.push(ch);
            backslashes = 0;
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

fn decode_console_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if bytes.len() >= 2 && bytes.len() % 2 == 0 && bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count() > bytes.len() / 4 {
        let u16s: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u16s).trim_matches('\0').to_string();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    // Console apps redirected to a file (pnputil, certutil) on Russian Windows write ANSI (CP1251 = 0xC0..0xFF),
    // whereas pure console-mode apps write OEM (CP866 = 0x80..0xAF / 0xE0..0xEF). Detect CP1251 vs CP866 cleanly.
    let c_acp = bytes.iter().filter(|&&b| (0xC0..=0xFF).contains(&b)).count();
    let c_oem = bytes.iter().filter(|&&b| (0x80..=0xAF).contains(&b)).count();
    let cp = if c_acp >= c_oem { 0 /* CP_ACP */ } else { 1 /* CP_OEMCP */ };
    let needed = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0) };
    if needed > 0 {
        let mut wbuf = vec![0u16; needed as usize];
        let written = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), bytes.len() as i32, wbuf.as_mut_ptr(), needed) };
        if written > 0 {
            return String::from_utf16_lossy(&wbuf[..written as usize]);
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Run an executable hidden (CREATE_NO_WINDOW) without pipe deadlock, with optional timeout and 1s tick callback.
/// Returns (exit_code, combined_output, timed_out).
pub fn run_hidden_timeout_with_ticks<F>(
    exe: &Path,
    args: &[&str],
    timeout_ms: u32,
    mut on_tick: F,
) -> Result<(i32, String, bool), String>
where
    F: FnMut(u64),
{
    log_line(&format!("RUN (timeout={}ms): {} {}", timeout_ms, exe.display(), args.join(" ")));

    let seq = CMD_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_out = std::env::temp_dir().join(format!("ofd_cmd_{}_{seq}.log", std::process::id()));
    let w_tmp = to_wide(&tmp_out.to_string_lossy());
    let w_nul = to_wide("NUL");

    let mut sa = SECURITY_ATTRIBUTES {
        n_length: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lp_security_descriptor: std::ptr::null_mut(),
        b_inherit_handle: 1,
    };

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const CREATE_ALWAYS: u32 = 2;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x0000_0100;
    const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
    const WAIT_OBJECT_0: u32 = 0;

    let h_out = unsafe {
        CreateFileW(
            w_tmp.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &mut sa,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_TEMPORARY,
            std::ptr::null_mut(),
        )
    };
    let h_in = unsafe {
        CreateFileW(
            w_nul.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &mut sa,
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    let invalid = (-1isize) as HANDLE;

    let mut cmdline = quote_win_arg(&exe.to_string_lossy());
    for a in args {
        cmdline.push(' ');
        cmdline.push_str(&quote_win_arg(a));
    }
    let mut w_cmd = to_wide(&cmdline);

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    if h_out != invalid {
        si.dw_flags = STARTF_USESTDHANDLES;
        si.h_std_input = if h_in != invalid { h_in } else { std::ptr::null_mut() };
        si.h_std_output = h_out;
        si.h_std_error = h_out;
    }

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            w_cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            if h_out != invalid { 1 } else { 0 },
            CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        )
    };

    if h_out != invalid {
        unsafe { CloseHandle(h_out) };
    }
    if h_in != invalid {
        unsafe { CloseHandle(h_in) };
    }

    if ok == 0 {
        let _ = fs::remove_file(&tmp_out);
        return Err(format!("Не удалось запустить {}", exe.display()));
    }

    let start = tick_count();
    let mut timed_out = false;
    loop {
        let w = unsafe { WaitForSingleObject(pi.h_process, 1000) };
        let elapsed = tick_count().saturating_sub(start);
        if w == WAIT_OBJECT_0 {
            break;
        }
        on_tick(elapsed);
        if timeout_ms > 0 && elapsed >= timeout_ms as u64 {
            log_line(&format!(
                "TIMEOUT ({}ms): terminating {} (PID {})",
                timeout_ms,
                exe.display(),
                pi.dw_process_id
            ));
            unsafe {
                TerminateProcess(pi.h_process, 0x102);
                WaitForSingleObject(pi.h_process, 500);
            }
            timed_out = true;
            break;
        }
    }

    let mut raw_code = 0u32;
    unsafe {
        GetExitCodeProcess(pi.h_process, &mut raw_code);
        CloseHandle(pi.h_thread);
        CloseHandle(pi.h_process);
    }

    let raw_bytes = fs::read(&tmp_out).unwrap_or_default();
    let _ = fs::remove_file(&tmp_out);
    let txt = decode_console_bytes(&raw_bytes);
    let code = if timed_out { -1 } else { raw_code as i32 };
    let trimmed = txt.trim();
    if trimmed.is_empty() {
        log_line(&format!("EXIT {code} (timed_out={timed_out}): {}", exe.display()));
    } else {
        let snippet: String = trimmed.chars().take(1800).collect();
        log_line(&format!("EXIT {code} (timed_out={timed_out}): {} | OUT: {snippet}", exe.display()));
    }
    Ok((code, txt, timed_out))
}

/// Run an executable hidden (CREATE_NO_WINDOW) with a timeout in milliseconds.
pub fn run_hidden_timeout(exe: &Path, args: &[&str], timeout_ms: u32) -> Result<(i32, String, bool), String> {
    run_hidden_timeout_with_ticks(exe, args, timeout_ms, |_| {})
}

/// Run an executable hidden (CREATE_NO_WINDOW) and capture (exit_code, combined_output).
pub fn run_hidden(exe: &Path, args: &[&str]) -> Result<(i32, String), String> {
    let (code, txt, _) = run_hidden_timeout_with_ticks(exe, args, 0, |_| {})?;
    Ok((code, txt))
}

fn split_reg_path(full: &str) -> Result<(HKEY, &str), String> {
    let normalized = full.trim();
    for (prefix, root) in [
        ("HKLM\\", HKEY_LOCAL_MACHINE),
        ("HKEY_LOCAL_MACHINE\\", HKEY_LOCAL_MACHINE),
        ("HKCU\\", HKEY_CURRENT_USER),
        ("HKEY_CURRENT_USER\\", HKEY_CURRENT_USER),
        ("HKCR\\", HKEY_CLASSES_ROOT),
        ("HKEY_CLASSES_ROOT\\", HKEY_CLASSES_ROOT),
    ] {
        if normalized.len() >= prefix.len() && normalized[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return Ok((root, &normalized[prefix.len()..]));
        }
    }
    Err(format!("Unsupported registry root in '{full}'"))
}

pub fn reg_key_exists(path: &str, view: RegView) -> bool {
    let Ok((root, sub)) = split_reg_path(path) else {
        return false;
    };
    let w_sub = to_wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    let rc = unsafe { RegOpenKeyExW(root, w_sub.as_ptr(), 0, KEY_QUERY_VALUE | view.sam(), &mut h) };
    if rc == ERROR_SUCCESS {
        unsafe { RegCloseKey(h) };
        true
    } else {
        false
    }
}

pub fn reg_read_string(path: &str, value_name: &str, view: RegView) -> Option<String> {
    let (root, sub) = split_reg_path(path).ok()?;
    let w_sub = to_wide(sub);
    let w_val = to_wide(value_name);
    let mut h: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(root, w_sub.as_ptr(), 0, KEY_READ | view.sam(), &mut h) } != ERROR_SUCCESS {
        return None;
    }
    let mut typ: u32 = 0;
    let mut cb: u32 = 0;
    let rc = unsafe { RegQueryValueExW(h, w_val.as_ptr(), std::ptr::null_mut(), &mut typ, std::ptr::null_mut(), &mut cb) };
    if rc != ERROR_SUCCESS || cb == 0 {
        unsafe { RegCloseKey(h) };
        return None;
    }
    let mut buf = vec![0u8; cb as usize + 2];
    let rc2 = unsafe { RegQueryValueExW(h, w_val.as_ptr(), std::ptr::null_mut(), &mut typ, buf.as_mut_ptr(), &mut cb) };
    unsafe { RegCloseKey(h) };
    if rc2 != ERROR_SUCCESS {
        return None;
    }
    let u16_slice: Vec<u16> = buf[..cb as usize]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Some(from_wide(&u16_slice))
}

pub fn reg_read_dword(path: &str, value_name: &str, view: RegView) -> Option<u32> {
    let (root, sub) = split_reg_path(path).ok()?;
    let w_sub = to_wide(sub);
    let w_val = to_wide(value_name);
    let mut h: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(root, w_sub.as_ptr(), 0, KEY_READ | view.sam(), &mut h) } != ERROR_SUCCESS {
        return None;
    }
    let mut typ: u32 = 0;
    let mut data: u32 = 0;
    let mut cb: u32 = 4;
    let rc = unsafe { RegQueryValueExW(h, w_val.as_ptr(), std::ptr::null_mut(), &mut typ, &mut data as *mut u32 as *mut u8, &mut cb) };
    unsafe { RegCloseKey(h) };
    if rc == ERROR_SUCCESS && typ == REG_DWORD {
        Some(data)
    } else {
        None
    }
}

pub fn reg_ensure_key(path: &str, view: RegView) -> Result<(), String> {
    let (root, sub) = split_reg_path(path)?;
    let w_sub = to_wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            root, w_sub.as_ptr(), 0, std::ptr::null_mut(), 0,
            KEY_WRITE | view.sam(), std::ptr::null(), &mut h, std::ptr::null_mut(),
        )
    };
    if rc == ERROR_SUCCESS {
        unsafe { RegCloseKey(h) };
        Ok(())
    } else {
        Err(format!("RegCreateKeyExW({path}) failed: {rc}"))
    }
}

fn reg_set_raw(path: &str, value_name: &str, typ: u32, data: &[u8], view: RegView) -> Result<(), String> {
    let (root, sub) = split_reg_path(path)?;
    let w_sub = to_wide(sub);
    let w_val = to_wide(value_name);
    let mut h: HKEY = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            root, w_sub.as_ptr(), 0, std::ptr::null_mut(), 0,
            KEY_SET_VALUE | view.sam(), std::ptr::null(), &mut h, std::ptr::null_mut(),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(format!("RegCreateKeyExW({path}) failed: {rc}"));
    }
    let val_ptr = if value_name.is_empty() { std::ptr::null() } else { w_val.as_ptr() };
    let rc2 = unsafe { RegSetValueExW(h, val_ptr, 0, typ, data.as_ptr(), data.len() as u32) };
    unsafe { RegCloseKey(h) };
    if rc2 == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("RegSetValueExW({path}\\{value_name}) failed: {rc2}"))
    }
}

pub fn reg_set_string(path: &str, value_name: &str, value: &str, view: RegView) -> Result<(), String> {
    let w = to_wide(value);
    let bytes: Vec<u8> = w.iter().flat_map(|u| u.to_le_bytes()).collect();
    reg_set_raw(path, value_name, REG_SZ, &bytes, view)
}

pub fn reg_set_expand_string(path: &str, value_name: &str, value: &str, view: RegView) -> Result<(), String> {
    let w = to_wide(value);
    let bytes: Vec<u8> = w.iter().flat_map(|u| u.to_le_bytes()).collect();
    reg_set_raw(path, value_name, REG_EXPAND_SZ, &bytes, view)
}

pub fn reg_set_dword(path: &str, value_name: &str, value: u32, view: RegView) -> Result<(), String> {
    reg_set_raw(path, value_name, REG_DWORD, &value.to_le_bytes(), view)
}

pub fn reg_set_multi_sz(path: &str, value_name: &str, items: &[&str], view: RegView) -> Result<(), String> {
    let mut w: Vec<u16> = Vec::new();
    for item in items {
        w.extend(item.encode_utf16());
        w.push(0);
    }
    w.push(0);
    let bytes: Vec<u8> = w.iter().flat_map(|u| u.to_le_bytes()).collect();
    reg_set_raw(path, value_name, REG_MULTI_SZ, &bytes, view)
}

pub fn reg_delete_value(path: &str, value_name: &str, view: RegView) {
    let Ok((root, sub)) = split_reg_path(path) else {
        return;
    };
    let w_sub = to_wide(sub);
    let w_val = to_wide(value_name);
    let mut h: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(root, w_sub.as_ptr(), 0, KEY_SET_VALUE | view.sam(), &mut h) } == ERROR_SUCCESS {
        unsafe {
            RegDeleteValueW(h, w_val.as_ptr());
            RegCloseKey(h);
        }
    }
}

pub fn reg_delete_tree(path: &str, view: RegView) {
    let Ok((root, sub)) = split_reg_path(path) else {
        return;
    };
    let w_sub = to_wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(root, w_sub.as_ptr(), 0, KEY_ALL_ACCESS | view.sam(), &mut h) } == ERROR_SUCCESS {
        unsafe {
            RegDeleteTreeW(h, std::ptr::null());
            RegCloseKey(h);
        }
    }
    unsafe { RegDeleteKeyExW(root, w_sub.as_ptr(), view.sam(), 0) };
}

pub fn reg_enum_subkeys(path: &str, view: RegView) -> Vec<String> {
    let mut out = Vec::new();
    let Ok((root, sub)) = split_reg_path(path) else {
        return out;
    };
    let w_sub = to_wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(root, w_sub.as_ptr(), 0, KEY_ENUMERATE_SUB_KEYS | view.sam(), &mut h) } != ERROR_SUCCESS {
        return out;
    }
    let mut idx = 0u32;
    loop {
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let rc = unsafe {
            RegEnumKeyExW(
                h, idx, buf.as_mut_ptr(), &mut len,
                std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(),
            )
        };
        if rc == ERROR_NO_MORE_ITEMS || rc != ERROR_SUCCESS {
            break;
        }
        out.push(from_wide(&buf[..len as usize]));
        idx += 1;
    }
    unsafe { RegCloseKey(h) };
    out
}

pub fn stop_service(name: &str, timeout_ms: u64) {
    let w_name = to_wide(name);
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return;
        }
        let svc = OpenServiceW(scm, w_name.as_ptr(), SERVICE_STOP | SERVICE_QUERY_STATUS);
        if !svc.is_null() {
            let query_state = || -> Option<u32> {
                let mut sp: SERVICE_STATUS_PROCESS = std::mem::zeroed();
                let mut needed = 0u32;
                if QueryServiceStatusEx(
                    svc,
                    SC_STATUS_PROCESS_INFO,
                    &mut sp as *mut _ as *mut u8,
                    std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
                    &mut needed,
                ) != 0
                {
                    Some(sp.dw_current_state)
                } else {
                    None
                }
            };

            if query_state() != Some(SERVICE_STOPPED) {
                let mut st: SERVICE_STATUS = std::mem::zeroed();
                let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut st);
                let start = GetTickCount64();
                loop {
                    match query_state() {
                        None | Some(SERVICE_STOPPED) => break,
                        Some(state) if GetTickCount64().saturating_sub(start) > timeout_ms => {
                            log_line(&format!("Warning: stop_service({name}) timed out after {timeout_ms}ms (state={state})"));
                            break;
                        }
                        _ => std::thread::sleep(std::time::Duration::from_millis(150)),
                    }
                }
            }
            CloseServiceHandle(svc);
        }
        CloseServiceHandle(scm);
    }
}

pub fn start_service(name: &str, timeout_ms: u64) -> bool {
    let w_name = to_wide(name);
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            log_line(&format!("Warning: start_service({name}) OpenSCManagerW failed (err={})", GetLastError()));
            return false;
        }
        let svc = OpenServiceW(scm, w_name.as_ptr(), SERVICE_START | SERVICE_QUERY_STATUS);
        let mut ok = false;
        let mut last_err = 0u32;
        if !svc.is_null() {
            let start = GetTickCount64();
            let mut started_req = false;
            loop {
                let mut sp: SERVICE_STATUS_PROCESS = std::mem::zeroed();
                let mut needed = 0u32;
                if QueryServiceStatusEx(
                    svc,
                    SC_STATUS_PROCESS_INFO,
                    &mut sp as *mut _ as *mut u8,
                    std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
                    &mut needed,
                ) != 0
                {
                    if sp.dw_current_state == SERVICE_RUNNING {
                        ok = true;
                        break;
                    }
                    // Wait out pending stop (e.g. NvpWatchdog 15s child teardown) before calling StartServiceW
                    if sp.dw_current_state != SERVICE_STOP_PENDING && !started_req {
                        if StartServiceW(svc, 0, std::ptr::null()) != 0 {
                            started_req = true;
                        } else {
                            let err = GetLastError();
                            last_err = err;
                            if err == ERROR_SERVICE_ALREADY_RUNNING {
                                ok = true;
                                break;
                            } else if err != ERROR_SERVICE_CANNOT_ACCEPT_CTRL {
                                log_line(&format!("Warning: StartServiceW({name}) failed err={err}"));
                                break;
                            }
                        }
                    }
                }
                if GetTickCount64().saturating_sub(start) > timeout_ms {
                    log_line(&format!("Warning: start_service({name}) timed out after {timeout_ms}ms (last_err={last_err})"));
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            CloseServiceHandle(svc);
        } else {
            log_line(&format!("Warning: OpenServiceW({name}) failed (err={})", GetLastError()));
        }
        CloseServiceHandle(scm);
        ok
    }
}

pub fn terminate_processes_by_name(exe_name: &str) {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut pe) != 0 {
            loop {
                let name = from_wide(&pe.sz_exe_file);
                if name.eq_ignore_ascii_case(exe_name) {
                    let hp = OpenProcess(PROCESS_TERMINATE, 0, pe.th32_process_id);
                    if !hp.is_null() {
                        TerminateProcess(hp, 0);
                        CloseHandle(hp);
                    }
                }
                if Process32NextW(snap, &mut pe) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
}

#[derive(Debug, Clone)]
pub struct EnumDevice {
    pub instance_id: String,
    pub desc: String,
    pub hwids: Vec<String>,
    pub driver_key: String,
    pub dev_inst: u32,
}

pub fn enum_pci_nvidia_devices(present_only: bool) -> Vec<EnumDevice> {
    let mut out = Vec::new();
    let flags = if present_only {
        DIGCF_ALLCLASSES | DIGCF_PRESENT
    } else {
        DIGCF_ALLCLASSES
    };
    let pci_w = to_wide("PCI");
    unsafe {
        let devs = SetupDiGetClassDevsW(std::ptr::null(), pci_w.as_ptr(), std::ptr::null_mut(), flags);
        if devs.is_null() || devs as isize == -1 {
            return out;
        }
        let mut did: SP_DEVINFO_DATA = std::mem::zeroed();
        did.cb_size = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        let mut idx = 0u32;
        while SetupDiEnumDeviceInfo(devs, idx, &mut did) != 0 {
            idx += 1;
            let mut id_buf = [0u16; 512];
            if SetupDiGetDeviceInstanceIdW(devs, &did, id_buf.as_mut_ptr(), id_buf.len() as u32, std::ptr::null_mut()) == 0 {
                continue;
            }
            let inst = from_wide(&id_buf);
            if !inst.to_ascii_uppercase().contains("VEN_10DE") {
                continue;
            }
            let desc = read_dev_prop_str(devs, &did, SPDRP_DEVICEDESC).unwrap_or_default();
            let driver_key = read_dev_prop_str(devs, &did, SPDRP_DRIVER).unwrap_or_default();
            let hwids = read_dev_prop_multi_sz(devs, &did, SPDRP_HARDWAREID);
            out.push(EnumDevice {
                instance_id: inst,
                desc,
                hwids,
                driver_key,
                dev_inst: did.dev_inst,
            });
        }
        SetupDiDestroyDeviceInfoList(devs);
    }
    out
}

unsafe fn read_dev_prop_str(devs: HANDLE, did: &SP_DEVINFO_DATA, prop: u32) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut reg_typ = 0u32;
    let mut req = 0u32;
    if SetupDiGetDeviceRegistryPropertyW(
        devs, did, prop, &mut reg_typ,
        buf.as_mut_ptr() as *mut u8, (buf.len() * 2) as u32, &mut req,
    ) != 0 {
        Some(from_wide(&buf))
    } else {
        None
    }
}

unsafe fn read_dev_prop_multi_sz(devs: HANDLE, did: &SP_DEVINFO_DATA, prop: u32) -> Vec<String> {
    let mut buf = [0u16; 2048];
    let mut reg_typ = 0u32;
    let mut req = 0u32;
    if SetupDiGetDeviceRegistryPropertyW(
        devs, did, prop, &mut reg_typ,
        buf.as_mut_ptr() as *mut u8, (buf.len() * 2) as u32, &mut req,
    ) == 0 {
        return Vec::new();
    }
    let mut res = Vec::new();
    let mut start = 0usize;
    let max_u16 = (req as usize / 2).min(buf.len());
    while start < max_u16 {
        if buf[start] == 0 {
            break;
        }
        let end = buf[start..max_u16].iter().position(|&c| c == 0).map(|p| start + p).unwrap_or(max_u16);
        res.push(String::from_utf16_lossy(&buf[start..end]));
        start = end + 1;
    }
    res
}

/// Remove phantom (disconnected / non-present) NVIDIA PCI & HDAudio devices.
pub fn remove_phantom_nvidia_devices() -> usize {
    let mut removed = 0usize;
    unsafe {
        let devs = SetupDiGetClassDevsW(std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), DIGCF_ALLCLASSES);
        if devs.is_null() || devs as isize == -1 {
            return 0;
        }
        let mut did: SP_DEVINFO_DATA = std::mem::zeroed();
        did.cb_size = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        let mut idx = 0u32;
        while SetupDiEnumDeviceInfo(devs, idx, &mut did) != 0 {
            idx += 1;
            let mut id_buf = [0u16; 512];
            if SetupDiGetDeviceInstanceIdW(devs, &did, id_buf.as_mut_ptr(), id_buf.len() as u32, std::ptr::null_mut()) == 0 {
                continue;
            }
            let inst_up = from_wide(&id_buf).to_ascii_uppercase();
            if !(inst_up.starts_with("PCI\\VEN_10DE") || inst_up.starts_with("HDAUDIO\\FUNC_01&VEN_10DE")) {
                continue;
            }
            let mut st = 0u32;
            let mut prob = 0u32;
            let cm = CM_Get_DevNode_Status(&mut st, &mut prob, did.dev_inst, 0);
            if cm == CR_NO_SUCH_DEVNODE {
                let hdr = SP_CLASSINSTALL_HEADER {
                    cb_size: std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                    install_function: DIF_REMOVE,
                };
                if SetupDiSetClassInstallParamsW(devs, &did, &hdr, std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32) != 0
                    && SetupDiCallClassInstaller(DIF_REMOVE, devs, &did) != 0
                {
                    log_line(&format!("Removed phantom device: {inst_up}"));
                    removed += 1;
                }
            }
        }
        SetupDiDestroyDeviceInfoList(devs);
    }
    removed
}

/// Remove dangling ROOT\NVMODULETRACKER or ROOT\NVVHCI virtual devices if present.
pub fn remove_dangling_root_devices(prefixes: &[&str]) -> usize {
    let mut removed = 0usize;
    unsafe {
        let devs = SetupDiGetClassDevsW(std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), DIGCF_ALLCLASSES);
        if devs.is_null() || devs as isize == -1 {
            return 0;
        }
        let mut did: SP_DEVINFO_DATA = std::mem::zeroed();
        did.cb_size = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        let mut idx = 0u32;
        while SetupDiEnumDeviceInfo(devs, idx, &mut did) != 0 {
            idx += 1;
            let mut id_buf = [0u16; 512];
            if SetupDiGetDeviceInstanceIdW(devs, &did, id_buf.as_mut_ptr(), id_buf.len() as u32, std::ptr::null_mut()) == 0 {
                continue;
            }
            let inst_up = from_wide(&id_buf).to_ascii_uppercase();
            if prefixes.iter().any(|p| inst_up.starts_with(&p.to_ascii_uppercase())) {
                let hdr = SP_CLASSINSTALL_HEADER {
                    cb_size: std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                    install_function: DIF_REMOVE,
                };
                if SetupDiSetClassInstallParamsW(devs, &did, &hdr, std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32) != 0
                    && SetupDiCallClassInstaller(DIF_REMOVE, devs, &did) != 0
                {
                    log_line(&format!("Removed dangling virtual device: {inst_up}"));
                    removed += 1;
                }
            }
        }
        SetupDiDestroyDeviceInfoList(devs);
    }
    removed
}

/// Reproduces DisplayDriverExt.dll's CheckDriverInterruptModeIssue / FixDriverInterruptModeIssue:
/// If RMIntrDisableMsixOnNextReload != 0 and MSISupported == 1, resets MSISupported = 0 and restarts GPU.
/// Returns `true` if a device restart failed/requires reboot.
pub fn check_and_fix_msi_interrupt_mode() -> bool {
    let mut needs_reboot = false;
    let devs_info = enum_pci_nvidia_devices(true);
    for dev in devs_info {
        let dev_params_key = format!(r"HKLM\SYSTEM\CurrentControlSet\Enum\{}\Device Parameters", dev.instance_id);
        let flag = reg_read_dword(&dev_params_key, "RMIntrDisableMsixOnNextReload", RegView::Native64).unwrap_or(0);
        if flag == 0 {
            continue;
        }
        let msi_key = format!(r"{dev_params_key}\Interrupt Management\MessageSignaledInterruptProperties");
        if reg_read_dword(&msi_key, "MSISupported", RegView::Native64) == Some(1) {
            log_line(&format!("RMIntrDisableMsixOnNextReload active on {}; disabling MSISupported", dev.instance_id));
            if reg_set_dword(&msi_key, "MSISupported", 0, RegView::Native64).is_ok() {
                if !restart_device_by_instance(&dev.instance_id) {
                    needs_reboot = true;
                }
            } else {
                needs_reboot = true;
            }
        }
    }
    needs_reboot
}

fn restart_device_by_instance(instance_id: &str) -> bool {
    let w_inst = to_wide(instance_id);
    unsafe {
        let devs = SetupDiCreateDeviceInfoList(std::ptr::null(), std::ptr::null_mut());
        if devs.is_null() || devs as isize == -1 {
            return false;
        }
        let mut did: SP_DEVINFO_DATA = std::mem::zeroed();
        did.cb_size = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        let ok = if SetupDiOpenDeviceInfoW(devs, w_inst.as_ptr(), std::ptr::null_mut(), 0, &mut did) != 0 {
            let params = SP_PROPCHANGE_PARAMS {
                class_install_header: SP_CLASSINSTALL_HEADER {
                    cb_size: std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                    install_function: DIF_PROPERTYCHANGE,
                },
                state_change: DICS_PROPCHANGE,
                scope: DICS_FLAG_GLOBAL,
                hw_profile: 0,
            };
            SetupDiSetClassInstallParamsW(
                devs, &did,
                &params.class_install_header,
                std::mem::size_of::<SP_PROPCHANGE_PARAMS>() as u32,
            ) != 0 && SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, devs, &did) != 0
        } else {
            false
        };
        SetupDiDestroyDeviceInfoList(devs);
        ok
    }
}

pub fn resolve_driver_store_location(inf_path_or_oem: &str) -> Option<PathBuf> {
    let w = to_wide(inf_path_or_oem);
    let mut buf = [0u16; 520];
    let mut req = 0u32;
    let ok = unsafe {
        SetupGetInfDriverStoreLocationW(
            w.as_ptr(), std::ptr::null(), std::ptr::null(),
            buf.as_mut_ptr(), buf.len() as u32, &mut req,
        )
    };
    if ok != 0 {
        let p = PathBuf::from(from_wide(&buf));
        if let Some(parent) = p.parent() {
            if parent.is_dir() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

pub fn uninstall_msi_by_upgrade_codes(upgrade_codes: &[&str]) {
    unsafe {
        let mut prev_hwnd: HWND = std::ptr::null_mut();
        MsiSetInternalUI(2 /* INSTALLUILEVEL_NONE */, &mut prev_hwnd);
        for code in upgrade_codes {
            let w_code = to_wide(code);
            let mut idx = 0u32;
            loop {
                let mut prod_buf = [0u16; 64];
                let rc = MsiEnumRelatedProductsW(w_code.as_ptr(), 0, idx, prod_buf.as_mut_ptr());
                if rc != 0 {
                    break;
                }
                let prod = from_wide(&prod_buf);
                log_line(&format!("Uninstalling old MSI product {prod} (UpgradeCode {code})"));
                let _ = MsiConfigureProductExW(prod_buf.as_ptr(), 0, 2 /* INSTALLSTATE_ABSENT */, std::ptr::null());
                idx += 1;
            }
        }
    }
}

/// Collect basic Windows OS build, display version, and Secure Boot state for logs.
pub fn read_windows_diag_info() -> (String, String, Option<u32>) {
    let cv = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = reg_read_string(cv, "CurrentBuildNumber", RegView::Native64)
        .or_else(|| reg_read_string(cv, "CurrentBuild", RegView::Native64))
        .unwrap_or_else(|| "?".to_string());
    let ubr = reg_read_dword(cv, "UBR", RegView::Native64).unwrap_or(0);
    let disp = reg_read_string(cv, "DisplayVersion", RegView::Native64)
        .or_else(|| reg_read_string(cv, "ReleaseId", RegView::Native64))
        .unwrap_or_else(|| "?".to_string());
    let sb = reg_read_dword(
        r"HKLM\SYSTEM\CurrentControlSet\Control\SecureBoot\State",
        "UEFISecureBootEnabled",
        RegView::Native64,
    );
    (format!("{build}.{ubr}"), disp, sb)
}

/// Query CM_Get_DevNode_Status for all present PCI\VEN_10DE GPUs: returns Vec<(instance_id, status, problem_code)>.
pub fn check_nvidia_gpu_devnode_status() -> Vec<(String, u32, u32)> {
    let mut res = Vec::new();
    unsafe {
        let devs = SetupDiGetClassDevsW(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        );
        if devs.is_null() || devs as isize == -1 {
            return res;
        }
        let mut did: SP_DEVINFO_DATA = std::mem::zeroed();
        did.cb_size = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        let mut idx = 0u32;
        while SetupDiEnumDeviceInfo(devs, idx, &mut did) != 0 {
            idx += 1;
            let mut id_buf = [0u16; 512];
            if SetupDiGetDeviceInstanceIdW(devs, &did, id_buf.as_mut_ptr(), id_buf.len() as u32, std::ptr::null_mut()) == 0 {
                continue;
            }
            let inst_up = from_wide(&id_buf);
            if !inst_up.to_ascii_uppercase().starts_with("PCI\\VEN_10DE") {
                continue;
            }
            let mut st = 0u32;
            let mut prob = 0u32;
            let _ = CM_Get_DevNode_Status(&mut st, &mut prob, did.dev_inst, 0);
            res.push((inst_up, st, prob));
        }
        SetupDiDestroyDeviceInfoList(devs);
    }
    res
}

/// Enable SE_SHUTDOWN_NAME and trigger clean system reboot.
pub fn reboot_system_now() -> bool {
    const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
    const TOKEN_QUERY: u32 = 0x0008;
    const SE_PRIVILEGE_ENABLED: u32 = 0x0000_0002;
    const EWX_REBOOT: u32 = 0x0000_0002;

    #[repr(C)]
    struct LUID_AND_ATTRIBUTES {
        luid: u64,
        attributes: u32,
    }
    #[repr(C)]
    struct TOKEN_PRIVILEGES {
        privilege_count: u32,
        privileges: [LUID_AND_ATTRIBUTES; 1],
    }

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) != 0 {
            let mut luid = 0u64;
            let name = to_wide("SeShutdownPrivilege");
            if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) != 0 {
                let tp = TOKEN_PRIVILEGES {
                    privilege_count: 1,
                    privileges: [LUID_AND_ATTRIBUTES { luid, attributes: SE_PRIVILEGE_ENABLED }],
                };
                AdjustTokenPrivileges(token, 0, &tp as *const _ as *const c_void, 0, std::ptr::null_mut(), std::ptr::null_mut());
            }
            CloseHandle(token);
        }
        ExitWindowsEx(EWX_REBOOT, 0x0002_0004 /* SHTDN_REASON_MAJOR_SOFTWARE | MINOR_INSTALLATION */) != 0
    }
}

/// Best-effort immediate removal of everything inside `dir` (except locked running EXE),
/// then schedule delayed `cmd.exe /d /s /c` deletion of the remaining directory with CWD safely in `System32`.
pub fn schedule_dir_self_cleanup(dir: &Path, remove_empty_parent: bool) {
    use std::os::windows::process::CommandExt;

    let sys32 = system_directory();
    let _ = std::env::set_current_dir(&sys32);

    if let Ok(entries) = fs::read_dir(dir) {
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_dir() {
                let _ = fs::remove_dir_all(&p);
            } else {
                let _ = fs::remove_file(&p);
            }
        }
    }

    let cmd = sys32.join("cmd.exe");
    let target = dir.to_string_lossy();
    let parent_cmd = if remove_empty_parent {
        dir.parent()
            .map(|p| format!(" & rmdir \"{}\" 2>nul", p.to_string_lossy()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let raw = format!(
        "/d /s /c \"ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{target}\" 2>nul{parent_cmd}\""
    );
    let _ = std::process::Command::new(cmd)
        .current_dir(&sys32)
        .raw_arg(&raw)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_windows_cmdline_args() {
        assert_eq!(quote_win_arg("simple"), "simple");
        assert_eq!(quote_win_arg(r"C:\Program Files\Test"), r#""C:\Program Files\Test""#);
        assert_eq!(quote_win_arg(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn live_log_buffer_captures_immediately() {
        reset_session_log(&["HDR_TEST".to_string()]);
        log_line("MID_STEP_LINE");
        let txt = get_live_log_text();
        assert!(txt.contains("HDR_TEST"));
        assert!(txt.contains("MID_STEP_LINE"));
    }

    #[test]
    fn dir_self_cleanup_removes_nested_dir_even_when_cwd_inside() {
        let orig_cwd = std::env::current_dir().ok();
        let parent = std::env::temp_dir().join(format!("ofd cleanup space {}", std::process::id()));
        let target = parent.join("621.40");
        let nested = target.join("Display.Driver");
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&nested).expect("mkdir nested");
        fs::write(nested.join("dummy.inf"), b"ok").expect("write dummy");

        // Reproduce sfx_stub launching setup.exe with CWD inside the directory to be deleted
        std::env::set_current_dir(&target).expect("set cwd inside target");
        schedule_dir_self_cleanup(&target, true);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while parent.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if let Some(orig) = orig_cwd {
            let _ = std::env::set_current_dir(orig);
        }
        let still_there = target.exists() || parent.exists();
        let _ = fs::remove_dir_all(&parent);
        assert!(!still_there, "expected target and empty parent to be deleted cleanly");
    }
}
