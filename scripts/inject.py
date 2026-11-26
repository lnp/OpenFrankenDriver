import argparse
import json
import os
import re


SEC_17134 = "nvidia_devices.ntamd64.10.0...17134"
SEC_26200 = "nvidia_devices.ntamd64.10.0...26200"


def _parse_inf_sections(lines):
    """Parse INF lines into a lowercase section-name -> list of stripped lines map."""
    sections = {}
    current = None
    for line in lines:
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            current = s[1:-1].strip().lower()
            sections.setdefault(current, [])
        elif current is not None:
            sections[current].append(s)
    return sections


def _build_section_map(table_lines, templates, all_sections):
    """Resolve logical SectionXXX names to actual INF sections via donor HWIDs."""
    hwid_to_section = {}
    for line in table_lines:
        m = re.search(r"=\s*(Section\d+)\s*,\s*(PCI\\VEN_10DE&[^\s;]+)", line, re.I)
        if m:
            hwid_to_section[m.group(2).upper()] = m.group(1)

    resolved = {}
    for fallback_sec, donor_hwid in templates.items():
        actual = hwid_to_section.get(donor_hwid.upper())
        if actual:
            resolved[fallback_sec] = actual
        elif fallback_sec.lower() in all_sections:
            resolved[fallback_sec] = fallback_sec
        else:
            raise RuntimeError(
                f"Unable to resolve INF section for template {fallback_sec} (donor {donor_hwid})"
            )
    return resolved


def _rewrite_device_lines(entries, section_map, table_name):
    """Rewrite SectionXXX references using section_map and verify unique HWIDs."""
    rewritten = []
    seen_hwids = set()
    for entry in entries:
        m_sec = re.search(r"=\s*(Section\d+)\s*,\s*(PCI\\VEN_10DE&[^\s;]+)", entry, re.I)
        if not m_sec:
            raise RuntimeError(f"Invalid device entry in {table_name}: {entry}")
        orig_sec = m_sec.group(1)
        hwid = m_sec.group(2).upper()
        if hwid in seen_hwids:
            raise RuntimeError(f"Duplicate HWID detected in {table_name}: {hwid}")
        seen_hwids.add(hwid)

        target_sec = section_map.get(orig_sec, orig_sec)
        if target_sec != orig_sec:
            entry = re.sub(rf"=\s*{re.escape(orig_sec)}\s*,", f"= {target_sec},", entry, count=1)
        rewritten.append((entry.rstrip("\r\n") + "\n", target_sec))
    return rewritten


def _collect_rm_addreg_sections(target_install_sections, all_sections):
    """Find all nv_miscBase_addreg__XX sections referenced by target install sections."""
    rm_sections = set()
    for sec in target_install_sections:
        sec_lines = all_sections.get(sec.lower())
        if not sec_lines:
            raise RuntimeError(f"Resolved install section [{sec}] does not exist in INF!")
        body = "\n".join(sec_lines)
        for match in re.findall(r"nv_miscBase_addreg__\d+", body, re.I):
            rm_sections.add(match.lower())
    if not rm_sections:
        raise RuntimeError("No nv_miscBase_addreg__XX sections found for target install sections!")
    return rm_sections


def inject(src_inf, dst_inf=None, db_path=None):
    """Patch nv_dispi.inf with mutant GPU HWIDs, model strings, and RM1457588 bypass."""
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
    templates = db.get("section_templates", {})
    strings = db.get("strings", [])

    with open(src_inf, "r", encoding="latin1") as f:
        lines = f.readlines()

    all_sections = _parse_inf_sections(lines)
    if SEC_17134 not in all_sections or SEC_26200 not in all_sections:
        raise RuntimeError("Target NVIDIA_Devices sections missing from nv_dispi.inf!")

    map_17134 = _build_section_map(all_sections[SEC_17134], templates.get("devices_17134", {}), all_sections)
    map_26200 = _build_section_map(all_sections[SEC_26200], templates.get("devices_26200", {}), all_sections)

    entries_17134 = _rewrite_device_lines(devices_17134, map_17134, "devices_17134")
    entries_26200 = _rewrite_device_lines(devices_26200, map_26200, "devices_26200")

    used_install_sections = {sec for _, sec in entries_17134 + entries_26200}
    rm_sections = _collect_rm_addreg_sections(used_install_sections, all_sections)
    pending_rm = set(rm_sections)

    new_lines = []
    current_sec = None
    has_rm_in_current = False
    injected_17134 = False
    injected_26200 = False

    for line in lines:
        s = line.strip()

        # Remove laptop platform power controller INF reference (NVPCF)
        if "nvpcf.inf" in s.lower():
            continue

        if s.startswith("[") and s.endswith("]"):
            if current_sec and current_sec.lower() in pending_rm:
                if not has_rm_in_current:
                    new_lines.append("HKR,,RM1457588,%REG_DWORD%,1\n")
                pending_rm.remove(current_sec.lower())

            sec = s[1:-1].strip()
            sec_lower = sec.lower()
            current_sec = sec
            has_rm_in_current = False

            if sec_lower == SEC_17134:
                new_lines.append(line)
                new_lines.append("; ====================================================================\n")
                new_lines.append(f"; OpenFrankenDriver: {len(entries_17134)} Mutant / Laptop GPU entries for OS < 26200\n")
                new_lines.append("; ====================================================================\n")
                for dev_line, _ in entries_17134:
                    new_lines.append(dev_line)
                new_lines.append("; ====================================================================\n\n")
                injected_17134 = True
                continue

            if sec_lower == SEC_26200:
                new_lines.append(line)
                new_lines.append("; ====================================================================\n")
                new_lines.append(f"; OpenFrankenDriver: {len(entries_26200)} Mutant / Laptop GPU entries for OS >= 26200 (24H2+)\n")
                new_lines.append("; ====================================================================\n")
                for dev_line, _ in entries_26200:
                    new_lines.append(dev_line)
                new_lines.append("; ====================================================================\n\n")
                injected_26200 = True
                continue

            new_lines.append(line)
            continue

        if current_sec and current_sec.lower() in pending_rm and "rm1457588" in s.lower():
            has_rm_in_current = True

        new_lines.append(line)

    if current_sec and current_sec.lower() in pending_rm:
        if not has_rm_in_current:
            new_lines.append("HKR,,RM1457588,%REG_DWORD%,1\n")
        pending_rm.remove(current_sec.lower())

    if not injected_17134 or not injected_26200:
        raise RuntimeError(f"INF injection failed (17134={injected_17134}, 26200={injected_26200})")
    if pending_rm:
        raise RuntimeError(f"Failed to inject RM1457588 into sections: {sorted(pending_rm)}")

    new_lines.append("\n; ====================================================================\n")
    new_lines.append("; OpenFrankenDriver: Mutant Model String Definitions\n")
    new_lines.append("; ====================================================================\n")
    for st in strings:
        new_lines.append(st + "\n" if not st.endswith("\n") else st)
    new_lines.append("\n")

    with open(dst_inf, "w", encoding="latin1") as f:
        f.writelines(new_lines)

    print(f"[+] INF Patch Complete: {dst_inf}")
    print(f"    - Injected into [17134]: {injected_17134} ({len(entries_17134)} entries)")
    print(f"    - Injected into [26200]: {injected_26200} ({len(entries_26200)} entries)")
    print(f"    - Model strings added: {len(strings)}")
    print(f"    - Error 43 bypass (RM1457588) verified in: {sorted(rm_sections)}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Inject OpenFrankenDriver device definitions into NVIDIA INF")
    parser.add_argument("src", help="Source nv_dispi.inf file path")
    parser.add_argument("dst", nargs="?", default=None, help="Destination nv_dispi.inf file path (optional)")
    parser.add_argument("--db", default=None, help="Path to devices.json database (optional)")
    args = parser.parse_args()

    inject(args.src, args.dst, args.db)
