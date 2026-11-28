use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc_exe() -> Option<PathBuf> {
    if let Ok(p) = which_rc() {
        return Some(p);
    }
    let kits_root = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = fs::read_dir(kits_root) {
        let mut versions: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| p.join("x64").join("rc.exe").is_file())
            .collect();
        versions.sort();
        if let Some(latest) = versions.pop() {
            return Some(latest.join("x64").join("rc.exe"));
        }
    }
    None
}

fn which_rc() -> Result<PathBuf, ()> {
    if let Ok(out) = Command::new("where").arg("rc.exe").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(first) = s.lines().next() {
                let p = PathBuf::from(first.trim());
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    Err(())
}

fn main() {
    println!("cargo:rerun-if-changed=res/setup.rc");
    println!("cargo:rerun-if-changed=res/setup.manifest");
    println!("cargo:rerun-if-changed=res/app.ico");

    for lib in [
        "user32",
        "gdi32",
        "gdiplus",
        "advapi32",
        "shell32",
        "ole32",
        "setupapi",
        "cfgmgr32",
        "msi",
        "shlwapi",
        "comctl32",
        "version",
        "dwmapi",
    ] {
        println!("cargo:rustc-link-lib={lib}");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let res_dir = manifest_dir.join("res");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc_path = res_dir.join("setup.rc");
    let res_out = out_dir.join("setup.res");

    if let Some(rc_exe) = find_rc_exe() {
        let status = Command::new(&rc_exe)
            .args(["/nologo", "/fo"])
            .arg(&res_out)
            .arg(&rc_path)
            .current_dir(&res_dir)
            .status()
            .expect("failed to execute rc.exe");
        if status.success() && res_out.is_file() {
            println!("cargo:rustc-link-arg={}", res_out.display());
            return;
        }
    }
    panic!("rc.exe (Windows SDK) not found or failed compiling setup.rc");
}
