"""Variant 2 (Beta / v2) build helpers for OpenFrankenDriver.

Keeps Phase Guard (.nvi audit), PhysX manifest extraction, cover downloads,
Rust setup.exe compilation/signing, slim Beta trimming, and 7zSD SFX config patching
clean and modular without affecting Variant 1 (1.x) builds.
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

NVPI_VERSION = "3.0.2.1"
NVPI_ZIP_URL = f"https://github.com/Orbmu2k/nvidiaProfileInspector/releases/download/v{NVPI_VERSION}/nvidiaProfileInspector.zip"
NVPI_ZIP_SHA256 = "88dcf3514111e8de630688467c03c36d8c2a8ad9ebc8073f27c069f82b75bb40"


KNOWN_DISPLAY_PHASES = {
    "CheckDanglingDeviceRemoval", "CheckIfNvCplUIPresent", "Confirm-install",
    "GetEnvVariablesWhileInstall", "GetEnvVariablesWhileUninstall", "NGXUpdater",
    "SetRebootForWin8KFilter", "VulkanUninstall", "addNvsmiToReg",
    "changeDisplayResolution", "checkDriverInterruptModeIssue", "checkIfCleanInstall",
    "checkIfFreshInstall", "checkIfGridProxyScriptExists", "checkIfGridTPLFileExists",
    "checkIfNVWMIRunning", "checkIfNvFBCPluginExists", "checkIfNvToppsPluginExists",
    "checkIfVGPUFolderExists", "checkNVSMIPathExist", "checkRemoveOptimusFilters",
    "clearDCHCPLRegistryKeys", "containerInstall", "copyGridLicenseFile",
    "copyGridProxyScript", "copyLogo", "cplAppxPackageInstall", "cplInstall",
    "createDisableLoadRegForDCHRASPlugin", "createDisableLoadRegForLegacyRASPlugin",
    "createJunctionPointforNvVGPUPlugin", "createNGXInstallSettings",
    "createNvProfileUpdaterJunctionPoint", "createRealTimeRunlistKey",
    "decideIfSameDriver", "deleteComponents", "deleteDCHGridGeneratedFiles",
    "deleteDeprecatedContentProtectionDriverNameRegistry",
    "deleteDeprecatedFileNvmcumd", "deleteFilesFromDeprecatedLocations",
    "deleteGridProxyScript", "deleteGridSWLRegistryKey",
    "deleteJunctionPointforNvFBCPlugins", "deleteJunctionPointforNvToppsPlugins",
    "deleteJunctionPointforNvVGPUPlugin", "deleteLegacyGridRegistryKeys",
    "deleteMisc", "deleteNvFBCPlugins", "deleteNvFBCx86",
    "deleteNvProfileUpdaterPlugin", "deleteNvToppsPlugin", "deleteOTACache",
    "deleteOldNonOptioalNvToppsPlugin", "deleteOlderDrsFiles",
    "deleteOlderNGXInstallSettings", "deleteOlderRASPluginData",
    "deleteOldwksServicePluginZero", "deleteRMEnablePhyRepeaterRegistryKeyConditionally",
    "deleteRegistryKeys", "deleteStoreAppRegKeys", "deleteTrustedInsallerKey",
    "deregisterOSCTool", "disableOTA", "getDriverStoreLocation",
    "getNGXDriverStoreLocation", "hideNVMIARPEntry", "installAllDrivers",
    "installDisplayDriverRASPlugin", "installGSTPlugin", "installGridvGaming",
    "installNvFBCPlugins", "installNvProfileUpdaterPlugin", "installNvToppsPlugins",
    "installWksServicePlugin", "installWksServicePluginZero", "removeCPLInstallData",
    "removeCache", "removeComputeCache", "removeDXCache", "removeDXCacheOnUninstall",
    "removeDXCacheWhileInstall", "removeDeprecatedFiles", "removeDisplayPhantoms",
    "removeImplicitDirectories", "removeNvModuleTrackerDevice", "removeNvVHciDevice",
    "removeOldARPEntry", "removeOldOpenGLCache", "removeOpenCLLegacyx64",
    "removeOpenCLLegacyx86", "removeOpenGLCache", "removeStoreCPLApp",
    "removeStoreCPLAppIfNvCplUiPresent", "removeUserInstalledNVCPLAppx",
    "removeUserInstalledNVCPLAppxOnUninstall", "removenvOpenCLx64",
    "removenvOpenCLx86", "restartDevices", "runCacheCleanupTool",
    "startDisplayContainerLocalSystem", "startNVWMIService", "stopCplServiceMinVista",
    "stopCplServicePreVista", "updateCPLInstallPhaseApp",
}

KNOWN_HDAUDIO_PHASES = {
    "UninstallExtensionInfs", "installAudioDriver", "installAudioDriverExtension",
    "removeOldARPEntry", "removePhantomAudioDevices",
}

KNOWN_PHYSX_PHASES = {
    "AddEnvSettings", "Confirm-install", "addRegistries", "addRegistries64",
    "copyFiles", "uninstallOldMSI",
}


def download_cover_file(url, dst_path, crop_rect=None):
    """Download an image asset from URL with retries, optionally crop/resize, and save clean RGB JPEG."""
    import io
    from PIL import Image

    last_exc = None
    for attempt in range(4):
        if attempt > 0:
            time.sleep(1.5 * attempt)
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"})
            with urllib.request.urlopen(req, timeout=30) as resp:
                data = resp.read()
            with Image.open(io.BytesIO(data)) as im:
                im = im.convert("RGB")
                if crop_rect and len(crop_rect) == 4:
                    w, h = im.size
                    box = (
                        int(round(crop_rect[0] * w)),
                        int(round(crop_rect[1] * h)),
                        int(round(crop_rect[2] * w)),
                        int(round(crop_rect[3] * h)),
                    )
                    im = im.crop(box)
                if im.width > 1440 or im.height > 1080:
                    im.thumbnail((1440, 1080), Image.Resampling.LANCZOS)
                buf = io.BytesIO()
                im.save(buf, format="JPEG", quality=92)
                out_bytes = buf.getvalue()
            if out_bytes[:3] == b"\xff\xd8\xff":
                with open(dst_path, "wb") as f:
                    f.write(out_bytes)
                return len(out_bytes)
        except Exception as exc:
            last_exc = exc
    raise RuntimeError(f"Failed to download {url} after 4 attempts: {last_exc}")


def check_nvi_phases_guard(unpack_dir):
    """Audit DisplayDriver.nvi, HDAudio.nvi, PhysX.nvi for unexpected new phases in future driver releases."""
    targets = [
        (os.path.join(unpack_dir, "Display.Driver", "DisplayDriver.nvi"), KNOWN_DISPLAY_PHASES, "DisplayDriver.nvi"),
        (os.path.join(unpack_dir, "HDAudio", "HDAudio.nvi"), KNOWN_HDAUDIO_PHASES, "HDAudio.nvi"),
        (os.path.join(unpack_dir, "PhysX", "PhysX.nvi"), KNOWN_PHYSX_PHASES, "PhysX.nvi"),
    ]
    unknown_total = []
    for path, known, label in targets:
        if not os.path.isfile(path):
            continue
        root = ET.parse(path).getroot()
        found = {el.attrib["phase"] for el in root.findall(".//phases//*") if "phase" in el.attrib}
        unknown = sorted(found - known)
        if unknown:
            unknown_total.append((label, unknown))
            print(f"[!] PHASE GUARD WARNING ({label}): new unknown NVI phase(s) detected: {unknown}")
    if not unknown_total:
        print("[+] Phase Guard passed: all DisplayDriver / HDAudio / PhysX NVI phases match verified whitelist.")
    return unknown_total


def _read_nvi_string(nvi_path, str_name, default):
    if not os.path.isfile(nvi_path):
        return default
    root = ET.parse(nvi_path).getroot()
    for s in root.findall(".//strings/string"):
        if s.attrib.get("name") == str_name and "value" in s.attrib:
            return s.attrib["value"]
    return default


def extract_package_versions(unpack_dir, orig_exe):
    """Extract Display, PhysX, and HDAudio versions from official manifests before trimming."""
    ver_match = re.search(r"(\d{3}\.\d{2})", os.path.basename(orig_exe))
    drv_fallback = ver_match.group(1) if ver_match else "621.40"
    disp_ver = _read_nvi_string(os.path.join(unpack_dir, "Display.Driver", "DisplayDriver.nvi"), "version", drv_fallback)
    physx_ver = _read_nvi_string(os.path.join(unpack_dir, "PhysX", "PhysX.nvi"), "version", "9.26.0904")
    hda_ver = _read_nvi_string(os.path.join(unpack_dir, "HDAudio", "HDAudio.nvi"), "version", "1.4.6.6")
    return {
        "driver_version": disp_ver,
        "physx_version": physx_ver,
        "hdaudio_version": hda_ver,
        "nvpi_version": NVPI_VERSION,
    }


def generate_physx_manifest(unpack_dir):
    """Convert PhysX/PhysX.nvi into PhysX/physx_manifest.json for the Rust installer."""
    physx_dir = os.path.join(unpack_dir, "PhysX")
    nvi_path = os.path.join(physx_dir, "PhysX.nvi")
    if not os.path.isfile(nvi_path):
        raise RuntimeError(f"Missing {nvi_path}")

    root = ET.parse(nvi_path).getroot()
    vars_map = {}
    for s in root.findall(".//strings/string"):
        if "name" in s.attrib and "value" in s.attrib:
            vars_map[s.attrib["name"]] = s.attrib["value"]

    version = vars_map.get("version", "9.26.0904")

    def expand(val):
        out = val
        for _ in range(3):
            out = out.replace("${{version}}", version)
            out = out.replace("${{BUILD_VERSION_NUM}}", vars_map.get("BUILD_VERSION_NUM", "9260904"))
            out = out.replace("${{AgeiaSoftwareKey}}", "HKLM\\SOFTWARE\\AGEIA Technologies")
            out = out.replace("${{NvidiaSoftwareKey}}", "HKLM\\SOFTWARE\\NVIDIA Corporation")
            out = out.replace("${{NvidiaProgramFilesX86}}\\PhysX", "${InstallLocation}")
            out = out.replace("${{InstallLocation}}", "${InstallLocation}")
        return out

    copy_files = []
    registries = []
    path_add = ""

    phases = root.find("phases")
    if phases is not None:
        for st in phases.findall("standard"):
            phase_name = st.attrib.get("phase", "")
            if phase_name == "copyFiles":
                for cf in st.findall("copyFile"):
                    src = cf.attrib.get("source", "")
                    tgt = expand(cf.attrib.get("target", ""))
                    if tgt.startswith("${InstallLocation}\\"):
                        tgt = tgt[len("${InstallLocation}\\"):]
                    copy_files.append({"source": src, "target": tgt})
            elif phase_name in ("addRegistries", "addRegistries64"):
                view = "x64" if phase_name == "addRegistries64" else "x86"
                for reg in st.findall("addRegistry"):
                    registries.append({
                        "view": view,
                        "key": expand(reg.attrib.get("keyName", "")),
                        "value_name": reg.attrib.get("valueName", ""),
                        "type": reg.attrib.get("type", ""),
                        "value": expand(reg.attrib.get("value", "")),
                        "split": reg.attrib.get("split", "|"),
                    })
            elif phase_name == "AddEnvSettings":
                for ap in st.findall("addPath"):
                    path_add = expand(ap.attrib.get("target", ""))

    manifest = {
        "version": version,
        "upgrade_codes": [
            "{C40CFCD4-C757-4139-A4DA-7CB51A8DBF80}",
            "{C15730E2-145C-4c5e-B005-3BC753F42475}",
        ],
        "copy_files": copy_files,
        "registries": registries,
        "path_add": path_add,
    }

    out_path = os.path.join(physx_dir, "physx_manifest.json")
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
    print(f"[+] Generated PhysX/physx_manifest.json ({len(copy_files)} files, {len(registries)} registry entries).")


def sign_with_timestamp(signtool, pfx_path, pfx_pass, target_path, timestamp_url="http://timestamp.digicert.com"):
    """Sign a PE/CAT artifact with RFC3161 timestamp, retrying across fallback TSA servers."""
    tsa_candidates = []
    for url in [timestamp_url, "http://timestamp.sectigo.com", "http://rfc3161timestamp.globalsign.com/advanced"]:
        if url and url not in tsa_candidates:
            tsa_candidates.append(url)
    last_res = None
    for round_idx in range(2):
        for tsa in tsa_candidates:
            if round_idx > 0 or tsa != tsa_candidates[0]:
                time.sleep(1.5)
            res = subprocess.run(
                [signtool, "sign", "/f", pfx_path, "/p", pfx_pass, "/fd", "SHA256", "/tr", tsa, "/td", "SHA256", target_path],
                capture_output=True,
                text=True,
            )
            if res.returncode == 0:
                if res.stdout:
                    print(res.stdout.strip())
                return
            last_res = res
            print(f"[!] Timestamp attempt failed ({tsa}): {res.stderr.strip() or res.stdout.strip()}")
    raise subprocess.CalledProcessError(
        last_res.returncode if last_res else 1,
        [signtool, "sign", target_path],
        output=last_res.stdout if last_res else "",
        stderr=last_res.stderr if last_res else "",
    )


def build_and_install_rust_setup(repo_dir, unpack_dir, signtool, pfx_path, pfx_pass, timestamp_url):
    """Compile src/setup_rust in release mode, sign setup.exe, and place it into unpack_dir."""
    rust_dir = os.path.join(repo_dir, "src", "setup_rust")
    manifest = os.path.join(rust_dir, "Cargo.toml")
    if not os.path.isfile(manifest):
        raise RuntimeError(f"Rust installer Cargo.toml missing: {manifest}")

    print("[*] Building native Rust setup.exe (cargo build --release, static CRT)...")
    env = os.environ.copy()
    env.setdefault("RUSTFLAGS", "-C target-feature=+crt-static")
    subprocess.run(["cargo", "build", "--release"], cwd=rust_dir, env=env, check=True)
    built_exe = os.path.join(rust_dir, "target", "release", "setup.exe")
    if not os.path.isfile(built_exe):
        raise RuntimeError(f"Compiled setup.exe not found at {built_exe}")

    dst_exe = os.path.join(unpack_dir, "setup.exe")
    shutil.copy2(built_exe, dst_exe)

    print("[*] Signing native setup.exe with RFC3161 timestamp...")
    sign_with_timestamp(signtool, pfx_path, pfx_pass, dst_exe, timestamp_url)


def _dir_size_mb(path):
    total = 0
    for root, _, files in os.walk(path):
        for fn in files:
            try:
                total += os.path.getsize(os.path.join(root, fn))
            except OSError:
                pass
    return max(500, round(total / (1024 * 1024)))


def apply_beta_trim(unpack_dir):
    """Remove NVI2, NVIDIA App, Telemetry, and all NVI/Ext installer remnants for Beta."""
    keep_top_dirs = {
        "Display.Driver",
        "Display.NVCPL",
        "FrankenDriver",
        "HDAudio",
        "NVProfileInspector",
        "PhysX",
        "MSVCRT",
        "Showcase",
    }
    keep_top_files = {"setup.exe"}

    removed = 0
    for entry in os.listdir(unpack_dir):
        full = os.path.join(unpack_dir, entry)
        if os.path.isdir(full):
            if entry not in keep_top_dirs:
                shutil.rmtree(full)
                removed += 1
        else:
            if entry.lower() not in keep_top_files:
                os.remove(full)
                removed += 1

    # Keep Display.Driver/* and HDAudio/* intact so nv_dispi.inf [IncludedSubPackages] and signed nvhda.cat remain 100% valid.
    extra_cleanup = [
        os.path.join(unpack_dir, "PhysX", "PhysXExt.dll"),
        os.path.join(unpack_dir, "PhysX", "PhysXExt64.dll"),
        os.path.join(unpack_dir, "PhysX", "PhysX.nvi"),
        os.path.join(unpack_dir, "MSVCRT", "MSVCRTExt.dll"),
        os.path.join(unpack_dir, "MSVCRT", "MSVCRuntime_2019.nvi"),
        os.path.join(unpack_dir, "Display.NVCPL", "NVCPL.nvi"),
        os.path.join(unpack_dir, "Display.NVCPL", "install_helper.exe"),
        os.path.join(unpack_dir, "FrankenDriver", "FrankenDriver.nvi"),
    ]
    for p in extra_cleanup:
        if os.path.isfile(p):
            os.remove(p)

    print(f"[+] Beta trim complete ({removed} bloat root items + NVI2/Ext files stripped).")


def prepare_beta_nvpi(unpack_dir):
    """Fetch, SHA256-verify, and unpack stable NVIDIA Profile Inspector into unpack_dir/NVProfileInspector."""
    cache_dir = os.path.join(tempfile.gettempdir(), "OpenFrankenDriver_cache")
    os.makedirs(cache_dir, exist_ok=True)
    zip_path = os.path.join(cache_dir, f"nvidiaProfileInspector_v{NVPI_VERSION}.zip")

    def file_sha256(p):
        h = hashlib.sha256()
        with open(p, "rb") as f:
            for chunk in iter(lambda: f.read(65536), b""):
                h.update(chunk)
        return h.hexdigest().lower()

    if not os.path.isfile(zip_path) or file_sha256(zip_path) != NVPI_ZIP_SHA256:
        last_exc = None
        for attempt in range(4):
            if attempt > 0:
                time.sleep(1.5 * attempt)
            try:
                req = urllib.request.Request(NVPI_ZIP_URL, headers={"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"})
                with urllib.request.urlopen(req, timeout=30) as resp:
                    data = resp.read()
                got_hash = hashlib.sha256(data).hexdigest().lower()
                if got_hash != NVPI_ZIP_SHA256:
                    raise RuntimeError(f"NVPI SHA256 mismatch: expected {NVPI_ZIP_SHA256}, got {got_hash}")
                with open(zip_path, "wb") as f:
                    f.write(data)
                break
            except Exception as exc:
                last_exc = exc
        else:
            raise RuntimeError(f"Failed to download {NVPI_ZIP_URL} after 4 attempts: {last_exc}")

    nvpi_dir = os.path.join(unpack_dir, "NVProfileInspector")
    os.makedirs(nvpi_dir, exist_ok=True)
    wanted = {
        "nvidiaProfileInspector.exe",
        "nvidiaProfileInspector.exe.config",
        "Reference.xml",
    }
    with zipfile.ZipFile(zip_path, "r") as zf:
        for member in zf.infolist():
            base = os.path.basename(member.filename)
            if base in wanted:
                with zf.open(member, "r") as src, open(os.path.join(nvpi_dir, base), "wb") as dst:
                    shutil.copyfileobj(src, dst)
    print(f"[+] Bundled NVIDIA Profile Inspector v{NVPI_VERSION} (SHA256 verified) -> NVProfileInspector/")


def prepare_beta_showcase_and_meta(unpack_dir, repo_dir, meta):
    """Write Showcase/showcase.json, download Beta covers, and write Showcase/package_meta.json."""
    src_json = os.path.join(repo_dir, "data", "games_showcase.json")
    showcase_dir = os.path.join(unpack_dir, "Showcase")
    os.makedirs(showcase_dir, exist_ok=True)

    with open(src_json, "r", encoding="utf-8") as f:
        all_games = json.load(f)

    beta_games = [g for g in all_games if g.get("beta")]
    if not beta_games:
        beta_games = all_games[:4]

    for item in beta_games:
        url = item.get("cover_url", "")
        fname = item.get("slide_file", "")
        if url and fname:
            dst = os.path.join(showcase_dir, fname)
            size = download_cover_file(url, dst, item.get("crop_rect"))
            if size is not None:
                print(f"[+] Downloaded Beta cover: Showcase/{fname} ({size} bytes)")

    with open(os.path.join(showcase_dir, "showcase.json"), "w", encoding="utf-8") as f:
        json.dump(beta_games, f, ensure_ascii=False, indent=2)

    meta_out = dict(meta)
    meta_out["estimated_mb"] = _dir_size_mb(unpack_dir)
    with open(os.path.join(showcase_dir, "package_meta.json"), "w", encoding="utf-8") as f:
        json.dump(meta_out, f, indent=2)
    print(f"[+] Showcase ready ({len(beta_games)} slides, estimated size ~{meta_out['estimated_mb']} MiB).")


def build_rust_sfx_stub(repo_dir, ver_str):
    """Compile src/sfx_rust (pure Rust Variant 03E SFX) in release mode and append OFD metadata."""
    sfx_dir = os.path.join(repo_dir, "src", "sfx_rust")
    manifest = os.path.join(sfx_dir, "Cargo.toml")
    if not os.path.isfile(manifest):
        raise RuntimeError(f"Rust SFX Cargo.toml missing: {manifest}")

    print("[*] Building native Rust SFX unpacker (sfx_stub.exe, Variant 03E, static CRT)...")
    env = os.environ.copy()
    env.setdefault("RUSTFLAGS", "-C target-feature=+crt-static")
    subprocess.run(["cargo", "build", "--release"], cwd=sfx_dir, env=env, check=True)
    built_exe = os.path.join(sfx_dir, "target", "release", "sfx_stub.exe")
    if not os.path.isfile(built_exe):
        raise RuntimeError(f"Compiled sfx_stub.exe not found at {built_exe}")

    with open(built_exe, "rb") as f:
        clean_pe = f.read()

    cfg = (
        ";!@OFD@!1\r\n"
        f"version={ver_str}\r\n"
        f"install_dir=C:\\NVIDIA\\OpenFrankenDriver\\{ver_str}\r\n"
        ";!@OFD_END@!\r\n"
    ).encode("utf-8")
    print(f"[+] Pure Rust SFX stub ready: PE={len(clean_pe)} bytes, Config={len(cfg)} bytes")
    return clean_pe + cfg


def build_beta_sfx_stub(clean_pe_plus_orig_cfg, orig_exe, ver_str):
    """Replace PackageLauncher 7zSD config with silent C:\\NVIDIA\\OpenFrankenDriver\\<ver> auto-extract."""
    idx_cfg = clean_pe_plus_orig_cfg.find(b";!@Install@!UTF-8!")
    if idx_cfg == -1:
        raise RuntimeError("Could not locate SFX config block inside PackageLauncher stub!")
    clean_pe = clean_pe_plus_orig_cfg[:idx_cfg]
    _ = orig_exe

    cfg = (
        ";!@Install@!UTF-8!\r\n"
        f'InstallPath="C:\\\\NVIDIA\\\\OpenFrankenDriver\\\\{ver_str}"\r\n'
        'OverwriteMode="2"\r\n'
        'RunProgram=".\\\\setup.exe"\r\n'
        'SelfDelete="0"\r\n'
        'GUIMode="1"\r\n'
        'Progress="yes"\r\n'
        f'Title="OpenFrankenDriver {ver_str}"\r\n'
        f'ExtractTitle="OpenFrankenDriver {ver_str}"\r\n'
        f'ExtractDialogWidth="520"\r\n'
        f'ExtractDialogText="C:\\\\NVIDIA\\\\OpenFrankenDriver\\\\{ver_str}"\r\n'
        ";!@InstallEnd@!\r\n"
    ).encode("utf-8")
    return clean_pe + cfg


def run_beta_smoke_check(unpack_dir, signtool):
    """Validate Beta payload: INF, signed NV_DISP.CAT, signed setup.exe, PhysX JSON, Showcase, and bloat removal."""
    print("\n[6/8] Running pre-pack Beta Smoke Check...")

    inf_path = os.path.join(unpack_dir, "Display.Driver", "nv_dispi.inf")
    with open(inf_path, "r", encoding="latin1") as f:
        if "RM1457588" not in f.read():
            raise RuntimeError("Beta smoke check failed: RM1457588 missing from nv_dispi.inf!")

    cat_path = os.path.join(unpack_dir, "Display.Driver", "NV_DISP.CAT")
    res_cat = subprocess.run([signtool, "verify", "/pa", cat_path], capture_output=True, text=True)
    if res_cat.returncode != 0:
        raise RuntimeError(f"Beta smoke check failed: NV_DISP.CAT invalid!\n{res_cat.stderr}")

    setup_path = os.path.join(unpack_dir, "setup.exe")
    res_exe = subprocess.run([signtool, "verify", "/pa", setup_path], capture_output=True, text=True)
    if res_exe.returncode != 0:
        raise RuntimeError(f"Beta smoke check failed: setup.exe signature invalid!\n{res_exe.stderr}")

    for req in [
        os.path.join(unpack_dir, "Display.NVCPL", "nvcplui.exe"),
        os.path.join(unpack_dir, "NVProfileInspector", "nvidiaProfileInspector.exe"),
        os.path.join(unpack_dir, "NVProfileInspector", "Reference.xml"),
        os.path.join(unpack_dir, "FrankenDriver", "FrankenDriver.cer"),
        os.path.join(unpack_dir, "HDAudio", "nvhda.inf"),
        os.path.join(unpack_dir, "PhysX", "physx_manifest.json"),
        os.path.join(unpack_dir, "Showcase", "showcase.json"),
        os.path.join(unpack_dir, "Showcase", "package_meta.json"),
    ]:
        if not os.path.isfile(req):
            raise RuntimeError(f"Beta smoke check failed: missing {req}")

    with open(os.path.join(unpack_dir, "PhysX", "physx_manifest.json"), "r", encoding="utf-8") as f:
        pm = json.load(f)
    views = {r["view"] for r in pm.get("registries", [])}
    if not {"x86", "x64"}.issubset(views):
        raise RuntimeError(f"Beta smoke check failed: physx_manifest.json missing x86/x64 views ({views})")

    with open(os.path.join(unpack_dir, "Showcase", "showcase.json"), "r", encoding="utf-8") as f:
        slides = json.load(f)
    if len(slides) < 4:
        raise RuntimeError(f"Beta smoke check failed: expected >= 4 Beta slides, got {len(slides)}")
    for s in slides:
        img = os.path.join(unpack_dir, "Showcase", s["slide_file"])
        if not os.path.isfile(img):
            raise RuntimeError(f"Beta smoke check failed: missing cover {img}")

    for forbidden in ["NVI2", "NvApp", "NvTelemetry", "ShadowPlay", "FrameViewSDK"]:
        if os.path.exists(os.path.join(unpack_dir, forbidden)):
            raise RuntimeError(f"Beta smoke check failed: forbidden bloat directory still present: {forbidden}")

    print(f"[+] Beta Smoke Check passed! Verified INF, NV_DISP.CAT, signed setup.exe, PhysX JSON, {len(slides)}-game Showcase, and zero bloat.")
