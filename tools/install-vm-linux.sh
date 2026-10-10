#!/usr/bin/env bash
set -euo pipefail
[[ $(uname -s) == Linux ]] || { echo 'This installer is for a Linux host.' >&2; exit 1; }
privilege=()
if [[ $EUID != 0 ]]; then privilege=(sudo); fi
if command -v apt-get >/dev/null; then
    "${privilege[@]}" apt-get update
    "${privilege[@]}" apt-get install -y qemu-system-x86 qemu-utils libvirt-daemon-system libvirt-clients virt-viewer ovmf swtpm swtpm-tools python3
elif command -v dnf >/dev/null; then
    "${privilege[@]}" dnf install -y qemu-kvm qemu-img libvirt virt-viewer edk2-ovmf swtpm swtpm-tools python3
else
    echo 'Install QEMU/KVM, libvirt, OVMF Secure Boot firmware, swtpm, virt-viewer and Python 3 with your distro package manager.' >&2
    exit 1
fi
echo 'Packages installed. VM tasks use qemu:///session with user networking. No group membership or host audio changes requested.'
