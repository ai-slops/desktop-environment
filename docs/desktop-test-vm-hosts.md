# Windows guest tests on Windows and Linux hosts

The development defaults are **32 GiB RAM, 12 vCPUs, a sparse 96 GiB disk**, and a
40% per-vCPU execution limit while broadcasting. Allocation and execution limits
are separate: all 12 vCPUs are visible to Windows. Raise the limit to 100 explicitly
when full CPU use is appropriate. VM isolation protects host devices; CPU, RAM,
storage and network bandwidth remain shared resources. VM start checks available
RAM against the configured guest allocation plus a 4 GiB host reserve.

The application remains Windows-only. Linux support here means running the same
Windows guest acceptance tests from a Linux virtualization host.

## Windows host: existing Hyper-V

This PC already has a running Hyper-V service, VM PowerShell commands and
`vmconnect.exe`. Use those rather than installing another hypervisor during a
stream. `mise` was installed in the current user's scope via `winget install
--id jdx.mise --exact --source winget --scope user`. Open a new terminal for its
updated PATH, or use `%LOCALAPPDATA%/Microsoft/WinGet/Links/mise.exe` directly.

The script does not enable Windows features, reboot the host, add users to groups,
create/switch host network bridges or install host audio drivers. It connects the
VM to the existing **Default Switch**. If required components or permissions are
missing, it stops with an error. VM management requires an Administrator terminal. The mise wrapper selects the
execution policy for that PowerShell process only; it does not change the persisted
host policy. Group Policy restrictions still apply.

```powershell
mise trust mise.toml
mise run vm-plan
# In an Administrator PowerShell:
# Optional when the ISO is outside Downloads:
$env:DESKTOP_TEST_VM_ISO = 'C:\path\Windows11-x64.iso'
mise run vm-setup
mise run vm-status
mise run vm-start
mise run vm-console
```

On this PC the ISO can be discovered automatically as
`Downloads/Win11_25H2_English_x64.iso`. Setup creates a Generation 2 VM named
`DesktopEnvironment-AudioTest`, enables Secure Boot/vTPM, and leaves it **off**.
VM state/disk files live in ignored `target/vm-hyperv`. An ownership marker ties
subsequent actions to its exact VM ID. A different pre-existing VM is never reused.
Re-running setup on the owned, shut-down VM updates resources; it preserves its
disk and TPM. It refuses to reconfigure a running VM.

To change resources or lift the CPU limit while the VM is off:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/desktop-test-vm.ps1 -Action Setup -MemoryGiB 32 -Processors 12 -CpuMaximum 100
```

Complete Windows installation and use a license appropriate for that guest.
Host Windows licensing is not assumed to license the VM. Shut it down gracefully
with `mise run vm-stop`; VM tasks never automatically start on host boot.

## Linux host: QEMU/KVM with libvirt

Use an x86_64 host with hardware virtualization, `/dev/kvm` access and enough RAM.
Install mise through its [official installation instructions](https://mise.jdx.dev/getting-started.html).
The task wrapper installs system VM dependencies via the distro package manager;
QEMU, firmware and kernel access are not ordinary portable mise tool installs.
Debian/Ubuntu and Fedora installers are provided; other distributions get a clear
dependency list. Package installation can require `sudo` and distro-managed services.

```bash
mise trust mise.toml
mise run vm-install
export DESKTOP_TEST_VM_ISO="$HOME/Downloads/Windows11-x64.iso"
mise run vm-plan
mise run vm-setup
mise run vm-status
mise run vm-start
mise run vm-console
```

Definitions use `qemu:///session`, KVM (no software-emulation fallback), Q35, SATA,
automatic UEFI Secure Boot firmware, enrolled keys and swtpm TPM 2.0. Networking
uses the user-mode backend, without a new host bridge or incoming port forwards.
The VNC console listens on `127.0.0.1`. GPU, USB devices, audio and host folders
are not passed through. Guest disks/XML and an exact UUID marker are stored in
ignored `target/vm-libvirt`. Guest audio uses the same virtual driver fixture as
Hyper-V, keeping the acceptance procedure comparable.

Modern libvirt with Secure Boot firmware descriptors, swtpm and permission to
apply vCPU quotas is required. Missing firmware/cgroup permissions cause a failure;
the scripts do not silently remove the CPU cap or disable boot security. Setup
defines the VM without starting it. Existing VMs/disks are preserved; for resource
changes, review the owned VM in virt-manager/virsh while it is shut down. Use
`mise run vm-stop` to request a graceful guest shutdown.

## Common Windows guest setup

Keep the repository in the guest's own filesystem. A source ZIP can be made with
`git archive --format=zip --output=target/desktop-control-vm-source.zip HEAD`; this
contains committed source only, not the host's configs, target binaries or windows.
For Hyper-V, use Guest Service Interface / `Copy-VMFile` after installing Windows,
or download the committed source from your normal repository. Linux can use an
isolated ZIP directory served temporarily on loopback; the user-mode guest reaches
the host via `10.0.2.2`. Stop that local file server after the transfer. No permanent
shared host directory is needed.

Inside Windows, the VM-guarded bootstrap installs a checksum-pinned official mise
binary, Microsoft-signed C++ Build Tools with the recommended Windows SDK, then
the workspace Rust/just tools via mise. It needs an Administrator PowerShell in
the **guest** and never automatically restarts either computer:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/setup-desktop-test-guest.ps1 -Workspace C:\path\to\extracted-source -RunTests
```

If the C++ installer requests a reboot, restart only the guest and rerun setup.
The bootstrap checks the C++ installation before using mise: a fresh Windows
guest may not yet have the runtime needed to start its executable. The script
uses a standalone mise binary so App Installer/winget registration is not needed
in PowerShell Direct sessions. To
provide three guest render endpoints without host audio redirection, install
[Voicemeeter Banana](https://vb-audio.com/Voicemeeter/banana.htm) and
[VB-CABLE](https://vb-audio.com/Cable/index.htm) **inside the guest only**, following
their installers and guest-reboot instructions. Banana provides two virtual input
devices (Windows render endpoints); CABLE supplies a third. Use Banana's internal
clock when there is no physical guest audio output. Verify endpoint IDs and active
states rather than assuming names. These are third-party driver fixtures, not a
promise that all device drivers behave identically.

From the Windows guest run `mise run vm-test`, then perform the
[A/B/C switching acceptance](desktop-control-vm-testing.md). The VM guard accepts
Hyper-V and QEMU guests. Automated source checks alone do not certify live audio;
record the guest's routed/captured signal because host audio passthrough is disabled.

## Evidence and remaining setup

mise user installation is complete. Both host configurations reflect 32 GiB /
12 vCPUs; PowerShell parsing, read-only Windows plan and pure Linux XML tests are
checked on the Windows host. After retrying the canceled UAC request, Hyper-V
creation and startup succeeded on 2026-10-10. The owned VM reports **Running**,
32 GiB startup RAM, 12 vCPUs and a 40% CPU execution limit; its interactive console
was opened for Windows installation. Windows 11 Pro build 26200 was subsequently
installed; PowerShell Direct verified the guest model, 32 GiB RAM, 12 logical
processors and internet access. Toolchain/audio fixture setup and live acceptance
remain pending. Linux native libvirt definition, boot
and live guest audio checks require a Linux host and remain unverified.

## Keep one scoped Hyper-V management session

`tools/desktop-test-vm-session.ps1` keeps an elevated host worker and a guest
PowerShell Direct session alive for up to eight hours. It verifies the exact VM
ID from the ownership marker. It accepts only `Status`, `SyncSource`, `Setup`,
`Test`, `WorkspaceTest` and `Stop`; requests cannot specify host commands, paths,
another VM or guest code. The host remains available for broadcasting.

Provide `-CredentialPath` pointing to a `PSCredential` exported using
`Export-Clixml` by the same Windows account. Windows DPAPI encrypts the password;
keep this file in ignored `target/vm-session`, never in source control. Start the
worker once with UAC elevation. It does not install a service or change logon/group
permissions. A named mutex refuses a second worker for the same VM.

Requests are UUID-named JSON files under `target/vm-session/requests`, containing
only `{"action":"Status"}` or another permitted action. Publish complete files
atomically by writing a temporary file then renaming it to `.json`. Responses and
logs are under `responses`; `status.json` records process ID/expiry and idle
heartbeat. Submit `SyncSource` before setup/tests: the fixed
`target/desktop-control-vm-source.zip` is checked by SHA-256 after transfer and
extracted into a new hash-named guest directory, preserving prior guest checkouts.
`Stop` closes the worker/session while leaving the VM running. Guest test
operations run serially and a stop request takes effect after the current operation.
No credentials are included in request or response logs.

From a normal host terminal, queue operations with `mise run vm-session-status`,
`vm-sync-source`, `vm-guest-setup`, `vm-guest-test`, `vm-guest-test-all` or
`vm-session-stop`. These Windows-only worker tasks return a request ID and result/
log paths immediately. `vm-test` remains the direct **inside-guest** runner; the
worker tasks are the host-side alternative. Creating the ZIP still uses
`git archive --format=zip --output=target/desktop-control-vm-source.zip HEAD`.
Disable mise's host task auto-install if only managing VMs:
`$env:MISE_TASK_RUN_AUTO_INSTALL = 'false'`.

References: [Hyper-V Windows 11 generation/TPM setup](https://techcommunity.microsoft.com/blog/itopstalkblog/how-to-run-a-windows-11-vm-on-hyper-v/3713948),
[Hyper-V CPU limits](https://learn.microsoft.com/en-us/powershell/module/hyper-v/set-vmprocessor),
[libvirt domain configuration](https://libvirt.org/formatdomain.html),
[mise tasks and platform commands](https://mise.jdx.dev/tasks/task-configuration.html).
