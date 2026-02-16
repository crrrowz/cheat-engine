# Cheat Engine Driver Swap & Stealth Rebranding — Complete Implementation Plan

> **Goal**: Make Cheat Engine use our custom `ArcKernel` driver (`kmem_driver.sys`) instead of `dbk64.sys` for kernel memory operations, while keeping ALL of Cheat Engine's existing functionality (process selection, memory scanning, address list, etc.) intact. Additionally, rebrand every detectable string, registry key, window title, service name, and file reference to evade anti-cheat signature detection.

> **Important Constraint**: The custom driver is designed for a specific game (Arc Raiders) and provides only **Read Memory**, **Get Module Base**, and **Auth** IOCTLs. Cheat Engine's full feature set (debugging, ultimap, DBVM, etc.) will NOT be available through this driver. The strategy is to make the **memory read/write path** use our driver, while gracefully disabling or stubbing kernel-only features that require the original `dbk64.sys`.

---

## Table of Contents

0. [Phase 0 — ce_deploy.py Build Script](#0-phase-0--ce_deploypy-build-script)
1. [Architecture Overview](#1-architecture-overview)
2. [Source File Map — What To Analyze](#2-source-file-map--what-to-analyze)
3. [Phase 1 — Create ArcDriver.pas](#3-phase-1--create-arcdriverpas)
4. [Phase 2 — Patch DBK32functions.pas](#4-phase-2--patch-dbk32functionspas)
5. [Phase 3 — Patch NewKernelHandler.pas](#5-phase-3--patch-newkernelhandlerpas)
6. [Phase 4 — Stealth Rebranding](#6-phase-4--stealth-rebranding)
7. [Phase 5 — Registry Key Migration](#7-phase-5--registry-key-migration)
8. [Phase 6 — Binary & Resource Cleanup](#8-phase-6--binary--resource-cleanup)
9. [Phase 7 — Compile & Test](#9-phase-7--compile--test)
10. [Detailed AI Task Prompt](#10-detailed-ai-task-prompt)

---

## 1. Architecture Overview

### Current Cheat Engine Driver Flow

```
User clicks "Select Process" → ProcessHandlerUnit.pas (gets PID)
                                      ↓
User performs memory scan → NewKernelHandler.ReadProcessMemory (function pointer)
                                      ↓
                           Two possible paths:
                           ├─ Windows API: kernel32.ReadProcessMemory
                           └─ Kernel Driver: DBK32functions.RPM → DeviceIoControl → dbk64.sys
```

### Target Flow (After Modification)

```
User clicks "Select Process" → ProcessHandlerUnit.pas (gets PID) [NO CHANGE]
                                      ↓
User performs memory scan → NewKernelHandler.ReadProcessMemory (function pointer)
                                      ↓
                           Two possible paths:
                           ├─ Windows API: kernel32.ReadProcessMemory [NO CHANGE]
                           └─ ArcDriver: ArcDriver_ReadMemory → DeviceIoControl → ArcKernel (\\.\Nul)
                                      ↑
                              XOR encryption with 0xDEADBEEFCAFEBABE
                              Auth handshake on first connection
```

### Key Principle
- **Process selection, memory scanning, address list, pointer scanning** — ALL stay the same.
- **Only the kernel driver communication layer changes** (from `dbk64.sys` IOCTLs to `ArcKernel` IOCTLs).
- Features that require `dbk64.sys`-specific IOCTLs (Ultimap, DBVM, kernel debugging, physical memory) will be **gracefully disabled** (return failure, hide UI buttons).

### ⚠️ Critical: Dynamic Driver Name Rotation — Compile-Time Baking

The driver name is **randomly generated on every build** by `cli.py`:
```
cli.py build → suffix = "A1B2" → driver creates \Device\MyRoot_A1B2
```

**Solution: Python Orchestrator (`ce_deploy.py`) — The Build Pipeline**

> ⚠️ **Critical Design Rule:** CE (Pascal/Lazarus) does NOT build the driver. The driver (Rust/Cargo) does NOT know about CE.
> A Python script is the **conductor** that builds both projects **separately**, then assembles the output.

**Why NOT merge the source code into one project:**
- **Compilation Hell** — Lazarus builds Pascal, Cargo builds Rust. Mixing them requires both toolchains in one environment.
- **Signing & Loading** — The `.sys` driver is unsigned. Only KDU/kdmapper (called from Python) can load it. CE cannot load unsigned drivers by itself.
- **Chicken-and-egg** — CE needs a driver to work, but it can't load its own driver without a separate loader.

**The Pipeline:**
```
python ce_deploy.py deploy
  │
  │  ┌─────────────────────────────────┐
  ├─ │ 1. Generate random suffix       │  (Python)
  │  └─────────────────────────────────┘
  │  ┌─────────────────────────────────┐
  ├─ │ 2. Patch driver/src/lib.rs      │  (Python patches Rust source in ArcRaiders project)
  ├─ │ 3. Build driver (cargo build)   │  (Rust toolchain — produces .sys)
  │  └─────────────────────────────────┘
  │  ┌─────────────────────────────────┐
  ├─ │ 4. Patch ArcDriver.pas          │  (Python patches Pascal source in CE project)
  ├─ │ 5. Build CE (lazbuild)          │  (Lazarus toolchain — produces .exe)
  │  └─────────────────────────────────┘
  │  ┌─────────────────────────────────┐
  ├─ │ 6. Assemble output/             │  (Python copies .sys + .exe + kdu into one folder)
  ├─ │ 7. Load driver via KDU          │  (Python runs kdu.exe as admin)
  └─ │ 8. Launch modified CE           │  (Python launches arcdebugger.exe)
     └─────────────────────────────────┘
```

**Key principles:**
- **Projects stay separate** — driver source stays in `ArcRaiders-External-Cheat/driver/`, CE source stays in `cheat-engine/Cheat Engine/`
- **No source duplication** — Python patches files **in-place** in their original projects
- **One command** — `python ce_deploy.py deploy` does everything
- **One output** — `output/` folder contains everything needed: `arcdebugger.exe` + `kmem_driver.sys` + `kdu.exe`
- **Always in sync** — driver and CE are built with the same suffix in one pipeline run

See [Phase 0](#0-phase-0--ce_deploypy-build-script) for the full script specification.

---

## 0. Phase 0 — ce_deploy.py Build Pipeline (The Orchestrator)

Create `ArcRaiders-External-Cheat/ce_deploy.py` — a unified build pipeline that orchestrates **two separate projects**.

> ⚠️ **This script lives in the ArcRaiders project, NOT in the CE project.** It reaches into both projects to patch, build, and assemble.

### Architecture: Two Projects, One Pipeline

```
┌────────────────────────────────────────────────────────────────────────┐
│                        ce_deploy.py (Python)                          │
│                     The Orchestrator / Conductor                      │
└────────┬─────────────────────────────────┬────────────────────────────┘
         │                                 │
         ▼                                 ▼
┌─────────────────────────┐    ┌─────────────────────────────┐
│  ArcRaiders-External-   │    │      cheat-engine/          │
│  Cheat/                 │    │                             │
│                         │    │  Cheat Engine/              │
│  driver/                │    │  ├── ArcDriver.pas ← PATCH │
│  ├── src/lib.rs ← PATCH │    │  ├── DBK32functions.pas    │
│  ├── Cargo.toml         │    │  ├── NewKernelHandler.pas  │
│  └── target/            │    │  ├── MainUnit2.pas         │
│     └── release/        │    │  └── cheatengine.lpi       │
│        └── driver.sys   │    │                             │
│                         │    │  (Built by Lazarus)         │
│  tools/kdu.exe          │    └─────────────────────────────┘
│                         │
│  (Built by Cargo)       │
└─────────────────────────┘
                    │
                    ▼
         ┌──────────────────────┐
         │     output/          │  ← ASSEMBLED BY ce_deploy.py
         │  ├── arcdebugger.exe │  (from CE project)
         │  ├── kmem_driver.sys │  (from ArcRaiders project)
         │  └── kdu.exe         │  (from ArcRaiders tools/)
         └──────────────────────┘
```

**Key Rules:**
1. **No source code is copied** between projects — Python patches files **in-place**
2. **Each project uses its own toolchain** — Cargo for Rust, Lazarus for Pascal
3. **CE never touches Rust** — CE doesn't know the driver exists as source code
4. **The driver doesn't know about CE** — it's a standalone kernel module
5. **Only `ce_deploy.py` knows about both** — it's the single point of integration

### ce_deploy.py Implementation

```python
# ce_deploy.py — The Build Pipeline

ARC_PROJECT  = Path(r"D:\files\Contracted projects\IdeaProjects\ArcRaiders-External-Cheat")
CE_PROJECT   = Path(r"D:\files\Contracted projects\IdeaProjects\cheat-engine")

# Source locations (NEVER copied — patched in-place)
DRIVER_SRC   = ARC_PROJECT / "driver" / "src" / "lib.rs"
PASCAL_SRC   = CE_PROJECT / "Cheat Engine" / "ArcDriver.pas"
CE_LPI       = CE_PROJECT / "Cheat Engine" / "cheatengine.lpi"

# Build artifacts
DRIVER_SYS   = ARC_PROJECT / "driver" / "target" / "x86_64-pc-windows-msvc" / "release" / "kmem_driver.sys"
CE_EXE       = CE_PROJECT / "Cheat Engine" / "output" / "arcdebugger.exe"  # Lazarus output
KDU_EXE      = ARC_PROJECT / "tools" / "kdu.exe"

# Final assembled output
OUTPUT_DIR   = ARC_PROJECT / "output"

def cmd_build():
    """Full pipeline: Generate suffix → Build driver → Patch CE → Build CE → Assemble."""
    suffix = f"{int(time.time()) & 0xFFFF:04X}"
    print(f"[1/5] Suffix: MyRoot_{suffix}")

    # ── Step 1: Patch Rust driver source (in ArcRaiders project) ──
    print("[2/5] Patching driver source...")
    patch_driver_source(DRIVER_SRC, suffix)

    # ── Step 2: Build driver (Cargo — Rust toolchain) ──
    print("[3/5] Building driver (cargo)...")
    subprocess.run(
        "cargo build --release --target x86_64-pc-windows-msvc",
        shell=True, cwd=str(ARC_PROJECT / "driver"), check=True
    )

    # ── Step 3: Patch Pascal source (in CE project) ──
    print("[4/5] Patching ArcDriver.pas...")
    patch_pascal_source(PASCAL_SRC, suffix)

    # ── Step 4: Build CE (Lazarus — Pascal toolchain) ──
    print("[5/5] Building Cheat Engine (lazbuild)...")
    subprocess.run(f"lazbuild {CE_LPI}", shell=True, check=True)

    # ── Step 5: Assemble output folder ──
    assemble_output(suffix)
    print(f"[OK] Pipeline complete: output/ ready")

def assemble_output(suffix):
    """Copy build artifacts into a single deployment folder."""
    OUTPUT_DIR.mkdir(exist_ok=True)
    shutil.copy2(DRIVER_SYS, OUTPUT_DIR / f"MyRoot_{suffix}.sys")
    shutil.copy2(CE_EXE, OUTPUT_DIR / "arcdebugger.exe")
    shutil.copy2(KDU_EXE, OUTPUT_DIR / "kdu.exe")
    # Write a loader script
    (OUTPUT_DIR / "run.bat").write_text(
        f'@echo off\nkdu.exe -map MyRoot_{suffix}.sys\ntimeout /t 2\narcdebugger.exe\n',
        encoding='utf-8'
    )

def cmd_deploy():
    """Build + Load driver + Launch CE."""
    cmd_build()
    # Load driver via KDU (requires admin)
    subprocess.run(f"{KDU_EXE} -map {DRIVER_SYS}", shell=True, check=True)
    time.sleep(2)
    # Launch CE
    subprocess.Popen([str(CE_EXE)])

def patch_driver_source(lib_rs, suffix):
    """Patch Rust driver with new device names (in ArcRaiders project)."""
    content = lib_rs.read_text(encoding="utf-8")
    new_code = generate_driver_name_code(suffix)  # reused from cli.py
    content = re.sub(
        r"static DEV_NAME_U16.*?static SYM_NAME_U16.*?;",
        new_code, content, flags=re.DOTALL
    )
    lib_rs.write_text(content, encoding="utf-8")

def patch_pascal_source(pas_file, suffix):
    """Bake device path into ArcDriver.pas (in CE project)."""
    content = pas_file.read_text(encoding="utf-8")
    content = re.sub(
        r"ARC_SECONDARY_DEVICE\s*=\s*'[^']*'",
        f"ARC_SECONDARY_DEVICE = '\\\\.\\MyRoot_{suffix}'",
        content
    )
    pas_file.write_text(content, encoding="utf-8")
```

### Commands

| Command | What it does |
|---------|------|
| `python ce_deploy.py build` | Patch driver → cargo build → Patch CE → lazbuild → Assemble output/ |
| `python ce_deploy.py deploy` | build + load driver via KDU + launch CE |
| `python ce_deploy.py load` | Load already-built .sys via KDU only |
| `python ce_deploy.py status` | Check if driver is loaded |

### Why No `init` Command?

The previous plan had `cmd_init()` that copies source code. **This is eliminated.** Each project manages its own source. `ce_deploy.py` patches files in-place using regex — no copying, no duplication.

---

## 2. Source File Map — What To Analyze

The AI must read and understand ALL of the following files before making any changes:

### Critical Files (Must Modify)

| File | Path | Role | What Changes |
|------|------|------|--------------|
| `DBK32functions.pas` | `Cheat Engine/dbk32/` | Driver loading, all IOCTL wrappers, `hdevice` handle | Replace `DBK32Initialize` to open `\\.\Nul` instead of loading `dbk64.sys` service. Redirect `RPM`/`WPM` to use ArcDriver XOR+IOCTL. Stub unimplemented IOCTLs. |
| `NewKernelHandler.pas` | `Cheat Engine/` | Abstraction layer, function pointers for RPM/WPM/OpenProcess | Modify `LoadDBK32` to set up ArcDriver function pointers. Disable kernel-only features. |
| `MainUnit2.pas` | `Cheat Engine/` | Defines `strCheatEngine`, `strCheatTable`, version strings | Change ALL string constants to new brand name. |
| `first.pas` | `Cheat Engine/` | Registry path for DPI settings, uses hardcoded `'Cheat Engine'` | Replace hardcoded registry path strings. |
| `cheatengine.lpr` | `Cheat Engine/` | Main program file. `Application.Title`, resource references, `OutputDebugString('Starting CE')` | Change application title, debug strings. |

### Files To Analyze (May Need Modification)

| File | Path | Role | Potential Changes |
|------|------|------|-------------------|
| `ProcessHandlerUnit.pas` | `Cheat Engine/` | Process open/close, PID management | Should work unchanged — uses `OpenProcess` from `NewKernelHandler` which already uses Windows API. Verify no CE-specific strings. |
| `MainUnit.pas` | `Cheat Engine/` | Main form, UI labels, captions, About box references | Search for ALL `strCheatEngine` references, `Cheat Engine` literals, URLs to `cheatengine.org`. |
| `aboutunit.pas` | `Cheat Engine/` | About dialog | Remove/replace all CE branding. |
| `formsettingsunit.pas` | `Cheat Engine/` | Settings dialog, calls `LoadDBK32` | Verify driver loading path works. |
| `LuaHandler.pas` | `Cheat Engine/` | Lua scripting, calls `LoadDBK32`, `DBK32Initialize` | Verify Lua `getCheatEngineDir()` and similar functions work. |
| `MemoryBrowserFormUnit.pas` | `Cheat Engine/` | Memory browser, kernel tools menu | Disable kernel-only menu items gracefully. |
| `plugin.pas` / `cepluginsdk.pas` | `Cheat Engine/plugin/` | Plugin system, exports `LoadDBK32` pointer | Ensure plugin API still works. |
| `KernelDebuggerInterface.pas` | `Cheat Engine/` | Kernel debugger | Will not work — must be disabled gracefully. |
| `frmDriverLoadedUnit.pas` | `Cheat Engine/` | "Driver loaded" notification | Remove or rebrand. |
| `globals.pas` | `Cheat Engine/` | Global variables and constants | Check for CE-specific constants. |
| `Filehandler.pas` | `Cheat Engine/` | File format handling (.CT, .CETRAINER) | Check file extension references. |
| `DBK64SecondaryLoader.pas` | `Cheat Engine/` | Secondary driver loading via DBVM | Disable — not needed. |

### Files To Create

| File | Role |
|------|------|
| `ArcDriver.pas` | New unit: driver handle management, XOR encryption, IOCTL wrappers for ArcKernel. Device paths are **patched by `ce_deploy.py` at build time** — the secondary device path is baked into the compiled binary. |
| `ce_deploy.py` | Python build script: copies driver, patches sources, builds everything, loads driver, launches CE. See [Phase 0](#0-phase-0--ce_deploypy-build-script). |

### Reference Files (Read-Only, For Understanding the Driver Protocol)

| File | Path | Role |
|------|------|------|
| `kernel.py` | `ArcRaiders-External-Cheat/core/` | Python interface showing driver protocol, IOCTLs, encryption |
| `lib.rs` | `ArcRaiders-External-Cheat/rust_core/src/` | Rust driver implementation, IOCTL handlers, XOR key |
| `cli.py` | `ArcRaiders-External-Cheat/` | Existing build script — `ce_deploy.py` reuses its `generate_driver_name_code()` and `update_source_names()` functions |

---

## 3. Phase 1 — Create ArcDriver.pas

Create a new Pascal unit at `Cheat Engine/ArcDriver.pas` that encapsulates ALL communication with the custom driver.

### Requirements

1. **Constants** (the secondary device is patched by `ce_deploy.py` before each build):
   - `ARC_IOCTL_READ = $222000`
   - `ARC_IOCTL_GET_BASE = $222008`
   - `ARC_IOCTL_AUTH = $222010`
   - `ARC_XOR_KEY = $DEADBEEFCAFEBABE` (UInt64)
   - `ARC_PRIMARY_DEVICE = '\\.\Nul'` (constant, always works — NUL device hook)
   - `ARC_SECONDARY_DEVICE = '\\.\MyRoot_XXXX'` ← **patched at compile time by ce_deploy.py**

2. **Record Type**:
   ```pascal
   type
     TArcRequest = packed record
       process_id: UInt64;
       address: UInt64;
       size: UInt64;
     end;
   ```

3. **Functions to implement**:
   - `ArcDriver_Connect: Boolean` — Tries `ARC_PRIMARY_DEVICE` first, then `ARC_SECONDARY_DEVICE`. Both paths are **compiled in** (no runtime file reading).
   - `ArcDriver_Authorize: Boolean` — Sends `ARC_IOCTL_AUTH` with the current process PID (XOR-encrypted) to authorize this CE instance.
   - `ArcDriver_ReadMemory(ProcessID: DWORD; Address: UInt64; Buffer: Pointer; Size: DWORD; var BytesRead: PtrUInt): Boolean` — Builds `TArcRequest`, XOR-encrypts all three fields, calls `DeviceIoControl` with `ARC_IOCTL_READ`, receives data into `Buffer`.
   - `ArcDriver_GetModuleBase(ProcessID: DWORD; var BaseAddress: UInt64): Boolean` — Sends `ARC_IOCTL_GET_BASE` to get the main module base address.
   - `ArcDriver_Disconnect` — Closes the handle.
   - `ArcDriver_IsConnected: Boolean` — Returns whether the handle is valid.

4. **XOR Encryption Helper**:
   ```pascal
   procedure XorEncryptRequest(var Req: TArcRequest);
   begin
     Req.process_id := Req.process_id xor ARC_XOR_KEY;
     Req.address := Req.address xor ARC_XOR_KEY;
     Req.size := Req.size xor ARC_XOR_KEY;
   end;
   ```

5. **Connect Logic (Pseudocode)**:
   ```pascal
   function ArcDriver_Connect: Boolean;
   begin
     // Step 1: Try primary (hooked NUL device, always constant)
     hArcDevice := CreateFileW(ARC_PRIMARY_DEVICE, GENERIC_READ or GENERIC_WRITE, ...);
     if hArcDevice <> INVALID_HANDLE_VALUE then
     begin
       Result := ArcDriver_Authorize;
       Exit;
     end;

     // Step 2: Try secondary (baked in by ce_deploy.py at compile time)
     hArcDevice := CreateFileW(ARC_SECONDARY_DEVICE, GENERIC_READ or GENERIC_WRITE, ...);
     if hArcDevice <> INVALID_HANDLE_VALUE then
     begin
       Result := ArcDriver_Authorize;
       Exit;
     end;

     Result := False;
   end;
   ```

6. **How ce_deploy.py patches this file**:
   ```python
   # ce_deploy.py patches ArcDriver.pas before each Lazarus build:
   content = re.sub(
       r"ARC_SECONDARY_DEVICE\s*=\s*'[^']*'",
       f"ARC_SECONDARY_DEVICE = '\\\\.\\MyRoot_{suffix}'",
       content
   )
   ```
   The constant `ARC_SECONDARY_DEVICE` becomes a **compile-time literal** — no config files, no runtime file I/O, no forensic artifacts.

7. **Important**: The `ArcDriver_ReadMemory` function must match the signature expected by Cheat Engine's `RPM` function pointer type so it can be used as a drop-in replacement.

---

## 4. Phase 2 — Patch DBK32functions.pas

### 4.1 — Replace DBK32Initialize

**Current behavior** (lines 3154–3523):
- Opens SCManager
- Creates/opens a Windows service for `CEDRIVER73` → `dbk64.sys`
- Writes registry keys A, B, C, D under `HKLM\SYSTEM\CurrentControlSet\Services\CEDRIVER73`
- Starts the service
- Opens `\\.\CEDRIVER73` via `CreateFileW`
- Stores handle in global `hdevice`
- Calls `InitializeDriver`, `GetDriverVersion`

**New behavior**:
- Skip ALL service creation/management code
- Call `ArcDriver_Connect()` to open handle to `\\.\Nul`
- Call `ArcDriver_Authorize()` to authenticate
- Set `hdevice` to the ArcDriver handle (for compatibility checks elsewhere that test `hdevice <> INVALID_HANDLE_VALUE`)
- Skip `InitializeDriver`, `GetDriverVersion` (not applicable)
- Set `hUltimapDevice := INVALID_HANDLE_VALUE` (ultimap not supported)

### 4.2 — Replace RPM Function

The `RPM` function (declared at line 281) is used when kernel-mode memory reading is enabled. The AI must:
1. Find the implementation of `RPM` in `DBK32functions.pas`
2. Replace its body to call `ArcDriver_ReadMemory` instead of `DeviceIoControl(hdevice, IOCTL_CE_READMEMORY, ...)`
3. The function must extract the PID from the handle map (or directly use the PID stored in ProcessHandler)

### 4.3 — Replace WPM Function

The `WPM` (WriteProcessMemory) function — the ArcKernel driver does NOT support writes. Options:
- **Option A**: Fall back to Windows API `WriteProcessMemory` (will work for non-protected processes)
- **Option B**: Return failure (may break some CE features)
- **Recommended**: Option A — use Windows API fallback

### 4.4 — Replace ReadProcessMemory64 / ReadProcessMemory64_Internal

These 64-bit variants must also be redirected to `ArcDriver_ReadMemory`.

### 4.5 — Stub Unsupported Functions

The following functions use `hdevice` with CE-specific IOCTLs. They must be stubbed to return failure gracefully (no crash, no error dialog):

- `GetPEProcess` → return 0
- `GetPEThread` → return 0  
- `GetCR3` → return FALSE
- `GetCR4` → return 0
- `GetCR0` → return 0
- `GetSDT` → return 0
- `InitializeDriver` → return TRUE (pretend success)
- `GetDriverVersion` → return `currentversion` (pretend match)
- `StartProcessWatch` → return FALSE
- `ReadPhysicalMemory` → return FALSE
- `WritePhysicalMemory` → return FALSE
- `KernelAlloc` / `KernelAlloc64` → return nil/0
- `MapMemory` / `UnmapMemory` → no-op
- `ExecuteKernelCode` → return FALSE
- `ultimap*` functions → no-op / return FALSE
- `LaunchDBVM` → no-op
- `DBKSuspendThread` / `DBKResumeThread` → use Windows API fallback (`SuspendThread`/`ResumeThread`)
- `DBKSuspendProcess` / `DBKResumeProcess` → use `NtSuspendProcess`/`NtResumeProcess` fallback
- All debug-related IOCTLs → return FALSE

### 4.6 — Remove Error Dialogs

Replace or remove ALL `MessageBox` calls in `DBK32Initialize` that reference:
- `rsYouAreMissingTheDriver`
- `rsTheServiceCouldntGetOpened`
- `rsTheDriverCouldntBeOpened`
- `rsTheDriverThatIsCurrentlyLoaded`
- `rsPleaseRebootAndPressF8DuringBoot`
- `rsDBKBlockedDueToVulnerableDriverBlocklist`

These would reveal CE identity and confuse users.

### 4.7 — Remove `cheatengine.org` URL

Line 3417 contains: `shellexecute(0, 'open', 'https://cheatengine.org/dbkerror.php', ...)`
This MUST be removed.

---

## 5. Phase 3 — Patch NewKernelHandler.pas

### 5.1 — Modify LoadDBK32 (lines 1867–1970)

**Current behavior**:
- Calls `DBK32Initialize`
- Sets `DBKLoaded` based on `hdevice`
- Assigns function pointers for kernel operations (GetCR4, GetCR3, StartProcessWatch, etc.)

**New behavior**:
- Call `ArcDriver_Connect()` and `ArcDriver_Authorize()` (or call the modified `DBK32Initialize` which does this)
- Set `DBKLoaded := ArcDriver_IsConnected`
- Only assign the function pointers that actually work (stubbed versions)
- **Do NOT change**: `ReadProcessMemoryActual`, `WriteProcessMemoryActual`, `OpenProcess` — these are Windows API function pointers and must remain unchanged for normal CE operation

### 5.2 — UseDBKReadWriteMemory / DONTUseDBKReadWriteMemory

These procedures swap the `ReadProcessMemoryActual` function pointer between Windows API and kernel driver versions. The AI must:
1. Find where `UseDBKReadWriteMemory` assigns the kernel RPM function
2. Make it assign `ArcDriver_ReadMemory` instead
3. For writes, keep using Windows API `WriteProcessMemory`

### 5.3 — Disable Kernel-Only UI

In `LoadDBK32`, around line 1967:
```pascal
MemoryBrowser.Kerneltools1.Enabled := DBKLoaded or isRunningDBVM;
```
Change to:
```pascal
MemoryBrowser.Kerneltools1.Enabled := False; // Kernel tools not available with ArcDriver
```

---

## 6. Phase 4 — Stealth Rebranding

### 6.1 — String Constants in MainUnit2.pas

**File**: `Cheat Engine/MainUnit2.pas` (lines 22–48)

Replace ALL of these:

| Current | Replace With |
|---------|-------------|
| `strCheatEngine = 'Cheat Engine'` | `strCheatEngine = 'Arc Debugger'` |
| `strCheatTable = 'Cheat Table'` | `strCheatTable = 'Debug Table'` |
| `strCheatTableLower = 'cheat table'` | `strCheatTableLower = 'debug table'` |
| `strCheat = 'Cheat'` | `strCheat = 'Mod'` |
| `strTrainer = 'Trainer'` | `strTrainer = 'Tool'` |
| `strTrainerLower = 'trainer'` | `strTrainerLower = 'tool'` |
| `strMyCheatTables = 'My Cheat Tables'` | `strMyCheatTables = 'My Debug Tables'` |
| `strSpeedHack = 'Speedhack'` | `strSpeedHack = 'SpeedMod'` |

Also change:
| Current | Replace With |
|---------|-------------|
| `ceversion = 7.51` | Keep or change to custom version |
| `strVersionPart = '7.5.1'` | `strVersionPart = '1.0.0'` |

The `{$ifdef altname}` block (lines 25–33) already has an alternate set of names for `'Runtime Modifier'`. We could either:
- **Option A**: Define `altname` compiler flag to use the built-in alternate names
- **Option B**: Change the `{$else}` block directly (more control over names)
- **Recommended**: Option B

### 6.2 — Application Title in cheatengine.lpr

**File**: `Cheat Engine/cheatengine.lpr` (line 293)

```pascal
// BEFORE:
Application.Title := 'Cheat Engine 7.5';
// AFTER:
Application.Title := 'Arc Debugger 1.0';
```

Also line 416:
```pascal
// BEFORE:
OutputDebugString('Starting CE');
// AFTER:
OutputDebugString('Starting AD');  // Or remove entirely
```

### 6.3 — Registry Path in first.pas

**File**: `Cheat Engine/first.pas` (lines 84, 92)

```pascal
// BEFORE:
r.OpenKey('\Software\' + 'Cheat Engine', false)
r.OpenKey('\Software\' + 'Cheat Engine', true)
// AFTER:
r.OpenKey('\Software\' + 'Arc Debugger', false)
r.OpenKey('\Software\' + 'Arc Debugger', true)
```

Note: These use hardcoded strings, NOT `strCheatEngine`. Both instances must be changed.

### 6.4 — Resource Strings in DBK32functions.pas

**File**: `Cheat Engine/dbk32/DBK32functions.pas` (lines 438–460)

ALL resource strings that mention "Cheat Engine", "DBK32", "dbk", "CE" must be reworded:
- `rsYouAreMissingTheDriver` — references `strCheatEngine`
- `rsDriverError` — "Driver error" (generic, OK to keep)
- `rsDbk32Error` — "DBK32 error" → change to "Driver error"
- `rsTheDriverThatIsCurrentlyLoaded` — references `strCheatEngine`
- `rsPleaseRunThe64BitVersionOfCE` — references `strCheatEngine`
- `rsDBKBlockedDueToVulnerableDriverBlocklist` — remove or reword

### 6.5 — Window Class Names and Form Names

Anti-cheats scan for window class names. The AI must search for:
- `TMainForm` class name (may be detectable)
- Window captions set in `.lfm` form files
- Any `Caption := '...'` that contains "Cheat Engine"

Search locations:
- `MainUnit.pas` + `MainUnit.lfm`
- `MemoryBrowserFormUnit.pas` + `MemoryBrowserFormUnit.lfm`
- `aboutunit.pas` + `aboutunit.lfm`

### 6.6 — Service Name Constants

In `DBK32functions.pas` (line 3194):
```pascal
servicename := 'CEDRIVER73';
```
This is the Windows service name for the driver. Since we are NOT loading a service anymore (Phase 2), this code path should be dead. But verify it's never referenced elsewhere.

### 6.7 — Debug Output Strings

Search the ENTIRE `Cheat Engine/` directory for `OutputDebugString` calls that contain identifiable text:
- `'Starting CE'`
- `'LoadDBK32'`
- `'DBK32Initialize'`
- Any string with `'CE'`, `'Cheat'`, `'DBK'`

Either remove these or replace with generic strings.

### 6.8 — Compiled Resource File

**File**: `cheatengine.res` (line 128 of `.lpr`)

This resource file likely contains:
- Application icon
- Version info (CompanyName, ProductName, FileDescription)
- Manifest

The AI should note that this binary resource MUST be rebuilt with:
- No "Cheat Engine" in `FileDescription` or `ProductName`
- Custom icon
- Updated version numbers

### 6.9 — Program Name

The output binary is `cheatengine.exe`. The Lazarus project file (`.lpi` or `.lpr`) controls this. Change:
- Project output filename to `arcdebugger.exe` (or similar)
- Search for `.lpi` file and modify the `<Target><Filename>` element

---

## 7. Phase 5 — Registry Key Migration

Cheat Engine stores ALL its settings under:
```
HKEY_CURRENT_USER\Software\Cheat Engine\
```

This is defined by `strCheatEngine` in `MainUnit2.pas`. After changing `strCheatEngine` to `'Arc Debugger'`, the new path becomes:
```
HKEY_CURRENT_USER\Software\Arc Debugger\
```

**Action items**:
1. Changing `strCheatEngine` automatically changes ALL registry paths (since they use `'\Software\' + strCheatEngine`).
2. Verify `first.pas` hardcoded paths are also updated (Phase 4.3).
3. Old settings under `\Software\Cheat Engine\` will be ignored (clean start). This is acceptable.

---

## 8. Phase 6 — Binary & Resource Cleanup

### 8.1 — Driver Files

The following CE driver files should NOT be included in the distribution:
- `dbk32.sys`
- `dbk64.sys`
- `ultimap2-64.sys`
- `driver.dat` / `driver64.dat`

### 8.2 — DLL Files

Check if CE bundles `dbk32.dll` or `dbk64.dll` — these should not be present.

### 8.3 — File Extension Associations

CE registers `.CT` and `.CETRAINER` file extensions. The AI should check if these are registered in code and either:
- Keep them (low detection risk)
- Change to custom extensions

---

## 9. Phase 7 — Compile & Test

### Compilation Prerequisites
- Lazarus IDE (Free Pascal)
- The Cheat Engine `.lpi` project file
- Custom `ArcDriver.pas` added to the project

### Test Checklist

| Test | Expected Result |
|------|----------------|
| Launch application | Window title shows "Arc Debugger 1.0" |
| Click "Select Process" | Process list appears, can select any process |
| Select a running process | PID is captured, process name shown |
| Memory scan (exact value) | Scan completes, results shown |
| Change a value in address list | Value changes in target process |
| Check Windows registry | Settings stored under `\Software\Arc Debugger\` |
| Check with Process Explorer | No window title, class name, or string contains "Cheat Engine" |
| Run `strings.exe` on the binary | No occurrence of "Cheat Engine", "dbk32", "dbk64", "CEDRIVER" |
| Run alongside anti-cheat | Application is not flagged |

---

## 10. Detailed AI Task Prompt

> **Instructions for the AI performing the implementation:**
>
> You have access to the Cheat Engine source code at `D:\files\Contracted projects\IdeaProjects\cheat-engine\Cheat Engine\` and the custom driver reference at `D:\files\Contracted projects\IdeaProjects\ArcRaiders-External-Cheat\`.
>
> **Read and analyze these files FIRST before writing any code:**
>
> 1. `core/kernel.py` — Understand the driver protocol (device paths, IOCTLs, XOR encryption, auth flow)
> 2. `rust_core/src/lib.rs` — Understand the driver-side IOCTL handler
> 3. `Cheat Engine/dbk32/DBK32functions.pas` — Full file, especially:
>    - `DBK32Initialize` procedure (line ~3154)
>    - `RPM` function (search for `function.*RPM` or `ReadProcessMemory` wrapper)
>    - `ReadProcessMemory64_Internal` function
>    - `DeviceIoControl` wrapper (line ~469)
>    - All resource strings (lines 438–460)
> 4. `Cheat Engine/NewKernelHandler.pas` — Full file, especially:
>    - `LoadDBK32` procedure (line ~1867)
>    - `UseDBKReadWriteMemory` / `DONTUseDBKReadWriteMemory` procedures
>    - `ReadProcessMemoryActual` / `WriteProcessMemoryActual` variables
>    - `OpenProcess` function pointer
> 5. `Cheat Engine/MainUnit2.pas` — String constants (lines 22–48)
> 6. `Cheat Engine/first.pas` — Hardcoded registry paths
> 7. `Cheat Engine/cheatengine.lpr` — Application title, program entry
> 8. `Cheat Engine/ProcessHandlerUnit.pas` — Process handle management
> 9. `Cheat Engine/MainUnit.pas` — UI references to strCheatEngine
> 10. `Cheat Engine/aboutunit.pas` — About dialog branding
>
> **Then execute changes in this EXACT order:**
>
> ### Step 1: Create `ArcDriver.pas`
> - Implement all functions described in Phase 1
> - Unit must compile standalone with `{$MODE Delphi}` and use only `windows`, `sysutils`
> - Include error handling (try/except around CreateFileW, DeviceIoControl)
>
> ### Step 2: Modify `DBK32functions.pas`
> - Add `ArcDriver` to the `uses` clause
> - Replace `DBK32Initialize` body per Phase 2.1
> - Redirect RPM per Phase 2.2
> - Fallback WPM to Windows API per Phase 2.3
> - Stub all unsupported functions per Phase 2.5
> - Remove error dialogs per Phase 2.6
> - Remove cheatengine.org URL per Phase 2.7
>
> ### Step 3: Modify `NewKernelHandler.pas`
> - Modify `LoadDBK32` per Phase 3.1
> - Update `UseDBKReadWriteMemory` per Phase 3.2
> - Disable kernel tools UI per Phase 3.3
>
> ### Step 4: Rebrand strings
> - `MainUnit2.pas` per Phase 4.1
> - `cheatengine.lpr` per Phase 4.2
> - `first.pas` per Phase 4.3
> - `DBK32functions.pas` resource strings per Phase 4.4
> - Search and replace remaining "Cheat Engine" literals in all `.pas` files
>
> ### Step 5: Perform a global search
> Run a search across ALL `.pas`, `.lpr`, `.lfm`, `.lpi` files for:
> - `"Cheat Engine"` (case insensitive)
> - `"cheatengine"` (case insensitive)
> - `"dbk32"` / `"dbk64"` (case insensitive)
> - `"CEDRIVER"` (case insensitive)
> - `"cheatengine.org"` (case insensitive)
>
> Report any remaining occurrences that need attention.
>
> ### Step 6: List all files modified
> Provide a complete list of all modified files with a brief summary of changes.

---

## Appendix A — ArcKernel Driver Protocol Reference

### Device Paths
| Priority | Path | Notes |
|----------|------|-------|
| Primary | `\\.\Nul` | **Always constant.** Driver hooks the NUL device IRP. CE tries this first. |
| Secondary | `\\.\MyRoot_{suffix}` | **Changes every build.** Baked into `ArcDriver.pas` at compile time by `ce_deploy.py`. |

### Dynamic Name Rotation (ce_deploy.py)
```
ce_deploy.py build:
  1. Generate suffix = hex(time() & 0xFFFF)  →  e.g. "A1B2"
  2. Patch driver/src/lib.rs               →  \Device\MyRoot_A1B2
  3. Build driver (cargo build)
  4. Patch ArcDriver.pas (compile-time!)    →  ARC_SECONDARY_DEVICE = '\\.\MyRoot_A1B2'
  5. Build CE (lazbuild)                    →  suffix baked into binary
  6. Load driver via KDU
  7. Launch CE
```

**No runtime discovery needed. No config files. No forensic artifacts.**

### IOCTL Codes
| Code | Value | Description |
|------|-------|-------------|
| `IOCTL_READ` | `0x222000` | Read process memory |
| `IOCTL_GET_BASE` | `0x222008` | Get main module base address |
| `IOCTL_AUTH` | `0x222010` | Authorize calling process PID |

### Request Structure
```
Offset 0x00: process_id (u64) — XOR'd with key
Offset 0x08: address    (u64) — XOR'd with key
Offset 0x10: size       (u64) — XOR'd with key
Total: 24 bytes
```

### XOR Encryption Key
```
0xDEADBEEFCAFEBABE
```

### Auth Flow
1. Open device handle via `CreateFileW`
2. Build `TArcRequest` with `process_id = GetCurrentProcessId()`, `address = 0`, `size = 0`
3. XOR-encrypt all fields
4. Send via `DeviceIoControl(handle, IOCTL_AUTH, @request, 24, nil, 0, ...)`
5. If returns `TRUE`, driver is ready for read operations

### Read Flow
1. Build `TArcRequest` with target PID, address, size
2. XOR-encrypt all fields
3. Send via `DeviceIoControl(handle, IOCTL_READ, @request, 24, @buffer, size, ...)`
4. Buffer is filled with raw memory content (no encryption on output)

---

## Appendix C — ce_deploy.py Architecture

### Location
`D:\files\Contracted projects\IdeaProjects\ArcRaiders-External-Cheat\ce_deploy.py`

> **Lives in the ArcRaiders project** (the project that owns the driver). NOT in the CE project.

### Design Philosophy: The Orchestra

```
╔══════════════════════════════════════════════════════════════════╗
║                    ce_deploy.py (Conductor)                      ║
║                                                                  ║
║  "I build the instruments, tune them, and start the concert."   ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  Step 1: Generate suffix ──────────────────────── Python         ║
║  Step 2: Patch driver/src/lib.rs ──────────────── Python regex   ║
║  Step 3: Build driver ─────────────────────────── Cargo (Rust)   ║
║  Step 4: Patch ArcDriver.pas ──────────────────── Python regex   ║
║  Step 5: Build CE ─────────────────────────────── lazbuild       ║
║  Step 6: Assemble output/ ─────────────────────── Python shutil  ║
║  Step 7: Load driver ──────────────────────────── KDU (admin)    ║
║  Step 8: Launch CE ────────────────────────────── subprocess     ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  INPUT:  Two separate git repos                                  ║
║  OUTPUT: One folder with .exe + .sys + kdu + run.bat             ║
╚══════════════════════════════════════════════════════════════════╝
```

### What ce_deploy.py Does NOT Do

| ❌ Does NOT | Why |
|---|---|
| Copy Rust source into CE project | Causes duplication, version drift |
| Copy KDU into CE project | KDU stays in ArcRaiders/tools/ |
| Have an `init` command | No setup needed — patches in-place |
| Require CE to know about Rust | CE only sees ArcDriver.pas (pure Pascal) |
| Write config files | Suffix is baked at compile time |

### Final Deliverable: output/ Folder

```
output/
├── arcdebugger.exe      ← Modified CE (ARC_SECONDARY_DEVICE = '\\.\MyRoot_A1B2')
├── MyRoot_A1B2.sys      ← Kernel driver (creates \Device\MyRoot_A1B2)
├── kdu.exe              ← Driver loader (runs as admin)
└── run.bat              ← One-click: kdu -map → wait → launch CE
```

The user distributes this **single folder**. No Rust, no Pascal, no source code. Just run `run.bat` as admin.

---

## Appendix B — Anti-Cheat Detection Vectors

These are known detection methods that anti-cheats use to find Cheat Engine:

| Vector | Status After This Plan |
|--------|----------------------|
| Window title "Cheat Engine" | ✅ Eliminated |
| Window class `TMainForm` of CE | ⚠️ May need manual class name change in Lazarus project |
| Registry key `HKCU\Software\Cheat Engine` | ✅ Eliminated |
| Service name `CEDRIVER73` | ✅ Eliminated (no service created) |
| Driver file `dbk64.sys` | ✅ Eliminated (not loaded) |
| Process name `cheatengine.exe` | ✅ Eliminated (renamed output binary) |
| String signatures in binary | ✅ Eliminated (all strings replaced) |
| `OutputDebugString` with CE text | ✅ Eliminated (removed/replaced) |
| Known IOCTL codes for dbk64 | ✅ Eliminated (using custom IOCTLs) |
| Device name `\\.\CEDRIVER73` | ✅ Eliminated (using `\\.\Nul`) |
| File version info in PE header | ⚠️ Requires resource file rebuild |
| Icon signature | ⚠️ Requires custom icon |

---

*Generated: 2026-02-16 | ArcRaiders Driver Swap Plan v3.1 — Orchestrator Pipeline Architecture*
