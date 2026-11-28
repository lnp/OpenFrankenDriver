//! Complete driver & OpenFrankenDriver removal (Win32 NVCPL, PhysX, INF, caches, ARP, self-cleanup).

use std::fs;
use std::path::PathBuf;

use crate::gpu;
use crate::physx_manifest;
use crate::shortcut;
use crate::win32::{self, RegView};

pub fn run_uninstall<F>(mut on_progress: F) -> Result<bool, String>
where
    F: FnMut(u32, &str),
{
    let mut reboot_required = false;
    let sys32 = win32::system_directory();
    let windows_inf_dir = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string())).join("INF");

    win32::log_line("=== Starting OpenFrankenDriver Uninstall ===");

    // 1. Stop processes & container service
    on_progress(15, "[1/5] Остановка служб и Панели управления NVIDIA...");
    win32::terminate_processes_by_name("nvcplui.exe");
    win32::terminate_processes_by_name("NvGpuUtilization.exe");
    win32::stop_service("NVDisplay.ContainerLocalSystem", 12_000);
    win32::stop_service("nvsvc", 5_000);

    // 2. Remove Win32 Control Panel Client + shortcuts + registry
    on_progress(30, "[2/5] Удаление классической Панели управления NVIDIA...");
    remove_win32_nvcpl();

    // 3. Remove PhysX (files, x86/x64 registry, PATH)
    on_progress(50, "[3/5] Удаление NVIDIA PhysX...");
    physx_manifest::uninstall();

    // 4. Uninstall all NVIDIA Display & HDAudio OEM drivers via pnputil
    on_progress(75, "[4/5] Удаление драйверов из DriverStore (pnputil)...");
    let pnputil = sys32.join("pnputil.exe");
    let oems = gpu::find_nvidia_oem_infs(&windows_inf_dir);
    for oem in oems {
        match win32::run_hidden(&pnputil, &["/delete-driver", &oem, "/uninstall", "/force"]) {
            Ok((3010, _)) => reboot_required = true,
            Ok((rc, out)) if rc != 0 => {
                win32::log_line(&format!("Warning: pnputil /delete-driver {oem} exited {rc}: {}", out.trim()));
            }
            Err(e) => win32::log_line(&format!("Warning: pnputil /delete-driver {oem}: {e}")),
            _ => {}
        }
    }
    let _ = win32::remove_phantom_nvidia_devices();

    // 5. Clear NGXCore, _nvtopps, caches, and ARP entry
    on_progress(92, "[5/5] Очистка реестра, кэшей шейдеров и записи удаления...");
    win32::reg_delete_tree(r"HKLM\SOFTWARE\NVIDIA Corporation\Global\NGXCore", RegView::Native64);
    win32::reg_delete_tree(
        r"HKLM\SYSTEM\CurrentControlSet\Services\NVDisplay.ContainerLocalSystem\Parameters\LocalSystem\_nvtopps",
        RegView::Native64,
    );
    win32::reg_delete_tree(
        r"HKLM\SYSTEM\CurrentControlSet\Services\NVDisplay.ContainerLocalSystem\Parameters\LocalSystem\NvTopps",
        RegView::Native64,
    );
    win32::reg_delete_tree(
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\OpenFrankenDriver",
        RegView::Native64,
    );

    let app_dir = win32::program_files_64().join("OpenFrankenDriver");
    if let Ok(cur) = std::env::current_exe() {
        if !cur.to_string_lossy().to_ascii_lowercase().starts_with(&app_dir.to_string_lossy().to_ascii_lowercase()) {
            let _ = fs::remove_dir_all(&app_dir);
        }
    }

    if let Some(la) = win32::sh_folder_path(win32::CSIDL_LOCAL_APPDATA) {
        let _ = fs::remove_dir_all(la.join("NVIDIA").join("DXCache"));
        let _ = fs::remove_dir_all(la.join("NVIDIA").join("GLCache"));
        if let Some(parent) = la.parent() {
            let locallow = parent.join("LocalLow").join("NVIDIA");
            let _ = fs::remove_dir_all(locallow.join("DXCache"));
            let _ = fs::remove_dir_all(locallow.join("GLCache"));
            let _ = fs::remove_dir_all(locallow.join("PerDriverVersion"));
        }
    }
    if let Some(ra) = win32::sh_folder_path(win32::CSIDL_APPDATA) {
        let _ = fs::remove_dir_all(ra.join("NVIDIA").join("GLCache"));
        let _ = fs::remove_dir_all(ra.join("NVIDIA").join("ComputeCache"));
    }

    on_progress(100, "Удаление завершено.");
    win32::log_line("=== Uninstall Complete ===");
    Ok(reboot_required)
}

fn remove_win32_nvcpl() {
    for csidl in [win32::CSIDL_COMMON_DESKTOPDIRECTORY, win32::CSIDL_DESKTOPDIRECTORY] {
        if let Some(d) = win32::sh_folder_path(csidl) {
            shortcut::remove_matching_shortcuts(&d);
        }
    }
    for csidl in [win32::CSIDL_COMMON_PROGRAMS, win32::CSIDL_PROGRAMS] {
        if let Some(p) = win32::sh_folder_path(csidl) {
            let nv = p.join("NVIDIA Corporation");
            shortcut::remove_matching_shortcuts(&nv);
            let _ = fs::remove_dir(&nv);
        }
    }

    let nv_root = win32::program_files_64().join("NVIDIA Corporation");
    for sub in ["Control Panel Client", "Profile Inspector"] {
        let dir = nv_root.join(sub);
        if dir.is_dir() {
            let _ = fs::remove_dir_all(&dir);
        }
    }
    let _ = fs::remove_dir(&nv_root);

    win32::reg_delete_tree(
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\nvcplui.exe",
        RegView::Native64,
    );
    win32::reg_delete_tree(
        r"HKLM\SOFTWARE\Classes\DesktopBackground\Shell\NvControlPanel",
        RegView::Native64,
    );
    win32::reg_delete_value(
        r"HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm\Parameters\Global\NVTweak",
        "EnableNvcplUIOnDCH",
        RegView::Native64,
    );
    win32::reg_delete_value(
        r"HKLM\SOFTWARE\NVIDIA Corporation\Global\NVTweak",
        "EnableNvcplUIOnDCH",
        RegView::Native64,
    );
    win32::sh_notify_assoc_changed();
}

/// Schedule removal of `C:\Program Files\OpenFrankenDriver` if currently running from `uninstaller.exe`.
pub fn schedule_self_cleanup_if_installed() {
    if let Ok(cur) = std::env::current_exe() {
        let norm = cur.to_string_lossy().to_ascii_lowercase();
        if norm.ends_with(r"\openfrankendriver\uninstaller.exe") {
            if let Some(app_dir) = cur.parent() {
                win32::schedule_dir_self_cleanup(app_dir, false);
            }
        }
    }
}
