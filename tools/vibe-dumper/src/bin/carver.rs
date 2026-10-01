#![allow(non_snake_case, non_camel_case_types)]

//! External class carver.
//!
//! Opens the target with PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only:
//! no DLL is injected, nothing in the target is written, patched or suspended.
//! Class files are recovered by scanning committed private memory for the
//! class-file magic and computing the real file length by walking the constant
//! pool, which recovers classes the custom JVM has already decrypted in memory.
//!
//! usage: carver.exe <pid>

use std::collections::HashSet;
use std::ffi::{c_void, CString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr;

const OUT_DIR: &str = r"d:\project\rustme\dump\carved";
const REPORT: &str = r"d:\project\rustme\dump\carve_report.txt";

#[repr(C)]
struct MBI {
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
    fn OpenProcess(acc: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn ReadProcessMemory(h: *mut c_void, base: *const c_void, buf: *mut c_void, size: usize, read: *mut usize) -> i32;
    fn VirtualQueryEx(h: *mut c_void, addr: *const c_void, buf: *mut MBI, len: usize) -> usize;
    fn CloseHandle(h: isize) -> i32;
    fn GetLastError() -> u32;
    fn GetCurrentProcess() -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(h: *mut c_void, acc: u32, tok: *mut *mut c_void) -> i32;
    fn LookupPrivilegeValueA(host: *const u8, name: *const u8, luid: *mut u64) -> i32;
    fn AdjustTokenPrivileges(tok: *mut c_void, dis: i32, tp: *const u8, len: u32, prev: *mut c_void, plen: *mut u32) -> i32;
}

fn enable_debug_privilege() {
    unsafe {
        let mut token: *mut c_void = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), 0x0020 | 0x0008, &mut token) == 0 { return; }
        let mut luid: u64 = 0;
        let name = CString::new("SeDebugPrivilege").unwrap();
        if LookupPrivilegeValueA(ptr::null(), name.as_ptr() as *const u8, &mut luid) != 0 {
            #[repr(C)]
            struct TP { count: u32, luid: u64, attrs: u32 }
            let tp = TP { count: 1, luid, attrs: 0x2 };
            AdjustTokenPrivileges(token, 0, &tp as *const _ as *const u8,
                                  std::mem::size_of::<TP>() as u32, ptr::null_mut(), ptr::null_mut());
        }
        CloseHandle(token as isize);
    }
}

struct Target { h: *mut c_void }

impl Target {
    fn read_into(&self, addr: usize, buf: &mut [u8]) -> usize {
        unsafe {
            let mut got: usize = 0;
            if ReadProcessMemory(self.h, addr as *const c_void, buf.as_mut_ptr() as *mut c_void, buf.len(), &mut got) == 0 {
                return 0;
            }
            got
        }
    }

    /// Page-tolerant read: pages that cannot be read are zero-filled instead of
    /// aborting the whole request.
    fn read_pages(&self, addr: usize, buf: &mut [u8]) -> usize {
        const PAGE: usize = 4096;
        let mut off = 0usize;
        while off < buf.len() {
            let want = std::cmp::min(PAGE, buf.len() - off);
            let n = self.read_into(addr + off, &mut buf[off..off + want]);
            if n < want {
                for b in &mut buf[off + n..off + want] { *b = 0; }
            }
            off += want;
        }
        buf.len()
    }
}

impl Drop for Target {
    fn drop(&mut self) { unsafe { CloseHandle(self.h as isize); } }
}

// Class files are big-endian (JVMS §3.1); reading them little-endian makes
// every real class look like garbage.
#[inline]
fn u16be(b: &[u8], i: usize) -> Option<u16> {
    if i + 2 > b.len() { return None; }
    Some(u16::from_be_bytes([b[i], b[i + 1]]))
}

#[inline]
fn u32be(b: &[u8], i: usize) -> Option<u32> {
    if i + 4 > b.len() { return None; }
    Some(u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]))
}

/// Walk a class file starting at b[0]; returns (total length, class name).
/// Structural walk only — no semantic validation — so obfuscated and
/// crasher-patched classes still parse, while random CAFEBABE sequences do not.
/// Every offset is bounds-checked and every count capped, because the input is
/// attacker-shaped memory, not a trusted file. On rejection the Err value is
/// the failing step, which is what tells us whether the resident bytes are
/// real class files or something else.
fn parse_class(b: &[u8]) -> Result<(usize, String), &'static str> {
    if b.len() < 10 { return Err("too_short_for_header"); }
    if b[..4] != [0xCA, 0xFE, 0xBA, 0xBE] { return Err("bad_magic"); }
    let major = u16be(b, 6).ok_or("no_major")?;
    if !(45..=70).contains(&major) { return Err("major_out_of_range"); }
    let cp_count = u16be(b, 8).ok_or("no_cp_count")? as usize;
    if cp_count < 2 { return Err("cp_count_lt_2"); }

    let mut p = 10usize;
    let mut utf8: Vec<(u16, usize, usize)> = Vec::with_capacity(cp_count.min(4096));
    let mut classes: Vec<(u16, u16)> = Vec::with_capacity(cp_count.min(1024));

    let mut idx = 1usize;
    while idx < cp_count {
        if p >= b.len() { return Err("cp_out_of_buffer"); }
        let tag = b[p];
        p += 1;
        match tag {
            1 => {
                let l = u16be(b, p).ok_or("utf8_len")? as usize;
                p += 2;
                p = p.checked_add(l).ok_or("utf8_overflow")?;
                if p > b.len() { return Err("utf8_out_of_buffer"); }
                utf8.push((idx as u16, p - l, l));
            }
            7 => {
                let ni = u16be(b, p).ok_or("class_name_idx")?;
                classes.push((idx as u16, ni));
                p += 2;
            }
            9 | 10 | 11 => p += 4,
            8 => p += 2,
            3 | 4 => p += 4,
            5 | 6 => { p += 8; idx += 1; }
            12 => p += 4,
            15 => p += 3,
            16 | 19 | 20 => p += 2,
            17 | 18 => p += 4,
            _ => return Err("bad_cp_tag"),
        }
        if p > b.len() { return Err("cp_walk_overrun"); }
        idx += 1;
    }

    // access_flags(2) this_class(2) super_class(2)
    let this = u16be(b, p + 2).ok_or("no_this_class")?;
    p += 6;

    let ifc = u16be(b, p).ok_or("no_ifc_count")? as usize;
    p += 2 + ifc.checked_mul(2).ok_or("ifc_overflow")?;
    if p > b.len() { return Err("ifc_out_of_buffer"); }

    // field_info / method_info: access(2) name(2) desc(2) attributes_count(2)
    let fields = u16be(b, p).ok_or("no_fields_count")? as usize;
    p += 2;
    if fields > 30_000 { return Err("fields_count_implausible"); }
    for _ in 0..fields {
        let ac = u16be(b, p + 6).ok_or("field_attrs")? as usize;
        p += 8;
        p = skip_attributes(b, p, ac).ok_or("field_attr_walk")?;
    }

    let methods = u16be(b, p).ok_or("no_methods_count")? as usize;
    p += 2;
    if methods > 30_000 { return Err("methods_count_implausible"); }
    for _ in 0..methods {
        let ac = u16be(b, p + 6).ok_or("method_attrs")? as usize;
        p += 8;
        p = skip_attributes(b, p, ac).ok_or("method_attr_walk")?;
    }

    let cattrs = u16be(b, p).ok_or("no_class_attrs_count")? as usize;
    p += 2;
    p = skip_attributes(b, p, cattrs).ok_or("class_attr_walk")?;

    if p > 12 * 1024 * 1024 { return Err("class_too_large"); }

    let name_idx = match classes.iter().find(|(ci, _)| *ci == this) {
        Some(e) => e.1,
        None => return Err("this_class_not_in_pool"),
    };
    let entry = match utf8.iter().find(|(ci, _, _)| *ci == name_idx) {
        Some(e) => e,
        None => return Err("name_utf8_missing"),
    };
    let (s, l) = (entry.1, entry.2);
    if s + l > b.len() || l == 0 || l > 512 { return Err("name_slice_bounds"); }
    let name = String::from_utf8_lossy(&b[s..s + l]).to_string();
    if !plausible_class_name(&name) { return Err("implausible_name"); }
    Ok((p, name))
}

fn plausible_class_name(n: &str) -> bool {
    if n.is_empty() || n.starts_with('/') || n.contains("//") { return false; }
    !n.bytes().any(|c| {
        c < 0x20
            || c == 0x7F
            || matches!(c, b':' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|' | b'\\')
    })
}

/// attribute_info = name_index(2) length(4) bytes(length)
fn skip_attributes(b: &[u8], mut p: usize, count: usize) -> Option<usize> {
    if count > 100_000 { return None; }
    for _ in 0..count {
        let len = u32be(b, p + 2)? as usize;
        p = p.checked_add(6)?.checked_add(len)?;
        if p > b.len() { return None; }
    }
    Some(p)
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn main() {
    // Offline self-test: carver.exe --file <path.class> [more.class...]
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(|s| s.as_str()) == Some("--file") {
        let mut ok = 0usize;
        let mut bad = 0usize;
        for path in &argv[2..] {
            let data = match fs::read(path) { Ok(d) => d, Err(e) => { println!("READ FAIL {} {}", path, e); bad += 1; continue; } };
            match parse_class(&data) {
                Ok((len, name)) => {
                    let mark = if len == data.len() { "EXACT" } else { "LEN_MISMATCH" };
                    println!("OK   {} len={} file={} {} name={}", mark, len, data.len(), path, name);
                    ok += 1;
                }
                Err(why) => { println!("FAIL {} :: {}", path, why); bad += 1; }
            }
        }
        println!("selftest: ok={} fail={}", ok, bad);
        return;
    }

    let pid: u32 = match argv.get(1).and_then(|s| s.parse().ok()) {
        Some(p) => p,
        None => { eprintln!("usage: carver.exe <pid> | carver.exe --file <class...>"); std::process::exit(2); }
    };
    enable_debug_privilege();

    let h = unsafe { OpenProcess(0x0410, 0, pid) };
    if h.is_null() {
        eprintln!("OpenProcess({}) failed err={}", pid, unsafe { GetLastError() });
        std::process::exit(1);
    }
    let t = Target { h };
    fs::create_dir_all(OUT_DIR).ok();
    let mut rep = fs::File::create(REPORT).expect("report");
    let mut say = |s: String| { writeln!(rep, "{}", s).ok(); let _ = writeln!(std::io::stdout(), "{}", s); };

    say(format!("=== carver pid={} -> {} ===", pid, OUT_DIR));

    const CHUNK: usize = 8 * 1024 * 1024;
    let mut buf: Vec<u8> = vec![0u8; CHUNK];
    let mut seen: HashSet<u64> = HashSet::new();
    let mut stats = Stats::default();
    let mut last_print: u64 = 0;

    let mut addr: usize = 0;
    let mut dead_queries = 0usize;
    while addr < 0x0000_7FFF_FFFE_0000 {
        let mut mbi: MBI = unsafe { std::mem::zeroed() };
        let ok = unsafe { VirtualQueryEx(t.h, addr as *const c_void, &mut mbi, std::mem::size_of::<MBI>()) };
        if ok == 0 {
            dead_queries += 1;
            if dead_queries > 2000 { break; }
            addr += 0x10000;
            continue;
        }
        if mbi.RegionSize == 0 { break; }
        dead_queries = 0;

        let prot = mbi.Protect & 0xFF;
        let readable = matches!(prot, 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80);
        if mbi.State == 0x1000 && mbi.Type == 0x20000 && readable && (mbi.Protect & 0x100) == 0 {
            stats.regions += 1;
            let rbase = mbi.BaseAddress as usize;
            let mut off = 0usize;
            while off < mbi.RegionSize {
                let want = std::cmp::min(CHUNK, mbi.RegionSize - off);
                let got = t.read_pages(rbase + off, &mut buf[..want]);
                stats.bytes += got as u64;
                scan_chunk(&t, &buf[..want], rbase + off, &mut seen, &mut stats);
                off += want;
            }
        }
        addr = match addr.checked_add(mbi.RegionSize) { Some(n) if n > addr => n, _ => break };
        let mb = stats.bytes / 1048576;
        if mb / 512 != last_print {
            last_print = mb / 512;
            say(format!("  … scanned {} MB, {} classes written", mb, stats.written));
        }
    }

    say(format!("regions={} scanned={} MB candidates={} parsed={} written={} dupes={} rejected={}",
        stats.regions, stats.bytes / 1048576, stats.candidates, stats.parsed, stats.written, stats.dupes, stats.rejected));
    for r in &stats.rejects {
        say(format!("  {}", r));
    }
    say(format!("output: {}", OUT_DIR));
}

#[derive(Default)]
struct Stats {
    regions: u64,
    bytes: u64,
    candidates: u64,
    parsed: u64,
    written: u64,
    dupes: u64,
    rejected: u64,
    rejects: Vec<String>,
}

fn scan_chunk(t: &Target, buf: &[u8], chunk_addr: usize, seen: &mut HashSet<u64>, stats: &mut Stats) {
    let mut i = 0usize;
    while i + 10 <= buf.len() {
        if !(buf[i] == 0xCA && buf[i + 1] == 0xFE && buf[i + 2] == 0xBA && buf[i + 3] == 0xBE) {
            i += 1;
            continue;
        }
        stats.candidates += 1;
        let file_addr = chunk_addr + i;

        // Parse in-place; if that fails close to the chunk end, the class may
        // simply be truncated by the chunk boundary — re-read generously.
        let mut outcome = parse_class(&buf[i..]);
        if outcome.is_err() && buf.len() - i < 2 * 1024 * 1024 {
            let mut big = vec![0u8; 4 * 1024 * 1024];
            let gn = t.read_pages(file_addr, &mut big);
            big.truncate(gn);
            outcome = parse_class(&big);
        }
        let (len, name) = match outcome {
            Ok(v) => v,
            Err(why) => {
                stats.rejected += 1;
                if stats.rejects.len() < 24 {
                    let span = std::cmp::min(i + 96, buf.len());
                    let raw = &buf[i..span];
                    let hex: Vec<String> = raw.iter().map(|b| format!("{:02X}", b)).collect();
                    let ascii: String = raw.iter()
                        .map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '.' })
                        .collect();
                    let major = u16be(buf, i + 6).unwrap_or(0);
                    let cpc = u16be(buf, i + 8).unwrap_or(0);
                    stats.rejects.push(format!(
                        "reject @0x{:X}: {} (major={} cp_count={})",
                        file_addr, why, major, cpc));
                    stats.rejects.push(format!("       hex: {}", hex.chunks(16).map(|c| c.join(" ")).collect::<Vec<_>>().join(" | ")));
                    stats.rejects.push(format!("      asci: {}", ascii));
                }
                i += 4;
                continue;
            }
        };
        if !(10..=16 * 1024 * 1024).contains(&len) {
            stats.rejected += 1;
            i += 4;
            continue;
        }
        stats.parsed += 1;

        let data: Vec<u8> = if i + len <= buf.len() {
            buf[i..i + len].to_vec()
        } else {
            let mut d = vec![0u8; len];
            let gn = t.read_pages(file_addr, &mut d);
            d.truncate(gn);
            d
        };
        if data.len() != len {
            stats.rejected += 1;
            i += 4;
            continue;
        }

        // Dedup on identity + content: the same decrypted class is commonly
        // resident several times (loader cache, define buffer, metaspace copy).
        let key = fnv1a(&data) ^ fnv1a(name.as_bytes());
        if !seen.insert(key) {
            stats.dupes += 1;
            i += 4;
            continue;
        }

        let rel: PathBuf = name.split('/').collect();
        let outp = Path::new(OUT_DIR).join(rel).with_extension("class");
        if let Some(parent) = outp.parent() { fs::create_dir_all(parent).ok(); }
        match fs::write(&outp, &data) {
            Ok(_) => stats.written += 1,
            Err(_) => stats.rejected += 1,
        }
        i += 4;
    }
}
