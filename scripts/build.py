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
    # Try PATH
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

    # Locate tools
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

    # Locate original NVIDIA exe
    orig_exe = args.nvidia_exe
    if not orig_exe:
        # Search common directories
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

    # Certificates
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
        else:
            print("[*] Generating local self-signed certificate...")
            cert_script = os.path.join(repo_dir, "scripts", "create_cert.ps1")
            subprocess.run(["powershell", "-ExecutionPolicy", "Bypass", "-File", cert_script], check=True)
            pfx_path = default_pfx
            cer_path = os.path.join(repo_dir, "certs", "OpenFrankenDriver.cer")
    else:
        cer_path = os.path.splitext(pfx_path)[0] + ".cer"

    # Working and output paths
    base_name = os.path.splitext(os.path.basename(orig_exe))[0]
    work_dir = args.work_dir or os.path.join(repo_dir, "build_work")
    unpack_dir = os.path.join(work_dir, "unpacked")
    final_exe = args.output or os.path.join(repo_dir, f"{base_name}-openfrankendriver.exe")

    # Step 1: Clean & Extract
    print("\n[1/7] Extracting official NVIDIA package...")
    subprocess.run(["taskkill", "/f", "/im", "PackageLauncher.exe"], capture_output=True)
    if os.path.exists(work_dir):
        shutil.rmtree(work_dir, ignore_errors=True)
    os.makedirs(unpack_dir, exist_ok=True)

    subprocess.run([seven_zip, "x", orig_exe, f"-o{unpack_dir}", "-y"], check=True)

    # Expand nvngx_dlssd.dl_ if compressed
    dl_file = os.path.join(unpack_dir, "Display.Driver", "ngx-builtins", "nvngx_dlssd.dl_")
    dll_file = os.path.join(unpack_dir, "Display.Driver", "ngx-builtins", "nvngx_dlssd.dll")
    if os.path.exists(dl_file) and not os.path.exists(dll_file):
        print("[*] Expanding nvngx_dlssd.dl_ -> nvngx_dlssd.dll...")
        subprocess.run(["expand", dl_file, dll_file], check=True)

    # Remove laptop battery management driver NVPCF
    nvpcf_dir = os.path.join(unpack_dir, "NVPCF")
    if os.path.exists(nvpcf_dir):
        shutil.rmtree(nvpcf_dir, ignore_errors=True)

    # Step 2: Inject Mutant Models & Error 43 Fix
    print("\n[2/7] Injecting 68 mutant GPU IDs and RM1457588 (Error 43 bypass)...")
    inject_script = os.path.join(repo_dir, "scripts", "inject.py")
    inf_path = os.path.join(unpack_dir, "Display.Driver", "nv_dispi.inf")
    subprocess.run([sys.executable, inject_script, inf_path], check=True)

    # Step 3: Rebuild Security Catalog
    print("\n[3/7] Rebuilding security catalog (nv_disp.cat) recursively for all 271 driver files...")
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
        f_cdf.write("[CatalogHeader]\n")
        f_cdf.write("Name=nv_disp.cat\n")
        f_cdf.write("ResultDir=.\n\n")
        f_cdf.write("[CatalogFiles]\n")
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

    # Copy to EACSupport for Easy Anti-Cheat compatibility
    eac_dir = os.path.join(unpack_dir, "EACSupport")
    os.makedirs(eac_dir, exist_ok=True)
    shutil.copy2(cat_path, os.path.join(eac_dir, "nv_disp.cat"))

    # Step 4: Neutralize hostile removeCPLInstallData in DisplayDriver.nvi
    print("\n[4/7] Neutralizing removeCPLInstallData in DisplayDriver.nvi...")
    dd_nvi_path = os.path.join(driver_dir, "DisplayDriver.nvi")
    with open(dd_nvi_path, "r", encoding="utf-8") as f:
        dd_nvi_text = f.read()

    target_cpl_remove = '<custom phase="removeCPLInstallData" action="removeCPLInstallData" when="install"/>'
    if target_cpl_remove in dd_nvi_text:
        dd_nvi_text = dd_nvi_text.replace(target_cpl_remove, '<!-- ' + target_cpl_remove + ' -->')
        with open(dd_nvi_path, "w", encoding="utf-8") as f:
            f.write(dd_nvi_text)
        print("[+] removeCPLInstallData neutralized successfully.")

    # Step 5: Configure Subpackages & setup.cfg
    print("\n[5/7] Integrating standalone Win32 Control Panel and silent certificate package...")
    # 5.1 Certificate subpackage
    franken_pkg_dir = os.path.join(unpack_dir, "FrankenDriver")
    os.makedirs(franken_pkg_dir, exist_ok=True)
    shutil.copy2(cer_path, os.path.join(franken_pkg_dir, "FrankenDriver.cer"))
    shutil.copy2(os.path.join(repo_dir, "src", "FrankenDriver.nvi"), 
                 os.path.join(franken_pkg_dir, "FrankenDriver.nvi"))

    # 5.2 Classic NVIDIA Control Panel (Display.NVCPL) subpackage
    nvcpl_pkg_dir = os.path.join(unpack_dir, "Display.NVCPL")
    os.makedirs(nvcpl_pkg_dir, exist_ok=True)

    # Extract standalone Win32 binaries from NVCPL appx if needed
    appx_path = os.path.join(unpack_dir, "Display.Driver", "NVCPL", "7e010d7e2de54a439dbd340dd1f61212.appx")
    cpl_files = ['nvcplui.exe', 'nvcpluir.dll', 'nvImage.dll', 'NvGpuUtilization.exe', 'EULA.txt']
    if os.path.exists(appx_path):
        with zipfile.ZipFile(appx_path, 'r') as z:
            for name in cpl_files:
                z.extract(name, nvcpl_pkg_dir)

    # Copy native helper
    helper_exe = os.path.join(repo_dir, "src", "install_helper.exe")
    shutil.copy2(helper_exe, os.path.join(nvcpl_pkg_dir, "install_helper.exe"))

    # NVCPL.nvi definition
    nvcpl_nvi_content = """<?xml version="1.0" encoding="utf-8"?>
<nvi name="Display.NVCPL" title="${title}" version="${version}" versionText="${version}" timestamp="2026-11-06T00:00:00" disposition="default" hidden="false" installerMin="2.1002.158.1139" preserveCache="false">
\t<strings>
\t\t<string name="version" value="8.1.969.0"/>
\t\t<string name="AppPathsKey" value="HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\nvcplui.exe"/>
\t\t<string name="ContextMenuKey" value="HKEY_LOCAL_MACHINE\\SOFTWARE\\Classes\\DesktopBackground\\Shell\\NvControlPanel"/>
\t\t<localized locale="0x0409">
\t\t\t<string name="title" value="NVIDIA Control Panel (Classic)"/>
\t\t\t<string name="arpDisplayName" value="NVIDIA Control Panel (Classic)"/>
\t\t\t<string name="MenuTitle" value="NVIDIA Control Panel"/>
\t\t\t<string name="description" value="Installs the classic standalone Win32 NVIDIA Control Panel (nvcplui.exe) into Program Files."/>
\t\t</localized>
\t\t<localized locale="0x0419">
\t\t\t<string name="title" value="Панель управления NVIDIA (Классическая)"/>
\t\t\t<string name="arpDisplayName" value="Панель управления NVIDIA (Классическая)"/>
\t\t\t<string name="MenuTitle" value="Панель управления NVIDIA"/>
\t\t\t<string name="description" value="Устанавливает классическую автономную Win32 панель управления NVIDIA (nvcplui.exe) в Program Files."/>
\t\t</localized>
\t</strings>
\t<dependencies>
\t\t<package type="after" package="Display.Driver"/>
\t</dependencies>
\t<properties>
\t\t<bool name="FixedInstallLocation" value="true"/>
\t\t<string name="InstallLocation" value="${NvidiaProgramFiles}\\Control Panel Client"/>
\t\t<string name="ArpDisplayName" value="${arpDisplayName}"/>
\t\t<string name="ArpIcon" value="${InstallLocation}\\nvcplui.exe,0"/>
\t</properties>
\t<manifest>
\t\t<file name="NVCPL.nvi"/>
\t\t<file name="install_helper.exe"/>
\t\t<file name="nvcplui.exe"/>
\t\t<file name="nvcpluir.dll"/>
\t\t<file name="NvGpuUtilization.exe"/>
\t\t<file name="nvImage.dll"/>
\t\t<file name="EULA.txt"/>
\t</manifest>
\t<phases>
\t\t<standard phase="createInstallDir">
\t\t\t<createDirectory target="${InstallLocation}"/>
\t\t</standard>
\t\t<exe phase="deployControlPanelFiles" name="install_helper.exe" when="installClose" progress="120" autoTick="200">
\t\t\t<success value="0"/>
\t\t</exe>
\t\t<standard phase="RegisterAppPaths">
\t\t\t<addRegistry keyName="${AppPathsKey}" type="REG_SZ" valueName="" value="${InstallLocation}\\nvcplui.exe"/>
\t\t\t<addRegistry keyName="${AppPathsKey}" valueName="Path" type="REG_SZ" value="${InstallLocation}"/>
\t\t</standard>
\t\t<standard phase="RegisterContextMenu">
\t\t\t<addRegistry keyName="${ContextMenuKey}" type="REG_SZ" valueName="" value="${MenuTitle}"/>
\t\t\t<addRegistry keyName="${ContextMenuKey}" valueName="Icon" type="REG_SZ" value="${InstallLocation}\\nvcplui.exe,0"/>
\t\t\t<addRegistry keyName="${ContextMenuKey}\\command" type="REG_SZ" valueName="" value="&quot;${InstallLocation}\\nvcplui.exe&quot;"/>
\t\t\t<addRegistry keyName="HKLM\\SYSTEM\\CurrentControlSet\\Services\\nvlddmkm\\Parameters\\Global\\NVTweak" type="REG_DWORD" valueName="EnableNvcplUIOnDCH" value="1"/>
\t\t\t<addRegistry keyName="HKLM\\SOFTWARE\\NVIDIA Corporation\\Global\\NVTweak" type="REG_DWORD" valueName="EnableNvcplUIOnDCH" value="1"/>
\t\t</standard>
\t\t<exe phase="removeControlPanelFiles" name="install_helper.exe" when="uninstall" progress="120" autoTick="200">
\t\t\t<arg value="/uninstall"/>
\t\t\t<success value="0"/>
\t\t</exe>
\t\t<standard phase="CleanupOnUninstall" when="uninstall">
\t\t\t<deleteFile target="${InstallLocation}\\*.*"/>
\t\t\t<deleteDirectory target="${InstallLocation}"/>
\t\t\t<deleteRegistryKey keyName="${AppPathsKey}"/>
\t\t\t<deleteRegistryKey keyName="${ContextMenuKey}"/>
\t\t</standard>
\t</phases>
</nvi>
"""
    with open(os.path.join(nvcpl_pkg_dir, "NVCPL.nvi"), "w", encoding="utf-8") as f:
        f.write(nvcpl_nvi_content)

    # 5.3 setup.cfg bindings
    cfg_path = os.path.join(unpack_dir, "setup.cfg")
    with open(cfg_path, "r", encoding="utf-8") as f:
        cfg = f.read()

    cfg = cfg.replace('<sub-package name="NVPCF" />', '')

    if 'name="FrankenDriverCert"' not in cfg:
        franken_subpkgs = '\t\t<sub-package name="FrankenDriverCert" disposition="critical" hidden="true" />\n\t\t<sub-package name="EACSupportCatalog" disposition="optional" hidden="false" />\n\t\t<sub-package name="Display.Driver"'
        cfg = cfg.replace('\t\t<sub-package name="Display.Driver"', franken_subpkgs)

    target_disp = '<sub-package name="Display.Driver" disposition="critical">'
    dep_block = '<sub-package name="Display.Driver" disposition="critical">\n\t\t\t<dependencies><package package="FrankenDriverCert" type="after" /><package package="EACSupportCatalog" type="after" /></dependencies>'
    if 'package="FrankenDriverCert" type="after"' not in cfg:
        cfg = cfg.replace(target_disp, dep_block)

    if 'name="Display.NVCPL"' not in cfg:
        target_split = '</sub-package>\n\t\t<sub-package name="Display.NvApp"'
        nvcpl_entry = '''</sub-package>
\t\t<sub-package name="Display.NVCPL" disposition="default" hidden="false">
\t\t\t<dependencies>
\t\t\t\t<package type="after" package="Display.Driver"/>
\t\t\t\t<package type="after" package="Display.NvApp"/>
\t\t\t</dependencies>
\t\t</sub-package>
\t\t<sub-package name="Display.NvApp"'''
        cfg = cfg.replace(target_split, nvcpl_entry)

    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write(cfg)

    # Step 6: Monolithic Packaging
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

    temp_exe = os.path.join(work_dir, "unsigned.exe")
    with open(temp_exe, "wb") as f_out:
        f_out.write(clean_stub)
        with open(archive_7z, "rb") as f_in:
            while chunk := f_in.read(16 * 1024 * 1024):
                f_out.write(chunk)

    # Step 7: Digital Signature with Timestamp
    print("\n[7/7] Signing installer with RFC3161 timestamp...")
    sign_final = [
        signtool, "sign",
        "/f", pfx_path,
        "/p", args.pfx_pass,
        "/fd", "SHA256",
        "/tr", "http://timestamp.digicert.com",
        "/td", "SHA256",
        temp_exe
    ]
    subprocess.run(sign_final, check=True)

    for _ in range(10):
        try:
            if os.path.exists(final_exe):
                os.remove(final_exe)
            shutil.move(temp_exe, final_exe)
            break
        except Exception:
            time.sleep(1)

    shutil.rmtree(work_dir, ignore_errors=True)

    print("\n" + "=" * 75)
    print(f"[SUCCESS] OpenFrankenDriver successfully built at:\n--> {final_exe}")
    print(f"File size: {os.path.getsize(final_exe) / (1024*1024):.1f} MB")
    print("=" * 75)

if __name__ == "__main__":
    main()
