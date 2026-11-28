//! OpenFrankenDriver Pure-Rust Self-Extracting Launcher (Variant 03E UI + sevenz-turbo).

#![cfg_attr(not(test), windows_subsystem = "windows")]
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::collections::{HashSet, VecDeque};
use std::ffi::c_void;
use std::fs::{self, File, FileTimes};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::{FileExt, FileTimesExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sevenz_turbo::{ArchiveEntry, ArchiveLimits, ArchiveReader, NtTime, Password, ReadAt};

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HBITMAP = *mut c_void;
type HGDIOBJ = *mut c_void;
type HFONT = *mut c_void;
type HPEN = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl RECT {
    fn w(self) -> i32 {
        self.right - self.left
    }
    fn h(self) -> i32 {
        self.bottom - self.top
    }
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: usize,
    lParam: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct WNDCLASSEXW {
    cbSize: u32,
    style: u32,
    lpfnWndProc: Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HINSTANCE,
    hIcon: HICON,
    hCursor: HCURSOR,
    hbrBackground: HBRUSH,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
    hIconSm: HICON,
}

#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    fErase: i32,
    rcPaint: RECT,
    fRestore: i32,
    fIncUpdate: i32,
    rgbReserved: [u8; 32],
}

#[repr(C)]
struct MONITORINFO {
    cbSize: u32,
    rcMonitor: RECT,
    rcWork: RECT,
    dwFlags: u32,
}

#[repr(C)]
struct CREATESTRUCT {
    lpCreateParams: *mut c_void,
    hInstance: HINSTANCE,
    hMenu: *mut c_void,
    hwndParent: HWND,
    cy: i32,
    cx: i32,
    y: i32,
    x: i32,
    style: i32,
    lpszName: *const u16,
    lpszClass: *const u16,
    dwExStyle: u32,
}

extern "system" {
    fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    fn LoadIconW(hInstance: HINSTANCE, lpIconName: *const u16) -> HICON;
    fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
    fn RegisterClassExW(lpWndClass: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        dwExStyle: u32,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: u32,
        X: i32,
        Y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: HWND,
        hMenu: *mut c_void,
        hInstance: HINSTANCE,
        lpParam: *mut c_void,
    ) -> HWND;
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> i32;
    fn UpdateWindow(hWnd: HWND) -> i32;
    fn DestroyWindow(hWnd: HWND) -> i32;
    fn PostQuitMessage(nExitCode: i32);
    fn DefWindowProcW(hWnd: HWND, Msg: u32, wParam: usize, lParam: isize) -> isize;
    fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: u32, wMsgFilterMax: u32) -> i32;
    fn TranslateMessage(lpMsg: *const MSG) -> i32;
    fn DispatchMessageW(lpMsg: *const MSG) -> isize;
    fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> i32;
    fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> i32;
    fn ScreenToClient(hWnd: HWND, lpPoint: *mut POINT) -> i32;
    fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: i32) -> i32;
    fn SetWindowPos(
        hWnd: HWND,
        hWndInsertAfter: HWND,
        X: i32,
        Y: i32,
        cx: i32,
        cy: i32,
        uFlags: u32,
    ) -> i32;
    fn GetSystemMetrics(nIndex: i32) -> i32;
    fn GetDpiForWindow(hwnd: HWND) -> u32;
    fn MonitorFromWindow(hwnd: HWND, dwFlags: u32) -> *mut c_void;
    fn GetMonitorInfoW(hMonitor: *mut c_void, lpmi: *mut MONITORINFO) -> i32;
    fn SystemParametersInfoW(uiAction: u32, uiParam: u32, pvParam: *mut c_void, fWinIni: u32) -> i32;
    fn SetTimer(hWnd: HWND, nIDEvent: usize, uElapse: u32, lpTimerFunc: *const c_void) -> usize;
    fn KillTimer(hWnd: HWND, uIDEvent: usize) -> i32;
    fn PostMessageW(hWnd: HWND, Msg: u32, wParam: usize, lParam: isize) -> i32;
    fn GetWindowLongPtrW(hWnd: HWND, nIndex: i32) -> isize;
    fn SetWindowLongPtrW(hWnd: HWND, nIndex: i32, dwNewLong: isize) -> isize;
    fn AllowSetForegroundWindow(dwProcessId: u32) -> i32;
    fn MessageBoxW(hWnd: HWND, lpText: *const u16, lpCaption: *const u16, uType: u32) -> i32;

    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> i32;
    fn CreateCompatibleBitmap(hdc: HDC, cx: i32, cy: i32) -> HBITMAP;
    fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(ho: HGDIOBJ) -> i32;
    fn BitBlt(
        hdc: HDC,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: HDC,
        x1: i32,
        y1: i32,
        rop: u32,
    ) -> i32;
    fn CreateSolidBrush(color: u32) -> HBRUSH;
    fn CreatePen(iStyle: i32, cWidth: i32, color: u32) -> HPEN;
    fn FillRect(hDC: HDC, lprc: *const RECT, hbr: HBRUSH) -> i32;
    fn MoveToEx(hdc: HDC, x: i32, y: i32, lppt: *mut POINT) -> i32;
    fn LineTo(hdc: HDC, x: i32, y: i32) -> i32;
    fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    fn SetTextColor(hdc: HDC, color: u32) -> u32;
    fn CreateFontW(
        cHeight: i32,
        cWidth: i32,
        cEscapement: i32,
        cOrientation: i32,
        cWeight: i32,
        bItalic: u32,
        bUnderline: u32,
        bStrikeOut: u32,
        iCharSet: u32,
        iOutPrecision: u32,
        iClipPrecision: u32,
        iQuality: u32,
        iPitchAndFamily: u32,
        pszFaceName: *const u16,
    ) -> HFONT;
    fn DrawTextW(
        hdc: HDC,
        lpchText: *const u16,
        cchText: i32,
        lprc: *mut RECT,
        format: u32,
    ) -> i32;
    fn GetTextExtentPoint32W(hdc: HDC, lpString: *const u16, c: i32, psizl: *mut POINT) -> i32;
    fn DwmSetWindowAttribute(
        hwnd: HWND,
        dwAttribute: u32,
        pvAttribute: *const c_void,
        cbAttribute: u32,
    ) -> i32;
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_SETCURSOR: u32 = 0x0020;
const WM_DISPLAYCHANGE: u32 = 0x007E;
const WM_NCHITTEST: u32 = 0x0084;
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_APP_DONE: u32 = 0x8001;

const HTCAPTION: isize = 2;
const HTCLIENT: isize = 1;
const WS_POPUP: u32 = 0x8000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const SW_SHOW: i32 = 5;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const IDC_HAND: usize = 32649;
const IDC_ARROW: usize = 32512;
const TRANSPARENT: i32 = 1;
const PS_SOLID: i32 = 0;
const DT_LEFT: u32 = 0x0000_0000;
const DT_VCENTER: u32 = 0x0000_0004;
const DT_SINGLELINE: u32 = 0x0000_0020;
const DT_NOPREFIX: u32 = 0x0000_0800;
const DT_PATH_ELLIPSIS: u32 = 0x0000_4000;
const FW_NORMAL: i32 = 400;
const FW_BOLD: i32 = 700;
const MB_OK: u32 = 0x0000_0000;
const MB_ICONERROR: u32 = 0x0000_0010;
const TIMER_UI: usize = 1;

const COL_BG: u32 = rgb(14, 22, 16); // #0E1610 (03E Phosphor Console)
const COL_SCANLINE: u32 = rgb(22, 34, 21); // subtle CRT scanlines
const COL_BORDER: u32 = rgb(54, 70, 56); // #364638
const COL_DIVIDER: u32 = rgb(46, 80, 38); // #2E5026
const COL_TRACK: u32 = rgb(10, 18, 12); // #0A120C
const COL_TRACK_BORDER: u32 = rgb(72, 123, 30); // #487B1E
const COL_GREEN: u32 = rgb(118, 185, 0); // #76B900
const COL_LIME_TEXT: u32 = rgb(141, 248, 128); // #8DF880
const COL_OK_LOG: u32 = rgb(94, 166, 86); // #5EA656
const COL_CUR_LOG: u32 = rgb(184, 236, 76); // #B8EC4C
const COL_DARK_BAR_TXT: u32 = rgb(13, 21, 13); // #0D150D
const COL_WHITE: u32 = rgb(244, 246, 248);
const COL_RED: u32 = rgb(166, 40, 44);
const COL_RED_HOVER: u32 = rgb(198, 50, 55);

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct SfxMeta {
    version: String,
    install_dir: PathBuf,
    archive_offset: u64,
    archive_len: u64,
}

fn find_pe_end(head: &[u8]) -> Result<usize, String> {
    if head.len() < 0x40 || head[0] != b'M' || head[1] != b'Z' {
        return Err("Некорректный PE-заголовок MZ.".to_string());
    }
    let e_lfanew = u32::from_le_bytes(head[0x3C..0x40].try_into().unwrap()) as usize;
    if e_lfanew + 24 > head.len() || &head[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return Err("Некорректный PE-заголовок NT.".to_string());
    }
    let num_sections =
        u16::from_le_bytes(head[e_lfanew + 6..e_lfanew + 8].try_into().unwrap()) as usize;
    let opt_size =
        u16::from_le_bytes(head[e_lfanew + 20..e_lfanew + 22].try_into().unwrap()) as usize;
    let sec_table = e_lfanew + 24 + opt_size;
    let sec_end = sec_table + num_sections * 40;
    if sec_end > head.len() {
        return Err("Таблица секций PE выходит за буфер заголовка.".to_string());
    }
    let mut max_raw = sec_end;
    for i in 0..num_sections {
        let off = sec_table + i * 40;
        let raw_size = u32::from_le_bytes(head[off + 16..off + 20].try_into().unwrap()) as usize;
        let raw_ptr = u32::from_le_bytes(head[off + 20..off + 24].try_into().unwrap()) as usize;
        if raw_ptr > 0 {
            max_raw = max_raw.max(raw_ptr.saturating_add(raw_size));
        }
    }
    Ok(max_raw)
}

fn parse_version_from_exe_name(exe_path: &Path) -> String {
    if let Some(stem) = exe_path.file_stem().and_then(|s| s.to_str()) {
        let bytes = stem.as_bytes();
        for w in bytes.windows(6) {
            if w[0].is_ascii_digit()
                && w[1].is_ascii_digit()
                && w[2].is_ascii_digit()
                && w[3] == b'.'
                && w[4].is_ascii_digit()
                && w[5].is_ascii_digit()
            {
                if let Ok(s) = std::str::from_utf8(w) {
                    return s.to_string();
                }
            }
        }
    }
    "621.40".to_string()
}

fn decode_7z_magic() -> [u8; 6] {
    // Obfuscated bytes so our own .rdata doesn't hold literal 7z signature
    let enc = [
        0x37u8 ^ 0x5A,
        0x7Au8 ^ 0x5A,
        0xBCu8 ^ 0x5A,
        0xAFu8 ^ 0x5A,
        0x27u8 ^ 0x5A,
        0x1Cu8 ^ 0x5A,
    ];
    let mut out = [0u8; 6];
    for i in 0..6 {
        out[i] = enc[i] ^ 0x5A;
    }
    out
}

fn inspect_sfx_exe(exe_path: &Path) -> Result<SfxMeta, String> {
    let mut f = File::open(exe_path).map_err(|e| format!("Ошибка открытия EXE: {e}"))?;
    let file_len = f
        .metadata()
        .map_err(|e| format!("Ошибка чтения размера EXE: {e}"))?
        .len();

    let head_len = (file_len as usize).min(128 * 1024);
    let mut head = vec![0u8; head_len];
    f.read_exact(&mut head)
        .map_err(|e| format!("Ошибка чтения PE: {e}"))?;
    let pe_end = find_pe_end(&head)? as u64;

    if pe_end >= file_len {
        return Err("Внутри SFX-файла не найден встроенный архив драйвера (.7z).".to_string());
    }

    let scan_len = (file_len - pe_end).min(256 * 1024) as usize;
    let mut overlay = vec![0u8; scan_len];
    f.seek(SeekFrom::Start(pe_end))
        .map_err(|e| format!("Seek PE end failed: {e}"))?;
    f.read_exact(&mut overlay)
        .map_err(|e| format!("Read overlay failed: {e}"))?;

    let magic = decode_7z_magic();
    let rel_7z = overlay
        .windows(6)
        .position(|w| w == magic)
        .ok_or_else(|| "Сигнатура 7z архива не найдена после PE-заголовка.".to_string())?;
    let archive_offset = pe_end + rel_7z as u64;

    if archive_offset + 32 > file_len {
        return Err("Повреждён заголовок 7z в оверлее.".to_string());
    }
    let mut hdr_7z = [0u8; 32];
    f.seek(SeekFrom::Start(archive_offset))
        .map_err(|e| format!("Seek 7z failed: {e}"))?;
    f.read_exact(&mut hdr_7z)
        .map_err(|e| format!("Read 7z StartHeader failed: {e}"))?;
    let next_offset = u64::from_le_bytes(hdr_7z[12..20].try_into().unwrap());
    let next_size = u64::from_le_bytes(hdr_7z[20..28].try_into().unwrap());
    let declared_len = 32u64
        .saturating_add(next_offset)
        .saturating_add(next_size);
    let max_avail = file_len - archive_offset;
    let archive_len = declared_len.min(max_avail);

    // Optional SFX config block between PE end and 7z signature: `;!@OFD@!1\n...\n;!@OFD_END@!`
    let mut version = parse_version_from_exe_name(exe_path);
    let mut install_dir = PathBuf::from(format!(r"C:\NVIDIA\OpenFrankenDriver\{version}"));

    let meta_slice = &overlay[..rel_7z];
    if let Ok(meta_txt) = std::str::from_utf8(meta_slice) {
        if let Some(start) = meta_txt.find(";!@OFD@!1") {
            let sub = &meta_txt[start..];
            let body = sub.split(";!@OFD_END@!").next().unwrap_or(sub);
            for line in body.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("version=") {
                    if !v.trim().is_empty() {
                        version = v.trim().to_string();
                    }
                } else if let Some(d) = line.strip_prefix("install_dir=") {
                    if !d.trim().is_empty() {
                        install_dir = PathBuf::from(d.trim());
                    }
                }
            }
        }
    }

    Ok(SfxMeta {
        version,
        install_dir,
        archive_offset,
        archive_len,
    })
}

/// Bounded positional ReadAt source isolating the embedded `.7z` stream from preceding PE bytes and trailing Authenticode signatures.
struct SliceReadAt {
    file: File,
    base: u64,
    len: u64,
}

impl SliceReadAt {
    fn open(path: &Path, base: u64, len: u64) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            base,
            len,
        })
    }
}

impl ReadAt for SliceReadAt {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let rem = (self.len - offset) as usize;
        let to_read = buf.len().min(rem);
        self.file.seek_read(&mut buf[..to_read], self.base + offset)
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.len)
    }
}

fn safe_join(dest: &Path, entry_name: &str) -> Result<PathBuf, String> {
    let norm = entry_name.replace('\\', "/");
    let mut out = dest.to_path_buf();
    for c in Path::new(&norm).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Небезопасный путь в архиве: {entry_name}"));
            }
        }
    }
    if out == dest {
        return Err(format!("Пустое имя записи в архиве: {entry_name}"));
    }
    Ok(out)
}

fn clear_readonly_if_exists(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

fn nt_to_system_time(nt: NtTime) -> Option<std::time::SystemTime> {
    let raw: u64 = nt.into();
    if raw == 0 || raw >= (1u64 << 63) {
        return None;
    }
    Some(std::time::SystemTime::from(nt))
}

fn apply_entry_times(file: &File, entry: &ArchiveEntry) {
    let mut file_times = FileTimes::new();
    if let Some(time) = nt_to_system_time(entry.access_date()) {
        file_times = file_times.set_accessed(time);
    }
    if let Some(time) = nt_to_system_time(entry.last_modified_date()) {
        file_times = file_times.set_modified(time);
    }
    if let Some(time) = nt_to_system_time(entry.creation_date()) {
        file_times = file_times.set_created(time);
    }
    let _ = file.set_times(file_times);
}

struct UiShared {
    current_file: Mutex<String>,
    recent_files: Mutex<VecDeque<String>>,
    error: Mutex<Option<String>>,
    bytes_done: AtomicU64,
    total_bytes: AtomicU64,
    cancel: AtomicBool,
    done: AtomicBool,
}

fn extract_archive(
    exe_path: &Path,
    meta: &SfxMeta,
    shared: &Arc<UiShared>,
) -> Result<(), String> {
    let ra = SliceReadAt::open(exe_path, meta.archive_offset, meta.archive_len)
        .map_err(|e| format!("Ошибка открытия потока 7z: {e}"))?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(8)
        .clamp(1, 32);

    let mut ar = ArchiveReader::from_read_at(ra, Password::empty(), ArchiveLimits::unlimited())
        .map_err(|e| format!("Ошибка разбора 7z архива: {e}"))?
        .with_threads(threads);
    ar.set_input_complete(true);

    let total: u64 = ar
        .archive()
        .files
        .iter()
        .filter(|f| !f.is_directory)
        .map(|f| f.size)
        .sum();
    shared.total_bytes.store(total.max(1), Ordering::Relaxed);

    fs::create_dir_all(&meta.install_dir).map_err(|e| {
        format!(
            "Не удалось создать папку {}: {e}",
            meta.install_dir.display()
        )
    })?;

    let mut created_dirs: HashSet<PathBuf> = HashSet::new();
    created_dirs.insert(meta.install_dir.clone());
    let mut copy_buf = vec![0u8; 256 * 1024];
    let mut cancelled = false;

    ar.for_each_entries(|entry, reader| {
        if shared.cancel.load(Ordering::Relaxed) {
            cancelled = true;
            return Ok(false);
        }
        let dest_path = safe_join(&meta.install_dir, entry.name()).map_err(|msg| {
            sevenz_turbo::Error::Other(msg.into())
        })?;

        if entry.is_directory() {
            if created_dirs.insert(dest_path.clone()) {
                fs::create_dir_all(&dest_path)?;
            }
            return Ok(true);
        }

        let norm_name = entry.name().replace('/', "\\");
        let prev_name = shared.current_file.lock().ok().map(|mut g| {
            let prev = g.clone();
            *g = norm_name.clone();
            prev
        });
        if let Some(prev) = prev_name {
            if !prev.is_empty() && prev != norm_name {
                if let Ok(mut rec) = shared.recent_files.lock() {
                    if rec.back().map(|s| s != &prev).unwrap_or(true) {
                        rec.push_back(prev);
                        while rec.len() > 2 {
                            rec.pop_front();
                        }
                    }
                }
            }
        }
        if let Some(parent) = dest_path.parent() {
            if created_dirs.insert(parent.to_path_buf()) {
                fs::create_dir_all(parent)?;
            }
        }
        clear_readonly_if_exists(&dest_path);

        let mut out = File::create(&dest_path)?;
        loop {
            if shared.cancel.load(Ordering::Relaxed) {
                cancelled = true;
                return Ok(false);
            }
            let n = reader.read(&mut copy_buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&copy_buf[..n])?;
            shared.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
        }
        out.flush()?;
        apply_entry_times(&out, entry);
        Ok(true)
    })
    .map_err(|e| format!("Ошибка распаковки: {e}"))?;

    if cancelled || shared.cancel.load(Ordering::Relaxed) {
        return Err("Распаковка отменена пользователем.".to_string());
    }
    Ok(())
}

struct AppState {
    install_dir_str: String,
    dpi: i32,
    last_work_w: i32,
    last_work_h: i32,
    hover_close: bool,
    close_rect: RECT,
    started_at: Instant,
    shared: Arc<UiShared>,
}

/// Remove a half-extracted temporary package directory if cancelled or failed mid-way.
fn cleanup_partial_unpack_dir(dir: &Path) {
    let norm = dir.to_string_lossy().to_ascii_lowercase();
    if norm.contains(r"\nvidia\openfrankendriver\") && dir.is_dir() {
        let _ = fs::remove_dir_all(dir);
    }
}

impl AppState {
    fn s(&self, px: i32) -> i32 {
        (px * self.dpi + 48) / 96
    }
}

unsafe fn fill_rect(hdc: HDC, rc: RECT, color: u32) {
    let br = CreateSolidBrush(color);
    FillRect(hdc, &rc, br);
    DeleteObject(br as HGDIOBJ);
}

unsafe fn make_font(st: &AppState, pt: i32, weight: i32, mono: bool) -> HFONT {
    let h = -((pt * st.dpi + 36) / 72);
    let face = if mono { "Consolas" } else { "Segoe UI" };
    let wide = to_wide(face);
    CreateFontW(
        h,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        1,
        0,
        0,
        5, // CLEARTYPE_QUALITY
        0,
        wide.as_ptr(),
    )
}

unsafe fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
    let old = SelectObject(hdc, font as HGDIOBJ);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut sz = POINT { x: 0, y: 0 };
    if !wide.is_empty() {
        GetTextExtentPoint32W(hdc, wide.as_ptr(), wide.len() as i32, &mut sz);
    }
    SelectObject(hdc, old);
    sz.x
}

unsafe fn draw_text(
    hdc: HDC,
    text: &str,
    mut rc: RECT,
    font: HFONT,
    color: u32,
    flags: u32,
) {
    let old = SelectObject(hdc, font as HGDIOBJ);
    SetTextColor(hdc, color);
    let wide = to_wide(text);
    DrawTextW(hdc, wide.as_ptr(), -1, &mut rc, flags);
    SelectObject(hdc, old);
}

// Mirrored 5x7 matrix katakana glyphs (matches main installer aesthetic, zero font dependency)
const KATA_5X7: [[u8; 7]; 8] = [
    [0b11111, 0b10000, 0b10010, 0b00010, 0b00010, 0b00100, 0b01000],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b00010, 0b00010, 0b00010],
    [0b00100, 0b11111, 0b10001, 0b10000, 0b10000, 0b01000, 0b00110],
    [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111],
    [0b00100, 0b11111, 0b10100, 0b10100, 0b10100, 0b10010, 0b01001],
    [0b00100, 0b11111, 0b00100, 0b11111, 0b00100, 0b00110, 0b01100],
    [0b11111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
    [0b01010, 0b11111, 0b01010, 0b01010, 0b01000, 0b00100, 0b00010],
];

unsafe fn draw_frame_rect(hdc: HDC, rc: RECT, color: u32) {
    let pen = CreatePen(PS_SOLID, 1, color);
    let old_p = SelectObject(hdc, pen as HGDIOBJ);
    MoveToEx(hdc, rc.left, rc.top, std::ptr::null_mut());
    LineTo(hdc, rc.right - 1, rc.top);
    LineTo(hdc, rc.right - 1, rc.bottom - 1);
    LineTo(hdc, rc.left, rc.bottom - 1);
    LineTo(hdc, rc.left, rc.top);
    SelectObject(hdc, old_p);
    DeleteObject(pen as HGDIOBJ);
}

unsafe fn render_sfx_03e(hdc: HDC, cw: i32, ch: i32, st: &mut AppState) {
    // 1) 03E Phosphor Console dark green background + 1px outer frame
    let cr = RECT { left: 0, top: 0, right: cw, bottom: ch };
    fill_rect(hdc, cr, COL_BG);

    // 2) Subtle Matrix backdrop rain + horizontal CRT scanlines
    let t = st.started_at.elapsed().as_secs_f64();
    let px_s = st.s(1).max(1);
    let col_step = st.s(26).max(18);
    let n_cols = (cw / col_step).max(1);
    for i in 0..n_cols {
        let mx = st.s(12) + i * col_step;
        let spd = 28.0 + ((i * 11) % 22) as f64;
        let head_y = (((t * spd + (i * 37) as f64) % ((ch + st.s(40)) as f64)) as i32) - st.s(12);
        for k in 0..4i32 {
            let yy = head_y - k * st.s(11);
            if yy < st.s(2) || yy > ch - st.s(8) {
                continue;
            }
            let drop_col = match k {
                0 => rgb(52, 86, 42),
                1 => rgb(34, 58, 28),
                2 => rgb(25, 42, 22),
                _ => rgb(19, 31, 18),
            };
            let g = KATA_5X7[((i as usize) * 3 + (k as usize) + (t * 3.0) as usize) % KATA_5X7.len()];
            for (r, &row_bits) in g.iter().enumerate() {
                for c_i in 0..5 {
                    if (row_bits >> (4 - c_i)) & 1 != 0 {
                        let rx = mx + c_i * px_s;
                        let ry = yy + (r as i32) * px_s;
                        fill_rect(hdc, RECT { left: rx, top: ry, right: rx + px_s, bottom: ry + px_s }, drop_col);
                    }
                }
            }
        }
    }

    let scan_step = st.s(3).max(2);
    let mut sy = st.s(4);
    while sy < ch - st.s(2) {
        fill_rect(hdc, RECT { left: 1, top: sy, right: cw - 1, bottom: sy + 1 }, COL_SCANLINE);
        sy += scan_step;
    }

    draw_frame_rect(hdc, cr, COL_BORDER);

    // 3) Left 4px solid NVIDIA Green #76B900 edge stripe (03E signature)
    let left_bar_w = st.s(4).max(3);
    fill_rect(hdc, RECT { left: 0, top: 0, right: left_bar_w, bottom: ch }, COL_GREEN);

    // 4) Close [×] button top-right
    let hdr_h = st.s(30);
    let close_w = st.s(30);
    let close_rc = RECT {
        left: cw - 1 - close_w,
        top: 1,
        right: cw - 1,
        bottom: hdr_h,
    };
    st.close_rect = close_rc;
    fill_rect(
        hdc,
        close_rc,
        if st.hover_close { COL_RED_HOVER } else { COL_RED },
    );
    {
        let xp = CreatePen(PS_SOLID, st.s(2).max(1), COL_WHITE);
        let old_xp = SelectObject(hdc, xp as HGDIOBJ);
        let mx = (close_rc.left + close_rc.right) / 2;
        let my = (close_rc.top + close_rc.bottom) / 2;
        let d = st.s(4);
        MoveToEx(hdc, mx - d, my - d, std::ptr::null_mut());
        LineTo(hdc, mx + d + 1, my + d + 1);
        MoveToEx(hdc, mx + d, my - d, std::ptr::null_mut());
        LineTo(hdc, mx - d - 1, my + d + 1);
        SelectObject(hdc, old_xp);
        DeleteObject(xp as HGDIOBJ);
    }

    let f_path = make_font(st, 10, FW_BOLD, true);
    let f_log = make_font(st, 9, FW_NORMAL, true);
    let f_log_cur = make_font(st, 9, FW_BOLD, true);
    let f_bar = make_font(st, 10, FW_BOLD, true);

    let pad_x = st.s(15);
    let content_r = cw - st.s(15);

    // 5) Top Path line (NO ">" prefix!) + horizontal divider #2E5026
    draw_text(
        hdc,
        &st.install_dir_str,
        RECT {
            left: pad_x,
            top: st.s(4),
            right: close_rc.left - st.s(8),
            bottom: hdr_h,
        },
        f_path,
        COL_LIME_TEXT,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_PATH_ELLIPSIS | DT_NOPREFIX,
    );
    let div_y = hdr_h + st.s(2);
    fill_rect(hdc, RECT { left: pad_x, top: div_y, right: content_r, bottom: div_y + 1 }, COL_DIVIDER);

    // 6) Three console unpacking rows: two [OK] + active [..] █ (ZERO ">" arrows!)
    let recent: Vec<String> = st
        .shared
        .recent_files
        .lock()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default();
    let cur_file = st
        .shared
        .current_file
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    let ok1 = recent.first().cloned();
    let ok2 = recent.get(1).cloned();
    let mut ly = div_y + st.s(9);
    let row_h = st.s(18);

    if let Some(f1) = ok1 {
        draw_text(
            hdc,
            &format!("[OK] {f1}"),
            RECT { left: pad_x, top: ly, right: content_r, bottom: ly + row_h },
            f_log,
            COL_OK_LOG,
            DT_LEFT | DT_SINGLELINE | DT_PATH_ELLIPSIS | DT_NOPREFIX,
        );
    }
    ly += row_h;

    if let Some(f2) = ok2 {
        draw_text(
            hdc,
            &format!("[OK] {f2}"),
            RECT { left: pad_x, top: ly, right: content_r, bottom: ly + row_h },
            f_log,
            COL_OK_LOG,
            DT_LEFT | DT_SINGLELINE | DT_PATH_ELLIPSIS | DT_NOPREFIX,
        );
    }
    ly += row_h;

    let cursor_on = ((t * 3.0) as u64).is_multiple_of(2);
    let base_cur = if cur_file.is_empty() {
        "[..] Подготовка архива...".to_string()
    } else {
        format!("[..] {cur_file}")
    };
    let cur_txt = if cursor_on {
        format!("{base_cur} █")
    } else {
        base_cur
    };
    draw_text(
        hdc,
        &cur_txt,
        RECT { left: pad_x, top: ly, right: content_r, bottom: ly + row_h },
        f_log_cur,
        COL_CUR_LOG,
        DT_LEFT | DT_SINGLELINE | DT_PATH_ELLIPSIS | DT_NOPREFIX,
    );

    // 7) Framed 03E Bottom Progress Bar (#0A120C track, #487B1E border, #76B900 fill, #0D150D stats inside)
    let total_b = st.shared.total_bytes.load(Ordering::Relaxed).max(1);
    let done_b = st.shared.bytes_done.load(Ordering::Relaxed).min(total_b);
    let frac = (done_b as f64 / total_b as f64).clamp(0.0, 1.0);
    let pct = (frac * 100.0).floor().min(100.0) as u32;
    let mb_done = done_b / (1024 * 1024);
    let mb_tot = (total_b / (1024 * 1024)).max(1);
    let stat_txt = format!("{pct}%  •  {mb_done}/{mb_tot} МБ");

    let bar_rc = RECT {
        left: pad_x,
        top: ch - st.s(36),
        right: content_r,
        bottom: ch - st.s(14),
    };
    fill_rect(hdc, bar_rc, COL_TRACK);
    draw_frame_rect(hdc, bar_rc, COL_TRACK_BORDER);

    let inner_pad = st.s(2).max(2);
    let inner_w = (bar_rc.w() - inner_pad * 2).max(1);
    let fill_w = ((inner_w as f64) * frac).round() as i32;
    if fill_w > 0 {
        fill_rect(
            hdc,
            RECT {
                left: bar_rc.left + inner_pad,
                top: bar_rc.top + inner_pad,
                right: bar_rc.left + inner_pad + fill_w,
                bottom: bar_rc.bottom - inner_pad,
            },
            COL_GREEN,
        );
    }

    let stat_w = text_width(hdc, f_bar, &stat_txt);
    let text_inside_fill = fill_w >= stat_w + st.s(14);
    draw_text(
        hdc,
        &stat_txt,
        RECT {
            left: bar_rc.left + st.s(8),
            top: bar_rc.top,
            right: bar_rc.right - st.s(8),
            bottom: bar_rc.bottom,
        },
        f_bar,
        if text_inside_fill { COL_DARK_BAR_TXT } else { COL_LIME_TEXT },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );

    for f in [f_path, f_log, f_log_cur, f_bar] {
        DeleteObject(f as HGDIOBJ);
    }
}

unsafe fn paint_window(hwnd: HWND, hdc_screen: HDC, st: &mut AppState) {
    let mut cr = RECT::default();
    GetClientRect(hwnd, &mut cr);
    let cw = cr.w().max(1);
    let ch = cr.h().max(1);

    let mem = CreateCompatibleDC(hdc_screen);
    let bmp = CreateCompatibleBitmap(hdc_screen, cw, ch);
    let old_bmp = SelectObject(mem, bmp as HGDIOBJ);
    SetBkMode(mem, TRANSPARENT);

    render_sfx_03e(mem, cw, ch, st);

    BitBlt(hdc_screen, 0, 0, cw, ch, mem, 0, 0, 0x00CC_0020);
    SelectObject(mem, old_bmp);
    DeleteObject(bmp as HGDIOBJ);
    DeleteDC(mem);
}

unsafe fn get_state<'a>(hwnd: HWND) -> Option<&'a mut AppState> {
    let p = GetWindowLongPtrW(hwnd, -21) as *mut AppState;
    if p.is_null() {
        None
    } else {
        Some(&mut *p)
    }
}

fn pt_in_rect(rc: RECT, x: i32, y: i32) -> bool {
    x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom
}

unsafe fn get_monitor_work_area(hwnd: HWND) -> RECT {
    if !hwnd.is_null() {
        let hmon = MonitorFromWindow(hwnd, 2);
        if !hmon.is_null() {
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(hmon, &mut mi) != 0 && mi.rcWork.w() > 0 && mi.rcWork.h() > 0 {
                return mi.rcWork;
            }
        }
    }
    let mut work = RECT::default();
    if SystemParametersInfoW(0x0030, 0, &mut work as *mut RECT as *mut c_void, 0) != 0
        && work.w() > 0
        && work.h() > 0
    {
        return work;
    }
    RECT {
        left: 0,
        top: 0,
        right: GetSystemMetrics(SM_CXSCREEN).max(640),
        bottom: GetSystemMetrics(SM_CYSCREEN).max(480),
    }
}

fn clamp_dpi_to_work(raw_dpi: i32, work_rc: RECT) -> i32 {
    let raw = if raw_dpi > 0 { raw_dpi } else { 96 };
    let max_w = (work_rc.w().max(320) * 94 * 96) / (100 * 520);
    let max_h = (work_rc.h().max(180) * 94 * 96) / (100 * 156);
    raw.min(max_w.min(max_h).max(68)).max(68)
}

unsafe fn recenter_and_fit_window(hwnd: HWND, st: &mut AppState, raw_hint: i32, force_center: bool) {
    let work_rc = get_monitor_work_area(hwnd);
    let raw_dpi = if raw_hint > 0 {
        raw_hint
    } else {
        GetDpiForWindow(hwnd) as i32
    };
    let next_dpi = clamp_dpi_to_work(raw_dpi, work_rc);
    let changed = work_rc.w() != st.last_work_w || work_rc.h() != st.last_work_h;
    if force_center || changed || next_dpi != st.dpi {
        st.dpi = next_dpi;
        st.last_work_w = work_rc.w();
        st.last_work_h = work_rc.h();
        let ww = st.s(520);
        let wh = st.s(156);
        let wx = work_rc.left + ((work_rc.w() - ww) / 2).max(0);
        let wy = work_rc.top + ((work_rc.h() - wh) / 2).max(0);
        SetWindowPos(hwnd, std::ptr::null_mut(), wx, wy, ww, wh, 0x0014);
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if msg == WM_CREATE {
        let cs = &*(lparam as *const CREATESTRUCT);
        SetWindowLongPtrW(hwnd, -21, cs.lpCreateParams as isize);
        if let Some(st) = get_state(hwnd) {
            recenter_and_fit_window(hwnd, st, 0, true);
            SetTimer(hwnd, TIMER_UI, 33, std::ptr::null());
        }
        return 0;
    }

    let Some(st) = get_state(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };

    match msg {
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => {
            let def = DefWindowProcW(hwnd, msg, wparam, lparam);
            if def == HTCLIENT {
                let mut pt = POINT {
                    x: (lparam as i16) as i32,
                    y: ((lparam >> 16) as i16) as i32,
                };
                ScreenToClient(hwnd, &mut pt);
                if pt.y >= 0 && pt.y < st.s(34) && !pt_in_rect(st.close_rect, pt.x, pt.y) {
                    return HTCAPTION;
                }
            }
            def
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as i32;
            let work_rc = get_monitor_work_area(hwnd);
            if clamp_dpi_to_work(new_dpi, work_rc) != new_dpi {
                recenter_and_fit_window(hwnd, st, new_dpi, true);
            } else if new_dpi > 0 {
                st.dpi = new_dpi;
                st.last_work_w = work_rc.w();
                st.last_work_h = work_rc.h();
                let suggested = &*(lparam as *const RECT);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.w(),
                    suggested.h(),
                    0x0014,
                );
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_DISPLAYCHANGE => {
            recenter_and_fit_window(hwnd, st, 0, true);
            0
        }
        WM_SETTINGCHANGE => {
            recenter_and_fit_window(hwnd, st, 0, false);
            0
        }
        WM_SETCURSOR => {
            if (lparam & 0xFFFF) == 1 && st.hover_close {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND as *const u16));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let x = (lparam as i16) as i32;
            let y = ((lparam >> 16) as i16) as i32;
            let hv = pt_in_rect(st.close_rect, x, y);
            if hv != st.hover_close {
                st.hover_close = hv;
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam as i16) as i32;
            let y = ((lparam >> 16) as i16) as i32;
            if pt_in_rect(st.close_rect, x, y) {
                st.shared.cancel.store(true, Ordering::Relaxed);
                DestroyWindow(hwnd);
                return 0;
            }
            0
        }
        WM_TIMER => {
            InvalidateRect(hwnd, std::ptr::null(), 0);
            if st.shared.done.load(Ordering::Relaxed) {
                AllowSetForegroundWindow(0xFFFF_FFFF);
                DestroyWindow(hwnd);
            }
            0
        }
        WM_APP_DONE => {
            AllowSetForegroundWindow(0xFFFF_FFFF);
            DestroyWindow(hwnd);
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_window(hwnd, hdc, st);
            EndPaint(hwnd, &ps);
            0
        }
        WM_CLOSE => {
            st.shared.cancel.store(true, Ordering::Relaxed);
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_UI);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn run_gui_unpack(exe_path: PathBuf, meta: SfxMeta, forwarded_args: Vec<String>) -> i32 {
    let shared = Arc::new(UiShared {
        current_file: Mutex::new(String::new()),
        recent_files: Mutex::new(VecDeque::new()),
        error: Mutex::new(None),
        bytes_done: AtomicU64::new(0),
        total_bytes: AtomicU64::new(1),
        cancel: AtomicBool::new(false),
        done: AtomicBool::new(false),
    });

    let state = Box::new(AppState {
        install_dir_str: meta.install_dir.to_string_lossy().to_string(),
        dpi: 96,
        last_work_w: 0,
        last_work_h: 0,
        hover_close: false,
        close_rect: RECT::default(),
        started_at: Instant::now(),
        shared: Arc::clone(&shared),
    });

    let bg_handle = unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls_name = to_wide("OpenFrankenDriverSfx03EW");
        let hicon = LoadIconW(hinst, 101usize as *const u16);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0x0003,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: hicon,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW as *const u16),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls_name.as_ptr(),
            hIconSm: hicon,
        };
        RegisterClassExW(&wc);

        let ww = 520;
        let wh = 156;
        let sx = GetSystemMetrics(SM_CXSCREEN);
        let sy = GetSystemMetrics(SM_CYSCREEN);
        let wx = ((sx - ww) / 2).max(20);
        let wy = ((sy - wh) / 2).max(20);

        let title = to_wide(&format!("openfrankendriver {}", meta.version));
        let ptr = Box::into_raw(state);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            cls_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_VISIBLE,
            wx,
            wy,
            ww,
            wh,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            ptr as *mut c_void,
        );
        if hwnd.is_null() {
            let _ = Box::from_raw(ptr);
            return 1;
        }
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            20,
            &dark as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let hwnd_raw = hwnd as usize;
        let shared_bg = Arc::clone(&shared);
        let meta_bg = SfxMeta {
            version: meta.version.clone(),
            install_dir: meta.install_dir.clone(),
            archive_offset: meta.archive_offset,
            archive_len: meta.archive_len,
        };
        let handle = std::thread::spawn(move || {
            if let Err(e) = extract_archive(&exe_path, &meta_bg, &shared_bg) {
                if let Ok(mut err_g) = shared_bg.error.lock() {
                    *err_g = Some(e);
                }
            }
            shared_bg.done.store(true, Ordering::Release);
            PostMessageW(hwnd_raw as HWND, WM_APP_DONE, 0, 0);
        });

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = Box::from_raw(ptr);
        handle
    };

    if shared.cancel.load(Ordering::Relaxed) {
        let _ = bg_handle.join();
        cleanup_partial_unpack_dir(&meta.install_dir);
        return 1;
    }
    if let Ok(guard) = shared.error.lock() {
        if let Some(ref err) = *guard {
            let err_msg = err.clone();
            drop(guard);
            let _ = bg_handle.join();
            cleanup_partial_unpack_dir(&meta.install_dir);
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    to_wide(&err_msg).as_ptr(),
                    to_wide("openfrankendriver — Ошибка распаковки").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
            return 1;
        }
    }

    let setup_exe = meta.install_dir.join("setup.exe");
    unsafe {
        AllowSetForegroundWindow(0xFFFF_FFFF);
    }
    match Command::new(&setup_exe)
        .current_dir(&meta.install_dir)
        .args(&forwarded_args)
        .spawn()
    {
        Ok(child) => {
            unsafe {
                AllowSetForegroundWindow(child.id());
            }
            0
        }
        Err(e) => {
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    to_wide(&format!(
                        "Не удалось запустить {}:\n{e}",
                        setup_exe.display()
                    ))
                    .as_ptr(),
                    to_wide("openfrankendriver — Ошибка запуска").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let is_silent = args.iter().any(|a| {
        a.eq_ignore_ascii_case("--silent")
            || a.eq_ignore_ascii_case("-s")
            || a.eq_ignore_ascii_case("/silent")
    });

    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            if !is_silent {
                unsafe {
                    MessageBoxW(
                        std::ptr::null_mut(),
                        to_wide(&format!("current_exe failed: {e}")).as_ptr(),
                        to_wide("openfrankendriver").as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            std::process::exit(1);
        }
    };

    let meta = match inspect_sfx_exe(&exe_path) {
        Ok(m) => m,
        Err(e) => {
            if !is_silent {
                unsafe {
                    MessageBoxW(
                        std::ptr::null_mut(),
                        to_wide(&e).as_ptr(),
                        to_wide("openfrankendriver").as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            std::process::exit(1);
        }
    };

    if is_silent {
        let shared = Arc::new(UiShared {
            current_file: Mutex::new(String::new()),
            recent_files: Mutex::new(VecDeque::new()),
            error: Mutex::new(None),
            bytes_done: AtomicU64::new(0),
            total_bytes: AtomicU64::new(1),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
        });
        if extract_archive(&exe_path, &meta, &shared).is_err() {
            cleanup_partial_unpack_dir(&meta.install_dir);
            std::process::exit(1);
        }
        let setup_exe = meta.install_dir.join("setup.exe");
        let code = match Command::new(&setup_exe)
            .current_dir(&meta.install_dir)
            .args(&args)
            .status()
        {
            Ok(st) => st.code().unwrap_or(0),
            Err(_) => 1,
        };
        std::process::exit(code);
    }

    let code = run_gui_unpack(exe_path, meta, args);
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_from_exe_filename_and_decodes_magic() {
        assert_eq!(
            parse_version_from_exe_name(Path::new(r"C:\drivers\OpenFrankenDriver_v2_Beta_621.40.exe")),
            "621.40"
        );
        let m = decode_7z_magic();
        assert_eq!(&m, &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]);
    }

    #[test]
    fn finds_overlay_meta_and_bounded_7z_with_trailing_signature() {
        let dir = std::env::temp_dir().join("ofd_sfx_test_tmp");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Build minimal synthetic PE header + 1 section ending at raw_end=512
        let mut pe = vec![0u8; 512];
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        pe[0x86..0x88].copy_from_slice(&1u16.to_le_bytes()); // 1 section
        pe[0x94..0x96].copy_from_slice(&0x70u16.to_le_bytes()); // opt header size
        let sec = 0x80 + 24 + 0x70;
        pe[sec + 16..sec + 20].copy_from_slice(&256u32.to_le_bytes()); // SizeOfRawData
        pe[sec + 20..sec + 24].copy_from_slice(&256u32.to_le_bytes()); // PointerToRawData -> raw_end = 512

        // Inject false 7z magic INSIDE PE section data to prove PE boundary skipping works!
        let m = decode_7z_magic();
        pe[260..266].copy_from_slice(&m);

        let cfg = b";!@OFD@!1\nversion=625.10\ninstall_dir=C:\\NVIDIA\\OpenFrankenDriver\\625.10\n;!@OFD_END@!\n";
        let mut fake_7z = [0u8; 64];
        fake_7z[0..6].copy_from_slice(&m);
        fake_7z[12..20].copy_from_slice(&20u64.to_le_bytes()); // next_offset
        fake_7z[20..28].copy_from_slice(&12u64.to_le_bytes()); // next_size -> total archive = 32+20+12 = 64

        let fake_authenticode_sig = vec![0xAAu8; 128];

        let test_exe = dir.join("dummy_sfx.exe");
        let mut all = Vec::new();
        all.extend_from_slice(&pe);
        all.extend_from_slice(cfg);
        all.extend_from_slice(&fake_7z);
        all.extend_from_slice(&fake_authenticode_sig);
        fs::write(&test_exe, all).unwrap();

        let meta = inspect_sfx_exe(&test_exe).unwrap();
        assert_eq!(meta.version, "625.10");
        assert_eq!(meta.install_dir, PathBuf::from(r"C:\NVIDIA\OpenFrankenDriver\625.10"));
        assert_eq!(meta.archive_offset, 512 + cfg.len() as u64);
        assert_eq!(meta.archive_len, 64);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn render_sfx_03e_screenshot() {
        #[repr(C)]
        struct BitmapInfoHeader {
            bi_size: u32,
            bi_width: i32,
            bi_height: i32,
            bi_planes: u16,
            bi_bit_count: u16,
            bi_compression: u32,
            bi_size_image: u32,
            bi_x_pels: i32,
            bi_y_pels: i32,
            bi_clr_used: u32,
            bi_clr_important: u32,
        }
        #[repr(C)]
        struct BitmapInfo {
            bmi_header: BitmapInfoHeader,
            bmi_colors: [u32; 3],
        }
        extern "system" {
            fn GetDC(hwnd: HWND) -> HDC;
            fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
            fn CreateDIBSection(
                hdc: HDC,
                pbmi: *const BitmapInfo,
                usage: u32,
                ppv_bits: *mut *mut c_void,
                h_section: *mut c_void,
                offset: u32,
            ) -> HBITMAP;
            fn GdiFlush() -> i32;
        }

        let cw = 520i32;
        let ch = 156i32;
        let mut rec = VecDeque::new();
        rec.push_back(r"Display.Driver\nv_dispi.inf".to_string());
        rec.push_back(r"Display.Driver\nvwgf2umx.dll".to_string());
        let shared = Arc::new(UiShared {
            current_file: Mutex::new(r"Display.Driver\nvlddmkm.sys".to_string()),
            recent_files: Mutex::new(rec),
            error: Mutex::new(None),
            bytes_done: AtomicU64::new(412 * 1024 * 1024),
            total_bytes: AtomicU64::new(645 * 1024 * 1024),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
        });
        let mut st = AppState {
            install_dir_str: r"C:\NVIDIA\OpenFrankenDriver\621.40".to_string(),
            dpi: 96,
            last_work_w: 0,
            last_work_h: 0,
            hover_close: false,
            close_rect: RECT::default(),
            started_at: Instant::now(),
            shared,
        };

        unsafe {
            let scr_dc = GetDC(std::ptr::null_mut());
            let mem = CreateCompatibleDC(scr_dc);
            let bmi = BitmapInfo {
                bmi_header: BitmapInfoHeader {
                    bi_size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    bi_width: cw,
                    bi_height: -ch,
                    bi_planes: 1,
                    bi_bit_count: 32,
                    bi_compression: 0,
                    bi_size_image: (cw * ch * 4) as u32,
                    bi_x_pels: 0,
                    bi_y_pels: 0,
                    bi_clr_used: 0,
                    bi_clr_important: 0,
                },
                bmi_colors: [0; 3],
            };
            let mut bits_ptr: *mut c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(scr_dc, &bmi, 0, &mut bits_ptr, std::ptr::null_mut(), 0);
            assert!(!dib.is_null() && !bits_ptr.is_null());
            let old_bmp = SelectObject(mem, dib as HGDIOBJ);
            SetBkMode(mem, TRANSPARENT);

            render_sfx_03e(mem, cw, ch, &mut st);
            GdiFlush();

            let px_bytes = (cw * ch * 4) as usize;
            let file_size = 54 + px_bytes;
            let mut buf = Vec::with_capacity(file_size);
            buf.extend_from_slice(b"BM");
            buf.extend_from_slice(&(file_size as u32).to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&54u32.to_le_bytes());
            buf.extend_from_slice(&40u32.to_le_bytes());
            buf.extend_from_slice(&cw.to_le_bytes());
            buf.extend_from_slice(&(-ch).to_le_bytes());
            buf.extend_from_slice(&1u16.to_le_bytes());
            buf.extend_from_slice(&32u16.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&(px_bytes as u32).to_le_bytes());
            buf.extend_from_slice(&0i32.to_le_bytes());
            buf.extend_from_slice(&0i32.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(std::slice::from_raw_parts(bits_ptr as *const u8, px_bytes));
            let out_dir = std::env::var_os("OFD_SCREENSHOT_OUT")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join("ofd_screenshots"));
            let _ = fs::create_dir_all(&out_dir);
            fs::write(out_dir.join("sfx_03e.bmp"), &buf).unwrap();

            SelectObject(mem, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), scr_dc);
        }
    }

    #[test]
    fn safe_join_blocks_traversal_and_readonly_overwrite_works() {
        let root = std::env::temp_dir().join("ofd_sfx_ro_test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        assert!(safe_join(&root, "../evil.exe").is_err());
        assert!(safe_join(&root, r"C:\Windows\System32\cmd.exe").is_err());
        assert!(safe_join(&root, "/etc/passwd").is_err());
        assert_eq!(
            safe_join(&root, "Display.Driver/nv_dispi.inf").unwrap(),
            root.join("Display.Driver").join("nv_dispi.inf")
        );

        let fpath = root.join("ro_test.txt");
        fs::write(&fpath, b"old").unwrap();
        let mut perms = fs::metadata(&fpath).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&fpath, perms).unwrap();

        clear_readonly_if_exists(&fpath);
        fs::write(&fpath, b"new").unwrap();
        assert_eq!(fs::read(&fpath).unwrap(), b"new");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore]
    fn verify_real_monolithic_exe_extracts() {
        let exe = Path::new(r"C:\drivers\OpenFrankenDriver_v2_Beta_621.40.exe");
        assert!(exe.is_file(), "Monolithic exe missing: {}", exe.display());
        let meta = inspect_sfx_exe(exe).expect("inspect_sfx_exe failed on signed monolithic exe");
        assert_eq!(meta.version, "621.40");
        assert_eq!(meta.install_dir, PathBuf::from(r"C:\NVIDIA\OpenFrankenDriver\621.40"));

        let out_dir = std::env::temp_dir().join("ofd_verify_turbo_extract");
        let _ = fs::remove_dir_all(&out_dir);
        let shared = Arc::new(UiShared {
            current_file: Mutex::new(String::new()),
            recent_files: Mutex::new(VecDeque::new()),
            error: Mutex::new(None),
            bytes_done: AtomicU64::new(0),
            total_bytes: AtomicU64::new(1),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
        });
        let extract_meta = SfxMeta {
            version: meta.version.clone(),
            install_dir: out_dir.clone(),
            archive_offset: meta.archive_offset,
            archive_len: meta.archive_len,
        };

        let t0 = std::time::Instant::now();
        extract_archive(exe, &extract_meta, &shared).expect("sevenz-turbo full extraction failed");
        let elapsed = t0.elapsed().as_secs_f64();

        assert!(out_dir.join("setup.exe").is_file(), "setup.exe missing after extract");
        assert!(out_dir.join("Display.Driver").join("nv_dispi.inf").is_file(), "nv_dispi.inf missing");
        assert!(out_dir.join("PhysX").join("physx_manifest.json").is_file(), "physx_manifest.json missing");
        let pkg_meta = fs::read_to_string(out_dir.join("Showcase").join("package_meta.json")).expect("package_meta.json");
        assert!(pkg_meta.contains("621.40"), "package_meta.json unexpected: {pkg_meta}");

        let bytes_done = shared.bytes_done.load(Ordering::Relaxed);
        let total_bytes = shared.total_bytes.load(Ordering::Relaxed);
        assert_eq!(bytes_done, total_bytes, "bytes_done != total_bytes");
        eprintln!(
            "REAL_MONOLITHIC_VERIFY_OK: version={}, elapsed={:.3}s, bytes={}/{}, install_dir={}",
            meta.version,
            elapsed,
            bytes_done,
            total_bytes,
            meta.install_dir.display()
        );
        let _ = fs::remove_dir_all(&out_dir);
    }
}
