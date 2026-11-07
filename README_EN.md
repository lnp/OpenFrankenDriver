<div align="center">

[![English](https://img.shields.io/badge/Language-English-1f883d?style=for-the-badge)](#)
[![Русский](https://img.shields.io/badge/Язык-Русский-0969da?style=for-the-badge)](README.md)

<br>

<a href="https://github.com/lnp/OpenFrankenDriver"><img src="assets/header.png" alt="OpenFrankenDriver" width="840"></a>
<br>

**Fully open-source, free, and secure NVIDIA driver patcher & installer for desktop mutant GPUs**  
*(RTX 4070m, RTX 4060m, RTX 3070m, RTX 3060m, RTX 2060m, GTX 1660m, and 65 more models based on AD106/GA104/GA106/TU106 mobile silicon)*

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![NVIDIA Driver Version](https://img.shields.io/badge/NVIDIA-621.14%20WHQL-76B900?logo=nvidia)](https://www.nvidia.com)
[![Anti-Cheat Ready](https://img.shields.io/badge/Anti--Cheat-EAC%20%7C%20BattlEye%20%7C%20VAC%20Passed-success)](docs/ARCHITECTURE.md)
[![Platform](https://img.shields.io/badge/Windows-10%20%7C%2011%20(24H2%2B)-0078D6?logo=windows)](docs/SUPPORTED_DEVICES.md)
[![Boosty](https://img.shields.io/badge/Boosty-Support%20Project-F15F2C?logo=boosty)](https://boosty.to/defrag)

---

### 📥 [Download Pre-built OpenFrankenDriver 621.14 (Google Drive)](https://drive.google.com/file/d/1uIS2bM4CIcCxQ9KAcM-kB_A9dio3UTHF/view?usp=sharing)
*(Size: ~1.07 GB • Stable 621.14 WHQL • Bundled with standalone classic Control Panel)*

---

### Project Support
The project is developed out of pure enthusiasm. All drivers and build automation scripts will forever remain completely open-source and free.

If this build saved you time, revived your graphics card, and eliminated tedious troubleshooting — you are welcome to voluntarily support development on [Boosty](https://boosty.to/defrag).

---

[**Supported Hardware**](docs/SUPPORTED_DEVICES.md) • [**How to Build & Patch**](#-how-to-modify-official-nvidia-drivers) • [**Architecture**](docs/ARCHITECTURE.md) • [**Troubleshooting**](docs/TROUBLESHOOTING.md)

</div>

---

## 🚀 Why OpenFrankenDriver? (Advantages over original FrankenDriver)

The original closed-source **FrankenDriver** (`arutar/FrankenDriver`) was a popular tool for running mobile-on-desktop mutant GPUs. However, the original repository **contains no build code or scripts**, its free releases are long outdated, and newer updates are locked behind paid subscriptions.

**OpenFrankenDriver** is a fully open-source, free, and technically refined alternative to the original project:

| Feature / Issue | Original FrankenDriver | <img src="assets/badge.png" alt="OpenFrankenDriver" width="180"> |
|---|---|---|
| **Open Source & Transparency** | ❌ Closed-source binary. Repository has no code or build scripts. | ✅ **100% Open-Source**. Complete Python build pipeline is public and auditable. |
| **Availability** | ❌ Modern driver branches are paywalled. | ✅ **Always free**. Includes the latest stable **621.14 WHQL** branch. |
| **Zero-Touch Installation** | ❌ Annoying red Windows Security popup ("Publisher cannot be verified"). Required manual cert import. | ✅ **Silent automated trust**. Certificates are seamlessly imported into `Root` and `TrustedPublisher`. Zero popups. |
| **Standalone Control Panel** | ❌ Missing on clean Windows or stripped editions (required MS Store and internet). | ✅ **Standalone Win32 Control Panel** (`nvcplui.exe`) bundled natively. Works out of the box on Windows LTSC / Compact without Store or internet. |
| **Safety & Anti-Cheat** | ❓ Closed binaries raised security concerns. | ✅ **Genuine Untouched NVIDIA Kernel Binaries**. 100% WHQL integrity, `signtool` verified, `testsigning No`, full EAC / BattlEye / VAC support. |

---

## 💾 Quick Install (For Gamers & End Users)

1. **Download the pre-built installer** via the link above or directly from [Google Drive](https://drive.google.com/file/d/1uIS2bM4CIcCxQ9KAcM-kB_A9dio3UTHF/view?usp=sharing).
2. *(Recommended)* If updating from old modded drivers, run **DDU (Display Driver Uninstaller)** in Windows Safe Mode to clean legacy driver files.
3. Run the downloaded installer `.exe`.
4. Select **NVIDIA Graphics Driver** and click **Next**.
5. Once installation finishes, your mutant GPU will be fully active without Error 43, and the classic **NVIDIA Control Panel** will appear on your desktop and context menu.

---

## 🛠️ How to Modify Official NVIDIA Drivers

You can easily patch any official clean NVIDIA driver installer directly from NVIDIA's website using OpenFrankenDriver tools.

### Method 1: Automated Script Build (Recommended)

1. **Prerequisites**:
   - Python 3.10+ (install `pefile`: `pip install pefile`)
   - 7-Zip archiver (`C:\Program Files\7-Zip\7z.exe`)
   - Windows 10/11 SDK (required for `makecat.exe` and `signtool.exe`)
2. **Download official clean NVIDIA driver**:
   - Visit [nvidia.com](https://www.nvidia.com/Download/index.aspx) and download the standard DCH WHQL installer (e.g. `621.14-desktop-win10-win11-64bit-international-dch-whql.exe`).
3. **Run the automated builder in one command**:
   ```cmd
   python scripts/build.py --nvidia-exe "C:\path\to\official_nvidia_driver.exe"
   ```
4. The builder executes all steps automatically:
   - Unpacks the official driver package;
   - Injects all 71 mutant GPU identifiers and the Error 43 bypass into `nv_dispi.inf`;
   - Generates and signs the recursive security catalog `nv_disp.cat` (271 files) with a DigiCert RFC3161 timestamp;
   - Integrates the classic standalone Win32 Control Panel (`nvcplui.exe`);
   - Compiles and signs the monolithic self-extracting `.exe` installer without `0xc0000005` errors.

---

### Method 2: Manual Step-by-Step Modification

If you want to perform the patch manually without automated scripts:

1. **Extraction**:
   - Extract the official NVIDIA installer using 7-Zip into a separate directory (e.g. `Driver_Unpacked`).
   - Delete the `NVPCF` folder (laptop battery driver, not needed on desktop).
   - If `nvngx_dlssd.dl_` in `Display.Driver\ngx-builtins\` is compressed, extract it with:  
     `expand nvngx_dlssd.dl_ nvngx_dlssd.dll`.

2. **Modify the INF file (`Display.Driver\nv_dispi.inf`)**:
   - Open `Display.Driver\nv_dispi.inf` in a text editor.
   - Find the device sections:
     - `[NVIDIA_Devices.NTamd64.10.0...17134]` — add device strings for Windows 10/11 (see `data/devices.json`).
     - `[NVIDIA_Devices.NTamd64.10.0...26200]` — add device strings for Windows 11 24H2 (see `data/devices.json`).
   - In registry sections `[nv_miscBase_addreg__01]`, `[nv_miscBase_addreg__05]`, `[nv_miscBase_addreg__09]`, `[nv_miscBase_addreg__10]`, `[nv_miscBase_addreg__16]`, add the Error 43 bypass directive:
     ```ini
     HKR,,RM1457588,%REG_DWORD%,1
     ```
   - At the bottom of `[Strings]`, add the model display names (from `data/devices.json`).

3. **Rebuild & Sign Security Catalog (`nv_disp.cat`)**:
   - Create a CDF manifest covering all 271 files in `Display.Driver`:
     `makecat -v nv_disp.cdf`
   - Sign `nv_disp.cat` using `signtool.exe`:
     `signtool sign /f your_cert.pfx /p 1234 /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 nv_disp.cat`
   - Copy the signed `nv_disp.cat` into `EACSupport\nv_disp.cat`.

4. **Certificate Integration**:
   - Place public certificate `FrankenDriver.cer` and `src/FrankenDriver.nvi` into `FrankenDriver\`.
   - In `setup.cfg`, attach `FrankenDriverCert` as a pre-requisite dependency before `Display.Driver`.

5. **Run Setup**:
   - Launch `setup.exe` from `Driver_Unpacked` — installation proceeds smoothly.

See [**docs/HOW_TO_BUILD.md**](docs/HOW_TO_BUILD.md) for full architectural details.

---

## 🛡️ Safety FAQ: Will it harm my GPU? Can I get banned?

1. **Hardware Safety**:
   - The driver does not touch voltages, power curves, or VBIOS firmware.
   - It only instructs Windows: *"Allow this GPU to initialize on the desktop PCIe bus and bypass Error 43"*.
   - All factory thermal protections, fan profiles, and TDP power limiters remain handled by the graphics card hardware.
2. **Anti-Cheats & Online Multiplayer (EAC, BattlEye, CS2 VAC, FaceIT)**:
   - Kernel binaries (`nvlddmkm.sys`, DirectX, Vulkan, CUDA) **are strictly untouched**.
   - Original Microsoft WHQL digital signatures are fully preserved.
   - Windows Test Mode (`testsigning`) remains **OFF**. Kernel-level anti-cheats verify genuine signed binaries and do not trigger flags or bans.

---

## 🧩 Supported GPUs (71 Models / Configurations)

Supports all Chinese desktop mutant graphics cards utilizing mobile GeForce RTX 40, RTX 30, RTX 20, GTX 16, and professional mobile RTX / Quadro silicon.

**Common models include:**
- **NVIDIA GeForce RTX 4070 Laptop GPU / RTX 4070m** (`DEV_2820`, `DEV_2860`)
- **NVIDIA GeForce RTX 4060 Laptop GPU / RTX 4060m** (`DEV_28A0`)
- **NVIDIA GeForce RTX 4050 Laptop GPU / RTX 4050m** (`DEV_28A1`)
- **NVIDIA GeForce RTX 3070 Laptop GPU / RTX 3070m** (`DEV_249D`, `DEV_249C`)
- **NVIDIA GeForce RTX 3060 Laptop GPU / RTX 3060m (6GB & 12GB)** (`DEV_2520`, `DEV_2521`, `DEV_2560`)
- **NVIDIA RTX A3000 Laptop GPU / RTX A3000m** (`DEV_24B8`)
- **NVIDIA RTX A4000 Laptop GPU / RTX A4000m** (`DEV_24B7`)
- **NVIDIA RTX A5000 Laptop GPU / RTX A5000m** (`DEV_24B6`)
- **NVIDIA GeForce RTX 3080 Laptop GPU / RTX 3080m** (`DEV_2484`, `DEV_249C`)
- **NVIDIA GeForce RTX 3050 Laptop GPU / RTX 3050m** (`DEV_25A2`, `DEV_25E2`)
- **NVIDIA GeForce RTX 2060 Laptop GPU / RTX 2060m** (`DEV_1F11`, `DEV_1F15`, `DEV_1F51`)
- **NVIDIA GeForce RTX 2070 Laptop GPU / RTX 2070m** (`DEV_1F10`, `DEV_1F14`, `DEV_1F50`)
- **NVIDIA GeForce RTX 2080 Laptop GPU / RTX 2080m** (`DEV_1E90`, `DEV_1ED0`)
- **NVIDIA GeForce GTX 1660 Ti Laptop GPU / GTX 1660m** (`DEV_2191`)
- **NVIDIA GeForce GTX 1650 Laptop GPU / GTX 1650m** (`DEV_1F99`, `DEV_1F9D`)

👉 [**View full hardware compatibility table (all 71 models and IDs)**](docs/SUPPORTED_DEVICES.md)

---

## 📚 Additional Documentation

- [**docs/HOW_TO_BUILD.md**](docs/HOW_TO_BUILD.md) — Comprehensive build and modification guide.
- [**docs/SUPPORTED_DEVICES.md**](docs/SUPPORTED_DEVICES.md) — Complete database of all 71 devices and PCI Hardware IDs.
- [**docs/ARCHITECTURE.md**](docs/ARCHITECTURE.md) — Technical deep-dive: NVI2, Win32 Control Panel, PE-structure, and catalog hashes.
- [**docs/TROUBLESHOOTING.md**](docs/TROUBLESHOOTING.md) — DDU clean install instructions, verification via `nvidia-smi`.

---

## 📜 License & Legal Disclaimer

- Distributed under the open-source [MIT License](LICENSE).
- NVIDIA, GeForce, RTX, and GTX are trademarks of NVIDIA Corporation.
- This project is an independent community project and is not affiliated with or endorsed by NVIDIA Corporation.
