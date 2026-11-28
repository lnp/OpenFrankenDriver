//! Full driver installation pipeline reproducing all verified NVIDIA NVI / Ext phases.

use std::fs;
use std::path::{Path, PathBuf};

use crate::gpu::{self, PackageMeta, SystemSummary};
use crate::physx_manifest;
use crate::shortcut;
use crate::win32::{self, RegView};

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub install_nvcpl: bool,
    pub install_hdaudio: bool,
    pub install_physx: bool,
    pub install_vcpp: bool,
    pub install_nvpi: bool,
    pub clean_install: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            install_nvcpl: true,
            install_hdaudio: true,
            install_physx: true,
            install_vcpp: true,
            install_nvpi: false,
            clean_install: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub reboot_required: bool,
    pub installed_components: Vec<String>,
    pub warnings: Vec<String>,
    pub log_path: PathBuf,
}

pub fn run_install<F, G>(
    pkg_root: &Path,
    meta: &PackageMeta,
    sys: &SystemSummary,
    opts: &InstallOptions,
    mut on_progress: F,
    mut on_crt: G,
) -> Result<InstallReport, String>
where
    F: FnMut(u32, &str),
    G: FnMut(&str),
{
    if win32::is_safe_mode() {
        return Err("Установка видеодрайвера в безопасном режиме (Safe Mode) не поддерживается.".to_string());
    }

    let mut reboot_required = false;
    let mut installed = Vec::new();
    let mut warnings = Vec::new();
    let sys32 = win32::system_directory();
    let windows_inf_dir = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string())).join("INF");

    win32::log_line(&format!(
        "=== Starting OpenFrankenDriver {} install (nvcpl={}, hdaudio={}, physx={}, vcpp={}, nvpi={}, clean={}) ===",
        meta.driver_version, opts.install_nvcpl, opts.install_hdaudio, opts.install_physx, opts.install_vcpp, opts.install_nvpi, opts.clean_install
    ));
    let (os_build, _, sb) = win32::read_windows_diag_info();
    let sb_s = sb.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
    on_crt(&format!("SYS.INIT[{os_build}] SB={sb_s} SAFE=0"));
    on_crt(&format!("PKG.VER={} VC64={} VC86={}", meta.driver_version, sys.vc_x64_ok as u8, sys.vc_x86_ok as u8));
    for h in sys.hwids.iter().take(4) {
        on_crt(&format!("ENUM:{h}"));
    }

    // Step 1: Clean old state if requested + baseline NVIDIA cache/legacy cleanups
    on_progress(5, "[1/10] Подготовка системы и очистка кэшей...");
    if opts.clean_install {
        on_crt("CLEAN.MODE=ON");
        cleanup_existing_driver(&windows_inf_dir, &mut on_crt);
    } else {
        on_crt("STOP:nvcplui.exe");
        win32::terminate_processes_by_name("nvcplui.exe");
        on_crt("SVC.STOP:NVDisplay.ContainerLS");
        win32::stop_service("NVDisplay.ContainerLocalSystem", 12_000);
        win32::stop_service("nvsvc", 5_000);
        let _ = cleanup_conflicting_official_arp();
        on_crt("ARP.CLEAN:CONFLICTS");
    }
    cleanup_legacy_and_caches(opts.clean_install);
    on_crt("CACHE.PURGE:NV+GL+DRS");

    // Step 2: Import FrankenDriver certificate into Root, CA, TrustedPublisher
    on_progress(12, "[2/10] Импорт сертификата подписи каталога...");
    let cer = pkg_root.join("FrankenDriver").join("FrankenDriver.cer");
    if cer.is_file() {
        let certutil = sys32.join("certutil.exe");
        let cer_str = cer.to_string_lossy().into_owned();
        for store in ["Root", "CA", "TrustedPublisher"] {
            match win32::run_hidden_timeout(&certutil, &["-addstore", "-f", store, &cer_str], 15_000) {
                Ok((0, _, false)) => on_crt(&format!("CERT[{store}]=0x0")),
                Ok((_, _, true)) => {
                    on_crt(&format!("CERT[{store}]=TIMEOUT"));
                    warnings.push(format!("certutil({store}): таймаут 15с"));
                }
                Ok((rc, out, false)) => {
                    on_crt(&format!("CERT[{store}]=ERR({rc})"));
                    warnings.push(format!("certutil({store}) код {rc}: {}", out.trim()));
                }
                Err(e) => {
                    on_crt(&format!("CERT[{store}]=FAIL"));
                    warnings.push(format!("certutil({store}): {e}"));
                }
            }
        }
    } else {
        warnings.push("Сертификат FrankenDriver.cer не найден рядом с пакетом.".to_string());
        on_crt("CERT:MISSING");
    }

    // Step 3: Pre-configure _nvtopps DisableLoad=1 & NVTweak/Startup flags BEFORE PnP starts container
    on_progress(18, "[3/10] Отключение телеметрии контейнера (nvtopps)...");
    for svc_sub in [
        r"HKLM\SYSTEM\CurrentControlSet\Services\NVDisplay.ContainerLocalSystem\Parameters\LocalSystem\_nvtopps",
        r"HKLM\SYSTEM\CurrentControlSet\Services\NVDisplay.ContainerLocalSystem\Parameters\LocalSystem\NvTopps",
    ] {
        let _ = win32::reg_set_dword(svc_sub, "DisableLoad", 1, RegView::Native64);
    }
    on_crt("REG:_nvtopps.DisableLoad=1");
    if opts.install_nvcpl {
        let _ = apply_nvcpl_policy_flags();
        on_crt("REG:NVTweak.DCH_UI=1");
    }

    // Step 4: Remove conflicting Store NVCPL AppX & InstalledPfns key if Store NVCPL exists
    on_progress(24, "[4/10] Проверка конфликтующего Store AppX Панели управления...");
    on_crt("APPX.SCAN:NVIDIAControlPanel");
    let appx_msg = remove_store_nvcpl_if_present(&sys32);
    on_crt(appx_msg);

    // Step 5: Remove old NGXCore keys before installing Display.Driver, then install via pnputil
    on_progress(32, "[5/10] Установка видеодрайвера NVIDIA (pnputil)...");
    win32::reg_delete_tree(r"HKLM\SOFTWARE\NVIDIA Corporation\Global\NGXCore", RegView::Native64);
    on_crt("REG.DEL:NGXCore");

    let display_dir = pkg_root.join("Display.Driver");
    let target_inf = gpu::select_display_inf(&display_dir, &sys.hwids);
    if !target_inf.is_file() {
        return Err(format!("Не найден INF-файл видеодрайвера: {}", target_inf.display()));
    }
    let inf_name = target_inf.file_name().and_then(|n| n.to_str()).unwrap_or("nv_dispi.inf");
    on_crt(&format!("MATCH.INF=>{inf_name}"));
    on_crt("PNP.ADD:/install");

    let before_infs = gpu::snapshot_oem_infs(&windows_inf_dir);
    let pnputil = sys32.join("pnputil.exe");
    let inf_arg = target_inf.to_string_lossy().into_owned();
    let mut next_tick_ms = 2_000u64;
    let mut next_disk_log_ms = 12_000u64;
    let mut phase_idx = 0usize;
    let pnp_phases = [
        "STORE.STAGE", "CAT.VERIFY", "INF.PARSE", "DRV.COPY", "SYS.REG", "DEV.BIND",
    ];
    let (pnp_rc, pnp_out, pnp_timed_out) = win32::run_hidden_timeout_with_ticks(
        &pnputil,
        &["/add-driver", &inf_arg, "/install"],
        0,
        |elapsed_ms| {
            if elapsed_ms >= next_tick_ms {
                let secs = elapsed_ms / 1000;
                let tag = pnp_phases[phase_idx % pnp_phases.len()];
                phase_idx += 1;
                on_crt(&format!("{tag}[{secs}s]"));
                next_tick_ms += 2_000;
            }
            if elapsed_ms >= next_disk_log_ms {
                win32::log_line(&format!("pnputil heartbeat [{}s] still running...", elapsed_ms / 1000));
                next_disk_log_ms += 12_000;
            }
        },
    )?;
    let _ = pnp_timed_out;
    match pnp_rc {
        0 | 259 => {}
        3010 => reboot_required = true,
        other => {
            on_crt(&format!("PNP.ERR[{other}]"));
            return Err(format!(
                "pnputil вернул ошибку {other} при установке {}:\n{}",
                target_inf.display(),
                pnp_out.trim()
            ));
        }
    }
    let after_infs = gpu::snapshot_oem_infs(&windows_inf_dir);
    let newly_added_infs: Vec<String> = after_infs.difference(&before_infs).cloned().collect();
    win32::log_line(&format!("New oem*.inf after Display install: {:?}", newly_added_infs));
    let oem_label = newly_added_infs.first().map(|s| s.as_str()).unwrap_or("nv_dispi");
    on_crt(&format!("PNP.MOUNT:+{oem_label}[0x0]"));
    installed.push(format!("Видеодрайвер NVIDIA {}", meta.driver_version));

    // Step 6: NGXCore + nvngx_update.exe -forced_update (4s timeout) + MSI Interrupt fix + Dangling devices + Confirm-install
    on_progress(52, "[6/10] Настройка DLSS / NGXCore и проверка прерываний...");
    if let Err(e) = configure_ngx_core(&newly_added_infs, &windows_inf_dir, &mut on_crt) {
        warnings.push(e);
    }

    if win32::check_and_fix_msi_interrupt_mode() {
        reboot_required = true;
        on_crt("MSI.FIX:REBOOT_REQ");
    } else {
        on_crt("MSI.CHK:VEN_10DE[OK]");
    }
    let removed_root = win32::remove_dangling_root_devices(&["ROOT\\NVMODULETRACKER", "ROOT\\NVVHCI"]);
    on_crt(&format!("DEV.PRUNE:ROOT={removed_root}"));

    if let Err(e) = confirm_display_install(&sys32, !sys.hwids.is_empty()) {
        on_crt("VERIFY:NVAPI64=FAIL");
        rollback_new_oems(&pnputil, &newly_added_infs);
        return Err(e);
    }
    on_crt("VERIFY:nvinfo+nvapi64[OK]");

    // Step 7: HDAudio driver (if selected and folder exists)
    if opts.install_hdaudio {
        on_progress(64, "[7/10] Установка аудиодрайвера NVIDIA HD Audio...");
        let hda_inf = pkg_root.join("HDAudio").join("nvhda.inf");
        if hda_inf.is_file() {
            let ph = win32::remove_phantom_nvidia_devices();
            on_crt(&format!("HDA.PHANTOM={ph}"));
            on_crt("HDA.PNP:nvhda.inf");
            let hda_str = hda_inf.to_string_lossy().into_owned();
            match win32::run_hidden_timeout(&pnputil, &["/add-driver", &hda_str, "/install"], 60_000) {
                Ok((0 | 259, _, false)) => {
                    installed.push(format!("NVIDIA HD Audio {}", meta.hdaudio_version));
                    on_crt(&format!("HDA.OK[{}]", meta.hdaudio_version));
                }
                Ok((3010, _, false)) => {
                    reboot_required = true;
                    installed.push(format!("NVIDIA HD Audio {}", meta.hdaudio_version));
                    on_crt("HDA.OK[REBOOT]");
                }
                Ok((_, _, true)) => {
                    warnings.push("HDAudio pnputil превысил таймаут 60с".to_string());
                    on_crt("HDA.TIMEOUT[60s]");
                }
                Ok((rc, out, false)) => {
                    warnings.push(format!("HDAudio pnputil код {rc}: {}", out.trim()));
                    on_crt(&format!("HDA.WARN[{rc}]"));
                }
                Err(e) => warnings.push(format!("HDAudio: {e}")),
            }
        }
    }

    // Step 8: NVIDIA PhysX (files + WOW6432Node & Native64 registry + PATH)
    if opts.install_physx && pkg_root.join("PhysX").join("physx_manifest.json").is_file() {
        on_progress(75, "[8/10] Установка NVIDIA PhysX (32+64 реестр)...");
        on_crt("PHX.UNINST:OLD_MSI");
        on_crt("PHX.STAGE:53_FILES");
        physx_manifest::install(pkg_root)?;
        installed.push(format!("NVIDIA PhysX {}", meta.physx_version));
        on_crt(&format!("PHX.REG:x86+x64+PATH[{}]", meta.physx_version));
    }

    // Optional: NVIDIA Profile Inspector + Start Menu shortcut under NVIDIA Corporation
    if opts.install_nvpi && pkg_root.join("NVProfileInspector").join("nvidiaProfileInspector.exe").is_file() {
        on_progress(83, "Установка NVIDIA Profile Inspector...");
        install_nvpi_tool(pkg_root, &mut on_crt)?;
        installed.push(format!("NVIDIA Profile Inspector v{}", meta.nvpi_version));
    }

    // Step 9: Visual C++ Runtime 2015-2022 + ARP registration
    on_progress(88, "[9/10] Проверка VC++ Runtime и регистрация в Windows...");
    if opts.install_vcpp {
        on_crt("VC++.CHK:14.42");
        if install_vcpp_runtimes(pkg_root, sys, &mut warnings) {
            reboot_required = true;
        }
        on_crt("VC++.DONE[OK]");
    }

    register_arp_and_uninstaller(pkg_root, meta)?;
    on_crt("UNINST.COPY:uninstaller.exe");
    on_crt("ARP.REG:OpenFrankenDriver");

    // Step 10 (installClose parity with v1 NVCPL.nvi): Classic Win32 NVIDIA Control Panel strictly LAST
    // after Display.Driver + container + all other installers have settled!
    if opts.install_nvcpl && pkg_root.join("Display.NVCPL").is_dir() {
        on_progress(94, "[10/10] Установка классической Панели управления NVIDIA...");
        install_win32_nvcpl(pkg_root, &mut on_crt)?;
        installed.push("Панель управления NVIDIA (Win32)".to_string());
    }

    let gpu_stats = win32::check_nvidia_gpu_devnode_status();
    if gpu_stats.is_empty() {
        win32::log_line("Post-install GPU check: no active PCI\\VEN_10DE devnode found (offline/no-GPU).");
        on_crt("DEV_STATUS:OFFLINE[0x0]");
    } else {
        for (inst, st, prob) in &gpu_stats {
            win32::log_line(&format!("Post-install GPU check: {inst} -> Status=0x{st:08X}, Problem={prob}"));
            if *prob == 14 {
                reboot_required = true;
            } else if *prob != 0 {
                warnings.push(format!("Устройство {inst} вернуло код состояния Windows Error {prob} (может потребоваться перезагрузка)."));
            }
        }
        let max_prob = gpu_stats.iter().map(|(_, _, p)| *p).max().unwrap_or(0);
        let tag = if max_prob == 0 { "RUNNING" } else if max_prob == 14 { "REBOOT" } else { "WARN" };
        on_crt(&format!("DEV_STATUS=0x{max_prob:X}[{tag}]"));
    }

    on_progress(100, "Установка завершена.");

    Ok(InstallReport {
        reboot_required,
        installed_components: installed,
        warnings,
        log_path: win32::log_file_path(),
    })
}

fn cleanup_conflicting_official_arp() -> usize {
    cleanup_official_arp_entries(false)
}

fn cleanup_official_arp_entries(clean_all: bool) -> usize {
    let uninst_root = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let official_prefix = "{B2FE1952-0186-46C3-BAEC-A80AA35AC5B8}_";
    let conflict_suffixes = [
        "Display.Driver",
        "HDAudio.Driver",
        "Display.PhysX",
        "Display.ControlPanel",
        "Display.Optimus",
        "Display.Update",
    ];
    let mut count = 0usize;
    for sub in win32::reg_enum_subkeys(uninst_root, RegView::Native64) {
        if !sub.to_ascii_uppercase().starts_with(official_prefix) {
            continue;
        }
        let suffix = &sub[official_prefix.len()..];
        if clean_all || conflict_suffixes.iter().any(|s| suffix.eq_ignore_ascii_case(s)) {
            win32::log_line(&format!("Removing official ARP entry: {sub}"));
            win32::reg_delete_tree(&format!(r"{uninst_root}\{sub}"), RegView::Native64);
            count += 1;
        }
    }
    count
}

fn cleanup_existing_driver<G: FnMut(&str)>(windows_inf_dir: &Path, on_crt: &mut G) {
    win32::log_line("Clean Install: stopping official NVIDIA processes/services and purging remnants...");
    for proc_name in [
        "nvcplui.exe",
        "NvGpuUtilization.exe",
        "NVIDIA App.exe",
        "NVIDIA Overlay.exe",
        "nvcontainer.exe",
        "nvsphelper64.exe",
        "PresentMon_x64.exe",
    ] {
        win32::terminate_processes_by_name(proc_name);
    }
    for svc in [
        "NVDisplay.ContainerLocalSystem",
        "NvContainerLocalSystem",
        "NvContainerNetworkService",
        "NvTelemetryContainer",
        "FvSvc",
        "nvsvc",
    ] {
        win32::stop_service(svc, 8_000);
    }

    on_crt("Clean: removing old PhysX/NVCPL");
    physx_manifest::uninstall();

    let pf64 = win32::program_files_64();
    let pfx86 = win32::program_files_x86();
    let common_app = win32::sh_folder_path(win32::CSIDL_COMMON_APPDATA)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    for dir in [
        pf64.join("NVIDIA Corporation").join("Installer2"),
        pf64.join("NVIDIA Corporation").join("NvContainer"),
        pf64.join("NVIDIA Corporation").join("NVIDIA App"),
        pf64.join("NVIDIA Corporation").join("Control Panel Client"),
        pf64.join("NVIDIA Corporation").join("FrameViewSDK"),
        pfx86.join("NVIDIA Corporation").join("NvContainer"),
        common_app.join("NVIDIA Corporation").join("Downloader"),
        common_app.join("NVIDIA Corporation").join("NvTelemetry"),
    ] {
        let _ = fs::remove_dir_all(&dir);
    }

    if let Some(desktop) = win32::sh_folder_path(win32::CSIDL_COMMON_DESKTOPDIRECTORY) {
        shortcut::remove_matching_shortcuts(&desktop);
    }
    if let Some(programs) = win32::sh_folder_path(win32::CSIDL_COMMON_PROGRAMS) {
        let nv_dir = programs.join("NVIDIA Corporation");
        shortcut::remove_matching_shortcuts(&nv_dir);
    }

    let _ = cleanup_official_arp_entries(true);
    win32::reg_delete_tree(r"HKLM\SOFTWARE\NVIDIA Corporation", RegView::Native64);
    win32::reg_delete_tree(r"HKLM\SOFTWARE\NVIDIA Corporation", RegView::Wow32);
    win32::reg_delete_tree(r"HKCU\SOFTWARE\NVIDIA Corporation", RegView::Native64);

    let pnputil = win32::system_directory().join("pnputil.exe");
    let old_oems = gpu::find_nvidia_oem_infs(windows_inf_dir);
    if !old_oems.is_empty() {
        on_crt(&format!("Clean: purging {} old INF", old_oems.len()));
    }
    for oem in old_oems {
        let _ = win32::run_hidden_timeout(&pnputil, &["/delete-driver", &oem, "/uninstall", "/force"], 30_000);
    }
    win32::remove_phantom_nvidia_devices();
}

fn cleanup_legacy_and_caches(clean_install: bool) {
    let sys32 = win32::system_directory();
    let syswow = win32::syswow_directory();
    let pf64 = win32::program_files_64();
    let pfx86 = win32::program_files_x86();
    let common_app = win32::sh_folder_path(win32::CSIDL_COMMON_APPDATA)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    let local_app = win32::sh_folder_path(win32::CSIDL_LOCAL_APPDATA);
    let roam_app = win32::sh_folder_path(win32::CSIDL_APPDATA);

    let temp_nv = std::env::temp_dir().join("NVIDIA Corporation").join("NV_Cache");
    let _ = fs::remove_dir_all(&temp_nv);
    let _ = fs::remove_dir_all(common_app.join("NVIDIA Corporation").join("NV_Cache"));
    let _ = fs::remove_file(sys32.join("nvmcumd.dll"));

    // Legacy DRS / shims / opencl (from DisplayDriver.nvi)
    let drs_dir = pf64.join("NVIDIA Corporation").join("DRS");
    let _ = fs::remove_file(drs_dir.join("dbInstaller.exe"));
    let _ = fs::remove_file(drs_dir.join("nvdrsdb.bin"));
    let _ = fs::remove_dir_all(&drs_dir);

    for p in [
        sys32.join("nvinit.dll"),
        sys32.join("nvumdshim.dll"),
        sys32.join("nvinitx.dll"),
        sys32.join("nvumdshimx.dll"),
        sys32.join("nvoglshim32.dll"),
        sys32.join("nvoglshim64.dll"),
        syswow.join("nvinit.dll"),
        syswow.join("nvumdshim.dll"),
        syswow.join("nvoglshim32.dll"),
        sys32.join("nvopencl.dll"),
        syswow.join("nvopencl.dll"),
    ] {
        let _ = fs::remove_file(p);
    }
    let _ = fs::remove_dir_all(pf64.join("NVIDIA Corporation").join("coprocmanager"));
    let _ = fs::remove_dir_all(pfx86.join("NVIDIA Corporation").join("coprocmanager"));
    let _ = fs::remove_dir_all(pf64.join("NVIDIA Corporation").join("nvgsync"));
    let _ = fs::remove_dir_all(pfx86.join("NVIDIA Corporation").join("nvgsync"));

    // Legacy OpenGL/DX caches
    if let Some(ref la) = local_app {
        let _ = fs::remove_dir_all(la.join("NVIDIA Corporation").join("NV_Cache"));
        let _ = fs::remove_dir_all(la.join("NVIDIA").join("GLCache"));
        if let Some(parent) = la.parent() {
            let locallow = parent.join("LocalLow").join("NVIDIA");
            let _ = fs::remove_dir_all(locallow.join("GLCache"));
            if clean_install {
                let _ = fs::remove_dir_all(locallow.join("DXCache"));
                let _ = fs::remove_dir_all(locallow.join("PerDriverVersion"));
            }
        }
        if clean_install {
            let _ = fs::remove_dir_all(la.join("NVIDIA").join("DXCache"));
        }
    }
    if let Some(ref ra) = roam_app {
        let _ = fs::remove_dir_all(ra.join("NVIDIA").join("GLCache"));
        if clean_install {
            let _ = fs::remove_dir_all(ra.join("NVIDIA").join("ComputeCache"));
        }
    }
    if clean_install {
        let models = common_app.join("NVIDIA").join("NGX").join("models");
        for sub in [
            "sl_common_0", "sl_deepdvc_0", "sl_dlss_0", "sl_dlss_d_0",
            "sl_dlss_g_0", "sl_nis_0", "sl_nvperf_0", "sl_pcl_0",
            "sl_reflex_0", "sl_sdk_0",
        ] {
            let _ = fs::remove_dir_all(models.join(sub));
        }
        let _ = fs::remove_file(models.join("nvngx_config.txt"));
    }

    // Old ARP cleanup matching DisplayDriverExt!FUN_100c65e0
    win32::reg_delete_tree(
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\NVIDIA Drivers",
        RegView::Native64,
    );
    let _ = fs::remove_file(sys32.join("nvdisp.nvu"));
    let _ = fs::remove_file(sys32.join("nvdisp_ih.nvu"));

    // Legacy standalone VulkanRT cleanup if present
    if win32::reg_key_exists(r"HKLM\SOFTWARE\NVIDIA Corporation\Global\Vulkan", RegView::Native64) {
        if let Some(uninst) = win32::reg_read_string(
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VulkanRT",
            "UninstallString",
            RegView::Native64,
        ) {
            let clean = uninst.trim().trim_matches('"');
            let p = Path::new(clean);
            if p.is_file() {
                let _ = win32::run_hidden(p, &["/S"]);
            }
        }
        win32::reg_delete_tree(r"HKLM\SOFTWARE\NVIDIA Corporation\Global\Vulkan", RegView::Native64);
    }
}

fn remove_store_nvcpl_if_present(sys32: &Path) -> &'static str {
    let pfn_key = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\DeviceSetup\InstalledPfns";
    let pfn_name = "NVIDIACorp.NVIDIAControlPanel_56jybvy8sckqj";
    let has_pfn = win32::reg_read_string(pfn_key, pfn_name, RegView::Native64).is_some()
        || win32::reg_read_dword(pfn_key, pfn_name, RegView::Native64).is_some();
    if has_pfn {
        win32::reg_delete_value(pfn_key, pfn_name, RegView::Native64);
    }
    let ps = sys32.join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    if ps.is_file() {
        match win32::run_hidden_timeout(
            &ps,
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-AppxPackage -AllUsers -Name *NVIDIACorp.NVIDIAControlPanel* | Remove-AppxPackage -AllUsers",
            ],
            12_000,
        ) {
            Ok((_, out, false)) if !out.trim().is_empty() => return "AppX NVCPL: cleaned",
            Ok((_, _, true)) => return "AppX NVCPL: timeout 12s",
            _ => {}
        }
    }
    if has_pfn { "AppX NVCPL: pfn cleared" } else { "AppX NVCPL: none" }
}

fn configure_ngx_core<G: FnMut(&str)>(
    newly_added_infs: &[String],
    windows_inf_dir: &Path,
    on_crt: &mut G,
) -> Result<(), String> {
    let mut store_dir: Option<PathBuf> = None;
    for oem in newly_added_infs {
        let full = windows_inf_dir.join(oem);
        if let Some(d) = win32::resolve_driver_store_location(&full.to_string_lossy()) {
            if d.join("nv_dispi.inf").is_file() || d.to_string_lossy().to_ascii_lowercase().contains("nv_") {
                store_dir = Some(d);
                break;
            }
        }
    }
    if store_dir.is_none() {
        store_dir = find_latest_nv_driverstore();
    }
    let Some(dir) = store_dir else {
        on_crt("Store: NGX dir not found");
        return Err("NGXCore: не удалось определить папку DriverStore для nv_dispi.inf.".to_string());
    };

    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("nv_dispi");
    let short_dir: String = if dir_name.len() > 16 {
        format!("{}..{}", &dir_name[..7], &dir_name[dir_name.len() - 6..])
    } else {
        dir_name.to_string()
    };
    on_crt(&format!("Store: {short_dir}"));

    let updater = dir.join("nvngx_update.exe");
    if updater.is_file() {
        // Offline / slow network protection: hard 4s cap, non-fatal (matches DisplayDriver.nvi onError="ignore").
        match win32::run_hidden_timeout(&updater, &["-forced_update"], 4_000) {
            Ok((0, _, false)) => on_crt("NGX: nvngx_update OK"),
            Ok((_, _, true)) => on_crt("NGX: timeout 4s (offln)"),
            Ok((rc, _, false)) => {
                win32::log_line(&format!("nvngx_update.exe returned non-zero {rc} (non-fatal per NVI onError=ignore)"));
                on_crt(&format!("NGX: skipped (rc={rc})"));
            }
            Err(e) => {
                win32::log_line(&format!("nvngx_update.exe launch warning: {e}"));
                on_crt("NGX: skipped (err)");
            }
        }
    } else {
        on_crt("NGX: no updater (builtin)");
    }

    let ngx_key = r"HKLM\SOFTWARE\NVIDIA Corporation\Global\NGXCore";
    win32::reg_set_string(ngx_key, "FullPath", &dir.to_string_lossy(), RegView::Native64)?;
    win32::reg_set_dword(ngx_key, "Installed", 1, RegView::Native64)?;
    win32::log_line(&format!("NGXCore configured: FullPath={}", dir.display()));
    on_crt("NGXCore reg FullPath OK");
    Ok(())
}

fn find_latest_nv_driverstore() -> Option<PathBuf> {
    let repo = win32::system_directory().join("DriverStore").join("FileRepository");
    let entries = fs::read_dir(repo).ok()?;
    let mut cands: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("nv_dispi.inf_amd64_") && ent.path().is_dir() {
            let mtime = ent
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            cands.push((mtime, ent.path()));
        }
    }
    cands.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    cands.into_iter().next().map(|(_, p)| p)
}

fn confirm_display_install(sys32: &Path, gpu_present: bool) -> Result<(), String> {
    if !gpu_present {
        win32::log_line("Confirm-install: no NVIDIA PCI GPU detected; staged INF without active hardware binding.");
        return Ok(());
    }
    for f in ["nvinfo.pb", "nvapi64.dll"] {
        let p = sys32.join(f);
        if !p.is_file() {
            return Err(format!(
                "Confirm-install не пройден: в System32 отсутствует {}. Установка драйвера откачена/прервана.",
                f
            ));
        }
    }
    Ok(())
}

fn rollback_new_oems(pnputil: &Path, oems: &[String]) {
    for oem in oems {
        win32::log_line(&format!("Rolling back newly staged INF: {oem}"));
        let _ = win32::run_hidden(pnputil, &["/delete-driver", oem, "/uninstall", "/force"]);
    }
}

const NVCPL_FILES: [&str; 5] = [
    "nvcplui.exe",
    "nvcpluir.dll",
    "NvGpuUtilization.exe",
    "nvImage.dll",
    "EULA.txt",
];

/// Enforce DCH Win32 NVCPL visibility (`EnableNvcplUIOnDCH=1`) and mark nvxdapix Startup tasks as completed (`1`).
/// Note: in `nvxdapix.dll` (`StartupEngine`), `0` (or missing key) means PENDING (run at startup),
/// whereas `1` means COMPLETED (skip). Writing `0` re-arms `DeleteControlPanelClient`.
fn apply_nvcpl_policy_flags() -> Result<(), String> {
    for tweak in [
        r"HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm\Parameters\Global\NVTweak",
        r"HKLM\SOFTWARE\NVIDIA Corporation\Global\NVTweak",
    ] {
        win32::reg_set_dword(tweak, "EnableNvcplUIOnDCH", 1, RegView::Native64)?;
    }
    let startup_root = r"HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm\Parameters\Global\Startup";
    for flag in [
        "RemoveControlPanelClient",
        "RemoveControlPanelClientr",
        "ShowStoreNvcplNotification",
        "ShowStoreNvcplUpgradeNotification",
    ] {
        win32::reg_set_dword(&format!(r"{startup_root}\{flag}"), "", 1, RegView::Native64)?;
    }
    win32::log_line("NVCPL policy applied: EnableNvcplUIOnDCH=1, Startup\\RemoveControlPanelClient*=1");
    Ok(())
}

/// Copy NVCPL payload binaries from `src_dir` into `dst_dir` (logs full byte counts to setup.log).
fn deploy_nvcpl_files(src_dir: &Path, dst_dir: &Path, names: &[&str], tag: &str) -> Result<(), String> {
    for &f in names {
        let src = src_dir.join(f);
        let dst = dst_dir.join(f);
        if src.is_file() {
            let bytes = fs::copy(&src, &dst)
                .map_err(|e| format!("Не удалось скопировать {f}: {e}"))?;
            win32::log_line(&format!("NVCPL {tag} {f} ({bytes} bytes)"));
        } else {
            win32::log_line(&format!("WARNING: NVCPL source missing: {}", src.display()));
        }
    }
    Ok(())
}

pub fn install_win32_nvcpl<G: FnMut(&str)>(pkg_root: &Path, on_crt: &mut G) -> Result<(), String> {
    let src_dir = pkg_root.join("Display.NVCPL");
    let dst_dir = win32::program_files_64()
        .join("NVIDIA Corporation")
        .join("Control Panel Client");
    win32::log_line(&format!("Installing Win32 NVCPL: {} -> {}", src_dir.display(), dst_dir.display()));

    // Mark Startup removal triggers as completed (1) BEFORE copying files so nvxdapix skips DeleteControlPanelClient.
    fs::create_dir_all(&dst_dir).map_err(|e| format!("Не удалось создать {}: {e}", dst_dir.display()))?;
    apply_nvcpl_policy_flags()?;
    deploy_nvcpl_files(&src_dir, &dst_dir, &NVCPL_FILES, "copied")?;
    on_crt("NVCPL.CPY:5_FILES");

    let exe_path = dst_dir.join("nvcplui.exe");
    let exe_str = exe_path.to_string_lossy().into_owned();
    let dst_str = dst_dir.to_string_lossy().into_owned();

    let app_paths = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\nvcplui.exe";
    win32::reg_set_string(app_paths, "", &exe_str, RegView::Native64)?;
    win32::reg_set_string(app_paths, "Path", &dst_str, RegView::Native64)?;

    let ctx_key = r"HKLM\SOFTWARE\Classes\DesktopBackground\Shell\NvControlPanel";
    win32::reg_set_string(ctx_key, "", "NVIDIA Control Panel", RegView::Native64)?;
    win32::reg_set_string(ctx_key, "Icon", &format!("{exe_str},0"), RegView::Native64)?;
    win32::reg_set_string(&format!(r"{ctx_key}\command"), "", &format!("\"{exe_str}\""), RegView::Native64)?;
    on_crt("NVCPL.REG:AppPaths+CtxMenu");

    // Ensure NVDisplay.ContainerLocalSystem is running (returns immediately if already Running)
    // so nvcplui.exe launches cleanly when opened by the user.
    let mut svc_ok = win32::start_service("NVDisplay.ContainerLocalSystem", 25_000);
    if !svc_ok {
        win32::log_line("NVDisplay.ContainerLocalSystem not yet running; retrying...");
        std::thread::sleep(std::time::Duration::from_millis(1_500));
        svc_ok = win32::start_service("NVDisplay.ContainerLocalSystem", 15_000);
    }
    win32::log_line(&format!("SVC.START NVDisplay.ContainerLocalSystem running={svc_ok}"));
    on_crt(if svc_ok { "SVC.START:NVDisplayContainer[OK]" } else { "SVC.START:NVDisplayContainer[WARN]" });

    if exe_path.is_file() {
        if let Some(desktop) = win32::sh_folder_path(win32::CSIDL_COMMON_DESKTOPDIRECTORY) {
            shortcut::remove_nvcpl_shortcuts(&desktop);
            let lnk = desktop.join("NVIDIA Control Panel.lnk");
            match shortcut::create_shortcut(&lnk, &exe_path, &dst_dir, &exe_path, "NVIDIA Control Panel") {
                Ok(_) => win32::log_line(&format!("Created Desktop shortcut: {}", lnk.display())),
                Err(e) => win32::log_line(&format!("Desktop shortcut failed: {e}")),
            }
        }
        if let Some(programs) = win32::sh_folder_path(win32::CSIDL_COMMON_PROGRAMS) {
            let nv_dir = programs.join("NVIDIA Corporation");
            let _ = fs::create_dir_all(&nv_dir);
            // Keep existing NVIDIA Profile Inspector.lnk intact! Only replace NVCPL shortcuts.
            shortcut::remove_nvcpl_shortcuts(&nv_dir);
            let lnk = nv_dir.join("NVIDIA Control Panel.lnk");
            match shortcut::create_shortcut(&lnk, &exe_path, &dst_dir, &exe_path, "NVIDIA Control Panel") {
                Ok(_) => win32::log_line(&format!("Created StartMenu shortcut: {}", lnk.display())),
                Err(e) => win32::log_line(&format!("StartMenu shortcut failed: {e}")),
            }
        }
        win32::sh_notify_assoc_changed();
        on_crt("NVCPL.LNK:Desktop+StartMenu");
    }

    win32::log_line(&format!("NVCPL post-verify nvcplui.exe present={}", exe_path.is_file()));
    on_crt(if exe_path.is_file() { "NVCPL.VERIFY[OK]" } else { "NVCPL.VERIFY[MISSING]" });
    Ok(())
}

pub fn install_nvpi_tool<G: FnMut(&str)>(pkg_root: &Path, on_crt: &mut G) -> Result<(), String> {
    let src_dir = pkg_root.join("NVProfileInspector");
    let dst_dir = win32::program_files_64()
        .join("NVIDIA Corporation")
        .join("Profile Inspector");
    win32::log_line(&format!("Installing NVIDIA Profile Inspector: {} -> {}", src_dir.display(), dst_dir.display()));
    fs::create_dir_all(&dst_dir).map_err(|e| format!("Не удалось создать {}: {e}", dst_dir.display()))?;

    for f in [
        "nvidiaProfileInspector.exe",
        "nvidiaProfileInspector.exe.config",
        "Reference.xml",
    ] {
        let src = src_dir.join(f);
        let dst = dst_dir.join(f);
        if src.is_file() {
            let bytes = fs::copy(&src, &dst)
                .map_err(|e| format!("Не удалось скопировать {f}: {e}"))?;
            win32::log_line(&format!("NVPI copied {f} ({bytes} bytes)"));
        }
    }

    let exe_path = dst_dir.join("nvidiaProfileInspector.exe");
    if let Some(programs) = win32::sh_folder_path(win32::CSIDL_COMMON_PROGRAMS) {
        let nv_dir = programs.join("NVIDIA Corporation");
        let _ = fs::create_dir_all(&nv_dir);
        let lnk = nv_dir.join("NVIDIA Profile Inspector.lnk");
        let _ = fs::remove_file(&lnk);
        match shortcut::create_shortcut(&lnk, &exe_path, &dst_dir, &exe_path, "NVIDIA Profile Inspector") {
            Ok(_) => win32::log_line(&format!("Created StartMenu NVPI shortcut: {}", lnk.display())),
            Err(e) => win32::log_line(&format!("StartMenu NVPI shortcut failed: {e}")),
        }
    }
    win32::sh_notify_assoc_changed();
    on_crt("NVPI.LNK:StartMenu[OK]");
    Ok(())
}

fn install_vcpp_runtimes(pkg_root: &Path, sys: &SystemSummary, warnings: &mut Vec<String>) -> bool {
    let mut reboot = false;
    let msvcrt = pkg_root.join("MSVCRT");
    let items = [
        ("vc_redist.x64.exe", sys.vc_x64_ok),
        ("vc_redist.x86.exe", sys.vc_x86_ok),
    ];
    for (exe_name, already_ok) in items {
        if already_ok {
            win32::log_line(&format!("{exe_name} already present (>= 14.42.34438), skipping."));
            continue;
        }
        let exe = msvcrt.join(exe_name);
        if !exe.is_file() {
            continue;
        }
        match win32::run_hidden(&exe, &["/q", "/norestart"]) {
            Ok((0 | 1638, _)) => {}
            Ok((3010, _)) => reboot = true,
            Ok((rc, out)) => warnings.push(format!("{exe_name} вернул код {rc}: {}", out.trim())),
            Err(e) => warnings.push(format!("{exe_name}: {e}")),
        }
    }
    reboot
}

fn register_arp_and_uninstaller(pkg_root: &Path, meta: &PackageMeta) -> Result<(), String> {
    let app_dir = win32::program_files_64().join("OpenFrankenDriver");
    fs::create_dir_all(&app_dir).map_err(|e| format!("Не удалось создать {}: {e}", app_dir.display()))?;

    let uninst_exe = app_dir.join("uninstaller.exe");
    if let Ok(cur_exe) = std::env::current_exe() {
        if cur_exe != uninst_exe {
            fs::copy(&cur_exe, &uninst_exe)
                .map_err(|e| format!("Не удалось скопировать uninstaller.exe: {e}"))?;
        }
    }

    let physx_manifest = pkg_root.join("PhysX").join("physx_manifest.json");
    if physx_manifest.is_file() {
        let _ = fs::copy(&physx_manifest, app_dir.join("physx_manifest.json"));
    }

    let uninst_str = uninst_exe.to_string_lossy().into_owned();
    let arp_key = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\OpenFrankenDriver";
    win32::reg_set_string(arp_key, "DisplayName", &format!("OpenFrankenDriver {}", meta.driver_version), RegView::Native64)?;
    win32::reg_set_string(arp_key, "DisplayVersion", &meta.driver_version, RegView::Native64)?;
    win32::reg_set_string(arp_key, "Publisher", "OpenFrankenDriver", RegView::Native64)?;
    win32::reg_set_string(arp_key, "DisplayIcon", &format!("{uninst_str},0"), RegView::Native64)?;
    win32::reg_set_string(arp_key, "InstallLocation", &app_dir.to_string_lossy(), RegView::Native64)?;
    win32::reg_set_string(arp_key, "UninstallString", &format!("\"{uninst_str}\" --uninstall"), RegView::Native64)?;
    win32::reg_set_string(arp_key, "QuietUninstallString", &format!("\"{uninst_str}\" --uninstall --silent"), RegView::Native64)?;
    win32::reg_set_dword(arp_key, "NoModify", 1, RegView::Native64)?;
    win32::reg_set_dword(arp_key, "NoRepair", 1, RegView::Native64)?;
    Ok(())
}

/// Schedule deletion of `C:\NVIDIA\OpenFrankenDriver\<ver>` (and empty parent) on setup.exe exit.
/// Guarded strictly so manual test runs outside `...\NVIDIA\OpenFrankenDriver\...` never self-delete!
pub fn schedule_temp_unpack_cleanup_if_sfx(pkg_root: &Path) {
    let norm = pkg_root.to_string_lossy().to_ascii_lowercase();
    if !norm.contains(r"\nvidia\openfrankendriver\") {
        return;
    }
    win32::schedule_dir_self_cleanup(pkg_root, true);
}
