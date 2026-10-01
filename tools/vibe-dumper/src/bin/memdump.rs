// Dump the *loaded* (unpacked) image of chosen modules from a running process. Read-only.
// usage: memdump.exe <pid|auto> [moduleName ...]     default modules: Astraea.dll jvm.dll
#![allow(non_snake_case, non_camel_case_types)]

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
const SE_PRIVILEGE_ENABLED: DWORD = 0x00000002;
const TOKEN_ADJUST_PRIVILEGES: DWORD = 0x0020;
const TOKEN_QUERY: DWORD = 0x0008;
const TH32CS_SNAPPROCESS: DWORD = 2;
const TH32CS_SNAPMODULE: DWORD = 8;
const TH32CS_SNAPMODULE32: DWORD = 0x10;

const OUT_DIR: &str = r"d:\project\rustme\dump\modules";
const LOG: &str = r"d:\project\rustme\dump\memdump.log";

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
struct MODULEENTRY32 {
    dwSize: u32,
    th32ModuleID: u32,
    th32ProcessID: DWORD,
    GlountcntUsage: u32,
    ProccntUsage: u32,
    modBaseAddr: *mut u8,
    modBaseSize: u32,
    hModule: HANDLE,
    szModule: [u8; 256],
    szExePath: [u8; 260],
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

fn cstr(b: &[u8]) -> Vec<u8> {
    b.iter().take_while(|&&x| x != 0).cloned().collect()
}

fn find_game() -> Option<DWORD> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == -1isize as HANDLE { return None; }
        let mut pe: PROCESSENTRY32 = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;
        if Process32First(snap, &mut pe) == 0 { CloseHandle(snap); return None; }
        let mut best: Option<DWORD> = None;
        loop {
            if cstr(&pe.szExeFile) == b"rustme.exe" {
                match best { Some(p) if p >= pe.th32ProcessID => {} _ => best = Some(pe.th32ProcessID) }
            }
            if Process32Next(snap, &mut pe) == 0 { break; }
        }
        CloseHandle(snap);
        best
    }
}

fn modules(pid: DWORD) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snap == -1isize as HANDLE { return out; }
        let mut me: MODULEENTRY32 = std::mem::zeroed();
        me.dwSize = std::mem::size_of::<MODULEENTRY32>() as u32;
        if Module32First(snap, &mut me) == 0 { CloseHandle(snap); return out; }
        loop {
            let name = String::from_utf8_lossy(&cstr(&me.szModule)).to_string();
            out.push((name, me.modBaseAddr as usize, me.modBaseSize as usize));
            if Module32Next(snap, &mut me) == 0 { break; }
        }
        CloseHandle(snap);
    }
    out
}

fn read_tolerant(h: HANDLE, addr: usize, len: usize, out: &mut Vec<u8>) -> usize {
    out.clear();
    out.resize(len, 0);
    let mut done = 0usize;
    while done < len {
        let a = addr + done;
        let mut mbi: MBI = unsafe { std::mem::zeroed() };
        let ok = unsafe { VirtualQueryEx(h, a as *mut c_void, &mut mbi, std::mem::size_of::<MBI>()) } == std::mem::size_of::<MBI>()
            && mbi.State == MEM_COMMIT
            && (mbi.Protect & (0x02 | 0x04 | 0x08 | 0x10 | 0x20 | 0x40 | 0x80)) != 0;
        if !ok {
            done += 4096;
            continue;
        }
        let room = (mbi.BaseAddress as usize + mbi.RegionSize) - a;
        let want = std::cmp::min(std::cmp::min(room, len - done), 1 << 20);
        let mut got = 0usize;
        let r = unsafe { ReadProcessMemory(h, a as *const c_void, out.as_mut_ptr().add(done) as *mut c_void, want, &mut got) };
        if r == 0 || got == 0 {
            done += std::cmp::max(want, 4096);
        } else {
            done += got;
        }
    }
    done
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spec = args.get(1).cloned().unwrap_or_else(|| "auto".to_string());
    let want: Vec<String> = if args.len() > 2 { args[2..].to_vec() } else { vec!["Astraea.dll".into(), "jvm.dll".into()] };
    let auto = spec.eq_ignore_ascii_case("auto");
    let fixed: DWORD = if auto { 0 } else { spec.parse().unwrap_or(0) };

    fs::create_dir_all(OUT_DIR).ok();
    let mut logf = fs::OpenOptions::new().create(true).append(true).open(LOG).unwrap();
    let mut lw = |s: String| { let _ = writeln!(logf, "{}", s); let _ = logf.flush(); };

    debug_priv();
    let t0 = Instant::now();
    let mut first = true;
    loop {
        let pid = match if auto { find_game() } else { Some(fixed) } {
            Some(p) => p,
            None => {
                if first {
                    lw("[..] waiting for a process named rustme.exe".to_string());
                    // enumerate what IS there, so name mismatches are visible
                    unsafe {
                        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
                        if snap != -1isize as HANDLE {
                            let mut pe: PROCESSENTRY32 = std::mem::zeroed();
                            pe.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;
                            if Process32First(snap, &mut pe) != 0 {
                                let mut names = Vec::new();
                                loop {
                                    let n = String::from_utf8_lossy(&cstr(&pe.szExeFile)).to_string();
                                    if n.to_lowercase().contains("rust") || n.to_lowercase().contains("java") {
                                        names.push(format!("{}:{}", pe.th32ProcessID, n));
                                    }
                                    if Process32Next(snap, &mut pe) == 0 { break; }
                                }
                                lw(format!("    rust*/java* processes seen: {}", names.join(", ")));
                            }
                            CloseHandle(snap);
                        }
                    }
                    first = false;
                }
                if !auto { lw("[x] target not found".into()); break; }
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
        };
        unsafe {
            let h = OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, 0, pid);
            if h.is_null() {
                lw(format!("[x] OpenProcess pid={} err={}", pid, GetLastError()));
                if !auto { break; }
                std::thread::sleep(Duration::from_millis(300));
                continue;
            }
            let mods = modules(pid);
            lw(format!("[+] pid={} modules={}", pid, mods.len()));
            let mut buf: Vec<u8> = Vec::new();
            let mut dumped = 0usize;
            for (name, base, size) in &mods {
                let hit = want.iter().any(|w| w.eq_ignore_ascii_case(name));
                if !hit { continue; }
                dumped += 1;
                let mut mbi: MBI = std::mem::zeroed();
                let q = VirtualQueryEx(h, *base as *mut c_void, &mut mbi, std::mem::size_of::<MBI>());
                lw(format!("    {} base=0x{:X} size={} mapped={} protect=0x{:X}", name, base, size, q != 0, mbi.Protect));
                read_tolerant(h, *base, *size, &mut buf);
                let path = format!("{}\\{}_0x{:X}_mem.bin", OUT_DIR, name, base);
                match fs::write(&path, &buf) {
                    Ok(_) => lw(format!("    [+] wrote {} ({} bytes)", path, buf.len())),
                    Err(e) => lw(format!("    [x] write failed: {}", e)),
                }
            }
            CloseHandle(h);
            if dumped > 0 {
                lw(format!("[+] dumped {} module image(s), done", dumped));
                break;
            }
            if !auto { break; }
            if t0.elapsed() > Duration::from_secs(1800) { break; }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    lw("[+] memdump exited".into());
}
