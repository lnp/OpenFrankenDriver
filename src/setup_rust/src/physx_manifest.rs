//! Data-driven NVIDIA PhysX installer & uninstaller backed by `PhysX\physx_manifest.json`.
//! Handles BOTH 32-bit WOW6432Node and native 64-bit registry views exactly like PhysX.nvi + NVI2.

use std::fs;
use std::path::Path;

use crate::json_mini::{self, JsonValue};
use crate::win32::{self, RegView};

#[derive(Debug, Clone)]
pub struct CopyItem {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct RegEntry {
    pub view: String,
    pub key: String,
    pub value_name: String,
    pub reg_type: String,
    pub value: String,
    pub split: String,
}

#[derive(Debug, Clone)]
pub struct PhysxManifest {
    pub version: String,
    pub upgrade_codes: Vec<String>,
    pub copy_files: Vec<CopyItem>,
    pub registries: Vec<RegEntry>,
    pub path_add: String,
}

pub fn parse_manifest(json_text: &str) -> Result<PhysxManifest, String> {
    let root = json_mini::parse(json_text)?;
    let version = root.get_str("version").unwrap_or("9.26.0904").to_string();
    let path_add = root.get_str("path_add").unwrap_or("").to_string();

    let upgrade_codes = root
        .get("upgrade_codes")
        .and_then(|v| v.as_arr())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let mut copy_files = Vec::new();
    if let Some(arr) = root.get("copy_files").and_then(|v| v.as_arr()) {
        for item in arr {
            if let (Some(src), Some(dst)) = (item.get_str("source"), item.get_str("target")) {
                copy_files.push(CopyItem {
                    source: src.to_string(),
                    target: dst.to_string(),
                });
            }
        }
    }

    let mut registries = Vec::new();
    if let Some(arr) = root.get("registries").and_then(|v| v.as_arr()) {
        for item in arr {
            registries.push(parse_reg_entry(item));
        }
    }

    Ok(PhysxManifest {
        version,
        upgrade_codes,
        copy_files,
        registries,
        path_add,
    })
}

fn parse_reg_entry(v: &JsonValue) -> RegEntry {
    RegEntry {
        view: v.get_str("view").unwrap_or("x86").to_string(),
        key: v.get_str("key").unwrap_or("").to_string(),
        value_name: v.get_str("value_name").unwrap_or("").to_string(),
        reg_type: v.get_str("type").unwrap_or("").to_string(),
        value: v.get_str("value").unwrap_or("").to_string(),
        split: v.get_str("split").unwrap_or("|").to_string(),
    }
}

pub fn physx_install_root() -> std::path::PathBuf {
    win32::program_files_x86().join("NVIDIA Corporation").join("PhysX")
}

pub fn expand_physx_placeholders(s: &str, install_loc: &Path) -> String {
    let loc = install_loc.to_string_lossy();
    s.replace("${InstallLocation}", &loc)
}

/// Install PhysX files, dual WOW6432/Native64 registry keys, and System PATH from manifest.
pub fn install(pkg_root: &Path) -> Result<(), String> {
    let physx_dir = pkg_root.join("PhysX");
    let manifest_path = physx_dir.join("physx_manifest.json");
    let txt = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Не найден {}: {e}", manifest_path.display()))?;
    let manifest = parse_manifest(&txt)?;

    let codes: Vec<&str> = manifest.upgrade_codes.iter().map(|s| s.as_str()).collect();
    if !codes.is_empty() {
        win32::uninstall_msi_by_upgrade_codes(&codes);
    }

    let dst_root = physx_install_root();
    fs::create_dir_all(&dst_root).map_err(|e| format!("Ошибка создания папки PhysX: {e}"))?;

    for item in &manifest.copy_files {
        let src = physx_dir.join(&item.source);
        let dst = dst_root.join(&item.target);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Ошибка создания {}: {e}", parent.display()))?;
        }
        if src.is_file() {
            fs::copy(&src, &dst).map_err(|e| format!("Не удалось скопировать {}: {e}", item.source))?;
        } else {
            win32::log_line(&format!("Warning: PhysX file missing in package: {}", src.display()));
        }
    }

    for entry in &manifest.registries {
        let view = if entry.view.eq_ignore_ascii_case("x64") {
            RegView::Native64
        } else {
            RegView::Wow32
        };
        let key = expand_physx_placeholders(&entry.key, &dst_root);
        if entry.reg_type.is_empty() {
            win32::reg_ensure_key(&key, view)?;
            continue;
        }
        let val = expand_physx_placeholders(&entry.value, &dst_root);
        match entry.reg_type.as_str() {
            "REG_DWORD" => {
                let num: u32 = val.parse().unwrap_or(0);
                win32::reg_set_dword(&key, &entry.value_name, num, view)?;
            }
            "REG_MULTI_SZ" => {
                let delim = if entry.split.is_empty() { "|" } else { &entry.split };
                let parts: Vec<&str> = val.split(delim).filter(|p| !p.is_empty()).collect();
                win32::reg_set_multi_sz(&key, &entry.value_name, &parts, view)?;
            }
            _ => {
                win32::reg_set_string(&key, &entry.value_name, &val, view)?;
            }
        }
    }

    if !manifest.path_add.is_empty() {
        let target_dir = expand_physx_placeholders(&manifest.path_add, &dst_root);
        append_system_path(&target_dir);
    }

    if !dst_root.join("Common").join("PhysXLoader.dll").is_file() {
        return Err("Confirm-install PhysX не пройден: отсутствует PhysXLoader.dll".to_string());
    }

    win32::log_line(&format!("PhysX {} installed successfully ({} files).", manifest.version, manifest.copy_files.len()));
    Ok(())
}

pub fn uninstall() {
    let dst_root = physx_install_root();
    remove_from_system_path(&dst_root.join("Common").to_string_lossy());

    for view in [RegView::Wow32, RegView::Native64] {
        win32::reg_delete_tree(r"HKLM\SOFTWARE\AGEIA Technologies", view);
        win32::reg_delete_tree(r"HKLM\SOFTWARE\NVIDIA Corporation\PhysX_SystemSoftware", view);
        win32::reg_delete_tree(r"HKLM\SOFTWARE\NVIDIA Corporation\PhysXUpdateLoader", view);
    }

    if dst_root.is_dir() {
        let _ = fs::remove_dir_all(&dst_root);
    }
    win32::log_line("PhysX removed (files, x86/x64 registry, PATH).");
}

const ENV_KEY: &str = r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

pub fn append_system_path(entry: &str) {
    let current = win32::reg_read_string(ENV_KEY, "Path", RegView::Native64).unwrap_or_default();
    let updated = add_path_segment(&current, entry);
    if updated != current {
        let _ = win32::reg_set_expand_string(ENV_KEY, "Path", &updated, RegView::Native64);
        win32::broadcast_env_change();
    }
}

pub fn remove_from_system_path(entry: &str) {
    let current = win32::reg_read_string(ENV_KEY, "Path", RegView::Native64).unwrap_or_default();
    let updated = remove_path_segment(&current, entry);
    if updated != current {
        let _ = win32::reg_set_expand_string(ENV_KEY, "Path", &updated, RegView::Native64);
        win32::broadcast_env_change();
    }
}

pub fn add_path_segment(current: &str, seg: &str) -> String {
    let clean_seg = seg.trim().trim_end_matches('\\');
    let exists = current
        .split(';')
        .any(|p| p.trim().trim_end_matches('\\').eq_ignore_ascii_case(clean_seg));
    if exists || clean_seg.is_empty() {
        return current.to_string();
    }
    if current.trim().is_empty() {
        seg.to_string()
    } else if current.ends_with(';') {
        format!("{current}{seg}")
    } else {
        format!("{current};{seg}")
    }
}

pub fn remove_path_segment(current: &str, seg: &str) -> String {
    let clean_seg = seg.trim().trim_end_matches('\\');
    let kept: Vec<&str> = current
        .split(';')
        .filter(|p| {
            let t = p.trim();
            !t.is_empty() && !t.trim_end_matches('\\').eq_ignore_ascii_case(clean_seg)
        })
        .collect();
    kept.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_add_and_remove_are_idempotent() {
        let p = add_path_segment(r"C:\Windows;C:\Tools", r"C:\Program Files (x86)\NVIDIA Corporation\PhysX\Common");
        assert_eq!(p, r"C:\Windows;C:\Tools;C:\Program Files (x86)\NVIDIA Corporation\PhysX\Common");
        assert_eq!(
            add_path_segment(&p, r"C:\Program Files (x86)\NVIDIA Corporation\PhysX\Common\"),
            p
        );
        assert_eq!(
            remove_path_segment(&p, r"C:\Program Files (x86)\NVIDIA Corporation\PhysX\Common"),
            r"C:\Windows;C:\Tools"
        );
    }

    #[test]
    fn parses_physx_manifest_dual_views() {
        let sample = r#"{
          "version": "9.26.0904",
          "upgrade_codes": ["{C40CFCD4-C757-4139-A4DA-7CB51A8DBF80}"],
          "copy_files": [{"source": "files\\Common\\PhysXLoader.dll", "target": "Common\\PhysXLoader.dll"}],
          "registries": [
            {"view": "x86", "key": "HKLM\\SOFTWARE\\AGEIA Technologies", "value_name": "HwSelection", "type": "REG_SZ", "value": "GPU"},
            {"view": "x64", "key": "HKLM\\SOFTWARE\\AGEIA Technologies\\PhysX_A64_Engines", "value_name": "2.8.3", "type": "REG_DWORD", "value": "44"}
          ],
          "path_add": "${InstallLocation}\\Common"
        }"#;
        let m = parse_manifest(sample).unwrap();
        assert_eq!(m.registries[0].view, "x86");
        assert_eq!(m.registries[1].view, "x64");
        assert_eq!(m.upgrade_codes.len(), 1);
    }
}
