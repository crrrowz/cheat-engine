// ============================================================================
//  ARC CORE — Dynamic Configuration Engine (Rust FFI)
// ============================================================================

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

// ----------------------------------------------------------------------------
//  Imports
// ----------------------------------------------------------------------------
use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use winapi::shared::minwindef::{DWORD, LPVOID};
use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::ioapiset::DeviceIoControl;
use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use winapi::um::processthreadsapi::GetCurrentProcessId;
use winapi::um::winnt::{
    FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE, HANDLE,
};

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

// ----------------------------------------------------------------------------
//  §1 — Dynamic Configuration
// ----------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OffsetConfig {
    pub shuffle_offset_1: u64,
    pub shuffle_offset_2: u64,
    pub key_const: u64,
    pub bone_array_offset: u64,
    pub bone_index_offset: u64,
    pub uworld_encrypt_offset: u64,
}

static mut OFFSET_CONFIG: OffsetConfig = OffsetConfig {
    shuffle_offset_1: 0xA94B00,
    shuffle_offset_2: 0xAF231F0,
    key_const: 0x1554577E835E9F4,
    bone_array_offset: 0x7B0,
    bone_index_offset: 0x7FC,
    uworld_encrypt_offset: 0x2A0,
};

static mut MOCK_MODE: bool = false;

#[no_mangle]
pub extern "C" fn core_set_mock_mode(enabled: bool) {
    unsafe { MOCK_MODE = enabled; }
    if enabled { println!("[arc_core] MOCK MODE ACTIVATED."); }
}

#[inline(always)]
fn get_config() -> OffsetConfig {
    unsafe { OFFSET_CONFIG }
}

fn read<T: Copy + Default>(handle: HANDLE, pid: u32, address: u64) -> T {
    let mut res: T = Default::default();
    driver_read(handle, pid, address, &mut res as *mut T as *mut c_void, std::mem::size_of::<T>());
    res
}

// ----------------------------------------------------------------------------
//  §2 — FFI Exports: Configuration
// ----------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn core_update_offsets(
    shuffle_1: u64, shuffle_2: u64, key: u64, 
    bone_array: u64, bone_index: u64, uworld_encrypt: u64
) {
    unsafe {
        OFFSET_CONFIG.shuffle_offset_1 = shuffle_1;
        OFFSET_CONFIG.shuffle_offset_2 = shuffle_2;
        OFFSET_CONFIG.key_const = key;
        OFFSET_CONFIG.bone_array_offset = bone_array;
        OFFSET_CONFIG.bone_index_offset = bone_index;
        OFFSET_CONFIG.uworld_encrypt_offset = uworld_encrypt;
    }
}

// ----------------------------------------------------------------------------
//  §3 — Decryption Logic
// ----------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
unsafe fn bone_array_decrypt_impl(handle: HANDLE, pid: u32, game_base: u64, mesh: u64) -> u64 {
    let cfg = get_config();
    let shuffle_addr = game_base + cfg.shuffle_offset_1;
    let shuffle_mask: [u8; 16] = read(handle, pid, shuffle_addr);
    let xmm_shuffle = _mm_loadu_si128(shuffle_mask.as_ptr() as *const __m128i);

    let bones_mesh_addr = mesh + cfg.bone_array_offset;
    let bones_mesh_raw: [u8; 16] = read(handle, pid, bones_mesh_addr);
    let bones_mesh = _mm_loadu_si128(bones_mesh_raw.as_ptr() as *const __m128i);

    let mut v8 = _mm_shuffle_epi8(bones_mesh, xmm_shuffle);
    let sll = _mm_slli_epi32(v8, 0x11);
    let srl = _mm_srli_epi32(v8, 0x0F);
    let rotated = _mm_or_si128(sll, srl);
    let rotated_lo = _mm_extract_epi64(rotated, 0) as u64;
    v8 = _mm_insert_epi64(v8, rotated_lo as i64, 0);
    let v8_lo = _mm_extract_epi64(v8, 0) as u64;
    let xored = v8_lo ^ 0x4834C6DEA02581C7u64;

    let bone_index_val: i32 = read(handle, pid, mesh + cfg.bone_index_offset);
    let v9 = xored.wrapping_add((bone_index_val as u64) * 16);
    let mut bone_array: u64 = read(handle, pid, v9 + 0x88);
    if bone_array < 0x10000 {
        bone_array = read(handle, pid, v9 + 0x98);
    }
    bone_array
}

#[cfg(target_arch = "x86_64")]
unsafe fn game_instance_decrypt_impl(handle: HANDLE, pid: u32, game_base: u64, uworld_addr: u64) -> u64 {
    let cfg = get_config();
    let tmp1_raw: [u8; 16] = read(handle, pid, uworld_addr + cfg.uworld_encrypt_offset);
    let tmp1 = _mm_loadu_si128(tmp1_raw.as_ptr() as *const __m128i);
    let tmp2_raw: [u8; 16] = read(handle, pid, game_base + cfg.shuffle_offset_2);
    let tmp2 = _mm_loadu_si128(tmp2_raw.as_ptr() as *const __m128i);

    let v5 = _mm_shuffle_epi8(tmp1, tmp2);
    let sll = _mm_slli_epi16(v5, 2);
    let srl = _mm_srli_epi16(v5, 0x0E);
    let combined = _mm_or_si128(sll, srl);
    let v6 = _mm_extract_epi64(combined, 0) as u64;

    if v6 == cfg.key_const { uworld_addr } else { v6 ^ cfg.key_const }
}

// ----------------------------------------------------------------------------
//  §4 — Common Structures
// ----------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Vec3 { pub x: f64, pub y: f64, pub z: f64 }

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Quat { pub x: f64, pub y: f64, pub z: f64, pub w: f64 }

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Transform {
    pub rot: Quat,
    pub translation: Vec3,
    pub _pad: u32,
    pub scale: Vec3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CameraInfo {
    pub location: Vec3,
    pub rotation: Vec3,
    pub fov: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
    pub valid: i32,
}

impl Vec3 {
    fn sub(&self, other: &Vec3) -> Vec3 {
        Vec3 { x: self.x - other.x, y: self.y - other.y, z: self.z - other.z }
    }
    fn dot(&self, other: &Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }
}

// ----------------------------------------------------------------------------
//  §5 — Math Logic
// ----------------------------------------------------------------------------

fn transform_point(pos: &Vec3, transform: &Transform) -> Vec3 {
    let x = pos.x * transform.scale.x;
    let y = pos.y * transform.scale.y;
    let z = pos.z * transform.scale.z;

    let q = transform.rot;
    // Rotate vector by quaternion
    let num12 = q.x + q.x;
    let num2 = q.y + q.y;
    let num = q.z + q.z;
    let num11 = q.x * num12;
    let num10 = q.x * num2;
    let num9 = q.x * num;
    let num8 = q.y * num2;
    let num7 = q.y * num;
    let num6 = q.z * num;
    let num5 = q.w * num12;
    let num4 = q.w * num2;
    let num3 = q.w * num;

    let rx = (1.0 - (num8 + num6)) * x + (num10 - num3) * y + (num9 + num4) * z;
    let ry = (1.0 - (num11 + num6)) * y + (num10 + num3) * x + (num7 - num5) * z;
    let rz = (1.0 - (num11 + num8)) * z + (num9 - num4) * x + (num7 + num5) * y;

    Vec3 {
        x: rx + transform.translation.x,
        y: ry + transform.translation.y,
        z: rz + transform.translation.z,
    }
}

fn get_camera_axes(rotation: Vec3) -> (Vec3, Vec3, Vec3) {
    let rad = std::f64::consts::PI / 180.0;
    let pitch = rotation.x * rad;
    let yaw = rotation.y * rad;
    let roll = rotation.z * rad;

    let sp = pitch.sin(); let cp = pitch.cos();
    let sy = yaw.sin();   let cy = yaw.cos();
    let sr = roll.sin();  let cr = roll.cos();

    let forward = Vec3 { x: cp * cy, y: cp * sy, z: sp };
    let right = Vec3 { x: -1.0 * (sr * sp * cy + cr * -sy), y: -1.0 * (sr * sp * sy + cr * cy), z: -1.0 * (sr * cp) };
    let up = Vec3 { x: cr * sp * cy + -sr * -sy, y: cr * sp * sy + -sr * cy, z: cr * cp };

    (forward, right, up)
}

fn world_to_screen_math(world: &Vec3, cam: &CameraInfo, width: f32, height: f32) -> ScreenPoint {
    let (forward, right, up) = get_camera_axes(cam.rotation);
    let delta = world.sub(&cam.location);

    let cam_x = delta.dot(&right);
    let cam_y = delta.dot(&up);
    let cam_z = delta.dot(&forward);

    if cam_z < 0.1 { return ScreenPoint { x: 0.0, y: 0.0, valid: 0 }; }

    let aspect_ratio = width / height;
    let fov_rad = (cam.fov as f32).to_radians();
    let half_fov_tan = (fov_rad / 2.0).tan() as f64;

    let x = (cam_x / cam_z) / half_fov_tan;
    let y = (cam_y / cam_z) / (half_fov_tan / aspect_ratio as f64);

    let screen_x = (0.5 + x / 2.0) * (width as f64);
    let screen_y = (0.5 - y / 2.0) * (height as f64);

    ScreenPoint { x: screen_x, y: screen_y, valid: 1 }
}

// ----------------------------------------------------------------------------
//  §6 — Driver Logic
// ----------------------------------------------------------------------------

struct DriverState { handle: HANDLE, pid: u32, base_address: u64 }
unsafe impl Send for DriverState {}
static DRIVER: Mutex<Option<DriverState>> = Mutex::new(None);

#[repr(C)]
struct KmemRequest { process_id: u64, address: u64, size: u64 }

const IOCTL_READ: DWORD = 0x222000;          // func 0x800 = ReadProcessMemory
const IOCTL_GET_BASE: DWORD = 0x222008;      // func 0x802 = GetBaseAddress
const IOCTL_AUTH: DWORD = 0x222010;          // func 0x804 = SetAuthorizedPID
const XOR_KEY_CORE: u64 = 0xDEADBEEFCAFEBABE;

fn driver_read(handle: HANDLE, pid: u32, address: u64, buffer: *mut c_void, size: usize) -> bool {
    let req = KmemRequest {
        process_id: (pid as u64) ^ XOR_KEY_CORE,
        address: address ^ XOR_KEY_CORE,
        size: (size as u64) ^ XOR_KEY_CORE,
    };
    let mut bytes: DWORD = 0;
    unsafe {
        DeviceIoControl(
            handle, IOCTL_READ, &req as *const _ as LPVOID,
            std::mem::size_of::<KmemRequest>() as DWORD,
            buffer as LPVOID, size as DWORD,
            &mut bytes, ptr::null_mut()
        ) != 0
    }
}

// ----------------------------------------------------------------------------
//  §7 — FFI Exports: Public API
// ----------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn core_init(game_name_ptr: *const u8, _module_name_ptr: *const u8) -> i32 {
    unsafe {
        if MOCK_MODE {
            println!("[arc_core] Initialized in MOCK MODE (Skipping Driver).");
            return 1;
        }
    }

    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    
    // Try both device names: Nul (KDU mapped) and MyRoot (SCM loaded)
                                                                                                                                                                                let devices = [r"\\.\Nul", r"\\.\MyRoot_9123"];
    let mut handle = INVALID_HANDLE_VALUE;
    
    for dev in &devices {
        let path = std::ffi::OsStr::new(dev);
        let wide: Vec<u16> = path.encode_wide().chain(once(0)).collect();
        let h = unsafe {
            CreateFileW(wide.as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null_mut(), OPEN_EXISTING, 0, ptr::null_mut())
        };
        if h != INVALID_HANDLE_VALUE {
            println!("[arc_core] Connected via {}", dev);
            handle = h;
            break;
        }
    }

    if handle == INVALID_HANDLE_VALUE { return 0; }
    
    // Authorize this process with the driver (required before any other IOCTL)
    {
        let my_pid = unsafe { GetCurrentProcessId() };
        let auth_req = KmemRequest {
            process_id: (my_pid as u64) ^ XOR_KEY_CORE,
            address: 0,
            size: 0,
        };
        let mut bytes: DWORD = 0;
        let auth_ok = unsafe {
            DeviceIoControl(
                handle, IOCTL_AUTH, &auth_req as *const _ as LPVOID,
                std::mem::size_of::<KmemRequest>() as DWORD,
                ptr::null_mut(), 0, &mut bytes, ptr::null_mut()
            )
        };
        if auth_ok == 0 {
            println!("[arc_core] Auth failed");
            unsafe { CloseHandle(handle); }
            return 0;
        }
        println!("[arc_core] Authorized PID {}", my_pid);
    }

    // Find the game process
    let pid = unsafe { find_process_id("PioneerGame.exe") };
    if pid == 0 { unsafe { CloseHandle(handle); } return -1; }
    
    let base = unsafe { get_module_base(handle, pid, "PioneerGame.exe") };
    if base == 0 { unsafe { CloseHandle(handle); } return -2; }

    let mut state = DRIVER.lock().unwrap();
    *state = Some(DriverState { handle, pid, base_address: base });
    1 
}

#[no_mangle]
#[cfg(target_arch = "x86_64")]
pub extern "C" fn core_decrypt_bone_array(mesh_ptr: u64) -> u64 {
    if unsafe { MOCK_MODE } { return 0; }
    let state = DRIVER.lock().unwrap();
    match state.as_ref() {
        Some(s) => unsafe { bone_array_decrypt_impl(s.handle, s.pid, s.base_address, mesh_ptr) },
        None => 0,
    }
}

#[no_mangle]
#[cfg(target_arch = "x86_64")]
pub extern "C" fn core_decrypt_game_instance(uworld_addr: u64) -> u64 {
    if unsafe { MOCK_MODE } { return 0; }
    let state = DRIVER.lock().unwrap();
    match state.as_ref() {
        Some(s) => unsafe { game_instance_decrypt_impl(s.handle, s.pid, s.base_address, uworld_addr) },
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn core_read_u64(address: u64) -> u64 {
    if unsafe { MOCK_MODE } { return 0; }
    let state = DRIVER.lock().unwrap();
    match state.as_ref() { Some(s) => read(s.handle, s.pid, address), None => 0 }
}

#[no_mangle]
pub extern "C" fn core_read_u32(address: u64) -> u32 {
    if unsafe { MOCK_MODE } { return 0; }
    let state = DRIVER.lock().unwrap();
    match state.as_ref() { Some(s) => read(s.handle, s.pid, address), None => 0 }
}

#[no_mangle]
pub extern "C" fn core_read_i32(address: u64) -> i32 {
    if unsafe { MOCK_MODE } { return 0; }
    let state = DRIVER.lock().unwrap();
    match state.as_ref() { Some(s) => read(s.handle, s.pid, address), None => 0 }
}

#[no_mangle]
pub extern "C" fn core_read_transform(address: u64) -> Transform {
    if unsafe { MOCK_MODE } { return Default::default(); }
    let state = DRIVER.lock().unwrap();
    match state.as_ref() { Some(s) => read(s.handle, s.pid, address), None => Default::default() }
}

#[no_mangle]
pub extern "C" fn core_get_bones_batch(
    indices_ptr: *const u32, count: u32, bone_array_ptr: u64, c2w: &Transform
) -> *mut Vec3 {
    static mut BONE_BUFFER: [Vec3; 100] = [Vec3{x:0.0, y:0.0, z:0.0}; 100];
    
    if unsafe { MOCK_MODE } {
        unsafe {
            for i in 0..count.min(100) as usize {
                BONE_BUFFER[i] = c2w.translation; 
                if i == 0 { BONE_BUFFER[i].z += 160.0; } // Head
                else if i == 1 { BONE_BUFFER[i].y -= 30.0; BONE_BUFFER[i].z += 100.0; } 
                else if i == 2 { BONE_BUFFER[i].y += 30.0; BONE_BUFFER[i].z += 100.0; }
            }
            return BONE_BUFFER.as_mut_ptr();
        }
    }

    let state = DRIVER.lock().unwrap();
    if let Some(s) = state.as_ref() {
        unsafe {
            let slice = std::slice::from_raw_parts(indices_ptr, count as usize);
            for (i, &bone_idx) in slice.iter().enumerate() {
                if i >= 100 { break; }
                let bone_addr = bone_array_ptr + (bone_idx as u64) * 48;
                let bone_transform: Transform = read(s.handle, s.pid, bone_addr);
                let world_pos = transform_point(&bone_transform.translation, c2w);
                BONE_BUFFER[i] = world_pos;
            }
            BONE_BUFFER.as_mut_ptr()
        }
    } else {
        ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn core_world_to_screen_batch(
    world_points_ptr: *const Vec3, count: u32, cam: &CameraInfo, width: f32, height: f32
) -> *mut ScreenPoint {
    static mut SCREEN_BUFFER: [ScreenPoint; 100] = [ScreenPoint{x:0.0, y:0.0, valid:0}; 100];
    unsafe {
        let slice = std::slice::from_raw_parts(world_points_ptr, count as usize);
        for (i, &pt) in slice.iter().enumerate() {
            if i >= 100 { break; }
            SCREEN_BUFFER[i] = world_to_screen_math(&pt, cam, width, height);
        }
        SCREEN_BUFFER.as_mut_ptr()
    }
}

#[no_mangle]
pub extern "C" fn core_get_base_address() -> u64 {
    let state = DRIVER.lock().unwrap();
    match state.as_ref() {
        Some(s) => s.base_address,
        None => 0
    }
}

// ----------------------------------------------------------------------------
//  §8 — Helpers
// ----------------------------------------------------------------------------

unsafe fn find_process_id(name: &str) -> u32 {
    let mut entry: PROCESSENTRY32W = std::mem::zeroed();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snap == INVALID_HANDLE_VALUE { return 0; }

    if Process32FirstW(snap, &mut entry) != 0 {
        loop {
            let p_name = String::from_utf16_lossy(&entry.szExeFile);
            let p_name = p_name.trim_matches('\0');
            if p_name.eq_ignore_ascii_case(name) {
                CloseHandle(snap);
                return entry.th32ProcessID;
            }
            if Process32NextW(snap, &mut entry) == 0 { break; }
        }
    }
    CloseHandle(snap);
    0
}

unsafe fn get_module_base(handle: HANDLE, pid: u32, _name: &str) -> u64 {
    // Use Kernel Driver to get Base Address (Bypass Anti-Cheat)
    
    // In our driver implementation:
    // func 0x802 (IOCTL_GET_BASE) expects: 
    // - process_id (encrypted): CALLER PID (for auth check)
    // - address (encrypted): TARGET PID (process to find base of)
    // - size (ignored)
    
    let my_pid = GetCurrentProcessId();
    let req = KmemRequest {
        process_id: (my_pid as u64) ^ XOR_KEY_CORE,
        address: (pid as u64) ^ XOR_KEY_CORE,
        size: 0,
    };
    
    let mut base_addr: u64 = 0;
    let mut bytes: DWORD = 0;
    
    let status = DeviceIoControl(
        handle, IOCTL_GET_BASE, 
        &req as *const _ as LPVOID, std::mem::size_of::<KmemRequest>() as DWORD,
        &mut base_addr as *mut _ as LPVOID, std::mem::size_of::<u64>() as DWORD,
        &mut bytes, ptr::null_mut()
    );

    if status == 0 {
        return 0;
    }
    
    base_addr
}

#[no_mangle]
pub extern "C" fn core_scan_ue_chain() -> u64 {
    if unsafe { MOCK_MODE } { return 0; }
    
    // Explicit import for WinAPI void pointer
    use winapi::ctypes::c_void;

    // --- Local Struct Definitions to avoid dependencies ---
    #[repr(C)]
    struct CoreRequest {
        process_id: u64,
        address: u64,
        size: u64,
    }
    const IOCTL_READ: u32 = 0x222000;
    const XOR_KEY: u64 = 0xDEADBEEFCAFEBABE;

    // Helper: Read U64
    let read_u64 = |h: winapi::um::winnt::HANDLE, pid: u32, addr: u64| -> u64 {
        let mut val: u64 = 0;
        // Cast PID to u64 for the struct
        let mut req = CoreRequest { process_id: pid as u64, address: addr, size: 8 };
        req.process_id ^= XOR_KEY;
        req.address ^= XOR_KEY;
        req.size ^= XOR_KEY;
        let mut bytes = 0u32;
        unsafe {
            winapi::um::ioapiset::DeviceIoControl(
                h, IOCTL_READ,
                &mut req as *mut _ as *mut c_void,
                std::mem::size_of::<CoreRequest>() as u32,
                &mut val as *mut _ as *mut c_void,
                8,
                &mut bytes,
                std::ptr::null_mut()
            );
        }
        val
    };

    // Get Base internally
    let base = core_get_base_address();
    if base == 0 {
         println!("[rust_core] Internal GetBase failed.");
         return 0;
    }

    let scan_size: u64 = 500 * 1024 * 1024; // 500MB
    let chunk_size: u64 = 0x1000; // 4KB (Page Scans are safer)
    let mut buffer = vec![0u8; chunk_size as usize];

    // 1. Capture Handle/PID from Driver State
    let (handle, pid) = {
        let state_guard = DRIVER.lock().unwrap();
        match state_guard.as_ref() {
            Some(s) => (s.handle, s.pid),
            None => return 0,
        }
    }; 

    println!("[rust_core] Starting Heuristic Scan on 500MB with PID: {}...", pid); 

    let mut offset: u64 = 0;
    while offset < scan_size {
        let read_addr = base + offset;
        
        if offset == 0 {
             println!("[rust_core] Attempting read at Base: 0x{:X}", read_addr);
        }
        if offset % (50 * 1024 * 1024) == 0 {
            println!("[rust_core] Scanning Offset: +{}MB", offset / 1024 / 1024);
        }

        // Read Chunk via Driver
        // Cast PID to u64
        let mut req = CoreRequest { process_id: pid as u64, address: read_addr, size: chunk_size };
        req.process_id ^= XOR_KEY;
        req.address ^= XOR_KEY;
        req.size ^= XOR_KEY;
        let mut bytes = 0u32;
        let status = unsafe {
            winapi::um::ioapiset::DeviceIoControl(
                handle, IOCTL_READ,
                &mut req as *mut _ as *mut c_void,
                std::mem::size_of::<CoreRequest>() as u32,
                buffer.as_mut_ptr() as *mut c_void,
                chunk_size as u32,
                &mut bytes,
                std::ptr::null_mut()
            )
        };

        if status == 0 {
            if offset == 0 { 
                let err = unsafe { winapi::um::errhandlingapi::GetLastError() };
                println!("[rust_core] [ERROR] Read failed at Base! Error Code: {}", err); 
            }
            offset += chunk_size;
            continue;
        }
        
        let u64_count = (chunk_size / 8) as usize;
        let ptrs = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u64, u64_count) };
        
        for (i, &ptr_val) in ptrs.iter().enumerate() {
             if ptr_val > 0x10000 && ptr_val < 0x7FFFFFFFFFFF {
                 
                 // Check P + 0x30 (PersistentLevel)
                 let level_ptr = read_u64(handle, pid, ptr_val + 0x30);
                 if level_ptr > 0x10000 && level_ptr < 0x7FFFFFFFFFFF {
                     
                     let found_offset = offset + (i as u64 * 8);
                     
                     // Check GameInstances loosely
                     let gi = read_u64(handle, pid, ptr_val + 0x180);
                     let gi2 = read_u64(handle, pid, ptr_val + 0x190);
                     let gi3 = read_u64(handle, pid, ptr_val + 0x1A0);
                     
                     let has_gi = (gi > 0x10000 && gi < 0x7FFFFFFFFFFF) || 
                                  (gi2 > 0x10000 && gi2 < 0x7FFFFFFFFFFF) || 
                                  (gi3 > 0x10000 && gi3 < 0x7FFFFFFFFFFF);
                     
                     if has_gi {
                          println!("[rust_core] [STRONG MATCH] UWorld Found! Offset: 0x{:X}", found_offset);
                          return found_offset;
                     } else {
                          println!("[rust_core] [Weak Candidate] P+0x30 valid at Offset: 0x{:X}", found_offset);
                     }
                 }
             }
        }
        
        offset += chunk_size;
    }
    
    0
}

// ============================================================================
//  §4 — Deep Player Chain Scanner (Brute Force)
// ============================================================================

#[repr(C)]
pub struct PlayerChainResult {
    pub game_instance_offset: u64,
    pub local_players_offset: u64,
    pub player_controller_offset: u64,
    pub pawn_offset: u64,
}

#[no_mangle]
pub extern "C" fn core_scan_player_chain(uworld: u64) -> PlayerChainResult {
    let mut res = PlayerChainResult { 
        game_instance_offset: 0, 
        local_players_offset: 0, 
        player_controller_offset: 0,
        pawn_offset: 0 
    };

    if unsafe { MOCK_MODE } || uworld < 0x10000 { return res; }

    use winapi::ctypes::c_void;

    // --- Local Struct Definitions ---
    #[repr(C)]
    struct CoreRequest {
        process_id: u64,
        address: u64,
        size: u64,
    }
    const IOCTL_READ: u32 = 0x222000;
    const XOR_KEY: u64 = 0xDEADBEEFCAFEBABE;

    // 1. Capture Handle/PID from Driver State
    let (handle, pid) = {
        let state_guard = DRIVER.lock().unwrap();
        match state_guard.as_ref() {
            Some(s) => (s.handle, s.pid),
            None => return res,
        }
    };
    
    // Helper: Read U64
    let read_u64_internal = |h: winapi::um::winnt::HANDLE, pid: u32, addr: u64| -> u64 {
        let mut val: u64 = 0;
        let mut req = CoreRequest { process_id: pid as u64, address: addr, size: 8 };
        req.process_id ^= XOR_KEY;
        req.address ^= XOR_KEY;
        req.size ^= XOR_KEY;
        let mut bytes = 0u32;
        unsafe {
            winapi::um::ioapiset::DeviceIoControl(
                h, IOCTL_READ,
                &mut req as *mut _ as *mut c_void,
                std::mem::size_of::<CoreRequest>() as u32,
                &mut val as *mut _ as *mut c_void,
                8,
                &mut bytes,
                std::ptr::null_mut()
            );
        }
        val
    };

    println!("[rust_core] Deep Scan for Player Chain at UWorld: 0x{:X}", uworld);

    // A. Scan for GameInstance candidates in UWorld (0x100 to 0x300)
    for gi_off in (0x100..0x300).step_by(8) {
        let gi_ptr = read_u64_internal(handle, pid, uworld + gi_off);

        if gi_ptr < 0x10000000000 || gi_ptr > 0x7FFFFFFFFFFF { continue; }

        // B. Inside GI, scan for TArray<ULocalPlayer*> (0x0 to 0x100)
        // Looking for valid Ptr, Small Count, Matching Max
        for lp_arr_off in (0x0..0x200).step_by(8) {
             let arr_ptr = read_u64_internal(handle, pid, gi_ptr + lp_arr_off);
             if arr_ptr < 0x10000000000 || arr_ptr > 0x7FFFFFFFFFFF { continue; }
             
             let cnt_max = read_u64_internal(handle, pid, gi_ptr + lp_arr_off + 8);
             let cnt = (cnt_max & 0xFFFFFFFF) as u32;
             let max_v = ((cnt_max >> 32) & 0xFFFFFFFF) as u32;

             if cnt > 0 && cnt < 10 {
                 // C. Valid Array! Check first element (ULocalPlayer*)
                 let first_lp = read_u64_internal(handle, pid, arr_ptr);
                 if first_lp < 0x10000000000 || first_lp > 0x7FFFFFFFFFFF { continue; }

                 // D. Check PlayerController inside ULocalPlayer (Try 0x30, 0x0..0x60)
                 let pc_scan_range = [0x30, 0x40, 0x28, 0x50, 0x38, 0x60];
                 
                 for &pc_off in pc_scan_range.iter() {
                     let pc = read_u64_internal(handle, pid, first_lp + pc_off);

                     if pc < 0x10000000000 || pc > 0x7FFFFFFFFFFF { continue; }
                     
                     // E. Check AcknowledgedPawn inside PC (Try 0x2A0 to 0x600)
                     let pawn_start = 0x2A0;
                     let pawn_end = 0x600;
                     let mut found_pawn = 0;
                     
                     for pawn_off in (pawn_start..pawn_end).step_by(8) {
                         let pawn = read_u64_internal(handle, pid, pc + pawn_off);
                         if pawn > 0x10000000000 && pawn < 0x7FFFFFFFFFFF {
                             // Found Pawn, prefer this
                             found_pawn = pawn_off;
                             break;
                         }
                     }
                     
                     // If we have PC, we accept it even if Pawn is 0 (Menu Mode)
                     println!("[rust_core] CHAIN FOUND! GI:0x{:X} LP_Arr:0x{:X} PC:0x{:X} PawnOffset:0x{:X}", 
                              gi_off, lp_arr_off, pc_off, found_pawn);
                     
                     res.game_instance_offset = gi_off;
                     res.local_players_offset = lp_arr_off;
                     res.player_controller_offset = pc_off;
                     res.pawn_offset = if found_pawn != 0 { found_pawn } else { 0x308 }; // Default if null
                     return res;
                 }
             }
        }
    }
    
    println!("[rust_core] Deep Scan Failed.");
    res
}

#[no_mangle]
pub extern "C" fn core_scan_actors_array(uworld: u64, known_pawn: u64) -> u64 {
    if unsafe { MOCK_MODE } || uworld < 0x10000 { return 0; }

    use winapi::ctypes::c_void;

    // Reuse local definition (simplified for this scope)
    #[repr(C)]
    struct CoreRequest {
        process_id: u64,
        address: u64,
        size: u64,
    }
    const IOCTL_READ: u32 = 0x222000;
    const XOR_KEY: u64 = 0xDEADBEEFCAFEBABE;

    let (handle, pid) = {
        let state_guard = DRIVER.lock().unwrap();
        match state_guard.as_ref() {
            Some(s) => (s.handle, s.pid),
            None => return 0,
        }
    };
    
    let read_u64_internal = |h: winapi::um::winnt::HANDLE, pid: u32, addr: u64| -> u64 {
        let mut val: u64 = 0;
        let mut req = CoreRequest { process_id: pid as u64, address: addr, size: 8 };
        req.process_id ^= XOR_KEY;
        req.address ^= XOR_KEY;
        req.size ^= XOR_KEY;
        let mut bytes = 0u32;
        unsafe {
            winapi::um::ioapiset::DeviceIoControl(
                h, IOCTL_READ,
                &mut req as *mut _ as *mut c_void,
                std::mem::size_of::<CoreRequest>() as u32,
                &mut val as *mut _ as *mut c_void,
                8,
                &mut bytes,
                std::ptr::null_mut()
            );
        }
        val
    };
    
    // 2. Double Deep Scan: Find Level AND Actors Array
    // Loop through UWorld to find PersistentLevel (usually 0x30, but could be else)
    println!("[rust_core] Starting AGGRESSIVE Double Deep Scan (0x0-0x500)...");
    
    for level_off in (0x20..0x400).step_by(8) {
        let level_ptr = read_u64_internal(handle, pid, uworld + level_off);
        if level_ptr < 0x10000000000 || level_ptr > 0x7FFFFFFFFFFF { continue; }

        // For this candidate Level, scan for Actors Array
        for actors_off in (0x50..0x500).step_by(8) {
            let arr_ptr = read_u64_internal(handle, pid, level_ptr + actors_off);
            if arr_ptr < 0x10000000000 || arr_ptr > 0x7FFFFFFFFFFF { continue; }

            let cnt_max = read_u64_internal(handle, pid, level_ptr + actors_off + 8);
            let cnt = (cnt_max & 0xFFFFFFFF) as u32;
            let max_v = ((cnt_max >> 32) & 0xFFFFFFFF) as u32;

            if cnt > 10 && cnt < 20000 && max_v >= cnt {
                 // Valid Array structure.
                 println!("[rust_core] CANDIDATE Level:0x{:X} | Actors:0x{:X} | Count:{}", level_off, actors_off, cnt);

                 // Check content for Pawn.
                 // Limit to 5000 checks
                 let check_limit = if cnt > 5000 { 5000 } else { cnt };
                 
                 for i in 0..check_limit {
                     let actor = read_u64_internal(handle, pid, arr_ptr + (i as u64 * 8));
                     if actor == known_pawn {
                         println!("[rust_core] MATCH!!! Level:0x{:X} | Actors:0x{:X} | Count:{}", level_off, actors_off, cnt);
                         return (level_off << 32) | actors_off;
                     }
                 }
            }
        }
    }
    
    println!("[rust_core] AGGRESSIVE Scan Failed.");
    0
}
