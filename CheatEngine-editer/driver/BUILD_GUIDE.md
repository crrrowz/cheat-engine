# 🔧 KMem Driver — Build & Deployment Guide

## Prerequisites

### 1. Windows Driver Kit (WDK)

The WDK provides `ntoskrnl.lib` which exports all kernel API functions.

**Install via Visual Studio Installer:**
1. Open **Visual Studio Installer**
2. Click **Modify** on your VS installation
3. Go to **Individual Components**
4. Search and check: **"Windows Driver Kit"**
5. Click **Modify**

**Or install standalone:**
- Download from: [WDK Download](https://learn.microsoft.com/en-us/windows-hardware/drivers/download-the-wdk)
- Install the **Windows SDK** first, then the **WDK add-on**

### 2. Rust MSVC Toolchain (Required for Kernel)

Kernel drivers **must** use the MSVC target (not GNU) because they link against `.lib` files from the WDK:

```powershell
rustup target add x86_64-pc-windows-msvc
```

> **Note:** You need the MSVC linker (`link.exe`) from Visual Studio Build Tools.
> Install via: `winget install Microsoft.VisualStudio.2022.BuildTools`
> Then add the **C++ Build Tools** workload.

---

## Building the Driver

### Option A: Using cargo (with MSVC target override)

```powershell
cd driver

# Build for MSVC target (required for kernel .lib linking)
cargo build --release --target x86_64-pc-windows-msvc
```

The output will be at:
```
driver\target\x86_64-pc-windows-msvc\release\kmem_driver.dll
```

### Rename .dll → .sys

Windows expects kernel drivers to have a `.sys` extension:

```powershell
copy "target\x86_64-pc-windows-msvc\release\kmem_driver.dll" "kmem_driver.sys"
```

### Option B: Using cargo-make (automated)

Install cargo-make:
```powershell
cargo install cargo-make
```

Create a `Makefile.toml` with build + rename steps, then:
```powershell
cargo make build-driver
```

---

## Signing the Driver

### Why Signing is Required

Windows 10/11 (64-bit) **refuses to load unsigned drivers** by default.
For development/testing, you have two options:

### Option 1: Test Signing Mode (Recommended for Development)

This allows loading self-signed drivers:

```powershell
# Enable test signing (requires reboot)
bcdedit /set testsigning on

# Create a self-signed certificate
makecert -r -pe -ss PrivateCertStore -n "CN=KMem Dev Certificate" KMemCert.cer

# Sign the driver
signtool sign /s PrivateCertStore /n "KMem Dev Certificate" /t http://timestamp.digicert.com /fd sha256 kmem_driver.sys
```

> **Warning:** Test signing mode shows a watermark on the desktop.
> This is normal and expected.

### Option 2: Disable Driver Signature Enforcement (Temporary)

This lasts until the next reboot:

1. Hold **Shift** and click **Restart**
2. Go to **Troubleshoot → Advanced → Startup Settings → Restart**
3. Press **7** (Disable driver signature enforcement)

---

## Loading the Driver

### Using sc.exe (Service Control Manager)

```powershell
# Register the driver as a kernel service
sc.exe create KMem type=kernel binPath="C:\full\path\to\kmem_driver.sys"

# Start the driver
sc.exe start KMem

# Verify it's running
sc.exe query KMem
```

### Verify the Device Exists

```powershell
# Check if the device is accessible
python -c "import ctypes; h = ctypes.windll.kernel32.CreateFileW('\\\\.\\KMem', 0xC0000000, 3, None, 3, 0, None); print('OK' if h != -1 else 'FAILED')"
```

### Stop and Remove

```powershell
# Stop the driver
sc.exe stop KMem

# Remove the service registration
sc.exe delete KMem
```

---


### 1. Enable test signing (reboot required)
```
bcdedit /set testsigning on
```
### 2. Build
```
cd driver
cargo build --release --target x86_64-pc-windows-msvc
```

### 3. Rename to .sys
```
copy target\x86_64-pc-windows-msvc\release\kmem_driver.dll kmem_driver.sys
```
### 4. Load
```
sc.exe create KMem type=kernel binPath="C:\full\path\kmem_driver.sys"
sc.exe start KMem
```
### 5. Run
```
python main.py
```

## Debugging

### View Driver Debug Output

The driver uses `DbgPrint` with the `[KMem]` prefix. To see output:

1. **DebugView (Sysinternals):**
   - Download from: [DebugView](https://learn.microsoft.com/en-us/sysinternals/downloads/debugview)
   - Run as Administrator
   - Enable: **Capture → Capture Kernel**
   - Filter on: `KMem`

2. **WinDbg:**
   ```
   kd> ed nt!Kd_DEFAULT_Mask 0xFFFFFFFF
   kd> g
   ```

### Expected Debug Output on Load

```
[KMem] === KMem Driver Loading ===
[KMem] Device created: \\Device\\KMem
[KMem] Symbolic link created: \\?\KMem -> \\Device\\KMem
[KMem] === KMem Driver Loaded Successfully ===
[KMem] IOCTL_READ_MEMORY = 0x222004
```

---

## Troubleshooting

| Problem | Cause | Solution |
|---------|-------|----------|
| `cargo build` fails: `link.exe not found` | MSVC Build Tools not installed | Install VS Build Tools with C++ workload |
| `cargo build` fails: `ntoskrnl.lib not found` | WDK not installed or wrong path | Install WDK, verify path in `build.rs` |
| `sc start` fails: `access denied` | Not running as Administrator | Open PowerShell as Admin |
| `sc start` fails: `signature` error | Driver not signed | Enable test signing or disable enforcement |
| `CreateFile` returns INVALID_HANDLE | Driver not loaded or wrong name | Check `sc query KMem` status |
| BSOD on driver load | IRP struct layout mismatch | Verify struct offsets match your Windows version |

---

## Security Warnings

⚠️ **This driver provides unrestricted memory read access.**

- Any program can read any process's memory through `\\.\KMem`
- This includes passwords, encryption keys, and sensitive data
- **NEVER** load this driver on a machine with sensitive data
- **NEVER** distribute this driver without understanding the risks
- This is for **educational and research purposes only**

---

## File Structure

```
driver/
├── Cargo.toml          ← Crate config (panic=abort, no_std)
├── build.rs            ← Locates WDK, sets linker flags
├── BUILD_GUIDE.md      ← This file
└── src/
    └── lib.rs          ← Driver entry, IOCTL handler, memory copy
```
