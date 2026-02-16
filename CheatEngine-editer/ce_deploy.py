import os
import sys
import time
import shutil
import subprocess
import re
from pathlib import Path

# Configuration
# Configuration
ARC_PROJECT = Path(__file__).parent.resolve()
LAZBUILD_PATH = Path(r"E:\lazarus\lazbuild.exe")
NEW_NAME = "Arc Debugger"

# Source files to patch (Relative to ARC_PROJECT)
DRIVER_SRC  = ARC_PROJECT / "rust_core" / "src" / "lib.rs"

# Build artifacts (Relative to ARC_PROJECT)
DRIVER_SYS  = ARC_PROJECT / "rust_core" / "target" / "x86_64-pc-windows-msvc" / "release" / "arc_core.dll"
# Lazarus output path depends on project settings

KDU_EXE     = ARC_PROJECT / "tools" / "kdu.exe" # Assuming kdu is here
OUTPUT_DIR  = ARC_PROJECT / "output"

def generate_driver_name_code(suffix):
    """Generates the Rust code for dynamic driver names."""
    return f"""
static DEV_NAME_U16: [u16; 24] = [
    92, 68, 101, 118, 105, 99, 101, 92,     // "\\Device\\"
    77, 121, 82, 111, 111, 116, 95,         // "MyRoot_"
    {ord(suffix[0])}, {ord(suffix[1])}, {ord(suffix[2])}, {ord(suffix[3])}, // "{suffix}"
    0, 0, 0, 0, 0
];

static SYM_NAME_U16: [u16; 26] = [
    92, 68, 111, 115, 68, 101, 118, 105, 99, 101, 92, // "\\DosDevices\\"
    77, 121, 82, 111, 111, 116, 95,                   // "MyRoot_"
    {ord(suffix[0])}, {ord(suffix[1])}, {ord(suffix[2])}, {ord(suffix[3])}, // "{suffix}"
    0, 0, 0, 0, 0
];
"""

def get_ce_project_path():
    """
    Returns the Cheat Engine project path.
    Prioritizes hardcoded known paths for automation, then falls back to user input.
    """
    # 1. Check known locations
    known_paths = [
        Path(r"d:\files\Contracted projects\IdeaProjects\cheat-engine"),
        Path(r"d:\files\Contracted projects\IdeaProjects\cheat-engine\Cheat Engine")
    ]
    
    for path in known_paths:
        if (path / "Cheat Engine" / "cheatengine.lpi").exists():
            print(f"  -> Found Cheat Engine at: {path}")
            return path
        if (path / "cheatengine.lpi").exists():
            print(f"  -> Found Cheat Engine at: {path.parent}")
            return path.parent

    # 2. Fallback to manual input
    print(f"Working Directory: {ARC_PROJECT}")
    while True:
        val = input("Enter path to Cheat Engine source (folder with 'cheatengine.lpi' in 'Cheat Engine' subdir): ").strip()
        if not val: return None
        val = val.replace('"', '').replace("'", "")
        path = Path(val)
        
        # Check standard structure
        if (path / "Cheat Engine" / "cheatengine.lpi").exists():
            return path
        # Check if user pointed inside the subdir
        if (path / "cheatengine.lpi").exists():
            return path.parent
            
        print(f"[!] Invalid path. Could not find 'Cheat Engine/cheatengine.lpi' in {path}")
        retry = input("Try again? (y/n): ").lower()
        if retry != 'y': return None

def rebrand_source(ce_path):
    """
    Walks through the CE source and replaces 'Cheat Engine' with 'Arc Debugger'.
    Targets .pas, .lpr, .lpi, .inc files.
    Performs replacements within single and double quotes to be safe.
    """
    print(f"[+] Starting Auto-Rebranding in {ce_path}...")
    encodings = ['utf-8', 'latin-1', 'cp1252']  # Pascal files often vary
    
    # Extensions to process
    exts = {'.pas', '.lpr', '.lpi', '.inc', '.xml'}
    
    count = 0
    for file_path in ce_path.rglob("*"):
        if file_path.suffix.lower() in exts and file_path.is_file():
            # Skip output/backup dirs if they exist inside
            if "backup" in file_path.parts or "lib" in file_path.parts:
                continue

            # Read content trying different encodings
            content = None
            used_enc = None
            for enc in encodings:
                try:
                    content = file_path.read_text(encoding=enc)
                    used_enc = enc
                    break
                except UnicodeDecodeError:
                    continue
            
            if content is None:
                print(f"  [!] Skipping {file_path.name} (encoding unknown)")
                continue

            original_content = content
            
            # Prepare variations
            new_name = NEW_NAME
            new_name_nospace = new_name.replace(" ", "")

            # Perform Replacements
            # 1. 'Cheat Engine' -> NEW_NAME (Strings)
            content = content.replace("'Cheat Engine'", f"'{new_name}'")
            content = content.replace('"Cheat Engine"', f'"{new_name}"')
            
            # 2. 'Cheat Engine ' -> NEW_NAME (Version strings)
            content = content.replace("'Cheat Engine ", f"'{new_name} ")
            content = content.replace('"Cheat Engine ', f'"{new_name} ')
            
            # 3. Specific casing (CheatEngine -> ArcDebugger)
            # safe_replace: "CheatEngine" -> "ArcDebugger"
            content = content.replace('"CheatEngine"', f'"{new_name_nospace}"')
            content = content.replace("'CheatEngine'", f"'{new_name_nospace}'")

            # Write back if changed
            if content != original_content:
                try:
                    file_path.write_text(content, encoding=used_enc)
                    # print(f"  -> Rebranded: {file_path.name}") # Too verbose?
                    count += 1
                except Exception as e:
                    print(f"  [!] Failed to write {file_path.name}: {e}")

    print(f"[+] Rebranding complete. Modified {count} files.")


def fix_source_issues(ce_path):
    """
    Scans source files to fix common compilation errors (e.g. laz_avl_Tree missing).
    """
    print(f"[+] Scanning source for compatibility issues in {ce_path}...")
    count = 0
    
    # Files often vary in encoding
    encodings = ['utf-8', 'latin-1', 'cp1252']

    for file_path in ce_path.rglob("*.pas"):
        # Skip backup/lib
        if "backup" in file_path.parts or "lib" in file_path.parts:
            continue
            
        try:
            content = None
            used_enc = 'utf-8'
            for enc in encodings:
                try:
                    content = file_path.read_text(encoding=enc)
                    used_enc = enc
                    break
                except: continue
                
            if content is None: continue
            
            original_content = content
            
            # Fix 1: Generic laz_avl_Tree -> AvgLvlTree
            if "laz_avl_Tree" in content:
                # Naive replace is usually safe for unit names
                content = content.replace("laz_avl_Tree", "AvgLvlTree")
                
            # Fix 2: Clean up duplicate AvgLvlTree usages (caused by previous replacement)
            # Pattern in memscan.pas: "AvgLvlTree, {$ifdef laztrunk}AVL_Tree{$else}AvgLvlTree{$endif}"
            # After generic replace:  "AvgLvlTree, {$ifdef laztrunk}AVL_Tree{$else}AvgLvlTree{$endif}"
            # We want: "AvgLvlTree, AVL_Tree" (or just AvgLvlTree if AVL_Tree not needed, but safe to include if available)
            
            if "AvgLvlTree, {$ifdef laztrunk}AVL_Tree{$else}AvgLvlTree{$endif}" in content:
                 content = content.replace("AvgLvlTree, {$ifdef laztrunk}AVL_Tree{$else}AvgLvlTree{$endif}", "AvgLvlTree, AVL_Tree")

            # General deduplication just in case
            if "AvgLvlTree, AvgLvlTree" in content:
                content = content.replace("AvgLvlTree, AvgLvlTree", "AvgLvlTree")

            # Fix 3: DissectCodeThread.pas specific (Duplicate identifier / Missing TAVLTreeNode)
            if file_path.name.lower() == "dissectcodethread.pas":
                # Robust Regex Injection
                # Find 'uses ... commonTypeDefs ... ;' in the interface section
                # We want to ensure AvgLvlTree is present AND AVL_Tree is REMOVED (to avoid legacy issues).
                
                # 1. Get Interface section
                impl_idx = content.lower().find("implementation")
                if impl_idx != -1:
                    interface_content = content[:impl_idx]
                    
                    # Ensure AvgLvlTree is in uses
                    if not re.search(r"\bAvgLvlTree\b", interface_content, re.IGNORECASE):
                         # Insert after commonTypeDefs
                         match = re.search(r"(\bcommonTypeDefs\b)", interface_content, re.IGNORECASE)
                         if match:
                             span = match.span()
                             interface_content = interface_content[:span[1]] + ", AvgLvlTree" + interface_content[span[1]:]

                    # Ensure AVL_Tree is NOT in uses (causes circular/legacy issues)
                    interface_content = re.sub(r",\s*AVL_Tree\b", "", interface_content, flags=re.IGNORECASE)
                    interface_content = re.sub(r"\bAVL_Tree\s*,", "", interface_content, flags=re.IGNORECASE)
                    
                    content = interface_content + content[impl_idx:]

                # Clean up implementation uses (remove conditional AVL_Tree/AvgLvlTree blocks)
                content = re.sub(r",\s*\{\$ifdef\s+laztrunk\}AVL_Tree\{\$else\}AvgLvlTree\{\$endif\}\s*;", ";", content, flags=re.IGNORECASE)
                content = re.sub(r",\s*\{\$ifdef\s+laztrunk\}AVL_Tree\{\$else\}AvgLvlTree\{\$endif\}", "", content, flags=re.IGNORECASE)
                
                # Clean simple duplicates in implementation
                if impl_idx != -1:
                    impl_content = content[impl_idx:]
                    interface_content = content[:impl_idx]
                    impl_content = re.sub(r",\s*AvgLvlTree\s*;", ";", impl_content, flags=re.IGNORECASE) 
                    impl_content = re.sub(r",\s*AvgLvlTree", "", impl_content, flags=re.IGNORECASE)
                    content = interface_content + impl_content

            # Fix 3.1: Rename TAVLTreeNode -> TAvgLvlTreeNode (Type compatibility) - Global & Case Insensitive
            if "TreeNode" in content: # Optimization check
                content = re.sub(r"\bTAVLTreeNode\b", "TAvgLvlTreeNode", content, flags=re.IGNORECASE)
                content = re.sub(r"\bTAVLTreeNodeEnumerator\b", "TAvgLvlTreeNodeEnumerator", content, flags=re.IGNORECASE)

            # Fix 3.2: Cleanup self-referential type aliases caused by Fix 3.1
            if "TAvgLvlTreeNode = TAvgLvlTreeNode;" in content:
                 content = content.replace("TAvgLvlTreeNode = TAvgLvlTreeNode;", "")
            if "TAvgLvlTreeNodeEnumerator = TAvgLvlTreeNodeEnumerator;" in content:
                 content = content.replace("TAvgLvlTreeNodeEnumerator = TAvgLvlTreeNodeEnumerator;", "")


            # Fix 4: ExtractFileNameWithoutExt -> ChangeFileExt(ExtractFileName(...), '')
            # This allows building without LazFileUtils dependency issues in some units
            if "ExtractFileNameWithoutExt" in content:
                # Regex replace: ExtractFileNameWithoutExt(ARGS) -> ChangeFileExt(ExtractFileName(ARGS), '')
                # We use a simple regex that assumes balanced parens for the function call aren't too complex.
                # Since it's usually ExtractFileNameWithoutExt(application.ExeName) or similar simple args.
                
                # Pattern: ExtractFileNameWithoutExt ( ... )
                # We capture the content inside parens
                content = re.sub(r"ExtractFileNameWithoutExt\s*\(([^)]+)\)", r"ChangeFileExt(ExtractFileName(\1), '')", content)


            if content != original_content:
                file_path.write_text(content, encoding=used_enc)
                count += 1
                # print(f"  -> Fixed {file_path.name}")
        except Exception as e:
            print(f"  [!] Failed to fix {file_path.name}: {e}")
            
    if count > 0:
        print(f"[+] Fixed compatibility issues in {count} files.")


def fix_lpi_dependencies(ce_path):
    """
    Injects missing dependencies (LazUtils) into cheatengine.lpi safely.
    Fixes 'laz_avl_Tree' build errors by ensuring LazUtils package is linked.
    """
    lpi_path = ce_path / "cheatengine.lpi"
    if not lpi_path.exists():
        print(f"[!] Warning: {lpi_path} not found. Skipping dependency fix.")
        return

    content = None
    encodings = ['utf-8', 'latin-1', 'cp1252']
    used_enc = 'utf-8'
    
    for enc in encodings:
        try:
            content = lpi_path.read_text(encoding=enc)
            used_enc = enc
            break
        except Exception:
            continue
            
    if content is None:
        print(f"[!] Error: Could not read {lpi_path} with any known encoding.")
        return

    # 1. CLEANUP PREVIOUS INJECTIONS (Bad block without Count)
    # The previous script injected <RequiredPackages> without a Count attribute.
    # The valid block has <RequiredPackages Count="...">.
    # We remove the block that starts with <RequiredPackages> and ends with </RequiredPackages> 
    # BUT only if it doesn't have attributes.
    clean_content = re.sub(r"<RequiredPackages>\s*<Item>.*?</RequiredPackages>", "", content, flags=re.DOTALL)
    
    if clean_content != content:
        print("  -> Cleaned up duplicate/bad <RequiredPackages> block.")
        content = clean_content

    # 2. CHECK IF PRESENCE
    if "LazUtils" in content and 'PackageName Value="LazUtils"' in content:
        print("  -> LazUtils dependency checks out (already present).")
        # Ensure we write back if we cleaned up
        if clean_content != content or True: # Force write to save cleanup if needed
             lpi_path.write_text(content, encoding=used_enc)
        return

    print(f"[+] Fixing dependencies in {lpi_path.name}...")
    
    # 3. INJECT INTO VALID BLOCK
    # Strategy: Inject into existing <RequiredPackages> block if it exists
    if "</RequiredPackages>" in content:
        print("  -> Injecting into existing <RequiredPackages> block...")
        
        # Update Count attribute and get new count
        new_count_val = 0
        def inc_count(match):
            nonlocal new_count_val
            prefix = match.group(1)
            count = int(match.group(2))
            suffix = match.group(3)
            new_count_val = count + 1
            return f"{prefix}{new_count_val}{suffix}"
            
        new_content = re.sub(r'(<RequiredPackages\s+Count=")(\d+)(">)', inc_count, content)
        
        # Use indexed Item tag if we found a count, else fallback to generic Item
        item_tag = f"Item{new_count_val}" if new_count_val > 0 else "Item"
        
        new_item = f"""
      <{item_tag}>
        <PackageName Value="LazUtils"/>
      </{item_tag}>"""
        
        # Insert before closing tag
        new_content = new_content.replace("</RequiredPackages>", new_item + "\n    </RequiredPackages>", 1)
        
        try:
            lpi_path.write_text(new_content, encoding=used_enc)
            print("  -> Injected LazUtils and updated package count.")
            return
        except Exception as e:
            print(f"  [!] Failed to write fix: {e}")
            return
 
    print("[!] Failed to auto-inject LazUtils. Structure not found.")


def cmd_build():
    """Build pipeline: Patch -> Build Driver -> Patch CE -> Build CE -> Assemble."""
    print("="*60)
    print("      ARC DEBUGGER BUILD PIPELINE (DYNAMIC)      ")
    print("="*60)

    # 0. Get CE Path
    CE_PROJECT = get_ce_project_path()
    if not CE_PROJECT:
        print("[!] Build cancelled.")
        return

    # Define CE-dependent paths dynamically
    PASCAL_SRC  = CE_PROJECT / "Cheat Engine" / "ArcDriver.pas"
    CE_LPI      = CE_PROJECT / "Cheat Engine" / "cheatengine.lpi"
    CE_EXE      = CE_PROJECT / "Cheat Engine" / "bin" / "cheatengine-x86_64.exe"
    LUA_DLL     = CE_PROJECT / "Cheat Engine" / "bin" / "lua53-64.dll"

    # 1. Generate Suffix
    suffix = f"{int(time.time()) & 0xFFFF:04X}"
    print(f"[1/6] Generated Suffix: MyRoot_{suffix}")

    # 1.5 Apply Patches
    print("[1.5/6] Applying Patches from 'patches/'...")
    PATCH_DIR = ARC_PROJECT / "patches" / "Cheat Engine"
    if PATCH_DIR.exists():
        for src_file in PATCH_DIR.rglob("*"):
            if src_file.is_file():
                rel_path = src_file.relative_to(PATCH_DIR)
                dest_file = CE_PROJECT / "Cheat Engine" / rel_path
                
                try:
                    shutil.copy2(src_file, dest_file)
                    print(f"  -> Patched: {rel_path}")
                except Exception as e:
                    print(f"  [!] Failed to patch {rel_path}: {e}")
    else:
        print(f"[!] Warning: No patches found at {PATCH_DIR}")

    # 2. Patch Rust Driver (In-place modification of local source)
    print("[2/6] Patching Rust Driver...")
    if not DRIVER_SRC.exists():
        print(f"[!] Error: Driver source not found at {DRIVER_SRC}")
        return

    content = DRIVER_SRC.read_text(encoding="utf-8")
    new_code = generate_driver_name_code(suffix)
    # Regex to find the existing block (flexible)
    content = re.sub(
        r"static DEV_NAME_U16.*?static SYM_NAME_U16.*?;",
        lambda m: new_code, content, flags=re.DOTALL
    )
    DRIVER_SRC.write_text(content, encoding="utf-8")
    
    # 2.5 Auto-Fix Source Issues (NEW)
    fix_source_issues(CE_PROJECT / "Cheat Engine")

    # 2.6 Auto-Rebrand Source
    print(f"[2.6/6] Auto-Rebranding 'Cheat Engine' -> '{NEW_NAME}'...")
    rebrand_source(CE_PROJECT / "Cheat Engine")

    # 3. Build Driver
    print("[3/6] Building Driver (Cargo)...")
    try:
        subprocess.run(
            "cargo build --release --target x86_64-pc-windows-msvc",
            shell=True, cwd=str(ARC_PROJECT / "rust_core"), check=True
        )
    except subprocess.CalledProcessError:
        print("[!] Cargo build failed.")
        return

    # 4. Patch Pascal Source
    print("[4/6] Patching ArcDriver.pas...")
    if not PASCAL_SRC.exists():
        print(f"[!] Warning: {PASCAL_SRC} does not exist yet. Skipping patch (first run?).")
    else:
        content = PASCAL_SRC.read_text(encoding="utf-8")
        # Find ARC_SECONDARY_DEVICE = '...' and replace
        # Use regex to replace MyRoot_XXXX or MyRoot_1234
        content = re.sub(
            r"MyRoot_[a-zA-Z0-9]{4}", 
            f"MyRoot_{suffix}", 
            content
        )
        PASCAL_SRC.write_text(content, encoding="utf-8")

    # 5. Build Cheat Engine
    print("[5/6] Building Cheat Engine (Lazbuild)...")
    
    # 5.1 Fix Dependencies (LazUtils for laz_avl_Tree)
    fix_lpi_dependencies(CE_PROJECT / "Cheat Engine")

    # 5.2 Clean build artifacts (Nuclear Option)
    print("  -> Cleaning ALL build artifacts recursively...")
    ce_root = CE_PROJECT / "Cheat Engine"
    
    # Remove all 'lib' and 'backup' directories
    for path in ce_root.rglob("lib"):
        if path.is_dir(): shutil.rmtree(path, ignore_errors=True)
    for path in ce_root.rglob("backup"):
        if path.is_dir(): shutil.rmtree(path, ignore_errors=True)
        
    # Remove all .ppu and .o files (in case they are mixed in source)
    for ext in ["*.ppu", "*.o", "*.a", "*.or"]:
        for path in ce_root.rglob(ext):
            try: path.unlink()
            except: pass


    try:
        subprocess.run(
            [str(LAZBUILD_PATH), str(CE_LPI), "--build-mode=Release 64-Bit", "--no-write-project", "-B"],
            check=True
        )
    except FileNotFoundError:
        print(f"[!] Lazbuild not found at {LAZBUILD_PATH}. Check the path.")
        return
    except subprocess.CalledProcessError:
        print("[!] Lazbuild failed.")
        return

    # 6. Assemble Output
    print("[6/6] Assembling Output...")
    OUTPUT_DIR.mkdir(exist_ok=True)
    
    # Copy Driver
    if DRIVER_SYS.exists():
        shutil.copy2(DRIVER_SYS, OUTPUT_DIR / f"MyRoot_{suffix}.sys")
    else:
        print("[!] Driver .sys not found!")

    # Copy CE Exe
    # Note: lazbuild output location might vary. Assuming standard.
    # We might need to find it if it's not at CE_EXE
    ce_out = CE_EXE
    if not ce_out.exists(): 
        ce_out = CE_PROJECT / "Cheat Engine" / "cheatengine.exe" # Try root default

    if ce_out.exists():
        shutil.copy2(ce_out, OUTPUT_DIR / "arcdebugger.exe")
    else:
        print(f"[!] Cheat Engine executable not found at {ce_out}")

    # Copy Dependencies (Lua)
    if LUA_DLL.exists():
        shutil.copy2(LUA_DLL, OUTPUT_DIR / "lua53-64.dll")
    else:
        print(f"[!] Warning: lua53-64.dll not found at {LUA_DLL}")

    # Copy KDU and dependencies
    KDU_EXE = ARC_PROJECT / "tools" / "kdu.exe"
    DRV64_DLL = ARC_PROJECT / "tools" / "drv64.dll"

    if KDU_EXE.exists():
        shutil.copy2(KDU_EXE, OUTPUT_DIR / "kdu.exe")
    
    if DRV64_DLL.exists():
        shutil.copy2(DRV64_DLL, OUTPUT_DIR / "drv64.dll")
    
    # Create Runner
    (OUTPUT_DIR / "run.bat").write_text(
        f'@echo off\ncd /d "%~dp0"\necho Loading Driver MyRoot_{suffix}...\nkdu.exe -map MyRoot_{suffix}.sys\ntimeout /t 2\nstart "" arcdebugger.exe\n', 
        encoding="utf-8"
    )

    print(f"\n[SUCCESS] Build complete! Output is in: {OUTPUT_DIR}")


def cmd_deploy():
    """Build and Run."""
    cmd_build()
    print("\n[!] Launching deployment...")
    bat_path = OUTPUT_DIR / "run.bat"
    if bat_path.exists():
        # Requires Admin usually
        subprocess.run(f'powershell Start-Process "{bat_path}" -Verb RunAs', shell=True)

if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "deploy":
        cmd_deploy()
    else:
        cmd_build()
