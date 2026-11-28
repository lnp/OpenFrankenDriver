//! GPU detection, INF selection, VC++ Runtime checks, and package metadata loading.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::json_mini;
use crate::win32::{self, RegView};

#[derive(Debug, Clone)]
pub struct PackageMeta {
    pub driver_version: String,
    pub physx_version: String,
    pub hdaudio_version: String,
    pub nvpi_version: String,
    pub estimated_mb: u64,
}

impl Default for PackageMeta {
    fn default() -> Self {
        Self {
            driver_version: "621.40".to_string(),
            physx_version: "9.26.0904".to_string(),
            hdaudio_version: "1.4.6.6".to_string(),
            nvpi_version: "3.0.2.1".to_string(),
            estimated_mb: 3200,
        }
    }
}

pub fn load_package_meta(pkg_root: &Path) -> PackageMeta {
    let p = pkg_root.join("Showcase").join("package_meta.json");
    if let Ok(txt) = fs::read_to_string(p) {
        if let Ok(v) = json_mini::parse(&txt) {
            return PackageMeta {
                driver_version: v.get_str("driver_version").unwrap_or("621.40").to_string(),
                physx_version: v.get_str("physx_version").unwrap_or("9.26.0904").to_string(),
                hdaudio_version: v.get_str("hdaudio_version").unwrap_or("1.4.6.6").to_string(),
                nvpi_version: v.get_str("nvpi_version").unwrap_or("3.0.2.1").to_string(),
                estimated_mb: v.get("estimated_mb").and_then(|n| n.as_i64()).unwrap_or(3200).max(500) as u64,
            };
        }
    }
    PackageMeta::default()
}

#[derive(Debug, Clone)]
pub struct SystemSummary {
    pub gpu_names: Vec<String>,
    pub hwids: Vec<String>,
    pub current_driver_version: Option<String>,
    pub openfranken_installed_version: Option<String>,
    pub vc_x64_ok: bool,
    pub vc_x86_ok: bool,
}

pub fn detect_system_summary() -> SystemSummary {
    let devs = win32::enum_pci_nvidia_devices(true);
    let mut gpu_names = Vec::new();
    let mut hwids = Vec::new();
    let mut current_ver: Option<String> = None;

    for dev in devs {
        if !dev.desc.is_empty() && !gpu_names.contains(&dev.desc) {
            gpu_names.push(dev.desc.clone());
        }
        for h in dev.hwids {
            if h.to_ascii_uppercase().contains("VEN_10DE") && !hwids.contains(&h) {
                hwids.push(h);
            }
        }
        if current_ver.is_none() && !dev.driver_key.is_empty() {
            let class_reg = format!(r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{}", dev.driver_key);
            if let Some(raw) = win32::reg_read_string(&class_reg, "DriverVersion", RegView::Native64) {
                current_ver = Some(format_nvidia_version(&raw));
            }
        }
    }

    let of_ver = win32::reg_read_string(
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\OpenFrankenDriver",
        "DisplayVersion",
        RegView::Native64,
    );

    SystemSummary {
        gpu_names,
        hwids,
        current_driver_version: current_ver,
        openfranken_installed_version: of_ver,
        vc_x64_ok: is_vcpp_installed(r"HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X64", RegView::Native64),
        vc_x86_ok: is_vcpp_installed(r"HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X86", RegView::Wow32),
    }
}

/// Convert Windows DriverVersion (`32.0.16.2140`) to NVIDIA display version (`621.40`).
pub fn format_nvidia_version(raw: &str) -> String {
    let parts: Vec<&str> = raw.trim().split('.').collect();
    if parts.len() == 4 {
        let p3 = parts[2];
        let p4 = parts[3];
        if !p3.is_empty() && p4.len() == 4 {
            let last_two = p4.chars().last().unwrap_or('0');
            let _ = last_two;
            let digits = format!("{}{}", p3.chars().last().unwrap_or('0'), p4);
            if digits.len() == 5 {
                return format!("{}.{}", &digits[..3], &digits[3..]);
            }
        }
    }
    raw.to_string()
}

fn is_vcpp_installed(key: &str, view: RegView) -> bool {
    if win32::reg_read_dword(key, "Installed", view) != Some(1) {
        return false;
    }
    // MSVCRuntime_2019.nvi ships 14.42.34438.0
    win32::reg_read_dword(key, "Bld", view).unwrap_or(0) >= 34438
}

/// Choose Display.Driver INF file for detected GPU HWIDs:
/// Checks universal `nv_dispi.inf` first (~0.7 MiB, instant); falls back to OEM INFs only if needed.
pub fn select_display_inf(display_dir: &Path, hwids: &[String]) -> PathBuf {
    let primary = display_dir.join("nv_dispi.inf");
    if hwids.is_empty() || !primary.is_file() {
        return primary;
    }

    if inf_contains_any_hwid(&primary, hwids) {
        return primary;
    }

    if let Ok(entries) = fs::read_dir(display_dir) {
        let mut oems: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| {
                p.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("inf")).unwrap_or(false)
                    && !p.file_name().and_then(|n| n.to_str()).map(|s| s.eq_ignore_ascii_case("nv_dispi.inf")).unwrap_or(false)
            })
            .collect();
        oems.sort();
        for oem in oems {
            if inf_contains_any_hwid(&oem, hwids) {
                win32::log_line(&format!("Matched OEM INF instead of nv_dispi.inf: {}", oem.display()));
                return oem;
            }
        }
    }
    primary
}

fn decode_inf_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    // Heuristic: UTF-16LE without BOM has frequent 0x00 in odd byte positions
    if bytes.len() >= 8 && bytes[1] == 0 && bytes[3] == 0 {
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn inf_contains_any_hwid(inf_path: &Path, hwids: &[String]) -> bool {
    let Ok(bytes) = fs::read(inf_path) else {
        return false;
    };
    let text = decode_inf_text(&bytes).to_ascii_uppercase();
    // Match most-specific HWID first (longest strings usually have SUBSYS/REV).
    let mut sorted: Vec<String> = hwids.iter().map(|h| h.to_ascii_uppercase()).collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.len()));
    for hwid in sorted {
        // Strip &REV_XX suffix if checking PCI base DEV+SUBSYS
        if text.contains(&hwid) {
            return true;
        }
    }
    false
}

/// Snapshot all `oem*.inf` filenames currently inside `C:\Windows\INF`.
pub fn snapshot_oem_infs(windows_inf_dir: &Path) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(windows_inf_dir) {
        for ent in entries.flatten() {
            if let Some(name) = ent.file_name().to_str() {
                let low = name.to_ascii_lowercase();
                if low.starts_with("oem") && low.ends_with(".inf") {
                    set.insert(low);
                }
            }
        }
    }
    set
}

/// Find NVIDIA Display / HDAudio OEM INFs directly in `C:\Windows\INF` without parsing CP866 pnputil text.
pub fn find_nvidia_oem_infs(windows_inf_dir: &Path) -> Vec<String> {
    let mut matched = Vec::new();
    if let Ok(entries) = fs::read_dir(windows_inf_dir) {
        for ent in entries.flatten() {
            let p = ent.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let low = name.to_ascii_lowercase();
            if !(low.starts_with("oem") && low.ends_with(".inf")) {
                continue;
            }
            if is_nvidia_display_or_hdaudio_inf(&p) {
                matched.push(low);
            }
        }
    }
    matched.sort();
    matched
}

fn is_nvidia_display_or_hdaudio_inf(inf_path: &Path) -> bool {
    let Ok(bytes) = fs::read(inf_path) else {
        return false;
    };
    let slice = &bytes[..bytes.len().min(64 * 1024)];
    let txt = decode_inf_text(slice).to_ascii_lowercase();
    let is_nv = txt.contains("nvidia") || txt.contains("nv_disp") || txt.contains("nvhda");
    let is_target_class = txt.contains("{4d36e968-e325-11ce-bfc1-08002be10318}") // Display
        || txt.contains("{4d36e96c-e325-11ce-bfc1-08002be10318}") // MEDIA (HDAudio)
        || txt.contains("class       = display")
        || txt.contains("class=display")
        || txt.contains("nvhda64v.sys");
    is_nv && is_target_class
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_driver_version_to_nvidia() {
        assert_eq!(format_nvidia_version("32.0.16.2140"), "621.40");
        assert_eq!(format_nvidia_version("32.0.15.6636"), "566.36");
    }

    #[test]
    fn decodes_utf16le_inf_headers() {
        let header = "; NVIDIA Windows 10\r\nClassGUID = {4D36E968-E325-11CE-BFC1-08002BE10318}\r\n";
        let mut raw = vec![0xFFu8, 0xFEu8];
        for u in header.encode_utf16() {
            raw.extend_from_slice(&u.to_le_bytes());
        }
        let decoded = decode_inf_text(&raw).to_ascii_lowercase();
        assert!(decoded.contains("nvidia"));
        assert!(decoded.contains("{4d36e968-e325-11ce-bfc1-08002be10318}"));
    }

    #[test]
    fn inf_diff_detects_new_oem() {
        let before: BTreeSet<String> = ["oem1.inf", "oem2.inf"].into_iter().map(String::from).collect();
        let after: BTreeSet<String> = ["oem1.inf", "oem2.inf", "oem42.inf"].into_iter().map(String::from).collect();
        let diff: Vec<String> = after.difference(&before).cloned().collect();
        assert_eq!(diff, vec!["oem42.inf"]);
    }
}
