// Live sniffer: read-only polling scan for Astraea class images (CAFEBABE FADD1A9F).
// usage: sniffer.exe <pid|auto> [minutes]
#![allow(non_snake_case, non_camel_case_types)]

use std::collections::HashSet;
use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::time::{Duration, Instant};

type HANDLE = *mut c_void;
type BOOL = i32;
type DWORD = u32;

const PROCESS_VM_READ: DWORD = 0x0010;
const PROCESS_QUERY_INFORMATION: DWORD = 0x0400;
const MEM_COMMIT: DWORD = 0x1000;
const MEM_PRIVATE: DWORD = 0x20000;
const SE_PRIVILEGE_ENABLED: DWORD = 0x00000002;
const TOKEN_ADJUST_PRIVILEGES: DWORD = 0x0020;
const TOKEN_QUERY: DWORD = 0x0008;
const TH32CS_SNAPPROCESS: DWORD = 2;

const OUT_DIR: &str = r"d:\project\rustme\dump\images";
const LOG: &str = r"d:\project\rustme\dump\sniffer.log";
const MARKER: [u8; 8] = [0xCA, 0xFE, 0xBA, 0xBE, 0xFA, 0xDD, 0x1A, 0x9F];
const GRAB: usize = 1 << 20;

#[repr(C)]
#[derive(Copy, Clone)]
struct MBI {
    BaseAddress: *mut c_void,
    AllocationBase: *mut c_void,
    AllocationProtect: DWORD,
    RegionSize: usize,
    State: DWORD,
    Protect: DWORD,
    Type: DWORD,
}

#[repr(C)]
struct PROCESSENTRY32 {
    dwSize: u32,
    cntUsage: u32,
    th32ProcessID: DWORD,
    th32DefaultHeapID: usize,
    th32ModuleID: DWORD,
    cntThreads: u32,
    th32ParentProcessID: DWORD,
    pcPriClassBase: i32,
    dwFlags: DWORD,
    szExeFile: [u8; 260],
}

#[repr(C)]
struct LUID { LowPart: DWORD, HighPart: i32 }
#[repr(C)]
struct LUID_AND_ATTRIBUTES { Luid: LUID, Attributes: DWORD }
#[repr(C)]
struct TOKEN_PRIVILEGES { PrivilegeCount: DWORD, Privileges: [LUID_AND_ATTRIBUTES; 1] }

extern "system" {
    fn OpenProcess(flags: DWORD, inherit: BOOL, pid: DWORD) -> HANDLE;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetLastError() -> DWORD;
    fn VirtualQueryEx(h: HANDLE, addr: *mut c_void, mbi: *mut MBI, len: usize) -> usize;
    fn ReadProcessMemory(h: HANDLE, addr: *const c_void, buf: *mut c_void, len: usize, read: *mut usize) -> BOOL;
    fn CreateToolhelp32Snapshot(flags: DWORD, pid: DWORD) -> HANDLE;
    fn Process32First(h: HANDLE, pe: *mut PROCESSENTRY32) -> BOOL;
    fn Process32Next(h: HANDLE, pe: *mut PROCESSENTRY32) -> BOOL;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(h: HANDLE, access: DWORD, tok: *mut HANDLE) -> BOOL;
    fn LookupPrivilegeValueA(host: *const u8, name: *const u8, luid: *mut LUID) -> BOOL;
    fn AdjustTokenPrivileges(tok: HANDLE, disable: BOOL, tp: *const TOKEN_PRIVILEGES, len: DWORD, prev: *mut TOKEN_PRIVILEGES, ret: *mut DWORD) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
}

fn debug_priv() {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut tok) == 0 { return; }
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        if LookupPrivilegeValueA(std::ptr::null(), b"SeDebugPrivilege\0".as_ptr(), &mut luid) != 0 {
            let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
            tp.PrivilegeCount = 1;
            tp.Privileges[0].Luid = luid;
            tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            AdjustTokenPrivileges(tok, 0, &tp, std::mem::size_of::<TOKEN_PRIVILEGES>() as DWORD, std::ptr::null_mut(), std::ptr::null_mut());
        }
        CloseHandle(tok);
    }
}

/// Highest PID whose image name is exactly "rustme.exe" (the game; the launcher is RustMe.exe).
fn find_game() -> Option<DWORD> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == -1isize as HANDLE { return None; }
        let mut pe: PROCESSENTRY32 = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;
        if Process32First(snap, &mut pe) == 0 { CloseHandle(snap); return None; }
        let mut best: Option<DWORD> = None;
        loop {
            let name: Vec<u8> = pe.szExeFile.iter().take_while(|&&b| b != 0).cloned().collect();
            if name == b"rustme.exe" {
                match best { Some(p) if p >= pe.th32ProcessID => {}, _ => best = Some(pe.th32ProcessID) }
            }
            if Process32Next(snap, &mut pe) == 0 { break; }
        }
        CloseHandle(snap);
        best
    }
}

fn query(h: HANDLE, addr: usize, mbi: &mut MBI) -> bool {
    unsafe { VirtualQueryEx(h, addr as *mut c_void, mbi, std::mem::size_of::<MBI>()) == std::mem::size_of::<MBI>() }
}

/// Read len bytes, zero-filling unreadable pages. Returns bytes produced.
fn read_tolerant(h: HANDLE, addr: usize, len: usize, out: &mut Vec<u8>) -> usize {
    out.clear();
    out.reserve(len.min(1 << 22));
    let mut page = [0u8; 4096];
    let mut off = 0usize;
    while off < len {
        let a = addr + off;
        let align = a & 4095;
        let mut mbi: MBI = unsafe { std::mem::zeroed() };
        let ok = query(h, a - align, &mut mbi)
            && mbi.State == MEM_COMMIT
            && (mbi.Protect & 0x100) == 0
            && (mbi.Protect & (0x02 | 0x04 | 0x08 | 0x10 | 0x20 | 0x40 | 0x80)) != 0;
        let mut got = 0usize;
        if ok {
            let room = (mbi.BaseAddress as usize + mbi.RegionSize) - a;
            let want = std::cmp::min(std::cmp::min(room, 4096 - align), len - off);
            unsafe {
                let r = ReadProcessMemory(h, a as *const c_void, page.as_mut_ptr() as *mut c_void, want, &mut got);
                if r == 0 { got = 0; }
            }
        }
        if got == 0 {
            let fill = std::cmp::min(4096 - align, len - off);
            out.extend_from_slice(&[0u8; 4096][..fill]);
            off += fill;
        } else {
            out.extend_from_slice(&page[..got]);
            off += got;
        }
    }
    out.len()
}

fn fnv(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

fn private_rw_regions(h: HANDLE) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut addr = 0usize;
    unsafe {
        loop {
            let mut mbi: MBI = std::mem::zeroed();
            if !query(h, addr, &mut mbi) { break; }
            let base = mbi.BaseAddress as usize;
            let size = mbi.RegionSize;
            if size == 0 { break; }
            let next = base.saturating_add(size);
            if next <= addr { break; }
            if mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE && (mbi.Protect & (0x04 | 0x08 | 0x20 | 0x40) != 0) && size <= 512 << 20 {
                out.push((base, size));
            }
            addr = next;
            if addr >= 0x7FFFFFFFFFF0 { break; }
        }
    }
    out
}

fn scan(h: HANDLE, lw: &mut dyn FnMut(String), seen: &mut HashSet<u64>, saved: &mut usize, deadline: Duration) {
    let t0 = Instant::now();
    let mut chunk: Vec<u8> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut pass = 0usize;
    let mut empty = 0usize;
    let mut regions = private_rw_regions(h);
    lw(format!("[+] private-RW regions={} total={} MB", regions.len(), regions.iter().map(|r| r.1).sum::<usize>() / 1048576));
    let mut last_log = Instant::now();

    while t0.elapsed() < deadline {
        pass += 1;
        let pstart = Instant::now();
        let mut hits = 0usize;
        let mut new_hits = 0usize;
        let mut read_bytes = 0usize;
        for (base, size) in &regions {
            let mut off = 0usize;
            let step = 1usize << 20;
            while off < *size {
                let n = std::cmp::min(step, *size - off);
                let a = base + off;
                let got = read_tolerant(h, a, n, &mut chunk);
                read_bytes += got;
                let mut i = 0usize;
                while i + 8 <= got {
                    if chunk[i] != 0xCA { i += 1; continue; }
                    if &chunk[i..i + 8] != &MARKER { i += 1; continue; }
                    hits += 1;
                    let addr = a + i;
                    read_tolerant(h, addr, GRAB, &mut buf);
                    let key = fnv(&buf[..std::cmp::min(1024, buf.len())]);
                    if seen.insert(key) {
                        new_hits += 1;
                        *saved += 1;
                        let _ = fs::write(format!("{}\\{:X}_{:x}.bin", OUT_DIR, addr, key), &buf);
                    }
                    i += 8;
                }
                off += n;
            }
        }
        lw(format!("[pass {}] {:.1}s scanned={} MB hits={} new={} saved={} elapsed={:.0}s", pass, pstart.elapsed().as_secs_f64(), read_bytes / 1048576, hits, new_hits, saved, t0.elapsed().as_secs_f64()));
        if read_bytes == 0 {
            empty += 1;
            if empty >= 4 {
                lw("[!] no readable memory for 4 passes — target gone?".into());
                return;
            }
        } else {
            empty = 0;
        }
        if pass % 3 == 0 || pstart.elapsed() > Duration::from_secs(20) {
            regions = private_rw_regions(h);
        }
        let _ = last_log;
        std::thread::sleep(Duration::from_millis(120));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spec = args.get(1).cloned().unwrap_or_else(|| "auto".to_string());
    let minutes: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(45);
    let auto = spec.eq_ignore_ascii_case("auto");
    let fixed: DWORD = if auto { 0 } else { spec.parse().unwrap_or(0) };
    if !auto && fixed == 0 { eprintln!("usage: sniffer.exe <pid|auto> [minutes]"); std::process::exit(2); }

    fs::create_dir_all(OUT_DIR).ok();
    let logf = fs::OpenOptions::new().create(true).append(true).open(LOG).unwrap();
    let mut logf = logf;
    let mut lw = |s: String| { let _ = writeln!(logf, "{}", s); let _ = logf.flush(); };

    debug_priv();
    let deadline = Duration::from_secs(minutes * 60);
    let mut seen: HashSet<u64> = HashSet::new();
    let mut saved = 0usize;
    let t0 = Instant::now();

    loop {
        let pid = if auto { find_game() } else { Some(fixed) };
        let pid = match pid {
            Some(p) => p,
            None => {
                if !auto || t0.elapsed() > deadline { lw("[x] target not found".into()); break; }
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        unsafe {
            let h = OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, 0, pid);
            if h.is_null() {
                lw(format!("[!] OpenProcess pid={} err={} (retry)", pid, GetLastError()));
                if !auto { break; }
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            lw(format!("[+] attached pid={} (auto={})", pid, auto));
            let remain = deadline.saturating_sub(t0.elapsed());
            let dl = if auto { remain } else { deadline };
            if dl.is_zero() { CloseHandle(h); break; }
            scan(h, &mut lw, &mut seen, &mut saved, dl);
            CloseHandle(h);
            if !auto { break; }
            lw(format!("[..] process gone or window ended; saved={} total", saved));
            if t0.elapsed() > deadline { break; }
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    lw(format!("[+] exited saved={}", saved));
}
