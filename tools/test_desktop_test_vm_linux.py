"""Pure configuration tests: these never launch a VM or operate host devices."""
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

from desktop_test_vm_linux import domain_xml


class VmConfigurationTests(unittest.TestCase):
    def configuration(self, iso=Path('/tmp/Windows " & install.iso')):
        return ET.fromstring(domain_xml(iso, "00000000-0000-0000-0000-000000000001", 32, 12, 40))

    def test_requested_memory_and_cpu_allocation(self):
        root = self.configuration()
        self.assertEqual(root.findtext("memory"), "32")
        self.assertEqual(root.find("memory").get("unit"), "GiB")
        self.assertEqual(root.findtext("currentMemory"), "32")
        self.assertEqual(root.findtext("vcpu"), "12")
        self.assertEqual(root.find("cpu/topology").get("cores"), "12")
        self.assertEqual(root.findtext("cputune/quota"), "40000")

    def test_iso_paths_are_escaped_and_read_only(self):
        path = Path('/tmp/Windows " & install.iso')
        disk = self.configuration(path).find("devices/disk[@device='cdrom']")
        self.assertEqual(disk.find("source").get("file"), str(path))
        self.assertIsNotNone(disk.find("readonly"))

    def test_windows_11_boot_security(self):
        root = self.configuration()
        self.assertEqual(root.find("os").get("firmware"), "efi")
        for name in ["secure-boot", "enrolled-keys"]:
            self.assertEqual(root.find(f"os/firmware/feature[@name='{name}']").get("enabled"), "yes")
        self.assertEqual(root.find("devices/tpm/backend").get("version"), "2.0")

    def test_host_devices_and_external_console_are_not_exposed(self):
        root = self.configuration()
        for name in ["hostdev", "filesystem", "sound", "audio"]:
            self.assertIsNone(root.find(f"devices/{name}"))
        self.assertEqual(root.find("devices/graphics").get("listen"), "127.0.0.1")
        self.assertEqual(root.find("devices/interface").get("type"), "user")
        self.assertEqual(root.findtext("sysinfo/system/entry[@name='manufacturer']"), "QEMU")


if __name__ == "__main__":
    unittest.main()
