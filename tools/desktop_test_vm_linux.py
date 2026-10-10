"""Guest definition/lifecycle for Linux hosts; plan mode also works on Windows."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import uuid
import xml.etree.ElementTree as ET

NAME = "DesktopEnvironment-AudioTest"
URI = "qemu:///session"
ROOT = Path(__file__).resolve().parent.parent / "target" / "vm-libvirt"


def run(*args):
    return subprocess.run(args, check=True, text=True, capture_output=True).stdout.strip()


def element(parent, tag, text=None, **attributes):
    child = ET.SubElement(parent, tag, attributes)
    child.text = None if text is None else str(text)
    return child


def domain_xml(iso, identity, memory, processors, maximum):
    root = ET.Element("domain", type="kvm")
    element(root, "name", NAME)
    element(root, "uuid", identity)
    element(root, "memory", memory, unit="GiB")
    element(root, "currentMemory", memory, unit="GiB")
    element(root, "vcpu", processors, placement="static")
    tuning = element(root, "cputune")
    element(tuning, "period", 100000)
    element(tuning, "quota", maximum * 1000)
    cpu = element(root, "cpu", mode="host-passthrough", check="none")
    element(cpu, "topology", sockets="1", cores=str(processors), threads="1")
    os_node = element(root, "os", firmware="efi")
    element(os_node, "type", "hvm", arch="x86_64", machine="q35")
    firmware = element(os_node, "firmware")
    element(firmware, "feature", name="secure-boot", enabled="yes")
    element(firmware, "feature", name="enrolled-keys", enabled="yes")
    element(os_node, "boot", dev="cdrom")
    element(os_node, "boot", dev="hd")
    element(os_node, "smbios", mode="sysinfo")
    system = element(element(root, "sysinfo", type="smbios"), "system")
    element(system, "entry", "QEMU", name="manufacturer")
    element(system, "entry", "DesktopEnvironment Audio Test VM", name="product")
    features = element(root, "features")
    element(features, "acpi")
    element(features, "apic")
    element(root, "clock", offset="localtime")
    element(root, "on_poweroff", "destroy")
    element(root, "on_reboot", "restart")
    element(root, "on_crash", "preserve")
    devices = element(root, "devices")
    for device, path, target, kind in [
        ("disk", ROOT / "windows.qcow2", "sda", "qcow2"),
        ("cdrom", iso, "sdb", "raw"),
    ]:
        disk = element(devices, "disk", type="file", device=device)
        element(disk, "driver", name="qemu", type=kind)
        element(disk, "source", file=str(path))
        element(disk, "target", dev=target, bus="sata")
        if device == "cdrom":
            element(disk, "readonly")
    element(devices, "controller", type="sata", index="0")
    interface = element(devices, "interface", type="user")
    element(interface, "model", type="e1000e")
    graphics = element(devices, "graphics", type="vnc", autoport="yes", listen="127.0.0.1")
    element(graphics, "listen", type="address", address="127.0.0.1")
    element(element(devices, "video"), "model", type="vga", vram="16384", heads="1", primary="yes")
    element(devices, "input", type="tablet", bus="usb")
    element(devices, "controller", type="usb", model="qemu-xhci")
    element(element(devices, "tpm", model="tpm-crb"), "backend", type="emulator", version="2.0", persistent_state="yes")
    # Virtual audio drivers are installed in the guest. No host sound/USB/GPU passthrough.
    ET.indent(root)
    return ET.tostring(root, encoding="unicode")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["plan", "setup", "start", "status", "stop", "console"])
    parser.add_argument("--iso", default=os.environ.get("DESKTOP_TEST_VM_ISO"))
    parser.add_argument("--memory-gib", type=int, default=32)
    parser.add_argument("--processors", type=int, default=12)
    parser.add_argument("--cpu-maximum", type=int, default=40)
    args = parser.parse_args()
    if not (4 <= args.memory_gib <= 64 and 2 <= args.processors <= 32 and 1 <= args.cpu_maximum <= 100):
        parser.error("Resource values are out of range")
    iso = Path(args.iso).expanduser().resolve() if args.iso else None
    if args.action == "plan":
        print(domain_xml(iso or Path("WINDOWS_11_X64_ISO_REQUIRED"), str(uuid.uuid4()), args.memory_gib, args.processors, args.cpu_maximum))
        return
    if sys.platform != "linux":
        parser.error("Only plan mode is available outside Linux")
    if platform.machine() not in {"x86_64", "AMD64"}:
        parser.error("The Windows x64 guest requires an x86_64 KVM host")
    for tool in ["virsh", "qemu-img", "swtpm"]:
        if not shutil.which(tool):
            parser.error(f"Missing {tool}; run mise run vm-install")
    def virsh(*parts):
        return run("virsh", "--connect", URI, *parts)
    marker = ROOT / "owner.json"
    identities = set(virsh("list", "--all", "--uuid").splitlines())
    owner = json.loads(marker.read_text()) if marker.exists() else None
    if owner and owner["uuid"] not in identities:
        parser.error("Saved VM identity is missing. Inspect existing disk/state; nothing was replaced.")
    if owner and virsh("domname", owner["uuid"]) != NAME:
        parser.error("Saved identity belongs to a different VM")
    if not owner and NAME in virsh("list", "--all", "--name").splitlines():
        parser.error("An unrelated VM already uses the test name")
    if args.action == "setup":
        if not iso or not iso.is_file():
            parser.error("Set DESKTOP_TEST_VM_ISO to a Windows 11 x64 ISO")
        if not os.access("/dev/kvm", os.R_OK | os.W_OK):
            parser.error("KVM access is required; software emulation is intentionally disabled")
        if owner:
            parser.error("VM already exists. Resources are preserved; use virsh/virt-manager to review changes while shut down.")
        ROOT.mkdir(parents=True, exist_ok=True)
        disk = ROOT / "windows.qcow2"
        if disk.exists():
            parser.error("Existing unregistered disk is preserved; inspect it manually")
        identity = str(uuid.uuid4())
        xml = ROOT / "domain.xml"
        xml.write_text(domain_xml(iso, identity, args.memory_gib, args.processors, args.cpu_maximum))
        run("qemu-img", "create", "-f", "qcow2", str(disk), "96G")
        virsh("define", "--validate", str(xml))
        marker.write_text(json.dumps({"uuid": identity, "name": NAME}, indent=2))
        print("VM defined and left off. Run vm-start and vm-console to install Windows.")
    else:
        if not owner:
            parser.error("Create the owned VM first with vm-setup")
        identity = owner["uuid"]
        if args.action == "console":
            subprocess.run(["virt-viewer", "--connect", URI, identity], check=True)
        else:
            command = {"start": "start", "stop": "shutdown", "status": "dominfo"}[args.action]
            if args.action == "start":
                info = ET.fromstring(virsh("dumpxml", identity))
                memory = info.find("memory")
                if memory is None or memory.get("unit") != "KiB":
                    parser.error("Cannot verify libvirt's normalized memory allocation")
                available = next(int(line.split()[1]) for line in Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemAvailable:"))
                if available < int(memory.text) + 4 * 1024 * 1024:
                    parser.error("Insufficient available RAM for this guest plus a 4 GiB host reserve")
            print(virsh(command, identity))


if __name__ == "__main__":
    main()
