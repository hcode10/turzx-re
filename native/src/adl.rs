//! Capteurs GPU AMD via ADL (atiadlxx.dll, installée avec le pilote Radeon).

use std::ffi::c_void;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

const ADL_OK: i32 = 0;
pub const MAX_SENSORS: usize = 256;

// Indices ADL_PMLOG_SENSORS (adl_defines.h).
pub const PMLOG_TEMPERATURE_EDGE: usize = 8;
pub const PMLOG_FAN_RPM: usize = 14;
pub const PMLOG_INFO_ACTIVITY_GFX: usize = 19;
pub const PMLOG_ASIC_POWER: usize = 23;
pub const PMLOG_TEMPERATURE_HOTSPOT: usize = 27;

#[repr(C)]
#[derive(Clone, Copy)]
struct SensorData {
    supported: i32,
    value: i32,
}

#[repr(C)]
struct PmLogOutput {
    size: i32,
    sensors: [SensorData; MAX_SENSORS],
}

#[repr(C)]
struct AdapterInfo {
    size: i32,
    adapter_index: i32,
    udid: [u8; 256],
    bus_number: i32,
    device_number: i32,
    function_number: i32,
    vendor_id: i32,
    adapter_name: [u8; 256],
    display_name: [u8; 256],
    present: i32,
    exist: i32,
    driver_path: [u8; 256],
    driver_path_ext: [u8; 256],
    pnp_string: [u8; 256],
    os_display_index: i32,
}

type MallocCb = unsafe extern "system" fn(i32) -> *mut c_void;
type CreateFn = unsafe extern "C" fn(MallocCb, i32, *mut *mut c_void) -> i32;
type NumAdaptersFn = unsafe extern "C" fn(*mut c_void, *mut i32) -> i32;
type AdapterInfoFn = unsafe extern "C" fn(*mut c_void, *mut AdapterInfo, i32) -> i32;
type QueryFn = unsafe extern "C" fn(*mut c_void, i32, *mut PmLogOutput) -> i32;

extern "C" {
    fn malloc(n: usize) -> *mut c_void;
}

unsafe extern "system" fn adl_malloc(n: i32) -> *mut c_void {
    malloc(n.max(0) as usize)
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

pub struct Adl {
    ctx: *mut c_void,
    query: QueryFn,
    pub adapter: i32,
    pub name: String,
}

pub struct Adapter {
    pub index: i32,
    pub bus: i32,
    pub name: String,
}

impl Adl {
    /// Ouvre ADL et choisit le GPU dédié (celui qui répond à PMLog, iGPU en dernier recours).
    pub fn open() -> Option<Self> {
        unsafe {
            let lib = LoadLibraryA(b"atiadlxx.dll\0".as_ptr());
            if lib.is_null() {
                return None;
            }
            let sym = |n: &[u8]| GetProcAddress(lib, n.as_ptr());
            let create: CreateFn = std::mem::transmute(sym(b"ADL2_Main_Control_Create\0")?);
            let num: NumAdaptersFn = std::mem::transmute(sym(b"ADL2_Adapter_NumberOfAdapters_Get\0")?);
            let info: AdapterInfoFn = std::mem::transmute(sym(b"ADL2_Adapter_AdapterInfo_Get\0")?);
            let query: QueryFn = std::mem::transmute(sym(b"ADL2_New_QueryPMLogData_Get\0")?);

            let mut ctx = std::ptr::null_mut();
            if create(adl_malloc, 1, &mut ctx) != ADL_OK {
                return None;
            }
            let mut n = 0;
            if num(ctx, &mut n) != ADL_OK || n <= 0 {
                return None;
            }
            let mut infos: Vec<AdapterInfo> = (0..n).map(|_| std::mem::zeroed()).collect();
            for i in infos.iter_mut() {
                i.size = std::mem::size_of::<AdapterInfo>() as i32;
            }
            if info(ctx, infos.as_mut_ptr(), n * std::mem::size_of::<AdapterInfo>() as i32) != ADL_OK {
                return None;
            }
            let mut adapters: Vec<Adapter> = Vec::new();
            for i in &infos {
                if !adapters.iter().any(|a| a.bus == i.bus_number) {
                    adapters.push(Adapter { index: i.adapter_index, bus: i.bus_number, name: cstr(&i.adapter_name) });
                }
            }
            // Les iGPU s'appellent « AMD Radeon(TM) Graphics » : on les met en dernier.
            adapters.sort_by_key(|a| a.name.contains("(TM)"));
            let mut adl = Adl { ctx, query, adapter: -1, name: String::new() };
            for a in adapters {
                adl.adapter = a.index;
                if adl.sensors().is_some() {
                    adl.name = a.name;
                    return Some(adl);
                }
            }
            None
        }
    }

    /// (supporté, valeur) pour chaque capteur PMLog.
    pub fn sensors(&self) -> Option<Vec<(bool, i32)>> {
        unsafe {
            let mut out: Box<PmLogOutput> = Box::new(std::mem::zeroed());
            out.size = std::mem::size_of::<PmLogOutput>() as i32;
            if (self.query)(self.ctx, self.adapter, &mut *out) != ADL_OK {
                return None;
            }
            Some(out.sensors.iter().map(|s| (s.supported != 0, s.value)).collect())
        }
    }
}

// Le contexte ADL n'est utilisé que depuis le thread de rendu.
unsafe impl Send for Adl {}
