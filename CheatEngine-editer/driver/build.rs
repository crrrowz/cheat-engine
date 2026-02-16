// ============================================================================
//  build.rs — Kernel Driver Build Script
// ============================================================================
//  This script runs BEFORE compilation and configures:
//    1. Linker settings for kernel mode (subsystem:native, entry point)
//    2. Links against ntoskrnl.lib (kernel API)
//    3. Renames output from .dll to .sys
//
//  Requirements:
//    - Windows Driver Kit (WDK) must be installed
//    - The WDK provides ntoskrnl.lib which exports kernel functions
// ============================================================================

fn main() {
    // ------------------------------------------------------------------
    //  Step 1: Locate the WDK installation path
    // ------------------------------------------------------------------
    //  WDK installs its libraries under:
    //    C:\Program Files (x86)\Windows Kits\10\Lib\<version>\km\x64\
    //
    //  We read the registry to find the exact path, falling back to
    //  a default location if the registry key isn't found.

    let wdk_lib_path = find_wdk_km_lib_path().unwrap_or_else(|| {
        // Fallback: common default WDK path
        let default = r"C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\km\x64";
        println!("cargo:warning=WDK registry key not found, using default: {}", default);
        default.to_string()
    });

    println!("cargo:warning=WDK lib path: {}", wdk_lib_path);

    // ------------------------------------------------------------------
    //  Step 2: Tell the linker where to find kernel libraries
    // ------------------------------------------------------------------
    println!("cargo:rustc-link-search=native={}", wdk_lib_path);

    // ------------------------------------------------------------------
    //  Step 3: Link against ntoskrnl.lib
    // ------------------------------------------------------------------
    //  ntoskrnl.lib provides all kernel API functions we use:
    //    - IoCreateDevice, IoCreateSymbolicLink, IoDeleteDevice, etc.
    //    - PsLookupProcessByProcessId, ObDereferenceObject
    //    - MmCopyVirtualMemory (undocumented but exported)
    //    - RtlInitUnicodeString, DbgPrint
    println!("cargo:rustc-link-lib=ntoskrnl");

    // ------------------------------------------------------------------
    //  Step 4: Kernel-specific linker flags
    // ------------------------------------------------------------------
    //  /SUBSYSTEM:NATIVE    — This is a kernel driver, not a user app
    //  /DRIVER              — Enable driver-specific linking
    //  /ENTRY:DriverEntry   — The kernel calls DriverEntry on load
    //  /NODEFAULTLIB        — Don't link user-mode C runtime
    println!("cargo:rustc-cdylib-link-arg=/SUBSYSTEM:NATIVE");
    println!("cargo:rustc-cdylib-link-arg=/DRIVER");
    println!("cargo:rustc-cdylib-link-arg=/ENTRY:DriverEntry");
    println!("cargo:rustc-cdylib-link-arg=/NODEFAULTLIB");

    // ------------------------------------------------------------------
    //  Step 5: Re-run if build.rs changes
    // ------------------------------------------------------------------
    println!("cargo:rerun-if-changed=build.rs");
}


/// Search the Windows Registry for the WDK 10 installation path
/// and return the km\x64 library directory.
fn find_wdk_km_lib_path() -> Option<String> {
    // Try to read from registry
    let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);

    let kits_root = hklm
        .open_subkey(r"SOFTWARE\Microsoft\Windows Kits\Installed Roots")
        .ok()?;

    let root_path: String = kits_root.get_value("KitsRoot10").ok()?;

    // Find the latest installed SDK version
    let lib_dir = std::path::Path::new(&root_path).join("Lib");

    if !lib_dir.exists() {
        return None;
    }

    // Get all version directories and pick the latest
    let mut versions: Vec<String> = std::fs::read_dir(&lib_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with("10."))
        .collect();

    versions.sort();

    let latest = versions.last()?;

    let km_path = lib_dir
        .join(latest)
        .join("km")
        .join("x64");

    if km_path.exists() {
        Some(km_path.to_string_lossy().to_string())
    } else {
        None
    }
}
