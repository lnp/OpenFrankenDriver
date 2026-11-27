#!/usr/bin/env python3
"""
OpenFrankenDriver Master Build System.

Automates:
1. Official NVIDIA driver extraction
2. Dynamic INF patching (Error 43 bypass + mutant GPU HWIDs)
3. Native debloat & Control Panel protection in DisplayDriver.nvi
4. Optional --slim trimming of NVIDIA App / telemetry bloat (preserving EULA/manifests)
5. Recursive security catalog generation (nv_disp.cat) & RFC3161 signing
6. Auto-compilation of install_helper.c (with prebuilt fallback)
7. Comprehensive pre-pack & post-sign smoke checks
8. Monolithic SFX assembly with dynamic PE/stub boundary detection
"""

import argparse
from collections import Counter
import glob
import os
import re
import shutil
import struct
import subprocess
import sys
import zipfile

try:
    import pefile
except ImportError:
    pefile = None


TIMESTAMP_URL = "http://timestamp.digicert.com"

SLIM_REMOVE_PATHS = [
    "App",
    "App.exe",
    "Overlay",
    "Overlay.exe",
    "ShadowPlay",
    "FrameViewSDK",
    "NvApp.MessageBus",
    "NvBackend",
    "NvCpl",
    "NvDLISR",
    "NvTelemetry",
    "NvContainer",
    "NvVAD",
    "PPC",
    "NVPCF",
]

SLIM_KEEP_NVAPP_ENTRIES = {
    "privacypolicy",
    "unified_eula",
    "eula",
    "upgradeconsent",
    "eula.txt",
}

SLIM_SUBPACKAGES = [
    "Display.NvApp",
    "NvContainer",
    "NvContainer.LocalSystem",
    "NvContainer.Session",
    "NvContainer.User",
    "NvPlugin.Watchdog",
    "VirtualAudio.Driver",
    "ShadowPlay",
    "FrameViewSdk",
    "Display.NvApp.MessageBus",
    "Display.NvApp.NvBackend",
    "Display.NvApp.NvCPL",
    "NvDLISR",
    "NvTelemetry",
]


def find_tool(name, default_candidates):
    """Locate a required Windows/build tool by candidate paths or PATH."""
    for candidate in default_candidates:
        if os.path.isfile(candidate):
            return candidate
    return shutil.which(name)


def resolve_signing_certs(args, repo_dir):
    """Locate .pfx/.cer signing certificates, auto-generating local cert if omitted."""
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
            create_script = os.path.join(repo_dir, "scripts", "create_cert.ps1")
            if os.path.isfile(create_script):
                print("[*] No local .pfx found; generating self-signed certificate via create_cert.ps1...")
                subprocess.run(
                    ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", create_script, "-Password", args.pfx_pass],
                    check=True,
                )
                if os.path.isfile(default_pfx):
                    pfx_path = default_pfx
                    cer_path = os.path.join(repo_dir, "certs", "OpenFrankenDriver.cer")

    if not pfx_path or not os.path.isfile(pfx_path):
        sys.exit("[-] Error: Code signing .pfx certificate not found. Pass --pfx.")

    if not cer_path or not os.path.isfile(cer_path):
        cand = os.path.splitext(pfx_path)[0] + ".cer"
        if os.path.isfile(cand):
            cer_path = cand
        elif os.path.isfile(os.path.join(repo_dir, "certs", "OpenFrankenDriver.cer")):
            cer_path = os.path.join(repo_dir, "certs", "OpenFrankenDriver.cer")
        elif os.path.isfile(os.path.join(os.path.dirname(repo_dir), "FrankenDriver_Local.cer")):
            cer_path = os.path.join(os.path.dirname(repo_dir), "FrankenDriver_Local.cer")
        else:
            sys.exit("[-] Error: Matching .cer certificate not found.")

    return pfx_path, cer_path


def compile_install_helper(repo_dir, out_dir):
    """Compile src/install_helper.c using MSVC or GCC if available; fall back to prebuilt EXE."""
    src_dir = os.path.join(repo_dir, "src")
    c_src = os.path.join(src_dir, "install_helper.c")
    prebuilt_exe = os.path.join(src_dir, "install_helper.exe")
    compiled_exe = os.path.join(out_dir, "install_helper.exe")

    if os.path.isfile(c_src):
        # 1. Try Visual Studio vcvars64.bat + cl.exe
        vcvars_candidates = [
            r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
            r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
            r"C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
            r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat",
            r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat",
        ]
        vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
        if os.path.isfile(vswhere):
            res = subprocess.run(
                [vswhere, "-latest", "-products", "*", "-property", "installationPath"],
                capture_output=True, text=True
            )
            vs_path = res.stdout.strip()
            if vs_path:
                vcvars_candidates.insert(0, os.path.join(vs_path, "VC", "Auxiliary", "Build", "vcvars64.bat"))

        vcvars = next((p for p in vcvars_candidates if os.path.isfile(p)), None)
        cl_in_path = shutil.which("cl")

        if vcvars or cl_in_path:
            obj_file = os.path.join(out_dir, "install_helper.obj")
            bat_file = os.path.join(out_dir, "_compile_helper.bat")
            cl_line = (
                f'cl /nologo /utf-8 /O2 /MT /GS- /Fo"{obj_file}" /Fe"{compiled_exe}" "{c_src}" '
                'shell32.lib ole32.lib advapi32.lib /link /SUBSYSTEM:WINDOWS\n'
            )
            with open(bat_file, "w", encoding="utf-8") as bf:
                bf.write("@echo off\n")
                if vcvars and not cl_in_path:
                    bf.write(f'call "{vcvars}" >nul\n')
                bf.write(cl_line)
            res = subprocess.run(["cmd", "/c", bat_file], capture_output=True, text=True)
            for tmp in (obj_file, bat_file):
                if os.path.exists(tmp):
                    os.remove(tmp)
            if res.returncode == 0 and os.path.isfile(compiled_exe):
                print("[+] Compiled fresh install_helper.exe from src/install_helper.c (MSVC).")
                return compiled_exe
            print(f"[!] Warning: MSVC compilation failed (rc={res.returncode}), falling back to prebuilt binary.")

        # 2. Try MinGW gcc
        gcc = shutil.which("gcc")
        if gcc:
            res = subprocess.run(
                [gcc, "-O2", "-municode", "-mwindows", c_src, "-o", compiled_exe, "-lshell32", "-lole32", "-luuid", "-ladvapi32"],
                capture_output=True, text=True
            )
            if res.returncode == 0 and os.path.isfile(compiled_exe):
                print("[+] Compiled fresh install_helper.exe from src/install_helper.c (GCC).")
                return compiled_exe

    if not os.path.isfile(prebuilt_exe):
        sys.exit("[-] Error: Prebuilt src/install_helper.exe not found and no compiler succeeded.")
    shutil.copy2(prebuilt_exe, compiled_exe)
    print("[*] Using prebuilt src/install_helper.exe (fallback).")
    return compiled_exe


def patch_display_driver_nvi(unpack_dir, keep_telemetry):
    """Disable removeCPLInstallData / Store AppX DISM install and optionally inject _nvtopps SSD protection."""
    target_nvi = os.path.join(unpack_dir, "Display.Driver", "DisplayDriver.nvi")
    if not os.path.isfile(target_nvi):
        return

    with open(target_nvi, "r", encoding="utf-8") as f:
        nvi = f.read()

    # Neutralize DisplayDriverExt.dll's forced removal of nvcplui.exe
    nvi_updated = re.sub(
        r'(?<!<!--)([ \t]*<custom\s+phase="removeCPLInstallData"[^>]*/>)',
        r'<!-- OpenFrankenDriver: \1 -->',
        nvi
    )
    if nvi_updated != nvi:
        print("[+] Disabled removeCPLInstallData phase in DisplayDriver.nvi")
        nvi = nvi_updated

    # Remove Store NVCPL AppX manifest entries and Dism.exe provisioning phase (classic Win32 Display.NVCPL replaces it)
    nvi_no_appx = re.sub(r'<file\s+name="NVCPL\\[^"]+"\s*[^>]*/>', "", nvi)
    nvi_no_appx = re.sub(
        r'[ \t]*<exe\s+phase="cplAppxPackageInstall"[^>]*>.*?</exe>\r?\n?',
        "",
        nvi_no_appx,
        flags=re.DOTALL,
    )
    if nvi_no_appx != nvi:
        print("[+] Removed Store AppX manifest entries and Dism.exe cplAppxPackageInstall phase")
        nvi = nvi_no_appx

    if not keep_telemetry:
        ssd_fix_phase = (
            '\t\t<standard phase="disableNvtoppsSSDBug" onError="ignore" when="install">\n'
            '\t\t\t<addRegistry keyName="${{serviceHiveRegistryPath}}\\${{DisplayContainerServiceName}}\\Parameters\\LocalSystem\\_nvtopps" valueName="DisableLoad" value="1" type="REG_DWORD"/>\n'
            '\t\t\t<addRegistry keyName="${{serviceHiveRegistryPath}}\\${{DisplayContainerServiceName}}\\Parameters\\LocalSystem\\NvTopps" valueName="DisableLoad" value="1" type="REG_DWORD"/>\n'
            '\t\t</standard>\n'
        )
        if "disableNvtoppsSSDBug" not in nvi:
            nvi = nvi.replace("<phases>", "<phases>\n" + ssd_fix_phase, 1)
            print("[+] Injected native telemetry debloat & SSD wear protection into DisplayDriver.nvi")
    else:
        print("[*] Skipping telemetry debloat (--keep-telemetry specified).")

    with open(target_nvi, "w", encoding="utf-8") as f:
        f.write(nvi)


def generate_security_catalog(unpack_dir, makecat, signtool, pfx_path, pfx_pass):
    """Expand compressed payloads temporarily, build nv_disp.cat, and sign with RFC3161."""
    driver_dir = os.path.join(unpack_dir, "Display.Driver")
    cdf_path = os.path.join(driver_dir, "nv_disp.cdf")
    cat_path = os.path.join(driver_dir, "nv_disp.cat")

    # Temporarily expand .dl_ / .ex_ / .sy_ so makecat hashes uncompressed destination binaries
    # while keeping original compressed files intact for DisplayDriver.nvi's <manifest>.
    temp_expanded_files = []
    ext_map = {".dl_": ".dll", ".ex_": ".exe", ".sy_": ".sys"}
    for root, _, files in os.walk(driver_dir):
        for fname in files:
            low = fname.lower()
            suffix = os.path.splitext(low)[1]
            if suffix in ext_map:
                comp_path = os.path.join(root, fname)
                decomp_name = os.path.splitext(fname)[0] + ext_map[suffix]
                decomp_path = os.path.join(root, decomp_name)
                if not os.path.exists(decomp_path):
                    print(f"[*] Temporarily expanding for catalog hash: {fname} -> {decomp_name}")
                    subprocess.run(["expand", comp_path, decomp_path], check=True)
                    temp_expanded_files.append(decomp_path)

    raw_files = []
    for root, _, files in os.walk(driver_dir):
        for fname in files:
            if fname.lower() in ("nv_disp.cat", "nv_disp.cdf", "makecat.exe"):
                continue
            rel = os.path.relpath(os.path.join(root, fname), driver_dir)
            raw_files.append((fname, rel))

    name_counts = Counter(fname.lower() for fname, _ in raw_files)
    with open(cdf_path, "w", encoding="ascii") as f_cdf:
        f_cdf.write("[CatalogHeader]\nName=nv_disp.cat\nResultDir=.\n\n[CatalogFiles]\n")
        for fname, relpath in sorted(raw_files, key=lambda x: x[1]):
            tag = relpath.replace("\\", "_") if name_counts[fname.lower()] > 1 else fname
            f_cdf.write(f"<HASH>{tag}={relpath}\n")

    subprocess.run([makecat, "-v", "nv_disp.cdf"], cwd=driver_dir, check=True)

    for tmp_f in temp_expanded_files:
        if os.path.exists(tmp_f):
            os.remove(tmp_f)

    print("[*] Signing nv_disp.cat with RFC3161 DigiCert timestamp...")
    subprocess.run(
        [signtool, "sign", "/f", pfx_path, "/p", pfx_pass, "/fd", "SHA256", "/tr", TIMESTAMP_URL, "/td", "SHA256", cat_path],
        check=True,
    )


def integrate_custom_subpackages(unpack_dir, repo_dir, cer_path):
    """Add FrankenDriverCert and standalone Win32 Display.NVCPL subpackages."""
    franken_pkg_dir = os.path.join(unpack_dir, "FrankenDriver")
    os.makedirs(franken_pkg_dir, exist_ok=True)
    shutil.copy2(cer_path, os.path.join(franken_pkg_dir, "FrankenDriver.cer"))
    shutil.copy2(os.path.join(repo_dir, "src", "FrankenDriver.nvi"), os.path.join(franken_pkg_dir, "FrankenDriver.nvi"))

    nvcpl_pkg_dir = os.path.join(unpack_dir, "Display.NVCPL")
    os.makedirs(nvcpl_pkg_dir, exist_ok=True)

    appx_candidates = sorted(glob.glob(os.path.join(unpack_dir, "Display.Driver", "NVCPL", "*.appx")))
    if not appx_candidates:
        raise RuntimeError("No Control Panel .appx found under Display.Driver/NVCPL/!")

    cpl_files = ["nvcplui.exe", "nvcpluir.dll", "nvImage.dll", "NvGpuUtilization.exe", "EULA.txt"]
    with zipfile.ZipFile(appx_candidates[0], "r") as zf:
        for name in cpl_files:
            zf.extract(name, nvcpl_pkg_dir)
            if not os.path.isfile(os.path.join(nvcpl_pkg_dir, name)):
                raise RuntimeError(f"Failed to extract {name} from {appx_candidates[0]}")

    compile_install_helper(repo_dir, nvcpl_pkg_dir)

    # Delete redundant Store AppX payload now that Win32 NVCPL files are extracted
    store_nvcpl_dir = os.path.join(unpack_dir, "Display.Driver", "NVCPL")
    if os.path.isdir(store_nvcpl_dir):
        shutil.rmtree(store_nvcpl_dir)
        print("[+] Removed redundant Display.Driver/NVCPL Store AppX folder.")

    nvcpl_nvi = os.path.join(repo_dir, "src", "NVCPL.nvi")
    if not os.path.isfile(nvcpl_nvi):
        raise RuntimeError(f"Missing {nvcpl_nvi}")
    shutil.copy2(nvcpl_nvi, os.path.join(nvcpl_pkg_dir, "NVCPL.nvi"))


def apply_slim_trim(unpack_dir):
    """Strip NVIDIA App / CEF / telemetry bloat while preserving EULA/PrivacyPolicy for setup.cfg."""
    removed = 0
    for rel in SLIM_REMOVE_PATHS:
        target = os.path.join(unpack_dir, rel)
        if os.path.isdir(target):
            shutil.rmtree(target)
            removed += 1
        elif os.path.isfile(target):
            os.remove(target)
            removed += 1

    nvapp_dir = os.path.join(unpack_dir, "NvApp")
    if os.path.isdir(nvapp_dir):
        for entry in os.listdir(nvapp_dir):
            low = entry.lower()
            keep = (low in SLIM_KEEP_NVAPP_ENTRIES) or low.startswith("functionalconsent_")
            if not keep:
                p = os.path.join(nvapp_dir, entry)
                if os.path.isdir(p):
                    shutil.rmtree(p)
                else:
                    os.remove(p)
    print(f"[+] --slim trim complete ({removed} bloat packages removed; EULA/PrivacyPolicy preserved).")


def _remove_subpackage_xml(cfg, pkg_name):
    pattern = re.compile(
        rf'[ \t]*<sub-package\s+name="{re.escape(pkg_name)}"(?:\s+[^>]*)?(?:/>|>.*?</sub-package>)\r?\n?',
        re.DOTALL,
    )
    return pattern.sub("", cfg)


def update_setup_cfg(unpack_dir, slim):
    """Configure setup.cfg subpackages and dependencies cleanly."""
    cfg_path = os.path.join(unpack_dir, "setup.cfg")
    with open(cfg_path, "r", encoding="utf-8") as f:
        cfg = f.read()

    cfg = _remove_subpackage_xml(cfg, "NVPCF")
    if slim:
        for pkg in SLIM_SUBPACKAGES:
            cfg = _remove_subpackage_xml(cfg, pkg)

    if 'name="FrankenDriverCert"' not in cfg:
        franken_subpkgs = (
            '\t\t<sub-package name="FrankenDriverCert" disposition="default" hidden="true" />\n'
            '\t\t<sub-package name="Display.NVCPL" disposition="default" hidden="false" />\n'
            '\t\t<sub-package name="Display.Driver"'
        )
        cfg = cfg.replace('\t\t<sub-package name="Display.Driver"', franken_subpkgs, 1)

    target_disp = '<sub-package name="Display.Driver" disposition="critical">'
    dep_block = (
        '<sub-package name="Display.Driver" disposition="critical">\n'
        '\t\t\t<dependencies><package package="FrankenDriverCert" type="after" /></dependencies>'
    )
    if 'package="FrankenDriverCert"' not in cfg:
        cfg = cfg.replace(target_disp, dep_block, 1)

    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write(cfg)


def _resolve_manifest_glob(base_dir, pattern):
    """Expand NVI2 manifest entries; require concrete files and allow optional wildcard globs."""
    expanded = pattern.replace("${{LocaleName}}", "en-US").replace("${{InstallerDllDirectory}}", "NVI2").replace("${{ResourcesDirectory}}", "NVI2")
    if "${{" in expanded:
        return True  # Runtime-evaluated variable (outside unpack_dir, e.g. SystemDirectory)
    if "*" in expanded or "?" in expanded:
        return True  # NVI2 wildcard manifest entries (e.g. NVI2/*.jpg) are optional globs
    full_pat = os.path.normpath(os.path.join(base_dir, expanded))
    return os.path.isfile(full_pat)


def _collect_active_manifest_files(elem, inactive_filters):
    """Recursively collect <file name=...> elements respecting <if>/<else> NVI2 filter branches."""
    files = []
    for child in elem:
        tag = child.tag.lower()
        if tag == "file" and "name" in child.attrib:
            files.append(child.attrib["name"])
        elif tag == "if":
            flt = child.attrib.get("filter", "")
            else_elem = child.find("else")
            if flt in inactive_filters:
                if else_elem is not None:
                    files.extend(_collect_active_manifest_files(else_elem, inactive_filters))
            else:
                for sub in child:
                    if sub.tag.lower() != "else":
                        if sub.tag.lower() == "file" and "name" in sub.attrib:
                            files.append(sub.attrib["name"])
                        else:
                            files.extend(_collect_active_manifest_files(sub, inactive_filters))
        elif tag != "else":
            files.extend(_collect_active_manifest_files(child, inactive_filters))
    return files


def run_smoke_check(unpack_dir, signtool, keep_telemetry):
    """Validate manifests, INF integrity, NV_DISP.CAT signature, and telemetry flags before packing."""
    import xml.etree.ElementTree as ET

    print("\n[6/8] Running pre-pack Smoke Check...")

    # 1. Verify INF integrity
    inf_path = os.path.join(unpack_dir, "Display.Driver", "nv_dispi.inf")
    with open(inf_path, "r", encoding="latin1") as f:
        inf_text = f.read()
    if "RM1457588" not in inf_text:
        raise RuntimeError("Smoke check failed: RM1457588 missing from nv_dispi.inf!")

    # 2. Verify NV_DISP.CAT signature
    cat_path = os.path.join(unpack_dir, "Display.Driver", "NV_DISP.CAT")
    res_cat = subprocess.run([signtool, "verify", "/pa", cat_path], capture_output=True, text=True)
    if res_cat.returncode != 0:
        raise RuntimeError(f"Smoke check failed: NV_DISP.CAT signature invalid!\n{res_cat.stderr}")

    # 3. Verify setup.cfg root <manifest>
    cfg_path = os.path.join(unpack_dir, "setup.cfg")
    with open(cfg_path, "r", encoding="utf-8") as f:
        cfg_text = f.read()

    cfg_vars = {}
    for m_var in re.finditer(r'<string\s+name="([^"]+)"\s+value="([^"]+)"', cfg_text):
        cfg_vars[m_var.group(1)] = m_var.group(2)

    inactive_filters = {"x86", "!amd64", "ARM64Platform", "ARM64", "ARMPackage", "PackageTypeARM64", "xp", "minVista", "VistaAndWin7", "minWin8"}

    cfg_root = ET.fromstring(cfg_text)
    root_manifest = cfg_root.find("manifest")
    if root_manifest is not None:
        for fname in _collect_active_manifest_files(root_manifest, inactive_filters):
            resolved = fname
            for _ in range(3):
                for k, v in cfg_vars.items():
                    resolved = resolved.replace(f"${{{{{k}}}}}", v)
            if not _resolve_manifest_glob(unpack_dir, resolved):
                raise RuntimeError(f"Smoke check failed: root setup.cfg manifest file missing: {resolved}")

    # 4. Verify all active subpackage .nvi <manifest> entries
    active_subpkgs = {sp.attrib.get("name") for sp in cfg_root.findall(".//sub-package") if sp.attrib.get("name")}
    checked_nvis = 0
    for root, _, files in os.walk(unpack_dir):
        for fname in files:
            if not fname.lower().endswith(".nvi"):
                continue
            nvi_path = os.path.join(root, fname)
            with open(nvi_path, "r", encoding="utf-8", errors="ignore") as f:
                nvi_text = f.read()
            try:
                nvi_root = ET.fromstring(nvi_text)
            except ET.ParseError as exc:
                raise RuntimeError(f"Smoke check failed: invalid XML in {nvi_path}: {exc}")
            pkg_name = nvi_root.attrib.get("name")
            if pkg_name and pkg_name not in active_subpkgs and not pkg_name.startswith("HDAudio"):
                continue
            nvi_manifest = nvi_root.find("manifest")
            if nvi_manifest is not None:
                nvi_inactive = set(inactive_filters)
                for flt in nvi_root.findall("filter"):
                    plat = flt.attrib.get("platform", "").lower()
                    plats = [p.strip() for p in plat.split(",") if p.strip()]
                    if plats and "amd64" not in plats:
                        nvi_inactive.add(flt.attrib.get("name", ""))
                for entry in _collect_active_manifest_files(nvi_manifest, nvi_inactive):
                    if not _resolve_manifest_glob(root, entry):
                        raise RuntimeError(f"Smoke check failed: missing manifest file '{entry}' referenced by {nvi_path}")
            checked_nvis += 1

    # 5. Verify --keep-telemetry flag consistency
    if keep_telemetry:
        for path in [
            os.path.join(unpack_dir, "Display.Driver", "DisplayDriver.nvi"),
            os.path.join(unpack_dir, "FrankenDriver", "FrankenDriver.nvi"),
        ]:
            if os.path.isfile(path):
                txt = open(path, encoding="utf-8", errors="ignore").read()
                if "disableNvtoppsSSDBug" in txt or "DisableNvToppsSSD" in txt:
                    raise RuntimeError(f"Smoke check failed: nvtopps DisableLoad patch found in {path} despite --keep-telemetry!")

    print(f"[+] Smoke Check passed! Verified INF, NV_DISP.CAT, setup.cfg, and {checked_nvis} NVI manifests.")


def extract_clean_sfx_stub(orig_exe):
    """Dynamically locate PackageLauncher PE end + SFX config block and clear PE security directory."""
    with open(orig_exe, "rb") as f:
        head = f.read(4 * 1024 * 1024)

    idx_7z = head.find(b"7z\xbc\xaf\x27\x1c")
    if idx_7z == -1:
        raise RuntimeError("Could not locate 7z payload signature in official NVIDIA installer!")

    if pefile:
        pe = pefile.PE(data=head[:idx_7z], fast_load=True)
        pe_end = max(s.PointerToRawData + s.SizeOfRawData for s in pe.sections)
        pe_full = pefile.PE(data=head[:pe_end])
        sec_idx = pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_SECURITY"]
        pe_full.OPTIONAL_HEADER.DATA_DIRECTORY[sec_idx].VirtualAddress = 0
        pe_full.OPTIONAL_HEADER.DATA_DIRECTORY[sec_idx].Size = 0
        clean_pe = pe_full.write()
    else:
        idx_cfg = head.find(b";!@Install@!UTF-8!")
        if idx_cfg == -1 or idx_cfg >= idx_7z:
            raise RuntimeError("Install pefile (`pip install pefile`) or could not find SFX config marker.")
        pe_end = idx_cfg
        pe_part = bytearray(head[:pe_end])
        e_lfanew = struct.unpack_from("<I", pe_part, 0x3C)[0]
        magic = struct.unpack_from("<H", pe_part, e_lfanew + 24)[0]
        is_pe32 = magic == 0x10B
        sec_dir_offset = e_lfanew + 24 + (96 if is_pe32 else 112) + 4 * 8
        struct.pack_into("<II", pe_part, sec_dir_offset, 0, 0)
        clean_pe = bytes(pe_part)

    config_part = head[pe_end:idx_7z]
    print(f"[*] Dynamic SFX stub: PE={pe_end} bytes, Config={len(config_part)} bytes")
    return clean_pe + config_part


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
    parser.add_argument("--keep-telemetry", action="store_true", help="Keep NVIDIA nvtopps telemetry plugin enabled")
    parser.add_argument("--slim", action="store_true", help="Strip NVIDIA App / Overlay / Telemetry bloat (~650 MB unpacked)")
    args = parser.parse_args()

    repo_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    seven_zip = args.seven_zip or find_tool("7z.exe", [r"C:\Program Files\7-Zip\7z.exe", r"C:\Program Files (x86)\7-Zip\7z.exe"])
    if not seven_zip:
        sys.exit("[-] Error: 7z.exe not found. Please install 7-Zip or pass --seven-zip.")

    signtool = args.signtool or find_tool("signtool.exe", [
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\signtool.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe",
    ])
    if not signtool:
        sys.exit("[-] Error: signtool.exe not found. Install Windows SDK or pass --signtool.")

    makecat = args.makecat or find_tool("makecat.exe", [
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\makecat.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\makecat.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\makecat.exe",
    ])
    if not makecat:
        sys.exit("[-] Error: makecat.exe not found. Install Windows SDK or pass --makecat.")

    orig_exe = args.nvidia_exe
    if not orig_exe:
        for d in [repo_dir, os.path.dirname(repo_dir), os.getcwd()]:
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
    if args.slim:
        print("[*] Mode: SLIM (NVIDIA App / Overlay / Telemetry stripped)")

    pfx_path, cer_path = resolve_signing_certs(args, repo_dir)

    work_dir = args.work_dir or os.path.join(repo_dir, "build_temp")
    os.makedirs(work_dir, exist_ok=True)
    unpack_dir = os.path.join(work_dir, "driver_unpacked")

    print(f"\n[1/8] Extracting official NVIDIA installer to: {unpack_dir}")
    if os.path.exists(unpack_dir):
        shutil.rmtree(unpack_dir)
    os.makedirs(unpack_dir, exist_ok=True)
    subprocess.run([seven_zip, "x", orig_exe, f"-o{unpack_dir}", "-y"], check=True)

    print("\n[2/8] Patching nv_dispi.inf with Mutant GPUs database...")
    target_inf = os.path.join(unpack_dir, "Display.Driver", "nv_dispi.inf")
    inject_script = os.path.join(repo_dir, "scripts", "inject.py")
    subprocess.run([sys.executable, inject_script, target_inf], check=True)
    patch_display_driver_nvi(unpack_dir, args.keep_telemetry)

    print("\n[3/8] Integrating custom subpackages (FrankenDriverCert + Display.NVCPL)...")
    integrate_custom_subpackages(unpack_dir, repo_dir, cer_path)

    print("\n[4/8] Generating and signing security catalog (nv_disp.cat)...")
    generate_security_catalog(unpack_dir, makecat, signtool, pfx_path, args.pfx_pass)

    print("\n[5/8] Updating setup.cfg...")
    if args.slim:
        apply_slim_trim(unpack_dir)
    update_setup_cfg(unpack_dir, args.slim)

    run_smoke_check(unpack_dir, signtool, args.keep_telemetry)

    print("\n[7/8] Building monolithic self-extracting archive (-mx=4, d24)...")
    clean_stub = extract_clean_sfx_stub(orig_exe)
    archive_7z = os.path.join(work_dir, "payload.7z")
    subprocess.run(
        [seven_zip, "a", archive_7z, os.path.join(unpack_dir, "*"), "-m0=LZMA2:d24", "-mf=off", "-mx=4", "-ms=on", "-mmt=16"],
        check=True,
    )

    ver_match = re.search(r"(\d{3}\.\d{2})", os.path.basename(orig_exe))
    ver_str = ver_match.group(1) if ver_match else "621.xx"
    suffix = "_Slim_WHQL.exe" if args.slim else "_WHQL.exe"
    out_exe = args.output or os.path.join(os.path.dirname(repo_dir), f"OpenFrankenDriver_{ver_str}{suffix}")

    temp_exe = os.path.join(work_dir, "unsigned.exe")
    with open(temp_exe, "wb") as f_out:
        f_out.write(clean_stub)
        with open(archive_7z, "rb") as f_in:
            while chunk := f_in.read(16 * 1024 * 1024):
                f_out.write(chunk)

    print("\n[8/8] Signing and verifying final monolithic installer...")
    subprocess.run(
        [signtool, "sign", "/f", pfx_path, "/p", args.pfx_pass, "/fd", "SHA256", "/tr", TIMESTAMP_URL, "/td", "SHA256", temp_exe],
        check=True,
    )
    subprocess.run([signtool, "verify", "/pa", temp_exe], check=True)

    if os.path.exists(out_exe):
        os.remove(out_exe)
    shutil.move(temp_exe, out_exe)
    shutil.rmtree(work_dir, ignore_errors=True)

    print("=" * 75)
    print(f"[SUCCESS] Monolithic OpenFrankenDriver ready at:\n--> {out_exe}")
    print("=" * 75)


if __name__ == "__main__":
    main()
