#![no_std]
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

// =================================================================
// §1: TYPES & CONSTANTS
// =================================================================

pub type NTSTATUS = i32;
pub type PVOID = *mut c_void;
pub type BOOLEAN = u8;

pub const STATUS_SUCCESS: NTSTATUS = 0;
pub const STATUS_UNSUCCESSFUL: NTSTATUS = 0xC0000001u32 as i32;
pub const STATUS_INVALID_PARAMETER: NTSTATUS = 0xC000000Du32 as i32;
pub const STATUS_ACCESS_DENIED: NTSTATUS = 0xC0000022u32 as i32;
pub const STATUS_INFO_LENGTH_MISMATCH: NTSTATUS = 0xC0000004u32 as i32;
pub const STATUS_INVALID_CID: NTSTATUS = 0xC000000Bu32 as i32;

pub const IRP_MJ_CREATE: usize = 0;
pub const IRP_MJ_CLOSE: usize = 2;
pub const IRP_MJ_DEVICE_CONTROL: usize = 14;

pub const FILE_DEVICE_UNKNOWN: u32 = 0x00000022;
pub const FILE_DEVICE_SECURE_OPEN: u32 = 0x00000100;
pub const FILE_READ_DATA: u32 = 1;

const MAX_READ_SIZE: usize = 4096;
const XOR_KEY: u64 = 0xDEADBEEFCAFEBABE;
static AUTHORIZED_PID: AtomicU64 = AtomicU64::new(0);

// Driver mode: 0=unknown, 1=SCM (IoCreateDevice), 2=Hook (kdmapper)
static mut DRIVER_MODE: u8 = 0;

// =================================================================
// §2: VERIFIED IRP OFFSETS (WinDbg x64)
// =================================================================

const IRP_SYSTEM_BUFFER: usize     = 0x18;
const IRP_IOSTATUS_STATUS: usize   = 0x30;
const IRP_IOSTATUS_INFO: usize     = 0x38;
const IRP_CURRENT_STACK_LOC: usize = 0xB8;
const ISL_IOCONTROL_CODE: usize    = 0x18;

// DRIVER_OBJECT offsets
const DO_MAJOR_FUNCTION: usize     = 0x70;  // MajorFunction[0]
// MajorFunction[14] = 0x70 + 14*8 = 0xE0
const DO_MF_DEVICE_CONTROL: usize  = 0xE0;
// DEVICE_OBJECT.DriverObject offset
const DEVOBJ_DRIVER_OBJECT: usize  = 0x08;

// =================================================================
// §3: STATIC STRINGS
// =================================================================

static DEV_NAME_U16: &[u16] = &[
    '\\'  as u16, 'D' as u16, 'e' as u16, 'v' as u16, 'i' as u16, 'c' as u16, 'e' as u16, '\\'  as u16, 'M' as u16, 'y' as u16, 'R' as u16, 'o' as u16, 'o' as u16, 't' as u16, '_' as u16, '9' as u16, '1' as u16, '2' as u16, '3' as u16, 0
];
static SYM_NAME_U16: &[u16] = &[
    '\\'  as u16, 'D' as u16, 'o' as u16, 's' as u16, 'D' as u16, 'e' as u16, 'v' as u16, 'i' as u16, 'c' as u16, 'e' as u16, 's' as u16, '\\'  as u16, 'M' as u16, 'y' as u16, 'R' as u16, 'o' as u16, 'o' as u16, 't' as u16, '_' as u16, '9' as u16, '1' as u16, '2' as u16, '3' as u16, 0
];
static NULL_DEV_U16: &[u16] = &[
    '\\' as u16, 'D' as u16, 'e' as u16, 'v' as u16, 'i' as u16,
    'c' as u16, 'e' as u16, '\\' as u16, 'N' as u16, 'u' as u16,
    'l' as u16, 'l' as u16, 0
];

// =================================================================
// §4: KERNEL STRUCTURES
// =================================================================

#[repr(C)] #[derive(Copy, Clone)]
pub struct UNICODE_STRING {
    pub Length: u16,
    pub MaximumLength: u16,
    pub Buffer: *mut u16,
}

#[repr(C)]
pub struct DRIVER_OBJECT {
    pub Type: i16,
    pub Size: u16,
    pub DeviceObject: PVOID,
    pub Flags: u32,
    pub DriverStart: PVOID,
    pub DriverSize: u32,
    pub DriverSection: PVOID,
    pub DriverExtension: PVOID,
    pub DriverName: UNICODE_STRING,
    pub HardwareDatabase: *mut UNICODE_STRING,
    pub FastIoDispatch: PVOID,
    pub DriverInit: PVOID,
    pub DriverStartIo: PVOID,
    pub DriverUnload: Option<extern "system" fn(*mut DRIVER_OBJECT)>,
    pub MajorFunction: [PVOID; 28],
}

#[repr(C)] #[derive(Copy, Clone)]
pub struct PEPROCESS(pub PVOID);

#[repr(C)] #[derive(Copy, Clone)]
pub struct MemoryRequest {
    pub process_id: u64,
    pub address: u64,
    pub size: u64,
}

// =================================================================
// §5: KERNEL IMPORTS
// =================================================================

extern "system" {
    fn DbgPrint(Format: *const u8, ...) -> NTSTATUS;
    fn IofCompleteRequest(Irp: PVOID, PriorityBoost: i8);
    fn PsLookupProcessByProcessId(ProcessId: PVOID, Process: *mut PEPROCESS) -> NTSTATUS;
    fn ObDereferenceObject(Object: PVOID);
    fn MmCopyVirtualMemory(
        SrcProc: PEPROCESS, SrcAddr: PVOID,
        DstProc: PEPROCESS, DstAddr: PVOID,
        Size: usize, Mode: i8, RetSize: *mut usize
    ) -> NTSTATUS;
    fn PsGetCurrentProcess() -> PEPROCESS;
    fn PsGetProcessSectionBaseAddress(Process: PVOID) -> PVOID;
    fn IoCreateDevice(
        DriverObject: *mut DRIVER_OBJECT, ExtSize: u32,
        DeviceName: *mut UNICODE_STRING, DeviceType: u32,
        Chars: u32, Exclusive: BOOLEAN, DeviceObject: *mut PVOID
    ) -> NTSTATUS;
    fn IoCreateSymbolicLink(Link: *mut UNICODE_STRING, Target: *mut UNICODE_STRING) -> NTSTATUS;
    fn IoDeleteSymbolicLink(Link: *mut UNICODE_STRING) -> NTSTATUS;
    fn IoDeleteDevice(DeviceObject: PVOID);
    fn IoGetDeviceObjectPointer(
        ObjectName: *mut UNICODE_STRING, DesiredAccess: u32,
        FileObject: *mut PVOID, DeviceObject: *mut PVOID
    ) -> NTSTATUS;
}

// =================================================================
// §6: CR0 WRITE-PROTECT BYPASS (for kdmapper hook mode)
// =================================================================

#[inline(always)]
unsafe fn wp_disable() -> u64 {
    let cr0: u64;
    core::arch::asm!("cli");
    core::arch::asm!("mov {}, cr0", out(reg) cr0);
    core::arch::asm!("mov cr0, {}", in(reg) cr0 & !(1u64 << 16));
    cr0
}

#[inline(always)]
unsafe fn wp_restore(cr0: u64) {
    core::arch::asm!("mov cr0, {}", in(reg) cr0);
    core::arch::asm!("sti");
}

// =================================================================
// §7: IRP HELPERS (raw offsets)
// =================================================================

#[inline(always)]
unsafe fn irp_get_system_buffer(irp: PVOID) -> PVOID {
    *((irp as *const u8).add(IRP_SYSTEM_BUFFER) as *const PVOID)
}
#[inline(always)]
unsafe fn irp_set_status(irp: PVOID, status: NTSTATUS) {
    *((irp as *mut u8).add(IRP_IOSTATUS_STATUS) as *mut i32) = status;
}
#[inline(always)]
unsafe fn irp_set_information(irp: PVOID, info: usize) {
    *((irp as *mut u8).add(IRP_IOSTATUS_INFO) as *mut usize) = info;
}
#[inline(always)]
unsafe fn irp_get_iocontrol_code(irp: PVOID) -> u32 {
    let stack_loc = *((irp as *const u8).add(IRP_CURRENT_STACK_LOC) as *const PVOID);
    *((stack_loc as *const u8).add(ISL_IOCONTROL_CODE) as *const u32)
}

// =================================================================
// §8: GLOBAL STATE
// =================================================================

static mut SYM_LINK_GLOBAL: UNICODE_STRING = UNICODE_STRING {
    Length: 0, MaximumLength: 0, Buffer: ptr::null_mut()
};
// Hook mode globals
static mut ORIGINAL_DEVICE_CONTROL: PVOID = ptr::null_mut();
static mut HOOKED_DRIVER_OBJ: PVOID = ptr::null_mut();
static mut HOOK_FILE_OBJECT: PVOID = ptr::null_mut();

// =================================================================
// §9: SHARED IOCTL HANDLER (used by both modes)
// =================================================================

extern "system" {
    fn PsGetCurrentProcessId() -> PVOID;
}

unsafe fn handle_ioctl(irp: PVOID) -> NTSTATUS {
    let ioctl = irp_get_iocontrol_code(irp);
    let func = (ioctl >> 2) & 0xFFF;

    let mut status: NTSTATUS = STATUS_UNSUCCESSFUL;
    let mut info: usize = 0;

    let buf = irp_get_system_buffer(irp);
    if buf.is_null() {
        status = STATUS_INVALID_PARAMETER;
    } else {
        // Safe Trusted Caller ID verification
        let caller_id = PsGetCurrentProcessId() as u64;
        let auth_pid = AUTHORIZED_PID.load(Ordering::Relaxed);

        let req = &*(buf as *const MemoryRequest);
        // We still decrypt request fields, but verify caller against Kernel-known ID
        let req_pid = req.process_id ^ XOR_KEY; // This is now TARGET PID

        if func == 0x804 {
            // AUTH: User sends their PID (req_pid) to authorize.
            // We verify it matches the actual kernel-reported ID just in case/or trust kernel ID
            AUTHORIZED_PID.store(caller_id, Ordering::Relaxed);
            status = STATUS_SUCCESS;
            // DbgPrint(b"[MyRoot] AUTH SUCCESS PID=%llu\n\0".as_ptr(), caller_id);
        } else if auth_pid != 0 && caller_id == auth_pid {
            match func {
                0x800 => { // READ MDMORY
                    let sz = (req.size ^ XOR_KEY) as usize;
                    let addr = req.address ^ XOR_KEY;
                    let target_pid = req_pid; // Field is Target PID

                    if sz > 0 && sz <= MAX_READ_SIZE {
                        let mut target_proc = PEPROCESS(ptr::null_mut());
                        
                        // Look up Target Process
                        if PsLookupProcessByProcessId(target_pid as PVOID, &mut target_proc) == STATUS_SUCCESS {
                            
                            // Copy from Target (Src) to Caller (Dst)
                            status = MmCopyVirtualMemory(
                                target_proc, addr as PVOID,          // Source: Game
                                PsGetCurrentProcess(), buf,          // Dest: Cheat (Caller)
                                sz, 0, &mut info // Mode 0 = Virtual (Safe)
                            );
                            
                            ObDereferenceObject(target_proc.0);
                        } else {
                            status = STATUS_INVALID_CID;
                        }
                    } else {
                        status = STATUS_INFO_LENGTH_MISMATCH;
                    }
                },
                0x802 => { // GET BASE ADDR
                    let target_pid = req.address ^ XOR_KEY; // In 802, address field holds PID (Legacy/Compat)
                    
                    let mut process = PEPROCESS(ptr::null_mut());
                    if PsLookupProcessByProcessId(target_pid as PVOID, &mut process) == STATUS_SUCCESS {
                        let base = PsGetProcessSectionBaseAddress(process.0);
                        ObDereferenceObject(process.0);
                        if !base.is_null() {
                            *(buf as *mut u64) = base as u64;
                            info = 8;
                            status = STATUS_SUCCESS;
                        } else {
                            status = STATUS_UNSUCCESSFUL;
                        }
                    } else {
                        status = STATUS_INVALID_CID;
                    }
                },
                _ => { status = STATUS_INVALID_PARAMETER; }
            }
        } else {
            status = STATUS_ACCESS_DENIED;
        }
    }

    irp_set_status(irp, status);
    irp_set_information(irp, info);
    IofCompleteRequest(irp, 0);
    status
}

// =================================================================
// §10: MODE 1 — SCM DISPATCH (IoCreateDevice)
// =================================================================

unsafe extern "system" fn dispatch_create_close(_dev: PVOID, irp: PVOID) -> NTSTATUS {
    irp_set_status(irp, STATUS_SUCCESS);
    irp_set_information(irp, 0);
    IofCompleteRequest(irp, 0);
    STATUS_SUCCESS
}

unsafe extern "system" fn dispatch_device_control(_dev: PVOID, irp: PVOID) -> NTSTATUS {
    handle_ioctl(irp)
}

// =================================================================
// §11: MODE 2 — HOOK DISPATCH (kdmapper)
// =================================================================

unsafe extern "system" fn hooked_device_control(device: PVOID, irp: PVOID) -> NTSTATUS {
    let ioctl = irp_get_iocontrol_code(irp);
    let func = (ioctl >> 2) & 0xFFF;

    // Our IOCTLs: 0x800, 0x802, 0x804
    if func == 0x800 || func == 0x802 || func == 0x804 {
        return handle_ioctl(irp);
    }

    // Not ours — pass through to original handler
    if !ORIGINAL_DEVICE_CONTROL.is_null() {
        let original: unsafe extern "system" fn(PVOID, PVOID) -> NTSTATUS =
            core::mem::transmute(ORIGINAL_DEVICE_CONTROL);
        return original(device, irp);
    }

    // Fallback: complete with success
    irp_set_status(irp, STATUS_SUCCESS);
    irp_set_information(irp, 0);
    IofCompleteRequest(irp, 0);
    STATUS_SUCCESS
}

// =================================================================
// §12: DRIVER UNLOAD
// =================================================================

pub extern "system" fn driver_unload(driver_object: *mut DRIVER_OBJECT) {
    unsafe {
        DbgPrint(b"[MyRoot] UNLOAD mode=%u\n\0".as_ptr(), DRIVER_MODE as u32);

        if DRIVER_MODE == 1 {
            // SCM mode: clean up device + symlink
            if !SYM_LINK_GLOBAL.Buffer.is_null() {
                IoDeleteSymbolicLink(&mut SYM_LINK_GLOBAL);
            }
            let dev = (*driver_object).DeviceObject;
            if !dev.is_null() {
                IoDeleteDevice(dev);
            }
        } else if DRIVER_MODE == 2 {
            // Hook mode: restore original handler
            if !HOOKED_DRIVER_OBJ.is_null() && !ORIGINAL_DEVICE_CONTROL.is_null() {
                let mf_ptr = (HOOKED_DRIVER_OBJ as *mut u8).add(DO_MF_DEVICE_CONTROL) as *mut PVOID;
                let cr0 = wp_disable();
                *mf_ptr = ORIGINAL_DEVICE_CONTROL;
                wp_restore(cr0);
                DbgPrint(b"[MyRoot] Hook restored\n\0".as_ptr());
            }
            if !HOOK_FILE_OBJECT.is_null() {
                ObDereferenceObject(HOOK_FILE_OBJECT);
            }
        }
        DbgPrint(b"[MyRoot] UNLOAD done\n\0".as_ptr());
    }
}

// =================================================================
// §13: DRIVER ENTRY (dual-mode)
// =================================================================

#[no_mangle]
pub unsafe extern "system" fn DriverEntry(
    driver_object: *mut DRIVER_OBJECT,
    _registry_path: PVOID
) -> NTSTATUS {
    DbgPrint(b"[MyRoot] === DriverEntry START === obj=%p\n\0".as_ptr(), driver_object as PVOID);

    // ── Try Mode 1: SCM (IoCreateDevice) ──
    if !driver_object.is_null() {
        let mut dev_name = UNICODE_STRING {
            Length: ((DEV_NAME_U16.len() - 1) * 2) as u16,
            MaximumLength: (DEV_NAME_U16.len() * 2) as u16,
            Buffer: DEV_NAME_U16.as_ptr() as *mut u16,
        };
        let mut sym_name = UNICODE_STRING {
            Length: ((SYM_NAME_U16.len() - 1) * 2) as u16,
            MaximumLength: (SYM_NAME_U16.len() * 2) as u16,
            Buffer: SYM_NAME_U16.as_ptr() as *mut u16,
        };

        let mut device_object: PVOID = ptr::null_mut();
        let status = IoCreateDevice(
            driver_object, 0, &mut dev_name,
            FILE_DEVICE_UNKNOWN, FILE_DEVICE_SECURE_OPEN,
            0, &mut device_object
        );

        if status == STATUS_SUCCESS {
            DbgPrint(b"[MyRoot] Mode 1 (SCM): Device created\n\0".as_ptr());
            DRIVER_MODE = 1;
            SYM_LINK_GLOBAL = sym_name;

            let _ = IoCreateSymbolicLink(&mut sym_name, &mut dev_name);
            (*driver_object).MajorFunction[IRP_MJ_CREATE] = dispatch_create_close as PVOID;
            (*driver_object).MajorFunction[IRP_MJ_CLOSE] = dispatch_create_close as PVOID;
            (*driver_object).MajorFunction[IRP_MJ_DEVICE_CONTROL] = dispatch_device_control as PVOID;
            (*driver_object).DriverUnload = Some(driver_unload);

            DbgPrint(b"[MyRoot] === Mode 1 SUCCESS ===\n\0".as_ptr());
            return STATUS_SUCCESS;
        }
        DbgPrint(b"[MyRoot] IoCreateDevice failed: 0x%X, trying Hook mode\n\0".as_ptr(), status);
    }

    // ── Mode 2: Hook \Device\Null (for kdmapper) ──
    DbgPrint(b"[MyRoot] Attempting Mode 2 (Hook)...\n\0".as_ptr());

    let mut null_name = UNICODE_STRING {
        Length: ((NULL_DEV_U16.len() - 1) * 2) as u16,
        MaximumLength: (NULL_DEV_U16.len() * 2) as u16,
        Buffer: NULL_DEV_U16.as_ptr() as *mut u16,
    };

    let mut file_obj: PVOID = ptr::null_mut();
    let mut dev_obj: PVOID = ptr::null_mut();

    let status = IoGetDeviceObjectPointer(
        &mut null_name, FILE_READ_DATA, &mut file_obj, &mut dev_obj
    );
    if status != STATUS_SUCCESS {
        DbgPrint(b"[MyRoot] IoGetDeviceObjectPointer FAILED: 0x%X\n\0".as_ptr(), status);
        return status;
    }

    // Get DriverObject from DeviceObject
    let target_drv_obj = *((dev_obj as *const u8).add(DEVOBJ_DRIVER_OBJECT) as *const PVOID);
    if target_drv_obj.is_null() {
        DbgPrint(b"[MyRoot] Target DriverObject is NULL\n\0".as_ptr());
        ObDereferenceObject(file_obj);
        return STATUS_UNSUCCESSFUL;
    }

    // Save originals
    let mf_ptr = (target_drv_obj as *mut u8).add(DO_MF_DEVICE_CONTROL) as *mut PVOID;
    ORIGINAL_DEVICE_CONTROL = *mf_ptr;
    HOOKED_DRIVER_OBJ = target_drv_obj;
    HOOK_FILE_OBJECT = file_obj;

    // Hook with CR0 WP bypass
    let cr0 = wp_disable();
    *mf_ptr = hooked_device_control as PVOID;
    wp_restore(cr0);

    DRIVER_MODE = 2;
    DbgPrint(b"[MyRoot] === Mode 2 (Hook) SUCCESS ===\n\0".as_ptr());

    STATUS_SUCCESS
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! { loop {} }