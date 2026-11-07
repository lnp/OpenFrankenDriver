import os
import json
import sys
import argparse

def inject(src_inf, dst_inf=None, db_path=None):
    if dst_inf is None:
        dst_inf = src_inf

    if db_path is None:
        base_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        db_path = os.path.join(base_dir, "data", "devices.json")

    if not os.path.exists(db_path):
        raise FileNotFoundError(f"Device database not found at: {db_path}")

    with open(db_path, "r", encoding="utf-8") as f:
        db = json.load(f)

    devices_17134 = db.get("devices_17134", [])
    devices_26200 = db.get("devices_26200", [])
    strings = db.get("strings", [])

    with open(src_inf, "r", encoding="latin1") as f:
        lines = f.readlines()

    new_lines = []
    current_sec = None

    # Track sections where we need to inject Error 43 bypass (RM1457588 = 1)
    rm_sections = {
        "nv_miscbase_addreg__01",
        "nv_miscbase_addreg__05",
        "nv_miscbase_addreg__09",
        "nv_miscbase_addreg__10",
        "nv_miscbase_addreg__16"
    }

    injected_17134 = False
    injected_26200 = False

    for line in lines:
        s = line.strip()

        # Remove laptop battery driver reference (NVPCF)
        if "nvpcf.inf" in s.lower():
            continue

        if s.startswith("[") and s.endswith("]"):
            sec = s[1:-1].strip()
            sec_lower = sec.lower()

            # Inject RM1457588 before exiting target section
            if current_sec and current_sec.lower() in rm_sections:
                new_lines.append("HKR,,RM1457588,%REG_DWORD%,1\n")
                rm_sections.remove(current_sec.lower())

            # Section for Windows 10 and Windows 11 < 26200
            if sec_lower == "nvidia_devices.ntamd64.10.0...17134":
                new_lines.append(line)
                new_lines.append("; ====================================================================\n")
                new_lines.append("; OpenFrankenDriver: 68 Mutant / Laptop GPU models for OS < 26200\n")
                new_lines.append("; ====================================================================\n")
                for d in devices_17134:
                    new_lines.append(d + "\n" if not d.endswith("\n") else d)
                new_lines.append("; ====================================================================\n\n")
                injected_17134 = True
                current_sec = sec
                continue

            # Section for Windows 11 24H2+ (Build 26200+)
            if sec_lower == "nvidia_devices.ntamd64.10.0...26200":
                new_lines.append(line)
                new_lines.append("; ====================================================================\n")
                new_lines.append("; OpenFrankenDriver: 68 Mutant / Laptop GPU models for OS >= 26200 (24H2)\n")
                new_lines.append("; ====================================================================\n")
                for d in devices_26200:
                    new_lines.append(d + "\n" if not d.endswith("\n") else d)
                new_lines.append("; ====================================================================\n\n")
                injected_26200 = True
                current_sec = sec
                continue

            current_sec = sec
            new_lines.append(line)
            continue

        new_lines.append(line)

    if current_sec and current_sec.lower() in rm_sections:
        new_lines.append("HKR,,RM1457588,%REG_DWORD%,1\n")

    # Append mutant model display names to [Strings]
    new_lines.append("\n; ====================================================================\n")
    new_lines.append("; OpenFrankenDriver: Mutant Model String Definitions\n")
    new_lines.append("; ====================================================================\n")
    for st in strings:
        new_lines.append(st + "\n" if not st.endswith("\n") else st)
    new_lines.append("\n")

    with open(dst_inf, "w", encoding="latin1") as f:
        f.writelines(new_lines)

    print(f"[+] INF Patch Complete: {dst_inf}")
    print(f"    - Injected into [17134]: {injected_17134} ({len(devices_17134)} devices)")
    print(f"    - Injected into [26200]: {injected_26200} ({len(devices_26200)} devices)")
    print(f"    - Model strings added: {len(strings)}")
    print(f"    - Error 43 bypass (RM1457588) added into core registers.")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Inject OpenFrankenDriver device definitions into NVIDIA INF")
    parser.add_argument("src", help="Source nv_dispi.inf file path")
    parser.add_argument("dst", nargs="?", default=None, help="Destination nv_dispi.inf file path (optional, overwrites source if omitted)")
    parser.add_argument("--db", default=None, help="Path to devices.json database (optional)")
    args = parser.parse_args()

    inject(args.src, args.dst, args.db)
