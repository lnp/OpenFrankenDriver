//! Native COM IShellLinkW shortcut creation & cleanup (matching install_helper.c).

use std::ffi::c_void;
use std::fs;
use std::path::Path;

use crate::win32::{self, to_wide, GUID};

const CLSID_SHELL_LINK: GUID = GUID {
    data1: 0x00021401,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_ISHELL_LINK_W: GUID = GUID {
    data1: 0x000214F9,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_IPERSIST_FILE: GUID = GUID {
    data1: 0x0000010B,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

extern "system" {
    fn CoInitializeEx(pvReserved: *const c_void, dwCoInit: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const GUID,
        pUnkOuter: *mut c_void,
        dwClsContext: u32,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32;
}

unsafe fn vtbl_slot(obj: *mut c_void, idx: usize) -> usize {
    let vtbl = *(obj as *const *const usize);
    *vtbl.add(idx)
}

pub fn create_shortcut(
    lnk_path: &Path,
    target_path: &Path,
    work_dir: &Path,
    icon_path: &Path,
    desc: &str,
) -> Result<(), String> {
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const CLSCTX_INPROC_SERVER: u32 = 0x1;

    unsafe {
        let hr_init = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
        let did_init = hr_init >= 0;

        let mut psl: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_SHELL_LINK,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_ISHELL_LINK_W,
            &mut psl,
        );
        if hr < 0 || psl.is_null() {
            if did_init {
                CoUninitialize();
            }
            return Err(format!("CoCreateInstance(ShellLink) failed: 0x{hr:08X}"));
        }

        type FnSetStr = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
        type FnSetIcon = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
        type FnQi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32;
        type FnRelease = unsafe extern "system" fn(*mut c_void) -> u32;
        type FnSave = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;

        let set_desc: FnSetStr = std::mem::transmute(vtbl_slot(psl, 7)); // IShellLinkW::SetDescription
        let set_work: FnSetStr = std::mem::transmute(vtbl_slot(psl, 9)); // IShellLinkW::SetWorkingDirectory
        let set_icon: FnSetIcon = std::mem::transmute(vtbl_slot(psl, 17)); // IShellLinkW::SetIconLocation
        let set_path: FnSetStr = std::mem::transmute(vtbl_slot(psl, 20)); // IShellLinkW::SetPath
        let qi: FnQi = std::mem::transmute(vtbl_slot(psl, 0));
        let release: FnRelease = std::mem::transmute(vtbl_slot(psl, 2));

        let w_target = to_wide(&target_path.to_string_lossy());
        let w_work = to_wide(&work_dir.to_string_lossy());
        let w_icon = to_wide(&icon_path.to_string_lossy());
        let w_desc = to_wide(desc);

        set_path(psl, w_target.as_ptr());
        set_work(psl, w_work.as_ptr());
        set_desc(psl, w_desc.as_ptr());
        set_icon(psl, w_icon.as_ptr(), 0);

        let mut ppf: *mut c_void = std::ptr::null_mut();
        let hr_qi = qi(psl, &IID_IPERSIST_FILE, &mut ppf);
        let mut hr_save = hr_qi;
        if hr_qi >= 0 && !ppf.is_null() {
            let save: FnSave = std::mem::transmute(vtbl_slot(ppf, 6)); // IPersistFile::Save
            let pf_release: FnRelease = std::mem::transmute(vtbl_slot(ppf, 2));
            let _ = fs::remove_file(lnk_path);
            let w_lnk = to_wide(&lnk_path.to_string_lossy());
            hr_save = save(ppf, w_lnk.as_ptr(), 1);
            pf_release(ppf);
        }
        release(psl);

        if did_init {
            CoUninitialize();
        }
        if hr_save < 0 {
            return Err(format!("IPersistFile::Save failed: 0x{hr_save:08X}"));
        }
        win32::sh_notify_path(lnk_path);
        Ok(())
    }
}

fn remove_shortcuts_where<F: Fn(&str) -> bool>(dir: &Path, pred: F) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if !p.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
            continue;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        if pred(&name) {
            let _ = fs::remove_file(&p);
            win32::sh_notify_path(&p);
        }
    }
}

pub fn remove_nvcpl_shortcuts(dir: &Path) {
    remove_shortcuts_where(dir, |name| {
        name.contains("NVIDIA Control")
            || name.contains("NVCPL")
            || name.contains("Панель")
            || name.contains("Удалить")
            || name.contains("Рџ")
            || name.contains("РЈ")
    });
}

pub fn remove_matching_shortcuts(dir: &Path) {
    remove_shortcuts_where(dir, |name| {
        name.contains("NVIDIA Control")
            || name.contains("Profile Inspector")
            || name.contains("NVCPL")
            || name.contains("Панель")
            || name.contains("Удалить")
            || name.contains("Рџ")
            || name.contains("РЈ")
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_valid_windows_lnk_file() {
        let tmp = std::env::temp_dir().join(format!("ofd_lnk_test_{}.lnk", std::process::id()));
        let _ = fs::remove_file(&tmp);
        let cmd = win32::system_directory().join("cmd.exe");
        let work = win32::system_directory();
        create_shortcut(&tmp, &cmd, &work, &cmd, "OFD Shortcut Test").expect("create_shortcut failed");
        let meta = fs::metadata(&tmp).expect(".lnk missing on disk");
        assert!(meta.len() > 100, "unexpected small .lnk size: {}", meta.len());
        let _ = fs::remove_file(&tmp);
    }
}
