#![allow(non_snake_case, non_camel_case_types)]

//! External read-only JNI-table reconnaissance.
//!
//! Unlike the in-process dumper, this tool NEVER executes code inside the
//! target and NEVER calls through a JNIEnv vtable. It opens the target with
//! PROCESS_QUERY_INFORMATION | PROCESS_VM_READ and:
//!   1. locates jvm.dll (Toolhelp, falling back to a remote PE export scan),
//!   2. pattern-scans the jvm.dll image for the global JNIEnv function table,
//!   3. dumps all 233 entries with module attribution + first bytes of every
//!      unique target (inline-hook / heap-stub detection),
//!   4. saves the raw table for offline diffing between processes.
//!
//! Usage: probe.exe <pid>            (run elevated; target can be the launcher
//!                                    or the game — reads only, cannot crash it)

use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::io::Write;
use std::mem;
use std::ptr;

const REPORT_DIR: &str = r"d:\project\rustme\dump";

#[repr(C)]
struct MODULEENTRY32A {
    dwSize: u32,
    th32ModuleID: u32,
    th32ProcessID: u32,
    GlblcntUsage: u32,
    ProccntUsage: u32,
    modBaseAddr: *mut u8,
    modBaseSize: u32,
    hModule: isize,
    szModule: [u8; 256],
    szExePath: [u8; 260],
}

#[repr(C)]
struct MEMORY_BASIC_INFORMATION {
    BaseAddress: *mut c_void,
    AllocationBase: *mut c_void,
    AllocationProtect: u32,
    PartitionId: u16,
    RegionSize: usize,
    State: u32,
    Protect: u32,
    Type: u32,
}

extern "system" {
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> *mut c_void;
    fn ReadProcessMemory(hProcess: *mut c_void, lpBaseAddress: *const c_void, lpBuffer: *mut c_void, nSize: usize, lpNumberOfBytesRead: *mut usize) -> i32;
    fn VirtualQueryEx(hProcess: *mut c_void, lpAddress: *const c_void, lpBuffer: *mut MEMORY_BASIC_INFORMATION, dwLength: usize) -> usize;
    fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize;
    fn Module32First(hSnapshot: isize, lpme: *mut MODULEENTRY32A) -> i32;
    fn Module32Next(hSnapshot: isize, lpme: *mut MODULEENTRY32A) -> i32;
    fn CloseHandle(hObject: isize) -> i32;
    fn GetLastError() -> u32;
    fn GetCurrentProcess() -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(h: *mut c_void, acc: u32, tok: *mut *mut c_void) -> i32;
    fn LookupPrivilegeValueA(host: *const u8, name: *const u8, luid: *mut u64) -> i32;
    fn AdjustTokenPrivileges(tok: *mut c_void, disall: i32, tp: *const u8, len: u32, prev: *mut c_void, plen: *mut u32) -> i32;
}

const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_VM_READ: u32 = 0x0010;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_QUERY: u32 = 0x0008;
const SE_PRIVILEGE_ENABLED: u32 = 0x2;

fn enable_debug_privilege() {
    unsafe {
        let mut token: *mut c_void = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 { return; }
        let mut luid: u64 = 0;
        let name = CString::new("SeDebugPrivilege").unwrap();
        if LookupPrivilegeValueA(ptr::null(), name.as_ptr() as *const u8, &mut luid) != 0 {
            #[repr(C)]
            struct TokPriv { count: u32, luid: u64, attrs: u32 }
            let tp = TokPriv { count: 1, luid, attrs: SE_PRIVILEGE_ENABLED };
            AdjustTokenPrivileges(token, 0, &tp as *const _ as *const u8, mem::size_of::<TokPriv>() as u32, ptr::null_mut(), ptr::null_mut());
        }
        CloseHandle(token as isize);
    }
}

struct Proc { h: *mut c_void }

impl Proc {
    fn open(pid: u32) -> Option<Proc> {
        let h = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if h.is_null() { None } else { Some(Proc { h }) }
    }

    fn read(&self, addr: usize, buf: &mut [u8]) -> bool {
        unsafe {
            let mut got: usize = 0;
            ReadProcessMemory(self.h, addr as *const c_void, buf.as_mut_ptr() as *mut c_void, buf.len(), &mut got) != 0 && got == buf.len()
        }
    }

    /// Page-tolerant read: fills unreadable 4 KiB pages with zeros. RPM on a
    /// protected process can fail per-page; a single bad page must not drop
    /// a whole multi-megabyte window.
    fn read_tolerant(&self, addr: usize, buf: &mut [u8]) {
        const PAGE: usize = 4096;
        let mut off = 0usize;
        while off < buf.len() {
            let want = std::cmp::min(PAGE, buf.len() - off);
            if !self.read(addr + off, &mut buf[off..off + want]) {
                for b in &mut buf[off..off + want] { *b = 0; }
            }
            off += want;
        }
    }

    fn read_val<T: Copy>(&self, addr: usize) -> Option<T> {
        let mut v: T = unsafe { mem::zeroed() };
        let ok = self.read(addr, unsafe { std::slice::from_raw_parts_mut(&mut v as *mut T as *mut u8, mem::size_of::<T>()) });
        if ok { Some(v) } else { None }
    }

    fn query(&self, addr: usize) -> Option<MEMORY_BASIC_INFORMATION> {
        unsafe {
            let mut mbi: MEMORY_BASIC_INFORMATION = mem::zeroed();
            if VirtualQueryEx(self.h, addr as *const c_void, &mut mbi, mem::size_of::<MEMORY_BASIC_INFORMATION>()) != 0 { Some(mbi) } else { None }
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) { unsafe { CloseHandle(self.h as isize); } }
}

struct ModInfo { base: usize, size: usize, name: String }

fn remote_modules(pid: u32) -> Vec<ModInfo> {
    let mut mods = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(0x00000008, pid);
        if snap == -1 { return mods; }
        let mut me: MODULEENTRY32A = mem::zeroed();
        me.dwSize = mem::size_of::<MODULEENTRY32A>() as u32;
        if Module32First(snap, &mut me) != 0 {
            loop {
                let name = CStr::from_ptr(me.szModule.as_ptr() as *const c_char).to_string_lossy().to_string();
                mods.push(ModInfo { base: me.modBaseAddr as usize, size: me.modBaseSize as usize, name });
                if Module32Next(snap, &mut me) == 0 { break; }
            }
        }
        CloseHandle(snap);
    }
    mods
}

fn in_range(base: usize, size: usize, addr: usize) -> bool {
    addr != 0 && addr >= base && addr < base.saturating_add(size)
}

/// Remote PE export check: does the MEM_IMAGE mapping at `base` export `target`?
fn pe_exports(proc_: &Proc, base: usize, target: &str) -> bool {
    let mz: u16 = match proc_.read_val(base) { Some(v) => v, None => return false };
    if mz != 0x5A4D { return false; }
    let e_lfanew: i32 = match proc_.read_val(base + 0x3C) { Some(v) => v, None => return false };
    if e_lfanew <= 0 || e_lfanew > 0x1000 { return false; }
    let pe = base + e_lfanew as usize;
    let sig: u32 = match proc_.read_val(pe) { Some(v) => v, None => return false };
    if sig != 0x00004550 { return false; }
    let magic: u16 = match proc_.read_val(pe + 24) { Some(v) => v, None => return false };
    if magic != 0x20B { return false; }
    let opt = pe + 24;
    let size_of_image: u32 = match proc_.read_val(opt + 56) { Some(v) => v, None => return false };
    if size_of_image < 0x1000 || size_of_image > 0x4000_0000 { return false; }
    let export_rva: u32 = match proc_.read_val(opt + 112) { Some(v) => v, None => return false };
    let export_size: u32 = match proc_.read_val(opt + 116) { Some(v) => v, None => return false };
    if export_rva == 0 || export_size == 0 { return false; }
    let soi = size_of_image as usize;
    if (export_rva as usize) + (export_size as usize) > soi { return false; }
    let exp = base + export_rva as usize;
    let num_names: u32 = match proc_.read_val(exp + 20) { Some(v) => v, None => return false };
    let names_rva: u32 = match proc_.read_val(exp + 32) { Some(v) => v, None => return false };
    if num_names == 0 || num_names > 0x10000 { return false; }
    if (names_rva as usize) + (num_names as usize) * 4 > soi { return false; }
    let names = base + names_rva as usize;
    let t = target.as_bytes();
    let mut name_buf = vec![0u8; t.len() + 1];
    for i in 0..num_names as usize {
        let name_rva: u32 = match proc_.read_val(names + i * 4) { Some(v) => v, None => continue };
        if name_rva == 0 || (name_rva as usize) >= soi { continue; }
        if !proc_.read(base + name_rva as usize, &mut name_buf) { continue; }
        if &name_buf[..t.len()] == t && name_buf[t.len()] == 0 { return true; }
    }
    false
}

/// Find jvm.dll: prefer Toolhelp, fall back to walking the remote address space
/// for a MEM_IMAGE mapping that exports JNI_CreateJavaVM. Returns (base, size_of_image).
fn find_jvm(proc_: &Proc, mods: &[ModInfo]) -> Option<(usize, usize)> {
    for m in mods {
        if m.name.eq_ignore_ascii_case("jvm.dll") {
            return Some((m.base, m.size));
        }
    }
    const MEM_IMAGE: u32 = 0x1000000;
    const MEM_COMMIT: u32 = 0x1000;
    let mut curr: usize = 0;
    let mut last_probe: usize = 1;
    while curr < 0x0000_7FFF_FFFE_0000 {
        let mbi = match proc_.query(curr) { Some(m) => m, None => break };
        if mbi.RegionSize == 0 { break; }
        if mbi.State == MEM_COMMIT && mbi.Type == MEM_IMAGE {
            let probe = mbi.AllocationBase as usize;
            if probe != 0 && probe != last_probe && (probe & 0xFFF) == 0 {
                last_probe = probe;
                if pe_exports(proc_, probe, "JNI_CreateJavaVM") {
                    let e_lfanew: i32 = proc_.read_val(probe + 0x3C).unwrap_or(0);
                    let opt = probe + e_lfanew as usize + 24;
                    let soi: u32 = proc_.read_val(opt + 56).unwrap_or(0x21F0000);
                    return Some((probe, soi as usize));
                }
            }
        }
        curr = match curr.checked_add(mbi.RegionSize) { Some(n) if n > curr => n, _ => break };
    }
    None
}

/// Scan the jvm.dll image for the global JNIEnv function table:
/// qwords [0],[1],[2] == 0, [4] points into the image, and >=220 of entries
/// 4..233 are either 0 or in-image. Tolerates a handful of hooked (heap) entries.
fn find_jni_table(proc_: &Proc, jvm_base: usize, jvm_size: usize) -> Option<usize> {
    // Fast path: the RustMe build keeps the JNIEnv table at a stable RVA.
    // Observed across multiple runs (bases 0x7FFEA4A80000, 0x7FFF24480000):
    // table at jvm_base + 0xE6CAF0. Validate before trusting.
    const KNOWN_TABLE_RVA: usize = 0xE6CAF0;
    if KNOWN_TABLE_RVA + 233 * 8 <= jvm_size {
        let cand = jvm_base + KNOWN_TABLE_RVA;
        let get = |k: usize| -> usize { proc_.read_val::<u64>(cand + k * 8).unwrap_or(0x1) as usize };
        if get(0) == 0 && get(1) == 0 && get(2) == 0 && in_range(jvm_base, jvm_size, get(4)) {
            let good = (4..233).filter(|&k| get(k) == 0 || in_range(jvm_base, jvm_size, get(k))).count();
            if good >= 200 { return Some(cand); }
        }
    }

    const CHUNK: usize = 256 * 1024;
    const OVERLAP: usize = 233 * 8 + 8;
    let mut off = 0usize;
    let mut carry: Vec<u8> = Vec::new();
    while off < jvm_size {
        let want = std::cmp::min(CHUNK, jvm_size - off);
        let mut buf = vec![0u8; want + carry.len()];
        let start = jvm_base + off - carry.len();
        proc_.read_tolerant(start, &mut buf);
        let base_addr = start;
        let n = buf.len();
        if n >= OVERLAP {
            let mut i = 0usize;
            while i + 233 * 8 <= n {
                let get = |k: usize| -> usize {
                    u64::from_le_bytes(buf[i + k * 8..i + k * 8 + 8].try_into().unwrap()) as usize
                };
                if get(0) == 0 && get(1) == 0 && get(2) == 0 && in_range(jvm_base, jvm_size, get(4)) {
                    let good = (4..233).filter(|&k| get(k) == 0 || in_range(jvm_base, jvm_size, get(k))).count();
                    if good >= 220 {
                        return Some(base_addr + i);
                    }
                }
                i += 8;
            }
            carry = buf[n - OVERLAP..].to_vec();
        }
        off += want;
    }
    None
}

fn classify(mods: &[ModInfo], jvm_base: usize, jvm_size: usize, addr: usize) -> String {
    if addr == 0 { return "NULL".into(); }
    for m in mods {
        if in_range(m.base, m.size, addr) { return format!("{}+0x{:X}", m.name, addr - m.base); }
    }
    if in_range(jvm_base, jvm_size, addr) { return format!("jvm.dll+0x{:X} (unlinked)", addr - jvm_base); }
    format!("0x{:X} (foreign)", addr)
}

fn snap16(proc_: &Proc, addr: usize) -> String {
    if addr == 0 { return String::new(); }
    let mut b = [0u8; 16];
    if !proc_.read(addr, &mut b) { return "<unreadable>".into(); }
    b.iter().map(|x| format!("{:02X}", x)).collect::<Vec<_>>().join(" ")
}

/// Best-effort recovery of the original function a hook stub forwards to.
/// Parses the usual x86-64 trampoline shapes in the first 48 bytes.
fn resolve_stub_target(proc_: &Proc, stub: usize) -> Option<usize> {
    let mut b = [0u8; 48];
    if !proc_.read(stub, &mut b) { return None; }
    // E9 rel32
    if b[0] == 0xE9 {
        let rel = i32::from_le_bytes(b[1..5].try_into().unwrap()) as i64;
        return Some((stub as i64 + 5 + rel) as usize);
    }
    // EB rel8
    if b[0] == 0xEB {
        let rel = b[1] as i8 as i64;
        return Some((stub as i64 + 2 + rel) as usize);
    }
    // 48 B8 imm64 ; FF E0   (movabs rax, imm; jmp rax)
    if b[0] == 0x48 && b[1] == 0xB8 {
        let t = u64::from_le_bytes(b[2..10].try_into().unwrap()) as usize;
        if b[10] == 0xFF && b[11] == 0xE0 { return Some(t); }
    }
    // 49 BA imm64 ; 41 FF E2 (movabs r10, imm; jmp r10)
    if b[0] == 0x49 && b[1] == 0xBA {
        let t = u64::from_le_bytes(b[2..10].try_into().unwrap()) as usize;
        if b[10] == 0x41 && b[11] == 0xFF && b[12] == 0xE2 { return Some(t); }
    }
    // 48 B8 imm64 ; 41 FF E0 / 48 FF E0 ...
    if b[0] == 0x48 && b[1] == 0xB8 {
        return Some(u64::from_le_bytes(b[2..10].try_into().unwrap()) as usize);
    }
    // FF 25 rel32 / 48 FF 25 rel32: jmp [rip+rel]
    if b[0] == 0xFF && b[1] == 0x25 {
        let rel = i32::from_le_bytes(b[2..6].try_into().unwrap()) as i64;
        let slot = (stub as i64 + 6 + rel) as usize;
        return proc_.read_val::<u64>(slot).map(|v| v as usize);
    }
    if b[0] == 0x48 && b[1] == 0xFF && b[2] == 0x25 {
        let rel = i32::from_le_bytes(b[3..7].try_into().unwrap()) as i64;
        let slot = (stub as i64 + 7 + rel) as usize;
        return proc_.read_val::<u64>(slot).map(|v| v as usize);
    }
    None
}

fn main() {
    let pid: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        eprintln!("usage: probe.exe <pid> [rva len]   (run elevated)");
        std::process::exit(2);
    });

    enable_debug_privilege();

    let proc_ = match Proc::open(pid) {
        Some(p) => p,
        None => {
            eprintln!("OpenProcess(pid={}) failed, err={}. Run elevated?", pid, unsafe { GetLastError() });
            std::process::exit(1);
        }
    };
    println!("[+] opened pid {}", pid);

    let mods = remote_modules(pid);
    println!("[+] modules visible: {}", mods.len());

    let (jvm_base, jvm_size) = match find_jvm(&proc_, &mods) {
        Some(v) => v,
        None => { eprintln!("[x] jvm.dll not found"); std::process::exit(1); }
    };
    println!("[+] jvm.dll @ 0x{:X} (size 0x{:X})", jvm_base, jvm_size);

    // Raw hex-dump mode: probe.exe <pid> <rva> <len>
    if let (Some(rva_s), Some(len_s)) = (std::env::args().nth(2), std::env::args().nth(3)) {
        let rva: usize = usize::from_str_radix(rva_s.trim_start_matches("0x"), 16).unwrap();
        let len: usize = usize::from_str_radix(len_s.trim_start_matches("0x"), 16).unwrap();
        let mut buf = vec![0u8; len];
        proc_.read_tolerant(jvm_base + rva, &mut buf);
        println!("[+] jvm+0x{:X} ({} bytes):", rva, len);
        for chunk in buf.chunks(16) {
            let off = chunk.as_ptr() as usize - buf.as_ptr() as usize;
            let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
            println!("  +0x{:04X}: {}", off, hex.join(" "));
        }
        std::process::exit(0);
    }

    let table = match find_jni_table(&proc_, jvm_base, jvm_size) {
        Some(t) => t,
        None => { eprintln!("[x] JNIEnv table pattern not found in jvm.dll image"); std::process::exit(1); }
    };
    println!("[+] candidate JNIEnv table @ 0x{:X} (jvm.dll+0x{:X})", table, table - jvm_base);

    let mut entries = [0usize; 233];
    for (i, e) in entries.iter_mut().enumerate() {
        *e = proc_.read_val::<u64>(table + i * 8).unwrap_or(0) as usize;
    }

    let report_path = format!(r"{}\probe_report_{}.txt", REPORT_DIR, pid);
    let raw_path = format!(r"{}\probe_table_{}.bin", REPORT_DIR, pid);
    fs::create_dir_all(REPORT_DIR).ok();
    let mut f = fs::File::create(&report_path).expect("report");
    let mut w = |s: &str| writeln!(f, "{}", s).unwrap();

    w(&format!("pid={} jvm_base=0x{:X} jvm_size=0x{:X} table=0x{:X} table_rva=0x{:X}", pid, jvm_base, jvm_size, table, table - jvm_base));
    w("");
    for (i, &e) in entries.iter().enumerate() {
        w(&format!("[{:>3}] 0x{:016X}  {:<44} | {}", i, e, classify(&mods, jvm_base, jvm_size, e), snap16(&proc_, e)));
    }
    let raw: Vec<u8> = entries.iter().flat_map(|e| (*e as u64).to_le_bytes()).collect();
    fs::write(&raw_path, &raw).ok();

    println!("[+] report -> {}", report_path);
    println!("[+] raw    -> {}", raw_path);
    println!("[+] non-jvm targets:");
    for (i, &e) in entries.iter().enumerate() {
        if e != 0 && !in_range(jvm_base, jvm_size, e) {
            let cls = classify(&mods, jvm_base, jvm_size, e);
            let target = resolve_stub_target(&proc_, e);
            let tstr = match target {
                Some(t) => format!("  -> original: 0x{:X}  {}", t, classify(&mods, jvm_base, jvm_size, t)),
                None => "  -> unresolved stub".into(),
            };
            println!("    [{}] 0x{:X} {} {}", i, e, cls, tstr);
            let mut b64 = [0u8; 128];
            let hex64 = if proc_.read(e, &mut b64) {
                b64.iter().map(|x| format!("{:02X}", x)).collect::<Vec<_>>().join(" ")
            } else { "<unreadable>".into() };
            let disp = proc_.read_val::<u32>(jvm_base + 0xE93440).map(|v| format!("0x{:X}", v)).unwrap_or("<n/a>".into());
            w(&format!("[HOOK {}] stub=0x{:X} {} | {} | dispatch_u32@jvm+0xE93440={} | stub64: {}", i, e, cls, snap16(&proc_, e), disp, hex64));
            if let Some(t) = target {
                w(&format!("         -> original: 0x{:X}  {}", t, classify(&mods, jvm_base, jvm_size, t)));
            }
        }
    }
}
