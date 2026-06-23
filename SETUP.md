# Malbox Setup Guide

## Disclaimer

This project requires some efforts (i.e., Preparing the Windows gold image (disabling Defender, creating users, setting up scheduled tasks)) which is manual work that is part of the scope of this graduation project. It is not automated because it requires direct interaction with a Windows environment and cannot be done headlessly without additional tooling (e.g., Packer, Windows ADK, or Ansible with WinRM).

Setting up the full environment can take a lot of time and resources to maintain the folder structure of images overlays and ovmf vars and codes, as well as for setting up the image to have disabled updates and windows defender downloading required tools such as Microsoft Build tools (MSVC), .Net framework.

For evaluators who want to see integrate the sandbox into their MalWhere infrastructure, a live service URL can be provided via Ngrok (from a machine of ours). Contact me for the endpoint once you begin testing.

**Email**: [mohd17sayed@gmail.com]
**Phone Number**: [+20-100-193-8180]

## 1. Build Requirements

### Rust Toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install stable
rustup target add x86_64-pc-windows-gnu
rustup component add rust-src
```

### System Packages (Debian/Ubuntu)

```bash
sudo apt install \
  build-essential \
  gcc-mingw-w64-x86-64 \
  pkg-config \
  libvirt-dev \
  libssl-dev \
  protobuf-compiler
```

### Building

```bash
# Generate proto code
cargo build -p malbox-proto

# Controller (Linux)
cargo build -p malbox-controller

```

The controller binary is at `target/debug/malbox`.

---

## 2. Runtime Requirements

### VM Infrastructure

```bash
sudo apt install \
  libvirt-daemon-system \
  libvirt-clients \
  qemu-system-x86 \
  virt-viewer \
  qemu-utils \
  ovmf


sudo usermod -a -G libvirt $USER
sudo usermod -a -G kvm $USER
```

**Important**: To allow the controller to create and manage qcow2 overlays for linked clones, QEMU must run as root. Edit `/etc/libvirt/qemu.conf` and set:

```
user = "root"
group = "root"
```

Then restart libvirt:

```bash
sudo systemctl restart libvirtd
```

Without this, QEMU will not have permission to write overlay files to the configured `overlay_dir`.

### Directory Structure

```bash
mkdir -p ~/vm-images/{master,overlays,reports/samples,nvrams}
```

Adjust paths in `config.EXAMPLE.toml` to match your setup:

```toml
listen_addr = "0.0.0.0:8080"

[libvirt]
uri = "qemu:///system"

[paths]
master_nvram  = "/home/<user>/vm-images/master/master_vars.fd"
master_image  = "/home/<user>/vm-images/master/testmaster.qcow2"
overlay_dir   = "/home/<user>/vm-images/overlays/"
report_dir    = "/home/<user>/vm-images/reports/"
nvram_dir     = "/home/<user>/vm-images/nvrams/"

[vm]
vcpus     = 2
memory_mb = 4096
```

### libvirt Network

The controller finds the VM's IP by looking up the MAC address in libvirt's DHCP leases on the `default` network.

```bash
sudo virsh net-start default       # start if not running
sudo virsh net-autostart default   # persist across host reboots
```

### Agent Assets

The following pre-compiled binaries must be placed in `crates/malbox-agent/assets/`:

| File | Purpose |
|---|---|
| `loader.exe` | 32-bit CAPE process injector |
| `loader_x64.exe` | 64-bit CAPE process injector |
| `capemon.dll` | 32-bit CAPE API monitoring DLL |
| `capemon_x64.dll` | 64-bit CAPE API monitoring DLL |
| `version.dll` | Sidecar DLL used by the loader for injection |
| `version_x64.dll` | 64-bit sidecar DLL |

These are precompiled binaries. They are not built from source as part of this project.

---

## 3. Preparing the Windows Gold Image

This is the most labor-intensive part of the setup. It must be done manually once. The gold image is a Windows 10/11 IoT Enterprise VM with specific configuration that allows the sandbox to operate correctly.

### 3.1 Create the Base VM

1. Install Windows 10 or Windows 11 IoT Enterprise in a libvirt VM using the `qemu:///system` connection.
2. Use UEFI boot (OVMF). The domain template requires it.
3. Install the VirtIO drivers (storage, network, balloon) so QEMU can use paravirtualized devices.
4. Install QEMU Guest Agent (optional but recommended for debugging — not required for operation).

### 3.2 Create the `mohamed` User

The agent is hardcoded to use `C:\Users\mohamed\Desktop\sample` as the working directory. Create a local user account named `mohamed` (all lowercase):

```powershell
# In the Windows VM, run as Administrator
net user mohamed <password> /add
net localgroup Administrators mohamed /add
```

### 3.3 Create the Working Directory

```powershell
mkdir C:\Users\mohamed\Desktop\sample
```

This is where the agent writes capemon config files, the loader executes, and temporary artifacts are stored during analysis.

### 3.4 Disable Windows Defender

Windows Defender will quarantine `capemon.dll` and the loader. It must be fully disabled. There are two approaches:

#### Option A: Group Policy (Recommended for Enterprise editions)

```powershell
# Run as Administrator
gpedit.msc
```

Navigate to: **Computer Configuration → Administrative Templates → Windows Components → Microsoft Defender Antivirus**

Set the following policies:
- **Turn off Microsoft Defender Antivirus** → Enabled
- **Real-time Protection: Turn off real-time protection** → Enabled
- **Behavior Monitoring: Turn off behavior monitoring** → Enabled
- **Network Inspection System: Turn off** → Enabled

#### Option B: Registry Keys (Works on all editions)

```powershell
# Run as Administrator
reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows Defender" /v DisableAntiSpyware /t REG_DWORD /d 1 /f
reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows Defender\Real-Time Protection" /v DisableRealtimeMonitoring /t REG_DWORD /d 1 /f
reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows Defender\Real-Time Protection" /v DisableBehaviorMonitoring /t REG_DWORD /d 1 /f
reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows Defender\Real-Time Protection" /v DisableOnAccessProtection /t REG_DWORD /d 1 /f
reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows Defender\Real-Time Protection" /v DisableScanOnRealtimeEnable /t REG_DWORD /d 1 /f
```

After applying, reboot the VM.

#### Verify Defender is Off

```powershell
Get-MpPreference | Select-Object DisableRealtimeMonitoring
# Should return True
```

### 3.5 Auto-Login (Optional but Recommended)

If the VM reboots (e.g., after driver install), auto-login ensures it comes back to the desktop without manual intervention:

```powershell
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon" /v AutoAdminLogon /t REG_SZ /d 1 /f
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon" /v DefaultUserName /t REG_SZ /d mohamed /f
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon" /v DefaultPassword /t REG_SZ /d <password> /f
```

### 3.6 Scheduled Task for Agent Startup

The agent must be running on port 50055 when the controller connects. Create a Windows scheduled task that starts `malbox-agent.exe` on boot:

1. Copy `malbox-agent.exe` to the VM (e.g., `C:\Users\mohamed\Desktop\malbox-agent.exe`).
2. Create the scheduled task:

```powershell
# Run as Administrator
schtasks /create /tn "MalboxAgent" /tr "C:\Users\mohamed\Desktop\malbox-agent.exe" /sc onboot /ru mohamed /rp <password> /rl HIGHEST
```

Or use Task Scheduler GUI:
- **Task Name**: MalboxAgent
- **Trigger**: At startup
- **Action**: Start `C:\Users\mohamed\Desktop\malbox-agent.exe`
- **Run as**: `mohamed` with highest privileges

**Note**: The agent does not need to be in a specific directory to start, but it will create a work directory at `C:\Users\mohamed\Desktop\sample` when analysis begins.

### 3.7 Building the Agent on the VM

Building the agent natively inside the Windows VM using the Microsoft Visual C++ (MSVC) toolchain is the recommended approach. It avoids cross-compilation issues and produces a native Windows binary.

#### Prerequisites

Install the following inside the Windows VM:

1. **Rust** — Download and run [rustup-init.exe](https://rustup.rs) from the VM's browser.
2. **Visual Studio Build Tools** — Download the [Build Tools for Visual Studio](https://visualstudio.microsoft.com/visual-cpp-build-tools/). During installation, select the **"Desktop development with C++"** workload. This provides `cl.exe` (the MSVC compiler) and the necessary libraries. The Community edition is free.

#### Copying the Project into the VM

Use `virt-copy-in` from `libguestfs-tools` on the Linux host to copy the project source into the VM:

```bash
# Install libguestfs-tools on the host
sudo apt install libguestfs-tools

# Shut down the VM first (required for virt-copy-in)
virsh shutdown <vm-name>

# Copy the crates directory into the VM
virt-copy-in -d <vm-name> crates/malbox-agent /Users/mohamed/Desktop/

# Copy the workspace Cargo.toml and malbox-proto crate (needed for building)
virt-copy-in -d <vm-name> Cargo.toml /Users/mohamed/Desktop/malbox-agent/
virt-copy-in -d <vm-name> crates/malbox-proto /Users/mohamed/Desktop/malbox-agent/crates/

# Start the VM again
virsh start <vm-name>
```

#### Building

Inside the Windows VM, open a **Developer Command Prompt for VS** (search for "Developer Command Prompt" in the Start menu — this sets up the MSVC environment):

```powershell
cd C:\Users\mohamed\Desktop\malbox-agent
cargo build --release
```

The release binary will be at `C:\Users\mohamed\Desktop\malbox-agent\target\release\malbox-agent.exe`.

### 3.8 Disable Windows Update (Optional)

To prevent the VM from changing state between analysis runs:

```powershell
reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU" /v NoAutoUpdate /t REG_DWORD /d 1 /f
```

### 3.9 Shutdown and Create the Gold Image

1. Shut down the VM cleanly.
2. On the host, find the VM's disk path:

```bash
virsh domblklist <vm-name>
```

3. Copy the disk to the master image location:

```bash
cp /var/lib/libvirt/images/<vm-name>.qcow2 ~/vm-images/master/testmaster.qcow2
```

4. Export the NVRAM (UEFI variables):

```bash
cp /var/lib/libvirt/qemu/nvram/<vm-name>_VARS.fd ~/vm-images/master/master_vars.fd
```


---

## 4. Running

```bash
# Build
cargo build -p malbox-controller

# Run the controller (reads config.EXAMPLE.toml by default)
./target/debug/malbox
```

The server starts on `http://0.0.0.0:8080`. Open it in a browser to access the test frontend (Which is different from the main MalWhere frontend, this one was made purely to test the sandbox) .

### API Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/` | Web UI (embedded in binary) |
| `POST` | `/jobs` | Upload a sample (`multipart/form-data`). Returns `{job_id, status}` |
| `GET` | `/jobs/{id}` | Poll job status and result |
| `GET` | `/jobs/{id}/events` | SSE stream of live events (`status`, `completed`, `analysis_error`) |

### curl Examples

```bash
# Submit a sample
curl -s -F "file=@sample.exe" http://localhost:8080/jobs

# Check status
curl -s http://localhost:8080/jobs/<JOB_ID>

# Stream events
curl -N http://localhost:8080/jobs/<JOB_ID>/events
```

---
