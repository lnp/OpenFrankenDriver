#!/usr/bin/env python3
"""
OpenFrankenDriver Master Build System
Automates extraction, INF patching (Error 43 + 68 Mutant GPUs), security catalog creation,
WHQL catalog signing (RFC3161 DigiCert timestamp), Classic Win32 Control Panel integration,
and monolithic self-extracting installer creation.
"""

import os
import sys
import time
import shutil
import zipfile
import subprocess
import argparse
import pefile

def find_tool(name, default_candidates):
    for candidate in default_candidates:
        if os.path.isfile(candidate):
            return candidate
    which_res = shutil.which(name)
    if which_res:
        return which_res
    return None

def main():
    parser = argparse.ArgumentParser(description="OpenFrankenDriver Automated Build System")
    parser.add_argument("--nvidia-exe", help="Path to official NVIDIA driver installer executable")
    parser.add_argument("--output", help="Path for the generated OpenFrankenDriver installer executable")
    parser.add_argument("--work-dir", help="Temporary working directory for build artifacts")
    parser.add_argument("--pfx", help="Path to code signing .pfx file")
    parser.add_argument("--pfx-pass", default="1234", help="Password for the .pfx certificate (default: 1234)")
    parser.add_argument("--seven-zip", help="Path to 7z.exe")
    parser.add_argument("--signtool", help="Path to signtool.exe")
    parser.add_argument("--makecat", help="Path to makecat.exe")
    args = parser.parse_args()

    repo_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    seven_zip = args.seven_zip or find_tool("7z.exe", [
        r"C:\Program Files\7-Zip\7z.exe",
        r"C:\Program Files (x86)\7-Zip\7z.exe"
    ])
    if not seven_zip:
        sys.exit("[-] Error: 7z.exe not found. Please install 7-Zip or pass --seven-zip.")

    signtool = args.signtool or find_tool("signtool.exe", [
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\signtool.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe"
    ])
    if not signtool:
        sys.exit("[-] Error: signtool.exe not found. Install Windows SDK or pass --signtool.")

    makecat = args.makecat or find_tool("makecat.exe", [
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\makecat.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\makecat.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\makecat.exe"
    ])
    if not makecat:
        sys.exit("[-] Error: makecat.exe not found. Install Windows SDK or pass --makecat.")

    orig_exe = args.nvidia_exe
    if not orig_exe:
        search_dirs = [repo_dir, os.path.dirname(repo_dir), os.getcwd()]
        for d in search_dirs:
            if not os.path.exists(d):
                continue
            for f in os.listdir(d):
                if (f.startswith("ORIGINAL_") or "desktop-win10-win11" in f.lower()) and f.endswith(".exe") and "frankendriver" not in f.lower():
                    orig_exe = os.path.join(d, f)
                    break
            if orig_exe:
                break

    if not orig_exe or not os.path.isfile(orig_exe):
        sys.exit("[-] Error: Official NVIDIA installer .exe not found. Pass --nvidia-exe.")

    print("=" * 75)
    print(" OpenFrankenDriver - Open-Source GPU Driver Master Builder")
    print("=" * 75)
    print(f"[*] Original NVIDIA Installer: {orig_exe}")

    pfx_path = args.pfx
    cer_path = None
    if not pfx_path:
        default_pfx = os.path.join(repo_dir, "certs", "OpenFrankenDriver.pfx")
        parent_pfx = os.path.join(os.path.dirname(repo_dir), "FrankenDriver_Local.pfx")
        if os.path.exists(parent_pfx):
            pfx_path = parent_pfx
            cer_path = os.path.join(os.path.dirname(repo_dir), "FrankenDriver_Local.cer")
        elif os.path.exists(default_pfx):
            pfx_path = default_pfx
            cer_path = os.path.join(repo_dir, "certs", "OpenFrankenDriver.cer")

    if not pfx_path or not os.path.isfile(pfx_path):
        sys.exit("[-] Error: Code signing .pfx certificate not found. Pass --pfx.")

    work_dir = args.work_dir or os.path.join(repo_dir, "build_temp")
    os.makedirs(work_dir, exist_ok=True)
    unpack_dir = os.path.join(work_dir, "driver_unpacked")

    print(f"\n[1/7] Extracting official NVIDIA installer to: {unpack_dir}")
    if os.path.exists(unpack_dir):
        shutil.rmtree(unpack_dir)
    os.makedirs(unpack_dir, exist_ok=True)

    cmd_extract = [seven_zip, "x", orig_exe, f"-o{unpack_dir}", "-y"]
    subprocess.run(cmd_extract, check=True)

    print("\n[2/7] Patching nv_dispi.inf with Mutant GPUs database...")
    target_inf = os.path.join(unpack_dir, "Display.Driver", "nv_dispi.inf")
    inject_script = os.path.join(repo_dir, "scripts", "inject.py")
    subprocess.run([sys.executable, inject_script, target_inf], check=True)

    print("\n[3/7] Generating and signing security catalog (nv_disp.cat)...")
    driver_dir = os.path.join(unpack_dir, "Display.Driver")
    cdf_path = os.path.join(driver_dir, "nv_disp.cdf")
    cat_path = os.path.join(driver_dir, "nv_disp.cat")

    all_files = []
    for root, dirs, files in os.walk(driver_dir):
        for f in files:
            if f.lower() in ["nv_disp.cat", "nv_disp.cdf", "makecat.exe"]:
                continue
            rel = os.path.relpath(os.path.join(root, f), driver_dir)
            all_files.append((f, rel))

    with open(cdf_path, "w", encoding="ascii") as f_cdf:
        f_cdf.write("[CatalogHeader]\nName=nv_disp.cat\nResultDir=.\n\n[CatalogFiles]\n")
        for fname, relpath in sorted(all_files):
            f_cdf.write(f"<HASH>{fname}={relpath}\n")

    subprocess.run([makecat, "-v", "nv_disp.cdf"], cwd=driver_dir, check=True)

    print("[*] Signing nv_disp.cat with RFC3161 DigiCert timestamp...")
    cmd_sign_cat = [
        signtool, "sign",
        "/f", pfx_path,
        "/p", args.pfx_pass,
        "/fd", "SHA256",
        "/tr", "http://timestamp.digicert.com",
        "/td", "SHA256",
        cat_path
    ]
    subprocess.run(cmd_sign_cat, check=True)

    eac_dir = os.path.join(unpack_dir, "EACSupport")
    os.makedirs(eac_dir, exist_ok=True)
    shutil.copy2(cat_path, os.path.join(eac_dir, "nv_disp.cat"))

    print("\n[4/7] Neutralizing NVI2 installer signature verification...")
    nvi2_dll = os.path.join(unpack_dir, "NVI2", "NVI2.dll")
    with open(nvi2_dll, "rb") as f:
        dll_bytes = f.read()

    pos1 = dll_bytes.find(bytes.fromhex("E8 D2 85 1B 00"))
    if pos1 != -1 and dll_bytes[pos1 - 6] == 0x74:
        dll_bytes = dll_bytes[:pos1 - 6] + b"\xEB" + dll_bytes[pos1 - 5:]
        print("[+] Patched WinVerifyTrust early-exit jump.")

    pos2 = dll_bytes.find(bytes.fromhex("55 8B EC 83 EC 14 53 56 8B 75 08 85 F6 57"))
    if pos2 != -1:
        patch_stub = bytes.fromhex("31 C0 40 C2 04 00")
        dll_bytes = dll_bytes[:pos2] + patch_stub + dll_bytes[pos2 + len(patch_stub):]
        print("[+] Patched signature validator stub.")

    with open(nvi2_dll, "wb") as f:
        f.write(dll_bytes)

    print("\n[5/7] Integrating custom subpackages...")
    franken_pkg_dir = os.path.join(unpack_dir, "FrankenDriver")
    os.makedirs(franken_pkg_dir, exist_ok=True)
    shutil.copy2(cer_path, os.path.join(franken_pkg_dir, "FrankenDriver.cer"))
    shutil.copy2(os.path.join(repo_dir, "src", "FrankenDriver.nvi"), 
                 os.path.join(franken_pkg_dir, "FrankenDriver.nvi"))

    nvcpl_pkg_dir = os.path.join(unpack_dir, "Display.NVCPL")
    os.makedirs(nvcpl_pkg_dir, exist_ok=True)

    appx_path = os.path.join(unpack_dir, "Display.Driver", "NVCPL", "7e010d7e2de54a439dbd340dd1f61212.appx")
    cpl_files = ['nvcplui.exe', 'nvcpluir.dll', 'nvImage.dll', 'NvGpuUtilization.exe', 'EULA.txt']
    if os.path.exists(appx_path):
        with zipfile.ZipFile(appx_path, 'r') as z:
            for name in cpl_files:
                z.extract(name, nvcpl_pkg_dir)

    helper_exe = os.path.join(repo_dir, "src", "install_helper.exe")
    shutil.copy2(helper_exe, os.path.join(nvcpl_pkg_dir, "install_helper.exe"))

    cfg_path = os.path.join(unpack_dir, "setup.cfg")
    with open(cfg_path, "r", encoding="utf-8") as f:
        cfg = f.read()

    cfg = cfg.replace('<sub-package name="NVPCF" />', '')

    if 'name="FrankenDriverCert"' not in cfg:
        franken_subpkgs = '\t\t<sub-package name="FrankenDriverCert" disposition="default" hidden="true" />\n\t\t<sub-package name="EACSupportCatalog" disposition="optional" hidden="false" />\n\t\t<sub-package name="Display.Driver"'
        cfg = cfg.replace('\t\t<sub-package name="Display.Driver"', franken_subpkgs)

    target_disp = '<sub-package name="Display.Driver" disposition="critical">'
    dep_block = '<sub-package name="Display.Driver" disposition="critical">\n\t\t\t<dependencies><package package="FrankenDriverCert" type="after" /><package package="EACSupportCatalog" type="after" /></dependencies>'
    if 'package="FrankenDriverCert" type="after"' not in cfg:
        cfg = cfg.replace(target_disp, dep_block)

    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write(cfg)

    print("\n[6/7] Building monolithic self-extracting archive...")
    with open(orig_exe, "rb") as f:
        orig_stub = f.read(1120292)

    pe_part = orig_stub[:1118720]
    pe = pefile.PE(data=pe_part)
    sec_idx = pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_SECURITY']
    pe.OPTIONAL_HEADER.DATA_DIRECTORY[sec_idx].VirtualAddress = 0
    pe.OPTIONAL_HEADER.DATA_DIRECTORY[sec_idx].Size = 0
    clean_pe = pe.write()

    config_part = orig_stub[1118720:1120292]
    clean_stub = clean_pe + config_part

    archive_7z = os.path.join(work_dir, "payload.7z")
    cmd_7z = [
        seven_zip, "a", archive_7z,
        os.path.join(unpack_dir, "*"),
        "-m0=LZMA2:d26",
        "-mf=off",
        "-mx=9",
        "-ms=on",
        "-mmt=16"
    ]
    subprocess.run(cmd_7z, check=True)

    out_exe = args.output or os.path.join(os.path.dirname(repo_dir), "OpenFrankenDriver_621.14_WHQL.exe")
    temp_exe = os.path.join(work_dir, "unsigned.exe")
    with open(temp_exe, "wb") as f_out:
        f_out.write(clean_stub)
        with open(archive_7z, "rb") as f_in:
            while chunk := f_in.read(16 * 1024 * 1024):
                f_out.write(chunk)

    print("\n[7/7] Signing final monolithic installer...")
    cmd_sign_exe = [
        signtool, "sign",
        "/f", pfx_path,
        "/p", args.pfx_pass,
        "/fd", "SHA256",
        "/tr", "http://timestamp.digicert.com",
        "/td", "SHA256",
        temp_exe
    ]
    subprocess.run(cmd_sign_exe, check=True)

    if os.path.exists(out_exe):
        os.remove(out_exe)
    shutil.move(temp_exe, out_exe)
    shutil.rmtree(work_dir, ignore_errors=True)

    print("=" * 75)
    print(f"[SUCCESS] Monolithic OpenFrankenDriver ready at:\n--> {out_exe}")
    print("=" * 75)

if __name__ == "__main__":
    main()
