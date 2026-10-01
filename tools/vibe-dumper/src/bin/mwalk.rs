// Phase 1 of the metaspace walker: locate HotSpot object vtable clusters externally.
// usage: mwalk.exe <pid> [jvm_dll_path]
#![allow(non_snake_case, non_camel_case_types)]

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::fs;
use std::io::Write;

type HANDLE = *mut c_void;
type BOOL = i32;
type DWORD = u32;

const PROCESS_VM_READ: DWORD = 0x0010;
const PROCESS_QUERY_INFORMATION: DWORD = 0x0400;
const MEM_COMMIT: DWORD = 0x1000;
const SE_PRIVILEGE_ENABLED: DWORD = 0x00000002;
const TOKEN_ADJUST_PRIVILEGES: DWORD = 0x0020;
const TOKEN_QUERY: DWORD = 0x0008;

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
#[derive(Copy, Clone)]
struct MODULEENTRY32 {
    dwSize: u32,
    th32ModuleID: u32,
    th32ProcessID: u32,
    GlountcntUsage: u32,
    ProccntUsage: u32,
    modBaseAddr: *mut u8,
    modBaseSize: u32,
    hModule: HANDLE,
    szModule: [u8; 256],
    szExePath: [u8; 260],
}

#[repr(C)]
struct LUID {
    LowPart: DWORD,
    HighPart: i32,
}
#[repr(C)]
struct LUID_AND_ATTRIBUTES {
    Luid: LUID,
    Attributes: DWORD,
}
#[repr(C)]
struct TOKEN_PRIVILEGES {
    PrivilegeCount: DWORD,
    Privileges: [LUID_AND_ATTRIBUTES; 1],
}

extern "system" {
    fn OpenProcess(flags: DWORD, inherit: BOOL, pid: DWORD) -> HANDLE;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetLastError() -> DWORD;
    fn VirtualQueryEx(h: HANDLE, addr: *mut c_void, mbi: *mut MBI, len: usize) -> usize;
    fn ReadProcessMemory(h: HANDLE, addr: *const c_void, buf: *mut c_void, len: usize, read: *mut usize) -> BOOL;
    fn CreateToolhelp32Snapshot(flags: DWORD, pid: DWORD) -> HANDLE;
    fn Module32First(h: HANDLE, me: *mut MODULEENTRY32) -> BOOL;
    fn Module32Next(h: HANDLE, me: *mut MODULEENTRY32) -> BOOL;
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
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut tok) == 0 {
            return;
        }
        let name = b"SeDebugPrivilege\0";
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        if LookupPrivilegeValueA(std::ptr::null(), name.as_ptr(), &mut luid) != 0 {
            let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
            tp.PrivilegeCount = 1;
            tp.Privileges[0].Luid = luid;
            tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            AdjustTokenPrivileges(tok, 0, &tp, std::mem::size_of::<TOKEN_PRIVILEGES>() as DWORD, std::ptr::null_mut(), std::ptr::null_mut());
        }
        CloseHandle(tok);
    }
}

fn remote_modules(h: HANDLE, pid: u32) -> Vec<(String, usize, usize)> {
    unsafe {
        let mut out = Vec::new();
        let snap = CreateToolhelp32Snapshot(2, pid); // TH32CS_SNAPMODULE
        if snap == -1isize as HANDLE {
            return out;
        }
        let mut me: MODULEENTRY32 = std::mem::zeroed();
        me.dwSize = std::mem::size_of::<MODULEENTRY32>() as u32;
        if Module32First(snap, &mut me) == 0 {
            CloseHandle(snap);
            return out;
        }
        loop {
            let nb: Vec<u8> = me.szModule.iter().take_while(|&&b| b != 0).cloned().collect();
            let name = String::from_utf8_lossy(&nb).to_string();
            out.push((name, me.modBaseAddr as usize, me.modBaseSize as usize));
            if Module32Next(snap, &mut me) == 0 {
                break;
            }
        }
        CloseHandle(snap);
        out
    }
}

fn readable(h: HANDLE, addr: usize, mbi: &mut MBI) -> bool {
    unsafe {
        if VirtualQueryEx(h, addr as *mut c_void, mbi, std::mem::size_of::<MBI>()) == 0 {
            return false;
        }
        mbi.State == MEM_COMMIT && (mbi.Protect & 0x100) == 0 && (mbi.Protect & (0x02 | 0x04 | 0x08 | 0x10 | 0x20 | 0x40 | 0x80)) != 0
    }
}

fn read_exact(h: HANDLE, addr: usize, len: usize, out: &mut Vec<u8>) -> bool {
    out.clear();
    out.resize(len, 0);
    let mut done = 0usize;
    while done < len {
        let page = 4096;
        let n = std::cmp::min(page, len - done);
        let mut got = 0usize;
        let ok = unsafe { ReadProcessMemory(h, (addr + done) as *const c_void, out.as_mut_ptr().add(done) as *mut c_void, n, &mut got) };
        if ok == 0 || got == 0 {
            return false;
        }
        done += got;
    }
    true
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pid: DWORD = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    if pid == 0 {
        eprintln!("usage: mwalk.exe <pid>");
        std::process::exit(2);
    }
    debug_priv();

    let mut rep = fs::File::create(r"d:\project\rustme\dump\mwalk_report.txt").unwrap();
    let mut log = |s: &str| {
        let _ = writeln!(rep, "{}", s);
        rep.flush().ok();
    };

    unsafe {
        let h = OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, 0, pid);
        if h.is_null() {
            log(&format!("[x] OpenProcess err={}", GetLastError()));
            return;
        }

        let mods = remote_modules(h, pid);
        let mut jvm: Option<(String, usize, usize)> = None;
        for (n, b, s) in &mods {
            if n.eq_ignore_ascii_case("jvm.dll") {
                jvm = Some((n.clone(), *b, *s));
            }
        }
        log(&format!("modules={} jvm={:?}", mods.len(), jvm.as_ref().map(|x| (x.0.clone(), format!("0x{:X}", x.1), x.2))));
        let (jb, js) = match jvm {
            Some((_, b, s)) => (b, s),
            None => {
                log("[x] jvm.dll not visible");
                return;
            }
        };

        // Region inventory
        let mut addr = 0usize;
        let mut regions: Vec<(usize, usize, DWORD, DWORD)> = Vec::new();
        loop {
            let mut mbi: MBI = std::mem::zeroed();
            if !readable(h, addr, &mut mbi) {
                if mbi.RegionSize == 0 {
                    break;
                }
                addr += mbi.RegionSize;
                continue;
            }
            if addr + mbi.RegionSize < addr {
                break;
            }
            regions.push((addr, mbi.RegionSize, mbi.Protect, mbi.Type));
            addr += mbi.RegionSize;
            if addr >= 0x7FFFFFFF0000 {
                break;
            }
        }
        let total: usize = regions.iter().map(|r| r.1).sum();
        log(&format!("readable regions={} total={} MB", regions.len(), total / 1024 / 1024));

        // vtable histogram: 8-aligned qwords pointing inside jvm.dll
        let mut hist: HashMap<u64, u64> = HashMap::new();
        let mut buf: Vec<u8> = Vec::new();
        let mut samples: HashMap<u64, Vec<usize>> = HashMap::new();
        let chunk = 1 << 20;
        let mut scanned = 0usize;
        for (base, size, _p, _t) in &regions {
            let mut off = 0usize;
            while off < *size {
                let n = std::cmp::min(chunk, size - off);
                if !read_exact(h, base + off, n, &mut buf) {
                    off += n;
                    continue;
                }
                scanned += n;
                for i in (0..n - 8).step_by(8) {
                    let v = u64::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]]);
                    if v >= jb as u64 && v < (jb + js) as u64 {
                        *hist.entry(v).or_insert(0) += 1;
                        if hist.len() > 400_000 {
                            break;
                        }
                        let e = samples.entry(v).or_insert_with(Vec::new);
                        if e.len() < 6 {
                            e.push(base + off + i);
                        }
                    }
                }
                off += n;
            }
        }
        log(&format!("scanned {} MB, distinct in-image qwords={}", scanned / 1048576, hist.len()));

        let mut top: Vec<(u64, u64)> = hist.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        log("--- top qvalues (vtable candidates) ---");
        for (v, c) in top.iter().take(30) {
            log(&format!("  0x{:016X} rva=0x{:X} count={} objs_at={:?}", v, v - jb as u64, c, samples.get(v).map(|s| s.iter().take(3).map(|a| format!("{:X}", a)).collect::<Vec<_>>()).unwrap_or_default()));
        }
        log("done");
    }
}
