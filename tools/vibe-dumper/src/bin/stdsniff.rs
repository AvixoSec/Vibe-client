// Live sniffer for *decoded* (standard) classfiles in the running JVM's private memory.
// Strict in-process parser: only writes buffers that fully parse as a JVMS classfile.
// usage: stdsniff.exe <pid|auto> [minutes]
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

const OUT_DIR: &str = r"d:\project\rustme\dump\std";
const LOG: &str = r"d:\project\rustme\dump\stdsniff.log";
const WINDOW: usize = 1 << 20;
const CHUNK: usize = 1 << 20;
const OVERLAP: usize = 4096;

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

#[allow(dead_code)]
fn readable(h: HANDLE, a: usize) -> bool {
    let mut mbi: MBI = unsafe { std::mem::zeroed() };
    query(h, a - (a & 4095), &mut mbi)
        && mbi.State == MEM_COMMIT
        && (mbi.Protect & 0x100) == 0
        && (mbi.Protect & (0x02 | 0x04 | 0x08 | 0x10 | 0x20 | 0x40 | 0x80)) != 0
}

/// Bulk read of a range that lies entirely inside one committed, readable region:
/// one ReadProcessMemory call. Falls back to page-wise only when that call fails.
fn read_bulk(h: HANDLE, addr: usize, len: usize, out: &mut Vec<u8>) -> usize {
    out.clear();
    out.resize(len, 0);
    let mut got = 0usize;
    unsafe {
        if ReadProcessMemory(h, addr as *const c_void, out.as_mut_ptr() as *mut c_void, len, &mut got) != 0 && got == len {
            return got;
        }
    }
    // partial/failed: page-wise retry, keep whatever is readable
    let mut page = [0u8; 4096];
    let mut off = 0usize;
    while off < len {
        let a = addr + off;
        let want = std::cmp::min(4096, len - off);
        let mut n = 0usize;
        unsafe {
            if ReadProcessMemory(h, a as *const c_void, page.as_mut_ptr() as *mut c_void, want, &mut n) == 0 { n = 0; }
        }
        if n > 0 { out[off..off + n].copy_from_slice(&page[..n]); }
        off += want;
    }
    len
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

#[inline]
fn u16(b: &[u8], p: usize) -> Option<usize> {
    if p + 2 > b.len() { None } else { Some(((b[p] as usize) << 8) | b[p + 1] as usize) }
}
#[inline]
fn u32_(b: &[u8], p: usize) -> Option<usize> {
    if p + 4 > b.len() { None } else { Some(((b[p] as u32 as usize) << 24) | ((b[p + 1] as usize) << 16) | ((b[p + 2] as usize) << 8) | b[p + 3] as usize) }
}

fn tag_width(t: u8) -> usize {
    match t {
        3 | 4 => 4,
        5 | 6 => 8,
        7 | 8 | 19 | 20 => 2,
        9 | 10 | 11 | 12 | 17 | 18 => 4,
        15 => 3,
        16 => 2,
        _ => 0,
    }
}

/// Strict classfile parse. Returns (size, class_name) or None.
fn parse(b: &[u8]) -> Option<(usize, String)> {
    if b.len() < 10 || u32_(b, 0)? != 0xcafebabe { return None; }
    if u16(b, 4)? != 0 { return None; }
    let major = u16(b, 6)?;
    if major < 45 || major > 69 { return None; }
    let cc = u16(b, 8)?;
    if cc < 3 || cc > 70000 { return None; }
    let mut p = 10usize;
    let mut utf8: Vec<Option<(usize, usize)>> = vec![None; cc];
    let mut cls: Vec<Option<usize>> = vec![None; cc];
    let mut i = 1usize;
    while i < cc {
        let t = *b.get(p)?;
        if t == 1 {
            let l = u16(b, p + 1)?;
            let s = p + 3;
            if s + l > b.len() { return None; }
            utf8[i] = Some((s, l));
            p = s + l;
        } else {
            let w = tag_width(t);
            if w == 0 { return None; }
            if t == 7 { cls[i] = Some(u16(b, p + 1)?); }
            if t == 5 || t == 6 { i += 1; if i >= cc { return None; } }
            p += 1 + w;
        }
        i += 1;
    }
    let body = p;
    p += 8; // access, this_class, super_class, interfaces_count
    let n_if = u16(b, body + 6)?;
    if n_if > 2000 { return None; }
    p = body + 8 + n_if * 2;

    let rd_attrs = |b: &[u8], p: &mut usize, c: usize| -> bool {
        for _ in 0..c {
            let l = match u32_(b, *p + 2) { Some(v) => v, None => return false };
            if l > 4_000_000 || *p + 6 + l > b.len() { return false; }
            *p += 6 + l;
        }
        true
    };

    for _ in 0..2 {
        let c = u16(b, p)?;
        if c > 60000 { return None; }
        p += 2;
        for _ in 0..c {
            p += 6;
            let ac = u16(b, p)?;
            p += 2;
            if ac > 200 { return None; }
            if !rd_attrs(b, &mut p, ac) { return None; }
        }
    }
    let ac = u16(b, p)?;
    p += 2;
    if ac > 200 { return None; }
    if !rd_attrs(b, &mut p, ac) { return None; }

    let this = u16(b, body + 2)?;
    let nm = *cls.get(this)?.as_ref()?;
    let (s, l) = *utf8.get(nm)?.as_ref()?;
    if l == 0 || l > 512 { return None; }
    let raw = &b[s..s + l];
    if !raw.iter().all(|&c| (0x20..0x7f).contains(&c) || c == b'\t') { return None; }
    let name = String::from_utf8_lossy(raw).to_string();
    if !name.contains('/') && !name.contains('.') { /* synthetic/anonymous ok, still require printable */ }
    Some((p, name))
}

fn safe_name(n: &str) -> String {
    n.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '$' || c == '[' || c == ';' { c } else { '_' }).collect()
}

fn fnv(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &bb in data { h ^= bb as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

struct Stat { regions: usize, cands: u64, valid: u64, saved: u64, by_name: usize }

fn scan(h: HANDLE, lw: &mut dyn FnMut(String), st: &mut Stat, deadline: Duration) {
    let t0 = Instant::now();
    let mut chunk: Vec<u8> = Vec::new();
    let mut win: Vec<u8> = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    let mut pass = 0usize;
    let mut empty = 0usize;
    let mut regions = private_rw_regions(h);
    st.regions = regions.len();
    lw(format!("[+] private-RW regions={} total={} MB", regions.len(), regions.iter().map(|r| r.1).sum::<usize>() / 1048576));

    while t0.elapsed() < deadline {
        pass += 1;
        let pstart = Instant::now();
        let mut pass_cands = 0u64;
        let mut pass_valid = 0u64;
        let mut pass_saved = 0u64;
        let mut read_bytes = 0usize;
        for (base, size) in &regions {
            let mut off = 0usize;
            while off < *size {
                let n = std::cmp::min(CHUNK, *size - off);
                let a = base + off;
                let got = read_bulk(h, a, n, &mut chunk);
                read_bytes += got;
                let mut i = 0usize;
                while i + 10 <= got {
                    if chunk[i] != 0xCA || chunk[i + 1] != 0xFE { i += 1; continue; }
                    if chunk[i + 2] != 0xBA || chunk[i + 3] != 0xBE { i += 4; continue; }
                    if chunk[i + 6] == 0xFA { i += 8; continue; }
                    pass_cands += 1;
                    let addr = a + i;
                    let mut res = parse(&chunk[i..got]);
                    if res.is_none() && got - i < WINDOW {
                        let w = std::cmp::min(WINDOW, *size - (addr - *base));
                        read_bulk(h, addr, w, &mut win);
                        res = parse(&win[..w]);
                    }
                    if let Some((sz, name)) = res {
                        pass_valid += 1;
                        if names.insert(name.clone()) {
                            pass_saved += 1;
                            st.saved += 1;
                            let src = if sz <= got - i { &chunk[i..i + sz] } else { &win[..sz] };
                            let key = fnv(&src[..std::cmp::min(sz, 4096)]);
                            let _ = fs::write(format!("{}\\{}_{}_{:x}.class", OUT_DIR, safe_name(&name), sz, key), src);
                        }
                    }
                    i += 8;
                }
                if n > OVERLAP && *size - off > n { off += n - OVERLAP; } else { off += n; }
            }
        }
        st.cands += pass_cands;
        st.valid += pass_valid;
        st.by_name = names.len();
        lw(format!("[pass {}] {:.1}s read={} MB cands={} valid={} new={} uniq_names={} saved={} total_saved={}",
            pass, pstart.elapsed().as_secs_f64(), read_bytes / 1048576, pass_cands, pass_valid, pass_saved, names.len(), st.saved, st.saved));
        if read_bytes == 0 {
            empty += 1;
            if empty >= 4 { lw("[!] no readable memory for 4 passes — target gone?".into()); return; }
        } else { empty = 0; }
        if pass % 2 == 0 || pstart.elapsed() > Duration::from_secs(20) { regions = private_rw_regions(h); }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("selftest") {
        // selftest <dir-with-.class-files>: prove the in-process parser accepts real classfiles
        let dir = args.get(2).cloned().unwrap_or_default();
        let mut ok = 0usize; let mut bad = 0usize;
        if let Ok(rd) = fs::read_dir(&dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().map(|e| e != "class").unwrap_or(true) { continue; }
                let b = match fs::read(&p) { Ok(v) => v, Err(_) => continue };
                match parse(&b) {
                    Some((sz, nm)) => { ok += 1; println!("ok  {:>8} / {}  (file {})", sz, nm, b.len()); }
                    None => { bad += 1; println!("FAIL {}", p.display()); }
                }
            }
        }
        println!("selftest: parsed {} ok, {} fail", ok, bad);
        return;
    }
    let spec = args.get(1).cloned().unwrap_or_else(|| "auto".to_string());
    let minutes: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let auto = spec.eq_ignore_ascii_case("auto");
    let fixed: DWORD = if auto { 0 } else { spec.parse().unwrap_or(0) };
    if !auto && fixed == 0 { eprintln!("usage: stdsniff.exe <pid|auto> [minutes]"); std::process::exit(2); }

    fs::create_dir_all(OUT_DIR).ok();
    let logf = fs::OpenOptions::new().create(true).append(true).open(LOG).unwrap();
    let mut logf = logf;
    let mut lw = |s: String| { let _ = writeln!(logf, "{}", s); let _ = logf.flush(); };

    debug_priv();
    let deadline = Duration::from_secs(minutes * 60);
    let mut st = Stat { regions: 0, cands: 0, valid: 0, saved: 0, by_name: 0 };
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
            scan(h, &mut lw, &mut st, dl);
            CloseHandle(h);
            if !auto { break; }
            if t0.elapsed() > deadline { break; }
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    lw(format!("[+] exited cands={} valid={} uniq={} saved={}", st.cands, st.valid, st.by_name, st.saved));
}
