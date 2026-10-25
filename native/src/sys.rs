//! Mesures système via Win32 (pas de WMI, pas de pilote).

use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

fn ft(t: FILETIME) -> u64 {
    (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64
}

pub struct CpuSampler {
    prev: (u64, u64),
}

impl CpuSampler {
    pub fn new() -> Self {
        let mut s = CpuSampler { prev: (0, 0) };
        s.prev = Self::read();
        s
    }

    /// (idle, total) ; le temps noyau inclut déjà le temps idle.
    fn read() -> (u64, u64) {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut idle, mut kernel, mut user) = (z, z, z);
        unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
        (ft(idle), ft(kernel) + ft(user))
    }

    /// Charge CPU globale en % depuis l'appel précédent.
    pub fn sample(&mut self) -> f32 {
        let cur = Self::read();
        let (di, dt) = (cur.0 - self.prev.0, cur.1 - self.prev.1);
        self.prev = cur;
        if dt == 0 {
            return 0.0;
        }
        100.0 * (1.0 - di as f32 / dt as f32)
    }
}

pub struct Memory {
    pub load: u32,
    pub total: u64,
    pub avail: u64,
}

pub fn memory() -> Memory {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    unsafe { GlobalMemoryStatusEx(&mut m) };
    Memory { load: m.dwMemoryLoad, total: m.ullTotalPhys, avail: m.ullAvailPhys }
}

/// (octets libres, octets totaux)
pub fn disk(root: &str) -> Option<(u64, u64)> {
    let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0 && total > 0).then_some((avail, total))
}

/// Débit réseau des cartes physiques (ni loopback, ni vEthernet / Tailscale / filtres).
pub struct NetSampler {
    prev: (u64, u64),
    at: std::time::Instant,
}

impl NetSampler {
    pub fn new() -> Self {
        NetSampler { prev: Self::read(), at: std::time::Instant::now() }
    }

    fn read() -> (u64, u64) {
        use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
        let (mut rx, mut tx) = (0u64, 0u64);
        unsafe {
            let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut t) != 0 || t.is_null() {
                return (0, 0);
            }
            let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
            for r in rows {
                let f = r.InterfaceAndOperStatusFlags._bitfield;
                let hardware = f & 0b001 != 0;
                let filter = f & 0b010 != 0;
                let connector = f & 0b100 != 0;
                if r.OperStatus == 1 && hardware && !filter && connector {
                    rx += r.InOctets;
                    tx += r.OutOctets;
                }
            }
            FreeMibTable(t as *const _);
        }
        (rx, tx)
    }

    /// (alias, drapeaux, état, octets reçus) de chaque interface, pour diagnostic.
    pub fn interfaces() -> Vec<(String, u8, i32, u64)> {
        use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
        let mut out = Vec::new();
        unsafe {
            let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut t) != 0 || t.is_null() {
                return out;
            }
            let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
            for r in rows {
                let end = r.Alias.iter().position(|&c| c == 0).unwrap_or(r.Alias.len());
                let alias = String::from_utf16_lossy(&r.Alias[..end]);
                out.push((alias, r.InterfaceAndOperStatusFlags._bitfield, r.OperStatus, r.InOctets));
            }
            FreeMibTable(t as *const _);
        }
        out
    }

    /// (réception, émission) en octets/s depuis l'appel précédent.
    pub fn sample(&mut self) -> (f64, f64) {
        let cur = Self::read();
        let dt = self.at.elapsed().as_secs_f64().max(1e-3);
        let rate = (
            cur.0.saturating_sub(self.prev.0) as f64 / dt,
            cur.1.saturating_sub(self.prev.1) as f64 / dt,
        );
        self.prev = cur;
        self.at = std::time::Instant::now();
        rate
    }
}

pub struct Time {
    pub hour: u16,
    pub min: u16,
    pub sec: u16,
    pub ms: u16,
    pub day: u16,
    pub month: u16,
    pub weekday: u16,
}

pub fn local_time() -> Time {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    Time {
        hour: t.wHour,
        min: t.wMinute,
        sec: t.wSecond,
        ms: t.wMilliseconds,
        day: t.wDay,
        month: t.wMonth,
        weekday: t.wDayOfWeek,
    }
}
