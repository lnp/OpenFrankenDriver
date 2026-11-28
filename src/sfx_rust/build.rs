use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc_exe() -> Option<PathBuf> {
    if let Ok(out) = Command::new("where").arg("rc.exe").output() {
        if out.status.success() {
            if let Some(first) = String::from_utf8_lossy(&out.stdout).lines().next() {
                let p = PathBuf::from(first.trim());
                if p.is_file() {
                    return Some(p);
                }
            }
        }
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

fn main() {
    println!("cargo:rerun-if-changed=res/sfx.rc");
    println!("cargo:rerun-if-changed=res/sfx.manifest");
    println!("cargo:rerun-if-changed=res/app.ico");

    for lib in ["user32", "gdi32", "shell32", "dwmapi", "advapi32"] {
        println!("cargo:rustc-link-lib={lib}");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let res_dir = manifest_dir.join("res");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc_path = res_dir.join("sfx.rc");
    let res_out = out_dir.join("sfx.res");

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
    panic!("rc.exe (Windows SDK) not found or failed compiling sfx.rc");
}
