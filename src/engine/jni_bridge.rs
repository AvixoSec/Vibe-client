#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::missing_safety_doc)]

//! Astraea-aware JNI bridge for the Vibe Client.
//!
//! Provides crash-proof JNI calls through a VEH gate, Astraea stub
//! unwrapping, PE-scan fallback for unlinked jvm.dll, and full
//! game-state reading from MCP 1.12.2 obfuscated classes.

use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::ptr;

use crate::core::snapshot::*;

// ══════════════════════════════════════════════════════════════════════
// JNI Type Aliases
// ══════════════════════════════════════════════════════════════════════

pub type Jint = i32;
pub type Jsize = i32;
pub type Jboolean = u8;
pub type Jlong = i64;
pub type Jfloat = f32;
pub type Jdouble = f64;
pub type Jobject = *mut c_void;
pub type Jclass = Jobject;
pub type JmethodID = *mut c_void;
pub type JfieldID = *mut c_void;
pub type Jstring = Jobject;
pub type Jarray = Jobject;

pub type JavaVM = *mut *const JNIInvokeInterface;
pub type JNIEnv = *mut *const c_void;

pub const JNI_OK: Jint = 0;
pub const JNI_VERSION_1_8: Jint = 0x00010008;

#[repr(C)]
pub struct JNIInvokeInterface {
    pub reserved0: *mut c_void,
    pub reserved1: *mut c_void,
    pub reserved2: *mut c_void,
    pub destroy_java_vm: unsafe extern "system" fn(JavaVM) -> Jint,
    pub attach_current_thread: unsafe extern "system" fn(JavaVM, *mut *mut c_void, *mut c_void) -> Jint,
    pub detach_current_thread: unsafe extern "system" fn(JavaVM) -> Jint,
    pub get_env: unsafe extern "system" fn(JavaVM, *mut *mut c_void, Jint) -> Jint,
    pub attach_current_thread_as_daemon: unsafe extern "system" fn(JavaVM, *mut *mut c_void, *mut c_void) -> Jint,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union jvalue {
    pub z: Jboolean,
    pub b: i8,
    pub c: u16,
    pub s: i16,
    pub i: Jint,
    pub j: Jlong,
    pub f: Jfloat,
    pub d: Jdouble,
    pub l: Jobject,
}

#[repr(C)]
struct JavaVMAttachArgs {
    version: i32,
    name: *const c_char,
    group: *mut c_void,
}

// ══════════════════════════════════════════════════════════════════════
// JNI Function Table Offsets
// ══════════════════════════════════════════════════════════════════════

const JNI_GET_VERSION: usize = 4;
const JNI_FIND_CLASS: usize = 6;
const JNI_FROM_REFLECTED_METHOD: usize = 7;
const JNI_FROM_REFLECTED_FIELD: usize = 8;
const JNI_EXCEPTION_OCCURRED: usize = 15;
const JNI_EXCEPTION_CLEAR: usize = 17;
const JNI_PUSH_LOCAL_FRAME: usize = 19;
const JNI_POP_LOCAL_FRAME: usize = 20;
const JNI_NEW_GLOBAL_REF: usize = 21;
const JNI_DELETE_GLOBAL_REF: usize = 22;
const JNI_DELETE_LOCAL_REF: usize = 23;
const JNI_IS_SAME_OBJECT: usize = 24;
const JNI_GET_OBJECT_CLASS: usize = 31;
const JNI_GET_METHOD_ID: usize = 33;
const JNI_CALL_OBJECT_METHOD_A: usize = 36;
const JNI_CALL_BOOLEAN_METHOD_A: usize = 39;
const JNI_CALL_INT_METHOD_A: usize = 51;
const JNI_CALL_LONG_METHOD_A: usize = 54;
const JNI_CALL_FLOAT_METHOD_A: usize = 57;
const JNI_CALL_DOUBLE_METHOD_A: usize = 60;
const JNI_CALL_VOID_METHOD_A: usize = 63;
const JNI_GET_FIELD_ID: usize = 94;
const JNI_GET_OBJECT_FIELD: usize = 95;
const JNI_GET_BOOLEAN_FIELD: usize = 96;
const JNI_GET_INT_FIELD: usize = 100;
const JNI_GET_FLOAT_FIELD: usize = 102;
const JNI_GET_DOUBLE_FIELD: usize = 103;
const JNI_SET_BOOLEAN_FIELD: usize = 105;
const JNI_SET_INT_FIELD: usize = 109;
const JNI_SET_FLOAT_FIELD: usize = 111;
const JNI_SET_DOUBLE_FIELD: usize = 112;
const JNI_GET_STATIC_METHOD_ID: usize = 113;
const JNI_CALL_STATIC_OBJECT_METHOD_A: usize = 116;
const JNI_CALL_STATIC_BOOLEAN_METHOD_A: usize = 119;
const JNI_CALL_STATIC_INT_METHOD_A: usize = 122;
const JNI_CALL_STATIC_VOID_METHOD_A: usize = 143;
const JNI_GET_STATIC_FIELD_ID: usize = 144;
const JNI_GET_STATIC_OBJECT_FIELD: usize = 145;
const JNI_NEW_STRING_UTF: usize = 167;
const JNI_GET_STRING_UTF_CHARS: usize = 169;
const JNI_RELEASE_STRING_UTF_CHARS: usize = 170;
const JNI_GET_ARRAY_LENGTH: usize = 171;
const JNI_NEW_OBJECT_ARRAY: usize = 172;
const JNI_GET_OBJECT_ARRAY_ELEMENT: usize = 173;
const JNI_EXCEPTION_CHECK: usize = 228;

// ══════════════════════════════════════════════════════════════════════
// VEH Crash-Proof Gate
// ══════════════════════════════════════════════════════════════════════

pub static mut G_GUARD_ON: bool = false;
pub static mut G_GUARD_THREAD_ID: u32 = 0;
pub static mut G_RENDER_GUARD: bool = false;
pub static mut G_RENDER_RSP: usize = 0;
pub static mut G_RENDER_CALSAVE: [usize; 8] = [0; 8];
pub static mut G_RENDER_TID: u32 = 0;
pub static mut G_RENDER_FAULTED: bool = false;
static mut G_GUARD_RSP: usize = 0;
static mut G_GUARD_FAULTED: bool = false;
static mut G_CALSAVE: [usize; 8] = [0; 8];
static mut G_JVM_BASE: usize = 0;
static mut G_JVM_SIZE: usize = 0;
static mut G_VEH_INSTALLED: bool = false;

#[inline(always)]
unsafe fn save_calsave() {
    let (rbx, rbp, rdi, rsi, r12, r13, r14, r15): (usize, usize, usize, usize, usize, usize, usize, usize);
    core::arch::asm!(
        "mov {0}, rbx", "mov {1}, rbp", "mov {2}, rdi", "mov {3}, rsi",
        "mov {4}, r12", "mov {5}, r13", "mov {6}, r14", "mov {7}, r15",
        out(reg) rbx, out(reg) rbp, out(reg) rdi, out(reg) rsi,
        out(reg) r12, out(reg) r13, out(reg) r14, out(reg) r15,
        options(nostack)
    );
    G_CALSAVE = [rbx, rbp, rdi, rsi, r12, r13, r14, r15];
}

#[no_mangle]
extern "C" fn vibe_guard_abort() -> u64 { 0 }

#[no_mangle]
pub extern "C" fn render_abort() -> u64 { 0 }

pub unsafe fn guarded_render_call(f: unsafe extern "C" fn()) -> bool {
    let (rbx, rbp, rdi, rsi, r12, r13, r14, r15): (usize, usize, usize, usize, usize, usize, usize, usize);
    core::arch::asm!(
        "mov {0}, rbx", "mov {1}, rbp", "mov {2}, rdi", "mov {3}, rsi",
        "mov {4}, r12", "mov {5}, r13", "mov {6}, r14", "mov {7}, r15",
        out(reg) rbx, out(reg) rbp, out(reg) rdi, out(reg) rsi,
        out(reg) r12, out(reg) r13, out(reg) r14, out(reg) r15,
        options(nostack)
    );
    G_RENDER_CALSAVE = [rbx, rbp, rdi, rsi, r12, r13, r14, r15];
    G_RENDER_RSP = cur_rsp();
    G_RENDER_FAULTED = false;
    G_RENDER_TID = GetCurrentThreadId();
    G_RENDER_GUARD = true;

    f();

    G_RENDER_GUARD = false;
    !G_RENDER_FAULTED
}

#[inline(always)]
fn cur_rsp() -> usize {
    let r: usize;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) r, options(nomem, nostack, preserves_flags)) };
    r
}

unsafe fn guarded1<R: Copy>(f: usize, a0: usize) -> Option<R> {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_THREAD_ID = GetCurrentThreadId();
    G_GUARD_ON = true;
    let fp: extern "C" fn(usize) -> R = std::mem::transmute(f);
    let r = fp(a0);
    G_GUARD_ON = false;
    if G_GUARD_FAULTED { None } else { Some(r) }
}

unsafe fn guarded2<R: Copy>(f: usize, a0: usize, a1: usize) -> Option<R> {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_THREAD_ID = GetCurrentThreadId();
    G_GUARD_ON = true;
    let fp: extern "C" fn(usize, usize) -> R = std::mem::transmute(f);
    let r = fp(a0, a1);
    G_GUARD_ON = false;
    if G_GUARD_FAULTED { None } else { Some(r) }
}

unsafe fn guarded3<R: Copy>(f: usize, a0: usize, a1: usize, a2: usize) -> Option<R> {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_THREAD_ID = GetCurrentThreadId();
    G_GUARD_ON = true;
    let fp: extern "C" fn(usize, usize, usize) -> R = std::mem::transmute(f);
    let r = fp(a0, a1, a2);
    G_GUARD_ON = false;
    if G_GUARD_FAULTED { None } else { Some(r) }
}

unsafe fn guarded4<R: Copy>(f: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> Option<R> {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_THREAD_ID = GetCurrentThreadId();
    G_GUARD_ON = true;
    let fp: extern "C" fn(usize, usize, usize, usize) -> R = std::mem::transmute(f);
    let r = fp(a0, a1, a2, a3);
    G_GUARD_ON = false;
    if G_GUARD_FAULTED { None } else { Some(r) }
}

macro_rules! jni_raw {
    ($env:expr, $offset:expr) => {{
        let table = *$env;
        *(table as *const *const c_void).add($offset) as usize
    }};
}

/// Unwrap Astraea gate stub: if vtable entry points outside jvm.dll,
/// scan the stub body for `49 BA <imm64> 41 FF E2` (movabs r10, addr; jmp r10)
/// and recover the original function inside jvm.dll.
unsafe fn effective_fn(env: JNIEnv, slot: usize) -> usize {
    let cur = jni_raw!(env, slot);
    let (base, size) = (G_JVM_BASE, G_JVM_SIZE);
    if base == 0 || size == 0 || (cur >= base && cur < base.wrapping_add(size)) {
        return cur;
    }
    let mut b = [0u8; 128];
    if !read_mem_safe(cur, &mut b) { return cur; }
    let mut i = 0usize;
    while i + 13 <= b.len() {
        if b[i] == 0x49 && b[i + 1] == 0xBA {
            let t = u64::from_le_bytes(b[i + 2..i + 10].try_into().unwrap()) as usize;
            if b[i + 10] == 0x41 && b[i + 11] == 0xFF && b[i + 12] == 0xE2 {
                if t >= base && t < base.wrapping_add(size) {
                    return t;
                }
            }
        }
        i += 1;
    }
    cur
}

unsafe fn read_mem_safe(addr: usize, buf: &mut [u8]) -> bool {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_THREAD_ID = GetCurrentThreadId();
    G_GUARD_ON = true;
    std::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), buf.len());
    G_GUARD_ON = false;
    !G_GUARD_FAULTED
}

// ══════════════════════════════════════════════════════════════════════
// VEH Handler
// ══════════════════════════════════════════════════════════════════════

#[repr(C)]
struct EXCEPTION_RECORD {
    ExceptionCode: u32,
    ExceptionFlags: u32,
    ExceptionRecord: *mut EXCEPTION_RECORD,
    ExceptionAddress: *mut c_void,
    NumberParameters: u32,
    ExceptionInformation: [usize; 15],
}

#[repr(C, align(16))]
struct M128A { low: u64, high: i64 }

#[repr(C, align(16))]
pub struct CONTEXT {
    p1_home: u64, p2_home: u64, p3_home: u64, p4_home: u64, p5_home: u64, p6_home: u64,
    pub context_flags: u32, mx_csr: u32,
    seg_cs: u16, seg_ds: u16, seg_es: u16, seg_fs: u16, seg_gs: u16, seg_ss: u16, e_flags: u32,
    dr0: u64, dr1: u64, dr2: u64, dr3: u64, dr6: u64, dr7: u64,
    pub rax: u64, pub rcx: u64, pub rdx: u64, pub rbx: u64,
    pub rsp: u64, pub rbp: u64, pub rsi: u64, pub rdi: u64,
    pub r8: u64, pub r9: u64, pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64,
    flt_save: [u8; 512],
    vector_register: [M128A; 26],
    vector_control: u64, debug_control: u64,
    last_branch_to_rip: u64, last_branch_from_rip: u64,
    last_exception_to_rip: u64, last_exception_from_rip: u64,
}

#[repr(C)]
struct EXCEPTION_POINTERS {
    ExceptionRecord: *mut EXCEPTION_RECORD,
    ContextRecord: *mut c_void,
}

unsafe extern "system" fn veh_crash_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    if info.is_null() || (*info).ContextRecord.is_null() || (*info).ExceptionRecord.is_null() {
        return 0; // EXCEPTION_CONTINUE_SEARCH
    }

    let cur_tid = GetCurrentThreadId();

    // 1. Guard render overlay: if an OpenGL call inside render_overlay crashed, catch it so the game thread never dies!
    if G_RENDER_GUARD && cur_tid == G_RENDER_TID {
        G_RENDER_GUARD = false;
        G_RENDER_FAULTED = true;
        let code = (*(*info).ExceptionRecord).ExceptionCode;
        log_msg(&format!(
            "[RENDER] Caught hardware exception 0x{:08X} in render_overlay, recovering game thread!",
            code
        ));
        let ctx = &mut *(*info).ContextRecord.cast::<CONTEXT>();
        ctx.rip = render_abort as *const () as usize as u64;
        ctx.rsp = G_RENDER_RSP.wrapping_sub(8) as u64;
        ctx.rax = 0;
        let s = G_RENDER_CALSAVE;
        ctx.rbx = s[0] as u64; ctx.rbp = s[1] as u64;
        ctx.rdi = s[2] as u64; ctx.rsi = s[3] as u64;
        ctx.r12 = s[4] as u64; ctx.r13 = s[5] as u64;
        ctx.r14 = s[6] as u64; ctx.r15 = s[7] as u64;
        return -1; // EXCEPTION_CONTINUE_EXECUTION
    }

    // 2. Guard JNI calls: ONLY on our specific cheat worker thread!
    if G_GUARD_ON && cur_tid == G_GUARD_THREAD_ID {
        let ex_addr = (*(*info).ExceptionRecord).ExceptionAddress as usize;
        let (base, size) = (G_JVM_BASE, G_JVM_SIZE);
        // If exception is inside jvm.dll, HotSpot MUST handle its own implicit null checks / safepoints!
        if base != 0 && size != 0 && ex_addr >= base && ex_addr < base.wrapping_add(size) {
            return 0; // EXCEPTION_CONTINUE_SEARCH -> let HotSpot handle it!
        }
        G_GUARD_ON = false;
        G_GUARD_FAULTED = true;
        let code = (*(*info).ExceptionRecord).ExceptionCode;
        log_msg(&format!(
            "[JNI-VEH] Caught hardware exception 0x{:08X} at 0x{:X} outside JVM, recovering thread!",
            code, ex_addr
        ));
        let ctx = &mut *(*info).ContextRecord.cast::<CONTEXT>();
        ctx.rip = vibe_guard_abort as *const () as usize as u64;
        ctx.rsp = G_GUARD_RSP.wrapping_sub(8) as u64;
        ctx.rax = 0;
        let s = G_CALSAVE;
        ctx.rbx = s[0] as u64; ctx.rbp = s[1] as u64;
        ctx.rdi = s[2] as u64; ctx.rsi = s[3] as u64;
        ctx.r12 = s[4] as u64; ctx.r13 = s[5] as u64;
        ctx.r14 = s[6] as u64; ctx.r15 = s[7] as u64;
        return -1; // EXCEPTION_CONTINUE_EXECUTION
    }

    // 3. For any other thread (e.g. JVM internal implicit null checks, GC, JIT):
    // MUST pass through to JVM's own exception handler!
    0 // EXCEPTION_CONTINUE_SEARCH
}

pub unsafe fn install_veh() {
    if !G_VEH_INSTALLED {
        AddVectoredExceptionHandler(1, veh_crash_handler);
        G_VEH_INSTALLED = true;
    }
}

// ══════════════════════════════════════════════════════════════════════
// JNI Helper Functions
// ══════════════════════════════════════════════════════════════════════

unsafe fn jni_clear_exception(env: JNIEnv) -> bool {
    let f_check = effective_fn(env, JNI_EXCEPTION_CHECK);
    let had = guarded1::<Jboolean>(f_check, env as usize).unwrap_or(0) != 0;
    if had {
        let f_clear = effective_fn(env, JNI_EXCEPTION_CLEAR);
        let _ = guarded1::<()>(f_clear, env as usize);
    }
    had
}

pub unsafe fn jni_find_class(env: JNIEnv, name: &str) -> Jclass {
    let cname = match CString::new(name) { Ok(c) => c, Err(_) => return ptr::null_mut() };
    let f = effective_fn(env, JNI_FIND_CLASS);
    let res = guarded2::<Jclass>(f, env as usize, cname.as_ptr() as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env);
    res
}

pub unsafe fn jni_get_method_id(env: JNIEnv, cls: Jclass, name: &str, sig: &str) -> JmethodID {
    if cls.is_null() { return ptr::null_mut(); }
    let cn = CString::new(name).unwrap(); let cs = CString::new(sig).unwrap();
    let f = effective_fn(env, JNI_GET_METHOD_ID);
    let r = guarded4::<JmethodID>(f, env as usize, cls as usize, cn.as_ptr() as usize, cs.as_ptr() as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_static_method_id(env: JNIEnv, cls: Jclass, name: &str, sig: &str) -> JmethodID {
    if cls.is_null() { return ptr::null_mut(); }
    let cn = CString::new(name).unwrap(); let cs = CString::new(sig).unwrap();
    let f = effective_fn(env, JNI_GET_STATIC_METHOD_ID);
    let r = guarded4::<JmethodID>(f, env as usize, cls as usize, cn.as_ptr() as usize, cs.as_ptr() as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_field_id(env: JNIEnv, cls: Jclass, name: &str, sig: &str) -> JfieldID {
    if cls.is_null() { return ptr::null_mut(); }
    let cn = match CString::new(name) { Ok(c) => c, Err(_) => return ptr::null_mut() };
    let cs = match CString::new(sig) { Ok(c) => c, Err(_) => return ptr::null_mut() };
    let f = effective_fn(env, JNI_GET_FIELD_ID);
    let r = guarded4::<JfieldID>(f, env as usize, cls as usize, cn.as_ptr() as usize, cs.as_ptr() as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_static_field_id(env: JNIEnv, cls: Jclass, name: &str, sig: &str) -> JfieldID {
    if cls.is_null() { return ptr::null_mut(); }
    let cn = match CString::new(name) { Ok(c) => c, Err(_) => return ptr::null_mut() };
    let cs = match CString::new(sig) { Ok(c) => c, Err(_) => return ptr::null_mut() };
    let f = effective_fn(env, JNI_GET_STATIC_FIELD_ID);
    let r = guarded4::<JfieldID>(f, env as usize, cls as usize, cn.as_ptr() as usize, cs.as_ptr() as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_object_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> Jobject {
    if obj.is_null() || fid.is_null() { return ptr::null_mut(); }
    let f = effective_fn(env, JNI_GET_OBJECT_FIELD);
    let r = guarded3::<Jobject>(f, env as usize, obj as usize, fid as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_double_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> f64 {
    if obj.is_null() || fid.is_null() { return 0.0; }
    let f = effective_fn(env, JNI_GET_DOUBLE_FIELD);
    let r = guarded3::<f64>(f, env as usize, obj as usize, fid as usize).unwrap_or(0.0);
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_float_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> f32 {
    if obj.is_null() || fid.is_null() { return 0.0; }
    let f = effective_fn(env, JNI_GET_FLOAT_FIELD);
    let r = guarded3::<f32>(f, env as usize, obj as usize, fid as usize).unwrap_or(0.0);
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_int_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> Jint {
    if obj.is_null() || fid.is_null() { return 0; }
    let f = effective_fn(env, JNI_GET_INT_FIELD);
    let r = guarded3::<Jint>(f, env as usize, obj as usize, fid as usize).unwrap_or(0);
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_boolean_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> bool {
    if obj.is_null() || fid.is_null() { return false; }
    let f = effective_fn(env, JNI_GET_BOOLEAN_FIELD);
    let r = guarded3::<Jboolean>(f, env as usize, obj as usize, fid as usize).unwrap_or(0);
    jni_clear_exception(env); r != 0
}

pub unsafe fn jni_set_float_field(env: JNIEnv, obj: Jobject, fid: JfieldID, val: f32) {
    if obj.is_null() || fid.is_null() { return; }
    let f = effective_fn(env, JNI_SET_FLOAT_FIELD);
    let _ = guarded4::<()>(f, env as usize, obj as usize, fid as usize, val.to_bits() as usize);
    jni_clear_exception(env);
}

pub unsafe fn jni_set_double_field(env: JNIEnv, obj: Jobject, fid: JfieldID, val: f64) {
    if obj.is_null() || fid.is_null() { return; }
    let f = effective_fn(env, JNI_SET_DOUBLE_FIELD);
    let _ = guarded4::<()>(f, env as usize, obj as usize, fid as usize, val.to_bits() as usize);
    jni_clear_exception(env);
}

pub unsafe fn jni_set_boolean_field(env: JNIEnv, obj: Jobject, fid: JfieldID, val: bool) {
    if obj.is_null() || fid.is_null() { return; }
    let f = effective_fn(env, JNI_SET_BOOLEAN_FIELD);
    let _ = guarded4::<()>(f, env as usize, obj as usize, fid as usize, val as usize);
    jni_clear_exception(env);
}

pub unsafe fn jni_call_object(env: JNIEnv, obj: Jobject, mid: JmethodID, args: &[jvalue]) -> Jobject {
    if obj.is_null() || mid.is_null() { return ptr::null_mut(); }
    let f = effective_fn(env, JNI_CALL_OBJECT_METHOD_A);
    let p = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let r = guarded4::<Jobject>(f, env as usize, obj as usize, mid as usize, p as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_call_float(env: JNIEnv, obj: Jobject, mid: JmethodID, args: &[jvalue]) -> f32 {
    if obj.is_null() || mid.is_null() { return 0.0; }
    let f = effective_fn(env, JNI_CALL_FLOAT_METHOD_A);
    let p = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let r = guarded4::<f32>(f, env as usize, obj as usize, mid as usize, p as usize).unwrap_or(0.0);
    jni_clear_exception(env); r
}

pub unsafe fn jni_call_int(env: JNIEnv, obj: Jobject, mid: JmethodID, args: &[jvalue]) -> Jint {
    if obj.is_null() || mid.is_null() { return 0; }
    let f = effective_fn(env, JNI_CALL_INT_METHOD_A);
    let p = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let r = guarded4::<Jint>(f, env as usize, obj as usize, mid as usize, p as usize).unwrap_or(0);
    jni_clear_exception(env); r
}

pub unsafe fn jni_call_void(env: JNIEnv, obj: Jobject, mid: JmethodID, args: &[jvalue]) {
    if obj.is_null() || mid.is_null() { return; }
    let f = effective_fn(env, JNI_CALL_VOID_METHOD_A);
    let p = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let _ = guarded4::<()>(f, env as usize, obj as usize, mid as usize, p as usize);
    jni_clear_exception(env);
}

pub unsafe fn jni_call_static_object(env: JNIEnv, cls: Jclass, mid: JmethodID, args: &[jvalue]) -> Jobject {
    if cls.is_null() || mid.is_null() { return ptr::null_mut(); }
    let f = effective_fn(env, JNI_CALL_STATIC_OBJECT_METHOD_A);
    let p = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let r = guarded4::<Jobject>(f, env as usize, cls as usize, mid as usize, p as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_call_static_void(env: JNIEnv, cls: Jclass, mid: JmethodID, args: &[jvalue]) {
    if cls.is_null() || mid.is_null() { return; }
    let f = effective_fn(env, JNI_CALL_STATIC_VOID_METHOD_A);
    let p = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let _ = guarded4::<()>(f, env as usize, cls as usize, mid as usize, p as usize);
    jni_clear_exception(env);
}

pub unsafe fn jni_get_static_object_field(env: JNIEnv, cls: Jclass, fid: JfieldID) -> Jobject {
    if cls.is_null() || fid.is_null() { return ptr::null_mut(); }
    let f = effective_fn(env, JNI_GET_STATIC_OBJECT_FIELD);
    let r = guarded3::<Jobject>(f, env as usize, cls as usize, fid as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_array_length(env: JNIEnv, array: Jarray) -> Jsize {
    if array.is_null() { return 0; }
    let f = effective_fn(env, JNI_GET_ARRAY_LENGTH);
    let r = guarded2::<Jsize>(f, env as usize, array as usize).unwrap_or(0);
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_object_array_element(env: JNIEnv, array: Jarray, idx: Jsize) -> Jobject {
    if array.is_null() { return ptr::null_mut(); }
    let f = effective_fn(env, JNI_GET_OBJECT_ARRAY_ELEMENT);
    let r = guarded3::<Jobject>(f, env as usize, array as usize, idx as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_new_string_utf(env: JNIEnv, s: &str) -> Jstring {
    let cs = match CString::new(s) { Ok(c) => c, Err(_) => return ptr::null_mut() };
    let f = effective_fn(env, JNI_NEW_STRING_UTF);
    let r = guarded2::<Jstring>(f, env as usize, cs.as_ptr() as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

pub unsafe fn jni_get_string_utf(env: JNIEnv, s: Jstring) -> Option<String> {
    if s.is_null() { return None; }
    let f = effective_fn(env, JNI_GET_STRING_UTF_CHARS);
    let chars = guarded2::<*const c_char>(f, env as usize, s as usize)?;
    if chars.is_null() { return None; }
    let rust_str = CStr::from_ptr(chars).to_string_lossy().to_string();
    let f_rel = effective_fn(env, JNI_RELEASE_STRING_UTF_CHARS);
    let _ = guarded3::<()>(f_rel, env as usize, s as usize, chars as usize);
    Some(rust_str)
}

pub unsafe fn jni_push_local_frame(env: JNIEnv, capacity: Jint) -> Jint {
    let f = effective_fn(env, JNI_PUSH_LOCAL_FRAME);
    guarded2::<Jint>(f, env as usize, capacity as usize).unwrap_or(-1)
}

pub unsafe fn jni_pop_local_frame(env: JNIEnv, result: Jobject) -> Jobject {
    let f = effective_fn(env, JNI_POP_LOCAL_FRAME);
    guarded2::<Jobject>(f, env as usize, result as usize).unwrap_or(ptr::null_mut())
}

pub unsafe fn jni_new_global_ref(env: JNIEnv, obj: Jobject) -> Jobject {
    if obj.is_null() { return ptr::null_mut(); }
    let f = effective_fn(env, JNI_NEW_GLOBAL_REF);
    guarded2::<Jobject>(f, env as usize, obj as usize).unwrap_or(ptr::null_mut())
}

pub unsafe fn jni_delete_global_ref(env: JNIEnv, obj: Jobject) {
    if obj.is_null() { return; }
    let f = effective_fn(env, JNI_DELETE_GLOBAL_REF);
    let _ = guarded2::<()>(f, env as usize, obj as usize);
}

pub unsafe fn jni_delete_local_ref(env: JNIEnv, obj: Jobject) {
    if obj.is_null() { return; }
    let f = effective_fn(env, JNI_DELETE_LOCAL_REF);
    let _ = guarded2::<()>(f, env as usize, obj as usize);
}

pub unsafe fn jni_is_same_object(env: JNIEnv, ref1: Jobject, ref2: Jobject) -> bool {
    let f = effective_fn(env, JNI_IS_SAME_OBJECT);
    guarded3::<Jboolean>(f, env as usize, ref1 as usize, ref2 as usize).unwrap_or(0) != 0
}

pub unsafe fn jni_new_object_array(env: JNIEnv, len: Jsize, element_class: Jclass, initial_element: Jobject) -> Jarray {
    let f = effective_fn(env, JNI_NEW_OBJECT_ARRAY);
    let r = guarded4::<Jarray>(f, env as usize, len as usize, element_class as usize, initial_element as usize).unwrap_or(ptr::null_mut());
    jni_clear_exception(env); r
}

// ══════════════════════════════════════════════════════════════════════
// Win32 FFI
// ══════════════════════════════════════════════════════════════════════

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

#[repr(C)]
struct MODULEENTRY32A {
    dwSize: u32, th32ModuleID: u32, th32ProcessID: u32,
    GlblcntUsage: u32, ProccntUsage: u32,
    modBaseAddr: *mut u8, modBaseSize: u32, hModule: isize,
    szModule: [u8; 256], szExePath: [u8; 260],
}

extern "system" {
    fn GetModuleHandleA(lpModuleName: *const u8) -> isize;
    fn GetProcAddress(hModule: isize, lpProcName: *const u8) -> Option<unsafe extern "system" fn() -> isize>;
    fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize;
    fn Module32First(hSnapshot: isize, lpme: *mut MODULEENTRY32A) -> i32;
    fn Module32Next(hSnapshot: isize, lpme: *mut MODULEENTRY32A) -> i32;
    fn CloseHandle(hObject: isize) -> i32;
    fn VirtualQuery(lpAddress: *const c_void, lpBuffer: *mut MEMORY_BASIC_INFORMATION, dwLength: usize) -> usize;
    fn AddVectoredExceptionHandler(First: u32, Handler: unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32) -> *mut c_void;
    fn GetCurrentThreadId() -> u32;
}

type JNI_GetCreatedJavaVMs_fn = unsafe extern "system" fn(*mut JavaVM, Jsize, *mut Jsize) -> Jint;

// ══════════════════════════════════════════════════════════════════════
// PE Scan (for unlinked jvm.dll)
// ══════════════════════════════════════════════════════════════════════

unsafe fn pe_has_export(base: usize, target: &str) -> bool {
    if base == 0 || (base & 0xFFF) != 0 { return false; }
    if *(base as *const u16) != 0x5A4D { return false; }
    let e_lfanew = *(base.wrapping_add(0x3C) as *const i32) as i64;
    if e_lfanew <= 0 || e_lfanew > 0x1000 { return false; }
    let pe = base.wrapping_add(e_lfanew as usize);
    if *(pe as *const u32) != 0x00004550 { return false; }
    if *(pe.wrapping_add(24) as *const u16) != 0x20B { return false; }
    let opt = pe.wrapping_add(24);
    let size_of_image = *(opt.wrapping_add(56) as *const u32) as usize;
    if size_of_image < 0x1000 || size_of_image > 0x40000000 { return false; }
    let export_rva = *(opt.wrapping_add(112) as *const u32) as usize;
    let export_size = *(opt.wrapping_add(116) as *const u32) as usize;
    if export_rva == 0 || export_size == 0 { return false; }
    if export_rva.checked_add(export_size).map_or(true, |e| e > size_of_image) { return false; }
    let exp = base.wrapping_add(export_rva);
    let num_names = *(exp.wrapping_add(20) as *const u32) as usize;
    let names_rva = *(exp.wrapping_add(32) as *const u32) as usize;
    if num_names == 0 || num_names > 0x10000 { return false; }
    if names_rva.checked_add(num_names * 4).map_or(true, |e| e > size_of_image) { return false; }
    let names_base = base.wrapping_add(names_rva);
    let tbytes = target.as_bytes();
    for i in 0..num_names {
        let name_rva = *(names_base.wrapping_add(i * 4) as *const u32) as usize;
        if name_rva == 0 || name_rva >= size_of_image { continue; }
        let name_ptr = base.wrapping_add(name_rva) as *const u8;
        let mut ok = true;
        for (j, &tb) in tbytes.iter().enumerate() {
            let b = *name_ptr.add(j);
            if b == 0 || b != tb { ok = false; break; }
        }
        if ok && *name_ptr.add(tbytes.len()) == 0 { return true; }
    }
    false
}

unsafe fn find_jvm_by_pe_scan() -> Option<(usize, usize)> {
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_IMAGE: u32 = 0x1000000;
    let mut curr: usize = 0;
    let mut last_probe: usize = 0;
    while curr < 0x0000_7FFF_FFFE_0000 {
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let q = VirtualQuery(curr as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        if q == 0 || mbi.RegionSize == 0 { break; }
        if mbi.State == MEM_COMMIT && mbi.Type == MEM_IMAGE {
            let probe = mbi.AllocationBase as usize;
            if probe != 0 && probe != last_probe && (probe & 0xFFF) == 0 {
                last_probe = probe;
                if pe_has_export(probe, "JNI_CreateJavaVM") {
                    let opt = probe + (*(probe.wrapping_add(0x3C) as *const i32) as usize) + 24;
                    let soi = *(opt.wrapping_add(56) as *const u32) as usize;
                    return Some((probe, soi));
                }
            }
        }
        curr = match curr.checked_add(mbi.RegionSize) {
            Some(next) if next > curr => next,
            _ => break,
        };
    }
    None
}

// ══════════════════════════════════════════════════════════════════════
// JavaVM Scanner
// ══════════════════════════════════════════════════════════════════════

unsafe fn scan_for_javavm(jvm_base: usize, jvm_size: usize) -> JavaVM {
    let jvm_end = jvm_base + jvm_size;
    let code_end = jvm_base + 0x2400000;
    let mut candidates = Vec::new();
    let mut curr = jvm_base;
    while curr < jvm_end {
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        if VirtualQuery(curr as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
            curr += 4096; continue;
        }
        if mbi.State != 0x1000 || (mbi.Protect & 0x101) != 0 {
            curr = curr.saturating_add(mbi.RegionSize); continue;
        }
        let end = std::cmp::min(curr.saturating_add(mbi.RegionSize), jvm_end);
        let limit = end.saturating_sub(64);
        let mut p = (curr + 7) & !7;
        while p < limit {
            let e = p as *const usize;
            if *e == 0 && *e.add(1) == 0 && *e.add(2) == 0
                && *e.add(3) >= jvm_base && *e.add(3) < code_end
                && *e.add(4) >= jvm_base && *e.add(4) < code_end
                && *e.add(5) >= jvm_base && *e.add(5) < code_end
            {
                candidates.push(p);
            }
            p += 8;
        }
        curr = curr.saturating_add(mbi.RegionSize);
    }
    for &table in &candidates {
        let mut scan = jvm_base;
        while scan < jvm_end {
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            if VirtualQuery(scan as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
                scan += 4096; continue;
            }
            if mbi.State == 0x1000 && (mbi.Protect & 0x0C) != 0 {
                let end = std::cmp::min(scan.saturating_add(mbi.RegionSize), jvm_end);
                let limit = end.saturating_sub(8);
                let mut p = (scan + 7) & !7;
                while p < limit {
                    if *(p as *const usize) == table {
                        let test_vm = p as JavaVM;
                        let mut test_env: *mut c_void = ptr::null_mut();
                        let iface = *test_vm;
                        let rc = ((*iface).get_env)(test_vm, &mut test_env, JNI_VERSION_1_8);
                        if rc == 0 || rc == -2 { return test_vm; }
                    }
                    p += 8;
                }
            }
            scan = scan.saturating_add(mbi.RegionSize);
        }
    }
    ptr::null_mut()
}

// ══════════════════════════════════════════════════════════════════════
// Cached JNI IDs & Game Thread Resolution
// ══════════════════════════════════════════════════════════════════════

pub static mut G_JAVAVM: JavaVM = ptr::null_mut();
pub static mut G_RESOLVED_IDS: Option<CachedIds> = None;
pub static mut G_RESOLVED_MC: Jobject = ptr::null_mut();
static RESOLVED_FLAG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RESOLVE_TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn is_resolved() -> bool {
    RESOLVED_FLAG.load(std::sync::atomic::Ordering::Acquire)
}

#[derive(Clone, Copy)]
pub struct CachedIds {
    // Global references to classes
    pub cls_minecraft: Jclass,
    pub cls_entity: Jclass,
    pub cls_entity_living: Jclass,
    pub cls_world: Jclass,
    pub cls_list: Jclass,

    // Minecraft fields
    pub fid_mc_player: JfieldID,       // bib.h = thePlayer
    pub fid_mc_world: JfieldID,        // bib.f = theWorld

    // Entity fields (vg)
    pub fid_pos_x: JfieldID,           // vg.p = posX (double)
    pub fid_pos_y: JfieldID,           // vg.q = posY (double)
    pub fid_pos_z: JfieldID,           // vg.r = posZ (double)
    pub fid_motion_x: JfieldID,        // vg.v = motionX (double)
    pub fid_motion_y: JfieldID,        // vg.w = motionY (double)
    pub fid_motion_z: JfieldID,        // vg.x = motionZ (double)
    pub fid_yaw: JfieldID,             // vg.A = rotationYaw (float)
    pub fid_pitch: JfieldID,           // vg.B = rotationPitch (float)
    pub fid_on_ground: JfieldID,       // vg.an = onGround (boolean)
    pub fid_entity_id: JfieldID,       // vg.g = entityId (int)
    pub fid_is_dead: JfieldID,         // vg.aJ = isDead (boolean)
    pub fid_width: JfieldID,           // vg.J = width (float)
    pub fid_height: JfieldID,          // vg.K = height (float)
    pub fid_fall_distance: JfieldID,   // vg.ab = fallDistance (float)

    // EntityLivingBase fields (vp)
    pub fid_health: JfieldID,          // vp.cb = health (float)

    // World fields (amu)
    pub fid_entity_list: JfieldID,     // amu.e = loadedEntityList (List)

    // List methods
    pub mid_list_size: JmethodID,
    pub mid_list_get: JmethodID,

    // Entity methods
    pub mid_get_name: JmethodID,       // vg.h_() -> String (getName)

    // Mouse class
    pub cls_mouse: Jclass,
    pub mid_mouse_set_grabbed: JmethodID,
}

unsafe fn get_all_class_loaders(env: JNIEnv) -> Vec<Jobject> {
    let mut loaders: Vec<Jobject> = Vec::new();

    let mut add_loader = |loader: Jobject, desc: &str| {
        if loader.is_null() { return; }
        for &existing in &loaders {
            if jni_is_same_object(env, existing, loader) {
                return;
            }
        }
        let g = jni_new_global_ref(env, loader);
        if !g.is_null() {
            log_msg(&format!("[JNI] Discovered ClassLoader ({}): 0x{:X}", desc, g as usize));
            loaders.push(g);
        }
    };

    let cls_class = jni_find_class(env, "java/lang/Class");
    let mid_get_class_loader = if !cls_class.is_null() {
        jni_get_method_id(env, cls_class, "getClassLoader", "()Ljava/lang/ClassLoader;")
    } else {
        ptr::null_mut()
    };

    // 1. Check known entrypoint classes directly (ru.meproject.Main, ru.rustme.*)
    for class_name in &[
        "ru/meproject/Main",
        "ru/rustme/client/Main",
        "net/minecraft/client/main/Main",
        "net/minecraft/launchwrapper/Launch",
    ] {
        let c = jni_find_class(env, class_name);
        if !c.is_null() {
            if !mid_get_class_loader.is_null() {
                let cl = jni_call_object(env, c, mid_get_class_loader, &[]);
                if !cl.is_null() {
                    add_loader(cl, &format!("entry class '{}' getClassLoader", class_name));
                    jni_delete_local_ref(env, cl);
                }
            }
            if *class_name == "net/minecraft/launchwrapper/Launch" {
                let fid_cl = jni_get_static_field_id(env, c, "classLoader", "Lnet/minecraft/launchwrapper/LaunchClassLoader;");
                if !fid_cl.is_null() {
                    let cl = jni_get_static_object_field(env, c, fid_cl);
                    if !cl.is_null() {
                        add_loader(cl, "Launch.classLoader");
                        jni_delete_local_ref(env, cl);
                    }
                }
            }
            jni_delete_local_ref(env, c);
        }
    }

    // 2. Thread.getAllStackTraces() -> Set<Thread> -> iterate all threads
    let thread_cls = jni_find_class(env, "java/lang/Thread");
    if !thread_cls.is_null() {
        let mid_get_cl = jni_get_method_id(env, thread_cls, "getContextClassLoader", "()Ljava/lang/ClassLoader;");
        let mid_get_name = jni_get_method_id(env, thread_cls, "getName", "()Ljava/lang/String;");
        let mid_get_all_stacks = jni_get_static_method_id(env, thread_cls, "getAllStackTraces", "()Ljava/util/Map;");

        if !mid_get_all_stacks.is_null() {
            let map = jni_call_static_object(env, thread_cls, mid_get_all_stacks, &[]);
            if !map.is_null() {
                let map_cls = jni_find_class(env, "java/util/Map");
                if !map_cls.is_null() {
                    let mid_key_set = jni_get_method_id(env, map_cls, "keySet", "()Ljava/util/Set;");
                    if !mid_key_set.is_null() {
                        let set = jni_call_object(env, map, mid_key_set, &[]);
                        if !set.is_null() {
                            let set_cls = jni_find_class(env, "java/util/Set");
                            if !set_cls.is_null() {
                                let mid_to_array = jni_get_method_id(env, set_cls, "toArray", "()[Ljava/lang/Object;");
                                if !mid_to_array.is_null() {
                                    let arr = jni_call_object(env, set, mid_to_array, &[]);
                                    if !arr.is_null() {
                                        let len = jni_get_array_length(env, arr);
                                        for i in 0..len {
                                            let th = jni_get_object_array_element(env, arr, i);
                                            if th.is_null() { continue; }

                                            let mut th_name = String::new();
                                            if !mid_get_name.is_null() {
                                                let name_j = jni_call_object(env, th, mid_get_name, &[]);
                                                if !name_j.is_null() {
                                                    th_name = jni_get_string_utf(env, name_j).unwrap_or_default();
                                                    jni_delete_local_ref(env, name_j);
                                                }
                                            }

                                            // Thread contextClassLoader
                                            if !mid_get_cl.is_null() {
                                                let cl = jni_call_object(env, th, mid_get_cl, &[]);
                                                if !cl.is_null() {
                                                    add_loader(cl, &format!("thread '{}' contextClassLoader", th_name));
                                                    jni_delete_local_ref(env, cl);
                                                }
                                            }

                                            // Thread object classloader
                                            let obj_cls = jni_find_class(env, "java/lang/Object");
                                            if !obj_cls.is_null() {
                                                let mid_get_class = jni_get_method_id(env, obj_cls, "getClass", "()Ljava/lang/Class;");
                                                if !mid_get_class.is_null() && !mid_get_class_loader.is_null() {
                                                    let th_c = jni_call_object(env, th, mid_get_class, &[]);
                                                    if !th_c.is_null() {
                                                        let th_c_loader = jni_call_object(env, th_c, mid_get_class_loader, &[]);
                                                        if !th_c_loader.is_null() {
                                                            add_loader(th_c_loader, &format!("thread '{}' class ClassLoader", th_name));
                                                            jni_delete_local_ref(env, th_c_loader);
                                                        }
                                                        jni_delete_local_ref(env, th_c);
                                                    }
                                                }
                                                jni_delete_local_ref(env, obj_cls);
                                            }

                                            jni_delete_local_ref(env, th);
                                        }
                                        jni_delete_local_ref(env, arr);
                                    }
                                }
                                jni_delete_local_ref(env, set_cls);
                            }
                            jni_delete_local_ref(env, set);
                        }
                    }
                    jni_delete_local_ref(env, map_cls);
                }
                jni_delete_local_ref(env, map);
            }
        }

        // ThreadGroup fallback
        let tg_cls = jni_find_class(env, "java/lang/ThreadGroup");
        if !tg_cls.is_null() {
            let mid_cur = jni_get_static_method_id(env, thread_cls, "currentThread", "()Ljava/lang/Thread;");
            let mid_get_tg = jni_get_method_id(env, thread_cls, "getThreadGroup", "()Ljava/lang/ThreadGroup;");
            let mid_get_parent = jni_get_method_id(env, tg_cls, "getParent", "()Ljava/lang/ThreadGroup;");
            let mid_active_count = jni_get_method_id(env, tg_cls, "activeCount", "()I");
            let mid_enumerate = jni_get_method_id(env, tg_cls, "enumerate", "([Ljava/lang/Thread;)I");

            if !mid_cur.is_null() && !mid_get_tg.is_null() && !mid_get_parent.is_null()
                && !mid_active_count.is_null() && !mid_enumerate.is_null() {
                let cur_th = jni_call_static_object(env, thread_cls, mid_cur, &[]);
                if !cur_th.is_null() {
                    let mut grp = jni_call_object(env, cur_th, mid_get_tg, &[]);
                    while !grp.is_null() {
                        let parent = jni_call_object(env, grp, mid_get_parent, &[]);
                        if parent.is_null() { break; }
                        jni_delete_local_ref(env, grp);
                        grp = parent;
                    }

                    if !grp.is_null() {
                        let active = jni_call_int(env, grp, mid_active_count, &[]);
                        let alloc_len = ((active * 2 + 32) as i32).max(64);
                        let threads_arr = jni_new_object_array(env, alloc_len, thread_cls, ptr::null_mut());
                        if !threads_arr.is_null() {
                            let args = [jvalue { l: threads_arr }];
                            let count = jni_call_int(env, grp, mid_enumerate, &args);
                            for i in 0..count {
                                let th = jni_get_object_array_element(env, threads_arr, i);
                                if th.is_null() { continue; }
                                if !mid_get_cl.is_null() {
                                    let cl = jni_call_object(env, th, mid_get_cl, &[]);
                                    if !cl.is_null() {
                                        add_loader(cl, "ThreadGroup enumerated thread contextClassLoader");
                                        jni_delete_local_ref(env, cl);
                                    }
                                }
                                jni_delete_local_ref(env, th);
                            }
                            jni_delete_local_ref(env, threads_arr);
                        }
                        jni_delete_local_ref(env, grp);
                    }
                    jni_delete_local_ref(env, cur_th);
                }
            }
            jni_delete_local_ref(env, tg_cls);
        }

        jni_delete_local_ref(env, thread_cls);
    }

    // 3. System ClassLoader
    let cl_cls = jni_find_class(env, "java/lang/ClassLoader");
    if !cl_cls.is_null() {
        let mid_sys = jni_get_static_method_id(env, cl_cls, "getSystemClassLoader", "()Ljava/lang/ClassLoader;");
        if !mid_sys.is_null() {
            let sys_cl = jni_call_static_object(env, cl_cls, mid_sys, &[]);
            if !sys_cl.is_null() {
                add_loader(sys_cl, "SystemClassLoader");
                jni_delete_local_ref(env, sys_cl);
            }
        }
        jni_delete_local_ref(env, cl_cls);
    }

    if !cls_class.is_null() {
        jni_delete_local_ref(env, cls_class);
    }

    log_msg(&format!("[JNI] Discovered {} total ClassLoaders", loaders.len()));
    loaders
}

unsafe fn try_find_class(env: JNIEnv, names: &[&str], loaders: &[Jobject]) -> Option<Jclass> {
    for &n in names {
        // 1. Try JNI FindClass directly
        let slash = n.replace('.', "/");
        let c = jni_find_class(env, &slash);
        if !c.is_null() {
            log_msg(&format!("[JNI] Found class '{}' via FindClass -> 0x{:X}", n, c as usize));
            return Some(c);
        }

        // 2. Try ClassLoader.loadClass across active loaders
        let cl_cls = jni_find_class(env, "java/lang/ClassLoader");
        if !cl_cls.is_null() {
            let mid_load = jni_get_method_id(env, cl_cls, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;");
            if !mid_load.is_null() {
                let dot_name = n.replace('/', ".");
                let jstr = jni_new_string_utf(env, &dot_name);
                if !jstr.is_null() {
                    let args = [jvalue { l: jstr }];
                    for (i, &loader) in loaders.iter().enumerate() {
                        let loaded = jni_call_object(env, loader, mid_load, &args);
                        if !loaded.is_null() {
                            log_msg(&format!("[JNI] Found class '{}' via Loader #{} -> 0x{:X}", n, i, loaded as usize));
                            jni_delete_local_ref(env, jstr);
                            jni_delete_local_ref(env, cl_cls);
                            return Some(loaded);
                        }
                    }
                    jni_delete_local_ref(env, jstr);
                }
            }
            jni_delete_local_ref(env, cl_cls);
        }
    }
    None
}

impl CachedIds {
    pub unsafe fn resolve(env: JNIEnv) -> Option<Self> {
        let loaders = get_all_class_loaders(env);

        let cls_mc = try_find_class(env, &["bib", "net.minecraft.client.Minecraft"], &loaders);
        let cls_ent = try_find_class(env, &["vg", "net.minecraft.entity.Entity"], &loaders);
        let cls_elb = try_find_class(env, &["vp", "net.minecraft.entity.EntityLivingBase"], &loaders);
        let cls_world = try_find_class(env, &["amu", "net.minecraft.world.World"], &loaders);
        let cls_list = try_find_class(env, &["java.util.List", "java/util/List"], &loaders);

        let (cls_mc, cls_ent, cls_elb, cls_world, cls_list) = match (cls_mc, cls_ent, cls_elb, cls_world, cls_list) {
            (Some(mc), Some(ent), Some(elb), Some(world), Some(list)) => (mc, ent, elb, world, list),
            _ => {
                log_msg("[JNI] Missing one or more critical classes during resolve");
                if let Some(c) = cls_mc { jni_delete_local_ref(env, c); }
                if let Some(c) = cls_ent { jni_delete_local_ref(env, c); }
                if let Some(c) = cls_elb { jni_delete_local_ref(env, c); }
                if let Some(c) = cls_world { jni_delete_local_ref(env, c); }
                if let Some(c) = cls_list { jni_delete_local_ref(env, c); }
                for l in loaders { jni_delete_global_ref(env, l); }
                return None;
            }
        };

        // Convert class references to Global References to keep them permanently valid!
        let g_mc = jni_new_global_ref(env, cls_mc);
        let g_ent = jni_new_global_ref(env, cls_ent);
        let g_elb = jni_new_global_ref(env, cls_elb);
        let g_world = jni_new_global_ref(env, cls_world);
        let g_list = jni_new_global_ref(env, cls_list);

        jni_delete_local_ref(env, cls_mc);
        jni_delete_local_ref(env, cls_ent);
        jni_delete_local_ref(env, cls_elb);
        jni_delete_local_ref(env, cls_world);
        jni_delete_local_ref(env, cls_list);

        if g_mc.is_null() || g_ent.is_null() || g_elb.is_null() || g_world.is_null() || g_list.is_null() {
            log_msg("[JNI] Failed to create global references for classes");
            if !g_mc.is_null() { jni_delete_global_ref(env, g_mc); }
            if !g_ent.is_null() { jni_delete_global_ref(env, g_ent); }
            if !g_elb.is_null() { jni_delete_global_ref(env, g_elb); }
            if !g_world.is_null() { jni_delete_global_ref(env, g_world); }
            if !g_list.is_null() { jni_delete_global_ref(env, g_list); }
            for l in loaders { jni_delete_global_ref(env, l); }
            return None;
        }

        let cleanup_all = |msg: &str| {
            log_msg(msg);
            jni_delete_global_ref(env, g_mc);
            jni_delete_global_ref(env, g_ent);
            jni_delete_global_ref(env, g_elb);
            jni_delete_global_ref(env, g_world);
            jni_delete_global_ref(env, g_list);
            for l in &loaders { jni_delete_global_ref(env, *l); }
        };

        // Minecraft fields - try obfuscated then MCP
        let fid_player = match try_field(env, g_mc, &[
            ("h", "Lbud;"),
            ("h", "Lnet/minecraft/client/entity/EntityPlayerSP;"),
            ("thePlayer", "Lbud;"),
            ("thePlayer", "Lnet/minecraft/client/entity/EntityPlayerSP;"),
        ]) {
            Some(f) => f,
            None => { cleanup_all("[JNI] Failed to resolve Minecraft thePlayer"); return None; }
        };

        let fid_world = match try_field(env, g_mc, &[
            ("f", "Lbsb;"),
            ("f", "Lnet/minecraft/client/multiplayer/WorldClient;"),
            ("theWorld", "Lbsb;"),
            ("theWorld", "Lnet/minecraft/client/multiplayer/WorldClient;"),
        ]) {
            Some(f) => f,
            None => { cleanup_all("[JNI] Failed to resolve Minecraft theWorld"); return None; }
        };

        // Entity fields
        let fid_px = match try_field(env, g_ent, &[("p", "D"), ("posX", "D")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve posX"); return None; }
        };
        let fid_py = match try_field(env, g_ent, &[("q", "D"), ("posY", "D")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve posY"); return None; }
        };
        let fid_pz = match try_field(env, g_ent, &[("r", "D"), ("posZ", "D")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve posZ"); return None; }
        };
        let fid_mx = match try_field(env, g_ent, &[("v", "D"), ("motionX", "D")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve motionX"); return None; }
        };
        let fid_my = match try_field(env, g_ent, &[("w", "D"), ("motionY", "D")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve motionY"); return None; }
        };
        let fid_mz = match try_field(env, g_ent, &[("x", "D"), ("motionZ", "D")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve motionZ"); return None; }
        };
        let fid_yaw = match try_field(env, g_ent, &[("A", "F"), ("rotationYaw", "F")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve rotationYaw"); return None; }
        };
        let fid_pitch = match try_field(env, g_ent, &[("B", "F"), ("rotationPitch", "F")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve rotationPitch"); return None; }
        };
        let fid_og = match try_field(env, g_ent, &[("an", "Z"), ("onGround", "Z")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve onGround"); return None; }
        };
        let fid_eid = match try_field(env, g_ent, &[("g", "I"), ("entityId", "I")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve entityId"); return None; }
        };
        let fid_dead = match try_field(env, g_ent, &[("aJ", "Z"), ("isDead", "Z")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve isDead"); return None; }
        };
        let fid_w = match try_field(env, g_ent, &[("J", "F"), ("width", "F")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve width"); return None; }
        };
        let fid_h = match try_field(env, g_ent, &[("K", "F"), ("height", "F")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve height"); return None; }
        };
        let fid_fd = match try_field(env, g_ent, &[("ab", "F"), ("fallDistance", "F")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve fallDistance"); return None; }
        };

        // EntityLivingBase health
        let fid_hp = match try_field(env, g_elb, &[("cb", "F"), ("health", "F")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve health"); return None; }
        };

        // World entity list
        let fid_el = match try_field(env, g_world, &[("e", "Ljava/util/List;"), ("loadedEntityList", "Ljava/util/List;")]) {
            Some(f) => f, None => { cleanup_all("[JNI] Failed to resolve entity list"); return None; }
        };

        // List methods
        let mid_size = jni_get_method_id(env, g_list, "size", "()I");
        let mid_get = jni_get_method_id(env, g_list, "get", "(I)Ljava/lang/Object;");
        if mid_size.is_null() || mid_get.is_null() {
            cleanup_all("[JNI] Failed to resolve List.size or List.get");
            return None;
        }

        // Entity.getName
        let mid_name = try_method(env, g_ent, &[("h_", "()Ljava/lang/String;"), ("getName", "()Ljava/lang/String;")]);

        // Mouse class
        let cls_mouse_local = try_find_class(env, &["org.lwjgl.input.Mouse", "org/lwjgl/input/Mouse"], &loaders);
        let (g_mouse, mid_mouse_grab) = if let Some(cm) = cls_mouse_local {
            let gm = jni_new_global_ref(env, cm);
            let mid = jni_get_static_method_id(env, gm, "setGrabbed", "(Z)V");
            jni_delete_local_ref(env, cm);
            (gm, mid)
        } else {
            (ptr::null_mut(), ptr::null_mut())
        };

        for l in loaders {
            jni_delete_global_ref(env, l);
        }

        Some(CachedIds {
            cls_minecraft: g_mc, cls_entity: g_ent,
            cls_entity_living: g_elb, cls_world: g_world, cls_list: g_list,
            fid_mc_player: fid_player, fid_mc_world: fid_world,
            fid_pos_x: fid_px, fid_pos_y: fid_py, fid_pos_z: fid_pz,
            fid_motion_x: fid_mx, fid_motion_y: fid_my, fid_motion_z: fid_mz,
            fid_yaw, fid_pitch, fid_on_ground: fid_og,
            fid_entity_id: fid_eid, fid_is_dead: fid_dead,
            fid_width: fid_w, fid_height: fid_h,
            fid_fall_distance: fid_fd,
            fid_health: fid_hp, fid_entity_list: fid_el,
            mid_list_size: mid_size, mid_list_get: mid_get,
            mid_get_name: mid_name.unwrap_or(ptr::null_mut()),
            cls_mouse: g_mouse,
            mid_mouse_set_grabbed: mid_mouse_grab,
        })
    }
}

pub unsafe fn try_resolve_on_game_thread() {
    if is_resolved() { return; }
    let vm = *(&raw const G_JAVAVM);
    if vm.is_null() { return; }

    let t = RESOLVE_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if t % 30 != 0 { return; }

    let iface = *vm;
    let mut env_ptr: *mut c_void = ptr::null_mut();
    let rc = ((*iface).get_env)(vm, &mut env_ptr, JNI_VERSION_1_8);
    if rc != 0 || env_ptr.is_null() { return; }
    let env = env_ptr as JNIEnv;

    if jni_push_local_frame(env, 128) != 0 { return; }

    let ids = match *(&raw const G_RESOLVED_IDS) {
        Some(i) => i,
        None => {
            if let Some(resolved) = CachedIds::resolve(env) {
                G_RESOLVED_IDS = Some(resolved);
                log_msg("[JNI-GAME-THREAD] All class IDs resolved on Client thread!");
                resolved
            } else {
                jni_pop_local_frame(env, ptr::null_mut());
                return;
            }
        }
    };

    let mut mc_inst: Jobject = ptr::null_mut();
    let fid_z = jni_get_static_field_id(env, ids.cls_minecraft, "z", "Lbib;");
    if !fid_z.is_null() {
        mc_inst = jni_get_static_object_field(env, ids.cls_minecraft, fid_z);
    }
    if mc_inst.is_null() {
        let fid_tm = jni_get_static_field_id(env, ids.cls_minecraft, "theMinecraft", "Lnet/minecraft/client/Minecraft;");
        if !fid_tm.is_null() {
            mc_inst = jni_get_static_object_field(env, ids.cls_minecraft, fid_tm);
        }
    }
    if mc_inst.is_null() {
        let mid_z = jni_get_static_method_id(env, ids.cls_minecraft, "z", "()Lbib;");
        if !mid_z.is_null() {
            mc_inst = jni_call_static_object(env, ids.cls_minecraft, mid_z, &[]);
        }
    }
    if mc_inst.is_null() {
        let mid_tm = jni_get_static_method_id(env, ids.cls_minecraft, "getMinecraft", "()Lnet/minecraft/client/Minecraft;");
        if !mid_tm.is_null() {
            mc_inst = jni_call_static_object(env, ids.cls_minecraft, mid_tm, &[]);
        }
    }

    if !mc_inst.is_null() {
        let g_mc = jni_new_global_ref(env, mc_inst);
        G_RESOLVED_MC = g_mc;
        RESOLVED_FLAG.store(true, std::sync::atomic::Ordering::Release);
        log_msg(&format!("[JNI-GAME-THREAD] Global Minecraft instance created: 0x{:X}", g_mc as usize));
    } else {
        log_msg("[JNI-GAME-THREAD] Classes found, waiting for Minecraft singleton initialization");
    }

    jni_pop_local_frame(env, ptr::null_mut());
}

unsafe fn try_field(env: JNIEnv, cls: Jclass, pairs: &[(&str, &str)]) -> Option<JfieldID> {
    for &(name, sig) in pairs {
        let f = jni_get_field_id(env, cls, name, sig);
        if !f.is_null() { return Some(f); }
    }
    None
}

unsafe fn try_method(env: JNIEnv, cls: Jclass, pairs: &[(&str, &str)]) -> Option<JmethodID> {
    for &(name, sig) in pairs {
        let m = jni_get_method_id(env, cls, name, sig);
        if !m.is_null() { return Some(m); }
    }
    None
}

// ══════════════════════════════════════════════════════════════════════
// Logger
// ══════════════════════════════════════════════════════════════════════

const LOG_DIR: &str = r"d:\project\rustme\dump";

pub fn log_msg(msg: &str) {
    let log_path = PathBuf::from(LOG_DIR).join("client.log");
    let _ = fs::create_dir_all(LOG_DIR);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        writeln!(f, "{}", msg).ok();
    }
}

// ══════════════════════════════════════════════════════════════════════
// JniBridge
// ══════════════════════════════════════════════════════════════════════

pub struct JniBridge {
    pub is_connected: bool,
    pub jvm_ptr: JavaVM,
    pub env_ptr: *mut c_void,
    cached_ids: Option<CachedIds>,
    mc_instance: Jobject, // cached Minecraft singleton
}

unsafe impl Send for JniBridge {}
unsafe impl Sync for JniBridge {}

impl Default for JniBridge {
    fn default() -> Self { Self::new() }
}

impl JniBridge {
    pub fn new() -> Self {
        Self {
            is_connected: false,
            jvm_ptr: ptr::null_mut(),
            env_ptr: ptr::null_mut(),
            cached_ids: None,
            mc_instance: ptr::null_mut(),
        }
    }

    pub fn env(&self) -> JNIEnv {
        self.env_ptr as JNIEnv
    }

    #[cfg(target_os = "windows")]
    pub fn attach_to_game_process(&mut self) -> Result<(), &'static str> {
        unsafe {
            install_veh();

            // Find jvm.dll
            let mut jvm_handle: isize = 0;
            let mut jvm_size: usize = 0;

            // Method 1: Toolhelp32
            let snap = CreateToolhelp32Snapshot(0x00000008, 0);
            if snap != -1 {
                let mut me: MODULEENTRY32A = std::mem::zeroed();
                me.dwSize = std::mem::size_of::<MODULEENTRY32A>() as u32;
                if Module32First(snap, &mut me) != 0 {
                    loop {
                        let name = CStr::from_ptr(me.szModule.as_ptr() as *const c_char).to_string_lossy().to_lowercase();
                        if name == "jvm.dll" {
                            jvm_handle = me.hModule;
                            jvm_size = me.modBaseSize as usize;
                            break;
                        }
                        if Module32Next(snap, &mut me) == 0 { break; }
                    }
                }
                CloseHandle(snap);
            }

            // Method 2: GetModuleHandleA
            if jvm_handle == 0 {
                let n = CString::new("jvm.dll").unwrap();
                jvm_handle = GetModuleHandleA(n.as_ptr() as *const u8);
                jvm_size = 0x2400000;
            }

            // Method 3: PE export scan (Astraea unlinks jvm.dll from PEB)
            if jvm_handle == 0 {
                if let Some((base, size)) = find_jvm_by_pe_scan() {
                    jvm_handle = base as isize;
                    jvm_size = size;
                    log_msg(&format!("[JNI] jvm.dll recovered via PE scan @ 0x{:X}", base));
                }
            }

            if jvm_handle == 0 {
                return Err("jvm.dll not found");
            }

            G_JVM_BASE = jvm_handle as usize;
            G_JVM_SIZE = if jvm_size != 0 { jvm_size } else { 0x2400000 };
            log_msg(&format!("[JNI] jvm.dll base=0x{:X} size=0x{:X}", *(&raw const G_JVM_BASE), *(&raw const G_JVM_SIZE)));

            // Get JavaVM
            let mut vm: JavaVM = ptr::null_mut();
            let fn_name = CString::new("JNI_GetCreatedJavaVMs").unwrap();
            if let Some(proc_addr) = GetProcAddress(jvm_handle, fn_name.as_ptr() as *const u8) {
                let get_vms: JNI_GetCreatedJavaVMs_fn = std::mem::transmute(proc_addr);
                let mut vms: [JavaVM; 4] = [ptr::null_mut(); 4];
                let mut count: Jsize = 0;
                let rc = get_vms(vms.as_mut_ptr(), 4, &mut count);
                if rc == JNI_OK && count > 0 && !vms[0].is_null() {
                    vm = vms[0];
                    log_msg(&format!("[JNI] GetCreatedJavaVMs: {} VMs", count));
                }
            }

            if vm.is_null() {
                vm = scan_for_javavm(jvm_handle as usize, jvm_size);
                if !vm.is_null() {
                    log_msg(&format!("[JNI] VM recovered via memory scan @ 0x{:X}", vm as usize));
                }
            }

            if vm.is_null() {
                return Err("Could not locate JavaVM");
            }

            G_JAVAVM = vm;

            // Attach thread
            let thread_name = CString::new("VibeClientThread").unwrap();
            let mut attach_args = JavaVMAttachArgs {
                version: JNI_VERSION_1_8,
                name: thread_name.as_ptr(),
                group: ptr::null_mut(),
            };

            let iface = *vm;
            let mut env: *mut c_void = ptr::null_mut();

            let rc = ((*iface).get_env)(vm, &mut env, JNI_VERSION_1_8);
            if rc != 0 || env.is_null() {
                let rc = ((*iface).attach_current_thread)(vm, &mut env, ptr::null_mut());
                if rc != 0 || env.is_null() {
                    let rc = ((*iface).attach_current_thread_as_daemon)(vm, &mut env, &mut attach_args as *mut _ as *mut c_void);
                    if rc != 0 || env.is_null() {
                        return Err("Failed to attach to JVM");
                    }
                }
            }

            // Verify env with GetEnv
            let mut env2: *mut c_void = ptr::null_mut();
            let rc2 = ((*iface).get_env)(vm, &mut env2, JNI_VERSION_1_8);
            if rc2 == 0 && !env2.is_null() && env2 as usize != env as usize {
                env = env2;
            }

            self.jvm_ptr = vm;
            self.env_ptr = env;
            self.is_connected = true;

            log_msg(&format!("[JNI] Attached! env=0x{:X}", env as usize));

            // Resolve cached IDs
            let e = self.env();
            match CachedIds::resolve(e) {
                Some(ids) => {
                    log_msg("[JNI] All class/field/method IDs resolved successfully");
                    self.cached_ids = Some(ids);
                }
                None => {
                    log_msg("[JNI] WARNING: Could not resolve some IDs — game state reading will be limited");
                }
            }

            // Cache Minecraft singleton
            if let Some(ref ids) = self.cached_ids {
                let mut local_mc: Jobject = ptr::null_mut();
                // 1. Try static field (z / theMinecraft)
                let fid_z = jni_get_static_field_id(e, ids.cls_minecraft, "z", "Lbib;");
                if !fid_z.is_null() {
                    local_mc = jni_get_static_object_field(e, ids.cls_minecraft, fid_z);
                }
                if local_mc.is_null() {
                    let fid_tm = jni_get_static_field_id(e, ids.cls_minecraft, "theMinecraft", "Lnet/minecraft/client/Minecraft;");
                    if !fid_tm.is_null() {
                        local_mc = jni_get_static_object_field(e, ids.cls_minecraft, fid_tm);
                    }
                }
                // 2. Try static methods (z() / getMinecraft())
                if local_mc.is_null() {
                    let mid = jni_get_static_method_id(e, ids.cls_minecraft, "z", "()Lbib;");
                    if !mid.is_null() {
                        local_mc = jni_call_static_object(e, ids.cls_minecraft, mid, &[]);
                    }
                }
                if local_mc.is_null() {
                    let mid2 = jni_get_static_method_id(e, ids.cls_minecraft, "getMinecraft", "()Lnet/minecraft/client/Minecraft;");
                    if !mid2.is_null() {
                        local_mc = jni_call_static_object(e, ids.cls_minecraft, mid2, &[]);
                    }
                }
                if !local_mc.is_null() {
                    let g_mc = jni_new_global_ref(e, local_mc);
                    jni_delete_local_ref(e, local_mc);
                    self.mc_instance = g_mc;
                    G_RESOLVED_MC = g_mc;
                    RESOLVED_FLAG.store(true, std::sync::atomic::Ordering::Release);
                    log_msg(&format!("[JNI] Minecraft instance resolved & global ref created: 0x{:X}", g_mc as usize));
                } else {
                    log_msg("[JNI] WARNING: Could not resolve Minecraft instance (will retry in loop)");
                }
            }

            Ok(())
        }
    }

    #[cfg(not(target_os = "windows"))]
    pub fn attach_to_game_process(&mut self) -> Result<(), &'static str> {
        Err("Windows only")
    }

    /// Read full game state from JVM into the snapshot.
    pub fn read_game_state(&mut self, snap: &mut GameSnapshot) {
        if !self.is_connected { return; }
        let env = self.env();
        if env.is_null() { return; }

        if self.cached_ids.is_none() {
            if is_resolved() {
                unsafe {
                    self.cached_ids = *(&raw const G_RESOLVED_IDS);
                    self.mc_instance = *(&raw const G_RESOLVED_MC);
                }
                log_msg("[JNI] Adopted game thread resolved IDs & MC instance");
            } else {
                return;
            }
        }

        let ids = match &self.cached_ids { Some(ids) => ids, None => return };

        unsafe {
            if self.mc_instance.is_null() {
                let mut local_mc: Jobject = ptr::null_mut();
                let fid_z = jni_get_static_field_id(env, ids.cls_minecraft, "z", "Lbib;");
                if !fid_z.is_null() {
                    local_mc = jni_get_static_object_field(env, ids.cls_minecraft, fid_z);
                }
                if local_mc.is_null() {
                    let fid_tm = jni_get_static_field_id(env, ids.cls_minecraft, "theMinecraft", "Lnet/minecraft/client/Minecraft;");
                    if !fid_tm.is_null() {
                        local_mc = jni_get_static_object_field(env, ids.cls_minecraft, fid_tm);
                    }
                }
                if local_mc.is_null() {
                    let mid = jni_get_static_method_id(env, ids.cls_minecraft, "z", "()Lbib;");
                    if !mid.is_null() {
                        local_mc = jni_call_static_object(env, ids.cls_minecraft, mid, &[]);
                    }
                }
                if local_mc.is_null() {
                    let mid2 = jni_get_static_method_id(env, ids.cls_minecraft, "getMinecraft", "()Lnet/minecraft/client/Minecraft;");
                    if !mid2.is_null() {
                        local_mc = jni_call_static_object(env, ids.cls_minecraft, mid2, &[]);
                    }
                }
                if !local_mc.is_null() {
                    let g_mc = jni_new_global_ref(env, local_mc);
                    jni_delete_local_ref(env, local_mc);
                    self.mc_instance = g_mc;
                    G_RESOLVED_MC = g_mc;
                    log_msg(&format!("[JNI] Minecraft global singleton cached in read_game_state: 0x{:X}", g_mc as usize));
                } else {
                    return;
                }
            }

            if jni_push_local_frame(env, 512) != 0 { return; }

            let mc = self.mc_instance;
            if mc.is_null() {
                jni_pop_local_frame(env, ptr::null_mut());
                return;
            }

            snap.timestamp_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            // Get thePlayer
            let player = jni_get_object_field(env, mc, ids.fid_mc_player);
            if player.is_null() {
                snap.local_player.entity.is_alive = false;
                snap.local_player.entity.is_dead = true;
                jni_pop_local_frame(env, ptr::null_mut());
                return;
            }

            let hp = jni_get_float_field(env, player, ids.fid_health);
            let dead = jni_get_boolean_field(env, player, ids.fid_is_dead);
            let eid = jni_get_int_field(env, player, ids.fid_entity_id) as u32;

            if hp <= 0.0 || dead {
                snap.local_player.entity.id = eid;
                snap.local_player.entity.health = hp.max(0.0);
                snap.local_player.entity.is_alive = false;
                snap.local_player.entity.is_dead = true;
                jni_delete_local_ref(env, player);
                jni_pop_local_frame(env, ptr::null_mut());
                return;
            }

            // Read local player data
            let px = jni_get_double_field(env, player, ids.fid_pos_x);
            let py = jni_get_double_field(env, player, ids.fid_pos_y);
            let pz = jni_get_double_field(env, player, ids.fid_pos_z);
            let mx = jni_get_double_field(env, player, ids.fid_motion_x);
            let my = jni_get_double_field(env, player, ids.fid_motion_y);
            let mz = jni_get_double_field(env, player, ids.fid_motion_z);
            let yaw = jni_get_float_field(env, player, ids.fid_yaw);
            let pitch = jni_get_float_field(env, player, ids.fid_pitch);
            let on_ground = jni_get_boolean_field(env, player, ids.fid_on_ground);
            let hp = jni_get_float_field(env, player, ids.fid_health);
            let fall_dist = jni_get_float_field(env, player, ids.fid_fall_distance);
            let eid = jni_get_int_field(env, player, ids.fid_entity_id) as u32;

            let pos = Vector3::new(px, py, pz);
            snap.local_player.entity.id = eid;
            snap.local_player.entity.position = pos;
            snap.local_player.entity.velocity = Vector3::new(mx, my, mz);
            snap.local_player.entity.rotation = Rotation::new(pitch, yaw);
            snap.local_player.entity.health = hp;
            snap.local_player.entity.is_alive = hp > 0.0;
            snap.local_player.entity.is_dead = hp <= 0.0;
            snap.local_player.entity.bounding_box = BoundingBox::from_center_and_size(pos, 0.6, 1.8);
            snap.local_player.is_grounded = on_ground;
            snap.local_player.fall_distance = fall_dist;

            // Camera from player
            snap.camera.position = Vector3::new(px, py + 1.62, pz); // eye height
            snap.camera.rotation = Rotation::new(pitch, yaw);

            // Read entity list from world
            let world = jni_get_object_field(env, mc, ids.fid_mc_world);
            if !world.is_null() {
                let entity_list = jni_get_object_field(env, world, ids.fid_entity_list);
                if !entity_list.is_null() {
                    let count = jni_call_int(env, entity_list, ids.mid_list_size, &[]);
                    snap.entities.clear();

                    let max_entities = count.min(256); // cap to avoid lag
                    for i in 0..max_entities {
                        let args = [jvalue { i }];
                        let ent = jni_call_object(env, entity_list, ids.mid_list_get, &args);
                        if ent.is_null() { continue; }

                        let e_id = jni_get_int_field(env, ent, ids.fid_entity_id) as u32;
                        if e_id == eid { jni_delete_local_ref(env, ent); continue; } // skip self

                        let ex = jni_get_double_field(env, ent, ids.fid_pos_x);
                        let ey = jni_get_double_field(env, ent, ids.fid_pos_y);
                        let ez = jni_get_double_field(env, ent, ids.fid_pos_z);
                        let dead = jni_get_boolean_field(env, ent, ids.fid_is_dead);
                        let e_hp = jni_get_float_field(env, ent, ids.fid_health);
                        let e_yaw = jni_get_float_field(env, ent, ids.fid_yaw);
                        let e_pitch = jni_get_float_field(env, ent, ids.fid_pitch);
                        let e_w = jni_get_float_field(env, ent, ids.fid_width);
                        let e_h = jni_get_float_field(env, ent, ids.fid_height);

                        let mut name = String::new();
                        if !ids.mid_get_name.is_null() {
                            let name_j = jni_call_object(env, ent, ids.mid_get_name, &[]);
                            if !name_j.is_null() {
                                name = jni_get_string_utf(env, name_j).unwrap_or_default();
                                jni_delete_local_ref(env, name_j);
                            }
                        }

                        let epos = Vector3::new(ex, ey, ez);
                        let w = if e_w > 0.0 { e_w as f64 } else { 0.6 };
                        let h = if e_h > 0.0 { e_h as f64 } else { 1.8 };
                        let bbox = BoundingBox::from_center_and_size(epos, w, h);

                        // Determine entity type by checking if it's a player
                        // (has health > 0 and width ~0.6 = Player)
                        let etype = if e_hp > 0.0 && !dead {
                            EntityType::Player
                        } else if dead {
                            EntityType::Corpse
                        } else {
                            EntityType::Unknown
                        };

                        snap.entities.push(Entity {
                            id: e_id,
                            entity_type: etype,
                            name,
                            position: epos,
                            prev_position: epos,
                            velocity: Vector3::ZERO,
                            rotation: Rotation::new(e_pitch, e_yaw),
                            health: e_hp,
                            max_health: 20.0,
                            is_alive: !dead && e_hp > 0.0,
                            is_sleeping: false,
                            is_dead: dead,
                            despawn_timer_ticks: 0,
                            bounding_box: bbox,
                            held_item: None,
                            armor_slots: [None, None, None, None, None, None, None],
                        });

                        jni_delete_local_ref(env, ent);
                    }
                    jni_delete_local_ref(env, entity_list);
                }
                jni_delete_local_ref(env, world);
            }
            jni_delete_local_ref(env, player);
            jni_pop_local_frame(env, ptr::null_mut());
        }
    }


    /// Set mouse grabbed state (false = free cursor for GUI, true = grabbed by game).
    pub fn set_mouse_grabbed(&self, grabbed: bool) {
        if !self.is_connected { return; }
        let ids = match &self.cached_ids { Some(ids) => ids, None => return };
        if ids.cls_mouse.is_null() || ids.mid_mouse_set_grabbed.is_null() { return; }
        let env = self.env();
        if env.is_null() { return; }
        unsafe {
            if jni_push_local_frame(env, 16) != 0 { return; }
            let args = [jvalue { z: if grabbed { 1 } else { 0 } }];
            jni_call_static_void(env, ids.cls_mouse, ids.mid_mouse_set_grabbed, &args);
            jni_pop_local_frame(env, ptr::null_mut());
        }
    }

    /// Set player rotation (yaw / pitch).
    pub fn set_player_rotation(&self, yaw: f32, pitch: f32) {
        if !self.is_connected { return; }
        let ids = match &self.cached_ids { Some(ids) => ids, None => return };
        let env = self.env();
        unsafe {
            if jni_push_local_frame(env, 16) != 0 { return; }
            let mc = self.mc_instance;
            if !mc.is_null() {
                let player = jni_get_object_field(env, mc, ids.fid_mc_player);
                if !player.is_null() {
                    jni_set_float_field(env, player, ids.fid_yaw, yaw);
                    jni_set_float_field(env, player, ids.fid_pitch, pitch);
                    jni_delete_local_ref(env, player);
                }
            }
            jni_pop_local_frame(env, ptr::null_mut());
        }
    }

    /// Set player onGround flag.
    pub fn set_on_ground(&self, on_ground: bool) {
        if !self.is_connected { return; }
        let ids = match &self.cached_ids { Some(ids) => ids, None => return };
        let env = self.env();
        unsafe {
            if jni_push_local_frame(env, 16) != 0 { return; }
            let mc = self.mc_instance;
            if !mc.is_null() {
                let player = jni_get_object_field(env, mc, ids.fid_mc_player);
                if !player.is_null() {
                    jni_set_boolean_field(env, player, ids.fid_on_ground, on_ground);
                    jni_delete_local_ref(env, player);
                }
            }
            jni_pop_local_frame(env, ptr::null_mut());
        }
    }

    /// Set player motion.
    pub fn set_player_motion(&self, mx: f64, my: f64, mz: f64) {
        if !self.is_connected { return; }
        let ids = match &self.cached_ids { Some(ids) => ids, None => return };
        let env = self.env();
        unsafe {
            if jni_push_local_frame(env, 16) != 0 { return; }
            let mc = self.mc_instance;
            if !mc.is_null() {
                let player = jni_get_object_field(env, mc, ids.fid_mc_player);
                if !player.is_null() {
                    jni_set_double_field(env, player, ids.fid_motion_x, mx);
                    jni_set_double_field(env, player, ids.fid_motion_y, my);
                    jni_set_double_field(env, player, ids.fid_motion_z, mz);
                    jni_delete_local_ref(env, player);
                }
            }
            jni_pop_local_frame(env, ptr::null_mut());
        }
    }

    pub fn detach(&mut self) {
        if self.is_connected && !self.jvm_ptr.is_null() {
            unsafe {
                let vm_table = *self.jvm_ptr;
                ((*vm_table).detach_current_thread)(self.jvm_ptr);
            }
            self.is_connected = false;
            self.env_ptr = ptr::null_mut();
        }
    }
}

impl Drop for JniBridge {
    fn drop(&mut self) {
        self.detach();
    }
}

/// Unlocks and shows OS cursor when ClickGUI is open, clips/hides when closed.
pub unsafe fn set_cursor_unlocked(unlocked: bool) {
    extern "system" {
        fn ClipCursor(lpRect: *const c_void) -> i32;
        fn ShowCursor(bShow: i32) -> i32;
    }

    if unlocked {
        ClipCursor(ptr::null());
        ShowCursor(1);
    } else {
        ShowCursor(0);
    }

    // Attempt to set GLFW cursor mode if glfw.dll is loaded
    let mut glfw_mod = GetModuleHandleA(b"glfw.dll\0".as_ptr());
    if glfw_mod == 0 {
        glfw_mod = GetModuleHandleA(b"glfw3.dll\0".as_ptr());
    }
    if glfw_mod != 0 {
        if let (Some(get_ctx), Some(set_mode)) = (
            GetProcAddress(glfw_mod, b"glfwGetCurrentContext\0".as_ptr()),
            GetProcAddress(glfw_mod, b"glfwSetInputMode\0".as_ptr()),
        ) {
            let get_context_fn: extern "C" fn() -> usize = std::mem::transmute(get_ctx);
            let set_input_mode_fn: extern "C" fn(usize, i32, i32) = std::mem::transmute(set_mode);
            let win = get_context_fn();
            if win != 0 {
                const GLFW_CURSOR: i32 = 0x00033001;
                const GLFW_CURSOR_NORMAL: i32 = 0x00034001;
                const GLFW_CURSOR_DISABLED: i32 = 0x00034003;
                let mode = if unlocked { GLFW_CURSOR_NORMAL } else { GLFW_CURSOR_DISABLED };
                set_input_mode_fn(win, GLFW_CURSOR, mode);
            }
        }
    }
}
