<div align="center">

[![Русский](https://img.shields.io/badge/Язык-Русский-1f883d?style=for-the-badge)](README.md)
[![English](https://img.shields.io/badge/Language-English-0969da?style=for-the-badge)](#)

<br>

<a href="https://github.com/lnp/OpenFrankenDriver"><img src="assets/header.png" alt="OpenFrankenDriver" width="840"></a>
<br>

**Completely open-source, free, and secure NVIDIA driver for Chinese mutant GPUs**  
*(RTX 4070m, RTX 4060m, RTX 3070m, RTX 3060m, RTX 2060m, GTX 1660m and 65+ other models powered by AD106/GA104/GA106/TU106 mobile silicon)*

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![NVIDIA Driver Version](https://img.shields.io/badge/NVIDIA-621.14%20WHQL-76B900?logo=nvidia)](https://www.nvidia.com)
[![Anti-Cheat Ready](https://img.shields.io/badge/Anti--Cheat-EAC%20%7C%20BattlEye%20%7C%20VAC%20Passed-success)](docs/ARCHITECTURE.md)
[![Platform](https://img.shields.io/badge/Windows-10%20%7C%2011%20(24H2%2B)-0078D6?logo=windows)](docs/SUPPORTED_DEVICES.md)
[![Boosty](https://img.shields.io/badge/Boosty-Support%20Project-F15F2C?logo=boosty)](https://boosty.to/defrag)

---

### 📥 [Download Pre-built OpenFrankenDriver 621.14 (Google Drive)](https://drive.google.com/file/d/19S8CCdlnbFrJYeqBKCQSK9nPMyMB8aFr/view?usp=sharing)
*(Size: ~1.07 GB • Stable 621.14 WHQL • Tested & verified out-of-the-box on Windows 10 & Windows 11 24H2 • Bundled with standalone classic Control Panel)*

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

1. **Download the pre-built installer** via the link above or directly from [Google Drive](https://drive.google.com/file/d/19S8CCdlnbFrJYeqBKCQSK9nPMyMB8aFr/view?usp=sharing).
2. *(Recommended)* If updating from old modded drivers, run **DDU (Display Driver Uninstaller)** in Windows Safe Mode to clean legacy driver files.
3. Run the downloaded installer `.exe`.
4. Select **NVIDIA Graphics Driver** and click **Next**.
5. Once installation finishes, your mutant GPU will be fully active without Error 43, and the classic **NVIDIA Control Panel** will appear on your desktop and context menu.

---

## 🛠️ How to Modify Official NVIDIA Drivers

You can easily patch any official clean NVIDIA driver installer directly from NVIDIA's website using OpenFrankenDriver tools.

### Method 1: Automated Script Build (Recommended)

1. Install Python 3.10+ and requirements:
   ```cmd
   pip install pefile
   ```
2. Download official clean NVIDIA driver package (621.xx WHQL or newer) from nvidia.com.
3. Run the automated builder:
   ```cmd
   python scripts/build.py --nvidia-exe "path\to\official_installer.exe"
   ```
   The script automatically:
   - Extracts payload;
   - Injects 71 mutant GPU configurations into sections `17134` (Win10) and `26200` (Win11 24H2);
   - Removes vendor hardware locks and fixes Code 43 (`RM1457588`);
   - Compiles and signs catalog `nv_disp.cat` with official RFC3161 DigiCert timestamp;
   - Integrates the offline Win32 Control Panel (`nvcplui.exe`);
   - Packages a monolithic SFX installer.

---

### Method 2: Manual Modification

If you prefer step-by-step customization:

1. **Extraction**:
   Unpack the official NVIDIA installer `.exe` with 7-Zip.

2. **Patching INF**:
   - Open `Display.Driver\nv_dispi.inf`;
   - Locate `[NVIDIA_Devices.NTamd64.10.0...17134]` and `[NVIDIA_Devices.NTamd64.10.0...26200]`;
   - Append IDs from [`data/devices.json`](data/devices.json);
   - Ensure the Code 43 fix flag is present:
     ```ini
     HKR,,RM1457588,%REG_DWORD%,1
     ```

3. **Catalog Creation & Signing (`nv_disp.cat`)**:
   - Generate CDF file for `Display.Driver`;
   - Compile catalog using Windows SDK `makecat`:
     ```cmd
     makecat -v nv_disp.cdf
     ```
   - Sign catalog via `signtool.exe` with RFC3161 DigiCert timestamp:
     ```cmd
     signtool sign /f certs/OpenFrankenDriver.pfx /p 1234 /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 nv_disp.cat
     ```
   - Copy signed `nv_disp.cat` into `EACSupport\` folder.

4. **Control Panel Integration**:
   - Create subfolder `Display.NVCPL`;
   - Extract `nvcplui.exe`, `nvcpluir.dll`, `NvGpuUtilization.exe`, `nvImage.dll` from `Display.Driver\NVCPL\*.appx`;
   - Provide `NVCPL.nvi` configuration and native `install_helper.exe`.

5. **Packaging**:
   - Compress with 7-Zip into `payload.7z` (LZMA2 ultra);
   - Prepend `PackageLauncher.exe` SFX stub;
   - Sign final `.exe` with project certificate.

---

## 🔒 Security & WHQL Integrity

OpenFrankenDriver is strictly non-invasive:
- **No patched kernel drivers**: All core binaries (`nvlddmkm.sys`, DirectX, Vulkan, CUDA) remain byte-for-byte identical to official NVIDIA WHQL release;
- Does **not** require dangerous `testsigning on` modes;
- Anti-cheats **Easy Anti-Cheat (Apex Legends, Fortnite)**, **BattlEye (R6 Siege)**, **VAC (CS2)**, and **Vanguard (Valorant)** work without restrictions because kernel drivers are genuine and uncompromised.

---

## 🤝 Contributing & Hardware Submissions

If your mutant card is not listed yet, please open an [Issue](https://github.com/lnp/OpenFrankenDriver/issues) with your Hardware ID (from Device Manager) and GPU-Z screenshot. We will gladly add support.

---

## 📄 License

Build scripts, device mappings, and tools are licensed under the **MIT License**. NVIDIA binaries and drivers are proprietary property of NVIDIA Corporation.
