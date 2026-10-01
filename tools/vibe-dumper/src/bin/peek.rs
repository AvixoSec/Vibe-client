// Absolute-address memory reader for external analysis.
// usage: peek.exe <pid> <hex_addr> <len> [count]
//   writes d:\project\rustme\dump\peek\<addr>_<n>.bin and a hex dump to peek_out.txt
#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;
use std::fs;
use std::io::Write;

type HANDLE = *mut c_void;
type BOOL = i32;
type DWORD = u32;
type ULONG_PTR = usize;

const PROCESS_VM_READ: DWORD = 0x0010;
const PROCESS_QUERY_INFORMATION: DWORD = 0x0400;
const SE_PRIVILEGE_ENABLED: DWORD = 0x00000002;
const TOKEN_ADJUST_PRIVILEGES: DWORD = 0x0020;
const TOKEN_QUERY: DWORD = 0x0008;
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const MEM_COMMIT: DWORD = 0x1000;
const PAGE_NOACCESS: DWORD = 0x01;

#[repr(C)]
#[derive(Copy, Clone)]
struct MEMORY_BASIC_INFORMATION {
    BaseAddress: *mut c_void,
    AllocationBase: *mut c_void,
    AllocationProtect: DWORD,
    RegionSize: usize,
    State: DWORD,
    Protect: DWORD,
    Type: DWORD,
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
    fn VirtualQueryEx(h: HANDLE, addr: *mut c_void, mbi: *mut MEMORY_BASIC_INFORMATION, len: usize) -> usize;
    fn ReadProcessMemory(h: HANDLE, addr: *const c_void, buf: *mut c_void, len: usize, read: *mut usize) -> BOOL;
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

fn readable(h: HANDLE, addr: usize) -> bool {
    unsafe {
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        if VirtualQueryEx(h, addr as *mut c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
            return false;
        }
        mbi.State == MEM_COMMIT
            && (mbi.Protect & PAGE_NOACCESS) == 0
            && (mbi.Protect & 0x100) == 0 // guard pages
            && (mbi.Protect & (0x02 | 0x04 | 0x08 | 0x10 | 0x20 | 0x40 | 0x80)) != 0
    }
}

fn read_tolerant(h: HANDLE, addr: usize, len: usize, out: &mut Vec<u8>) -> usize {
    out.clear();
    out.reserve(len);
    let mut page = [0u8; 4096];
    let mut off = 0usize;
    while off < len {
        let a = addr + off;
        let align = a & 4095;
        let start = a - align;
        if !readable(h, start) {
            // hole: record 4096-align bytes of zeros and continue past this page
            let fill = std::cmp::min(4096 - align, len - off);
            out.extend_from_slice(&[0u8; 4096][..fill]);
            off += fill;
            continue;
        }
        let mut got = 0usize;
        let ok = unsafe { ReadProcessMemory(h, start as *const c_void, page.as_mut_ptr() as *mut c_void, 4096, &mut got) };
        if ok == 0 || got == 0 {
            let fill = std::cmp::min(4096 - align, len - off);
            out.extend_from_slice(&[0u8; 4096][..fill]);
            off += fill;
            continue;
        }
        let avail = got - align;
        let take = std::cmp::min(avail, len - off);
        out.extend_from_slice(&page[align..align + take]);
        off += take;
    }
    off
}

fn hexdump(buf: &[u8], base: usize, w: &mut dyn Write, limit: usize) {
    let n = std::cmp::min(buf.len(), limit);
    for (i, chunk) in buf[..n].chunks(16).enumerate() {
        let mut hex = String::new();
        let mut asc = String::new();
        for &b in chunk {
            hex.push_str(&format!("{:02X} ", b));
            asc.push(if (0x20..0x7F).contains(&b) { b as char } else { '.' });
        }
        let _ = writeln!(w, "{:016X}  {:<48} |{}", base + i * 16, hex, asc);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: peek.exe <pid> <hex_addr> <len> [count]");
        std::process::exit(2);
    }
    let pid: DWORD = args[1].parse().unwrap();
    let addr: usize = usize::from_str_radix(args[2].trim_start_matches("0x").trim_start_matches("0X"), 16).unwrap();
    let len: usize = args[3].parse().unwrap();
    let count: usize = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(1);

    debug_priv();
    unsafe {
        let h = OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, 0, pid);
        if h.is_null() {
            eprintln!("[x] OpenProcess pid={} err={}", pid, GetLastError());
            std::process::exit(1);
        }

        let dir = r"d:\project\rustme\dump\peek";
        fs::create_dir_all(dir).ok();
        let mut rep = fs::File::create(format!("{}\\peek_out.txt", dir)).unwrap();
        let _ = writeln!(rep, "=== peek pid={} addr=0x{:X} len={} count={} ===", pid, addr, len, count);

        for i in 0..count {
            let a = addr + i * len;
            let mut buf = Vec::new();
            let got = read_tolerant(h, a, len, &mut buf);
            let path = format!("{}\\{:X}_{:03}.bin", dir, a, i);
            let _ = fs::write(&path, &buf);
            let _ = writeln!(rep, "\n--- [{}] 0x{:X} ({} bytes) -> {}", i, a, got, path);
            hexdump(&buf, a, &mut rep, 512);
        }
        CloseHandle(h);
        println!("done -> {}\\peek_out.txt", dir);
    }
}
