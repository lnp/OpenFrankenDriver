//! Pure Win32 API + double-buffered GDI / GDI+ GUI (PerMonitorV2 DPI aware).
//! Resilient to GPU display-stack restarts during pnputil driver installation.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::gpu::{PackageMeta, SystemSummary};
use crate::installer::{self, InstallOptions, InstallReport};
use crate::slides::SlideItem;
use crate::uninstaller;
use crate::win32::{self, to_wide, HWND};

type HINSTANCE = *mut c_void;
type HDC = *mut c_void;
type HBITMAP = *mut c_void;
type HBRUSH = *mut c_void;
type HPEN = *mut c_void;
type HFONT = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HGDIOBJ = *mut c_void;

const WS_POPUP: u32 = 0x8000_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const WS_BORDERLESS_SQUARE: u32 = WS_POPUP | WS_MINIMIZEBOX | WS_VISIBLE;

const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_DISPLAYCHANGE: u32 = 0x007E;
const WM_NCHITTEST: u32 = 0x0084;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_KEYDOWN: u32 = 0x0100;
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_SETCURSOR: u32 = 0x0020;
const WM_APP_WORKER: u32 = 0x8001;

const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;

const VK_LEFT: usize = 0x25;
const VK_RIGHT: usize = 0x27;
const VK_ESCAPE: usize = 0x1B;
const VK_RETURN: usize = 0x0D;

const TIMER_SLIDE: usize = 1;
const TIMER_POLL: usize = 2;
const TIMER_CRT: usize = 3;
const TIMER_ANIM: usize = 4;

const ETO_OPAQUE: u32 = 0x0002;

const SW_SHOW: i32 = 5;
const SW_MINIMIZE: i32 = 6;
const PS_SOLID: i32 = 0;
const TRANSPARENT: i32 = 1;
const DEFAULT_CHARSET: u32 = 1;
const CLEARTYPE_QUALITY: u32 = 5;
const FW_NORMAL: i32 = 400;
const FW_SEMIBOLD: i32 = 600;
const FW_BOLD: i32 = 700;

const DT_LEFT: u32 = 0x0000;
const DT_CENTER: u32 = 0x0001;
const DT_RIGHT: u32 = 0x0002;
const DT_VCENTER: u32 = 0x0004;
const DT_WORDBREAK: u32 = 0x0010;
const DT_SINGLELINE: u32 = 0x0020;
const DT_END_ELLIPSIS: u32 = 0x8000;
const DT_NOPREFIX: u32 = 0x0800;

const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const IDC_ARROW: usize = 32512;
const IDC_HAND: usize = 32649;
const MB_OK: u32 = 0;
const MB_ICONERROR: u32 = 0x10;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl RECT {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    fn w(&self) -> i32 {
        self.right - self.left
    }
    fn h(&self) -> i32 {
        self.bottom - self.top
    }
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MONITORINFO {
    cbSize: u32,
    rcMonitor: RECT,
    rcWork: RECT,
    dwFlags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct POINTF {
    x: f32,
    y: f32,
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
struct GdiplusStartupInput {
    gdiplus_version: u32,
    debug_event_callback: usize,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

extern "system" {
    fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    fn LoadIconW(hInstance: HINSTANCE, lpIconName: *const u16) -> HICON;
    fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
    fn RegisterClassExW(lpWndClass: *const WNDCLASSEXW) -> u16;
    fn AdjustWindowRectEx(lpRect: *mut RECT, dwStyle: u32, bMenu: i32, dwExStyle: u32) -> i32;
    fn CreateWindowExW(
        dwExStyle: u32, lpClassName: *const u16, lpWindowName: *const u16, dwStyle: u32,
        X: i32, Y: i32, nWidth: i32, nHeight: i32,
        hWndParent: HWND, hMenu: *mut c_void, hInstance: HINSTANCE, lpParam: *mut c_void,
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
    fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    fn SetCapture(hWnd: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
    fn ShellExecuteW(
        hwnd: HWND, lpOperation: *const u16, lpFile: *const u16,
        lpParameters: *const u16, lpDirectory: *const u16, nShowCmd: i32,
    ) -> *mut c_void;
    fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: i32) -> i32;
    fn SetWindowPos(hWnd: HWND, hWndInsertAfter: HWND, X: i32, Y: i32, cx: i32, cy: i32, uFlags: u32) -> i32;
    fn GetForegroundWindow() -> HWND;
    fn SetForegroundWindow(hWnd: HWND) -> i32;
    fn BringWindowToTop(hWnd: HWND) -> i32;
    fn SetActiveWindow(hWnd: HWND) -> HWND;
    fn SetFocus(hWnd: HWND) -> HWND;
    fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut u32) -> u32;
    fn AttachThreadInput(idAttach: u32, idAttachTo: u32, fAttach: i32) -> i32;
    fn GetCurrentThreadId() -> u32;
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
    fn MessageBoxW(hWnd: HWND, lpText: *const u16, lpCaption: *const u16, uType: u32) -> i32;

    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> i32;
    fn CreateCompatibleBitmap(hdc: HDC, cx: i32, cy: i32) -> HBITMAP;
    fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(ho: HGDIOBJ) -> i32;
    fn BitBlt(hdc: HDC, x: i32, y: i32, cx: i32, cy: i32, hdcSrc: HDC, x1: i32, y1: i32, rop: u32) -> i32;
    fn CreateSolidBrush(color: u32) -> HBRUSH;
    fn CreatePen(iStyle: i32, cWidth: i32, color: u32) -> HPEN;
    fn FillRect(hDC: HDC, lprc: *const RECT, hbr: HBRUSH) -> i32;
    fn RoundRect(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32, width: i32, height: i32) -> i32;
    fn Ellipse(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn MoveToEx(hdc: HDC, x: i32, y: i32, lppt: *mut POINT) -> i32;
    fn LineTo(hdc: HDC, x: i32, y: i32) -> i32;
    fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    fn SetBkColor(hdc: HDC, color: u32) -> u32;
    fn SetTextColor(hdc: HDC, color: u32) -> u32;
    fn ExtTextOutW(
        hdc: HDC, x: i32, y: i32, options: u32, lprc: *const RECT,
        lpString: *const u16, c: u32, lpDx: *const i32,
    ) -> i32;
    fn SaveDC(hdc: HDC) -> i32;
    fn RestoreDC(hdc: HDC, nSavedDC: i32) -> i32;
    fn IntersectClipRect(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn CreateFontW(
        cHeight: i32, cWidth: i32, cEscapement: i32, cOrientation: i32, cWeight: i32,
        bItalic: u32, bUnderline: u32, bStrikeOut: u32, iCharSet: u32, iOutPrecision: u32,
        iClipPrecision: u32, iQuality: u32, iPitchAndFamily: u32, pszFaceName: *const u16,
    ) -> HFONT;
    fn DrawTextW(hdc: HDC, lpchText: *const u16, cchText: i32, lprc: *mut RECT, format: u32) -> i32;
    fn GetTextExtentPoint32W(hdc: HDC, lpString: *const u16, c: i32, psizl: *mut POINT) -> i32;

    fn DwmSetWindowAttribute(hwnd: HWND, dwAttribute: u32, pvAttribute: *const c_void, cbAttribute: u32) -> i32;

    fn SHCreateMemStream(pInit: *const u8, cbInit: u32) -> *mut c_void;
    fn GdiplusStartup(token: *mut usize, input: *const GdiplusStartupInput, output: *mut c_void) -> i32;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateFromHDC(hdc: HDC, graphics: *mut *mut c_void) -> i32;
    fn GdipDeleteGraphics(graphics: *mut c_void) -> i32;
    fn GdipSetInterpolationMode(graphics: *mut c_void, mode: i32) -> i32;
    fn GdipLoadImageFromStream(stream: *mut c_void, image: *mut *mut c_void) -> i32;
    fn GdipDisposeImage(image: *mut c_void) -> i32;
    fn GdipGetImageWidth(image: *mut c_void, width: *mut u32) -> i32;
    fn GdipGetImageHeight(image: *mut c_void, height: *mut u32) -> i32;
    fn GdipDrawImageRectI(graphics: *mut c_void, image: *mut c_void, x: i32, y: i32, width: i32, height: i32) -> i32;
    fn GdipDrawImageRectRectI(
        graphics: *mut c_void, image: *mut c_void,
        dstx: i32, dsty: i32, dstwidth: i32, dstheight: i32,
        srcx: i32, srcy: i32, srcwidth: i32, srcheight: i32,
        srcUnit: i32, imageAttributes: *const c_void, callback: *const c_void, callbackData: *const c_void,
    ) -> i32;
    fn GdipCreateLineBrushI(
        point1: *const POINT, point2: *const POINT, color1: u32, color2: u32,
        wrapMode: i32, lineGradient: *mut *mut c_void,
    ) -> i32;
    fn GdipFillRectangleI(graphics: *mut c_void, brush: *mut c_void, x: i32, y: i32, width: i32, height: i32) -> i32;
    fn GdipDeleteBrush(brush: *mut c_void) -> i32;
    fn GdipSetSmoothingMode(graphics: *mut c_void, smoothingMode: i32) -> i32;
    fn GdipCreateSolidFill(color: u32, brush: *mut *mut c_void) -> i32;
    fn GdipCreatePen1(color: u32, width: f32, unit: i32, pen: *mut *mut c_void) -> i32;
    fn GdipSetPenStartCap(pen: *mut c_void, startCap: i32) -> i32;
    fn GdipSetPenEndCap(pen: *mut c_void, endCap: i32) -> i32;
    fn GdipSetPenLineJoin(pen: *mut c_void, lineJoin: i32) -> i32;
    fn GdipDeletePen(pen: *mut c_void) -> i32;
    fn GdipFillEllipse(graphics: *mut c_void, brush: *mut c_void, x: f32, y: f32, width: f32, height: f32) -> i32;
    fn GdipDrawEllipse(graphics: *mut c_void, pen: *mut c_void, x: f32, y: f32, width: f32, height: f32) -> i32;
    fn GdipFillPolygon(graphics: *mut c_void, brush: *mut c_void, points: *const POINTF, count: i32, fillMode: i32) -> i32;
    fn GdipDrawPolygon(graphics: *mut c_void, pen: *mut c_void, points: *const POINTF, count: i32) -> i32;
    fn GdipDrawLine(graphics: *mut c_void, pen: *mut c_void, x1: f32, y1: f32, x2: f32, y2: f32) -> i32;
    fn GdipDrawBezier(
        graphics: *mut c_void, pen: *mut c_void,
        x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32, x4: f32, y4: f32,
    ) -> i32;
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

const fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

const COL_BG: u32 = rgb(22, 25, 29);
const COL_SIDEBAR: u32 = rgb(22, 25, 29);
const COL_CARD: u32 = rgb(28, 32, 38);
const COL_CARD_HOVER: u32 = rgb(36, 42, 50);
const COL_BORDER: u32 = rgb(46, 54, 63);
const COL_OUTER_BORDER: u32 = rgb(56, 65, 75);
const COL_GREEN: u32 = rgb(118, 185, 0);
const COL_GREEN_HOVER: u32 = rgb(134, 206, 8);
const COL_GREEN_DARK: u32 = rgb(62, 102, 0);
const COL_AMBER: u32 = rgb(224, 159, 42);
const COL_RED: u32 = rgb(166, 40, 44);
const COL_RED_HOVER: u32 = rgb(198, 50, 55);
const COL_TEXT: u32 = rgb(244, 246, 248);
const COL_MUTED: u32 = rgb(146, 156, 166);
const COL_DARK_TXT: u32 = rgb(14, 18, 12);
const COL_CRT_BG: u32 = rgb(8, 12, 10);
const COL_CRT_BORDER: u32 = rgb(28, 42, 32);
const COL_CRT_PH: u32 = rgb(110, 225, 130);
const COL_CRT_DIM: u32 = rgb(58, 122, 72);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Components,
    Installing,
    Done,
    UninstallConfirm,
    Uninstalling,
    UninstallDone,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitId {
    None,
    BtnCopyLog,
    BtnGitHub,
    BtnWinMin,
    BtnWinClose,
    TabCrtSys,
    TabCrtLog,
    ChkNvcpl,
    ChkHdaudio,
    ChkPhysx,
    ChkVcpp,
    ChkNvpi,
    ChkClean,
    BtnInstall,
    BtnCancel,
    DotTrack,
    BtnDonePrimary,
    BtnDoneClose,
    BtnUninstallConfirm,
    BtnUninstallBack,
}

struct HitRect {
    id: HitId,
    rect: RECT,
}

struct LoadedImage {
    gp_img: *mut c_void,
    w: u32,
    h: u32,
}

impl Drop for LoadedImage {
    fn drop(&mut self) {
        if !self.gp_img.is_null() {
            unsafe { GdipDisposeImage(self.gp_img) };
            self.gp_img = std::ptr::null_mut();
        }
    }
}

struct WorkerProgress {
    percent: u32,
    status: String,
    crt_lines: Vec<String>,
    install_res: Option<Result<InstallReport, String>>,
    uninstall_res: Option<Result<bool, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CatBehavior {
    SleepPc,
    PcBelly,
    WakePc,
    HopOffPc,
    PrepJumpPc,
    JumpToPc,
    BellyBliss,
    PlayCaught,
    RecoverBelly,
    AmbushDrop,
    AmbushCursor,
    PounceCatch,
    PounceCursor,
    SwatCursor,
    ChaseRun,
    SidePounce,
    SitWatchUp,
    WallReach,
    HopGrab,
    RearBatLog,
    Walk,
    Loaf,
    Groom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CatchPlayMode {
    BunnyKicks = 0,
    PawJuggle = 1,
    MouthShake = 2,
    FloorRoll = 3,
    HighToss = 4,
    OrbitSpin = 5,
    PounceSlam = 6,
}

#[derive(Clone, Copy)]
struct LogShard {
    x: f64,
    y: f64,
    tx: f64,
    ty: f64,
    vx: f64,
    vy: f64,
    life: f64,
    max_life: f64,
    is_bright: bool,
}

#[derive(Clone, Copy)]
struct HeartParticle {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    size: f32,
    life: f64,
    tint_idx: usize,
}

#[derive(Clone, Copy)]
struct ShatterSlot {
    row: usize,
    col: usize,
    life: f64,
}

struct CaughtRibbon {
    glyph_idx: usize,
    is_red: bool,
    mode: CatchPlayMode,
    vx: f64,
    vy: f64,
    nodes_x: [f64; 6],
    nodes_y: [f64; 6],
}

struct CatState {
    x: f64,
    y: f64,
    on_pc: bool,
    target_x: f64,
    dir: f64,
    behavior: CatBehavior,
    timer: f64,
    phase: f64,
    sleep_timer: f64,
    hop_from_x: f64,
    hop_from_y: f64,
    hop_p: f64,
    paw_lift: f64,
    rear_amt: f64,
    belly_roll: f64,
    pending_belly_t: f64,
    belly_t: f64,
    hind_step_phase: f64,
    step_vel: f64,
    walk_cycle: f64,
    hip_sway: f64,
    lean_ang: f64,
    active_paw: i32,
    paw_extend: f64,
    poke_flick: f64,
    poke_wait: f64,
    jump_y: f64,
    jump_v: f64,
    jump_vx: f64,
    hop_cooldown: f64,
    pc_return_cooldown: f64,
    wants_pc_jump: bool,
    low_cycle_t: f64,
    low_play_budget: f64,
    low_rest_t: f64,
    tease_time: f64,
    target_col_idx: usize,
    next_play_mode: usize,
    interesting_seq: u32,
    caught: Option<CaughtRibbon>,
    poke_times_ms: Vec<u64>,
    boosty_banner_t: f64,
    blink_t: f64,
    rng_state: u32,
    mouse_active: bool,
    mouse_x: f64,
    mouse_y: f64,
    mouse_speed: f64,
    last_move_ms: u64,
    bored_timeout_ms: u64,
    cursor_bored: bool,
    last_tick_ms: u64,
    paw_tip_x: f64,
    paw_tip_y: f64,
    paw_touching: bool,
    rain_init: bool,
    rain_y: [f64; 40],
    rain_speed: [f64; 40],
    rain_red: [bool; 40],
    rain_interesting: [bool; 40],
    rain_sway_x: [f64; 40],
    rain_sway_v: [f64; 40],
    rain_flash: [f64; 40],
    shards: Vec<LogShard>,
    shattered: Vec<ShatterSlot>,
    hearts: Vec<HeartParticle>,
}

impl CatState {
    fn new(now_ms: u64) -> Self {
        let mut s = Self {
            x: 243.0,
            y: 514.0,
            on_pc: true,
            target_x: 110.0,
            dir: -1.0,
            behavior: CatBehavior::SleepPc,
            timer: 7.5,
            phase: 0.0,
            sleep_timer: 7.5,
            hop_from_x: 243.0,
            hop_from_y: 514.0,
            hop_p: 0.0,
            paw_lift: 0.0,
            rear_amt: 0.0,
            belly_roll: 0.0,
            pending_belly_t: 0.0,
            belly_t: 0.0,
            hind_step_phase: 0.0,
            step_vel: 0.0,
            walk_cycle: 0.0,
            hip_sway: 0.0,
            lean_ang: 0.0,
            active_paw: 1,
            paw_extend: 0.0,
            poke_flick: 0.0,
            poke_wait: 0.4,
            jump_y: 0.0,
            jump_v: 0.0,
            jump_vx: 0.0,
            hop_cooldown: 0.0,
            pc_return_cooldown: 14.0,
            wants_pc_jump: false,
            low_cycle_t: 0.0,
            low_play_budget: 4.8,
            low_rest_t: 0.0,
            tease_time: 0.0,
            target_col_idx: 8,
            next_play_mode: 0,
            interesting_seq: 1,
            caught: None,
            poke_times_ms: Vec::new(),
            boosty_banner_t: 0.0,
            blink_t: 2.2,
            rng_state: 0x9E37_79B9,
            mouse_active: false,
            mouse_x: 0.0,
            mouse_y: 0.0,
            mouse_speed: 0.0,
            last_move_ms: now_ms,
            bored_timeout_ms: 11_400,
            cursor_bored: false,
            last_tick_ms: now_ms,
            paw_tip_x: 118.0,
            paw_tip_y: 500.0,
            paw_touching: false,
            rain_init: false,
            rain_y: [0.0; 40],
            rain_speed: [34.0; 40],
            rain_red: [false; 40],
            rain_interesting: [false; 40],
            rain_sway_x: [0.0; 40],
            rain_sway_v: [0.0; 40],
            rain_flash: [0.0; 40],
            shards: Vec::new(),
            shattered: Vec::new(),
            hearts: Vec::new(),
        };
        for gx in 0..40 {
            s.rain_speed[gx] = 32.0 + hash2_u32(gx as i32, 77) * 26.0;
            s.rain_red[gx] = hash2_u32(gx as i32 + 5, 37) < 0.14;
            s.rain_interesting[gx] = gx % 7 == 2;
        }
        s
    }

    fn next_rand(&mut self) -> f64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x as f64) / (u32::MAX as f64)
    }
}

struct AppState {
    pkg_root: PathBuf,
    meta: PackageMeta,
    sys: SystemSummary,
    opts: InstallOptions,
    slides: Vec<SlideItem>,
    slide_imgs: Vec<Option<LoadedImage>>,
    slide_idx: usize,
    slide_visual_pos: f32,
    dots_track_rc: RECT,
    dots_first_cx: f32,
    dots_step_x: f32,
    dots_dragging: bool,
    dots_hover_idx: Option<usize>,
    screen: Screen,
    dpi: i32,
    last_work_w: i32,
    last_work_h: i32,
    hover: HitId,
    hits: Vec<HitRect>,
    progress: u32,
    status_text: String,
    install_report: Option<InstallReport>,
    error_text: Option<String>,
    uninstall_reboot: bool,
    worker: Arc<Mutex<WorkerProgress>>,
    should_reboot_on_exit: bool,
    started_in_uninstall: bool,
    crt_tab: u8, // 0 = Система, 1 = Live MM:SS log
    crt_lines: Vec<String>,
    crt_stream: String,
    crt_char_phases: Vec<u8>,
    crt_revealed_chars: usize,
    crt_blink_on: bool,
    install_start_ms: Option<u64>,
    install_end_ms: Option<u64>,
    copied_until_ms: u64,
    anim_start_ms: u64,
    morph_start_ms: u64,
    morph_from_idx: u8,
    morph_to_idx: u8,
    stage_fx_variant: u8,
    cat: CatState,
}

impl AppState {
    fn trigger_stage_morph(&mut self, from_idx: u8, to_idx: u8, now_ms: u64) {
        self.morph_from_idx = from_idx;
        self.morph_to_idx = to_idx;
        self.morph_start_ms = now_ms;
        // Randomly alternate between #01 (TNT Red Ignition) and #09 (Shrapnel Scatter + Cheek Puff)
        self.stage_fx_variant = if self.cat.next_rand() < 0.5 { 1 } else { 9 };
    }

    fn current_crt_phase(&self) -> u8 {
        if matches!(self.screen, Screen::Done | Screen::UninstallDone) || self.progress >= 100 {
            5
        } else if self.progress >= 94 {
            4
        } else if self.progress >= 75 {
            3
        } else if self.progress >= 64 {
            2
        } else if self.progress >= 32 {
            1
        } else {
            0
        }
    }

    fn push_crt_tokens(&mut self, tokens: &[String], phase: u8) {
        for tok in tokens {
            if !self.crt_stream.is_empty() {
                self.crt_stream.push_str(" / ");
                self.crt_char_phases.extend_from_slice(&[phase, phase, phase]);
            }
            let ch_len = tok.chars().count();
            self.crt_stream.push_str(tok);
            self.crt_char_phases.resize(self.crt_char_phases.len() + ch_len, phase);
        }
        let total = self.crt_stream.chars().count();
        if total > 8000 {
            let drop_chars = total - 8000;
            if let Some((byte_idx, _)) = self.crt_stream.char_indices().nth(drop_chars) {
                self.crt_stream.drain(..byte_idx);
                let dp = drop_chars.min(self.crt_char_phases.len());
                self.crt_char_phases.drain(..dp);
                self.crt_revealed_chars = self.crt_revealed_chars.saturating_sub(drop_chars);
            }
        }
    }

    fn advance_crt_typewriter(&mut self) {
        let total = self.crt_stream.chars().count();
        if self.crt_revealed_chars < total {
            let backlog = total - self.crt_revealed_chars;
            // Fast char-by-char teletype (~90-300 ch/s adaptive) so characters always fly visibly without popping full words
            let step = (backlog / 35).clamp(3, 10);
            self.crt_revealed_chars = (self.crt_revealed_chars + step).min(total);
        }
    }

    fn s(&self, px: i32) -> i32 {
        (px * self.dpi + 48) / 96
    }

    fn estimated_install_mb(&self) -> u64 {
        let mut total = self.meta.estimated_mb;
        if !self.opts.install_physx {
            total = total.saturating_sub(140);
        }
        if !self.opts.install_nvcpl {
            total = total.saturating_sub(25);
        }
        if !self.opts.install_nvpi {
            total = total.saturating_sub(2);
        }
        total
    }

    fn dot_index_for_x(&self, x: i32) -> usize {
        let n = self.slides.len();
        if n <= 1 || self.dots_step_x <= 0.5 {
            return 0;
        }
        let rel = ((x as f32) - self.dots_first_cx) / self.dots_step_x;
        (rel.round().clamp(0.0, (n - 1) as f32)) as usize
    }

    fn dot_continuous_pos_for_x(&self, x: i32) -> f32 {
        let n = self.slides.len();
        if n <= 1 || self.dots_step_x <= 0.5 {
            return 0.0;
        }
        (((x as f32) - self.dots_first_cx) / self.dots_step_x).clamp(0.0, (n - 1) as f32)
    }

    fn set_slide_scrub(&mut self, hwnd: HWND, x: i32, snap_idx: bool) {
        let n = self.slides.len();
        if n == 0 {
            return;
        }
        let pos = self.dot_continuous_pos_for_x(x);
        let idx = self.dot_index_for_x(x).min(n - 1);
        if !snap_idx {
            self.slide_visual_pos = pos;
        }
        if idx != self.slide_idx {
            self.slide_idx = idx;
            unsafe {
                SetTimer(hwnd, TIMER_SLIDE, 9000, std::ptr::null());
            }
        }
    }

    fn tick_slide_pill(&mut self) {
        if self.dots_dragging || self.slides.is_empty() {
            return;
        }
        let target = self.slide_idx.min(self.slides.len() - 1) as f32;
        let diff = target - self.slide_visual_pos;
        if diff.abs() < 0.015 {
            self.slide_visual_pos = target;
        } else {
            self.slide_visual_pos += diff * 0.28;
        }
    }
}

fn load_image_from_disk_no_lock(path: &Path) -> Option<LoadedImage> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > (u32::MAX as usize) {
        return None;
    }
    unsafe {
        let stream = SHCreateMemStream(bytes.as_ptr(), bytes.len() as u32);
        if stream.is_null() {
            return None;
        }
        let mut img: *mut c_void = std::ptr::null_mut();
        let rc = GdipLoadImageFromStream(stream, &mut img);
        // Release IStream (IUnknown::Release is vtable slot 2)
        type FnRelease = unsafe extern "system" fn(*mut c_void) -> u32;
        let vtbl = *(stream as *const *const usize);
        let release: FnRelease = std::mem::transmute(*vtbl.add(2));
        release(stream);

        if rc != 0 || img.is_null() {
            return None;
        }
        let mut w = 0u32;
        let mut h = 0u32;
        GdipGetImageWidth(img, &mut w);
        GdipGetImageHeight(img, &mut h);
        Some(LoadedImage { gp_img: img, w, h })
    }
}

pub fn run_gui(
    pkg_root: PathBuf,
    meta: PackageMeta,
    sys: SystemSummary,
    slides: Vec<SlideItem>,
    start_uninstall: bool,
) -> i32 {
    unsafe {
        let mut gp_token: usize = 0;
        let gp_in = GdiplusStartupInput {
            gdiplus_version: 1,
            debug_event_callback: 0,
            suppress_background_thread: 0,
            suppress_external_codecs: 0,
        };
        let _ = GdiplusStartup(&mut gp_token, &gp_in, std::ptr::null_mut());

        let mut slide_imgs = Vec::with_capacity(slides.len());
        for s in &slides {
            let p = pkg_root.join("Showcase").join(&s.slide_file);
            slide_imgs.push(load_image_from_disk_no_lock(&p));
        }

        let worker = Arc::new(Mutex::new(WorkerProgress {
            percent: 0,
            status: String::new(),
            crt_lines: Vec::new(),
            install_res: None,
            uninstall_res: None,
        }));

        let initial_screen = if start_uninstall {
            Screen::UninstallConfirm
        } else {
            Screen::Components
        };

        let init_word_idx = if start_uninstall { 3 } else { 0 };
        let init_ms = win32::tick_count();
        let state = Box::new(AppState {
            pkg_root: pkg_root.clone(),
            meta,
            sys,
            opts: InstallOptions::default(),
            slides,
            slide_imgs,
            slide_idx: 0,
            slide_visual_pos: 0.0,
            dots_track_rc: RECT::default(),
            dots_first_cx: 0.0,
            dots_step_x: 0.0,
            dots_dragging: false,
            dots_hover_idx: None,
            screen: initial_screen,
            dpi: 96,
            last_work_w: 0,
            last_work_h: 0,
            hover: HitId::None,
            hits: Vec::new(),
            progress: 0,
            status_text: String::new(),
            install_report: None,
            error_text: None,
            uninstall_reboot: false,
            worker,
            should_reboot_on_exit: false,
            started_in_uninstall: start_uninstall,
            crt_tab: 0,
            crt_lines: Vec::new(),
            crt_stream: String::new(),
            crt_char_phases: Vec::new(),
            crt_revealed_chars: 0,
            crt_blink_on: true,
            install_start_ms: None,
            install_end_ms: None,
            copied_until_ms: 0,
            anim_start_ms: init_ms,
            morph_start_ms: 0,
            morph_from_idx: init_word_idx,
            morph_to_idx: init_word_idx,
            stage_fx_variant: 1,
            cat: CatState::new(init_ms),
        });

        let hinst = GetModuleHandleW(std::ptr::null());
        let cls_name = to_wide("OpenFrankenDriverBetaSetupWnd");
        let hicon = LoadIconW(hinst, 101usize as *const u16);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0x0003, // CS_HREDRAW | CS_VREDRAW
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

        let style = WS_BORDERLESS_SQUARE;
        let work_rc = get_monitor_work_area(std::ptr::null_mut());
        let ww = 900.min(work_rc.w().max(320));
        let wh = 580.min(work_rc.h().max(240));
        let wx = work_rc.left + ((work_rc.w() - ww) / 2).max(0);
        let wy = work_rc.top + ((work_rc.h() - wh) / 2).max(0);

        let title = to_wide(&format!(
            "openfrankendriver {}",
            state.meta.driver_version
        ));
        let ptr = Box::into_raw(state);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            cls_name.as_ptr(),
            title.as_ptr(),
            style,
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
            GdiplusShutdown(gp_token);
            MessageBoxW(std::ptr::null_mut(), to_wide("Не удалось создать окно установки.").as_ptr(), to_wide("openfrankendriver").as_ptr(), MB_OK | MB_ICONERROR);
            return 1;
        }

        // Force square corners even on Windows 11 (DWMWA_WINDOW_CORNER_PREFERENCE = 33, DWMWCP_DONOTROUND = 1)
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, 20, &dark as *const i32 as *const c_void, 4);
        let corner_square: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, 33, &corner_square as *const i32 as *const c_void, 4);

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        bring_window_to_front_once(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let state_back = Box::from_raw(ptr);
        let reboot = state_back.should_reboot_on_exit;
        let was_uninstall = state_back.started_in_uninstall || matches!(state_back.screen, Screen::UninstallDone);
        drop(state_back);
        GdiplusShutdown(gp_token);

        if was_uninstall {
            uninstaller::schedule_self_cleanup_if_installed();
        }
        installer::schedule_temp_unpack_cleanup_if_sfx(&pkg_root);
        if reboot {
            win32::reboot_system_now();
        }
        msg.wParam as i32
    }
}

unsafe fn bring_window_to_front_once(hwnd: HWND) {
    const HWND_TOPMOST: HWND = -1isize as HWND;
    const HWND_NOTOPMOST: HWND = -2isize as HWND;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;

    let _ = BringWindowToTop(hwnd);
    let _ = SetForegroundWindow(hwnd);
    let fg = GetForegroundWindow();
    if fg != hwnd && !fg.is_null() {
        let fg_tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let cur_tid = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != cur_tid && AttachThreadInput(cur_tid, fg_tid, 1) != 0;
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(hwnd);
        if attached {
            let _ = AttachThreadInput(cur_tid, fg_tid, 0);
        }
    }
    // Non-sticky one-shot Z-order bump: raise above all windows once, then drop TOPMOST immediately.
    let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
    let _ = SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

unsafe fn get_state<'a>(hwnd: HWND) -> Option<&'a mut AppState> {
    let p = GetWindowLongPtrW(hwnd, -21 /* GWLP_USERDATA */) as *mut AppState;
    if p.is_null() { None } else { Some(&mut *p) }
}

unsafe fn get_monitor_work_area(hwnd: HWND) -> RECT {
    if !hwnd.is_null() {
        let hmon = MonitorFromWindow(hwnd, 2 /* MONITOR_DEFAULTTONEAREST */);
        if !hmon.is_null() {
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(hmon, &mut mi) != 0 && mi.rcWork.w() > 0 && mi.rcWork.h() > 0 {
                return mi.rcWork;
            }
        }
    }
    let mut work = RECT::default();
    if SystemParametersInfoW(0x0030 /* SPI_GETWORKAREA */, 0, &mut work as *mut RECT as *mut c_void, 0) != 0
        && work.w() > 0
        && work.h() > 0
    {
        return work;
    }
    RECT {
        left: 0,
        top: 0,
        right: GetSystemMetrics(SM_CXSCREEN).max(800),
        bottom: GetSystemMetrics(SM_CYSCREEN).max(600),
    }
}

fn clamp_dpi_to_work(raw_dpi: i32, work_rc: RECT) -> i32 {
    let raw = if raw_dpi > 0 { raw_dpi } else { 96 };
    let max_w = (work_rc.w().max(320) * 94 * 96) / (100 * 900);
    let max_h = (work_rc.h().max(240) * 94 * 96) / (100 * 580);
    let cap = max_w.min(max_h).max(68);
    raw.min(cap).max(68)
}

unsafe fn recenter_and_fit_window(hwnd: HWND, st: &mut AppState, raw_dpi_hint: i32, force_center: bool) {
    let work_rc = get_monitor_work_area(hwnd);
    let raw_dpi = if raw_dpi_hint > 0 {
        raw_dpi_hint
    } else {
        GetDpiForWindow(hwnd) as i32
    };
    let next_dpi = clamp_dpi_to_work(raw_dpi, work_rc);
    let work_changed = work_rc.w() != st.last_work_w || work_rc.h() != st.last_work_h;
    if force_center || work_changed || next_dpi != st.dpi {
        st.dpi = next_dpi;
        st.last_work_w = work_rc.w();
        st.last_work_h = work_rc.h();
        let ww = st.s(900);
        let wh = st.s(580);
        let wx = work_rc.left + ((work_rc.w() - ww) / 2).max(0);
        let wy = work_rc.top + ((work_rc.h() - wh) / 2).max(0);
        SetWindowPos(hwnd, std::ptr::null_mut(), wx, wy, ww, wh, 0x0014 /* SWP_NOZORDER | SWP_NOACTIVATE */);
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    if msg == WM_CREATE {
        #[repr(C)]
        struct CREATESTRUCT {
            lpCreateParams: *mut c_void,
        }
        let cs = &*(lparam as *const CREATESTRUCT);
        SetWindowLongPtrW(hwnd, -21, cs.lpCreateParams as isize);
        if let Some(st) = get_state(hwnd) {
            recenter_and_fit_window(hwnd, st, 0, true);
            SetTimer(hwnd, TIMER_CRT, 500, std::ptr::null());
            SetTimer(hwnd, TIMER_ANIM, 33, std::ptr::null());
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
                if pt.y >= 0 && pt.y < st.s(36) && hit_test(&st.hits, pt.x, pt.y) == HitId::None {
                    return HTCAPTION;
                }
            }
            def
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as i32;
            let work_rc = get_monitor_work_area(hwnd);
            let clamped = clamp_dpi_to_work(new_dpi, work_rc);
            if clamped != new_dpi {
                recenter_and_fit_window(hwnd, st, new_dpi, true);
            } else if new_dpi > 0 {
                st.dpi = new_dpi;
                st.last_work_w = work_rc.w();
                st.last_work_h = work_rc.h();
                let suggested = &*(lparam as *const RECT);
                SetWindowPos(hwnd, std::ptr::null_mut(), suggested.left, suggested.top, suggested.w(), suggested.h(), 0x0014);
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
        WM_SIZE => {
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_SETCURSOR => {
            if (lparam & 0xFFFF) == 1 && (st.hover != HitId::None || st.dots_dragging) {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND as *const u16));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let x = (lparam as i16) as i32;
            let y = ((lparam >> 16) as i16) as i32;
            if st.dots_dragging {
                st.set_slide_scrub(hwnd, x, false);
                st.dots_hover_idx = Some(st.dot_index_for_x(x));
                InvalidateRect(hwnd, std::ptr::null(), 0);
                return 0;
            }
            let hit = hit_test(&st.hits, x, y);
            let hover_dot = if hit == HitId::DotTrack && !st.slides.is_empty() {
                Some(st.dot_index_for_x(x))
            } else {
                None
            };
            if hit != st.hover || hover_dot != st.dots_hover_idx {
                st.hover = hit;
                st.dots_hover_idx = hover_dot;
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam as i16) as i32;
            let y = ((lparam >> 16) as i16) as i32;
            let hit = hit_test(&st.hits, x, y);
            if hit == HitId::DotTrack && st.screen == Screen::Installing && !st.slides.is_empty() {
                st.dots_dragging = true;
                SetCapture(hwnd);
                st.set_slide_scrub(hwnd, x, true);
                st.dots_hover_idx = Some(st.dot_index_for_x(x));
                InvalidateRect(hwnd, std::ptr::null(), 0);
                return 0;
            }
            if hit == HitId::None {
                let cx = st.cat.x;
                let cy = st.cat.y + st.cat.jump_y - (st.s(18) as f64);
                if ((x as f64) - cx).hypot((y as f64) - cy) <= (st.s(46) as f64) {
                    poke_cat(hwnd, st);
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                    return 0;
                }
            }
            handle_click(hwnd, st, hit);
            0
        }
        WM_LBUTTONUP => {
            if st.dots_dragging {
                st.dots_dragging = false;
                ReleaseCapture();
                SetTimer(hwnd, TIMER_SLIDE, 9000, std::ptr::null());
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_CAPTURECHANGED => {
            if st.dots_dragging {
                st.dots_dragging = false;
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_KEYDOWN => {
            if st.screen == Screen::Installing && !st.slides.is_empty() {
                if wparam == VK_LEFT {
                    st.slide_idx = if st.slide_idx == 0 { st.slides.len() - 1 } else { st.slide_idx - 1 };
                    SetTimer(hwnd, TIMER_SLIDE, 9000, std::ptr::null());
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                } else if wparam == VK_RIGHT {
                    st.slide_idx = (st.slide_idx + 1) % st.slides.len();
                    SetTimer(hwnd, TIMER_SLIDE, 9000, std::ptr::null());
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            } else if wparam == VK_ESCAPE && matches!(st.screen, Screen::Components | Screen::Done | Screen::UninstallConfirm | Screen::UninstallDone) {
                DestroyWindow(hwnd);
            } else if wparam == VK_RETURN && st.screen == Screen::Done {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_SLIDE && st.screen == Screen::Installing && !st.slides.is_empty() {
                st.slide_idx = (st.slide_idx + 1) % st.slides.len();
                InvalidateRect(hwnd, std::ptr::null(), 0);
            } else if wparam == TIMER_POLL {
                poll_worker(hwnd, st);
            } else if wparam == TIMER_CRT {
                st.crt_blink_on = !st.crt_blink_on;
                recenter_and_fit_window(hwnd, st, 0, false);
                InvalidateRect(hwnd, std::ptr::null(), 0);
            } else if wparam == TIMER_ANIM {
                st.advance_crt_typewriter();
                st.tick_slide_pill();
                tick_cat_and_matrix(hwnd, st);
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_APP_WORKER => {
            poll_worker(hwnd, st);
            0
        }
        WM_CLOSE => {
            if matches!(st.screen, Screen::Installing | Screen::Uninstalling) {
                return 0; // Block closing mid-driver swap so Windows isn't left half-installed
            }
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_SLIDE);
            KillTimer(hwnd, TIMER_POLL);
            KillTimer(hwnd, TIMER_CRT);
            KillTimer(hwnd, TIMER_ANIM);
            PostQuitMessage(0);
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_window(hwnd, hdc, st);
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn hit_test(hits: &[HitRect], x: i32, y: i32) -> HitId {
    for h in hits.iter().rev() {
        if h.rect.contains(x, y) {
            return h.id;
        }
    }
    HitId::None
}

unsafe fn handle_click(hwnd: HWND, st: &mut AppState, hit: HitId) {
    match hit {
        HitId::BtnCopyLog => {
            let live_txt = win32::get_live_log_text();
            if win32::set_clipboard_unicode(hwnd, &live_txt) {
                st.copied_until_ms = win32::tick_count().saturating_add(2000);
            }
        }
        HitId::BtnGitHub => {
            let op = to_wide("open");
            let url = to_wide("https://github.com/lnp/OpenFrankenDriver");
            ShellExecuteW(hwnd, op.as_ptr(), url.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOW);
            return;
        }
        HitId::BtnWinMin => {
            ShowWindow(hwnd, SW_MINIMIZE);
            return;
        }
        HitId::BtnWinClose => {
            if !matches!(st.screen, Screen::Installing | Screen::Uninstalling) {
                DestroyWindow(hwnd);
                return;
            }
        }
        HitId::TabCrtSys => {
            st.crt_tab = 0;
        }
        HitId::TabCrtLog => {
            st.crt_tab = 1;
        }
        HitId::ChkNvcpl => st.opts.install_nvcpl = !st.opts.install_nvcpl,
        HitId::ChkHdaudio => st.opts.install_hdaudio = !st.opts.install_hdaudio,
        HitId::ChkPhysx => st.opts.install_physx = !st.opts.install_physx,
        HitId::ChkVcpp => st.opts.install_vcpp = !st.opts.install_vcpp,
        HitId::ChkNvpi => st.opts.install_nvpi = !st.opts.install_nvpi,
        HitId::ChkClean => st.opts.clean_install = !st.opts.clean_install,
        HitId::BtnCancel => {
            DestroyWindow(hwnd);
            return;
        }
        HitId::BtnUninstallBack => {
            if st.started_in_uninstall {
                DestroyWindow(hwnd);
                return;
            }
            st.screen = Screen::Components;
        }
        HitId::BtnInstall => {
            start_install_worker(hwnd, st);
        }
        HitId::BtnUninstallConfirm => {
            start_uninstall_worker(hwnd, st);
        }
        HitId::DotTrack => {}
        HitId::BtnDonePrimary => {
            let need_reboot = st
                .install_report
                .as_ref()
                .map(|r| r.reboot_required)
                .unwrap_or(st.uninstall_reboot);
            if need_reboot && st.error_text.is_none() {
                st.should_reboot_on_exit = true;
            }
            DestroyWindow(hwnd);
            return;
        }
        HitId::BtnDoneClose => {
            DestroyWindow(hwnd);
            return;
        }
        HitId::None => return,
    }
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

unsafe fn start_install_worker(hwnd: HWND, st: &mut AppState) {
    let start_ms = win32::tick_count();
    st.trigger_stage_morph(0, 1, start_ms);
    st.screen = Screen::Installing;
    st.progress = 2;
    st.status_text = "Подготовка установки...".to_string();
    st.error_text = None;
    st.install_report = None;
    st.slide_idx = 0;
    st.slide_visual_pos = 0.0;
    st.dots_dragging = false;
    st.crt_tab = 1;
    st.crt_lines.clear();
    st.crt_stream.clear();
    st.crt_char_phases.clear();
    st.crt_revealed_chars = 0;
    st.install_start_ms = Some(start_ms);
    st.install_end_ms = None;

    {
        let mut w = st.worker.lock().unwrap();
        w.percent = 2;
        w.status = st.status_text.clone();
        w.crt_lines.clear();
        w.install_res = None;
        w.uninstall_res = None;
    }

    let hwnd_val = hwnd as usize;
    let pkg_root = st.pkg_root.clone();
    let meta = st.meta.clone();
    let sys = st.sys.clone();
    let opts = st.opts.clone();
    let worker = Arc::clone(&st.worker);

    std::thread::spawn(move || {
        let w_cb = Arc::clone(&worker);
        let w_crt = Arc::clone(&worker);
        let res = installer::run_install(
            &pkg_root,
            &meta,
            &sys,
            &opts,
            move |pct, text| {
                {
                    let mut g = w_cb.lock().unwrap();
                    g.percent = pct;
                    g.status = text.to_string();
                }
                PostMessageW(hwnd_val as HWND, WM_APP_WORKER, 0, 0);
            },
            move |crt_msg| {
                let mut g = w_crt.lock().unwrap();
                g.crt_lines.push(crt_msg.to_string());
                PostMessageW(hwnd_val as HWND, WM_APP_WORKER, 0, 0);
            },
        );
        {
            let mut g = worker.lock().unwrap();
            g.install_res = Some(res);
        }
        PostMessageW(hwnd_val as HWND, WM_APP_WORKER, 0, 0);
    });

    SetTimer(hwnd, TIMER_SLIDE, 9000, std::ptr::null());
    SetTimer(hwnd, TIMER_POLL, 150, std::ptr::null());
}

unsafe fn start_uninstall_worker(hwnd: HWND, st: &mut AppState) {
    let start_ms = win32::tick_count();
    st.trigger_stage_morph(3, 4, start_ms);
    st.screen = Screen::Uninstalling;
    st.progress = 5;
    st.status_text = "Подготовка удаления...".to_string();
    st.error_text = None;
    st.crt_tab = 1;
    st.crt_lines.clear();
    st.crt_stream.clear();
    st.crt_char_phases.clear();
    st.crt_revealed_chars = 0;
    st.install_start_ms = Some(start_ms);
    st.install_end_ms = None;

    {
        let mut w = st.worker.lock().unwrap();
        w.percent = 5;
        w.status = st.status_text.clone();
        w.crt_lines.clear();
        w.install_res = None;
        w.uninstall_res = None;
    }

    let hwnd_val = hwnd as usize;
    let worker = Arc::clone(&st.worker);

    std::thread::spawn(move || {
        let w_cb = Arc::clone(&worker);
        let res = uninstaller::run_uninstall(move |pct, text| {
            let elapsed_s = win32::tick_count().saturating_sub(start_ms) / 1000;
            {
                let mut g = w_cb.lock().unwrap();
                g.percent = pct;
                g.status = text.to_string();
                g.crt_lines.push(format!("{:02}:{:02} {}", elapsed_s / 60, elapsed_s % 60, text));
            }
            PostMessageW(hwnd_val as HWND, WM_APP_WORKER, 0, 0);
        });
        {
            let mut g = worker.lock().unwrap();
            g.uninstall_res = Some(res);
        }
        PostMessageW(hwnd_val as HWND, WM_APP_WORKER, 0, 0);
    });

    SetTimer(hwnd, TIMER_POLL, 150, std::ptr::null());
}

unsafe fn poll_worker(hwnd: HWND, st: &mut AppState) {
    let (pct, status, new_crt, inst_done, uninst_done) = {
        let mut g = st.worker.lock().unwrap();
        let drained = std::mem::take(&mut g.crt_lines);
        (g.percent, g.status.clone(), drained, g.install_res.take(), g.uninstall_res.take())
    };
    let mut changed = pct != st.progress || status != st.status_text || !new_crt.is_empty();
    st.progress = pct;
    st.status_text = status;
    if !new_crt.is_empty() {
        let phase = st.current_crt_phase();
        st.push_crt_tokens(&new_crt, phase);
        st.crt_lines.extend(new_crt);
        if st.crt_lines.len() > 160 {
            let drain_n = st.crt_lines.len() - 160;
            st.crt_lines.drain(0..drain_n);
        }
    }

    if let Some(res) = inst_done {
        KillTimer(hwnd, TIMER_SLIDE);
        KillTimer(hwnd, TIMER_POLL);
        let done_ms = win32::tick_count();
        st.install_end_ms = Some(done_ms);
        match res {
            Ok(rep) => st.install_report = Some(rep),
            Err(e) => st.error_text = Some(e),
        }
        st.trigger_stage_morph(1, 2, done_ms);
        st.screen = Screen::Done;
        changed = true;
    }
    if let Some(res) = uninst_done {
        KillTimer(hwnd, TIMER_POLL);
        let done_ms = win32::tick_count();
        st.install_end_ms = Some(done_ms);
        match res {
            Ok(reboot) => st.uninstall_reboot = reboot,
            Err(e) => st.error_text = Some(e),
        }
        st.trigger_stage_morph(4, 2, done_ms);
        st.screen = Screen::UninstallDone;
        changed = true;
    }
    if changed {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

unsafe fn make_font(st: &AppState, px: i32, weight: i32, italic: bool) -> HFONT {
    let face = to_wide("Segoe UI");
    CreateFontW(
        -st.s(px),
        0,
        0,
        0,
        weight,
        if italic { 1 } else { 0 },
        0,
        0,
        DEFAULT_CHARSET,
        0,
        0,
        CLEARTYPE_QUALITY,
        0,
        face.as_ptr(),
    )
}

unsafe fn make_mono_font(st: &AppState, px: i32, weight: i32) -> HFONT {
    let face = to_wide("Consolas");
    CreateFontW(
        -st.s(px),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        0,
        0,
        CLEARTYPE_QUALITY,
        0,
        face.as_ptr(),
    )
}

unsafe fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
    let w = to_wide(text);
    let len = w.len().saturating_sub(1) as i32;
    let old = SelectObject(hdc, font as HGDIOBJ);
    let mut sz = POINT { x: 0, y: 0 };
    GetTextExtentPoint32W(hdc, w.as_ptr(), len, &mut sz);
    SelectObject(hdc, old);
    sz.x
}

unsafe fn btn_width(hdc: HDC, font: HFONT, label: &str, min_px: i32, st: &AppState) -> i32 {
    (text_width(hdc, font, label) + st.s(28)).max(st.s(min_px))
}

unsafe fn fill_rect(hdc: HDC, r: RECT, color: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    SetBkColor(hdc, color);
    ExtTextOutW(hdc, 0, 0, ETO_OPAQUE, &r, std::ptr::null(), 0, std::ptr::null());
}

unsafe fn draw_square_border(hdc: HDC, r: RECT, border: u32) {
    fill_rect(hdc, RECT { left: r.left, top: r.top, right: r.right, bottom: r.top + 1 }, border);
    fill_rect(hdc, RECT { left: r.left, top: r.bottom - 1, right: r.right, bottom: r.bottom }, border);
    fill_rect(hdc, RECT { left: r.left, top: r.top, right: r.left + 1, bottom: r.bottom }, border);
    fill_rect(hdc, RECT { left: r.right - 1, top: r.top, right: r.right, bottom: r.bottom }, border);
}

unsafe fn draw_square_box(hdc: HDC, r: RECT, fill: u32, border: u32) {
    fill_rect(hdc, r, fill);
    draw_square_border(hdc, r, border);
}

unsafe fn draw_round_box(hdc: HDC, r: RECT, fill: u32, border: u32, _radius: i32) {
    draw_square_box(hdc, r, fill, border);
}

unsafe fn draw_text(hdc: HDC, text: &str, mut r: RECT, font: HFONT, color: u32, flags: u32) {
    let old_f = SelectObject(hdc, font as HGDIOBJ);
    SetTextColor(hdc, color);
    let w = to_wide(text);
    DrawTextW(hdc, w.as_ptr(), -1, &mut r, flags | DT_NOPREFIX);
    SelectObject(hdc, old_f);
}

fn hash2_u32(x: i32, y: i32) -> f64 {
    let mut n = (x.wrapping_mul(374_761_393)) ^ (y.wrapping_mul(668_265_263));
    n = (n ^ ((n as u32 >> 13) as i32)).wrapping_mul(1_274_126_177);
    let u = (n as u32) ^ ((n as u32) >> 16);
    (u as f64) / 4_294_967_295.0
}

fn clamp_u8(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

// Horizontally mirrored 5x7 pixel katakana + numerals (Simon Whiteley Matrix 1999 style, zero font dependency).
const KATA_5X7: [[u8; 7]; 16] = [
    [0b11111, 0b10000, 0b10010, 0b00010, 0b00010, 0b00100, 0b01000], // mirrored ｱ
    [0b00010, 0b00110, 0b01010, 0b10010, 0b00010, 0b00010, 0b00010], // mirrored ｲ
    [0b00100, 0b11111, 0b10001, 0b10000, 0b10000, 0b01000, 0b00110], // mirrored ｳ
    [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111], // mirrored ｴ
    [0b00100, 0b11111, 0b10100, 0b10100, 0b10100, 0b10010, 0b01001], // mirrored ｶ
    [0b00100, 0b11111, 0b00100, 0b11111, 0b00100, 0b00110, 0b01100], // mirrored ｷ
    [0b11111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111], // mirrored ｺ
    [0b01010, 0b11111, 0b01010, 0b01010, 0b01000, 0b00100, 0b00010], // mirrored ｻ
    [0b00011, 0b10000, 0b00011, 0b01000, 0b00100, 0b00010, 0b00001], // mirrored ｼ
    [0b00010, 0b01110, 0b10010, 0b01110, 0b00100, 0b01000, 0b10000], // mirrored ﾀ
    [0b11111, 0b00000, 0b11111, 0b00100, 0b00100, 0b01000, 0b10000], // mirrored ﾃ
    [0b00100, 0b11111, 0b00100, 0b10101, 0b10101, 0b00100, 0b00100], // mirrored ﾎ
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110], // 0
    [0b01110, 0b10001, 0b10000, 0b01110, 0b10000, 0b10001, 0b01110], // mirrored 3
    [0b01000, 0b01100, 0b01010, 0b11111, 0b01000, 0b01000, 0b01000], // mirrored 4
    [0b11111, 0b10000, 0b01000, 0b00100, 0b00100, 0b00100, 0b00100], // mirrored 7
];

fn bold_glyph_rows(ch: char) -> Option<&'static [&'static str; 7]> {
    match ch {
        'Э' => Some(&["01110", "11011", "00011", "01111", "00011", "11011", "01110"]),
        'Т' => Some(&["1111", "1111", "0110", "0110", "0110", "0110", "0110"]),
        'А' => Some(&["01110", "11011", "11011", "11111", "11111", "11011", "11011"]),
        'П' => Some(&["11111", "11011", "11011", "11011", "11011", "11011", "11011"]),
        'К' => Some(&["11011", "11011", "11110", "11100", "11110", "11011", "11011"]),
        'О' => Some(&["01110", "11011", "11011", "11011", "11011", "11011", "01110"]),
        'М' => Some(&["1100011", "1110111", "1111111", "1101011", "1100011", "1100011", "1100011"]),
        'Н' => Some(&["11011", "11011", "11111", "11111", "11011", "11011", "11011"]),
        'Е' => Some(&["1111", "1100", "1110", "1110", "1100", "1100", "1111"]),
        'Ы' => Some(&["110011", "110011", "111011", "110111", "110111", "110111", "111011"]),
        'У' => Some(&["11011", "11011", "11011", "01111", "00011", "11011", "01110"]),
        'С' => Some(&["01111", "11011", "11000", "11000", "11000", "11011", "01111"]),
        'В' => Some(&["11110", "11011", "11110", "11110", "11011", "11011", "11110"]),
        'Г' => Some(&["11111", "11111", "11000", "11000", "11000", "11000", "11000"]),
        'Д' => Some(&["011110", "010010", "010010", "010010", "010010", "111111", "110011"]),
        'Л' => Some(&["00111", "01111", "11011", "11011", "11011", "11011", "11011"]),
        'И' => Some(&["110011", "110011", "110111", "111111", "111011", "110011", "110011"]),
        _ => None,
    }
}

struct BoldWordGrid {
    w: usize,
    rows: [Vec<bool>; 7],
}

fn rasterize_bold_word(word: &str) -> BoldWordGrid {
    let mut rows: [Vec<bool>; 7] = std::array::from_fn(|_| Vec::new());
    for (idx, ch) in word.chars().enumerate() {
        let Some(g) = bold_glyph_rows(ch) else {
            continue;
        };
        if idx > 0 {
            for r in 0..7 {
                rows[r].push(false);
            }
        }
        for r in 0..7 {
            for b in g[r].bytes() {
                rows[r].push(b == b'1');
            }
        }
    }
    let w = rows[0].len();
    BoldWordGrid { w, rows }
}

const MAX_STAGE_WORD_COLS: usize = 60;

fn stage_word_by_idx(idx: u8) -> &'static str {
    match idx {
        0 => "КОМПОНЕНТЫ",
        1 => "УСТАНОВКА",
        2 => "ГОТОВО",
        3 => "ПОДГОТОВКА",
        _ => "УДАЛЕНИЕ",
    }
}

fn screen_to_word_idx(screen: Screen) -> u8 {
    match screen {
        Screen::Components => 0,
        Screen::Installing => 1,
        Screen::Done => 2,
        Screen::UninstallConfirm => 3,
        Screen::Uninstalling => 4,
        Screen::UninstallDone => 2,
    }
}

fn screen_phase_mode(idx: u8) -> u8 {
    match idx {
        0 | 3 => 0, // Comp / Prepare
        1 | 4 => 1, // Installing / Uninstalling
        _ => 2,     // Done
    }
}

fn creeper_mosaic_rgb(c: i32, r: i32, t: f64) -> u32 {
    let base_tick = (t * 1.7).floor();
    let sub = t * 1.7 - base_tick;
    let seed = hash2_u32(c * 3 + r * 5, 19);
    let (br, bg, bb) = if seed > 0.68 {
        (158.0, 224.0, 38.0)
    } else if seed < 0.26 {
        (72.0, 128.0, 0.0)
    } else {
        (118.0, 185.0, 0.0)
    };
    let sp_hash = hash2_u32(c + (base_tick as i32) * 13, r + (base_tick as i32) * 7);
    let env = if sp_hash > 0.81 {
        (sub * std::f64::consts::PI).sin()
    } else {
        0.0
    };
    let scale = 0.94 + env * 0.36;
    let lift = env * 28.0;
    rgb(
        clamp_u8(br * scale + lift * 0.7),
        clamp_u8(bg * scale + lift * 0.9),
        clamp_u8(bb * scale + lift * 0.4),
    )
}

fn steel_mosaic_rgb(c: i32, r: i32, t: f64) -> u32 {
    let seed = hash2_u32(c * 5 + r * 3, 41);
    let pulse = 0.5 + 0.5 * (t * 1.5 + seed * 6.28318).sin();
    if seed > 0.68 {
        rgb(clamp_u8(94.0 + pulse * 14.0), clamp_u8(108.0 + pulse * 14.0), clamp_u8(122.0 + pulse * 14.0))
    } else if seed < 0.26 {
        rgb(clamp_u8(58.0 + pulse * 8.0), clamp_u8(70.0 + pulse * 8.0), clamp_u8(82.0 + pulse * 8.0))
    } else {
        rgb(clamp_u8(76.0 + pulse * 10.0), clamp_u8(88.0 + pulse * 10.0), clamp_u8(100.0 + pulse * 10.0))
    }
}

fn comp_mosaic_rgb(c: i32, r: i32) -> u32 {
    let s = hash2_u32(c * 3, r * 7);
    if s > 0.65 {
        rgb(220, 230, 240)
    } else if s < 0.25 {
        rgb(164, 180, 196)
    } else {
        rgb(196, 210, 222)
    }
}

fn drop_rgb(k_in: f64, is_head: bool, is_red: bool) -> u32 {
    let k = k_in.clamp(0.0, 1.0);
    if is_red {
        if is_head {
            rgb(clamp_u8(44.0 + k * 192.0), clamp_u8(24.0 + k * 40.0), clamp_u8(30.0 + k * 44.0))
        } else {
            rgb(clamp_u8(26.0 + k * 112.0), clamp_u8(22.0 + k * 18.0), clamp_u8(26.0 + k * 24.0))
        }
    } else if is_head {
        rgb(clamp_u8(42.0 + k * 110.0), clamp_u8(54.0 + k * 186.0), clamp_u8(38.0 + k * 88.0))
    } else {
        rgb(clamp_u8(22.0 + k * 42.0), clamp_u8(27.0 + k * 90.0), clamp_u8(30.0 + k * 34.0))
    }
}

/// Harder bottom-to-top Anvil mechanical impact curve: deep upper cushion slam (-5), heavy rebound (+kick), tight settle.
fn eval_anvil_impact(bp: f64, kick: f64) -> i32 {
    let p = bp.clamp(0.0, 1.0);
    let pi = std::f64::consts::PI;
    if p < 0.10 {
        (-5.0 * ((p / 0.10) * pi).sin()).round() as i32
    } else if p < 0.44 {
        ((((p - 0.10) / 0.34) * pi).sin() * kick).round() as i32
    } else if p < 0.76 {
        (-(((p - 0.44) / 0.32) * pi).sin() * (kick * 0.34)).round() as i32
    } else {
        ((((p - 0.76) / 0.24) * pi).sin() * 2.5).round() as i32
    }
}

/// Draw 8x8 pixel creeper face (#8 Bio-Spark + reactive Stage Impact #01 TNT Red Ignition / #09 Shrapnel Scatter & Cheek Puff).
unsafe fn draw_creeper(hdc: HDC, st: &AppState, x: i32, y: i32, size: i32, t: f64, shock: f64, fx_variant: u8) {
    let pi = std::f64::consts::PI;
    let s = shock.clamp(0.0, 1.0);
    let ox = x;
    let mut oy = y;
    let mut sh = size;

    if s > 0.01 {
        if fx_variant == 1 {
            // #01 TNT Ignition: high upward hop + brief landing squash
            oy -= (((s * 1.4).min(1.0) * pi).sin() * (st.s(12) as f64)).round() as i32;
            if s > 0.15 && s < 0.45 {
                let sq = st.s(6).max(3);
                sh = (size - sq).max(16);
                oy += st.s(4);
            }
        } else {
            // #09 Shrapnel Scatter: crisp upward jolt
            oy -= (s * (st.s(9) as f64)).round() as i32;
        }
    }

    let cw = (size as f64) / 8.0;
    let ch_f = (sh as f64) / 8.0;

    let slot = (t * 2.0).floor() as i32;
    let sub = t * 2.0 - (slot as f64);

    let rows: [&[u8; 8]; 8] = [
        b"31211213",
        b"12122121",
        b"1BB11BB1",
        b"1BB11BB1",
        b"311BB113",
        b"11BBBB11",
        b"12BBBB21",
        b"31B33B13",
    ];
    for (ry, row) in rows.iter().enumerate() {
        for (rx, &ch) in row.iter().enumerate() {
            let mut px_shift = 0i32;
            if fx_variant == 9 && s > 0.15 && (4..=6).contains(&ry) && (rx == 0 || rx == 7) {
                let puff = (s * (st.s(4) as f64)).round() as i32;
                px_shift = if rx == 0 { -puff } else { puff };
            }

            let col = if ch == b'B' {
                let is_eye = ry <= 3;
                if fx_variant == 1 && s > 0.08 {
                    if ((t * 24.0) as i32) % 2 == 0 || !is_eye {
                        rgb(255, clamp_u8(45.0 * (1.0 - s)), 50)
                    } else {
                        rgb(255, 255, 255)
                    }
                } else if fx_variant == 9 && s > 0.10 {
                    if is_eye { rgb(255, 184, 48) } else { rgb(28, 18, 8) }
                } else {
                    rgb(14, 17, 20)
                }
            } else {
                let (mut br, mut bg, mut bb) = match ch {
                    b'2' => (158.0, 224.0, 38.0),
                    b'3' => (72.0, 128.0, 0.0),
                    _ => (118.0, 185.0, 0.0),
                };
                let irx = rx as i32;
                let iry = ry as i32;
                let spark_on = hash2_u32(irx + slot * 13, iry + slot * 7) > 0.78;
                let env = if spark_on { (sub * pi).sin() } else { 0.0 };
                let is_red = spark_on && hash2_u32(irx * 11, iry + slot * 3) < 0.14;
                if is_red {
                    br = br + env * (210.0 - br);
                    bg = bg * (1.0 - env * 0.65);
                    bb = bb + env * 52.0;
                } else {
                    let mul = 0.94 + env * 0.42;
                    br = br * mul + env * 22.0;
                    bg = bg * mul + env * 28.0;
                    bb = bb * mul + env * 12.0;
                }
                if fx_variant == 1 && s > 0.45 {
                    let k = (s - 0.45) * 0.65;
                    br += k * 70.0;
                    bg += k * 55.0;
                }
                rgb(clamp_u8(br), clamp_u8(bg), clamp_u8(bb))
            };
            let x0 = ox + ((rx as f64) * cw).round() as i32 + px_shift;
            let y0 = oy + ((ry as f64) * ch_f).round() as i32;
            let x1 = ox + (((rx + 1) as f64) * cw).round() as i32 + px_shift;
            let y1 = oy + (((ry + 1) as f64) * ch_f).round() as i32;
            fill_rect(
                hdc,
                RECT { left: x0, top: y0, right: x1.max(x0 + 1), bottom: y1.max(y0 + 1) },
                col,
            );
        }
    }
}

/// Pure code-driven Matrix rain (mirrored 5x7 katakana head + square trail) synced with cat physics & pendulum sway!
unsafe fn draw_matrix_rain(
    hdc: HDC,
    st: &AppState,
    sb_w: i32,
    ch: i32,
    creeper_rc: RECT,
    stage_rc: RECT,
    log_top: i32,
    t: f64,
) {
    let hdr_h = st.s(36);
    let rain_cell = st.s(8).max(6);
    let r_cols = ((sb_w + rain_cell - 1) / rain_cell).min(40);
    let trail = 6i32;

    for gx in 0..r_cols {
        let gxu = gx as usize;
        let col_red = st.cat.rain_red[gxu];
        let head_y = st.cat.rain_y[gxu];
        let is_special = st.cat.rain_interesting[gxu];
        let sway_col = st.cat.rain_sway_x[gxu];
        let flash_col = st.cat.rain_flash[gxu];

        for ti in 0..trail {
            let y_px = head_y.round() as i32 - ti * rain_cell;
            let x_base = gx * rain_cell;
            if y_px < hdr_h || y_px > ch - rain_cell {
                continue;
            }
            if x_base + rain_cell > creeper_rc.left && x_base < creeper_rc.right
                && y_px + rain_cell > creeper_rc.top && y_px < creeper_rc.bottom
            {
                continue;
            }
            if x_base + rain_cell > stage_rc.left && x_base < stage_rc.right
                && y_px + rain_cell > stage_rc.top && y_px < stage_rc.bottom
            {
                continue;
            }
            // Keep Matrix rain cleanly outside the opaque framed CRT log box (only around/below it)
            if x_base + rain_cell > st.s(22) && x_base < sb_w - st.s(22)
                && y_px + rain_cell > log_top && y_px < ch - st.s(62)
            {
                continue;
            }

            let depth_w = ((y_px as f64 - (log_top as f64 * 0.45)) / (ch as f64).max(1.0)).clamp(0.0, 1.35);
            let x_px = (x_base as f64 + sway_col * depth_w * (1.0 - (ti as f64) / ((trail as f64) * 1.4))).round() as i32;
            if x_px < 0 || x_px + rain_cell > sb_w {
                continue;
            }

            let k = (1.0 - (ti as f64) / (trail as f64)).powf(1.45);
            if k < 0.05 {
                continue;
            }

            if ti == 0 {
                let glyph_tick = (t * 6.0).floor() as i32;
                let g_idx = (hash2_u32(gx, (y_px / rain_cell) + glyph_tick) * 16.0).floor() as usize;
                let glyph = &KATA_5X7[g_idx.min(KATA_5X7.len() - 1)];
                let col = if flash_col > 0.15 {
                    rgb(clamp_u8(165.0 + flash_col * 85.0), clamp_u8(235.0 + flash_col * 20.0), clamp_u8(90.0 + flash_col * 90.0))
                } else if is_special {
                    rgb(clamp_u8(175.0 + k * 70.0), clamp_u8(230.0 + k * 25.0), clamp_u8(85.0 + k * 75.0))
                } else {
                    drop_rgb(k, true, col_red)
                };
                let px_s = (rain_cell / 6).max(1);
                let gx0 = x_px + (rain_cell - 5 * px_s) / 2;
                let gy0 = y_px + (rain_cell - 7 * px_s) / 2;
                for (r, &bits) in glyph.iter().enumerate() {
                    for c in 0..5 {
                        if (bits >> (4 - c)) & 1 != 0 {
                            fill_rect(
                                hdc,
                                RECT {
                                    left: gx0 + c * px_s,
                                    top: gy0 + (r as i32) * px_s,
                                    right: gx0 + (c + 1) * px_s,
                                    bottom: gy0 + ((r as i32) + 1) * px_s,
                                },
                                col,
                            );
                        }
                    }
                }
            } else {
                let pad = st.s(1).max(1);
                let col = if flash_col > 0.25 {
                    rgb(clamp_u8(105.0 + flash_col * 95.0), clamp_u8(190.0 + flash_col * 55.0), clamp_u8(50.0 + flash_col * 60.0))
                } else {
                    drop_rgb(k * 0.72, false, col_red)
                };
                fill_rect(
                    hdc,
                    RECT {
                        left: x_px + pad,
                        top: y_px + pad,
                        right: x_px + rain_cell - pad,
                        bottom: y_px + rain_cell - pad,
                    },
                    col,
                );
            }
        }
    }
}

/// Hydraulic reverse V-bend wave during Anvil #02 landing impact (down -> snap back up -> settle).
fn eval_hydraulic_wave(bp: f64) -> f64 {
    let p = bp.clamp(0.0, 1.0);
    let pi = std::f64::consts::PI;
    if p < 0.36 {
        ((p / 0.36) * pi).sin()
    } else if p < 0.72 {
        -(((p - 0.36) / 0.36) * pi).sin() * 0.65
    } else {
        (((p - 0.72) / 0.28) * pi).sin() * 0.22
    }
}

unsafe fn draw_stage_snapshot(
    hdc: HDC,
    st: &AppState,
    sb_w: i32,
    y_base: i32,
    word_idx: u8,
    progress_f: f64,
    comp_squash: i32,
    impact_wave: f64,
    t: f64,
) {
    let cw = ((sb_w - st.s(44)) / (MAX_STAGE_WORD_COLS as i32)).max(2);
    let word_px_w = (MAX_STAGE_WORD_COLS as i32) * cw;
    let word_x0 = (sb_w - word_px_w) / 2;

    let g_etap = rasterize_bold_word("ЭТАП");
    let e_step = st.s(2).max(2);
    let ew = (g_etap.w as i32) * e_step;
    let eh = 7 * e_step;
    let ex = (sb_w - ew) / 2;
    let ey_base = y_base + st.s(4);
    let lw = (ex - word_x0 - st.s(7)).max(6);
    let rx_start = ex + ew + st.s(7);

    let phase_mode = screen_phase_mode(word_idx);
    let eff_p = if phase_mode == 2 {
        1.0
    } else if phase_mode == 1 {
        progress_f.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let fill_x = (word_x0 as f64) + eff_p * (word_px_w as f64);

    let out_col = rgb(8, 10, 13);
    // Variant 07 · "Гидравлический Обратный Прогиб": thick 10px flanking girders + V-bend flex!
    let bar_h = st.s(10).max(6);
    let seg = st.s(3).max(2);
    let flex_amp = st.s(6) as f64;
    let cy_bar_base = ey_base + eh / 2;

    let eval_vbend_dy = |px_center: i32| -> i32 {
        if impact_wave.abs() < 0.01 {
            return 0;
        }
        let nx = ((((px_center - word_x0) as f64) / (word_px_w.max(1) as f64)) * 2.0 - 1.0).clamp(-1.0, 1.0);
        (impact_wave * flex_amp * (1.0 - nx.abs())).round() as i32
    };

    // Pass 1: Black +1px outline under thick 10px girders & "ЭТАП"
    for (sx, width) in [(word_x0, lw), (rx_start, lw)] {
        let mut dx = 0;
        while dx < width {
            let px = sx + dx;
            let sw = seg.min(width - dx);
            let bend = eval_vbend_dy(px + sw / 2);
            let top_y = cy_bar_base + bend - bar_h / 2;
            fill_rect(hdc, RECT { left: px - 1, top: top_y - 1, right: px + sw + 1, bottom: top_y + bar_h + 1 }, out_col);
            dx += seg;
        }
    }
    for r in 0..7usize {
        for c in 0..g_etap.w {
            if g_etap.rows[r][c] {
                let px = ex + (c as i32) * e_step;
                let bend = eval_vbend_dy(px + e_step / 2);
                let py = ey_base + bend + (r as i32) * e_step;
                fill_rect(hdc, RECT { left: px - 1, top: py - 1, right: px + e_step + 1, bottom: py + e_step + 1 }, out_col);
            }
        }
    }

    // Pass 2: 2-row textured thick 10px beams flexing with the V-bend
    for (sx, width) in [(word_x0, lw), (rx_start, lw)] {
        let mut dx = 0;
        while dx < width {
            let px = sx + dx;
            let sw = seg.min(width - dx);
            let bend = eval_vbend_dy(px + sw / 2);
            let top_y = cy_bar_base + bend - bar_h / 2;
            for ri in 0..2i32 {
                let ry0 = top_y + (ri * bar_h) / 2;
                let ry1 = top_y + ((ri + 1) * bar_h) / 2;
                let col = if phase_mode == 0 {
                    comp_mosaic_rgb(px / 3, ri * 3 + 1)
                } else if phase_mode == 2 {
                    creeper_mosaic_rgb(px / 3, ri * 3 + 1, t)
                } else {
                    let d_edge = fill_x - (px as f64);
                    if d_edge >= 0.0 {
                        if d_edge < 6.0 {
                            if hash2_u32(px, (t * 18.0) as i32) > 0.72 { rgb(226, 68, 74) } else { rgb(204, 255, 84) }
                        } else {
                            creeper_mosaic_rgb(px / 3, ri * 3 + 1, t)
                        }
                    } else {
                        steel_mosaic_rgb(px / 3, ri * 3 + 1, t)
                    }
                };
                fill_rect(hdc, RECT { left: px, top: ry0, right: px + sw, bottom: ry1 }, col);
            }
            dx += seg;
        }
    }

    // "ЭТАП" pixel word
    for r in 0..7usize {
        for c in 0..g_etap.w {
            if !g_etap.rows[r][c] {
                continue;
            }
            let px = ex + (c as i32) * e_step;
            let bend = eval_vbend_dy(px + e_step / 2);
            let py = ey_base + bend + (r as i32) * e_step;
            let col = if phase_mode == 0 {
                comp_mosaic_rgb(c as i32, r as i32)
            } else if phase_mode == 2 {
                creeper_mosaic_rgb(c as i32, r as i32, t)
            } else {
                let d_edge = fill_x - (px as f64);
                if d_edge >= 0.0 {
                    if d_edge < 6.0 {
                        if hash2_u32(c as i32, (t * 18.0) as i32) > 0.72 { rgb(226, 68, 74) } else { rgb(204, 255, 84) }
                    } else {
                        creeper_mosaic_rgb(c as i32, r as i32, t)
                    }
                } else {
                    steel_mosaic_rgb(c as i32, r as i32, t)
                }
            };
            fill_rect(hdc, RECT { left: px, top: py, right: px + e_step, bottom: py + e_step }, col);
        }
    }

    // 2) Main bold morphing stage word ("КОМПОНЕНТЫ" / "УСТАНОВКА" / "ГОТОВО" / uninstall)
    let word = stage_word_by_idx(word_idx);
    let g_word = rasterize_bold_word(word);
    let col_off = (MAX_STAGE_WORD_COLS.saturating_sub(g_word.w)) / 2;
    let word_y = y_base + st.s(25);
    let word_h = (7 * cw - comp_squash).max(14);
    let fill_cols = progress_f.clamp(0.0, 1.0) * (MAX_STAGE_WORD_COLS as f64);

    // Pass 1: #10 +1px Black outline
    for r in 0..7usize {
        let ry0 = word_y + (r as i32) * word_h / 7;
        let ry1 = word_y + ((r as i32) + 1) * word_h / 7;
        for c in 0..g_word.w {
            if !g_word.rows[r][c] {
                continue;
            }
            let px = word_x0 + ((c + col_off) as i32) * cw;
            fill_rect(hdc, RECT { left: px - 1, top: ry0 - 1, right: px + cw + 1, bottom: ry1 + 1 }, out_col);
        }
    }

    // Pass 2: Creeper/Steel camo textured double-stem blocks with jagged thermal progress frontier
    let box_gap = if cw >= 4 { 1 } else { 0 };
    for r in 0..7usize {
        let ry0 = word_y + (r as i32) * word_h / 7;
        let ry1 = (word_y + ((r as i32) + 1) * word_h / 7 - box_gap).max(ry0 + 1);
        let eff_fill = fill_cols + (hash2_u32(r as i32, (t * 14.0) as i32) - 0.5) * 2.4;
        for c in 0..g_word.w {
            if !g_word.rows[r][c] {
                continue;
            }
            let gc = (c + col_off) as i32;
            let px = word_x0 + gc * cw;
            let dist_edge = eff_fill - (gc as f64);
            let col = if phase_mode == 2 {
                creeper_mosaic_rgb(gc, r as i32, t)
            } else if phase_mode == 1 {
                if dist_edge >= 0.0 {
                    if dist_edge < 1.9 {
                        if hash2_u32(gc, (t * 18.0) as i32) > 0.72 { rgb(226, 68, 74) } else { rgb(204, 255, 84) }
                    } else {
                        creeper_mosaic_rgb(gc, r as i32, t)
                    }
                } else {
                    steel_mosaic_rgb(gc, r as i32, t)
                }
            } else {
                comp_mosaic_rgb(gc, r as i32)
            };
            fill_rect(hdc, RECT { left: px, top: ry0, right: px + cw - box_gap, bottom: ry1 }, col);
        }
    }
}

/// Render unified Stage block ("══ ЭТАП ══" Hydraulic V-Bend + bold word) with heavy bottom-to-top Anvil slam & return shock `0.0..1.0`.
unsafe fn draw_stage_block(hdc: HDC, st: &AppState, sb_w: i32, stage_rc: RECT, now_ms: u64, t: f64) -> f64 {
    const MORPH_DUR_MS: u64 = 410;
    const SHOCK_DECAY_MS: u64 = 600;
    let y0 = stage_rc.top;
    let box_h = stage_rc.h();
    let cur_idx = screen_to_word_idx(st.screen);
    let prog = (st.progress.min(100) as f64) / 100.0;

    let saved = SaveDC(hdc);
    IntersectClipRect(hdc, 0, y0 - st.s(12), sb_w, stage_rc.bottom + st.s(18));

    let elapsed_morph = if st.morph_start_ms > 0 { now_ms.saturating_sub(st.morph_start_ms) } else { u64::MAX };
    let in_morph = elapsed_morph < MORPH_DUR_MS;
    let impact_at_ms = ((MORPH_DUR_MS as f64) * 0.42).round() as u64;
    let shock = if st.morph_start_ms > 0 && elapsed_morph >= impact_at_ms {
        let se = ((elapsed_morph - impact_at_ms) as f64) / (SHOCK_DECAY_MS as f64);
        if se < 1.0 { (1.0 - se).powf(1.35) } else { 0.0 }
    } else {
        0.0
    };

    if !in_morph {
        draw_stage_snapshot(hdc, st, sb_w, y0, cur_idx, prog, 0, 0.0, t);
    } else {
        let u = (elapsed_morph as f64) / (MORPH_DUR_MS as f64);
        let split_at = 0.42;
        let to_p = if st.morph_to_idx == 2 { 1.0 } else { prog };
        let from_p = if st.morph_from_idx == 1 || st.morph_from_idx == 4 { 1.0 } else { 0.0 };

        if u < split_at {
            // Accelerating bottom -> top heavy Anvil slam
            let p = u / split_at;
            let off = (p.powf(2.0) * (box_h as f64)).round() as i32;
            draw_stage_snapshot(hdc, st, sb_w, y0 - off, st.morph_from_idx, from_p, 0, 0.0, t);
            draw_stage_snapshot(hdc, st, sb_w, y0 + box_h - off, st.morph_to_idx, to_p, 0, 0.0, t);

            // Sync seam flash riding the upward frame boundary
            let seam_y = y0 + box_h - off;
            fill_rect(hdc, RECT { left: 0, top: seam_y - st.s(4), right: sb_w, bottom: seam_y + st.s(4) }, rgb(7, 9, 12));
            fill_rect(hdc, RECT { left: st.s(22), top: seam_y - 1, right: sb_w - st.s(22), bottom: seam_y + 1 }, rgb(204, 255, 84));
        } else {
            // Hard 17px Anvil recoil + Hydraulic V-bend + vertical squash
            let bp = (u - split_at) / (1.0 - split_at);
            let dy = eval_anvil_impact(bp, st.s(17) as f64);
            let wave = eval_hydraulic_wave(bp);
            let comp_h = if bp < 0.18 {
                (((bp / 0.18) * std::f64::consts::PI).sin() * (st.s(5) as f64)).round() as i32
            } else {
                0
            };
            draw_stage_snapshot(hdc, st, sb_w, y0 + dy, st.morph_to_idx, to_p, comp_h, wave, t);
        }
    }

    RestoreDC(hdc, saved);
    shock
}

fn format_elapsed(st: &AppState) -> String {
    let Some(start) = st.install_start_ms else {
        return "00:00".to_string();
    };
    let end = st.install_end_ms.unwrap_or_else(win32::tick_count);
    let secs = end.saturating_sub(start) / 1000;
    format!("{:02}:{:02}", secs / 60, secs % 60)
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

    st.hits.clear();
    fill_rect(mem, cr, COL_BG);

    let sb_w = st.s(284);
    paint_header(mem, st, sb_w, cw);
    paint_sidebar(mem, st, sb_w, ch);
    paint_content(mem, st, sb_w, cw, ch);
    draw_square_border(mem, cr, COL_OUTER_BORDER);

    BitBlt(hdc_screen, 0, 0, cw, ch, mem, 0, 0, 0x00CC_0020 /* SRCCOPY */);
    SelectObject(mem, old_bmp);
    DeleteObject(bmp as HGDIOBJ);
    DeleteDC(mem);
}

unsafe fn paint_header(hdc: HDC, st: &mut AppState, sb_w: i32, cw: i32) {
    let f_brand = make_font(st, 15, FW_BOLD, false);
    let f_ver = make_font(st, 12, FW_BOLD, false);
    let f_btn = make_font(st, 12, FW_BOLD, false);

    // Centered over creeper head column (0 .. sb_w): "open" white + "frankendriver" green + [ver]
    let w_open = text_width(hdc, f_brand, "open");
    let w_fran = text_width(hdc, f_brand, "frankendriver");
    let ver_str = &st.meta.driver_version;
    let w_ver_txt = text_width(hdc, f_ver, ver_str);
    let ver_box_w = w_ver_txt + st.s(14);
    let total_w = w_open + w_fran + st.s(10) + ver_box_w;
    let bx = ((sb_w - total_w) / 2).max(st.s(10));
    let by = st.s(9);

    draw_text(hdc, "open", RECT { left: bx, top: by, right: bx + w_open + 2, bottom: st.s(30) }, f_brand, COL_TEXT, DT_LEFT | DT_SINGLELINE);
    draw_text(hdc, "frankendriver", RECT { left: bx + w_open, top: by, right: bx + w_open + w_fran + 2, bottom: st.s(30) }, f_brand, COL_GREEN, DT_LEFT | DT_SINGLELINE);

    let vx = bx + w_open + w_fran + st.s(10);
    let v_rc = RECT { left: vx, top: st.s(9), right: vx + ver_box_w, bottom: st.s(29) };
    draw_square_box(hdc, v_rc, rgb(24, 30, 22), rgb(66, 104, 24));
    draw_text(hdc, ver_str, v_rc, f_ver, COL_GREEN, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // Right flush window controls: [Copy log] [—] [×]
    let hdr_h = st.s(35);
    let lock_close = matches!(st.screen, Screen::Installing | Screen::Uninstalling);
    let close_rc = RECT { left: cw - st.s(41), top: 1, right: cw - 1, bottom: hdr_h };
    let close_bg = if lock_close {
        rgb(76, 36, 40)
    } else if st.hover == HitId::BtnWinClose {
        COL_RED_HOVER
    } else {
        COL_RED
    };
    fill_rect(hdc, close_rc, close_bg);
    {
        let pen = CreatePen(PS_SOLID, st.s(2).max(1), if lock_close { COL_MUTED } else { COL_TEXT });
        let op = SelectObject(hdc, pen as HGDIOBJ);
        let mx = (close_rc.left + close_rc.right) / 2;
        let my = (close_rc.top + close_rc.bottom) / 2;
        let d = st.s(5);
        MoveToEx(hdc, mx - d, my - d, std::ptr::null_mut());
        LineTo(hdc, mx + d + 1, my + d + 1);
        MoveToEx(hdc, mx + d, my - d, std::ptr::null_mut());
        LineTo(hdc, mx - d - 1, my + d + 1);
        SelectObject(hdc, op);
        DeleteObject(pen as HGDIOBJ);
    }
    if !lock_close {
        st.hits.push(HitRect { id: HitId::BtnWinClose, rect: close_rc });
    }

    let min_rc = RECT { left: close_rc.left - st.s(40), top: 1, right: close_rc.left, bottom: hdr_h };
    fill_rect(hdc, min_rc, if st.hover == HitId::BtnWinMin { COL_CARD_HOVER } else { COL_CARD });
    {
        let mx = (min_rc.left + min_rc.right) / 2;
        let my = (min_rc.top + min_rc.bottom) / 2;
        fill_rect(hdc, RECT { left: mx - st.s(5), top: my, right: mx + st.s(6), bottom: my + st.s(2).max(2) }, COL_TEXT);
    }
    st.hits.push(HitRect { id: HitId::BtnWinMin, rect: min_rc });

    // Variant 01 · Classic NVIDIA Green #76B900 GitHub Octocat icon button (left of Minimize)
    let gh_rc = RECT { left: min_rc.left - st.s(40), top: 1, right: min_rc.left, bottom: hdr_h };
    let gh_hovered = st.hover == HitId::BtnGitHub;
    let gh_bg = if gh_hovered { COL_CARD_HOVER } else { COL_CARD };
    fill_rect(hdc, gh_rc, gh_bg);
    draw_github_icon(hdc, st, gh_rc, gh_hovered, gh_bg);
    st.hits.push(HitRect { id: HitId::BtnGitHub, rect: gh_rc });

    let copy_w = (text_width(hdc, f_btn, "Copy log") + st.s(42)).max(text_width(hdc, f_btn, "✓ Copied!") + st.s(20)).max(st.s(102));
    let copy_rc = RECT { left: gh_rc.left - copy_w, top: 1, right: gh_rc.left, bottom: hdr_h };
    let is_copied = win32::tick_count() < st.copied_until_ms;
    let copy_bg = if is_copied {
        COL_GREEN
    } else if st.hover == HitId::BtnCopyLog {
        COL_CARD_HOVER
    } else {
        COL_CARD
    };
    fill_rect(hdc, copy_rc, copy_bg);
    if is_copied {
        draw_text(hdc, "✓ Copied!", copy_rc, f_btn, COL_DARK_TXT, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    } else {
        let icon_rc = RECT {
            left: copy_rc.left + st.s(14),
            top: copy_rc.top + st.s(12),
            right: copy_rc.left + st.s(23),
            bottom: copy_rc.top + st.s(24),
        };
        draw_square_box(hdc, icon_rc, rgb(42, 54, 36), COL_GREEN);
        fill_rect(hdc, RECT { left: icon_rc.left + st.s(2), top: icon_rc.top - st.s(2), right: icon_rc.right - st.s(2), bottom: icon_rc.top + 1 }, COL_GREEN);
        draw_text(
            hdc,
            "Copy log",
            RECT { left: icon_rc.right + st.s(6), top: copy_rc.top, right: copy_rc.right - st.s(6), bottom: copy_rc.bottom },
            f_btn,
            COL_TEXT,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }
    st.hits.push(HitRect { id: HitId::BtnCopyLog, rect: copy_rc });

    for f in [f_brand, f_ver, f_btn] {
        DeleteObject(f as HGDIOBJ);
    }
}

fn col_argb(a: u8, c: u32) -> u32 {
    let r = c & 0xFF;
    let g = (c >> 8) & 0xFF;
    let b = (c >> 16) & 0xFF;
    ((a as u32) << 24) | (r << 16) | (g << 8) | b
}

/// Variant 01 · Classic NVIDIA Green #76B900 circular GitHub Octocat icon left of Minimize.
unsafe fn draw_github_icon(hdc: HDC, st: &AppState, rc: RECT, hovered: bool, bg_col: u32) {
    let mut gfx: *mut c_void = std::ptr::null_mut();
    if GdipCreateFromHDC(hdc, &mut gfx) != 0 || gfx.is_null() {
        return;
    }
    let _ = GdipSetSmoothingMode(gfx, 4 /* AntiAlias */);

    let cx = ((rc.left + rc.right) as f32) * 0.5;
    let cy = ((rc.top + rc.bottom) as f32) * 0.5;
    let r = (st.s(9) as f32).max(7.0);
    let fg = if hovered { rgb(138, 224, 16) } else { COL_GREEN };

    let mut br_fg: *mut c_void = std::ptr::null_mut();
    let mut br_cut: *mut c_void = std::ptr::null_mut();
    let mut pen_tail: *mut c_void = std::ptr::null_mut();
    let _ = GdipCreateSolidFill(col_argb(255, fg), &mut br_fg);
    let _ = GdipCreateSolidFill(col_argb(255, bg_col), &mut br_cut);
    let _ = GdipCreatePen1(col_argb(255, bg_col), (r * 0.14).max(1.2), 2, &mut pen_tail);

    if !br_fg.is_null() && !br_cut.is_null() {
        let _ = GdipFillEllipse(gfx, br_fg, cx - r, cy - r, r * 2.0, r * 2.0);

        // Octocat head dome
        let _ = GdipFillEllipse(gfx, br_cut, cx - r * 0.50, cy - r * 0.45, r * 1.0, r * 0.78);
        // Pointed cat ears
        let ear_l = [
            POINTF { x: cx - r * 0.44, y: cy - r * 0.22 },
            POINTF { x: cx - r * 0.40, y: cy - r * 0.62 },
            POINTF { x: cx - r * 0.14, y: cy - r * 0.36 },
        ];
        let ear_r = [
            POINTF { x: cx + r * 0.44, y: cy - r * 0.22 },
            POINTF { x: cx + r * 0.40, y: cy - r * 0.62 },
            POINTF { x: cx + r * 0.14, y: cy - r * 0.36 },
        ];
        let _ = GdipFillPolygon(gfx, br_cut, ear_l.as_ptr(), 3, 0);
        let _ = GdipFillPolygon(gfx, br_cut, ear_r.as_ptr(), 3, 0);

        // Octocat torso & tentacles meeting bottom arc
        let torso = [
            POINTF { x: cx - r * 0.28, y: cy + r * 0.26 },
            POINTF { x: cx + r * 0.28, y: cy + r * 0.26 },
            POINTF { x: cx + r * 0.32, y: cy + r * 0.96 },
            POINTF { x: cx - r * 0.32, y: cy + r * 0.96 },
        ];
        let _ = GdipFillPolygon(gfx, br_cut, torso.as_ptr(), 4, 0);

        // Curly Octocat tail curving up from bottom-left
        if !pen_tail.is_null() {
            let _ = GdipSetPenStartCap(pen_tail, 2);
            let _ = GdipSetPenEndCap(pen_tail, 2);
            let _ = GdipDrawBezier(
                gfx, pen_tail,
                cx - r * 0.24, cy + r * 0.68,
                cx - r * 0.56, cy + r * 0.68,
                cx - r * 0.52, cy + r * 0.30,
                cx - r * 0.72, cy + r * 0.24,
            );
        }
    }

    if !pen_tail.is_null() { let _ = GdipDeletePen(pen_tail); }
    if !br_cut.is_null() { let _ = GdipDeleteBrush(br_cut); }
    if !br_fg.is_null() { let _ = GdipDeleteBrush(br_fg); }
    let _ = GdipDeleteGraphics(gfx);
}

unsafe fn spawn_hearts(st: &mut AppState, count: usize) {
    let scale = (st.dpi as f64) / 96.0;
    let c = &mut st.cat;
    let top_y = c.y + c.jump_y - if c.rear_amt > 0.4 { 40.0 * scale } else { 18.0 * scale };
    for i in 0..count {
        let rx = c.next_rand() - 0.5;
        let ry = c.next_rand();
        let r_vx = c.next_rand() - 0.5;
        let r_vy = c.next_rand();
        c.hearts.push(HeartParticle {
            x: c.x + rx * 28.0 * scale,
            y: top_y - ry * 10.0 * scale,
            vx: r_vx * 44.0 * scale,
            vy: (-36.0 - r_vy * 48.0) * scale,
            size: ((9.0 + (i % 4) as f64 * 1.4) * scale) as f32,
            life: 1.2 + (i % 3) as f64 * 0.2,
            tint_idx: i,
        });
    }
}

unsafe fn poke_cat(hwnd: HWND, st: &mut AppState) {
    let now_ms = win32::tick_count();
    st.cat.poke_times_ms.retain(|&ts| now_ms.saturating_sub(ts) <= 5_000);
    st.cat.poke_times_ms.push(now_ms);
    let streak = st.cat.poke_times_ms.len();
    let is_boosty = streak >= 4;

    if is_boosty {
        spawn_hearts(st, 22);
        st.cat.boosty_banner_t = 4.5;
        st.cat.poke_times_ms.clear();
        let op = to_wide("open");
        let url = to_wide("https://boosty.to/defrag");
        ShellExecuteW(hwnd, op.as_ptr(), url.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOW);
    } else {
        let cnt = match streak {
            1 => 4,
            2 => 8,
            _ => 13,
        };
        spawn_hearts(st, cnt);
    }

    let c = &mut st.cat;
    let roll_rand = c.next_rand();
    let wants_belly = is_boosty || streak >= 2 || roll_rand < 0.65;
    if !wants_belly {
        return;
    }

    let dur = if is_boosty { 2.8 } else { 2.2 + (streak as f64) * 0.35 };
    if c.on_pc && matches!(c.behavior, CatBehavior::SleepPc | CatBehavior::WakePc | CatBehavior::PcBelly) {
        c.caught = None;
        c.behavior = CatBehavior::PcBelly;
        c.belly_t = dur;
        return;
    }
    if c.jump_y < -1.5 || matches!(c.behavior, CatBehavior::HopOffPc | CatBehavior::JumpToPc) {
        c.pending_belly_t = c.pending_belly_t.max(dur);
        return;
    }
    c.wants_pc_jump = false;
    c.caught = None;
    c.behavior = CatBehavior::BellyBliss;
    c.belly_t = dur;
}

fn relax_ribbon(nodes_x: &mut [f64; 6], nodes_y: &mut [f64; 6], floor_y: f64, dt: f64, grav: f64, seg_len: f64) {
    for i in 1..6 {
        nodes_y[i] = (nodes_y[i] + grav * dt).min(floor_y - 2.0);
        let dx = nodes_x[i] - nodes_x[i - 1];
        let dy = nodes_y[i] - nodes_y[i - 1];
        let d = dx.hypot(dy).max(0.001);
        if d > seg_len {
            nodes_x[i] = nodes_x[i - 1] + (dx / d) * seg_len;
            nodes_y[i] = (nodes_y[i - 1] + (dy / d) * seg_len).min(floor_y - 2.0);
        }
    }
}

fn impulse_rain(c: &mut CatState, sb_w: i32, rain_cell: i32, px: f64, dir_now: f64, strength: f64, radius: f64) {
    let cols = ((sb_w / rain_cell) as usize).min(40);
    for gx in 0..cols {
        let cx = (gx as i32 * rain_cell + rain_cell / 2) as f64;
        let d = (cx - px).abs();
        if d < radius {
            let falloff = 1.0 - d / radius;
            let push_dir = if (cx - px).abs() < 1.0 { dir_now } else { (cx - px).signum() };
            c.rain_sway_v[gx] += push_dir * strength * falloff * 18.0;
            c.rain_flash[gx] = c.rain_flash[gx].max(0.75 * falloff);
        }
    }
}

unsafe fn tick_cat_and_matrix(hwnd: HWND, st: &mut AppState) {
    let now_ms = win32::tick_count();
    let prev_ms = st.cat.last_tick_ms;
    st.cat.last_tick_ms = now_ms;
    let dt = ((now_ms.saturating_sub(prev_ms)) as f64 / 1000.0).clamp(0.005, 0.05);
    let t = (now_ms.saturating_sub(st.anim_start_ms) as f64) / 1000.0;

    let mut cr = RECT::default();
    GetClientRect(hwnd, &mut cr);
    let ch = cr.h().max(st.s(420));
    let sb_w = st.s(284);
    let hdr_h = st.s(36);
    let scale = (st.dpi as f64) / 96.0;
    let floor_y = (ch - st.s(10)) as f64;
    let rain_cell = st.s(8).max(6);

    // Must match BF-2 tower + iiyama 413 #1 coordinates in draw_pc_corner
    let right_edge = (sb_w - st.s(22)) as f64;
    let pc_tw_w = 28.0 * scale;
    let pc_tw_h = 58.0 * scale;
    let pc_tw_dx = 10.0 * scale;
    let pc_tw_dy = 6.0 * scale;
    let pc_tw_x = right_edge - pc_tw_w - pc_tw_dx - 2.0 * scale;
    let pc_tw_y = floor_y - pc_tw_h;
    let pc_mon_x = right_edge - 109.0 * scale;
    let sleep_x = pc_tw_x + pc_tw_w * 0.5 + pc_tw_dx * 0.45;
    let sleep_y = pc_tw_y + pc_tw_dy * 0.45;

    let floor_left = st.s(30) as f64;
    let floor_right = (pc_tw_x - (st.s(12) as f64)).max(floor_left + 30.0);
    let pc_launch_x = (pc_tw_x - 22.0 * scale).clamp(floor_left + 30.0, floor_right);

    // Initialize falling rain positions cleanly once window metrics exist
    if !st.cat.rain_init {
        st.cat.rain_init = true;
        st.cat.x = sleep_x;
        st.cat.y = sleep_y;
        st.cat.hop_from_x = sleep_x;
        st.cat.hop_from_y = sleep_y;
        for gx in 0..40 {
            st.cat.rain_y[gx] = -hash2_u32(gx as i32, 99) * (ch as f64);
        }
    }

    // Live cursor polling
    let mut pt = POINT { x: -1, y: -1 };
    GetCursorPos(&mut pt);
    ScreenToClient(hwnd, &mut pt);
    let in_sidebar = pt.x >= 0 && pt.x < sb_w && pt.y >= hdr_h && pt.y <= ch;

    let mx = pt.x as f64;
    let my = pt.y as f64;
    let prev_mx = st.cat.mouse_x;
    let prev_my = st.cat.mouse_y;

    st.cat.mouse_speed *= 0.85;
    if in_sidebar {
        if !st.cat.mouse_active {
            st.cat.mouse_active = true;
            st.cat.last_move_ms = now_ms;
            st.cat.cursor_bored = false;
        } else {
            let move_dist = (mx - prev_mx).hypot(my - prev_my);
            st.cat.mouse_speed = move_dist * 24.0;
            if move_dist > 4.0 * scale {
                st.cat.last_move_ms = now_ms;
                st.cat.cursor_bored = false;
            }
        }
        st.cat.mouse_x = mx;
        st.cat.mouse_y = my;
    } else {
        st.cat.mouse_active = false;
        st.cat.cursor_bored = false;
    }

    let c = &mut st.cat;
    c.phase += dt;
    c.timer -= dt;
    c.hop_cooldown = (c.hop_cooldown - dt).max(0.0);
    c.low_rest_t = (c.low_rest_t - dt).max(0.0);
    c.boosty_banner_t = (c.boosty_banner_t - dt).max(0.0);
    c.poke_times_ms.retain(|&ts| now_ms.saturating_sub(ts) <= 5_000);
    c.blink_t -= dt;
    if c.blink_t < -0.14 {
        c.blink_t = 2.2 + c.next_rand() * 2.8;
    }

    let h_above = floor_y - c.mouse_y;
    let zone = if !c.mouse_active || c.cursor_bored {
        0 // NONE
    } else if h_above <= 42.0 * scale {
        1 // LOW_PLAY
    } else if h_above <= 78.0 * scale {
        2 // MID_POUNCE
    } else {
        3 // HIGH_WALL
    };

    if zone != 1 {
        c.low_play_budget = (c.low_play_budget + dt * 0.7).min(5.0);
    }

    // Boredom check after 10-13 seconds of stationary mouse
    if c.mouse_active && !c.cursor_bored && now_ms.saturating_sub(c.last_move_ms) >= c.bored_timeout_ms {
        c.cursor_bored = true;
        let r = c.next_rand();
        c.bored_timeout_ms = 10_200 + (r * 2_700.0) as u64;
        if matches!(
            c.behavior,
            CatBehavior::WallReach | CatBehavior::ChaseRun | CatBehavior::SwatCursor | CatBehavior::AmbushCursor | CatBehavior::SitWatchUp
        ) {
            c.behavior = CatBehavior::Walk;
            c.timer = 2.0;
            c.phase = 0.0;
            c.tease_time = 0.0;
        }
    }

    // Priority pipeline: SleepOnPC -> HopOffPc -> Belly/PlayCaught -> Airborne -> Grounded Cursor/Autonomous
    if c.on_pc {
        c.x = sleep_x;
        c.y = sleep_y;
        match c.behavior {
            CatBehavior::SleepPc => {
                c.belly_roll += (0.0 - c.belly_roll) * (dt * 8.0).min(1.0);
                c.sleep_timer -= dt;
                if c.sleep_timer <= 0.0 {
                    c.behavior = CatBehavior::WakePc;
                    c.timer = 1.05;
                    c.phase = 0.0;
                }
            }
            CatBehavior::PcBelly => {
                c.belly_roll += (1.0 - c.belly_roll) * (dt * 7.5).min(1.0);
                c.belly_t -= dt;
                if c.belly_t <= 0.0 {
                    c.behavior = CatBehavior::WakePc;
                    c.timer = 0.85;
                    c.phase = 0.0;
                }
            }
            CatBehavior::WakePc => {
                c.belly_roll += (0.0 - c.belly_roll) * (dt * 8.0).min(1.0);
                if c.timer <= 0.0 && c.belly_roll < 0.08 {
                    c.on_pc = false;
                    c.behavior = CatBehavior::HopOffPc;
                    c.hop_from_x = c.x;
                    c.hop_from_y = c.y;
                    c.hop_p = 0.0;
                    c.dir = -1.0;
                }
            }
            _ => {
                c.behavior = CatBehavior::SleepPc;
            }
        }
    } else if c.behavior == CatBehavior::HopOffPc {
        c.belly_roll += (0.0 - c.belly_roll) * (dt * 10.0).min(1.0);
        c.hop_p += dt * 1.55;
        let p = c.hop_p.min(1.0);
        let dest_x = (pc_mon_x + 16.0 * scale).clamp(floor_left + 20.0, floor_right - 4.0 * scale);
        c.x = c.hop_from_x + (dest_x - c.hop_from_x) * p;
        c.y = c.hop_from_y + (floor_y - c.hop_from_y) * (p * p) - (p * std::f64::consts::PI).sin() * 28.0 * scale;
        if p >= 1.0 {
            c.y = floor_y;
            c.pc_return_cooldown = 13.0 + c.next_rand() * 7.0;
            c.wants_pc_jump = false;
            if c.pending_belly_t > 0.0 {
                c.behavior = CatBehavior::BellyBliss;
                c.belly_t = c.pending_belly_t;
                c.pending_belly_t = 0.0;
            } else {
                c.behavior = CatBehavior::Walk;
                c.target_x = floor_left + 40.0 * scale;
                c.timer = 2.4;
            }
        }
    } else if c.behavior == CatBehavior::PrepJumpPc {
        c.y = floor_y;
        c.belly_roll += (0.0 - c.belly_roll) * (dt * 10.0).min(1.0);
        c.dir = if sleep_x >= c.x { 1.0 } else { -1.0 };
        if zone != 0 {
            c.wants_pc_jump = false;
            c.behavior = CatBehavior::Walk;
            c.timer = 0.6;
        } else if c.timer <= 0.0 {
            c.behavior = CatBehavior::JumpToPc;
            c.hop_from_x = c.x;
            c.hop_from_y = floor_y;
            c.hop_p = 0.0;
            c.phase = 0.0;
        }
    } else if c.behavior == CatBehavior::JumpToPc {
        c.belly_roll += (0.0 - c.belly_roll) * (dt * 10.0).min(1.0);
        c.hop_p += dt * 1.50;
        let p = c.hop_p.min(1.0);
        let one_minus = 1.0 - p;
        let base_y = c.hop_from_y + (sleep_y - c.hop_from_y) * (1.0 - one_minus * one_minus);
        c.x = c.hop_from_x + (sleep_x - c.hop_from_x) * p;
        c.y = base_y - (p * std::f64::consts::PI).sin() * 24.0 * scale;
        c.dir = if sleep_x >= c.hop_from_x { 1.0 } else { -1.0 };
        if p >= 1.0 {
            c.on_pc = true;
            c.x = sleep_x;
            c.y = sleep_y;
            c.dir = -1.0;
            c.behavior = CatBehavior::SleepPc;
            c.sleep_timer = 7.0 + c.next_rand() * 4.5;
            c.pc_return_cooldown = 13.0 + c.next_rand() * 7.0;
            c.wants_pc_jump = false;
            c.pending_belly_t = 0.0;
            c.phase = 0.0;
        }
    } else if matches!(c.behavior, CatBehavior::BellyBliss | CatBehavior::PlayCaught) {
        c.y = floor_y;
        if c.behavior == CatBehavior::BellyBliss {
            c.rear_amt += (0.0 - c.rear_amt) * (dt * 10.0).min(1.0);
            let tr = if c.rear_amt < 0.25 { 1.0 } else { 0.0 };
            c.belly_roll += (tr - c.belly_roll) * (dt * 8.5).min(1.0);
        } else if c.caught.is_some() {
            if c.jump_y < 0.0 || c.jump_v < 0.0 {
                c.jump_v += 520.0 * scale * dt;
                c.jump_y += c.jump_v * dt;
                if c.jump_y >= 0.0 {
                    c.jump_y = 0.0;
                    c.jump_v = 0.0;
                }
            }
            let r1 = c.next_rand();
            let r2 = c.next_rand();
            if let Some(ref mut cd) = c.caught {
            match cd.mode {
                CatchPlayMode::BunnyKicks => {
                    c.rear_amt += (0.0 - c.rear_amt) * (dt * 11.0).min(1.0);
                    let tr = if c.rear_amt < 0.25 { 1.0 } else { 0.0 };
                    c.belly_roll += (tr - c.belly_roll) * (dt * 8.5).min(1.0);
                    if (t * 17.0).sin() > 0.85 && cd.vy > -10.0 * scale {
                        cd.vx = (r1 - 0.5) * 90.0 * scale;
                        cd.vy = (-52.0 - r2 * 44.0) * scale;
                    }
                    cd.vy += 260.0 * scale * dt;
                    cd.nodes_x[0] += cd.vx * dt;
                    cd.nodes_y[0] += cd.vy * dt;
                    cd.nodes_x[0] += (c.x + c.dir * 3.0 * scale - cd.nodes_x[0]) * (dt * 6.0).min(1.0);
                    if cd.nodes_y[0] > floor_y - 19.0 * scale {
                        cd.nodes_y[0] = floor_y - 19.0 * scale;
                        cd.vy = -44.0 * scale;
                    }
                    relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 100.0 * scale, 7.0 * scale);
                }
                CatchPlayMode::PawJuggle => {
                    c.belly_roll += (0.0 - c.belly_roll) * (dt * 9.0).min(1.0);
                    cd.vy += 240.0 * scale * dt;
                    cd.nodes_x[0] += cd.vx * dt;
                    cd.nodes_y[0] += cd.vy * dt;
                    if cd.nodes_x[0] < floor_left || cd.nodes_x[0] > floor_right {
                        cd.vx = -cd.vx;
                    }
                    let dx = cd.nodes_x[0] - c.x;
                    if dx.abs() > 6.0 * scale {
                        c.dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                        let mv = dx.signum() * dx.abs().min(68.0 * scale * dt);
                        c.x = (c.x + mv).clamp(floor_left, floor_right);
                        c.walk_cycle += mv.abs() * 0.30;
                    }
                    if cd.nodes_y[0] >= floor_y - 26.0 * scale && cd.vy > 0.0 {
                        cd.vy = (-125.0 - r1 * 35.0) * scale;
                        cd.vx = (r2 - 0.5) * 70.0 * scale;
                        c.poke_flick = 1.0;
                    }
                    c.poke_flick = (c.poke_flick - dt * 3.5).max(0.0);
                    relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 90.0 * scale, 7.5 * scale);
                }
                CatchPlayMode::MouthShake => {
                    c.belly_roll += (0.0 - c.belly_roll) * (dt * 9.0).min(1.0);
                    let whip = (t * 22.0).sin();
                    cd.nodes_x[0] = c.x + c.dir * 18.0 * scale + whip * 7.0 * scale;
                    cd.nodes_y[0] = floor_y - 21.0 * scale + (t * 11.0).cos() * 3.0 * scale;
                    cd.nodes_x[1] += whip * 120.0 * scale * dt;
                    relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 65.0 * scale, 8.0 * scale);
                }
                CatchPlayMode::FloorRoll => {
                    c.belly_roll += (0.0 - c.belly_roll) * (dt * 9.0).min(1.0);
                    cd.vy += 280.0 * scale * dt;
                    cd.vx *= 0.985;
                    cd.nodes_x[0] += cd.vx * dt;
                    cd.nodes_y[0] += cd.vy * dt;
                    if cd.nodes_y[0] >= floor_y - 4.0 * scale {
                        cd.nodes_y[0] = floor_y - 4.0 * scale;
                        cd.vy = -cd.vy.abs() * 0.35;
                    }
                    if cd.nodes_x[0] < floor_left {
                        cd.nodes_x[0] = floor_left;
                        cd.vx = cd.vx.abs() + 55.0 * scale;
                    }
                    if cd.nodes_x[0] > floor_right {
                        cd.nodes_x[0] = floor_right;
                        cd.vx = -cd.vx.abs() - 55.0 * scale;
                    }
                    let dx = cd.nodes_x[0] - c.x;
                    c.dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                    if dx.abs() > 15.0 * scale {
                        let mv = dx.signum() * dx.abs().min(96.0 * scale * dt);
                        c.x = (c.x + mv).clamp(floor_left, floor_right);
                        c.walk_cycle += mv.abs() * 0.34;
                    } else {
                        let sign = if r1 < 0.5 { -1.0 } else { 1.0 };
                        cd.vx = sign * (85.0 + r2 * 55.0) * scale;
                        cd.vy = -48.0 * scale;
                        c.poke_flick = 1.0;
                    }
                    c.poke_flick = (c.poke_flick - dt * 3.5).max(0.0);
                    relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 90.0 * scale, 7.5 * scale);
                }
                CatchPlayMode::HighToss => {
                    // 5th mode: tosses letter sky-high, sits watching upward, mini-hops to swat again
                    c.belly_roll += (0.0 - c.belly_roll) * (dt * 9.0).min(1.0);
                    cd.vy += 235.0 * scale * dt;
                    cd.nodes_x[0] = (cd.nodes_x[0] + cd.vx * dt).clamp(floor_left, floor_right);
                    cd.nodes_y[0] += cd.vy * dt;
                    if cd.nodes_x[0] <= floor_left + 1.0 || cd.nodes_x[0] >= floor_right - 1.0 {
                        cd.vx = -cd.vx;
                    }
                    let dx = cd.nodes_x[0] - c.x;
                    if dx.abs() > 4.0 * scale {
                        c.dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                    }
                    if dx.abs() > 14.0 * scale {
                        let mv = dx.signum() * dx.abs().min(72.0 * scale * dt);
                        c.x = (c.x + mv).clamp(floor_left, floor_right);
                        c.walk_cycle += mv.abs() * 0.28;
                    }
                    if cd.nodes_y[0] >= floor_y - 30.0 * scale && cd.vy > 0.0 {
                        cd.vy = (-172.0 - r1 * 26.0) * scale;
                        cd.vx = (r2 - 0.5) * 60.0 * scale;
                        if c.jump_y >= 0.0 {
                            c.jump_v = -96.0 * scale;
                        }
                        c.poke_flick = 1.0;
                    }
                    c.poke_flick = (c.poke_flick - dt * 3.2).max(0.0);
                    relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 85.0 * scale, 7.5 * scale);
                }
                CatchPlayMode::OrbitSpin => {
                    // 6th mode: rapid figure-8 spin around the cat while chasing the swinging 5-cube ribbon
                    c.belly_roll += (0.0 - c.belly_roll) * (dt * 9.0).min(1.0);
                    let ang = t * 5.6;
                    let tx = (c.x + ang.sin() * 26.0 * scale).clamp(floor_left, floor_right);
                    let ty = floor_y - 12.0 * scale - (ang * 2.0).cos().abs() * 11.0 * scale;
                    cd.nodes_x[0] += (tx - cd.nodes_x[0]) * (dt * 14.0).min(1.0);
                    cd.nodes_y[0] += (ty - cd.nodes_y[0]) * (dt * 14.0).min(1.0);
                    c.dir = if ang.cos() >= 0.0 { 1.0 } else { -1.0 };
                    c.walk_cycle += dt * 11.5;
                    c.poke_flick = (ang * 2.0).sin().abs();
                    relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 70.0 * scale, 7.5 * scale);
                }
                CatchPlayMode::PounceSlam => {
                    // 7th mode: sneaky ambush crouch -> sudden front-paw slam pinning twitching letter
                    c.belly_roll += (0.0 - c.belly_roll) * (dt * 9.0).min(1.0);
                    let cycle = (t * 1.15) % 1.0;
                    if cycle < 0.58 {
                        // Letter twitches slightly like prey while cat wiggles in ambush
                        cd.nodes_x[0] = (cd.nodes_x[0] + (t * 28.0).sin() * 24.0 * scale * dt).clamp(floor_left, floor_right);
                        cd.nodes_y[0] = floor_y - 5.0 * scale - ((t * 18.0).sin().max(0.0) * 6.0 * scale);
                        let dx = cd.nodes_x[0] - c.x;
                        c.dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                    } else if cycle < 0.80 {
                        // Slam! Cat pounces onto letter pinning it down
                        let dx = cd.nodes_x[0] - c.x;
                        let mv = dx.signum() * dx.abs().min(120.0 * scale * dt);
                        c.x = (c.x + mv).clamp(floor_left, floor_right);
                        cd.nodes_x[0] = (c.x + c.dir * 13.0 * scale).clamp(floor_left, floor_right);
                        cd.nodes_y[0] = floor_y - 3.5 * scale;
                        c.poke_flick = 1.0;
                    } else {
                        // Bats it slightly away for next crouch
                        cd.nodes_x[0] = (cd.nodes_x[0] + c.dir * 75.0 * scale * dt).clamp(floor_left, floor_right);
                        cd.nodes_y[0] = floor_y - 10.0 * scale;
                        c.poke_flick = (c.poke_flick - dt * 4.0).max(0.0);
                    }
                    relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 90.0 * scale, 7.2 * scale);
                }
            }
            }
        }
        c.belly_t -= dt;
        if c.belly_t <= 0.0 && c.jump_y >= 0.0 {
            let was_belly = c.belly_roll > 0.35;
            c.caught = None;
            c.jump_y = 0.0;
            c.jump_v = 0.0;
            c.behavior = if was_belly { CatBehavior::RecoverBelly } else { CatBehavior::Walk };
            c.timer = if was_belly { 0.4 } else { 2.0 };
            c.phase = 0.0;
        }
    } else if c.behavior == CatBehavior::RecoverBelly {
        c.y = floor_y;
        c.belly_roll += (0.0 - c.belly_roll) * (dt * 8.5).min(1.0);
        if c.belly_roll < 0.06 {
            c.belly_roll = 0.0;
            c.behavior = CatBehavior::Walk;
            c.timer = 2.0;
            c.phase = 0.0;
        }
    } else if c.jump_y < 0.0 || c.jump_v < 0.0 || matches!(c.behavior, CatBehavior::SidePounce | CatBehavior::PounceCatch | CatBehavior::PounceCursor | CatBehavior::HopGrab) {
        c.belly_roll += (0.0 - c.belly_roll) * (dt * 10.0).min(1.0);
        if c.behavior == CatBehavior::PounceCatch && c.caught.is_none() {
            let tgx = c.target_col_idx;
            let drop_x = (tgx as i32 * rain_cell + rain_cell / 2) as f64;
            let drop_y = c.rain_y[tgx];
            if (drop_x - c.x).hypot(drop_y - (floor_y + c.jump_y - 22.0 * scale)) < 32.0 * scale {
                let g_idx = (hash2_u32(tgx as i32, (t * 5.0) as i32) * 16.0).floor() as usize;
                let mode = match c.next_play_mode % 7 {
                    0 => CatchPlayMode::BunnyKicks,
                    1 => CatchPlayMode::PawJuggle,
                    2 => CatchPlayMode::MouthShake,
                    3 => CatchPlayMode::FloorRoll,
                    4 => CatchPlayMode::HighToss,
                    5 => CatchPlayMode::OrbitSpin,
                    _ => CatchPlayMode::PounceSlam,
                };
                c.next_play_mode += 1;
                let mut nx = [0.0; 6];
                let mut ny = [0.0; 6];
                for i in 0..6 {
                    nx[i] = drop_x;
                    ny[i] = drop_y - (i as f64) * (rain_cell as f64);
                }
                c.caught = Some(CaughtRibbon {
                    glyph_idx: g_idx.min(KATA_5X7.len() - 1),
                    is_red: c.rain_red[tgx],
                    mode,
                    vx: (c.next_rand() - 0.5) * 45.0 * scale,
                    vy: -25.0 * scale,
                    nodes_x: nx,
                    nodes_y: ny,
                });
                c.rain_flash[tgx] = 1.0;
                c.rain_y[tgx] = -48.0 * scale;
            }
        }
        if let Some(ref mut cd) = c.caught {
            if c.behavior == CatBehavior::PounceCatch {
                cd.nodes_x[0] = c.x + c.dir * 10.0 * scale;
                cd.nodes_y[0] = floor_y + c.jump_y - 20.0 * scale;
                relax_ribbon(&mut cd.nodes_x, &mut cd.nodes_y, floor_y, dt, 60.0 * scale, 7.5 * scale);
            }
        }
        c.jump_v += 520.0 * scale * dt;
        c.jump_y += c.jump_v * dt;
        c.x = (c.x + c.jump_vx * dt).clamp(floor_left, floor_right);
        if c.jump_y >= 0.0 {
            c.jump_y = 0.0;
            c.jump_v = 0.0;
            c.jump_vx = 0.0;
            if c.pending_belly_t > 0.0 {
                c.behavior = CatBehavior::BellyBliss;
                c.belly_t = c.pending_belly_t;
                c.pending_belly_t = 0.0;
            } else if c.behavior == CatBehavior::PounceCatch {
                if c.caught.is_some() {
                    c.behavior = CatBehavior::PlayCaught;
                    c.belly_t = 2.8;
                } else {
                    c.behavior = CatBehavior::Walk;
                    c.timer = 1.4;
                }
            } else if c.behavior == CatBehavior::PounceCursor {
                c.behavior = CatBehavior::SwatCursor;
                c.timer = 0.75;
            } else if c.behavior == CatBehavior::SidePounce {
                c.behavior = CatBehavior::SitWatchUp;
                c.timer = 1.4;
            } else {
                c.behavior = CatBehavior::WallReach;
                c.timer = 0.8;
            }
            c.phase = 0.0;
        }
    } else {
        // Grounded cursor zones + Autonomous AI
        c.y = floor_y;
        c.belly_roll += (0.0 - c.belly_roll) * (dt * 10.0).min(1.0);
        let mdx = c.mouse_x - c.x;
        let mut engaged = false;

        if zone == 1 && c.low_rest_t <= 0.0 {
            engaged = true;
            c.low_play_budget -= dt;
            if c.low_play_budget <= 0.0 {
                c.low_rest_t = 3.8 + c.next_rand() * 1.8;
                c.low_play_budget = 4.5 + c.next_rand() * 2.0;
                c.behavior = if c.next_rand() < 0.5 { CatBehavior::Loaf } else { CatBehavior::SitWatchUp };
                c.timer = 2.2;
                engaged = false;
            } else {
                c.low_cycle_t -= dt;
                if !matches!(c.behavior, CatBehavior::ChaseRun | CatBehavior::SwatCursor | CatBehavior::AmbushCursor) {
                    if c.next_rand() < 0.35 && mdx.abs() > 22.0 * scale {
                        c.behavior = CatBehavior::AmbushCursor;
                        c.timer = 0.9 + c.next_rand() * 0.6;
                    } else {
                        c.behavior = CatBehavior::ChaseRun;
                        c.timer = 0.8 + c.next_rand() * 0.7;
                    }
                    c.low_cycle_t = c.timer;
                    c.phase = 0.0;
                }
                if c.behavior == CatBehavior::ChaseRun {
                    c.dir = if mdx >= 0.0 { 1.0 } else { -1.0 };
                    if mdx.abs() > 22.0 * scale {
                        let spd = (62.0 * scale + mdx.abs() * 0.65).min(115.0 * scale);
                        let step = mdx.signum() * (mdx.abs() - 18.0 * scale).min(spd * dt);
                        c.x = (c.x + step).clamp(floor_left, floor_right);
                        c.walk_cycle += step.abs() * 0.30;
                    }
                    if mdx.abs() <= 28.0 * scale {
                        c.behavior = if c.next_rand() < 0.58 { CatBehavior::SwatCursor } else { CatBehavior::AmbushCursor };
                        c.timer = 0.95;
                        c.phase = 0.0;
                    } else if c.low_cycle_t <= 0.0 {
                        c.behavior = CatBehavior::AmbushCursor;
                        c.timer = 1.05;
                        c.phase = 0.0;
                    }
                } else if c.behavior == CatBehavior::SwatCursor {
                    c.dir = if mdx >= 0.0 { 1.0 } else { -1.0 };
                    if c.timer <= 0.0 {
                        if mdx.abs() > 30.0 * scale && c.next_rand() < 0.55 {
                            c.behavior = CatBehavior::AmbushCursor;
                            c.timer = 0.95;
                        } else if mdx.abs() > 24.0 * scale {
                            c.behavior = CatBehavior::ChaseRun;
                            c.timer = 1.1;
                        } else {
                            c.timer = 0.65;
                        }
                        c.phase = 0.0;
                    }
                } else if c.behavior == CatBehavior::AmbushCursor {
                    c.dir = if mdx >= 0.0 { 1.0 } else { -1.0 };
                    if c.timer <= 0.0 {
                        c.behavior = CatBehavior::PounceCursor;
                        c.jump_v = -128.0 * scale;
                        c.jump_vx = (mdx * 2.1).clamp(-135.0 * scale, 135.0 * scale);
                        c.phase = 0.0;
                    }
                }
            }
        } else if zone == 2 {
            engaged = true;
            if c.behavior == CatBehavior::WallReach {
                if mdx < -8.0 * scale { c.active_paw = -1; } else if mdx > 8.0 * scale { c.active_paw = 1; }
            } else if c.behavior == CatBehavior::SitWatchUp {
                c.dir = if mdx >= 0.0 { 1.0 } else { -1.0 };
                c.tease_time += if c.mouse_speed > 10.0 * scale || mdx.abs() > 20.0 * scale { dt * 1.35 } else { dt * 0.35 };
                if c.tease_time > 1.25 {
                    c.behavior = CatBehavior::WallReach;
                    c.active_paw = if mdx < 0.0 { -1 } else { 1 };
                    c.phase = 0.0;
                } else if mdx.abs() > 54.0 * scale && c.hop_cooldown <= 0.0 {
                    c.behavior = CatBehavior::SidePounce;
                    c.jump_v = -125.0 * scale;
                    c.jump_vx = mdx.signum() * (115.0 * scale).min(mdx.abs() * 1.6);
                    c.hop_cooldown = 1.25;
                }
            } else {
                c.dir = if mdx >= 0.0 { 1.0 } else { -1.0 };
                if c.hop_cooldown <= 0.0 && mdx.abs() > 14.0 * scale {
                    c.behavior = CatBehavior::SidePounce;
                    c.jump_v = -130.0 * scale;
                    c.jump_vx = mdx.signum() * (120.0 * scale).min(mdx.abs() * 1.5);
                    c.hop_cooldown = 1.25;
                } else {
                    c.behavior = CatBehavior::SitWatchUp;
                    c.timer = 1.2;
                }
            }
        } else if zone == 3 {
            engaged = true;
            if c.behavior != CatBehavior::WallReach {
                c.behavior = CatBehavior::WallReach;
                c.timer = 0.5;
                c.phase = 0.0;
            }
            if mdx < -8.0 * scale { c.active_paw = -1; } else if mdx > 8.0 * scale { c.active_paw = 1; }

            let target_foot_x = c.mouse_x.clamp(floor_left, floor_right);
            let delta_x = target_foot_x - c.x;
            if delta_x.abs() > 4.0 * scale && c.rear_amt > 0.45 {
                let step = delta_x.signum() * delta_x.abs().min(76.0 * scale * dt);
                c.x += step;
                c.dir = if delta_x >= 0.0 { 1.0 } else { -1.0 };
                c.step_vel = step / dt;
                c.hind_step_phase += step.abs() * 0.22;
                c.hip_sway = c.hind_step_phase.sin() * 2.5 * scale;
                c.lean_ang += ((delta_x * 0.003).clamp(-0.16, 0.16) - c.lean_ang) * (dt * 10.0).min(1.0);
            } else {
                c.step_vel *= 0.82;
                c.hip_sway = (t * 2.0).sin() * 0.9 * scale;
                c.lean_ang += ((mdx * 0.002) - c.lean_ang) * (dt * 8.0).min(1.0);
            }

            c.poke_wait -= dt;
            if c.poke_wait <= 0.0 {
                c.poke_flick = 1.0;
                c.poke_wait = 0.55 + c.next_rand() * 0.65;
            }
            c.poke_flick = (c.poke_flick - dt * 3.0).max(0.0);

            let reachable = c.mouse_y >= floor_y - 96.0 * scale;
            if c.rear_amt >= 0.85 && c.phase > 0.35 && reachable && c.hop_cooldown <= 0.0 && mdx.abs() < 38.0 * scale {
                c.behavior = CatBehavior::HopGrab;
                let need = (floor_y - 62.0 * scale - c.mouse_y).clamp(18.0 * scale, 48.0 * scale);
                c.jump_v = -(135.0 * scale + need * 1.25);
                c.jump_vx = (mdx * 1.1).clamp(-45.0 * scale, 45.0 * scale);
                c.hop_cooldown = 1.25;
            }
        }

        if engaged {
            c.wants_pc_jump = false;
        }
        if !engaged {
            c.tease_time = (c.tease_time - dt * 2.0).max(0.0);
            if c.behavior != CatBehavior::AmbushDrop {
                c.pc_return_cooldown = (c.pc_return_cooldown - dt).max(0.0);
            }
            if c.behavior == CatBehavior::AmbushDrop {
                c.wants_pc_jump = false;
                let tgx = c.target_col_idx;
                let tx = (tgx as i32 * rain_cell + rain_cell / 2) as f64;
                let ty = c.rain_y[tgx];
                if ty > floor_y + 8.0 || c.timer <= 0.0 {
                    c.behavior = CatBehavior::Walk;
                    c.timer = 1.5;
                } else {
                    let dx = tx - c.x;
                    c.dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                    if dx.abs() > 40.0 * scale {
                        let mv = dx.signum() * dx.abs().min(78.0 * scale * dt);
                        c.x = (c.x + mv).clamp(floor_left, floor_right);
                        c.walk_cycle += mv.abs() * 0.30;
                    }
                    if ty >= floor_y - 102.0 * scale && ty <= floor_y - 54.0 * scale && dx.abs() < 52.0 * scale {
                        c.behavior = CatBehavior::PounceCatch;
                        c.jump_v = -192.0 * scale;
                        c.jump_vx = (dx * 2.1).clamp(-145.0 * scale, 145.0 * scale);
                        c.phase = 0.0;
                    }
                }
            } else {
                let r_cols = ((sb_w + rain_cell - 1) / rain_cell).min(40) as usize;
                let mut found_ambush = None;
                if !c.wants_pc_jump && matches!(c.behavior, CatBehavior::Walk | CatBehavior::Loaf | CatBehavior::Groom | CatBehavior::SitWatchUp) {
                    for gx in 0..r_cols {
                        if !c.rain_interesting[gx] { continue; }
                        let xb = (gx as i32 * rain_cell + rain_cell / 2) as f64;
                        let yb = c.rain_y[gx];
                        if xb >= floor_left && xb <= floor_right && (xb - c.x).abs() <= 95.0 * scale
                            && yb >= floor_y - 160.0 * scale && yb <= floor_y - 96.0 * scale
                        {
                            found_ambush = Some(gx);
                            break;
                        }
                    }
                }
                if let Some(gx) = found_ambush {
                    c.target_col_idx = gx;
                    c.behavior = CatBehavior::AmbushDrop;
                    c.timer = 3.2;
                    c.phase = 0.0;
                } else if c.pc_return_cooldown <= 0.0 && !c.wants_pc_jump
                    && matches!(c.behavior, CatBehavior::Walk | CatBehavior::Loaf | CatBehavior::Groom | CatBehavior::SitWatchUp)
                {
                    c.wants_pc_jump = true;
                    c.behavior = CatBehavior::Walk;
                    c.target_x = pc_launch_x;
                    c.timer = 5.5;
                    c.phase = 0.0;
                } else if !c.wants_pc_jump && (c.timer <= 0.0 || matches!(c.behavior, CatBehavior::ChaseRun | CatBehavior::SwatCursor | CatBehavior::AmbushCursor | CatBehavior::WallReach)) {
                    let r = c.next_rand();
                    if r < 0.36 {
                        c.behavior = CatBehavior::Walk;
                        c.target_x = floor_left + c.next_rand() * (floor_right - floor_left);
                        c.timer = 2.0 + c.next_rand() * 1.4;
                    } else if r < 0.68 {
                        c.behavior = CatBehavior::RearBatLog;
                        let log_right = ((sb_w as f64) - 132.0 * scale).max(floor_left + 12.0 * scale);
                        if c.x > log_right {
                            c.x = floor_left + c.next_rand() * (log_right - floor_left);
                        }
                        c.active_paw = if c.next_rand() > 0.5 { 1 } else { -1 };
                        c.timer = 1.65;
                    } else if r < 0.84 {
                        c.behavior = CatBehavior::Loaf;
                        c.timer = 2.6 + c.next_rand() * 1.2;
                    } else {
                        c.behavior = CatBehavior::Groom;
                        c.timer = 2.2;
                    }
                    c.phase = 0.0;
                }

                if c.behavior == CatBehavior::Walk {
                    let dx = c.target_x - c.x;
                    if dx.abs() > 4.0 * scale {
                        c.dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                        let speed = if c.wants_pc_jump { 56.0 } else { 48.0 };
                        let mv = dx.signum() * dx.abs().min(speed * scale * dt);
                        c.x = (c.x + mv).clamp(floor_left, floor_right);
                        c.walk_cycle += mv.abs() * 0.28;
                    } else if c.wants_pc_jump {
                        c.behavior = CatBehavior::PrepJumpPc;
                        c.timer = 0.52;
                        c.phase = 0.0;
                        c.wants_pc_jump = false;
                        c.dir = 1.0;
                    } else {
                        c.target_x = floor_left + c.next_rand() * (floor_right - floor_left);
                    }
                }
            }
        }
    }

    // Smooth posture blending & matrix/log disturbance
    let wants_rear = if matches!(c.behavior, CatBehavior::WallReach | CatBehavior::HopGrab | CatBehavior::RearBatLog) { 1.0 } else { 0.0 };
    c.rear_amt += (wants_rear - c.rear_amt) * (dt * 10.0).min(1.0);

    let tgt_ext = if matches!(c.behavior, CatBehavior::WallReach | CatBehavior::HopGrab) {
        0.45 + (c.poke_flick * std::f64::consts::PI).sin() * 0.55
    } else if matches!(c.behavior, CatBehavior::SwatCursor | CatBehavior::RearBatLog | CatBehavior::PlayCaught) {
        0.45 + (c.phase * 12.0).sin() * 0.55
    } else {
        0.0
    };
    c.paw_extend += (tgt_ext - c.paw_extend) * (dt * 15.0).min(1.0);

    c.paw_touching = false;
    if matches!(c.behavior, CatBehavior::WallReach | CatBehavior::HopGrab | CatBehavior::RearBatLog) && c.rear_amt > 0.45 {
        let px = c.x + (c.active_paw as f64) * 8.0 * scale;
        let reach_off = if c.behavior == CatBehavior::RearBatLog {
            54.0 + c.paw_extend * 14.0
        } else {
            48.0 + c.paw_extend * 10.0
        };
        let py = floor_y + c.jump_y - reach_off * scale;
        c.paw_tip_x = px;
        c.paw_tip_y = py;
        c.paw_touching = true;
        let dir_now = c.dir;
        impulse_rain(c, sb_w, rain_cell, px, dir_now, 55.0 * dt, 22.0 * scale);
    }

    // Advance synced rain drops + pendulum sway
    for gx in 0..40 {
        c.rain_y[gx] += c.rain_speed[gx] * scale * dt;
        if c.rain_y[gx] > (ch as f64) + (46.0 * scale) {
            c.rain_y[gx] = -(14.0 * scale);
            c.interesting_seq = (c.interesting_seq + 1) % 7;
            c.rain_interesting[gx] = c.interesting_seq == 0;
        }
        let sp = -42.0 * c.rain_sway_x[gx] - 4.0 * c.rain_sway_v[gx];
        c.rain_sway_v[gx] += sp * dt;
        c.rain_sway_x[gx] = (c.rain_sway_x[gx] + c.rain_sway_v[gx] * dt).clamp(-20.0 * scale, 20.0 * scale);
        c.rain_flash[gx] = (c.rain_flash[gx] - dt * 1.4).max(0.0);
    }

    // Hearts & Log vortex shards
    let mut hi = 0;
    while hi < c.hearts.len() {
        c.hearts[hi].life -= dt;
        let sway = (t * 5.0 + (hi as f64)).sin() * 14.0 * scale;
        c.hearts[hi].x += (c.hearts[hi].vx + sway) * dt;
        c.hearts[hi].y += c.hearts[hi].vy * dt;
        c.hearts[hi].vy *= 0.985;
        if c.hearts[hi].life <= 0.0 {
            c.hearts.swap_remove(hi);
        } else {
            hi += 1;
        }
    }

    let mut si = 0;
    while si < c.shattered.len() {
        c.shattered[si].life -= dt;
        if c.shattered[si].life <= 0.0 {
            c.shattered.swap_remove(si);
        } else {
            si += 1;
        }
    }

    let px_t = c.paw_tip_x;
    let py_t = c.paw_tip_y;
    let mut shi = 0;
    while shi < c.shards.len() {
        c.shards[shi].life -= dt;
        if c.shards[shi].life <= 0.0 {
            c.shards.swap_remove(shi);
            continue;
        }
        let prog = 1.0 - (c.shards[shi].life / c.shards[shi].max_life);
        if prog < 0.44 {
            let sh = &mut c.shards[shi];
            sh.vx += (px_t - sh.x) * 15.0 * dt - (py_t - sh.y) * 10.0 * dt;
            sh.vy += (py_t - sh.y) * 15.0 * dt + (px_t - sh.x) * 10.0 * dt;
            sh.vx *= 0.93;
            sh.vy *= 0.93;
            sh.x += sh.vx * dt;
            sh.y += sh.vy * dt;
        } else {
            let pull = (dt * 12.0).min(1.0);
            let sh = &mut c.shards[shi];
            sh.x += (sh.tx - sh.x) * pull;
            sh.y += (sh.ty - sh.y) * pull;
        }
        shi += 1;
    }
}

unsafe fn fill_poly(gfx: *mut c_void, br: *mut c_void, pts: &[(f32, f32)]) {
    let p: Vec<POINTF> = pts.iter().map(|&(x, y)| POINTF { x, y }).collect();
    let _ = GdipFillPolygon(gfx, br, p.as_ptr(), p.len() as i32, 0);
}

unsafe fn draw_poly(gfx: *mut c_void, pen: *mut c_void, pts: &[(f32, f32)]) {
    let p: Vec<POINTF> = pts.iter().map(|&(x, y)| POINTF { x, y }).collect();
    let _ = GdipDrawPolygon(gfx, pen, p.as_ptr(), p.len() as i32);
}

unsafe fn draw_pc_corner(hdc: HDC, st: &AppState, sb_w: i32, ch: i32, t: f64) {
    let s = (st.dpi as f32) / 96.0;
    let right_edge = (sb_w - st.s(22)) as f32;
    let floor_y = (ch - st.s(10)) as f32;

    let mx = right_edge - 109.0 * s;
    let my = floor_y - 53.0 * s;
    let bw = 42.0 * s;
    let bh = 36.0 * s;
    let mcx = mx + bw * 0.5;
    let side_depth = 8.0 * s;

    let tw = 28.0 * s;
    let th = 58.0 * s;
    let dx = 10.0 * s;
    let dy = 6.0 * s;
    let tx = right_edge - tw - dx - 2.0 * s;
    let ty = floor_y - th;
    let fx = tx;
    let fy = ty + dy;
    let fh = th - dy;

    let mut gfx: *mut c_void = std::ptr::null_mut();
    if GdipCreateFromHDC(hdc, &mut gfx) == 0 && !gfx.is_null() {
        let _ = GdipSetSmoothingMode(gfx, 4);

        let mk_br = |a: u8, r: u8, g: u8, b: u8| -> *mut c_void {
            let mut br: *mut c_void = std::ptr::null_mut();
            let _ = GdipCreateSolidFill(argb(a, r, g, b), &mut br);
            br
        };
        let mk_pen = |a: u8, r: u8, g: u8, b: u8, w: f32| -> *mut c_void {
            let mut pen: *mut c_void = std::ptr::null_mut();
            let _ = GdipCreatePen1(argb(a, r, g, b), w, 2, &mut pen);
            pen
        };

        let mut brushes: Vec<*mut c_void> = Vec::new();
        let mut pens: Vec<*mut c_void> = Vec::new();
        let mut add_b = |a, r, g, b| -> *mut c_void {
            let ptr = mk_br(a, r, g, b);
            brushes.push(ptr);
            ptr
        };
        let mut add_p = |a, r, g, b, w| -> *mut c_void {
            let ptr = mk_pen(a, r, g, b, w);
            pens.push(ptr);
            ptr
        };

        // 0) Desk contact shadow + rear cable between monitor and tower
        let b_shad = add_b(118, 0, 0, 0);
        let _ = GdipFillEllipse(gfx, b_shad, right_edge - 114.0 * s, floor_y - 2.5 * s, 112.0 * s, 7.0 * s);
        let p_cable = add_p(255, 62, 73, 84, 1.5 * s);
        let _ = GdipDrawBezier(
            gfx,
            p_cable,
            mx + 42.0 * s, floor_y - 17.0 * s,
            tx - 6.0 * s, floor_y - 26.0 * s,
            tx - 2.0 * s, floor_y - 23.0 * s,
            tx, floor_y - 21.0 * s,
        );

        // 1) BF-2 Glass Glow Tall Mid-Tower (58px high, upper backlit blue fan, steady power button)
        let b_tw_side = add_b(255, 25, 32, 40);
        let b_tw_top = add_b(255, 58, 70, 84);
        let b_tw_front = add_b(255, 38, 47, 58);
        let p_tw_edge = add_p(255, 78, 95, 112, 1.0 * s);

        fill_poly(gfx, b_tw_side, &[
            (tx + tw, ty + dy),
            (tx + tw + dx, ty),
            (tx + tw + dx, floor_y - dy),
            (tx + tw, floor_y),
        ]);
        fill_poly(gfx, b_tw_top, &[
            (tx, ty + dy),
            (tx + dx, ty),
            (tx + tw + dx, ty),
            (tx + tw, ty + dy),
        ]);
        let b_glass = add_b(255, 11, 21, 34);
        fill_poly(gfx, b_glass, &[
            (tx + tw + 1.5 * s, ty + 8.0 * s),
            (tx + tw + dx - 1.5 * s, ty + 3.5 * s),
            (tx + tw + dx - 1.5 * s, floor_y - 12.0 * s),
            (tx + tw + 1.5 * s, floor_y - 7.0 * s),
        ]);
        // Internal cyan-blue glass glow + GPU edge bar
        let b_sg1 = add_b(46, 56, 189, 248);
        let b_sg2 = add_b(78, 56, 189, 248);
        let b_gpu = add_b(255, 125, 211, 252);
        let _ = GdipFillEllipse(gfx, b_sg1, tx + tw + 0.5 * s, ty + 14.0 * s, dx - 1.0 * s, 28.0 * s);
        let _ = GdipFillEllipse(gfx, b_sg2, tx + tw + 2.0 * s, ty + 20.0 * s, (dx - 4.0 * s).max(2.0), 16.0 * s);
        fill_poly(gfx, b_gpu, &[
            (tx + tw + 2.5 * s, ty + 27.0 * s),
            (tx + tw + dx - 2.5 * s, ty + 25.2 * s),
            (tx + tw + dx - 2.5 * s, ty + 27.2 * s),
            (tx + tw + 2.5 * s, ty + 29.0 * s),
        ]);

        // Tower front panel + upper fan bay
        fill_poly(gfx, b_tw_front, &[(fx, fy), (fx + tw, fy), (fx + tw, fy + fh), (fx, fy + fh)]);
        draw_poly(gfx, p_tw_edge, &[(fx, fy), (fx + tw, fy), (fx + tw, fy + fh), (fx, fy + fh)]);

        let b_bay = add_b(255, 17, 24, 34);
        fill_poly(gfx, b_bay, &[
            (fx + 3.5 * s, fy + 4.0 * s),
            (fx + tw - 3.5 * s, fy + 4.0 * s),
            (fx + tw - 3.5 * s, fy + 25.0 * s),
            (fx + 3.5 * s, fy + 25.0 * s),
        ]);

        // Backlit upper blue spinning fan (with 3-ring radial aura)
        let fcx = fx + tw * 0.5;
        let fcy = fy + 14.5 * s;
        let fr = 8.0 * s;
        let b_au1 = add_b(28, 37, 99, 235);
        let b_au2 = add_b(58, 56, 189, 248);
        let b_au3 = add_b(110, 56, 189, 248);
        let _ = GdipFillEllipse(gfx, b_au1, fcx - fr * 1.75, fcy - fr * 1.75, fr * 3.5, fr * 3.5);
        let _ = GdipFillEllipse(gfx, b_au2, fcx - fr * 1.35, fcy - fr * 1.35, fr * 2.7, fr * 2.7);
        let b_fcav = add_b(255, 12, 19, 28);
        let _ = GdipFillEllipse(gfx, b_fcav, fcx - fr, fcy - fr, fr * 2.0, fr * 2.0);
        let _ = GdipFillEllipse(gfx, b_au3, fcx - fr * 0.65, fcy - fr * 0.65, fr * 1.3, fr * 1.3);

        let p_fring = add_p(255, 56, 189, 248, 1.7 * s);
        let _ = GdipDrawEllipse(gfx, p_fring, fcx - fr + 0.6 * s, fcy - fr + 0.6 * s, (fr - 0.6 * s) * 2.0, (fr - 0.6 * s) * 2.0);
        let b_blade = add_b(235, 125, 211, 252);
        let br_len = (fr - 1.5 * s).max(2.0);
        for i in 0..5 {
            let a0 = (t as f32) * 5.8 + (i as f32) * (std::f32::consts::TAU / 5.0);
            let a1 = a0 + 0.56;
            let am = (a0 + a1) * 0.5;
            fill_poly(gfx, b_blade, &[
                (fcx, fcy),
                (fcx + a0.cos() * br_len, fcy + a0.sin() * br_len),
                (fcx + am.cos() * br_len, fcy + am.sin() * br_len),
                (fcx + a1.cos() * br_len, fcy + a1.sin() * br_len),
            ]);
        }
        let b_hub = add_b(255, 30, 58, 95);
        let p_hub = add_p(255, 186, 230, 253, 0.8 * s);
        let hr = fr * 0.30;
        let _ = GdipFillEllipse(gfx, b_hub, fcx - hr, fcy - hr, hr * 2.0, hr * 2.0);
        let _ = GdipDrawEllipse(gfx, p_hub, fcx - hr, fcy - hr, hr * 2.0, hr * 2.0);

        // Lower intake louver lines + STEADY non-blinking blue/cyan power button
        let b_vent = add_b(255, 20, 26, 33);
        for i in 0..4 {
            let vy = fy + (29.0 + (i as f32) * 3.8) * s;
            fill_poly(gfx, b_vent, &[
                (fx + 4.0 * s, vy),
                (fx + tw - 4.0 * s, vy),
                (fx + tw - 4.0 * s, vy + 1.8 * s),
                (fx + 4.0 * s, vy + 1.8 * s),
            ]);
        }
        let px = fx + tw * 0.5;
        let py = fy + fh - 5.0 * s;
        let pr = 2.2 * s;
        let b_btn_bg = add_b(255, 18, 24, 32);
        let p_btn = add_p(255, 56, 189, 248, 1.1 * s);
        let b_btn_core = add_b(255, 96, 165, 250);
        let _ = GdipFillEllipse(gfx, b_btn_bg, px - pr - 1.1 * s, py - pr - 1.1 * s, (pr + 1.1 * s) * 2.0, (pr + 1.1 * s) * 2.0);
        let _ = GdipDrawEllipse(gfx, p_btn, px - pr - 0.3 * s, py - pr - 0.3 * s, (pr + 0.3 * s) * 2.0, (pr + 0.3 * s) * 2.0);
        let _ = GdipFillEllipse(gfx, b_btn_core, px - pr * 0.55, py - pr * 0.55, pr * 1.1, pr * 1.1);

        // 2) Approved iiyama Vision Master Pro 413 #1 CRT Monitor (no top wedge!)
        let b_neck = add_b(255, 185, 180, 170);
        let b_base = add_b(255, 207, 202, 192);
        let p_base_hi = add_p(255, 240, 236, 228, 0.7 * s);
        fill_poly(gfx, b_neck, &[
            (mcx - 7.0 * s, my + bh - 1.0 * s),
            (mcx + 7.0 * s, my + bh - 1.0 * s),
            (mcx + 10.0 * s, floor_y - 7.0 * s),
            (mcx - 10.0 * s, floor_y - 7.0 * s),
        ]);
        let _ = GdipFillEllipse(gfx, b_base, mcx - 16.5 * s, floor_y - 10.0 * s, 33.0 * s, 7.6 * s);
        let _ = GdipDrawEllipse(gfx, p_base_hi, mcx - 11.5 * s, floor_y - 9.2 * s, 23.0 * s, 4.4 * s);

        // Compact curved CRT rear housing receding behind & down
        let b_crt_rear = add_b(255, 167, 162, 152);
        fill_poly(gfx, b_crt_rear, &[
            (mx + bw - 1.0 * s, my + 3.0 * s),
            (mx + bw + side_depth * 0.75, my + 4.8 * s),
            (mx + bw + side_depth, my + 9.0 * s),
            (mx + bw + side_depth, my + bh - 10.0 * s),
            (mx + bw + side_depth * 0.6, my + bh - 4.0 * s),
            (mx + bw - 1.0 * s, my + bh - 2.0 * s),
        ]);
        let p_mvent = add_p(255, 109, 104, 94, 0.85 * s);
        for i in 0..4 {
            let vy = (11.0 + (i as f32) * 3.8) * s;
            let _ = GdipDrawLine(gfx, p_mvent, mx + bw + 2.0 * s, my + vy, mx + bw + side_depth - 1.5 * s, my + vy + 1.0 * s);
        }

        // Front off-white HM703UT bezel with gently arched top (NO protrusion!)
        let b_bezel = add_b(255, 226, 222, 213);
        let p_bezel = add_p(255, 138, 133, 124, 1.0 * s);
        let bezel_pts = [
            (mx, my + 4.5 * s),
            (mx + 2.2 * s, my + 1.2 * s),
            (mx + 5.5 * s, my + 0.6 * s),
            (mcx, my - 0.6 * s),
            (mx + bw - 5.5 * s, my + 0.6 * s),
            (mx + bw - 2.2 * s, my + 1.2 * s),
            (mx + bw, my + 4.5 * s),
            (mx + bw, my + bh - 2.0 * s),
            (mx + bw - 2.0 * s, my + bh),
            (mx + 2.0 * s, my + bh),
            (mx, my + bh - 2.0 * s),
        ];
        fill_poly(gfx, b_bezel, &bezel_pts);
        draw_poly(gfx, p_bezel, &bezel_pts);

        // Signature purple/periwinkle arched stripe near top
        let p_arch = add_p(255, 126, 124, 178, 1.35 * s);
        let _ = GdipDrawBezier(
            gfx,
            p_arch,
            mx + 6.0 * s, my + 2.4 * s,
            mcx - 8.0 * s, my + 0.9 * s,
            mcx + 8.0 * s, my + 0.9 * s,
            mx + bw - 6.0 * s, my + 2.4 * s,
        );

        // Inner dark bezel + Diamondtron screen background
        let b_inframe = add_b(255, 43, 48, 54);
        fill_poly(gfx, b_inframe, &[
            (mx + 3.2 * s, my + 4.0 * s),
            (mx + bw - 3.2 * s, my + 4.0 * s),
            (mx + bw - 3.2 * s, my + bh - 8.0 * s),
            (mx + 3.2 * s, my + bh - 8.0 * s),
        ]);
        let sx = mx + 4.2 * s;
        let sy = my + 5.0 * s;
        let sw = bw - 8.4 * s;
        let sh = bh - 14.0 * s;
        let b_scr = add_b(255, 8, 19, 11);
        fill_poly(gfx, b_scr, &[(sx, sy), (sx + sw, sy), (sx + sw, sy + sh), (sx, sy + sh)]);
        let b_glare = add_b(16, 170, 255, 160);
        fill_poly(gfx, b_glare, &[(sx + 1.0 * s, sy + 1.0 * s), (sx + sw * 0.55, sy + 1.0 * s), (sx + 1.0 * s, sy + sh * 0.58)]);
        let b_scan = add_b(28, 118, 185, 0);
        let mut scy = sy + 2.0 * s;
        while scy < sy + sh - 1.0 * s {
            fill_poly(gfx, b_scan, &[(sx + 1.0 * s, scy), (sx + sw - 1.0 * s, scy), (sx + sw - 1.0 * s, scy + 0.7 * s), (sx + 1.0 * s, scy + 0.7 * s)]);
            scy += 2.5 * s;
        }
        let b_div = add_b(72, 118, 185, 0);
        fill_poly(gfx, b_div, &[
            (sx + 3.0 * s, sy + sh * 0.48),
            (sx + sw - 5.0 * s, sy + sh * 0.48),
            (sx + sw - 5.0 * s, sy + sh * 0.48 + 1.2 * s),
            (sx + 3.0 * s, sy + sh * 0.48 + 1.2 * s),
        ]);

        // Recessed OSD groove + blue logo + 5 round buttons + steady green power LED
        let gy = my + bh - 6.8 * s;
        let b_grv = add_b(255, 199, 194, 183);
        fill_poly(gfx, b_grv, &[(mx + 3.2 * s, gy), (mx + bw - 3.2 * s, gy), (mx + bw - 3.2 * s, gy + 4.6 * s), (mx + 3.2 * s, gy + 4.6 * s)]);
        let b_logo = add_b(255, 76, 125, 196);
        fill_poly(gfx, b_logo, &[(mx + 5.0 * s, gy + 1.5 * s), (mx + 9.0 * s, gy + 1.5 * s), (mx + 9.0 * s, gy + 3.0 * s), (mx + 5.0 * s, gy + 3.0 * s)]);
        let b_obtn = add_b(255, 112, 108, 100);
        for b in 0..5 {
            let bx_o = mx + (14.0 + (b as f32) * 3.8) * s;
            let _ = GdipFillEllipse(gfx, b_obtn, bx_o - 1.0 * s, gy + 1.3 * s, 2.0 * s, 2.0 * s);
        }
        let b_opwr = add_b(255, 140, 135, 126);
        let _ = GdipFillEllipse(gfx, b_opwr, mx + bw - 8.1 * s, gy + 0.7 * s, 3.2 * s, 3.2 * s);
        let b_oled = add_b(255, 118, 185, 0);
        let _ = GdipFillEllipse(gfx, b_oled, mx + bw - 11.0 * s, gy + 1.35 * s, 1.9 * s, 1.9 * s);

        // 3) Off-white mechanical keyboard + green Esc + Mouse on dark pad
        let kx0 = mx - 4.0 * s;
        let ky0 = floor_y - 8.5 * s;
        let kw0 = 38.0 * s;
        let kh0 = 8.5 * s;
        let b_kbbg = add_b(255, 158, 151, 137);
        let b_kcap = add_b(255, 236, 231, 220);
        let b_kesc = add_b(255, 118, 185, 0);
        fill_poly(gfx, b_kbbg, &[(kx0, ky0), (kx0 + kw0, ky0), (kx0 + kw0, ky0 + kh0), (kx0, ky0 + kh0)]);
        let pad_k = 1.4 * s;
        let k_cols = 10;
        let k_rows = 3;
        let kw = (kw0 - pad_k * 2.0 - 9.0 * 0.9 * s) / (k_cols as f32);
        let kh = (kh0 - pad_k * 2.0 - 2.0 * 0.9 * s) / (k_rows as f32);
        for r in 0..k_rows {
            for c in 0..k_cols {
                if r == 2 && c >= 3 && c <= 6 {
                    if c > 3 { continue; }
                    let sw = kw * 4.0 + 3.0 * 0.9 * s;
                    let rx = kx0 + pad_k + (c as f32) * (kw + 0.9 * s);
                    let ry = ky0 + pad_k + (r as f32) * (kh + 0.9 * s);
                    fill_poly(gfx, b_kcap, &[(rx, ry), (rx + sw, ry), (rx + sw, ry + kh), (rx, ry + kh)]);
                } else {
                    let rx = kx0 + pad_k + (c as f32) * (kw + 0.9 * s);
                    let ry = ky0 + pad_k + (r as f32) * (kh + 0.9 * s);
                    let br = if r == 0 && c == 0 { b_kesc } else { b_kcap };
                    fill_poly(gfx, br, &[(rx, ry), (rx + kw, ry), (rx + kw, ry + kh), (rx, ry + kh)]);
                }
            }
        }

        // Mousepad + Mouse with cord
        let mox = mx + 37.0 * s;
        let moy = floor_y - 8.0 * s;
        let mow = 7.5 * s;
        let moh = 8.5 * s;
        let b_mpad = add_b(255, 28, 35, 43);
        fill_poly(gfx, b_mpad, &[
            (mox - 2.5 * s, moy - 1.5 * s),
            (mox + mow + 2.5 * s, moy - 1.5 * s),
            (mox + mow + 2.5 * s, moy + moh + 1.5 * s),
            (mox - 2.5 * s, moy + moh + 1.5 * s),
        ]);
        let p_mcord = add_p(255, 75, 89, 104, 1.0 * s);
        let _ = GdipDrawBezier(
            gfx,
            p_mcord,
            mox + mow * 0.5, moy,
            mox + mow * 0.5 + 2.0 * s, moy - 4.0 * s,
            mx + 41.0 * s, floor_y - 12.0 * s,
            mx + 41.0 * s, floor_y - 14.0 * s,
        );
        let b_mbody = add_b(255, 220, 215, 205);
        let p_mrim = add_p(255, 17, 22, 28, 0.8 * s);
        let _ = GdipFillEllipse(gfx, b_mbody, mox, moy, mow, moh);
        let _ = GdipDrawEllipse(gfx, p_mrim, mox, moy, mow, moh);
        let _ = GdipDrawLine(gfx, p_mrim, mox + 1.0 * s, moy + moh * 0.42, mox + mow - 1.0 * s, moy + moh * 0.42);
        let _ = GdipDrawLine(gfx, p_mrim, mox + mow * 0.5, moy + 0.8 * s, mox + mow * 0.5, moy + moh * 0.42);
        fill_poly(gfx, b_kesc, &[
            (mox + mow * 0.5 - 0.8 * s, moy + 1.5 * s),
            (mox + mow * 0.5 + 0.8 * s, moy + 1.5 * s),
            (mox + mow * 0.5 + 0.8 * s, moy + 4.3 * s),
            (mox + mow * 0.5 - 0.8 * s, moy + 4.3 * s),
        ]);

        for b in brushes {
            if !b.is_null() { let _ = GdipDeleteBrush(b); }
        }
        for p in pens {
            if !p.is_null() { let _ = GdipDeletePen(p); }
        }
        let _ = GdipDeleteGraphics(gfx);
    }

    // Crisp GDI 01A CRT Screen Text: "DRV OK" at top + "C:\>" + blinking block cursor at BOTTOM!
    let sx_i = (mx + 4.2 * s).round() as i32;
    let sy_i = (my + 5.0 * s).round() as i32;
    let sw_i = (bw - 8.4 * s).round() as i32;
    let sh_i = (bh - 14.0 * s).round() as i32;

    let f_crt = make_mono_font(st, 6, FW_BOLD);
    draw_text(
        hdc,
        "DRV OK",
        RECT { left: sx_i + st.s(2), top: sy_i + st.s(1), right: sx_i + sw_i - st.s(2), bottom: sy_i + sh_i / 2 },
        f_crt,
        rgb(93, 160, 0),
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
    );
    let pr_top = sy_i + sh_i - st.s(10);
    let pr_left = sx_i + st.s(2);
    draw_text(
        hdc,
        "C:\\>",
        RECT { left: pr_left, top: pr_top, right: sx_i + sw_i - st.s(2), bottom: sy_i + sh_i },
        f_crt,
        rgb(141, 248, 128),
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
    );
    if (t * 5.2).sin() > -0.1 {
        let cw_pr = text_width(hdc, f_crt, "C:\\>");
        let cx_cur = (pr_left + cw_pr + st.s(1)).min(sx_i + sw_i - st.s(5));
        fill_rect(
            hdc,
            RECT {
                left: cx_cur,
                top: pr_top + st.s(2),
                right: cx_cur + st.s(3).max(3),
                bottom: pr_top + st.s(7).max(5),
            },
            rgb(168, 255, 96),
        );
    }
    DeleteObject(f_crt as HGDIOBJ);
}

unsafe fn draw_plush_tail_gdip(
    gfx: *mut c_void,
    p_rim: *mut c_void,
    p_fill: *mut c_void,
    b_tip: *mut c_void,
    pts: [(f32, f32); 4],
    sc: f32,
) {
    let _ = GdipDrawBezier(gfx, p_rim, pts[0].0, pts[0].1, pts[1].0, pts[1].1, pts[2].0, pts[2].1, pts[3].0, pts[3].1);
    let _ = GdipDrawBezier(gfx, p_fill, pts[0].0, pts[0].1, pts[1].0, pts[1].1, pts[2].0, pts[2].1, pts[3].0, pts[3].1);
    let _ = GdipFillEllipse(gfx, b_tip, pts[3].0 - 2.6 * sc, pts[3].1 - 2.2 * sc, 5.2 * sc, 4.4 * sc);
}

unsafe fn draw_fluffy_head_gdip(
    gfx: *mut c_void,
    b_body: *mut c_void,
    b_patch: *mut c_void,
    b_white: *mut c_void,
    b_ear: *mut c_void,
    b_eye: *mut c_void,
    b_dark: *mut c_void,
    p_rim: *mut c_void,
    p_eye: *mut c_void,
    hx: f32,
    hy: f32,
    tilt_rad: f32,
    flip_x: f32,
    sc: f32,
    happy_eyes: bool,
    wide_eyes: bool,
    look_x: f32,
    look_y: f32,
) {
    let (sa, ca) = tilt_rad.sin_cos();
    let xh = |lx: f32, ly: f32| -> (f32, f32) {
        let sx = lx * flip_x;
        (hx + (sx * ca - ly * sa) * sc, hy + (sx * sa + ly * ca) * sc)
    };
    let pt_h = |lx: f32, ly: f32| -> POINTF {
        let (x, y) = xh(lx, ly);
        POINTF { x, y }
    };

    // Ears
    let e1 = [pt_h(-3.7, -3.5), pt_h(-5.0, -11.5), pt_h(-0.2, -4.8)];
    let e2 = [pt_h(3.7, -3.5), pt_h(5.0, -11.5), pt_h(0.2, -4.8)];
    let _ = GdipFillPolygon(gfx, b_body, e1.as_ptr(), 3, 0);
    let _ = GdipDrawPolygon(gfx, p_rim, e1.as_ptr(), 3);
    let _ = GdipFillPolygon(gfx, b_patch, e2.as_ptr(), 3, 0);
    let _ = GdipDrawPolygon(gfx, p_rim, e2.as_ptr(), 3);
    let ei1 = [pt_h(-3.4, -4.4), pt_h(-4.4, -9.0), pt_h(-1.4, -5.0)];
    let ei2 = [pt_h(3.4, -4.4), pt_h(4.4, -9.0), pt_h(1.4, -5.0)];
    let _ = GdipFillPolygon(gfx, b_ear, ei1.as_ptr(), 3, 0);
    let _ = GdipFillPolygon(gfx, b_ear, ei2.as_ptr(), 3, 0);

    // Cheek-tuft fluffy polygon head silhouette
    let head_poly = [
        pt_h(-6.2, -2.5),
        pt_h(0.0, -5.6),
        pt_h(6.2, -2.5),
        pt_h(8.6, 0.2),
        pt_h(6.4, 2.4),
        pt_h(4.0, 5.2),
        pt_h(0.0, 5.8),
        pt_h(-4.0, 5.2),
        pt_h(-6.4, 2.4),
        pt_h(-8.6, 0.2),
    ];
    let _ = GdipFillPolygon(gfx, b_body, head_poly.as_ptr(), head_poly.len() as i32, 0);
    let _ = GdipDrawPolygon(gfx, p_rim, head_poly.as_ptr(), head_poly.len() as i32);

    // Ginger forehead & white muzzle
    let (fx, fy) = xh(2.2, -3.3);
    let _ = GdipFillEllipse(gfx, b_patch, fx - 3.0 * sc, fy - 1.8 * sc, 6.0 * sc, 3.6 * sc);
    let (mx, my) = xh(0.6, 2.6);
    let _ = GdipFillEllipse(gfx, b_white, mx - 4.0 * sc, my - 2.2 * sc, 8.0 * sc, 4.4 * sc);

    if happy_eyes {
        for ex in [-2.2f32, 2.4] {
            let (e0x, e0y) = xh(ex - 1.3, -0.5);
            let (e1x, e1y) = xh(ex + 1.3, -0.5);
            let _ = GdipDrawLine(gfx, p_eye, e0x, e0y, e1x, e1y);
        }
    } else {
        let w = if wide_eyes { 1.22 } else { 1.0 };
        for ex in [-2.0f32, 2.8] {
            let (ecx, ecy) = xh(ex, -0.8);
            let _ = GdipFillEllipse(gfx, b_eye, ecx - 1.8 * w * sc, ecy - 2.0 * w * sc, 3.6 * w * sc, 4.0 * w * sc);
            let _ = GdipFillEllipse(gfx, b_dark, ecx + (look_x * 0.65 - 0.72 * w) * sc, ecy + (look_y * 0.6 - 1.3 * w) * sc, 1.44 * w * sc, 2.6 * w * sc);
            let _ = GdipFillEllipse(gfx, b_white, ecx - 0.8 * sc, ecy - 1.3 * sc, 1.0 * sc, 1.0 * sc);
        }
    }
    let (nx, ny) = xh(0.2, 1.8);
    let _ = GdipFillEllipse(gfx, b_ear, nx - 1.0 * sc, ny - 0.8 * sc, 2.0 * sc, 1.6 * sc);
}

unsafe fn draw_calico_cat_gdip(hdc: HDC, st: &AppState, ch: i32, t: f64) {
    let mut gfx: *mut c_void = std::ptr::null_mut();
    if GdipCreateFromHDC(hdc, &mut gfx) != 0 || gfx.is_null() {
        return;
    }
    let _ = GdipSetSmoothingMode(gfx, 4);

    let scale = (st.dpi as f32) / 96.0;
    let sc = scale * 1.26;
    let floor_y = (ch - st.s(10)) as f32;
    let c = &st.cat;
    let bx = c.x as f32;
    let by = (c.y + c.jump_y) as f32;
    let tf = t as f32;

    let col_body = argb(255, 42, 50, 60);
    let col_patch = argb(255, 226, 136, 62);
    let col_white = argb(255, 238, 244, 250);
    let col_rim = argb(255, 184, 236, 76);
    let col_ear_in = argb(255, 242, 160, 172);
    let col_eye = argb(255, 158, 224, 38);
    let col_dark = argb(255, 8, 10, 13);

    let mut b_body: *mut c_void = std::ptr::null_mut();
    let mut b_patch: *mut c_void = std::ptr::null_mut();
    let mut b_white: *mut c_void = std::ptr::null_mut();
    let mut b_ear: *mut c_void = std::ptr::null_mut();
    let mut b_eye: *mut c_void = std::ptr::null_mut();
    let mut b_dark: *mut c_void = std::ptr::null_mut();
    let mut b_shad: *mut c_void = std::ptr::null_mut();

    let mut p_rim: *mut c_void = std::ptr::null_mut();
    let mut p_tail_rim: *mut c_void = std::ptr::null_mut();
    let mut p_tail_in: *mut c_void = std::ptr::null_mut();
    let mut p_thigh_rim: *mut c_void = std::ptr::null_mut();
    let mut p_thigh_in: *mut c_void = std::ptr::null_mut();
    let mut p_leg_rim: *mut c_void = std::ptr::null_mut();
    let mut p_leg_w: *mut c_void = std::ptr::null_mut();
    let mut p_eye: *mut c_void = std::ptr::null_mut();

    let _ = GdipCreateSolidFill(col_body, &mut b_body);
    let _ = GdipCreateSolidFill(col_patch, &mut b_patch);
    let _ = GdipCreateSolidFill(col_white, &mut b_white);
    let _ = GdipCreateSolidFill(col_ear_in, &mut b_ear);
    let _ = GdipCreateSolidFill(col_eye, &mut b_eye);
    let _ = GdipCreateSolidFill(col_dark, &mut b_dark);
    let _ = GdipCreateSolidFill(argb(120, 0, 0, 0), &mut b_shad);

    let _ = GdipCreatePen1(col_rim, 1.3 * sc, 2, &mut p_rim);
    let _ = GdipCreatePen1(col_rim, 5.6 * sc, 2, &mut p_tail_rim);
    let _ = GdipCreatePen1(col_body, 4.0 * sc, 2, &mut p_tail_in);
    let _ = GdipCreatePen1(col_rim, 4.6 * sc, 2, &mut p_thigh_rim);
    let _ = GdipCreatePen1(col_body, 3.2 * sc, 2, &mut p_thigh_in);
    let _ = GdipCreatePen1(col_rim, 3.8 * sc, 2, &mut p_leg_rim);
    let _ = GdipCreatePen1(col_white, 2.4 * sc, 2, &mut p_leg_w);
    let _ = GdipCreatePen1(col_eye, 1.6 * sc, 2, &mut p_eye);
    for pen in [p_tail_rim, p_tail_in, p_thigh_rim, p_thigh_in, p_leg_rim, p_leg_w, p_eye] {
        if !pen.is_null() {
            let _ = GdipSetPenStartCap(pen, 2);
            let _ = GdipSetPenEndCap(pen, 2);
            let _ = GdipSetPenLineJoin(pen, 2);
        }
    }

    let draw_paw_tip = |px: f32, py: f32, rx: f32, ry: f32| {
        let _ = GdipFillEllipse(gfx, b_white, px - rx * sc, py - ry * sc, rx * 2.0 * sc, ry * 2.0 * sc);
        let _ = GdipDrawEllipse(gfx, p_rim, px - rx * sc, py - ry * sc, rx * 2.0 * sc, ry * 2.0 * sc);
    };

    // Belly-up helper used on both PC roof and floor!
    let draw_belly = |cx: f32, gy: f32, bs: f32, r_blend: f32, playing: bool| {
        let rate = if playing { 17.0 } else { 7.5 };
        let k1 = (tf * rate).sin() * if playing { 3.8 } else { 1.8 };
        let k2 = (tf * rate).cos() * if playing { 3.8 } else { 1.8 };
        draw_plush_tail_gdip(
            gfx, p_tail_rim, p_tail_in, b_white,
            [
                (cx - 11.0 * bs, gy - 3.0 * bs),
                (cx - 18.0 * bs, gy - 1.0 * bs),
                (cx - 22.0 * bs, gy + (-6.0 + (tf * 4.0).sin() * 2.0) * bs),
                (cx - 19.0 * bs, gy - 11.0 * bs),
            ],
            bs,
        );
        let spine = [
            POINTF { x: cx - 13.0 * bs, y: gy - 3.5 * bs },
            POINTF { x: cx - 11.0 * bs, y: gy - 12.0 * bs },
            POINTF { x: cx, y: gy - 13.0 * bs },
            POINTF { x: cx + 11.0 * bs, y: gy - 10.5 * bs },
            POINTF { x: cx + 12.0 * bs, y: gy - 4.0 * bs },
            POINTF { x: cx, y: gy - 0.5 * bs },
        ];
        let _ = GdipFillPolygon(gfx, b_body, spine.as_ptr(), spine.len() as i32, 0);
        let _ = GdipDrawPolygon(gfx, p_rim, spine.as_ptr(), spine.len() as i32);
        if r_blend > 0.2 {
            let _ = GdipFillEllipse(gfx, b_white, cx - 9.4 * bs, gy - 13.4 * bs, 18.8 * bs, 9.0 * bs);
            let _ = GdipFillEllipse(gfx, b_patch, cx - 12.5 * bs, gy - 9.0 * bs, 7.2 * bs, 5.4 * bs);
            // Kicking hind legs + kneading front paws with round paw tips
            for (x0, y0, x1, y1) in [
                (cx - 8.5 * bs, gy - 6.0 * bs, cx - 8.5 * bs, gy + (-15.0 + k2) * bs),
                (cx - 4.5 * bs, gy - 9.0 * bs, cx - 3.0 * bs, gy + (-16.8 - k2) * bs),
                (cx + 4.0 * bs, gy - 9.0 * bs, cx + 5.0 * bs, gy + (-16.0 + k1) * bs),
                (cx + 7.5 * bs, gy - 8.5 * bs, cx + 10.5 * bs, gy + (-14.8 - k1) * bs),
            ] {
                let _ = GdipDrawLine(gfx, p_leg_rim, x0, y0, x1, y1);
                let _ = GdipDrawLine(gfx, p_leg_w, x0, y0, x1, y1);
                let _ = GdipFillEllipse(gfx, b_white, x1 - 2.0 * bs, y1 - 1.6 * bs, 4.0 * bs, 3.2 * bs);
            }
        }
        draw_fluffy_head_gdip(
            gfx, b_body, b_patch, b_white, b_ear, b_eye, b_dark, p_rim, p_eye,
            cx + 13.5 * bs, gy - 5.5 * bs, -2.35 * r_blend, 1.0, bs, !playing, playing, 0.0, -0.6,
        );
    };

    if c.on_pc {
        if c.belly_roll > 0.35 {
            draw_belly(bx, by, sc * 0.90, c.belly_roll as f32, false);
        } else {
            let stretch = if c.behavior == CatBehavior::WakePc { ((c.timer / 1.05).clamp(0.0, 1.0) * std::f64::consts::PI).sin() as f32 * 4.2 } else { 0.0 };
            let breath = if c.behavior == CatBehavior::SleepPc { (tf * 2.1).sin() * 0.45 } else { 0.0 };
            let stretch_fwd = if c.behavior == CatBehavior::WakePc { stretch * 0.9 } else { 0.0 };
            let wake_squint = c.behavior == CatBehavior::SleepPc || (c.behavior == CatBehavior::WakePc && c.timer > 0.38);
            let ps = sc * 0.90;
            draw_plush_tail_gdip(
                gfx, p_tail_rim, p_tail_in, b_white,
                [
                    (bx + 10.0 * ps, by - 3.0 * ps),
                    (bx + 15.0 * ps, by + 2.0 * ps),
                    (bx + 14.0 * ps, by + 8.0 * ps),
                    (bx + 11.0 * ps, by + (11.0 + (tf * 1.9).sin() * 1.8) * ps),
                ],
                ps,
            );
            let _ = GdipFillEllipse(gfx, b_body, bx - 12.0 * ps, by - (10.5 + stretch + breath) * ps, 24.0 * ps, (10.5 + stretch * 0.6 + breath) * ps);
            let _ = GdipDrawEllipse(gfx, p_rim, bx - 12.0 * ps, by - (10.5 + stretch + breath) * ps, 24.0 * ps, (10.5 + stretch * 0.6 + breath) * ps);
            let _ = GdipFillEllipse(gfx, b_patch, bx - 2.0 * ps, by - (9.5 + stretch + breath) * ps, 9.5 * ps, 5.5 * ps);
            // Two tucked/stretching front paws resting on top of the PC tower
            let _ = GdipFillEllipse(gfx, b_white, bx - (12.2 + stretch_fwd) * ps, by - 3.2 * ps, 4.4 * ps, 3.0 * ps);
            let _ = GdipDrawEllipse(gfx, p_rim, bx - (12.2 + stretch_fwd) * ps, by - 3.2 * ps, 4.4 * ps, 3.0 * ps);
            let _ = GdipFillEllipse(gfx, b_white, bx - (7.6 + stretch_fwd * 0.45) * ps, by - 2.8 * ps, 4.4 * ps, 3.0 * ps);
            let _ = GdipDrawEllipse(gfx, p_rim, bx - (7.6 + stretch_fwd * 0.45) * ps, by - 2.8 * ps, 4.4 * ps, 3.0 * ps);
            draw_fluffy_head_gdip(
                gfx, b_body, b_patch, b_white, b_ear, b_eye, b_dark, p_rim, p_eye,
                bx - (8.0 + stretch_fwd * 0.25) * ps, by - (6.0 + stretch * 0.3 + breath * 0.4) * ps, 0.0, 1.0, ps, wake_squint, false, 0.0, 0.0,
            );
        }
    } else if c.belly_roll > 0.22 {
        let _ = GdipFillEllipse(gfx, b_shad, bx - 19.0 * sc, floor_y - 2.0, 38.0 * sc, 5.5 * sc);
        draw_belly(bx, by, sc, c.belly_roll as f32, c.behavior == CatBehavior::PlayCaught);
    } else if (matches!(c.behavior, CatBehavior::WallReach | CatBehavior::HopGrab | CatBehavior::RearBatLog) && c.rear_amt > 0.35) {
        let sh_sc = (1.0f32 + (c.jump_y as f32) / 90.0).max(0.45);
        let _ = GdipFillEllipse(gfx, b_shad, bx - 15.0 * sc * sh_sc, floor_y - 2.0, 30.0 * sc * sh_sc, 5.5 * sc * sh_sc);

        let ap = c.active_paw as f32;
        let hip_x = bx + (c.hip_sway as f32);
        let spd_n = ((c.step_vel as f32).abs() / 70.0).clamp(0.0, 1.0);
        let ph = c.hind_step_phase as f32;
        let l_lift = ph.sin().max(0.0) * (1.2 + spd_n * 3.8);
        let r_lift = (-ph).sin().max(0.0) * (1.2 + spd_n * 3.8);
        let l_st = ph.cos() * spd_n * 3.5;
        let r_st = -ph.cos() * spd_n * 3.5;
        let tiptoe = if c.mouse_active && (c.mouse_y as f32) < floor_y - 85.0 * scale { 2.0 } else { 0.0 };
        let bounce = ph.sin().abs() * spd_n * -2.0 - tiptoe;

        let cy0 = by + bounce;
        let sh_x = hip_x + ap * 5.0 * sc;
        let sh_y = cy0 - 22.0 * sc;
        let (rx, ry) = if c.behavior != CatBehavior::RearBatLog && c.mouse_active {
            ((c.mouse_x as f32) - sh_x, (c.mouse_y as f32) - sh_y)
        } else {
            (ap * 3.0, -15.0)
        };
        let d_vec = rx.hypot(ry).max(1.0);
        let arm_len = (11.5 + (c.paw_extend as f32) * 4.5) * sc;
        let arm_x = sh_x + (rx / d_vec) * arm_len;
        let arm_y = sh_y + (ry / d_vec * arm_len).min(-5.0);
        let brace_x = hip_x - ap * 7.8 * sc;
        let brace_y = cy0 - 24.0 * sc;

        // Braced support arm + active stretching paw
        let _ = GdipDrawLine(gfx, p_leg_rim, hip_x - ap * 4.5 * sc, cy0 - 21.0 * sc, brace_x, brace_y);
        let _ = GdipDrawLine(gfx, p_leg_w, hip_x - ap * 4.5 * sc, cy0 - 21.0 * sc, brace_x, brace_y);
        let _ = GdipFillEllipse(gfx, b_white, brace_x - 2.4 * sc, brace_y - 1.8 * sc, 4.8 * sc, 3.6 * sc);

        let _ = GdipDrawLine(gfx, p_leg_rim, sh_x, sh_y, arm_x, arm_y);
        let _ = GdipDrawLine(gfx, p_leg_w, sh_x, sh_y, arm_x, arm_y);
        let _ = GdipFillEllipse(gfx, b_white, arm_x - 2.2 * sc, arm_y - 2.2 * sc, 4.4 * sc, 4.4 * sc);

        // Fluffy tapered hourglass back silhouette (NO turtle shell!)
        let back_poly = [
            POINTF { x: hip_x, y: cy0 - 25.5 * sc },
            POINTF { x: hip_x + 5.6 * sc, y: cy0 - 22.0 * sc },
            POINTF { x: hip_x + 7.2 * sc, y: cy0 - 19.5 * sc },
            POINTF { x: hip_x + 5.2 * sc, y: cy0 - 13.5 * sc },
            POINTF { x: hip_x + 7.8 * sc, y: cy0 - 7.8 * sc },
            POINTF { x: hip_x, y: cy0 - 5.6 * sc },
            POINTF { x: hip_x - 7.8 * sc, y: cy0 - 7.8 * sc },
            POINTF { x: hip_x - 5.2 * sc, y: cy0 - 13.5 * sc },
            POINTF { x: hip_x - 7.2 * sc, y: cy0 - 19.5 * sc },
            POINTF { x: hip_x - 5.6 * sc, y: cy0 - 22.0 * sc },
        ];
        let _ = GdipFillPolygon(gfx, b_body, back_poly.as_ptr(), back_poly.len() as i32, 0);
        let _ = GdipDrawPolygon(gfx, p_rim, back_poly.as_ptr(), back_poly.len() as i32);
        let _ = GdipFillEllipse(gfx, b_patch, hip_x - 6.6 * sc, cy0 - 14.5 * sc, 6.2 * sc, 6.6 * sc);
        let _ = GdipFillEllipse(gfx, b_patch, hip_x + 0.5 * sc, cy0 - 23.2 * sc, 5.2 * sc, 4.6 * sc);

        // Articulated plush hind thighs -> white feet
        for (side, lift, sx) in [(-1.0f32, l_lift, l_st), (1.0f32, r_lift, r_st)] {
            let hx0 = hip_x + side * 4.5 * sc;
            let hy0 = cy0 - 9.5 * sc;
            let kx = hip_x + (side * 6.0 + sx * 0.45) * sc;
            let ky = cy0 - (4.8 + lift * 0.65) * sc;
            let fx = hip_x + (side * 5.2 + sx) * sc;
            let fy = cy0 - (0.8 + lift) * sc + tiptoe;
            let _ = GdipDrawLine(gfx, p_thigh_rim, hx0, hy0, kx, ky);
            let _ = GdipDrawLine(gfx, p_thigh_in, hx0, hy0, kx, ky);
            let _ = GdipDrawLine(gfx, p_leg_rim, kx, ky, fx, fy);
            let _ = GdipDrawLine(gfx, p_leg_w, kx, ky, fx, fy);
        }

        // Back of fluffy head with cheek tufts
        let hx = hip_x + ap * 0.9 * sc;
        let hy = cy0 - 30.5 * sc;
        let el = [POINTF { x: hx - 5.2 * sc, y: hy }, POINTF { x: hx - 6.6 * sc, y: hy - 8.8 * sc }, POINTF { x: hx - 1.4 * sc, y: hy - 4.0 * sc }];
        let er = [POINTF { x: hx + 5.2 * sc, y: hy }, POINTF { x: hx + 6.6 * sc, y: hy - 8.8 * sc }, POINTF { x: hx + 1.4 * sc, y: hy - 4.0 * sc }];
        let _ = GdipFillPolygon(gfx, b_body, el.as_ptr(), 3, 0);
        let _ = GdipDrawPolygon(gfx, p_rim, el.as_ptr(), 3);
        let _ = GdipFillPolygon(gfx, b_patch, er.as_ptr(), 3, 0);
        let _ = GdipDrawPolygon(gfx, p_rim, er.as_ptr(), 3);
        let h_poly = [
            POINTF { x: hx - 6.0 * sc, y: hy - 2.5 * sc },
            POINTF { x: hx, y: hy - 5.5 * sc },
            POINTF { x: hx + 6.0 * sc, y: hy - 2.5 * sc },
            POINTF { x: hx + 8.6 * sc, y: hy + 0.5 * sc },
            POINTF { x: hx + 5.2 * sc, y: hy + 4.4 * sc },
            POINTF { x: hx, y: hy + 5.2 * sc },
            POINTF { x: hx - 5.2 * sc, y: hy + 4.4 * sc },
            POINTF { x: hx - 8.6 * sc, y: hy + 0.5 * sc },
        ];
        let _ = GdipFillPolygon(gfx, b_body, h_poly.as_ptr(), h_poly.len() as i32, 0);
        let _ = GdipDrawPolygon(gfx, p_rim, h_poly.as_ptr(), h_poly.len() as i32);
        let _ = GdipFillEllipse(gfx, b_patch, hx - 1.0 * sc, hy - 4.8 * sc, 5.4 * sc, 3.8 * sc);

        let tail_wag = (tf * 3.0).sin() * 6.0 - ((c.step_vel as f32) * 0.08 / scale);
        draw_plush_tail_gdip(
            gfx, p_tail_rim, p_tail_in, b_white,
            [
                (hip_x, cy0 - 8.0 * sc),
                (hip_x + tail_wag * 0.4 * sc, cy0 - 2.0 * sc),
                (hip_x + tail_wag * 0.95 * sc, cy0 - 5.0 * sc),
                (hip_x + tail_wag * 1.15 * sc, cy0 - 13.5 * sc),
            ],
            sc,
        );
    } else {
        // Side-view Fluffy Calico Cat
        let alt = (floor_y - by).max(0.0);
        let sh_sc = (1.0f32 - alt / (115.0 * sc)).clamp(0.32, 1.0);
        let _ = GdipFillEllipse(gfx, b_shad, bx - 16.0 * sc * sh_sc, floor_y - 2.0, 32.0 * sc * sh_sc, 5.0 * sc * sh_sc);

        let dir = c.dir as f32;
        let xf = |lx: f32, ly: f32| -> (f32, f32) { (bx + lx * dir * sc, by + ly * sc) };
        let c_mode = c.caught.as_ref().map(|r| r.mode);
        let toss_air = c_mode == Some(CatchPlayMode::HighToss)
            && c.caught.as_ref().map_or(false, |r| r.nodes_y[0] < (floor_y - 44.0 * scale) as f64);
        let slam_phase = if c_mode == Some(CatchPlayMode::PounceSlam) { (t * 1.15) % 1.0 } else { 0.99 };
        let is_walk = matches!(c.behavior, CatBehavior::Walk | CatBehavior::ChaseRun)
            || (c.behavior == CatBehavior::PlayCaught && matches!(c_mode, Some(CatchPlayMode::PawJuggle | CatchPlayMode::FloorRoll | CatchPlayMode::OrbitSpin)))
            || (c_mode == Some(CatchPlayMode::HighToss) && !toss_air);
        let is_ambush = matches!(c.behavior, CatBehavior::AmbushDrop | CatBehavior::AmbushCursor | CatBehavior::PrepJumpPc)
            || (c_mode == Some(CatchPlayMode::PounceSlam) && slam_phase < 0.58);
        let is_sit = c.behavior == CatBehavior::SitWatchUp
            || (c.behavior == CatBehavior::BellyBliss && c.rear_amt > 0.15)
            || (c.behavior == CatBehavior::PlayCaught && toss_air);
        let is_loaf = c.behavior == CatBehavior::Loaf;
        let is_groom = c.behavior == CatBehavior::Groom;
        let is_swat = c.behavior == CatBehavior::SwatCursor
            || (c.behavior == CatBehavior::PlayCaught && matches!(c_mode, Some(CatchPlayMode::PawJuggle | CatchPlayMode::FloorRoll | CatchPlayMode::OrbitSpin)))
            || (c_mode == Some(CatchPlayMode::PounceSlam) && slam_phase >= 0.58 && slam_phase < 0.88)
            || (c_mode == Some(CatchPlayMode::HighToss) && c.poke_flick > 0.25);
        let is_pounce = matches!(
            c.behavior,
            CatBehavior::SidePounce | CatBehavior::PounceCatch | CatBehavior::PounceCursor | CatBehavior::JumpToPc | CatBehavior::HopOffPc
        );
        let is_shake = c.behavior == CatBehavior::PlayCaught && c_mode == Some(CatchPlayMode::MouthShake);

        let wiggle = if is_ambush { (tf * 18.0).sin() * 0.7 } else { 0.0 };
        let bob = if is_walk { -(c.walk_cycle as f32).sin().abs() * 1.4 } else { 0.0 };
        let body_y = if is_loaf { -6.5 } else if is_ambush { -7.2 + wiggle } else if is_sit { -12.0 } else { -13.5 + bob };

        let t_rate = if is_sit || is_ambush || is_swat || is_shake { 6.4 } else { 2.8 };
        let w1 = (tf * t_rate).sin() * 5.2;
        draw_plush_tail_gdip(
            gfx, p_tail_rim, p_tail_in, b_white,
            [
                xf(-11.0, if is_sit { -4.0 } else { body_y }),
                xf(-18.0, (if is_sit { -8.0 } else { body_y - 6.0 }) + w1 * 0.4),
                xf(-21.0, body_y - 15.0),
                xf(-14.0 + w1 * 0.7, body_y - 20.0),
            ],
            sc,
        );

        if is_sit {
            // Far supporting front leg (drawn behind chest/body)
            let (ff0x, ff0y) = xf(7.4, -10.5);
            let (ff1x, ff1y) = xf(8.6, -1.0);
            let _ = GdipDrawLine(gfx, p_leg_rim, ff0x, ff0y, ff1x, ff1y);
            let _ = GdipDrawLine(gfx, p_leg_w, ff0x, ff0y, ff1x, ff1y);
            let (fpwx, fpwy) = xf(9.2, -1.0);
            draw_paw_tip(fpwx, fpwy, 2.3, 1.6);

            let (sx, sy) = xf(-2.0, -9.0);
            let _ = GdipFillEllipse(gfx, b_body, sx - 9.4 * sc, sy - 9.0 * sc, 18.8 * sc, 18.0 * sc);
            let _ = GdipDrawEllipse(gfx, p_rim, sx - 9.4 * sc, sy - 9.0 * sc, 18.8 * sc, 18.0 * sc);
            let (px, py) = xf(-5.0, -10.0);
            let _ = GdipFillEllipse(gfx, b_patch, px - 4.2 * sc, py - 3.2 * sc, 8.4 * sc, 6.4 * sc);
            let (wx, wy) = xf(5.0, -11.5);
            let _ = GdipFillEllipse(gfx, b_white, wx - 3.6 * sc, wy - 4.2 * sc, 7.2 * sc, 8.4 * sc);

            // Near front leg drawn cleanly in front of chest so both paws are unmistakable
            let (f0x, f0y) = xf(3.8, -10.0);
            let (f1x, f1y) = xf(5.2, -0.8);
            let _ = GdipDrawLine(gfx, p_leg_rim, f0x, f0y, f1x, f1y);
            let _ = GdipDrawLine(gfx, p_leg_w, f0x, f0y, f1x, f1y);
            let (npwx, npwy) = xf(5.8, -0.8);
            draw_paw_tip(npwx, npwy, 2.4, 1.6);

            let (hx, hy) = xf(4.5, -20.5);
            draw_fluffy_head_gdip(gfx, b_body, b_patch, b_white, b_ear, b_eye, b_dark, p_rim, p_eye, hx, hy, -0.45 * dir, dir, sc, false, true, 0.2, -0.7);
        } else {
            let ph = c.walk_cycle as f32;
            let sw = if is_walk { ph.sin() * 4.8 } else if is_pounce { -5.0 } else { 0.0 };
            let sw_far = if is_walk { -ph.sin() * 4.8 } else if is_pounce { -3.5 } else { 0.0 };
            let l_f = if is_walk { ph.cos().max(0.0) * 2.5 } else { 0.0 };
            let l_b = if is_walk { (-ph.cos()).max(0.0) * 2.5 } else { 0.0 };

            // 1) Far-side background legs (drawn before torso for true 2.5D shoulder/hip depth)
            if !is_loaf && !is_ambush {
                let (fh0x, fh0y) = xf(-7.0, body_y + 2.0);
                let (fhkx, fhky) = xf(-8.6 - sw_far * 0.4, -5.5 - l_f * 0.5);
                let (fhfx, fhfy) = xf(-6.8 - sw_far, -0.7 - l_f);
                let _ = GdipDrawLine(gfx, p_thigh_rim, fh0x, fh0y, fhkx, fhky);
                let _ = GdipDrawLine(gfx, p_thigh_in, fh0x, fh0y, fhkx, fhky);
                let _ = GdipDrawLine(gfx, p_leg_rim, fhkx, fhky, fhfx, fhfy);
                let _ = GdipDrawLine(gfx, p_leg_w, fhkx, fhky, fhfx, fhfy);
                draw_paw_tip(fhfx + 0.8 * dir * sc, fhfy, 2.2, 1.5);

                // Far front leg stays firmly grounded while near paw swats cursor or grooms!
                let (ff0x, ff0y) = xf(9.6, body_y + 1.5);
                let (ff1x, ff1y) = if is_swat || is_groom {
                    xf(10.6, -0.7)
                } else if is_pounce {
                    xf(15.6, body_y - 2.8)
                } else {
                    xf(9.8 - sw, -0.7 - l_b)
                };
                let _ = GdipDrawLine(gfx, p_leg_rim, ff0x, ff0y, ff1x, ff1y);
                let _ = GdipDrawLine(gfx, p_leg_w, ff0x, ff0y, ff1x, ff1y);
                draw_paw_tip(ff1x + 0.9 * dir * sc, ff1y, 2.3, 1.6);
            }

            // 2) Fluffy Calico torso, orange saddle, and white chest
            let (tx1, ty1) = xf(-1.0, body_y);
            let bh = if is_loaf || is_ambush { 6.2 } else { 5.4 };
            let _ = GdipFillEllipse(gfx, b_body, tx1 - 12.6 * sc, ty1 - bh * sc, 25.2 * sc, bh * 2.0 * sc);
            let _ = GdipDrawEllipse(gfx, p_rim, tx1 - 12.6 * sc, ty1 - bh * sc, 25.2 * sc, bh * 2.0 * sc);
            let (px1, py1) = xf(-2.5, body_y - 1.2);
            let _ = GdipFillEllipse(gfx, b_patch, px1 - 4.8 * sc, py1 - 3.0 * sc, 9.6 * sc, 6.0 * sc);
            let (cx1, cy1) = xf(9.2, body_y + 0.6);
            let _ = GdipFillEllipse(gfx, b_white, cx1 - 3.8 * sc, cy1 - 3.4 * sc, 7.6 * sc, 6.8 * sc);

            // 3) Near-side foreground legs/paws (drawn AFTER torso & chest so limbs visibly connect and stand/swat!)
            if !is_loaf && !is_ambush {
                let (h0x, h0y) = xf(-9.4, body_y + 1.8);
                let (hkx, hky) = xf(-11.2 - sw * 0.4, -5.5 - l_b * 0.5);
                let (hfx, hfy) = xf(-9.4 - sw, -0.5 - l_b);
                let _ = GdipDrawLine(gfx, p_thigh_rim, h0x, h0y, hkx, hky);
                let _ = GdipDrawLine(gfx, p_thigh_in, h0x, h0y, hkx, hky);
                let _ = GdipDrawLine(gfx, p_leg_rim, hkx, hky, hfx, hfy);
                let _ = GdipDrawLine(gfx, p_leg_w, hkx, hky, hfx, hfy);
                draw_paw_tip(hfx + 0.8 * dir * sc, hfy, 2.3, 1.5);

                let (f0x, f0y) = xf(6.8, body_y + 1.8);
                let (f1x, f1y) = if is_swat {
                    xf(16.2, body_y - (if c_mode == Some(CatchPlayMode::PawJuggle) { 8.0 } else { 2.2 }) - (c.paw_extend as f32) * 5.2)
                } else if is_groom {
                    xf(14.4, body_y - 3.2 + (tf * 8.0).sin() * 1.6)
                } else if is_pounce {
                    xf(16.6, body_y - 5.2)
                } else {
                    xf(7.0 + sw, -0.5 - l_f)
                };
                let _ = GdipDrawLine(gfx, p_leg_rim, f0x, f0y, f1x, f1y);
                let _ = GdipDrawLine(gfx, p_leg_w, f0x, f0y, f1x, f1y);
                draw_paw_tip(f1x + (if is_swat || is_pounce { 0.4 } else { 0.9 }) * dir * sc, f1y, 2.4, 1.7);
            } else {
                // Both tucked/crouched front paws clearly visible in front of chest in Loaf & Ambush
                let (lp0x, lp0y) = if is_ambush { xf(12.2, -1.7) } else { xf(10.6, -1.6) };
                let (lp1x, lp1y) = if is_ambush { xf(7.4, -1.4) } else { xf(6.6, -1.5) };
                draw_paw_tip(lp0x, lp0y, 2.8, 1.7);
                draw_paw_tip(lp1x, lp1y, 2.9, 1.8);
            }

            let hx_l = (if is_ambush { 11.0 } else { 9.8 }) + if is_shake { (tf * 22.0).sin() * 3.0 } else { 0.0 };
            let hy_l = body_y - (if is_ambush { 3.2 } else { 6.0 }) + if is_shake { (tf * 11.0).cos() * 1.8 } else { 0.0 };
            let (hx, hy) = xf(hx_l, hy_l);
            let tilt = if is_shake {
                (tf * 22.0).sin() * 0.35
            } else if matches!(c.behavior, CatBehavior::AmbushDrop | CatBehavior::PrepJumpPc) {
                -0.35
            } else if c.behavior == CatBehavior::JumpToPc {
                -0.28
            } else if c.behavior == CatBehavior::HopOffPc {
                0.22
            } else if is_pounce {
                -0.2
            } else {
                0.0
            };
            let look_y = if matches!(c.behavior, CatBehavior::AmbushDrop | CatBehavior::PrepJumpPc | CatBehavior::JumpToPc) {
                -0.7
            } else if c.behavior == CatBehavior::HopOffPc {
                0.55
            } else {
                0.0
            };
            draw_fluffy_head_gdip(
                gfx, b_body, b_patch, b_white, b_ear, b_eye, b_dark, p_rim, p_eye,
                hx, hy, tilt * dir, dir, sc, c.blink_t < 0.0 || is_groom, is_ambush || is_pounce || is_swat || is_shake,
                0.35, look_y,
            );
        }
    }

    for ptr in [b_body, b_patch, b_white, b_ear, b_eye, b_dark, b_shad] {
        if !ptr.is_null() { let _ = GdipDeleteBrush(ptr); }
    }
    for pen in [p_rim, p_tail_rim, p_tail_in, p_thigh_rim, p_thigh_in, p_leg_rim, p_leg_w, p_eye] {
        if !pen.is_null() { let _ = GdipDeletePen(pen); }
    }
    let _ = GdipDeleteGraphics(gfx);

    // Draw sleep "Zzz" above cat on PC tower
    if c.on_pc && c.behavior == CatBehavior::SleepPc && c.belly_roll < 0.25 {
        let z_off = ((t * 18.0) % 14.0) as i32;
        let f_z = make_mono_font(st, 9, FW_BOLD);
        draw_text(hdc, "Zzz", RECT { left: (bx as i32) - st.s(14), top: (by as i32) - st.s(20) - z_off, right: (bx as i32) + st.s(14), bottom: (by as i32) - z_off }, f_z, rgb(204, 255, 84), DT_LEFT | DT_SINGLELINE);
        DeleteObject(f_z as HGDIOBJ);
    }

    // Draw Caught Katakana Head + its 5 Trailing Square Cubes!
    if let Some(ref cd) = c.caught {
        let rain_cell = st.s(8).max(6);
        for i in (1..6).rev() {
            let k = 1.0 - (i as f64) / 6.0;
            let sz = (rain_cell - i as i32).max(st.s(3));
            let col = drop_rgb(k * 0.9, false, cd.is_red);
            let nx = cd.nodes_x[i].round() as i32;
            let ny = cd.nodes_y[i].round() as i32;
            fill_rect(hdc, RECT { left: nx - sz / 2, top: ny - sz / 2, right: nx + sz / 2, bottom: ny + sz / 2 }, col);
        }
        let glyph = &KATA_5X7[cd.glyph_idx.min(KATA_5X7.len() - 1)];
        let px_s = (rain_cell / 5).max(1);
        let hx = cd.nodes_x[0].round() as i32 - (5 * px_s) / 2;
        let hy = cd.nodes_y[0].round() as i32 - (7 * px_s) / 2;
        let hcol = if cd.is_red { rgb(255, 104, 112) } else { rgb(204, 255, 84) };
        for (r, &bits) in glyph.iter().enumerate() {
            for col_i in 0..5 {
                if (bits >> (4 - col_i)) & 1 != 0 {
                    fill_rect(hdc, RECT { left: hx + col_i * px_s, top: hy + (r as i32) * px_s, right: hx + (col_i + 1) * px_s, bottom: hy + ((r as i32) + 1) * px_s }, hcol);
                }
            }
        }
    }

    // Floating hearts from poking cat
    if !c.hearts.is_empty() {
        let f_h = make_mono_font(st, 11, FW_BOLD);
        for hr in &c.hearts {
            let col = match hr.tint_idx % 3 {
                0 => rgb(255, 94, 138),
                1 => rgb(241, 95, 44),
                _ => rgb(204, 255, 84),
            };
            let hx = hr.x.round() as i32;
            let hy = hr.y.round() as i32;
            draw_text(hdc, "♥", RECT { left: hx - st.s(6), top: hy - st.s(6), right: hx + st.s(12), bottom: hy + st.s(12) }, f_h, col, DT_LEFT | DT_SINGLELINE);
        }
        DeleteObject(f_h as HGDIOBJ);
    }
}

/// Draw monospace glyphs with per-character colors & Variant 5 ("Вихрь Лапки") paw scatter!
unsafe fn draw_shatterable_glyphs(
    hdc: HDC,
    st: &mut AppState,
    row_id: usize,
    x0: i32,
    y0: i32,
    line_h: i32,
    glyphs: &[(char, u32)],
    font: HFONT,
    max_right: i32,
) {
    let char_w = text_width(hdc, font, "M").max(5);
    let paw_x = st.cat.paw_tip_x;
    let paw_y = st.cat.paw_tip_y;
    let paw_on = st.cat.paw_touching;
    let rad = st.s(32) as f64;
    let scale = (st.dpi as f64) / 96.0;

    let mut cur_x = x0;
    for (ci, &(ch, col)) in glyphs.iter().enumerate() {
        if cur_x + char_w > max_right {
            break;
        }
        if ch != ' ' {
            let cx = (cur_x + char_w / 2) as f64;
            let cy = (y0 + line_h / 2) as f64;
            let dist = (cx - paw_x).hypot(cy - paw_y);

            let already = st.cat.shattered.iter().position(|s| s.row == row_id && s.col == ci);
            if paw_on && dist < rad && already.is_none() {
                st.cat.shattered.push(ShatterSlot { row: row_id, col: ci, life: 0.90 });
                let vx_dir = st.cat.dir;
                for s in 0..2 {
                    let r1 = st.cat.next_rand() - 0.5;
                    let r2 = st.cat.next_rand() - 0.5;
                    let tx = cur_x as f64 + (s as f64) * 3.0 * scale;
                    let ty = y0 as f64 + 3.0 * scale;
                    st.cat.shards.push(LogShard {
                        x: tx,
                        y: ty,
                        tx,
                        ty,
                        vx: vx_dir * 35.0 * scale + r1 * 65.0 * scale,
                        vy: (-35.0 - r2 * 45.0) * scale,
                        life: 0.90,
                        max_life: 0.90,
                        is_bright: s == 0,
                    });
                }
            }

            let is_hidden = st.cat.shattered.iter().any(|s| s.row == row_id && s.col == ci && s.life > 0.18);
            if !is_hidden {
                let mut buf = [0u8; 4];
                let ch_str = ch.encode_utf8(&mut buf);
                draw_text(
                    hdc,
                    ch_str,
                    RECT { left: cur_x, top: y0, right: cur_x + char_w + 2, bottom: y0 + line_h },
                    font,
                    col,
                    DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
                );
            }
        }
        cur_x += char_w;
    }
}

unsafe fn draw_shatterable_line(
    hdc: HDC,
    st: &mut AppState,
    row_id: usize,
    x0: i32,
    y0: i32,
    line_h: i32,
    text: &str,
    font: HFONT,
    col: u32,
    max_right: i32,
) {
    let glyphs: Vec<(char, u32)> = text.chars().map(|ch| (ch, col)).collect();
    draw_shatterable_glyphs(hdc, st, row_id, x0, y0, line_h, &glyphs, font, max_right);
}

unsafe fn paint_sidebar(hdc: HDC, st: &mut AppState, sb_w: i32, ch: i32) {
    let sb_rect = RECT { left: 0, top: st.s(36), right: sb_w, bottom: ch };
    fill_rect(hdc, sb_rect, COL_SIDEBAR);

    let now_ms = win32::tick_count();
    let t = (now_ms.saturating_sub(st.anim_start_ms) as f64) / 1000.0;

    // 8x8 Creeper face (#8 Bio-Spark animated)
    let csize = (st.s(72) / 8) * 8;
    let creeper_x = (sb_w - csize) / 2;
    let creeper_y = st.s(48);
    let creeper_pad = RECT {
        left: creeper_x - st.s(4),
        top: creeper_y - st.s(4),
        right: creeper_x + csize + st.s(4),
        bottom: creeper_y + csize + st.s(4),
    };

    let stage_top = st.s(134);
    let stage_rc = RECT {
        left: st.s(22),
        top: stage_top,
        right: sb_w - st.s(22),
        bottom: stage_top + st.s(64),
    };
    let log_top = st.s(210);

    // 1) Pure code-driven mirrored Katakana + square trail Matrix rain with cat-reactive pendulum sway
    draw_matrix_rain(hdc, st, sb_w, ch, creeper_pad, stage_rc, log_top, t);

    // 2) Stage Block with hard bottom-to-top Anvil slam -> returns shock 0..1 to drive Creeper (#01 / #09)
    let stage_shock = draw_stage_block(hdc, st, sb_w, stage_rc, now_ms, t);
    draw_creeper(hdc, st, creeper_x, creeper_y, csize, t, stage_shock, st.stage_fx_variant);

    if stage_shock > 0.14 {
        let sp_count = if st.stage_fx_variant == 9 { 16 } else { 8 };
        let prog = 1.0 - stage_shock;
        let tick = (t * 25.0) as i32;
        for i in 0..sp_count {
            let side = if i % 2 == 0 { -1 } else { 1 };
            let (sx, sy) = if st.stage_fx_variant == 9 {
                let rx = creeper_x + (i % 4) * st.s(18)
                    + ((hash2_u32(i, 7) - 0.5) * (st.s(28) as f64)).round() as i32
                    - if side < 0 { st.s(18) } else { -st.s(10) };
                let ry = stage_rc.top - (prog * ((st.s(60) as f64) + hash2_u32(i, 3) * (st.s(45) as f64))).round() as i32;
                (rx, ry)
            } else {
                let rx = sb_w / 2 + side * (st.s(55) + (prog * (st.s(56) as f64)).round() as i32 + i * st.s(2));
                let ry = stage_rc.top + st.s(20) - (prog * (st.s(36) as f64)).round() as i32
                    + ((hash2_u32(i, tick) - 0.5) * (st.s(26) as f64)).round() as i32;
                (rx, ry)
            };
            let sp_col = if i % 3 == 0 { rgb(255, 255, 255) } else { rgb(255, 184, 48) };
            fill_rect(hdc, RECT { left: sx, top: sy, right: sx + st.s(3).max(2), bottom: sy + st.s(2).max(2) }, sp_col);
        }
    }

    // 3) Classic opaque framed CRT log panel (cat can still bat & scatter lines!)
    paint_crt_panel(hdc, st, sb_w, ch, log_top, t);

    // 4) Swirling log pixel shards + bottom-right PC corner + Fluffy Calico Cat + Caught 5-cube ribbon!
    let sh_sz = st.s(2).max(2);
    for sh in &st.cat.shards {
        let col = if sh.is_bright { rgb(204, 255, 84) } else { COL_CRT_PH };
        let sx = sh.x.round() as i32;
        let sy = sh.y.round() as i32;
        fill_rect(hdc, RECT { left: sx, top: sy, right: sx + sh_sz, bottom: sy + sh_sz }, col);
    }
    fill_rect(hdc, RECT { left: st.s(22), top: ch - st.s(9), right: sb_w - st.s(22), bottom: ch - st.s(8) }, rgb(36, 56, 26));
    draw_pc_corner(hdc, st, sb_w, ch, t);
    draw_calico_cat_gdip(hdc, st, ch, t);

    if st.cat.boosty_banner_t > 0.0 {
        let bx = st.s(22);
        let by = ch - st.s(86);
        let bw = sb_w - st.s(44);
        let bh = st.s(22);
        draw_square_box(hdc, RECT { left: bx, top: by, right: bx + bw, bottom: by + bh }, rgb(241, 95, 44), COL_TEXT);
        let f_egg = make_font(st, 11, FW_BOLD, false);
        draw_text(hdc, "♥ BOOSTY: boosty.to/defrag!", RECT { left: bx, top: by, right: bx + bw, bottom: by + bh }, f_egg, COL_TEXT, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        DeleteObject(f_egg as HGDIOBJ);
    }
}

unsafe fn paint_crt_panel(hdc: HDC, st: &mut AppState, sb_w: i32, ch: i32, log_top: i32, t: f64) {
    let crt = RECT { left: st.s(22), top: log_top, right: sb_w - st.s(22), bottom: ch - st.s(62) };

    // Classic opaque framed CRT box (prevents Matrix rain bleeding through log text)
    draw_square_box(hdc, crt, COL_CRT_BG, COL_CRT_BORDER);

    let f_tab = make_mono_font(st, 11, FW_BOLD);
    let f_log = make_mono_font(st, 10, FW_NORMAL);

    // Framed Tabs: tall narrow vertical bars flush full-height before BOTH [▏Система] | [▏00:28] (no bottom underline)
    let tab_h = st.s(24);
    fill_rect(hdc, RECT { left: crt.left, top: crt.top + tab_h, right: crt.right, bottom: crt.top + tab_h + 1 }, rgb(22, 36, 26));
    let mid_x = (crt.left + crt.right) / 2;
    let sys_rc = RECT { left: crt.left, top: crt.top, right: mid_x, bottom: crt.top + tab_h };
    let log_rc = RECT { left: mid_x, top: crt.top, right: crt.right, bottom: crt.top + tab_h };
    let accent_w = st.s(4).max(3);
    let elapsed_txt = format_elapsed(st);

    for (active, trc, label) in [
        (st.crt_tab == 0, sys_rc, "Система"),
        (st.crt_tab == 1, log_rc, elapsed_txt.as_str()),
    ] {
        let bar_rc = RECT {
            left: trc.left,
            top: trc.top,
            right: trc.left + accent_w,
            bottom: trc.bottom,
        };
        if active {
            fill_rect(hdc, bar_rc, rgb(60, 230, 80));
        } else {
            fill_rect(hdc, bar_rc, rgb(28, 51, 34));
        }
        draw_text(
            hdc,
            label,
            RECT { left: bar_rc.right + st.s(7), top: trc.top, right: trc.right - st.s(4), bottom: trc.bottom },
            f_tab,
            if active { COL_CRT_PH } else { COL_CRT_DIM },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }

    st.hits.push(HitRect { id: HitId::TabCrtSys, rect: sys_rc });
    st.hits.push(HitRect { id: HitId::TabCrtLog, rect: log_rc });

    let cx0 = crt.left + st.s(8);
    let cx1 = crt.right - st.s(8);
    let mut ly = crt.top + tab_h + st.s(8);

    if st.crt_tab == 0 {
        let gpu_text = if st.sys.gpu_names.is_empty() {
            "NVIDIA GPU не найдена".to_string()
        } else {
            st.sys.gpu_names.join(", ")
        };
        let cur_drv = match (
            &st.sys.openfranken_installed_version,
            &st.sys.current_driver_version,
        ) {
            (Some(ofv), _) => format!("OpenFranken {ofv}"),
            (None, Some(dv)) => dv.clone(),
            (None, None) => "Не установлен".to_string(),
        };
        let est_gb = (st.estimated_install_mb() as f64) / 1024.0;
        let lh = st.s(15);

        draw_shatterable_line(hdc, st, 100, cx0, ly, lh, "> GPU:", f_log, COL_CRT_DIM, cx1);
        ly += st.s(16);
        draw_shatterable_line(hdc, st, 101, cx0, ly, st.s(18), &gpu_text, f_log, COL_CRT_PH, cx1);
        ly += st.s(36);

        draw_shatterable_line(hdc, st, 102, cx0, ly, lh, "> DRIVER:", f_log, COL_CRT_DIM, cx1);
        ly += st.s(16);
        draw_shatterable_line(hdc, st, 103, cx0, ly, st.s(18), &cur_drv, f_log, COL_CRT_PH, cx1);
        ly += st.s(26);

        draw_shatterable_line(hdc, st, 104, cx0, ly, lh, "> DISK SPACE:", f_log, COL_CRT_DIM, cx1);
        ly += st.s(16);
        draw_shatterable_line(hdc, st, 105, cx0, ly, st.s(18), &format!("~{est_gb:.1} ГБ"), f_log, COL_CRT_PH, cx1);

        let prompt = if st.crt_blink_on { ">_" } else { ">" };
        draw_shatterable_line(hdc, st, 106, cx0, crt.bottom - st.s(22), st.s(16), prompt, f_tab, COL_GREEN, cx1);
    } else {
        let active_phase = st.current_crt_phase();
        let line_h = st.s(13).max(11);

        // Variant #11: Live Component Checklist at top, synced 1:1 with log token dimming below
        if !st.started_in_uninstall && !matches!(st.screen, Screen::UninstallConfirm | Screen::Uninstalling | Screen::UninstallDone) {
            let mut items: Vec<(u8, String)> = vec![
                (0, "Подготовка / Сертификат".to_string()),
                (1, format!("Display.Driver {}", st.meta.driver_version)),
            ];
            if st.opts.install_hdaudio {
                items.push((2, format!("HD Audio {}", st.meta.hdaudio_version)));
            }
            if st.opts.install_physx || st.opts.install_nvpi || st.opts.install_vcpp {
                items.push((3, "PhysX + VC++ / ARP".to_string()));
            }
            if st.opts.install_nvcpl {
                items.push((4, "NVCPL Win32 (installClose)".to_string()));
            }

            for (idx, (p_id, label)) in items.iter().enumerate() {
                let is_active = *p_id == active_phase && matches!(st.screen, Screen::Installing);
                if is_active {
                    fill_rect(hdc, RECT { left: crt.left + st.s(2), top: ly - st.s(1), right: crt.right - st.s(2), bottom: ly + line_h - st.s(1) }, rgb(13, 37, 20));
                }
                let (mark, col) = if *p_id < active_phase {
                    ("✓", rgb(56, 124, 72))
                } else if is_active {
                    (" ", rgb(66, 255, 122))
                } else {
                    (" ", rgb(37, 69, 46))
                };
                let row_txt = format!("[{mark}] {label}");
                draw_shatterable_line(hdc, st, 210 + idx, cx0, ly, line_h, &row_txt, f_log, col, cx1);
                if is_active && !st.cat.shattered.iter().any(|s| s.row == 210 + idx && s.col == 1) {
                    let char_w = text_width(hdc, f_log, "M").max(5);
                    let sq = st.s(4).max(3);
                    let ix = cx0 + char_w + (char_w - sq) / 2;
                    let iy = ly + (line_h - sq) / 2;
                    let u = (t * 2.2).sin() * 0.5 + 0.5;
                    let sr = (53.0 + (110.0 - 53.0) * u).round() as u8;
                    let sg = (184.0 + (255.0 - 184.0) * u).round() as u8;
                    let sb = (86.0 + (142.0 - 86.0) * u).round() as u8;
                    fill_rect(hdc, RECT { left: ix, top: iy, right: ix + sq, bottom: iy + sq }, rgb(sr, sg, sb));
                }
                ly += line_h;
            }
            ly += st.s(2);
            fill_rect(hdc, RECT { left: crt.left + st.s(6), top: ly, right: crt.right - st.s(6), bottom: ly + 1 }, rgb(24, 43, 28));
            ly += st.s(6);
        } else {
            let trace_hdr = format!("CALL OPT::NV{} TRACE:ACTIVE", st.meta.driver_version);
            draw_shatterable_line(hdc, st, 200, cx0, ly, line_h, &trace_hdr, f_tab, COL_GREEN, cx1);
            ly += line_h + st.s(4);
        }

        let avail_h = (crt.bottom - st.s(4) - ly).max(line_h);
        let max_lines = (avail_h / line_h).max(1) as usize;
        let max_w = (cx1 - cx0).max(40);
        let cursor_w = text_width(hdc, f_log, "_");

        let total_chars = st.crt_stream.chars().count();
        let revealed = st.crt_revealed_chars.min(total_chars);
        let is_typing = revealed < total_chars;
        let show_cursor = is_typing || st.crt_blink_on;

        if revealed == 0 {
            let prompt = if st.crt_blink_on { ">_" } else { ">" };
            draw_shatterable_line(hdc, st, 201, cx0, ly, line_h, prompt, f_log, COL_GREEN, cx1);
        } else {
            let end_byte = st
                .crt_stream
                .char_indices()
                .nth(revealed)
                .map(|(idx, _)| idx)
                .unwrap_or(st.crt_stream.len());
            let stream = &st.crt_stream[..end_byte];
            let mut wrapped: Vec<Vec<(char, u8)>> = Vec::new();
            let mut cur: Vec<(char, u8)> = Vec::new();
            let mut cur_str = String::new();
            for (ci, ch) in stream.chars().enumerate() {
                let ph = st.crt_char_phases.get(ci).copied().unwrap_or(active_phase);
                let mut cand = cur_str.clone();
                cand.push(ch);
                if text_width(hdc, f_log, &cand) > max_w.saturating_sub(cursor_w) && !cur.is_empty() {
                    wrapped.push(cur);
                    cur = Vec::new();
                    cur_str = String::new();
                    if ch != ' ' {
                        cur.push((ch, ph));
                        cur_str.push(ch);
                    }
                } else {
                    if !(cur.is_empty() && ch == ' ') {
                        cur.push((ch, ph));
                        cur_str.push(ch);
                    }
                }
            }
            if !cur.is_empty() || wrapped.is_empty() {
                wrapped.push(cur);
            }

            let total = wrapped.len();
            let start_idx = total.saturating_sub(max_lines);
            let slice = &wrapped[start_idx..];
            let f_log_bold = make_mono_font(st, 10, FW_BOLD);
            for (idx, row_chars) in slice.iter().enumerate() {
                let is_last = idx + 1 == slice.len();
                let mut row_glyphs: Vec<(char, u32)> = row_chars
                    .iter()
                    .map(|&(ch, ph)| {
                        let col = if ph < active_phase {
                            // Completed component above is checked [✓] -> dim ONLY its log tokens!
                            rgb(53, 111, 66)
                        } else if is_last {
                            rgb(168, 255, 146)
                        } else {
                            COL_CRT_PH
                        };
                        (ch, col)
                    })
                    .collect();
                if is_last && show_cursor {
                    row_glyphs.push(('_', rgb(168, 255, 146)));
                }
                let font = if is_last { f_log_bold } else { f_log };
                let pc_safe_right = if ly + line_h > ch - st.s(86) {
                    (sb_w - st.s(132)).min(cx1).max(cx0 + st.s(40))
                } else {
                    cx1
                };
                draw_shatterable_glyphs(hdc, st, 300 + idx, cx0, ly, line_h, &row_glyphs, font, pc_safe_right);
                ly += line_h;
            }
            DeleteObject(f_log_bold as HGDIOBJ);
        }
    }

    for f in [f_tab, f_log] {
        DeleteObject(f as HGDIOBJ);
    }
}

unsafe fn paint_content(hdc: HDC, st: &mut AppState, x0: i32, cw: i32, ch: i32) {
    match st.screen {
        Screen::Components => paint_screen_components(hdc, st, x0, cw, ch),
        Screen::Installing => paint_screen_installing(hdc, st, x0, cw, ch),
        Screen::Done => paint_screen_done(hdc, st, x0, cw, ch),
        Screen::UninstallConfirm => paint_screen_uninstall_confirm(hdc, st, x0, cw, ch),
        Screen::Uninstalling => paint_screen_uninstalling(hdc, st, x0, cw, ch),
        Screen::UninstallDone => paint_screen_uninstall_done(hdc, st, x0, cw, ch),
    }
}

unsafe fn draw_checkbox_row(
    hdc: HDC,
    st: &mut AppState,
    rect: RECT,
    id: HitId,
    checked: bool,
    locked: bool,
    amber: bool,
    title: &str,
    sub: &str,
    f_title: HFONT,
    f_sub: HFONT,
) {
    let hovered = !locked && st.hover == id;
    let fill = if hovered { COL_CARD_HOVER } else { COL_CARD };
    let border = if amber && checked { COL_AMBER } else if hovered { COL_GREEN_DARK } else { COL_BORDER };
    draw_square_box(hdc, rect, fill, border);

    let bs = st.s(18);
    let bx = rect.left + st.s(14);
    let by = rect.top + (rect.h() - bs) / 2;
    let box_rc = RECT { left: bx, top: by, right: bx + bs, bottom: by + bs };

    let box_fill = if checked {
        if locked { COL_GREEN_DARK } else if amber { COL_AMBER } else { COL_GREEN }
    } else {
        COL_BG
    };
    let box_border = if checked { box_fill } else { COL_MUTED };
    draw_square_box(hdc, box_rc, box_fill, box_border);

    if checked {
        let pen = CreatePen(PS_SOLID, st.s(2).max(2), COL_DARK_TXT);
        let op = SelectObject(hdc, pen as HGDIOBJ);
        MoveToEx(hdc, bx + st.s(4), by + st.s(9), std::ptr::null_mut());
        LineTo(hdc, bx + st.s(8), by + st.s(13));
        LineTo(hdc, bx + st.s(14), by + st.s(5));
        SelectObject(hdc, op);
        DeleteObject(pen as HGDIOBJ);
    }

    let tx = bx + bs + st.s(14);
    draw_text(
        hdc,
        title,
        RECT { left: tx, top: rect.top + st.s(6), right: rect.right - st.s(12), bottom: rect.top + st.s(24) },
        f_title,
        if amber && checked { COL_AMBER } else { COL_TEXT },
        DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    draw_text(
        hdc,
        sub,
        RECT { left: tx, top: rect.top + st.s(24), right: rect.right - st.s(12), bottom: rect.bottom - st.s(5) },
        f_sub,
        COL_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    if !locked {
        st.hits.push(HitRect { id, rect });
    }
}

unsafe fn draw_button(hdc: HDC, st: &mut AppState, rect: RECT, id: HitId, label: &str, primary: bool, danger: bool, f_btn: HFONT) {
    let hovered = st.hover == id;
    let (fill, border, txt_col) = if danger {
        (if hovered { COL_RED_HOVER } else { COL_RED }, COL_RED, COL_TEXT)
    } else if primary {
        (if hovered { COL_GREEN_HOVER } else { COL_GREEN }, COL_GREEN, COL_DARK_TXT)
    } else {
        (if hovered { COL_CARD_HOVER } else { COL_CARD }, COL_BORDER, COL_TEXT)
    };
    draw_square_box(hdc, rect, fill, border);
    draw_text(hdc, label, rect, f_btn, txt_col, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    st.hits.push(HitRect { id, rect });
}

unsafe fn paint_screen_components(hdc: HDC, st: &mut AppState, x0: i32, cw: i32, ch: i32) {
    let left = x0;
    let right = cw - st.s(22);

    let f_h1 = make_font(st, 22, FW_BOLD, false);
    let f_sub = make_font(st, 13, FW_NORMAL, false);
    let f_row_t = make_font(st, 13, FW_SEMIBOLD, false);
    let f_row_s = make_font(st, 11, FW_NORMAL, false);
    let f_btn = make_font(st, 13, FW_BOLD, false);

    draw_text(hdc, "Выбор компонентов", RECT { left, top: st.s(48), right, bottom: st.s(76) }, f_h1, COL_TEXT, DT_LEFT | DT_SINGLELINE);
    draw_text(
        hdc,
        "Нативная быстрая установка без NVI2, телеметрии и NVIDIA App",
        RECT { left, top: st.s(78), right, bottom: st.s(98) },
        f_sub,
        COL_MUTED,
        DT_LEFT | DT_SINGLELINE,
    );

    let mut y = st.s(104);
    let rh = st.s(46);
    let gap = st.s(5);

    draw_checkbox_row(
        hdc, st, RECT { left, top: y, right, bottom: y + rh }, HitId::None, true, true, false,
        "Видеодрайвер NVIDIA (Display.Driver)",
        "Обязательный компонент: ядро, CUDA, Vulkan, DLSS, мутанты и Error 43",
        f_row_t, f_row_s,
    );
    y += rh + gap;

    draw_checkbox_row(
        hdc, st, RECT { left, top: y, right, bottom: y + rh }, HitId::ChkNvcpl, st.opts.install_nvcpl, false, false,
        "Классическая Панель управления (nvcplui.exe)",
        "Автономная Win32-панель без Microsoft Store + ярлыки и контекстное меню",
        f_row_t, f_row_s,
    );
    y += rh + gap;

    draw_checkbox_row(
        hdc, st, RECT { left, top: y, right, bottom: y + rh }, HitId::ChkHdaudio, st.opts.install_hdaudio, false, false,
        &format!("Аудиодрайвер NVIDIA HD Audio ({})", st.meta.hdaudio_version),
        "Вывод звука через HDMI и DisplayPort (~2.5 МБ)",
        f_row_t, f_row_s,
    );
    y += rh + gap;

    draw_checkbox_row(
        hdc, st, RECT { left, top: y, right, bottom: y + rh }, HitId::ChkPhysx, st.opts.install_physx, false, false,
        &format!("NVIDIA PhysX ({})", st.meta.physx_version),
        "Игровой физический движок (32+64 реестр, снос старого MSI)",
        f_row_t, f_row_s,
    );
    y += rh + gap;

    let vc_sub = if st.sys.vc_x64_ok && st.sys.vc_x86_ok {
        "Уже установлен в системе (пропуск при установке)"
    } else {
        "Установит отсутствующие библиотеки VC++ x64 / x86 тихо"
    };
    draw_checkbox_row(
        hdc, st, RECT { left, top: y, right, bottom: y + rh }, HitId::ChkVcpp, st.opts.install_vcpp, false, false,
        "Visual C++ Runtime 2015–2022",
        vc_sub,
        f_row_t, f_row_s,
    );
    y += rh + gap;

    draw_checkbox_row(
        hdc, st, RECT { left, top: y, right, bottom: y + rh }, HitId::ChkNvpi, st.opts.install_nvpi, false, false,
        &format!("NVIDIA Profile Inspector (v{})", st.meta.nvpi_version),
        "Продвинутая настройка профилей игр и скрытых флагов (ярлык в Пуск -> NVIDIA)",
        f_row_t, f_row_s,
    );
    y += rh + st.s(13);

    fill_rect(hdc, RECT { left, top: y - st.s(7), right, bottom: y - st.s(6) }, COL_BORDER);

    draw_checkbox_row(
        hdc, st, RECT { left, top: y, right, bottom: y + rh }, HitId::ChkClean, st.opts.clean_install, false, true,
        "Чистая установка",
        "Полностью удалить прежний драйвер NVIDIA (в т.ч. официальный) и кэши",
        f_row_t, f_row_s,
    );

    let btn_y1 = ch - st.s(54);
    let btn_y2 = ch - st.s(18);
    let bw_cancel = btn_width(hdc, f_btn, "Отмена", 108, st);
    let bw_inst = btn_width(hdc, f_btn, "Установить", 146, st);
    draw_button(hdc, st, RECT { left: right - bw_inst, top: btn_y1, right, bottom: btn_y2 }, HitId::BtnInstall, "Установить", true, false, f_btn);
    draw_button(hdc, st, RECT { left: right - bw_inst - st.s(10) - bw_cancel, top: btn_y1, right: right - bw_inst - st.s(10), bottom: btn_y2 }, HitId::BtnCancel, "Отмена", false, false, f_btn);

    for f in [f_h1, f_sub, f_row_t, f_row_s, f_btn] {
        DeleteObject(f as HGDIOBJ);
    }
}

unsafe fn paint_screen_installing(hdc: HDC, st: &mut AppState, x0: i32, cw: i32, ch: i32) {
    let left = x0;
    let right = cw - st.s(22);

    let f_gtitle = make_font(st, 20, FW_BOLD, false);
    let f_quote = make_font(st, 13, FW_NORMAL, true);
    let f_author = make_font(st, 12, FW_NORMAL, false);
    let f_stat = make_font(st, 13, FW_SEMIBOLD, false);

    // Full-bleed borderless showcase artwork + bottom dark fade into COL_BG (#16191D)
    let img_box = RECT { left, top: st.s(46), right, bottom: ch - st.s(86) };
    fill_rect(hdc, img_box, rgb(14, 17, 20));

    // Variant 02 ("Elongated Pill"): right-aligned micro-dot strip with generous hit track + smooth scrub
    let scale_f = (st.dpi as f32) / 96.0;
    let slide_n = st.slides.len();
    let dot_step = 12.0 * scale_f;
    let pill_w = 16.0 * scale_f;
    let pill_h = if st.dots_dragging || st.hover == HitId::DotTrack { 5.8 * scale_f } else { 5.0 * scale_f };
    let dot_d = 4.8 * scale_f;
    let dots_cy = (img_box.bottom as f32) - 24.0 * scale_f;
    let dots_right = (img_box.right as f32) - 18.0 * scale_f;
    let dots_last_cx = dots_right - pill_w * 0.5;
    let dots_first_cx = if slide_n > 1 {
        dots_last_cx - ((slide_n - 1) as f32) * dot_step
    } else {
        dots_last_cx
    };
    if slide_n > 1 {
        let pad_x = st.s(14);
        let pad_y = st.s(14);
        let track_rc = RECT {
            left: (dots_first_cx - pill_w * 0.5).round() as i32 - pad_x,
            top: dots_cy.round() as i32 - pad_y,
            right: dots_right.round() as i32 + pad_x,
            bottom: dots_cy.round() as i32 + pad_y,
        };
        st.dots_track_rc = track_rc;
        st.dots_first_cx = dots_first_cx;
        st.dots_step_x = dot_step;
        st.hits.push(HitRect { id: HitId::DotTrack, rect: track_rc });
    } else {
        st.dots_track_rc = RECT::default();
        st.dots_first_cx = 0.0;
        st.dots_step_x = 0.0;
    }

    if !st.slides.is_empty() {
        let idx = st.slide_idx.min(st.slides.len() - 1);
        let slide_title = st.slides[idx].title.clone();
        let slide_quote = st.slides[idx].quote.clone();
        let slide_author = st.slides[idx].author.clone();

        if let Some(Some(img)) = st.slide_imgs.get(idx) {
            if img.w > 0 && img.h > 0 && img_box.w() > 0 && img_box.h() > 0 {
                let mut gp: *mut c_void = std::ptr::null_mut();
                if GdipCreateFromHDC(hdc, &mut gp) == 0 && !gp.is_null() {
                    GdipSetInterpolationMode(gp, 7 /* HighQualityBicubic */);
                    let bw = img_box.w() as f64;
                    let bh = img_box.h() as f64;
                    let box_ar = bw / bh;
                    let iw = img.w as f64;
                    let ih = img.h as f64;
                    let img_ar = iw / ih;

                    let (sx, sy, sw, sh) = if img_ar > box_ar {
                        let crop_w = (ih * box_ar).round().max(1.0) as i32;
                        (((img.w as i32 - crop_w) / 2).max(0), 0, crop_w.min(img.w as i32), img.h as i32)
                    } else {
                        let crop_h = (iw / box_ar).round().max(1.0) as i32;
                        (0, ((img.h as i32 - crop_h) / 4).max(0), img.w as i32, crop_h.min(img.h as i32))
                    };
                    GdipDrawImageRectRectI(
                        gp,
                        img.gp_img,
                        img_box.left,
                        img_box.top,
                        img_box.w(),
                        img_box.h(),
                        sx,
                        sy,
                        sw,
                        sh,
                        2, /* UnitPixel */
                        std::ptr::null(),
                        std::ptr::null(),
                        std::ptr::null(),
                    );

                    // Soft dark gradient over bottom portion seamlessly matching COL_BG
                    let grad_h = st.s(190).min(img_box.h());
                    let grad_top = img_box.bottom - grad_h;
                    let p1 = POINT { x: img_box.left, y: grad_top };
                    let p2 = POINT { x: img_box.left, y: img_box.bottom };
                    let mut br: *mut c_void = std::ptr::null_mut();
                    if GdipCreateLineBrushI(
                        &p1,
                        &p2,
                        argb(0, 22, 25, 29),
                        argb(252, 22, 25, 29),
                        3, /* WrapModeTileFlipXY */
                        &mut br,
                    ) == 0
                        && !br.is_null()
                    {
                        GdipFillRectangleI(gp, br, img_box.left, grad_top, img_box.w(), grad_h);
                        GdipDeleteBrush(br);
                    }
                    GdipDeleteGraphics(gp);
                }
            }
        }

        // Overlaid Variant 2 editorial block (bottom-left with green accent bar) kept clear of dot pill track
        let tx0 = img_box.left + st.s(30);
        let right_guard = if slide_n > 1 {
            (dots_first_cx - pill_w * 0.5).round() as i32 - st.s(18)
        } else {
            img_box.right - st.s(20)
        };
        let tx1 = right_guard.max(tx0 + st.s(180));
        let txt_w = (tx1 - tx0).max(st.s(180));

        // Measure auto-fitted title height (1..2 lines, shrinking font cleanly so long titles never clip)
        let mut chosen_font = make_font(st, 20, FW_BOLD, false);
        let mut title_h = st.s(26);
        for px in [21, 19, 17, 15] {
            let cand = make_font(st, px, FW_BOLD, false);
            let old_f = SelectObject(hdc, cand as HGDIOBJ);
            let mut calc = RECT { left: tx0, top: 0, right: tx0 + txt_w, bottom: 0 };
            let w_t = to_wide(&slide_title);
            DrawTextW(hdc, w_t.as_ptr(), -1, &mut calc, DT_LEFT | DT_WORDBREAK | 0x0400 /* DT_CALCRECT */ | DT_NOPREFIX);
            SelectObject(hdc, old_f);
            let single_h = st.s(px + 8);
            if calc.h() <= single_h || px == 15 {
                DeleteObject(chosen_font as HGDIOBJ);
                chosen_font = cand;
                title_h = calc.h().min(st.s(54));
                break;
            }
            DeleteObject(cand as HGDIOBJ);
        }

        // Measure quote height cleanly
        let quote_str = format!("«{slide_quote}»");
        let quote_h = {
            let old_f = SelectObject(hdc, f_quote as HGDIOBJ);
            let mut calc = RECT { left: tx0, top: 0, right: tx0 + txt_w, bottom: 0 };
            let w_q = to_wide(&quote_str);
            DrawTextW(hdc, w_q.as_ptr(), -1, &mut calc, DT_LEFT | DT_WORDBREAK | 0x0400 | DT_NOPREFIX);
            SelectObject(hdc, old_f);
            calc.h().min(st.s(64))
        };

        let has_author = !slide_author.is_empty();
        let author_h = if has_author { st.s(20) } else { 0 };
        let total_blk_h = title_h + st.s(5) + quote_h + if has_author { st.s(4) + author_h } else { 0 };
        let blk_bottom = img_box.bottom - st.s(16);
        let title_top = blk_bottom - total_blk_h;

        // Vertical green accent bar
        fill_rect(
            hdc,
            RECT { left: img_box.left + st.s(16), top: title_top + st.s(2), right: img_box.left + st.s(19), bottom: blk_bottom - st.s(2) },
            COL_GREEN,
        );

        draw_text(
            hdc,
            &slide_title,
            RECT { left: tx0, top: title_top, right: tx1, bottom: title_top + title_h + st.s(4) },
            chosen_font,
            COL_TEXT,
            DT_LEFT | DT_WORDBREAK,
        );
        DeleteObject(chosen_font as HGDIOBJ);

        let q_top = title_top + title_h + st.s(5);
        draw_text(
            hdc,
            &quote_str,
            RECT { left: tx0, top: q_top, right: tx1, bottom: q_top + quote_h + st.s(4) },
            f_quote,
            rgb(214, 222, 230),
            DT_LEFT | DT_WORDBREAK,
        );
        if has_author {
            let a_top = q_top + quote_h + st.s(4);
            draw_text(
                hdc,
                &format!("— {slide_author}"),
                RECT { left: tx0, top: a_top, right: tx1, bottom: a_top + st.s(20) },
                f_author,
                COL_MUTED,
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }

        // Draw Variant 02 micro-dots + floating elongated green pill
        if slide_n > 1 {
            let mut gp: *mut c_void = std::ptr::null_mut();
            if GdipCreateFromHDC(hdc, &mut gp) == 0 && !gp.is_null() {
                let _ = GdipSetSmoothingMode(gp, 4 /* AntiAlias */);
                let pill_pos = st.slide_visual_pos.clamp(0.0, (slide_n - 1) as f32);
                let pill_cx = dots_first_cx + pill_pos * dot_step;

                let mut b_dot: *mut c_void = std::ptr::null_mut();
                let mut b_dot_hov: *mut c_void = std::ptr::null_mut();
                let _ = GdipCreateSolidFill(argb(128, 152, 162, 174), &mut b_dot);
                let _ = GdipCreateSolidFill(argb(228, 212, 226, 238), &mut b_dot_hov);

                for i in 0..slide_n {
                    let cx = dots_first_cx + (i as f32) * dot_step;
                    let dist = (cx - pill_cx).abs();
                    let vis = ((dist - pill_w * 0.34) / (dot_step * 0.45)).clamp(0.0, 1.0);
                    if vis > 0.08 {
                        let is_hov = st.dots_hover_idx == Some(i) && i != idx;
                        let d_eff = dot_d * (0.55 + 0.45 * vis) * if is_hov { 1.24 } else { 1.0 };
                        let br = if is_hov { b_dot_hov } else { b_dot };
                        if !br.is_null() {
                            let _ = GdipFillEllipse(gp, br, cx - d_eff * 0.5, dots_cy - d_eff * 0.5, d_eff, d_eff);
                        }
                    }
                }
                if !b_dot.is_null() { let _ = GdipDeleteBrush(b_dot); }
                if !b_dot_hov.is_null() { let _ = GdipDeleteBrush(b_dot_hov); }

                let pill_col = if st.dots_dragging {
                    argb(255, 141, 248, 128)
                } else if st.hover == HitId::DotTrack {
                    argb(255, 134, 206, 8)
                } else {
                    argb(255, 118, 185, 0)
                };
                let mut p_pill: *mut c_void = std::ptr::null_mut();
                if GdipCreatePen1(pill_col, pill_h, 2, &mut p_pill) == 0 && !p_pill.is_null() {
                    let _ = GdipSetPenStartCap(p_pill, 2);
                    let _ = GdipSetPenEndCap(p_pill, 2);
                    let half_line = ((pill_w - pill_h) * 0.5).max(1.0);
                    let _ = GdipDrawLine(gp, p_pill, pill_cx - half_line, dots_cy, pill_cx + half_line, dots_cy);
                    let _ = GdipDeletePen(p_pill);
                }
                GdipDeleteGraphics(gp);
            }
        }
    }

    // Bottom progress bar (0px square)
    let bar_top = ch - st.s(42);
    draw_text(
        hdc,
        &st.status_text,
        RECT { left, top: ch - st.s(68), right: right - st.s(54), bottom: ch - st.s(46) },
        f_stat,
        COL_TEXT,
        DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    draw_text(
        hdc,
        &format!("{}%", st.progress),
        RECT { left: right - st.s(50), top: ch - st.s(68), right, bottom: ch - st.s(46) },
        f_stat,
        COL_GREEN,
        DT_RIGHT | DT_SINGLELINE,
    );

    let track = RECT { left, top: bar_top, right, bottom: bar_top + st.s(14) };
    draw_square_box(hdc, track, rgb(32, 37, 44), COL_BORDER);
    let fw = ((track.w() as u32) * st.progress.min(100) / 100) as i32;
    if fw > 0 {
        fill_rect(hdc, RECT { left: track.left, top: track.top, right: track.left + fw, bottom: track.bottom }, COL_GREEN);
    }

    for f in [f_gtitle, f_quote, f_author, f_stat] {
        DeleteObject(f as HGDIOBJ);
    }
}

unsafe fn paint_screen_done(hdc: HDC, st: &mut AppState, x0: i32, cw: i32, ch: i32) {
    let left = x0;
    let right = cw - st.s(22);

    let f_h1 = make_font(st, 22, FW_BOLD, false);
    let f_txt = make_font(st, 13, FW_NORMAL, false);
    let f_small = make_font(st, 11, FW_NORMAL, false);
    let f_btn = make_font(st, 13, FW_BOLD, false);

    if let Some(err) = st.error_text.clone() {
        draw_text(hdc, "Установка прервана с ошибкой", RECT { left, top: st.s(48), right, bottom: st.s(78) }, f_h1, COL_RED_HOVER, DT_LEFT | DT_SINGLELINE);
        let box_rc = RECT { left, top: st.s(92), right, bottom: ch - st.s(94) };
        draw_square_box(hdc, box_rc, COL_CARD, COL_RED_HOVER);
        draw_text(hdc, &err, RECT { left: box_rc.left + st.s(16), top: box_rc.top + st.s(16), right: box_rc.right - st.s(16), bottom: box_rc.bottom - st.s(16) }, f_txt, COL_TEXT, DT_LEFT | DT_WORDBREAK);
        draw_text(hdc, &format!("Лог сохранён: {}", win32::log_file_path().display()), RECT { left, top: ch - st.s(82), right, bottom: ch - st.s(62) }, f_small, COL_MUTED, DT_LEFT | DT_SINGLELINE);
        let bw_close = btn_width(hdc, f_btn, "Закрыть", 130, st);
        draw_button(hdc, st, RECT { left: right - bw_close, top: ch - st.s(54), right, bottom: ch - st.s(18) }, HitId::BtnDoneClose, "Закрыть", false, false, f_btn);
    } else if let Some(rep) = st.install_report.clone() {
        let heading = if rep.reboot_required {
            "Установка завершена (нужна перезагрузка)"
        } else {
            "Установка успешно завершена"
        };
        draw_text(hdc, heading, RECT { left, top: st.s(48), right, bottom: st.s(78) }, f_h1, COL_GREEN, DT_LEFT | DT_SINGLELINE);

        let box_rc = RECT { left, top: st.s(92), right, bottom: ch - st.s(96) };
        draw_square_box(hdc, box_rc, COL_CARD, COL_BORDER);

        let mut summary = String::from("Установлены компоненты:\r\n");
        for c in &rep.installed_components {
            summary.push_str(&format!("  • {c}\r\n"));
        }
        if !rep.warnings.is_empty() {
            summary.push_str("\r\nПримечания:\r\n");
            for w in &rep.warnings {
                summary.push_str(&format!("  • {w}\r\n"));
            }
        }
        draw_text(hdc, &summary, RECT { left: box_rc.left + st.s(16), top: box_rc.top + st.s(16), right: box_rc.right - st.s(16), bottom: box_rc.bottom - st.s(16) }, f_txt, COL_TEXT, DT_LEFT | DT_WORDBREAK);
        draw_text(hdc, &format!("Лог сохранён: {}", rep.log_path.display()), RECT { left, top: ch - st.s(82), right, bottom: ch - st.s(62) }, f_small, COL_MUTED, DT_LEFT | DT_SINGLELINE);

        if rep.reboot_required {
            let bw_reb = btn_width(hdc, f_btn, "Перезагрузить сейчас", 200, st);
            let bw_lat = btn_width(hdc, f_btn, "Позже", 110, st);
            draw_button(hdc, st, RECT { left: right - bw_reb, top: ch - st.s(54), right, bottom: ch - st.s(18) }, HitId::BtnDonePrimary, "Перезагрузить сейчас", true, false, f_btn);
            draw_button(hdc, st, RECT { left: right - bw_reb - st.s(10) - bw_lat, top: ch - st.s(54), right: right - bw_reb - st.s(10), bottom: ch - st.s(18) }, HitId::BtnDoneClose, "Позже", false, false, f_btn);
        } else {
            let bw_done = btn_width(hdc, f_btn, "Готово", 146, st);
            draw_button(hdc, st, RECT { left: right - bw_done, top: ch - st.s(54), right, bottom: ch - st.s(18) }, HitId::BtnDonePrimary, "Готово", true, false, f_btn);
        }
    }

    for f in [f_h1, f_txt, f_small, f_btn] {
        DeleteObject(f as HGDIOBJ);
    }
}

unsafe fn paint_screen_uninstall_confirm(hdc: HDC, st: &mut AppState, x0: i32, cw: i32, ch: i32) {
    let left = x0;
    let right = cw - st.s(22);

    let f_h1 = make_font(st, 22, FW_BOLD, false);
    let f_txt = make_font(st, 13, FW_NORMAL, false);
    let f_btn = make_font(st, 13, FW_BOLD, false);

    draw_text(hdc, "Удаление OpenFrankenDriver", RECT { left, top: st.s(48), right, bottom: st.s(78) }, f_h1, COL_TEXT, DT_LEFT | DT_SINGLELINE);
    let box_rc = RECT { left, top: st.s(92), right, bottom: ch - st.s(90) };
    draw_square_box(hdc, box_rc, COL_CARD, COL_BORDER);

    let msg = "Будут полностью удалены:\r\n\
      • Видеодрайвер NVIDIA и аудиодрайвер HD Audio из DriverStore (pnputil)\r\n\
      • Классическая Панель управления NVIDIA (Win32) и ярлыки\r\n\
      • NVIDIA PhysX (файлы, 32+64 ветки реестра и переменная PATH)\r\n\
      • Настройки NGXCore, nvtopps, кэши шейдеров (DXCache / GLCache / ComputeCache)\r\n\
      • Запись OpenFrankenDriver в «Программах и компонентах» Windows";
    draw_text(hdc, msg, RECT { left: box_rc.left + st.s(18), top: box_rc.top + st.s(18), right: box_rc.right - st.s(18), bottom: box_rc.bottom - st.s(18) }, f_txt, COL_TEXT, DT_LEFT | DT_WORDBREAK);

    let back_lbl = if st.started_in_uninstall { "Отмена" } else { "Назад" };
    let bw_del = btn_width(hdc, f_btn, "Удалить полностью", 180, st);
    let bw_back = btn_width(hdc, f_btn, back_lbl, 110, st);
    draw_button(hdc, st, RECT { left: right - bw_del, top: ch - st.s(54), right, bottom: ch - st.s(18) }, HitId::BtnUninstallConfirm, "Удалить полностью", false, true, f_btn);
    draw_button(hdc, st, RECT { left: right - bw_del - st.s(10) - bw_back, top: ch - st.s(54), right: right - bw_del - st.s(10), bottom: ch - st.s(18) }, HitId::BtnUninstallBack, back_lbl, false, false, f_btn);

    for f in [f_h1, f_txt, f_btn] {
        DeleteObject(f as HGDIOBJ);
    }
}

unsafe fn paint_screen_uninstalling(hdc: HDC, st: &mut AppState, x0: i32, cw: i32, ch: i32) {
    let left = x0;
    let right = cw - st.s(22);
    let f_h1 = make_font(st, 22, FW_BOLD, false);
    let f_stat = make_font(st, 13, FW_SEMIBOLD, false);

    draw_text(hdc, "Удаление драйвера...", RECT { left, top: st.s(48), right, bottom: st.s(78) }, f_h1, COL_TEXT, DT_LEFT | DT_SINGLELINE);
    draw_text(hdc, &st.status_text, RECT { left, top: st.s(180), right: right - st.s(60), bottom: st.s(204) }, f_stat, COL_TEXT, DT_LEFT | DT_SINGLELINE);
    draw_text(hdc, &format!("{}%", st.progress), RECT { left: right - st.s(56), top: st.s(180), right, bottom: st.s(204) }, f_stat, COL_GREEN, DT_RIGHT | DT_SINGLELINE);

    let track = RECT { left, top: st.s(212), right, bottom: st.s(226) };
    draw_square_box(hdc, track, rgb(32, 37, 44), COL_BORDER);
    let fw = ((track.w() as u32) * st.progress.min(100) / 100) as i32;
    if fw > 0 {
        fill_rect(hdc, RECT { left: track.left, top: track.top, right: track.left + fw, bottom: track.bottom }, COL_GREEN);
    }

    let _ = ch;
    DeleteObject(f_h1 as HGDIOBJ);
    DeleteObject(f_stat as HGDIOBJ);
}

unsafe fn paint_screen_uninstall_done(hdc: HDC, st: &mut AppState, x0: i32, cw: i32, ch: i32) {
    let left = x0;
    let right = cw - st.s(22);
    let f_h1 = make_font(st, 22, FW_BOLD, false);
    let f_txt = make_font(st, 13, FW_NORMAL, false);
    let f_btn = make_font(st, 13, FW_BOLD, false);

    draw_text(hdc, "Удаление завершено", RECT { left, top: st.s(48), right, bottom: st.s(78) }, f_h1, COL_GREEN, DT_LEFT | DT_SINGLELINE);
    let info = if st.uninstall_reboot {
        "Драйвер NVIDIA и все компоненты OpenFrankenDriver удалены.\r\nРекомендуется перезагрузить компьютер."
    } else {
        "Драйвер NVIDIA и все компоненты OpenFrankenDriver полностью удалены."
    };
    draw_text(hdc, info, RECT { left, top: st.s(96), right, bottom: st.s(166) }, f_txt, COL_TEXT, DT_LEFT | DT_WORDBREAK);

    if st.uninstall_reboot {
        let bw_reb = btn_width(hdc, f_btn, "Перезагрузить сейчас", 200, st);
        let bw_lat = btn_width(hdc, f_btn, "Позже", 110, st);
        draw_button(hdc, st, RECT { left: right - bw_reb, top: ch - st.s(54), right, bottom: ch - st.s(18) }, HitId::BtnDonePrimary, "Перезагрузить сейчас", true, false, f_btn);
        draw_button(hdc, st, RECT { left: right - bw_reb - st.s(10) - bw_lat, top: ch - st.s(54), right: right - bw_reb - st.s(10), bottom: ch - st.s(18) }, HitId::BtnDoneClose, "Позже", false, false, f_btn);
    } else {
        let bw_done = btn_width(hdc, f_btn, "Готово", 146, st);
        draw_button(hdc, st, RECT { left: right - bw_done, top: ch - st.s(54), right, bottom: ch - st.s(18) }, HitId::BtnDoneClose, "Готово", true, false, f_btn);
    }

    for f in [f_h1, f_txt, f_btn] {
        DeleteObject(f as HGDIOBJ);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu;

    #[test]
    fn bold_stage_words_fit_60_cols_and_anvil_settles() {
        for word in ["ЭТАП", "КОМПОНЕНТЫ", "УСТАНОВКА", "ГОТОВО", "ПОДГОТОВКА", "УДАЛЕНИЕ"] {
            let g = rasterize_bold_word(word);
            assert!(g.w > 0 && g.w <= MAX_STAGE_WORD_COLS, "{word} width {} exceeds 60", g.w);
        }
        assert_eq!(eval_anvil_impact(1.0, 11.0), 0);
        assert!(eval_anvil_impact(0.30, 11.0) > 8);
    }

    #[test]
    #[ignore]
    fn render_three_stage_screenshots() {
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

        let out_dir = std::env::var_os("OFD_SCREENSHOT_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("ofd_screenshots"));
        let _ = std::fs::create_dir_all(&out_dir);

        unsafe {
            let mut gp_token: usize = 0;
            let gp_in = GdiplusStartupInput {
                gdiplus_version: 1,
                debug_event_callback: 0,
                suppress_background_thread: 0,
                suppress_external_codecs: 0,
            };
            let _ = GdiplusStartup(&mut gp_token, &gp_in, std::ptr::null_mut());

            let cw = 900i32;
            let ch = 580i32;
            let sb_w = 284i32;
            let scr_dc = GetDC(std::ptr::null_mut());
            let mem = CreateCompatibleDC(scr_dc);
            let bmi = BitmapInfo {
                bmi_header: BitmapInfoHeader {
                    bi_size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    bi_width: cw,
                    bi_height: -ch, // top-down
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
            assert!(!dib.is_null() && !bits_ptr.is_null(), "CreateDIBSection failed: scr_dc={:?}", scr_dc);
            let old_bmp = SelectObject(mem, dib as HGDIOBJ);
            SetBkMode(mem, TRANSPARENT);
            eprintln!("DIB OK: mem={:?}, dib={:?}, bits={:?}", mem, dib, bits_ptr);

            let pkg_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\drivers\OpenFrankenDriver"));
            let meta = gpu::load_package_meta(&pkg_root);
            let sys = gpu::detect_system_summary();
            let now_ms = win32::tick_count();

            let slide = crate::slides::SlideItem {
                id: "homm3".to_string(),
                title: "Heroes of Might and Magic III".to_string(),
                quote: "Астрологи объявили неделю Золотого Дракона. Прирост Золотых Драконов увеличен на 5.".to_string(),
                author: String::new(),
                slide_file: "slide_screen_homm3.png".to_string(),
                kind: "classic".to_string(),
                pinned: false,
            };
            let showcase_dir = std::env::var_os("OFD_SHOWCASE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| pkg_root.join("Showcase"));
            let slide_img = load_image_from_disk_no_lock(&showcase_dir.join("slide_homm3.jpg"));

            let mut st = AppState {
                pkg_root,
                meta,
                sys,
                opts: InstallOptions::default(),
                slides: vec![slide],
                slide_imgs: vec![slide_img],
                slide_idx: 0,
                slide_visual_pos: 0.0,
                dots_track_rc: RECT::default(),
                dots_first_cx: 0.0,
                dots_step_x: 0.0,
                dots_dragging: false,
                dots_hover_idx: None,
                screen: Screen::Components,
                dpi: 96,
                last_work_w: 0,
                last_work_h: 0,
                hover: HitId::None,
                hits: Vec::new(),
                progress: 0,
                status_text: String::new(),
                install_report: None,
                error_text: None,
                uninstall_reboot: false,
                worker: Arc::new(Mutex::new(WorkerProgress {
                    percent: 0,
                    status: String::new(),
                    crt_lines: Vec::new(),
                    install_res: None,
                    uninstall_res: None,
                })),
                should_reboot_on_exit: false,
                started_in_uninstall: false,
                crt_tab: 0,
                crt_lines: vec![
                    "[INF] Проверка пакета 621.40 ok".to_string(),
                    "[DEV] PCI\\VEN_10DE обнаружен".to_string(),
                    "[SYS] Запуск nv_dispwi.inf".to_string(),
                    "[DRV] Копирование OpenFranken...".to_string(),
                    "[EXT] Установка PhysX 9.23".to_string(),
                ],
                crt_stream: "[INF] Проверка пакета 621.40 ok / [DEV] PCI\\VEN_10DE обнаружен / [SYS] Запуск nv_dispi.inf / [DRV] Копирование OpenFranken... / [EXT] Установка PhysX 9.23".to_string(),
                crt_char_phases: Vec::new(),
                crt_revealed_chars: 0,
                crt_blink_on: true,
                install_start_ms: None,
                install_end_ms: None,
                copied_until_ms: 0,
                anim_start_ms: now_ms.saturating_sub(3_200),
                morph_start_ms: 0,
                morph_from_idx: 0,
                morph_to_idx: 0,
                stage_fx_variant: 1,
                cat: CatState::new(now_ms),
            };

            let save_bmp = |path: PathBuf, ptr: *const u8| {
                let pixel_bytes = (cw as usize) * (ch as usize) * 4;
                let file_size = 54 + pixel_bytes;
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
                buf.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
                buf.extend_from_slice(&0i32.to_le_bytes());
                buf.extend_from_slice(&0i32.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                let slice = std::slice::from_raw_parts(ptr, pixel_bytes);
                buf.extend_from_slice(slice);
                std::fs::write(path, buf).expect("write bmp");
            };

            let render_frame = |st: &mut AppState, fname: &str| {
                let cr = RECT { left: 0, top: 0, right: cw, bottom: ch };
                st.hits.clear();
                eprintln!("[{fname}] header");
                fill_rect(mem, cr, COL_BG);
                paint_header(mem, st, sb_w, cw);
                eprintln!("[{fname}] sidebar");
                paint_sidebar(mem, st, sb_w, ch);
                eprintln!("[{fname}] content");
                paint_content(mem, st, sb_w, cw, ch);
                draw_square_border(mem, cr, COL_OUTER_BORDER);
                GdiFlush();
                save_bmp(out_dir.join(fname), bits_ptr as *const u8);
                eprintln!("[{fname}] saved");
            };

            // Seed matrix rain columns & PC sleep anchor for Stage 1
            let floor_y = (ch - 10) as f64;
            let right_edge = (sb_w - 22) as f64;
            let pc_tw_x = right_edge - 28.0 - 10.0 - 2.0;
            let pc_tw_y = floor_y - 58.0;
            st.cat.rain_init = true;
            for gx in 0..40 {
                st.cat.rain_y[gx] = 60.0 + ((gx as f64 * 53.0) % (ch as f64 - 80.0));
            }
            st.cat.x = pc_tw_x + 14.0 + 4.5;
            st.cat.y = pc_tw_y + 2.7;
            st.cat.on_pc = true;
            st.cat.behavior = CatBehavior::SleepPc;
            render_frame(&mut st, "stage1_components.bmp");

            // Stage 2: Installing (64%, HOMM3 slide, live MM:SS log tab, cat playing with caught 5-cube ribbon!)
            st.screen = Screen::Installing;
            st.morph_from_idx = 1;
            st.morph_to_idx = 1;
            st.progress = 64;
            st.crt_stream.clear();
            st.crt_char_phases.clear();
            st.push_crt_tokens(&["CERT[Root]=0x0".to_string(), "APPX NVCPL: none".to_string()], 0);
            st.push_crt_tokens(&["MATCH.INF=>nv_dispi.inf".to_string(), "PNP.MOUNT:+oem3.inf[0x0]".to_string()], 1);
            st.push_crt_tokens(&["HDA.PNP:nvhda.inf".to_string()], 2);
            st.crt_revealed_chars = st.crt_stream.chars().count().saturating_sub(2);
            st.status_text = "Установка видеодрайвера NVIDIA (pnputil /add-driver)...".to_string();
            st.crt_tab = 1;
            st.install_start_ms = Some(now_ms.saturating_sub(28_000));
            st.cat.on_pc = false;
            st.cat.x = 76.0;
            st.cat.y = floor_y;
            st.cat.dir = 1.0;
            st.cat.behavior = CatBehavior::PlayCaught;
            st.cat.belly_t = 2.0;
            st.cat.caught = Some(CaughtRibbon {
                glyph_idx: 4,
                is_red: false,
                mode: CatchPlayMode::PawJuggle,
                vx: 22.0,
                vy: -95.0,
                nodes_x: [94.0, 96.0, 99.0, 103.0, 107.0, 111.0],
                nodes_y: [floor_y - 38.0, floor_y - 31.0, floor_y - 24.0, floor_y - 17.0, floor_y - 10.0, floor_y - 4.0],
            });
            render_frame(&mut st, "stage2_installing.bmp");

            // Stage 3: Done (completed screen + cat standing on hind legs hitting log + glowing hearts)
            st.screen = Screen::Done;
            st.morph_from_idx = 2;
            st.morph_to_idx = 2;
            st.install_end_ms = Some(now_ms.saturating_sub(2_000));
            st.install_report = Some(InstallReport {
                reboot_required: true,
                installed_components: vec![
                    "Видеодрайвер NVIDIA 621.40".to_string(),
                    "Классическая Панель управления NVIDIA (Win32)".to_string(),
                    "NVIDIA PhysX System Software".to_string(),
                    "Visual C++ Redistributable (x64 + x86)".to_string(),
                ],
                warnings: vec![],
                log_path: PathBuf::from(r"C:\ProgramData\OpenFrankenDriver\setup.log"),
            });
            st.cat.caught = None;
            st.cat.behavior = CatBehavior::BellyBliss;
            st.cat.belly_roll = 1.0;
            st.cat.belly_t = 2.5;
            st.cat.x = 76.0;
            st.cat.y = floor_y;
            st.cat.hearts = vec![
                HeartParticle { x: 66.0, y: floor_y - 32.0, vx: -8.0, vy: -22.0, size: 11.0, life: 0.9, tint_idx: 0 },
                HeartParticle { x: 82.0, y: floor_y - 44.0, vx: 6.0, vy: -26.0, size: 12.0, life: 0.8, tint_idx: 2 },
                HeartParticle { x: 74.0, y: floor_y - 24.0, vx: 2.0, vy: -19.0, size: 10.0, life: 0.9, tint_idx: 1 },
            ];
            render_frame(&mut st, "stage3_done.bmp");

            drop(st);
            SelectObject(mem, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), scr_dc);
            if gp_token != 0 {
                GdiplusShutdown(gp_token);
            }
        }
    }

    #[test]
    #[ignore]
    fn render_all_showcase_covers() {
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

        let pkg_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\drivers\OpenFrankenDriver"));
        let showcase_dir = std::env::var_os("OFD_SHOWCASE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| pkg_root.join("Showcase"));
        let out_dir = std::env::var_os("OFD_SCREENSHOT_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("ofd_screenshots").join("all_covers_bmp"));
        std::fs::create_dir_all(&out_dir).expect("mkdir out_dir");

        let json_path = if showcase_dir.join("showcase.json").is_file() {
            showcase_dir.join("showcase.json")
        } else {
            pkg_root.join("data").join("games_showcase.json")
        };
        let json_txt = std::fs::read_to_string(&json_path).expect("read showcase json");
        let slides = crate::slides::parse_slides(&json_txt).expect("parse_slides");
        assert!(!slides.is_empty(), "expected non-empty slides");

        let cw = 900i32;
        let ch = 580i32;
        let sb_w = 266i32;

        unsafe {
            let mut gp_token: usize = 0;
            let gp_in = GdiplusStartupInput {
                gdiplus_version: 1,
                debug_event_callback: 0,
                suppress_background_thread: 0,
                suppress_external_codecs: 0,
            };
            let _ = GdiplusStartup(&mut gp_token, &gp_in, std::ptr::null_mut());

            let slide_imgs: Vec<Option<LoadedImage>> = slides
                .iter()
                .map(|s| load_image_from_disk_no_lock(&showcase_dir.join(&s.slide_file)))
                .collect();

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

            let meta = gpu::load_package_meta(&pkg_root);
            let sys = gpu::detect_system_summary();
            let now_ms = win32::tick_count();

            let mut st = AppState {
                pkg_root,
                meta,
                sys,
                opts: InstallOptions::default(),
                slides,
                slide_imgs,
                slide_idx: 0,
                slide_visual_pos: 0.0,
                dots_track_rc: RECT::default(),
                dots_first_cx: 0.0,
                dots_step_x: 0.0,
                dots_dragging: false,
                dots_hover_idx: None,
                screen: Screen::Installing,
                dpi: 96,
                last_work_w: 0,
                last_work_h: 0,
                hover: HitId::None,
                hits: Vec::new(),
                progress: 64,
                status_text: "Установка видеодрайвера NVIDIA (pnputil /add-driver)...".to_string(),
                install_report: None,
                error_text: None,
                uninstall_reboot: false,
                worker: Arc::new(Mutex::new(WorkerProgress {
                    percent: 64,
                    status: String::new(),
                    crt_lines: Vec::new(),
                    install_res: None,
                    uninstall_res: None,
                })),
                should_reboot_on_exit: false,
                started_in_uninstall: false,
                crt_tab: 1,
                crt_lines: Vec::new(),
                crt_stream: String::new(),
                crt_char_phases: Vec::new(),
                crt_revealed_chars: 0,
                crt_blink_on: true,
                install_start_ms: Some(now_ms.saturating_sub(28_000)),
                install_end_ms: None,
                copied_until_ms: 0,
                anim_start_ms: now_ms.saturating_sub(3_200),
                morph_start_ms: 0,
                morph_from_idx: 1,
                morph_to_idx: 1,
                stage_fx_variant: 1,
                cat: CatState::new(now_ms),
            };
            st.push_crt_tokens(&["CERT[Root]=0x0".to_string(), "APPX NVCPL: none".to_string()], 0);
            st.push_crt_tokens(&["MATCH.INF=>nv_dispi.inf".to_string(), "PNP.MOUNT:+oem3.inf[0x0]".to_string()], 1);
            st.push_crt_tokens(&["HDA.PNP:nvhda.inf".to_string()], 2);
            st.crt_revealed_chars = st.crt_stream.chars().count().saturating_sub(2);

            let floor_y = (ch - 10) as f64;
            st.cat.rain_init = true;
            for gx in 0..40 {
                st.cat.rain_y[gx] = 60.0 + ((gx as f64 * 53.0) % (ch as f64 - 80.0));
            }
            st.cat.x = 76.0;
            st.cat.y = floor_y;
            st.cat.dir = 1.0;
            st.cat.behavior = CatBehavior::PlayCaught;
            st.cat.belly_t = 2.0;
            st.cat.caught = Some(CaughtRibbon {
                glyph_idx: 4,
                is_red: false,
                mode: CatchPlayMode::PawJuggle,
                vx: 22.0,
                vy: -95.0,
                nodes_x: [94.0, 96.0, 99.0, 103.0, 107.0, 111.0],
                nodes_y: [floor_y - 38.0, floor_y - 31.0, floor_y - 24.0, floor_y - 17.0, floor_y - 10.0, floor_y - 4.0],
            });

            let n = st.slides.len();
            for idx in 0..n {
                st.slide_idx = idx;
                st.slide_visual_pos = idx as f32;
                let id = st.slides[idx].id.clone();
                let cr = RECT { left: 0, top: 0, right: cw, bottom: ch };
                st.hits.clear();
                fill_rect(mem, cr, COL_BG);
                paint_header(mem, &mut st, sb_w, cw);
                paint_sidebar(mem, &mut st, sb_w, ch);
                paint_content(mem, &mut st, sb_w, cw, ch);
                draw_square_border(mem, cr, COL_OUTER_BORDER);
                GdiFlush();

                let pixel_bytes = (cw as usize) * (ch as usize) * 4;
                let file_size = 54 + pixel_bytes;
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
                buf.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
                buf.extend_from_slice(&0i32.to_le_bytes());
                buf.extend_from_slice(&0i32.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(std::slice::from_raw_parts(bits_ptr as *const u8, pixel_bytes));
                let fname = format!("cover_{:02}_{}.bmp", idx + 1, id);
                std::fs::write(out_dir.join(fname), buf).expect("write cover bmp");
            }

            drop(st);
            SelectObject(mem, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), scr_dc);
            if gp_token != 0 {
                GdiplusShutdown(gp_token);
            }
        }
    }

    #[test]
    fn dot_scrubbing_snaps_nearest_and_clamps() {
        let now_ms = 1000u64;
        let slides = (0..17)
            .map(|i| crate::slides::SlideItem {
                id: format!("s{i}"),
                title: format!("T{i}"),
                quote: "q".to_string(),
                author: String::new(),
                slide_file: "x.jpg".to_string(),
                kind: "classic".to_string(),
                pinned: false,
            })
            .collect();
        let st = AppState {
            pkg_root: PathBuf::from("."),
            meta: gpu::PackageMeta::default(),
            sys: gpu::detect_system_summary(),
            opts: InstallOptions::default(),
            slides,
            slide_imgs: Vec::new(),
            slide_idx: 0,
            slide_visual_pos: 0.0,
            dots_track_rc: RECT::default(),
            dots_first_cx: 500.0,
            dots_step_x: 12.0,
            dots_dragging: false,
            dots_hover_idx: None,
            screen: Screen::Installing,
            dpi: 96,
            last_work_w: 0,
            last_work_h: 0,
            hover: HitId::None,
            hits: Vec::new(),
            progress: 0,
            status_text: String::new(),
            install_report: None,
            error_text: None,
            uninstall_reboot: false,
            worker: Arc::new(Mutex::new(WorkerProgress {
                percent: 0,
                status: String::new(),
                crt_lines: Vec::new(),
                install_res: None,
                uninstall_res: None,
            })),
            should_reboot_on_exit: false,
            started_in_uninstall: false,
            crt_tab: 0,
            crt_lines: Vec::new(),
            crt_stream: String::new(),
            crt_char_phases: Vec::new(),
            crt_revealed_chars: 0,
            crt_blink_on: true,
            install_start_ms: None,
            install_end_ms: None,
            copied_until_ms: 0,
            anim_start_ms: now_ms,
            morph_start_ms: 0,
            morph_from_idx: 0,
            morph_to_idx: 0,
            stage_fx_variant: 1,
            cat: CatState::new(now_ms),
        };
        assert_eq!(st.dot_index_for_x(400), 0);
        assert_eq!(st.dot_index_for_x(505), 0); // +5px from dot 0 -> snaps to dot 0
        assert_eq!(st.dot_index_for_x(507), 1); // +7px from dot 0 (closer to 512) -> snaps to dot 1
        assert_eq!(st.dot_index_for_x(518), 2); // halfway/past 512..524 -> snaps to dot 2
        assert_eq!(st.dot_index_for_x(999), 16);
        assert!((st.dot_continuous_pos_for_x(506) - 0.5).abs() < 0.01);
        assert!((st.dot_continuous_pos_for_x(300) - 0.0).abs() < 0.01);
        assert!((st.dot_continuous_pos_for_x(999) - 16.0).abs() < 0.01);
    }
}
