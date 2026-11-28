//! OpenFrankenDriver Beta native Rust setup & uninstaller entrypoint.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod gpu;
mod installer;
mod json_mini;
mod physx_manifest;
mod shortcut;
mod slides;
mod ui;
mod uninstaller;
mod win32;

use std::path::PathBuf;

use crate::installer::InstallOptions;

fn resolve_package_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_uninstall = args.iter().any(|a| {
        a.eq_ignore_ascii_case("--uninstall")
            || a.eq_ignore_ascii_case("-uninstall")
            || a.eq_ignore_ascii_case("/uninstall")
    });
    let is_silent = args.iter().any(|a| {
        a.eq_ignore_ascii_case("--silent")
            || a.eq_ignore_ascii_case("-s")
            || a.eq_ignore_ascii_case("/silent")
    });
    let is_clean = args.iter().any(|a| {
        a.eq_ignore_ascii_case("--clean")
            || a.eq_ignore_ascii_case("-clean")
    });
    let is_nvpi = args.iter().any(|a| {
        a.eq_ignore_ascii_case("--nvpi")
            || a.eq_ignore_ascii_case("-nvpi")
    });

    let pkg_root = resolve_package_root();
    let meta = gpu::load_package_meta(&pkg_root);
    let sys = gpu::detect_system_summary();

    let (os_build, os_disp, sb) = win32::read_windows_diag_info();
    let sb_str = sb.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
    win32::reset_session_log(&[
        format!("=== OpenFrankenDriver {} Session (Uninstall={}, Silent={}, CleanFlag={}) ===", meta.driver_version, is_uninstall, is_silent, is_clean),
        format!("OS: Windows Build {} ({}) | SecureBoot={} | SafeMode={}", os_build, os_disp, sb_str, win32::is_safe_mode()),
        format!("PackageRoot: {}", pkg_root.display()),
        format!("GPUs: {}", if sys.gpu_names.is_empty() { "none".to_string() } else { sys.gpu_names.join(" | ") }),
        format!("HWIDs: {}", if sys.hwids.is_empty() { "none".to_string() } else { sys.hwids.join(", ") }),
        format!("PrevDriver: {:?} | PrevOFD: {:?} | VC++(x64={}, x86={})", sys.current_driver_version, sys.openfranken_installed_version, sys.vc_x64_ok, sys.vc_x86_ok),
    ]);

    if is_silent {
        let code = if is_uninstall {
            match uninstaller::run_uninstall(|_, msg| win32::log_line(msg)) {
                Ok(_) => {
                    uninstaller::schedule_self_cleanup_if_installed();
                    0
                }
                Err(e) => {
                    win32::log_line(&format!("SILENT UNINSTALL ERROR: {e}"));
                    1
                }
            }
        } else {
            let mut opts = InstallOptions::default();
            opts.clean_install = is_clean;
            opts.install_nvpi = is_nvpi;
            let res = installer::run_install(&pkg_root, &meta, &sys, &opts, |_, msg| win32::log_line(msg), |_| {});
            installer::schedule_temp_unpack_cleanup_if_sfx(&pkg_root);
            match res {
                Ok(_) => 0,
                Err(e) => {
                    win32::log_line(&format!("SILENT INSTALL ERROR: {e}"));
                    1
                }
            }
        };
        std::process::exit(code);
    }

    let seed = win32::tick_count() ^ (std::process::id() as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    let slides = slides::load_from_package(&pkg_root, seed);
    let code = ui::run_gui(pkg_root, meta, sys, slides, is_uninstall);
    std::process::exit(code);
}
