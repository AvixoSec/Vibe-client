#![allow(non_snake_case, non_camel_case_types, clippy::missing_safety_doc)]

//! Runtime class dumper v7 for RustMe.
//!
//! Features:
//! 1. Robust PE memory scanner with VirtualQuery protection to find JavaVM even when hooked by anticheat.
//! 2. Proper JavaVMAttachArgs thread attachment for OpenJDK 21.
//! 3. Probing of all JVMTI specification versions (1.0 .. 21).
//! 4. JNI exception-safe reflection pipeline:
//!    - Thread.getAllStackTraces() discovery of all active threads and ClassLoaders.
//!    - Targeted inspection of the "Client thread" ClassLoader.
//!    - Reflection on LaunchClassLoader's resourceCache (Map<String, byte[]>) and cachedClasses.
//!    - ClassLoader.loadClass() probing with MCP 1.12.2 obfuscated names + RustMe package names.
//!    - Bytecode extraction via getResourceAsStream and readAllBytes.
//!    - Complete summary and package reporting.

use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::ptr;

// ══════════════════════════════════════════════════════════════════════
// JNI Type Definitions
// ══════════════════════════════════════════════════════════════════════

type JBoolean = u8;
type JByte = i8;
type JInt = i32;
type JLong = i64;
type JSize = JInt;

type JObject = *mut c_void;
type JClass = JObject;
type JMethodID = *mut c_void;
type JFieldID = *mut c_void;
type JString = JObject;
type JArray = JObject;
type JObjectArray = JArray;
type JByteArray = JArray;

pub type JavaVM = *mut *const JNIInvokeInterface;
pub type JNIEnv = *mut *const c_void;

#[repr(C)]
#[derive(Copy, Clone)]
pub union jvalue {
    pub z: JBoolean,
    pub b: JByte,
    pub c: u16,
    pub s: i16,
    pub i: JInt,
    pub j: JLong,
    pub f: f32,
    pub d: f64,
    pub l: JObject,
}

#[repr(C)]
pub struct JNIInvokeInterface {
    pub reserved0: *mut c_void,
    pub reserved1: *mut c_void,
    pub reserved2: *mut c_void,
    pub DestroyJavaVM: unsafe extern "system" fn(JavaVM) -> JInt,
    pub AttachCurrentThread: unsafe extern "system" fn(JavaVM, *mut *mut c_void, *mut c_void) -> JInt,
    pub DetachCurrentThread: unsafe extern "system" fn(JavaVM) -> JInt,
    pub GetEnv: unsafe extern "system" fn(JavaVM, *mut *mut c_void, JInt) -> JInt,
    pub AttachCurrentThreadAsDaemon: unsafe extern "system" fn(JavaVM, *mut *mut c_void, *mut c_void) -> JInt,
}

// JNI function table offsets (JNI specification, 0-indexed)
const JNI_GET_VERSION: usize = 4;
const JNI_FIND_CLASS: usize = 6;
const JNI_EXCEPTION_OCCURRED: usize = 15;
const JNI_EXCEPTION_CLEAR: usize = 17;
const JNI_EXCEPTION_CHECK: usize = 228;
const JNI_NEW_GLOBAL_REF: usize = 21;
const JNI_DELETE_GLOBAL_REF: usize = 22;
const JNI_DELETE_LOCAL_REF: usize = 23;
const JNI_GET_OBJECT_CLASS: usize = 31;
const JNI_GET_METHOD_ID: usize = 33;
const JNI_CALL_OBJECT_METHOD_A: usize = 36;
const JNI_CALL_BOOLEAN_METHOD_A: usize = 39;
const JNI_CALL_INT_METHOD_A: usize = 51;
const JNI_CALL_VOID_METHOD_A: usize = 63;
const JNI_GET_FIELD_ID: usize = 94;
const JNI_GET_OBJECT_FIELD: usize = 95;
const JNI_GET_STATIC_METHOD_ID: usize = 113;
const JNI_CALL_STATIC_OBJECT_METHOD_A: usize = 116;
const JNI_CALL_STATIC_BOOLEAN_METHOD_A: usize = 119;
const JNI_CALL_STATIC_INT_METHOD_A: usize = 122;
const JNI_GET_STATIC_FIELD_ID: usize = 144;
const JNI_GET_STATIC_OBJECT_FIELD: usize = 145;
const JNI_NEW_STRING_UTF: usize = 167;
const JNI_GET_STRING_UTF_CHARS: usize = 169;
const JNI_RELEASE_STRING_UTF_CHARS: usize = 170;
const JNI_GET_ARRAY_LENGTH: usize = 171;
const JNI_GET_OBJECT_ARRAY_ELEMENT: usize = 173;
const JNI_GET_BYTE_ARRAY_REGION: usize = 200;

const JNI_OK: JInt = 0;

// JVMTI function table offsets
const JVMTI_DEALLOCATE: usize = 47;
const JVMTI_GET_LOADED_CLASSES: usize = 78;
const JVMTI_GET_CLASS_SIGNATURE: usize = 48;
const JVMTI_ADD_CAPABILITIES: usize = 142;

type JvmtiEnv = *mut *const c_void;

#[repr(C)]
#[derive(Default)]
struct JvmtiCapabilities {
    bits: [u32; 4],
}

const DUMP_DIR: &str = r"d:\project\rustme\dump";

// ══════════════════════════════════════════════════════════════════════
// Crash-proof JNI call gate
//
// Astraea hands injected threads a booby-trapped env / hooked vtable: the
// first call through it AVs (see dump/dumper.log, GetVersion @ jvm+0x3FEFA6).
// Rust cannot __try/__except, so every JNI call goes through this gate:
// before invoking, the current RSP is recorded; the VEH handler (installed in
// DllMain) redirects any faulting call to `vibe_guard_abort`, which "returns"
// into the gate with RAX=0. The call site sees None and moves on. A trapped
// slot can no longer kill the host process.
// ══════════════════════════════════════════════════════════════════════

static mut G_GUARD_ON: bool = false;
static mut G_GUARD_RSP: usize = 0;
static mut G_GUARD_FAULTED: bool = false;
// Snapshot of the Windows x64 callee-saved registers taken before each gated
// call. When the gate abandons a faulting JNI frame, that frame's register
// pushes are never popped — without this snapshot the caller would continue
// with clobbered RBX/RBP/RDI/RSI/R12-R15 (observed: dumper_main's `vm`
// turned to garbage after the first trapped call).
static mut G_CALSAVE: [usize; 8] = [0; 8]; // rbx, rbp, rdi, rsi, r12..r15

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

#[inline(always)]
fn cur_rsp() -> usize {
    let r: usize;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) r, options(nomem, nostack, preserves_flags)) };
    r
}

unsafe fn guarded0<R: Copy>(f: usize) -> Option<R> {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_ON = true;
    let fp: extern "C" fn() -> R = std::mem::transmute(f);
    let r = fp();
    G_GUARD_ON = false;
    if G_GUARD_FAULTED { None } else { Some(r) }
}
unsafe fn guarded1<R: Copy>(f: usize, a0: usize) -> Option<R> {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
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
    G_GUARD_ON = true;
    let fp: extern "C" fn(usize, usize, usize, usize) -> R = std::mem::transmute(f);
    let r = fp(a0, a1, a2, a3);
    G_GUARD_ON = false;
    if G_GUARD_FAULTED { None } else { Some(r) }
}
unsafe fn guarded5<R: Copy>(f: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> Option<R> {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_ON = true;
    let fp: extern "C" fn(usize, usize, usize, usize, usize) -> R = std::mem::transmute(f);
    let r = fp(a0, a1, a2, a3, a4);
    G_GUARD_ON = false;
    if G_GUARD_FAULTED { None } else { Some(r) }
}

macro_rules! jni_raw {
    ($env:expr, $offset:expr) => {{
        let table = *$env;
        *(table as *const *const c_void).add($offset) as usize
    }};
}

macro_rules! jni_fn {
    ($env:expr, $offset:expr, $sig:ty) => {{
        let table = *$env;
        let fn_ptr = *(table as *const *const c_void).add($offset);
        std::mem::transmute::<*const c_void, $sig>(fn_ptr)
    }};
}

static mut G_FAULT_LOGS: u32 = 0;

fn log_guard(site: &str) {
    unsafe {
        if G_FAULT_LOGS < 50 {
            G_FAULT_LOGS += 1;
            log(&format!("[GUARD] {} faulted (skipped)", site));
        }
    }
}

// jvm.dll image bounds, set by dumper_main. Used to tell hooked (foreign)
// vtable entries from genuine in-image ones.
static mut G_JVM_BASE: usize = 0;
static mut G_JVM_SIZE: usize = 0;

unsafe fn read_mem_guarded(addr: usize, buf: &mut [u8]) -> bool {
    save_calsave();
    G_GUARD_RSP = cur_rsp();
    G_GUARD_FAULTED = false;
    G_GUARD_ON = true;
    std::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), buf.len());
    G_GUARD_ON = false;
    !G_GUARD_FAULTED
}

/// Astraea replaces some JNIEnv slots with gate stubs that silently no-op in
/// the "closed" state. Each stub embeds the genuine function address:
///   ... 49 BA <imm64> 41 FF E2   (movabs r10, imm; jmp r10)
/// If the table entry points outside jvm.dll, recover the original from the
/// stub; otherwise return the entry unchanged.
unsafe fn effective_fn(env: JNIEnv, slot: usize) -> usize {
    let cur = jni_raw!(env, slot);
    let (base, size) = (G_JVM_BASE, G_JVM_SIZE);
    if base == 0 || size == 0 || (cur >= base && cur < base.wrapping_add(size)) {
        return cur;
    }
    let mut b = [0u8; 128];
    if !read_mem_guarded(cur, &mut b) {
        return cur;
    }
    let mut i = 0usize;
    while i + 13 <= b.len() {
        if b[i] == 0x49 && b[i + 1] == 0xBA {
            let t = u64::from_le_bytes(b[i + 2..i + 10].try_into().unwrap()) as usize;
            if b[i + 10] == 0x41 && b[i + 11] == 0xFF && b[i + 12] == 0xE2 {
                if t >= base && t < base.wrapping_add(size) {
                    log(&format!("[GATE] slot {} hooked (stub 0x{:X}), original recovered @ jvm+0x{:X}", slot, cur, t - base));
                    return t;
                }
            }
        }
        i += 1;
    }
    log(&format!("[GATE] slot {} hooked (stub 0x{:X}), original NOT recovered", slot, cur));
    cur
}

unsafe fn jni_clear_exception(env: JNIEnv) -> bool {
    let f_check = jni_raw!(env, JNI_EXCEPTION_CHECK);
    let f_clear = jni_raw!(env, JNI_EXCEPTION_CLEAR);
    let had = match guarded1::<JBoolean>(f_check, env as usize) {
        Some(v) => v != 0,
        None => { log_guard("ExceptionCheck"); false }
    };
    if had {
        let _ = guarded0::<()>(f_clear);
    }
    had
}

unsafe fn jni_find_class(env: JNIEnv, name: &str) -> JClass {
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let f = jni_raw!(env, JNI_FIND_CLASS);
    let res = match guarded2::<JClass>(f, env as usize, cname.as_ptr() as usize) {
        Some(v) => v,
        None => { log_guard(&format!("FindClass({})", name)); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_get_method_id(env: JNIEnv, cls: JClass, name: &str, sig: &str) -> JMethodID {
    if cls.is_null() { return ptr::null_mut(); }
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let csig = match CString::new(sig) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let f = jni_raw!(env, JNI_GET_METHOD_ID);
    let res = match guarded4::<JMethodID>(f, env as usize, cls as usize, cname.as_ptr() as usize, csig.as_ptr() as usize) {
        Some(v) => v,
        None => { log_guard(&format!("GetMethodID({}{})", name, sig)); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_get_static_method_id(env: JNIEnv, cls: JClass, name: &str, sig: &str) -> JMethodID {
    if cls.is_null() { return ptr::null_mut(); }
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let csig = match CString::new(sig) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let f = jni_raw!(env, JNI_GET_STATIC_METHOD_ID);
    let res = match guarded4::<JMethodID>(f, env as usize, cls as usize, cname.as_ptr() as usize, csig.as_ptr() as usize) {
        Some(v) => v,
        None => { log_guard(&format!("GetStaticMethodID({}{})", name, sig)); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_get_field_id(env: JNIEnv, cls: JClass, name: &str, sig: &str) -> JFieldID {
    if cls.is_null() { return ptr::null_mut(); }
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let csig = match CString::new(sig) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let f = jni_raw!(env, JNI_GET_FIELD_ID);
    let res = match guarded4::<JFieldID>(f, env as usize, cls as usize, cname.as_ptr() as usize, csig.as_ptr() as usize) {
        Some(v) => v,
        None => { log_guard(&format!("GetFieldID({}{})", name, sig)); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_get_object_field(env: JNIEnv, obj: JObject, fid: JFieldID) -> JObject {
    if obj.is_null() || fid.is_null() { return ptr::null_mut(); }
    let f = jni_raw!(env, JNI_GET_OBJECT_FIELD);
    let res = match guarded3::<JObject>(f, env as usize, obj as usize, fid as usize) {
        Some(v) => v,
        None => { log_guard("GetObjectField"); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_call_object_method_a(env: JNIEnv, obj: JObject, mid: JMethodID, args: &[jvalue]) -> JObject {
    if obj.is_null() || mid.is_null() { return ptr::null_mut(); }
    let f = effective_fn(env, JNI_CALL_OBJECT_METHOD_A);
    let p_args = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let res = match guarded4::<JObject>(f, env as usize, obj as usize, mid as usize, p_args as usize) {
        Some(v) => v,
        None => { log_guard("CallObjectMethodA"); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_call_static_object_method_a(env: JNIEnv, cls: JClass, mid: JMethodID, args: &[jvalue]) -> JObject {
    if cls.is_null() || mid.is_null() { return ptr::null_mut(); }
    let f = jni_raw!(env, JNI_CALL_STATIC_OBJECT_METHOD_A);
    let p_args = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let res = match guarded4::<JObject>(f, env as usize, cls as usize, mid as usize, p_args as usize) {
        Some(v) => v,
        None => { log_guard("CallStaticObjectMethodA"); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_call_int_method_a(env: JNIEnv, obj: JObject, mid: JMethodID, args: &[jvalue]) -> JInt {
    if obj.is_null() || mid.is_null() { return -1; }
    let f = jni_raw!(env, JNI_CALL_INT_METHOD_A);
    let p_args = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let res = match guarded4::<JInt>(f, env as usize, obj as usize, mid as usize, p_args as usize) {
        Some(v) => v,
        None => { log_guard("CallIntMethodA"); -1 }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_call_boolean_method_a(env: JNIEnv, obj: JObject, mid: JMethodID, args: &[jvalue]) -> JBoolean {
    if obj.is_null() || mid.is_null() { return 0; }
    let f = jni_raw!(env, JNI_CALL_BOOLEAN_METHOD_A);
    let p_args = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    let res = match guarded4::<JBoolean>(f, env as usize, obj as usize, mid as usize, p_args as usize) {
        Some(v) => v,
        None => { log_guard("CallBooleanMethodA"); 0 }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_call_void_method_a(env: JNIEnv, obj: JObject, mid: JMethodID, args: &[jvalue]) {
    if obj.is_null() || mid.is_null() { return; }
    let f = jni_raw!(env, JNI_CALL_VOID_METHOD_A);
    let p_args = if args.is_empty() { ptr::null() } else { args.as_ptr() };
    if guarded4::<()>(f, env as usize, obj as usize, mid as usize, p_args as usize).is_none() {
        log_guard("CallVoidMethodA");
    }
    jni_clear_exception(env);
}

unsafe fn jni_get_array_length(env: JNIEnv, array: JArray) -> JSize {
    if array.is_null() { return 0; }
    let f = jni_raw!(env, JNI_GET_ARRAY_LENGTH);
    let res = match guarded2::<JSize>(f, env as usize, array as usize) {
        Some(v) => v,
        None => { log_guard("GetArrayLength"); 0 }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_get_object_array_element(env: JNIEnv, array: JObjectArray, index: JSize) -> JObject {
    if array.is_null() { return ptr::null_mut(); }
    let f = jni_raw!(env, JNI_GET_OBJECT_ARRAY_ELEMENT);
    let res = match guarded3::<JObject>(f, env as usize, array as usize, index as usize) {
        Some(v) => v,
        None => { log_guard("GetObjectArrayElement"); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_get_string_utf(env: JNIEnv, s: JString) -> Option<String> {
    if s.is_null() { return None; }
    let f = jni_raw!(env, JNI_GET_STRING_UTF_CHARS);
    let chars = match guarded2::<*const c_char>(f, env as usize, s as usize) {
        Some(v) => v,
        None => { log_guard("GetStringUTFChars"); return None; }
    };
    jni_clear_exception(env);
    if chars.is_null() { return None; }
    let rust_str = CStr::from_ptr(chars).to_string_lossy().to_string();
    let f_rel = jni_raw!(env, JNI_RELEASE_STRING_UTF_CHARS);
    let _ = guarded3::<()>(f_rel, env as usize, s as usize, chars as usize);
    jni_clear_exception(env);
    Some(rust_str)
}

unsafe fn jni_new_string_utf(env: JNIEnv, s: &str) -> JString {
    let cs = match CString::new(s) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let f = jni_raw!(env, JNI_NEW_STRING_UTF);
    let res = match guarded2::<JString>(f, env as usize, cs.as_ptr() as usize) {
        Some(v) => v,
        None => { log_guard("NewStringUTF"); ptr::null_mut() }
    };
    jni_clear_exception(env);
    res
}

unsafe fn jni_delete_local_ref(env: JNIEnv, obj: JObject) {
    if obj.is_null() { return; }
    let f = jni_raw!(env, JNI_DELETE_LOCAL_REF);
    if guarded2::<()>(f, env as usize, obj as usize).is_none() {
        log_guard("DeleteLocalRef");
    }
}

unsafe fn jni_get_byte_array_region(env: JNIEnv, array: JByteArray, start: JSize, len: JSize, buf: *mut JByte) {
    if array.is_null() || buf.is_null() || len <= 0 { return; }
    let f = jni_raw!(env, JNI_GET_BYTE_ARRAY_REGION);
    if guarded5::<()>(f, env as usize, array as usize, start as usize, len as usize, buf as usize).is_none() {
        log_guard("GetByteArrayRegion");
    }
    jni_clear_exception(env);
}

// ══════════════════════════════════════════════════════════════════════
// JVMTI Probing & Dumping
// ══════════════════════════════════════════════════════════════════════

unsafe fn try_jvmti_dump(vm: JavaVM, env: JNIEnv) -> bool {
    let get_env: unsafe extern "system" fn(JavaVM, *mut *mut c_void, JInt) -> JInt = {
        let iface = *vm;
        std::mem::transmute((*iface).GetEnv)
    };

    let test_versions: [(&str, JInt); 8] = [
        ("JVMTI_VERSION_1_2", 0x30010200),
        ("JVMTI_VERSION_1_1", 0x30010100),
        ("JVMTI_VERSION_1_0", 0x30010000),
        ("JVMTI_VERSION_9",   0x30090000),
        ("JVMTI_VERSION_11",  0x300B0000),
        ("JVMTI_VERSION_17",  0x30110000),
        ("JVMTI_VERSION_21",  0x30150000),
        ("JVMTI_VERSION_ANY", 0x30000000),
    ];

    let mut jvmti_ptr: *mut c_void = ptr::null_mut();
    for &(vname, vcode) in &test_versions {
        let mut test_ptr: *mut c_void = ptr::null_mut();
        let rc = get_env(vm, &mut test_ptr, vcode);
        log(&format!("[JVMTI] GetEnv({}) -> rc={}, ptr=0x{:X}", vname, rc, test_ptr as usize));
        if rc == JNI_OK && !test_ptr.is_null() && jvmti_ptr.is_null() {
            jvmti_ptr = test_ptr;
            log(&format!("[JVMTI] Successfully obtained JVMTI environment using {}", vname));
        }
    }

    if jvmti_ptr.is_null() {
        log("[JVMTI] JVMTI unavailable via GetEnv (disabled by JVM or anticheat), proceeding to JNI reflection");
        return false;
    }

    let jvmti = jvmti_ptr as JvmtiEnv;

    // Add capabilities
    let mut caps = JvmtiCapabilities::default();
    caps.bits[0] = 0xFFFFFFFF;
    caps.bits[1] = 0xFFFFFFFF;
    let add_caps_f: usize = {
        let table = *jvmti;
        *(table as *const *const c_void).add(JVMTI_ADD_CAPABILITIES) as usize
    };
    let _ = guarded2::<JInt>(add_caps_f, jvmti as usize, &caps as *const _ as usize);

    // GetLoadedClasses
    let mut class_count: JInt = 0;
    let mut classes_ptr: *mut JClass = ptr::null_mut();
    let get_loaded_f: usize = {
        let table = *jvmti;
        *(table as *const *const c_void).add(JVMTI_GET_LOADED_CLASSES) as usize
    };
    let rc = guarded3::<JInt>(get_loaded_f, jvmti as usize, &mut class_count as *mut _ as usize, &mut classes_ptr as *mut _ as usize)
        .unwrap_or(-1);
    if rc != 0 || classes_ptr.is_null() {
        log(&format!("[JVMTI] GetLoadedClasses failed: rc={}", rc));
        return false;
    }
    log(&format!("[JVMTI] Found {} loaded classes in JVMTI!", class_count));

    let dump_dir = PathBuf::from(DUMP_DIR);
    fs::create_dir_all(&dump_dir).ok();

    let get_sig_f: usize = {
        let table = *jvmti;
        *(table as *const *const c_void).add(JVMTI_GET_CLASS_SIGNATURE) as usize
    };
    let dealloc_f: usize = {
        let table = *jvmti;
        *(table as *const *const c_void).add(JVMTI_DEALLOCATE) as usize
    };

    let mut dumped = 0u32;
    let mut summary = Vec::new();

    for i in 0..class_count {
        let cls = *classes_ptr.add(i as usize);
        let mut sig_ptr: *mut c_char = ptr::null_mut();
        let mut generic_ptr: *mut c_char = ptr::null_mut();
        let rc = guarded4::<JInt>(get_sig_f, jvmti as usize, cls as usize, &mut sig_ptr as *mut _ as usize, &mut generic_ptr as *mut _ as usize).unwrap_or(-1);
        if rc != 0 || sig_ptr.is_null() {
            continue;
        }
        let sig = CStr::from_ptr(sig_ptr).to_string_lossy().to_string();
        let _ = guarded2::<JInt>(dealloc_f, jvmti as usize, sig_ptr as usize);
        if !generic_ptr.is_null() {
            let _ = guarded2::<JInt>(dealloc_f, jvmti as usize, generic_ptr as usize);
        }

        if !sig.starts_with('L') || !sig.ends_with(';') {
            continue;
        }
        let class_name = &sig[1..sig.len()-1];
        if class_name.starts_with("java/") || class_name.starts_with("jdk/") || class_name.starts_with("sun/") {
            continue;
        }

        if let Some(bytes) = dump_class_bytes_via_resource(env, cls, class_name) {
            let out_path = dump_dir.join(format!("{}.class", class_name));
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            if fs::write(&out_path, &bytes).is_ok() {
                dumped += 1;
                note_dumped();
                summary.push(format!("OK {} ({} bytes)", class_name, bytes.len()));
            }
        } else {
            summary.push(format!("SIGNATURE_ONLY {}", class_name));
        }
    }

    let _ = guarded2::<JInt>(dealloc_f, jvmti as usize, classes_ptr as usize);

    if let Ok(mut f) = fs::File::create(dump_dir.join("jvmti_summary.txt")) {
        writeln!(f, "JVMTI Dumped: {} / {}", dumped, class_count).ok();
        for s in &summary {
            writeln!(f, "{}", s).ok();
        }
    }

    log(&format!("[JVMTI] Completed dump: {} classes saved", dumped));
    true
}

// ══════════════════════════════════════════════════════════════════════
// JNI Advanced Fallback Engine
// ══════════════════════════════════════════════════════════════════════

pub unsafe fn jni_fallback_dump(env: JNIEnv) {
    log("[JNI] Starting comprehensive JNI reflection class dumper");

    let dump_dir = PathBuf::from(DUMP_DIR);
    fs::create_dir_all(&dump_dir).ok();

    let mut dumped_total = 0u32;
    let mut class_loaders = Vec::new();

    // 1. Enumerate all active threads via Thread.getAllStackTraces()
    log("[JNI] Querying Thread.getAllStackTraces()...");
    let thread_cls = jni_find_class(env, "java/lang/Thread");
    log(&format!("[JNI] thread_cls: 0x{:X}", thread_cls as usize));
    if !thread_cls.is_null() {
        let mid_get_all = jni_get_static_method_id(env, thread_cls, "getAllStackTraces", "()Ljava/util/Map;");
        log(&format!("[JNI] mid_get_all: 0x{:X}", mid_get_all as usize));
        if !mid_get_all.is_null() {
            let map_obj = jni_call_static_object_method_a(env, thread_cls, mid_get_all, &[]);
            log(&format!("[JNI] map_obj: 0x{:X}", map_obj as usize));
            if !map_obj.is_null() {
                let map_cls = jni_find_class(env, "java/util/Map");
                let mid_key_set = jni_get_method_id(env, map_cls, "keySet", "()Ljava/util/Set;");
                if !mid_key_set.is_null() {
                    let set_obj = jni_call_object_method_a(env, map_obj, mid_key_set, &[]);
                    if !set_obj.is_null() {
                        let set_cls = jni_find_class(env, "java/util/Set");
                        let mid_to_array = jni_get_method_id(env, set_cls, "toArray", "()[Ljava/lang/Object;");
                        if !mid_to_array.is_null() {
                            let arr_obj = jni_call_object_method_a(env, set_obj, mid_to_array, &[]);
                            if !arr_obj.is_null() {
                                let count = jni_get_array_length(env, arr_obj);
                                log(&format!("[JNI] Found {} active threads in JVM", count));

                                let mid_get_name = jni_get_method_id(env, thread_cls, "getName", "()Ljava/lang/String;");
                                let mid_get_cl = jni_get_method_id(env, thread_cls, "getContextClassLoader", "()Ljava/lang/ClassLoader;");

                                for i in 0..count {
                                    let th = jni_get_object_array_element(env, arr_obj, i);
                                    if th.is_null() { continue; }
                                    let name_str = if !mid_get_name.is_null() {
                                        let name_j = jni_call_object_method_a(env, th, mid_get_name, &[]);
                                        let s = jni_get_string_utf(env, name_j).unwrap_or_default();
                                        jni_delete_local_ref(env, name_j);
                                        s
                                    } else {
                                        String::new()
                                    };

                                    let cl = if !mid_get_cl.is_null() {
                                        jni_call_object_method_a(env, th, mid_get_cl, &[])
                                    } else {
                                        ptr::null_mut()
                                    };

                                    log(&format!("  [Thread #{}] '{}' -> ClassLoader: 0x{:X}", i, name_str, cl as usize));

                                    if !cl.is_null() {
                                        if !class_loaders.iter().any(|&c| c == cl) {
                                            class_loaders.push(cl);
                                        }
                                    }
                                    jni_delete_local_ref(env, th);
                                }
                                jni_delete_local_ref(env, arr_obj);
                            }
                        }
                        jni_delete_local_ref(env, set_obj);
                    }
                }
                jni_delete_local_ref(env, map_obj);
            }
        }
    }

    log(&format!("[JNI] Collected {} unique ClassLoaders across all threads", class_loaders.len()));

    // 2. Try to inspect ClassLoader internals (LaunchClassLoader.resourceCache or cachedClasses)
    for (idx, &cl) in class_loaders.iter().enumerate() {
        log(&format!("[JNI] Inspecting ClassLoader #{} (0x{:X})...", idx, cl as usize));
        let dumped_from_cl = inspect_and_dump_classloader(env, cl, &dump_dir);
        dumped_total += dumped_from_cl;
        log(&format!("[JNI] Dumped {} classes directly from ClassLoader #{}", dumped_from_cl, idx));
    }

    // 3. Probing MCP 1.12.2 and RustMe known classes across all ClassLoaders
    log("[JNI] Probing known Minecraft 1.12.2 / RustMe / OptiFine classes...");
    let probed = probe_known_classes(env, &class_loaders, &dump_dir);
    dumped_total += probed;

    log(&format!("[JNI] Class dumping finished! Total classes saved: {}", dumped_total));
}

// ══════════════════════════════════════════════════════════════════════
// Inspect ClassLoader (LaunchClassLoader / URLClassLoader)
// ══════════════════════════════════════════════════════════════════════

unsafe fn inspect_and_dump_classloader(env: JNIEnv, loader: JObject, dump_dir: &PathBuf) -> u32 {
    let mut dumped = 0u32;

    // Get Class of the ClassLoader
    let get_cls_f = jni_raw!(env, JNI_GET_OBJECT_CLASS);
    let cl_class = match guarded2::<JClass>(get_cls_f, env as usize, loader as usize) {
        Some(c) => c,
        None => { log_guard("GetObjectClass"); return 0; }
    };
    if cl_class.is_null() { return 0; }

    // Get Class.getName()
    let class_cls = jni_find_class(env, "java/lang/Class");
    let mid_get_name = jni_get_method_id(env, class_cls, "getName", "()Ljava/lang/String;");
    let cl_name_j = jni_call_object_method_a(env, cl_class, mid_get_name, &[]);
    let cl_name = jni_get_string_utf(env, cl_name_j).unwrap_or_default();
    jni_delete_local_ref(env, cl_name_j);
    log(&format!("  [ClassLoader Class] {}", cl_name));

    // Check for LaunchClassLoader.resourceCache (Map<String, byte[]>)
    // Using Class.getDeclaredField
    let mid_get_field = jni_get_method_id(env, class_cls, "getDeclaredField", "(Ljava/lang/String;)Ljava/lang/reflect/Field;");
    if !mid_get_field.is_null() {
        for &field_name in &["resourceCache", "cachedClasses", "classes"] {
            let jstr_name = jni_new_string_utf(env, field_name);
            let field_args = [jvalue { l: jstr_name }];
            let field_obj = jni_call_object_method_a(env, cl_class, mid_get_field, &field_args);
            jni_delete_local_ref(env, jstr_name);

            if !field_obj.is_null() {
                log(&format!("  [FOUND FIELD] {} in {}", field_name, cl_name));
                let field_cls = jni_find_class(env, "java/lang/reflect/Field");
                let mid_set_acc = jni_get_method_id(env, field_cls, "setAccessible", "(Z)V");
                if !mid_set_acc.is_null() {
                    let acc_args = [jvalue { z: 1 }];
                    jni_call_void_method_a(env, field_obj, mid_set_acc, &acc_args);
                }

                let mid_get_val = jni_get_method_id(env, field_cls, "get", "(Ljava/lang/Object;)Ljava/lang/Object;");
                if !mid_get_val.is_null() {
                    let get_args = [jvalue { l: loader }];
                    let val_obj = jni_call_object_method_a(env, field_obj, mid_get_val, &get_args);
                    if !val_obj.is_null() {
                        if field_name == "resourceCache" {
                            dumped += dump_map_resource_cache(env, val_obj, dump_dir);
                        } else if field_name == "cachedClasses" {
                            dumped += dump_map_cached_classes(env, val_obj, loader, dump_dir);
                        }
                        jni_delete_local_ref(env, val_obj);
                    }
                }
                jni_delete_local_ref(env, field_obj);
            }
        }
    }

    dumped
}

unsafe fn dump_map_resource_cache(env: JNIEnv, map_obj: JObject, dump_dir: &PathBuf) -> u32 {
    let map_cls = jni_find_class(env, "java/util/Map");
    let mid_entry_set = jni_get_method_id(env, map_cls, "entrySet", "()Ljava/util/Set;");
    if mid_entry_set.is_null() { return 0; }
    let set_obj = jni_call_object_method_a(env, map_obj, mid_entry_set, &[]);
    if set_obj.is_null() { return 0; }

    let set_cls = jni_find_class(env, "java/util/Set");
    let mid_to_array = jni_get_method_id(env, set_cls, "toArray", "()[Ljava/lang/Object;");
    if mid_to_array.is_null() { jni_delete_local_ref(env, set_obj); return 0; }
    let arr_obj = jni_call_object_method_a(env, set_obj, mid_to_array, &[]);
    jni_delete_local_ref(env, set_obj);
    if arr_obj.is_null() { return 0; }

    let count = jni_get_array_length(env, arr_obj);
    log(&format!("    [resourceCache] Found {} cached class entries!", count));

    let entry_cls = jni_find_class(env, "java/util/Map$Entry");
    let mid_get_key = jni_get_method_id(env, entry_cls, "getKey", "()Ljava/lang/Object;");
    let mid_get_val = jni_get_method_id(env, entry_cls, "getValue", "()Ljava/lang/Object;");

    let mut dumped = 0u32;
    for i in 0..count {
        let entry = jni_get_object_array_element(env, arr_obj, i);
        if entry.is_null() { continue; }

        let key_obj = jni_call_object_method_a(env, entry, mid_get_key, &[]);
        let val_obj = jni_call_object_method_a(env, entry, mid_get_val, &[]);

        if let Some(key_str) = jni_get_string_utf(env, key_obj) {
            if !val_obj.is_null() {
                let byte_len = jni_get_array_length(env, val_obj as JByteArray);
                if byte_len > 0 {
                    let mut buf = vec![0i8; byte_len as usize];
                    jni_get_byte_array_region(env, val_obj as JByteArray, 0, byte_len, buf.as_mut_ptr());
                    let u8_bytes: Vec<u8> = buf.into_iter().map(|b| b as u8).collect();

                    let class_path = key_str.replace('.', "/");
                    let out_path = dump_dir.join(format!("{}.class", class_path));
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).ok();
                    }
                    if fs::write(&out_path, &u8_bytes).is_ok() {
                        dumped += 1;
                        note_dumped();
                    }
                }
            }
        }
        jni_delete_local_ref(env, key_obj);
        jni_delete_local_ref(env, val_obj);
        jni_delete_local_ref(env, entry);
    }

    jni_delete_local_ref(env, arr_obj);
    log(&format!("    [resourceCache] Dumped {} classes to disk", dumped));
    dumped
}

unsafe fn dump_map_cached_classes(env: JNIEnv, map_obj: JObject, loader: JObject, dump_dir: &PathBuf) -> u32 {
    let map_cls = jni_find_class(env, "java/util/Map");
    let mid_key_set = jni_get_method_id(env, map_cls, "keySet", "()Ljava/util/Set;");
    if mid_key_set.is_null() { return 0; }
    let set_obj = jni_call_object_method_a(env, map_obj, mid_key_set, &[]);
    if set_obj.is_null() { return 0; }

    let set_cls = jni_find_class(env, "java/util/Set");
    let mid_to_array = jni_get_method_id(env, set_cls, "toArray", "()[Ljava/lang/Object;");
    if mid_to_array.is_null() { jni_delete_local_ref(env, set_obj); return 0; }
    let arr_obj = jni_call_object_method_a(env, set_obj, mid_to_array, &[]);
    jni_delete_local_ref(env, set_obj);
    if arr_obj.is_null() { return 0; }

    let count = jni_get_array_length(env, arr_obj);
    log(&format!("    [cachedClasses] Found {} loaded class entries!", count));

    let mut dumped = 0u32;
    for i in 0..count {
        let key_obj = jni_get_object_array_element(env, arr_obj, i);
        if key_obj.is_null() { continue; }
        if let Some(key_str) = jni_get_string_utf(env, key_obj) {
            let class_path = key_str.replace('.', "/");
            if !class_path.starts_with("java/") && !class_path.starts_with("jdk/") {
                if let Some(bytes) = dump_class_bytes_from_loader(env, loader, &class_path) {
                    let out_path = dump_dir.join(format!("{}.class", class_path));
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).ok();
                    }
                    if fs::write(&out_path, &bytes).is_ok() {
                        dumped += 1;
                        note_dumped();
                    }
                }
            }
        }
        jni_delete_local_ref(env, key_obj);
    }

    jni_delete_local_ref(env, arr_obj);
    log(&format!("    [cachedClasses] Dumped {} classes via getResourceAsStream", dumped));
    dumped
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

unsafe fn inspect_and_dump_class_offsets(
    env: JNIEnv,
    cls: JClass,
    class_name: &str,
    json_classes: &mut Vec<String>,
    text_report: &mut String,
) -> bool {
    if cls.is_null() { return false; }

    let class_cls = jni_find_class(env, "java/lang/Class");
    if class_cls.is_null() { return false; }

    let mid_get_fields = jni_get_method_id(env, class_cls, "getDeclaredFields", "()[Ljava/lang/reflect/Field;");
    let mid_get_methods = jni_get_method_id(env, class_cls, "getDeclaredMethods", "()[Ljava/lang/reflect/Method;");
    let mid_get_name = jni_get_method_id(env, class_cls, "getName", "()Ljava/lang/String;");

    let field_cls = jni_find_class(env, "java/lang/reflect/Field");
    let method_cls = jni_find_class(env, "java/lang/reflect/Method");

    let mid_f_get_name = if !field_cls.is_null() { jni_get_method_id(env, field_cls, "getName", "()Ljava/lang/String;") } else { ptr::null_mut() };
    let mid_f_get_type = if !field_cls.is_null() { jni_get_method_id(env, field_cls, "getType", "()Ljava/lang/Class;") } else { ptr::null_mut() };
    let mid_f_get_mods = if !field_cls.is_null() { jni_get_method_id(env, field_cls, "getModifiers", "()I") } else { ptr::null_mut() };

    let mid_m_get_name = if !method_cls.is_null() { jni_get_method_id(env, method_cls, "getName", "()Ljava/lang/String;") } else { ptr::null_mut() };
    let mid_m_get_ret = if !method_cls.is_null() { jni_get_method_id(env, method_cls, "getReturnType", "()Ljava/lang/Class;") } else { ptr::null_mut() };
    let mid_m_get_mods = if !method_cls.is_null() { jni_get_method_id(env, method_cls, "getModifiers", "()I") } else { ptr::null_mut() };

    let f_from_reflected_field = effective_fn(env, 8); // JNI FromReflectedField (slot 8)
    let f_from_reflected_method = effective_fn(env, 7); // JNI FromReflectedMethod (slot 7)
    let f_call_int = effective_fn(env, JNI_CALL_INT_METHOD_A);

    log(&format!("[OFFSETS] Discovered class: {}", class_name));
    text_report.push_str(&format!("============================================================\n"));
    text_report.push_str(&format!("CLASS: {}\n", class_name));
    text_report.push_str(&format!("============================================================\n"));

    let mut fields_json = Vec::new();

    // 1. Process Fields
    if !mid_get_fields.is_null() {
        let fields_arr = jni_call_object_method_a(env, cls, mid_get_fields, &[]);
        if !fields_arr.is_null() {
            let n_fields = jni_get_array_length(env, fields_arr);
            log(&format!("  [FIELDS] {} declared fields:", n_fields));
            text_report.push_str(&format!("--- Fields ({}) ---\n", n_fields));

            for i in 0..n_fields {
                let f_obj = jni_get_object_array_element(env, fields_arr, i);
                if f_obj.is_null() { continue; }

                let fname_j = if !mid_f_get_name.is_null() { jni_call_object_method_a(env, f_obj, mid_f_get_name, &[]) } else { ptr::null_mut() };
                let fname = jni_get_string_utf(env, fname_j).unwrap_or_default();
                jni_delete_local_ref(env, fname_j);

                let ftype_j = if !mid_f_get_type.is_null() { jni_call_object_method_a(env, f_obj, mid_f_get_type, &[]) } else { ptr::null_mut() };
                let ftype_name = if !ftype_j.is_null() && !mid_get_name.is_null() {
                    let tn_j = jni_call_object_method_a(env, ftype_j, mid_get_name, &[]);
                    let s = jni_get_string_utf(env, tn_j).unwrap_or_default();
                    jni_delete_local_ref(env, tn_j);
                    jni_delete_local_ref(env, ftype_j);
                    s
                } else {
                    "unknown".to_string()
                };

                let mods = if !mid_f_get_mods.is_null() {
                    guarded2::<JInt>(f_call_int, env as usize, f_obj as usize).unwrap_or(0)
                } else {
                    0
                };
                let is_static = (mods & 0x0008) != 0;

                let fid = guarded2::<usize>(f_from_reflected_field, env as usize, f_obj as usize).unwrap_or(0);

                log(&format!("    field: {:<24} | type: {:<32} | static: {:<5} | offset: 0x{:04X} ({})",
                    fname, ftype_name, is_static, fid, fid));
                text_report.push_str(&format!("    0x{:04X} ({:5}) | {:<5} | {:<32} | {}\n",
                    fid, fid, if is_static { "STAT" } else { "INST" }, ftype_name, fname));

                fields_json.push(format!(
                    r#"{{"name":"{}","type":"{}","is_static":{},"offset":{}}}"#,
                    escape_json(&fname), escape_json(&ftype_name), is_static, fid
                ));

                jni_delete_local_ref(env, f_obj);
            }
            jni_delete_local_ref(env, fields_arr);
        }
    }

    // 2. Process Methods
    let mut methods_json = Vec::new();
    if !mid_get_methods.is_null() {
        let methods_arr = jni_call_object_method_a(env, cls, mid_get_methods, &[]);
        if !methods_arr.is_null() {
            let n_methods = jni_get_array_length(env, methods_arr);
            text_report.push_str(&format!("--- Methods ({}) ---\n", n_methods));

            for i in 0..n_methods {
                let m_obj = jni_get_object_array_element(env, methods_arr, i);
                if m_obj.is_null() { continue; }

                let mname_j = if !mid_m_get_name.is_null() { jni_call_object_method_a(env, m_obj, mid_m_get_name, &[]) } else { ptr::null_mut() };
                let mname = jni_get_string_utf(env, mname_j).unwrap_or_default();
                jni_delete_local_ref(env, mname_j);

                let mret_j = if !mid_m_get_ret.is_null() { jni_call_object_method_a(env, m_obj, mid_m_get_ret, &[]) } else { ptr::null_mut() };
                let mret_name = if !mret_j.is_null() && !mid_get_name.is_null() {
                    let tn_j = jni_call_object_method_a(env, mret_j, mid_get_name, &[]);
                    let s = jni_get_string_utf(env, tn_j).unwrap_or_default();
                    jni_delete_local_ref(env, tn_j);
                    jni_delete_local_ref(env, mret_j);
                    s
                } else {
                    "void".to_string()
                };

                let mods = if !mid_m_get_mods.is_null() {
                    guarded2::<JInt>(f_call_int, env as usize, m_obj as usize).unwrap_or(0)
                } else {
                    0
                };
                let is_static = (mods & 0x0008) != 0;

                let mid = guarded2::<usize>(f_from_reflected_method, env as usize, m_obj as usize).unwrap_or(0);

                text_report.push_str(&format!("    0x{:016X} | {:<5} | {:<24} | {}\n",
                    mid, if is_static { "STAT" } else { "INST" }, mret_name, mname));

                methods_json.push(format!(
                    r#"{{"name":"{}","return_type":"{}","is_static":{},"method_id":"0x{:X}"}}"#,
                    escape_json(&mname), escape_json(&mret_name), is_static, mid
                ));

                jni_delete_local_ref(env, m_obj);
            }
            jni_delete_local_ref(env, methods_arr);
        }
    }

    text_report.push_str("\n");

    let cls_json = format!(
        r#"{{"class":"{}","fields":[{}],"methods":[{}]}}"#,
        escape_json(class_name),
        fields_json.join(","),
        methods_json.join(",")
    );
    json_classes.push(cls_json);

    true
}

// ══════════════════════════════════════════════════════════════════════
// Known Classes Prober (Vanilla 1.12.2 MCP + Obfuscated + RustMe)
// ══════════════════════════════════════════════════════════════════════

unsafe fn probe_known_classes(env: JNIEnv, loaders: &[JObject], dump_dir: &PathBuf) -> u32 {
    let targets = [
        // 1.12.2 Minecraft Core (MCP & Obfuscated)
        "net.minecraft.client.Minecraft", "bib",
        "net.minecraft.client.entity.EntityPlayerSP", "bud", "bnp",
        "net.minecraft.entity.Entity", "vg",
        "net.minecraft.entity.EntityLivingBase", "vp",
        "net.minecraft.client.renderer.EntityRenderer", "buq",
        "net.minecraft.world.World", "amu",
        "net.minecraft.client.multiplayer.WorldClient", "bsb",
        "net.minecraft.network.NetworkManager", "gy",
        "net.minecraft.client.gui.GuiScreen", "bje",
        "net.minecraft.client.settings.GameSettings", "bis",
        "net.minecraft.util.math.Vec3d", "bhc",
        "net.minecraft.util.math.AxisAlignedBB", "bhb",
        "net.minecraft.client.renderer.GlStateManager", "bus",
        "net.minecraft.item.ItemStack", "aip",
        "net.minecraft.entity.player.EntityPlayer", "aed",
        "net.minecraft.entity.player.InventoryPlayer", "aec",
        "net.minecraft.client.multiplayer.PlayerControllerMP", "bsa",
        "net.minecraft.network.play.client.CPacketPlayer", "le",
        "net.minecraft.network.play.client.CPacketUseEntity", "lh",
        "net.minecraft.network.Packet", "ht",
        "net.minecraft.inventory.Container", "afp",
        "net.minecraft.inventory.Slot", "agr",
        "net.minecraft.client.gui.inventory.GuiContainer", "bln",
        "net.minecraft.util.MovementInput", "bnu",
        "net.minecraft.client.entity.AbstractClientPlayer", "buc",

        // RustMe Launcher & Game Client entries
        "ru.meproject.Main",
        "ru.meproject.client.RustMeClient",
        "ru.meproject.client.RMClient",
        "ru.rustme.client.Client",
        "ru.rustme.Main",
        "ru.rustme.client.RustMe",
        "ru.rustme.client.modules.Module",
        "me.rustme.Main",
        "me.rustme.client.Client",
        "rustme.Main",
        "rustme.client.Client",

        // OptiFine
        "optifine.OptiFineClassTransformer",
        "net.optifine.Config",
        "Config",

        // Voice chat
        "concentus.OpusEncoder",
    ];

    let cl_class = jni_find_class(env, "java/lang/ClassLoader");
    let mid_load_class = jni_get_method_id(env, cl_class, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;");

    let mut dumped = 0u32;
    let mut json_classes = Vec::new();
    let mut text_report = String::new();
    text_report.push_str("============================================================\n");
    text_report.push_str("  VIBE DUMPER — RUNTIME OFFSETS AND FIELD REPORT\n");
    text_report.push_str("  Target: RustMe 1.12.2 / OpenJDK 21 x64\n");
    text_report.push_str("============================================================\n\n");

    for &name in &targets {
        let mut target_cls: JClass = ptr::null_mut();

        // 1. Try direct FindClass first
        let slash_name = name.replace('.', "/");
        let direct_cls = jni_find_class(env, &slash_name);
        if !direct_cls.is_null() {
            target_cls = direct_cls;
        }

        // 2. If not found, try ClassLoader.loadClass across active loaders
        if target_cls.is_null() && !mid_load_class.is_null() {
            let jstr_name = jni_new_string_utf(env, name);
            let load_args = [jvalue { l: jstr_name }];

            for &loader in loaders {
                let loaded_cls = jni_call_object_method_a(env, loader, mid_load_class, &load_args);
                if !loaded_cls.is_null() {
                    target_cls = loaded_cls;
                    break;
                }
            }
            jni_delete_local_ref(env, jstr_name);
        }

        if !target_cls.is_null() {
            dumped += 1;
            note_dumped();
            inspect_and_dump_class_offsets(env, target_cls, name, &mut json_classes, &mut text_report);

            // Also try saving raw bytes if possible
            if let Some(bytes) = dump_class_bytes_via_resource(env, target_cls, &slash_name) {
                let out_path = dump_dir.join(format!("{}.class", slash_name));
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent).ok();
                }
                let _ = fs::write(&out_path, &bytes);
            }

            jni_delete_local_ref(env, target_cls);
        }
    }

    let offsets_json_path = dump_dir.join("offsets.json");
    let full_json = format!("[\n{}\n]\n", json_classes.join(",\n"));
    if fs::write(&offsets_json_path, full_json).is_ok() {
        log(&format!("[OFFSETS] Successfully saved offsets JSON to {:?}", offsets_json_path));
    }

    let offsets_txt_path = dump_dir.join("offsets.txt");
    if fs::write(&offsets_txt_path, text_report).is_ok() {
        log(&format!("[OFFSETS] Successfully saved readable report to {:?}", offsets_txt_path));
    }

    dumped
}

// ══════════════════════════════════════════════════════════════════════
// Bytecode Extraction Helpers
// ══════════════════════════════════════════════════════════════════════

unsafe fn dump_class_bytes_via_resource(env: JNIEnv, cls: JClass, class_name: &str) -> Option<Vec<u8>> {
    let class_cls = jni_find_class(env, "java/lang/Class");
    let mid_get_res = jni_get_method_id(env, class_cls, "getResourceAsStream", "(Ljava/lang/String;)Ljava/io/InputStream;");
    if mid_get_res.is_null() { return None; }

    let res_path = format!("/{}.class", class_name);
    let jstr_path = jni_new_string_utf(env, &res_path);
    let args = [jvalue { l: jstr_path }];
    let stream = jni_call_object_method_a(env, cls, mid_get_res, &args);
    jni_delete_local_ref(env, jstr_path);

    if stream.is_null() { return None; }
    let bytes = read_input_stream(env, stream);
    jni_delete_local_ref(env, stream);
    bytes
}

unsafe fn dump_class_bytes_from_loader(env: JNIEnv, loader: JObject, class_name: &str) -> Option<Vec<u8>> {
    let cl_cls = jni_find_class(env, "java/lang/ClassLoader");
    let mid_get_res = jni_get_method_id(env, cl_cls, "getResourceAsStream", "(Ljava/lang/String;)Ljava/io/InputStream;");
    if mid_get_res.is_null() { return None; }

    let res_path = format!("{}.class", class_name);
    let jstr_path = jni_new_string_utf(env, &res_path);
    let args = [jvalue { l: jstr_path }];
    let stream = jni_call_object_method_a(env, loader, mid_get_res, &args);
    jni_delete_local_ref(env, jstr_path);

    if stream.is_null() { return None; }
    let bytes = read_input_stream(env, stream);
    jni_delete_local_ref(env, stream);
    bytes
}

unsafe fn read_input_stream(env: JNIEnv, stream: JObject) -> Option<Vec<u8>> {
    if stream.is_null() { return None; }

    let is_cls = jni_find_class(env, "java/io/InputStream");
    if is_cls.is_null() { return None; }

    let mid_read_all = jni_get_method_id(env, is_cls, "readAllBytes", "()[B");
    if !mid_read_all.is_null() {
        let byte_arr = jni_call_object_method_a(env, stream, mid_read_all, &[]);
        if !byte_arr.is_null() {
            let len = jni_get_array_length(env, byte_arr);
            if len > 0 {
                let mut buf = vec![0i8; len as usize];
                jni_get_byte_array_region(env, byte_arr, 0, len, buf.as_mut_ptr());
                jni_delete_local_ref(env, byte_arr);
                return Some(buf.into_iter().map(|b| b as u8).collect());
            }
            jni_delete_local_ref(env, byte_arr);
        }
    }

    None
}

// ══════════════════════════════════════════════════════════════════════
// Logger
// ══════════════════════════════════════════════════════════════════════

fn log(msg: &str) {
    let log_path = PathBuf::from(DUMP_DIR).join("dumper.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        writeln!(f, "{}", msg).ok();
    }
}

// ══════════════════════════════════════════════════════════════════════
// PE Memory Scanner for JVM
// ══════════════════════════════════════════════════════════════════════

type JNI_GetCreatedJavaVMs_fn = unsafe extern "system" fn(*mut JavaVM, JSize, *mut JSize) -> JInt;

pub unsafe fn scan_for_javavm(jvm_base: usize, jvm_size: usize) -> Option<JavaVM> {
    let jvm_end = jvm_base + jvm_size;
    log(&format!("[SCAN] Starting memory scan for JavaVM in range 0x{:X} - 0x{:X} ({} MB)", jvm_base, jvm_end, jvm_size / (1024 * 1024)));

    let mut table_candidates = Vec::new();
    let mut curr = jvm_base;

    while curr < jvm_end {
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let q_res = VirtualQuery(curr as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        if q_res == 0 {
            curr += 4096;
            continue;
        }

        const MEM_COMMIT: u32 = 0x1000;
        const PAGE_NOACCESS: u32 = 0x01;
        const PAGE_GUARD: u32 = 0x100;

        if mbi.State != MEM_COMMIT || (mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD)) != 0 {
            curr = curr.saturating_add(mbi.RegionSize);
            continue;
        }

        let region_start = curr;
        let region_end = std::cmp::min(curr.saturating_add(mbi.RegionSize), jvm_end);
        let scan_limit = region_end.saturating_sub(64);

        let mut p = (region_start + 7) & !7;

        while p < scan_limit {
            let entry = p as *const usize;
            let r0 = *entry.add(0);
            let r1 = *entry.add(1);
            let r2 = *entry.add(2);
            let fn3 = *entry.add(3);
            let fn4 = *entry.add(4);
            let fn5 = *entry.add(5);
            let fn6 = *entry.add(6);
            let fn7 = *entry.add(7);

            let code_end = jvm_base + 0x2400000;

            if r0 == 0 && r1 == 0 && r2 == 0
                && fn3 >= jvm_base && fn3 < code_end
                && fn4 >= jvm_base && fn4 < code_end
                && fn5 >= jvm_base && fn5 < code_end
                && fn6 >= jvm_base && fn6 < code_end
                && fn7 >= jvm_base && fn7 < code_end
            {
                table_candidates.push(p);
                log(&format!("[SCAN] Found candidate JNIInvokeInterface at 0x{:X}", p));
            }

            p += 8;
        }

        curr = curr.saturating_add(mbi.RegionSize);
    }

    log(&format!("[SCAN] Found {} JNIInvokeInterface candidates", table_candidates.len()));

    for &table_addr in &table_candidates {
        log(&format!("[SCAN] Testing JNIInvokeInterface table at 0x{:X}", table_addr));

        let mut curr_scan = jvm_base;
        while curr_scan < jvm_end {
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            let q_res = VirtualQuery(curr_scan as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
            if q_res == 0 {
                curr_scan += 4096;
                continue;
            }

            const MEM_COMMIT: u32 = 0x1000;
            const PAGE_READWRITE: u32 = 0x04;
            const PAGE_WRITECOPY: u32 = 0x08;

            if mbi.State == MEM_COMMIT && (mbi.Protect & (PAGE_READWRITE | PAGE_WRITECOPY)) != 0 {
                let region_start = curr_scan;
                let region_end = std::cmp::min(curr_scan.saturating_add(mbi.RegionSize), jvm_end);
                let scan_limit = region_end.saturating_sub(8);

                let mut p = (region_start + 7) & !7;
                while p < scan_limit {
                    let ptr_val = *(p as *const usize);
                    if ptr_val == table_addr {
                        log(&format!("[SCAN] Found candidate main_vm at 0x{:X} -> 0x{:X}", p, table_addr));
                        let test_vm = p as JavaVM;

                        let mut test_env: *mut c_void = ptr::null_mut();
                        let iface = *test_vm;
                        let get_env_fn = (*iface).GetEnv;

                        let rc = get_env_fn(test_vm, &mut test_env, 0x00010008);
                        log(&format!("[SCAN] GetEnv test returned rc={}, env=0x{:X}", rc, test_env as usize));

                        // Genuine HotSpot JavaVM GetEnv on unattached thread returns JNI_EDETACHED (-2)
                        // If already attached, it returns 0 with a non-null env
                        if rc == -2 || (rc == 0 && !test_env.is_null()) {
                            log(&format!("[OK] Successfully verified active JavaVM at 0x{:X}!", p));
                            return Some(test_vm);
                        }
                    }
                    p += 8;
                }
            }

            curr_scan = curr_scan.saturating_add(mbi.RegionSize);
        }
    }

    None
}

// ══════════════════════════════════════════════════════════════════════
// Raw address-space PE scanner (anticheat-unlink resistant jvm.dll finder)
// ══════════════════════════════════════════════════════════════════════

static mut G_VEH_NOISE: u32 = 0;

const MEM_IMAGE: u32 = 0x1000000;

/// Parse the export directory of a PE mapped at `base` and check whether
/// it exports `target`. All reads are bounded by SizeOfImage.
unsafe fn pe_has_export(base: usize, target: &str) -> bool {
    if base == 0 || (base & 0xFFF) != 0 { return false; }
    if *(base as *const u16) != 0x5A4D { return false; } // MZ

    let e_lfanew = *(base.wrapping_add(0x3C) as *const i32) as i64;
    if e_lfanew <= 0 || e_lfanew > 0x1000 { return false; }
    let pe = base.wrapping_add(e_lfanew as usize);
    if *(pe as *const u32) != 0x00004550 { return false; } // PE\0\0
    if *(pe.wrapping_add(24) as *const u16) != 0x20B { return false; } // PE32+

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
    for i in 0..num_names {
        let name_rva = *(names_base.wrapping_add(i * 4) as *const u32) as usize;
        if name_rva == 0 || name_rva >= size_of_image { continue; }
        let name_ptr = base.wrapping_add(name_rva) as *const u8;
        // Compare with target without building a String per export.
        let tbytes = target.as_bytes();
        let mut ok = true;
        for (j, &tb) in tbytes.iter().enumerate() {
            let b = *name_ptr.add(j);
            if b == 0 || b != tb { ok = false; break; }
        }
        if ok && *name_ptr.add(tbytes.len()) == 0 { return true; }
    }
    false
}

/// Walk the whole user address space looking for a MEM_IMAGE mapping whose
/// export table contains JNI_CreateJavaVM. Returns (base, SizeOfImage).
unsafe fn find_jvm_by_pe_scan() -> Option<(usize, usize)> {
    const MEM_COMMIT: u32 = 0x1000;
    let mut curr: usize = 0;
    let mut last_probe: usize = 0;

    while curr < 0x0000_7FFF_FFFE_0000 {
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let q = VirtualQuery(curr as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        if q == 0 { break; }
        if mbi.RegionSize == 0 { break; }

        if mbi.State == MEM_COMMIT && mbi.Type == MEM_IMAGE {
            let probe = mbi.AllocationBase as usize;
            if probe != 0 && probe != last_probe && (probe & 0xFFF) == 0 {
                last_probe = probe;
                if pe_has_export(probe, "JNI_CreateJavaVM") {
                    let opt = probe + (*(probe.wrapping_add(0x3C) as *const i32) as usize) + 24;
                    let size_of_image = *(opt.wrapping_add(56) as *const u32) as usize;
                    return Some((probe, size_of_image));
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
// Thread hijack dumper (phase 2)
//
// Astraea cripples every thread attached through JNIInvokeInterface: the env
// is consistent but any JNIEnv call AVs (see [GUARD] lines in dumper.log).
// JNI only works on genuine Java threads, so we borrow one: suspend a thread
// whose RIP is inside jvm.dll (a JVM internal thread — JIT/GC), swap its
// context to run the dump on its identity and TLS, then restore the original
// context. The thread resumes exactly where it was interrupted.
// ══════════════════════════════════════════════════════════════════════

static mut G_DUMPED: u32 = 0;

fn note_dumped() {
    unsafe { G_DUMPED += 1; }
}

unsafe extern "C" fn hijack_entry(vm: JavaVM, orig_ctx: *const CONTEXT, done_event: isize, _unused: usize) {
    log("[HIJACK] entry running on victim thread");
    let iface = *vm;
    let get_env = (*iface).GetEnv;
    let mut env2: *mut c_void = ptr::null_mut();
    let rc = get_env(vm, &mut env2, 0x00010008);
    log(&format!("[HIJACK] GetEnv on victim thread: rc={}, env=0x{:X}", rc, env2 as usize));
    if rc == 0 && !env2.is_null() {
        let env = env2 as JNIEnv;
        if !try_jvmti_dump(vm, env) {
            jni_fallback_dump(env);
        }
        log(&format!("[HIJACK] dump phase finished, total classes saved so far: {}", G_DUMPED));
    } else {
        log("[HIJACK] victim GetEnv failed — this thread cannot do JNI either");
    }
    SetEvent(done_event);
    // Park ourselves; the coordinator restores the original context and
    // resumes us — execution then continues at the pre-hijack RIP.
    SuspendThread(GetCurrentThread());
    // Unreachable: resumed with the original context elsewhere.
}

extern "system" {
    fn OpenThread(dwDesiredAccess: u32, bInheritHandle: i32, dwThreadId: u32) -> isize;
    fn SuspendThread(hThread: isize) -> u32;
    fn ResumeThread(hThread: isize) -> u32;
    fn GetThreadContext(hThread: isize, lpContext: *mut CONTEXT) -> i32;
    fn SetThreadContext(hThread: isize, lpContext: *const CONTEXT) -> i32;
    fn GetCurrentThread() -> isize;
    fn GetCurrentThreadId() -> u32;
    fn WaitForSingleObject(hHandle: isize, dwMilliseconds: u32) -> u32;
    fn CreateEventW(lpEventAttributes: *mut c_void, bManualReset: i32, bInitialState: i32, lpName: *const u16) -> isize;
    fn SetEvent(hEvent: isize) -> i32;
    fn VirtualAlloc(lpAddress: *mut c_void, dwSize: usize, flAllocationType: u32, flProtect: u32) -> *mut c_void;
    fn VirtualFree(lpAddress: *mut c_void, dwSize: usize, dwFreeType: u32) -> i32;
    fn GetExitCodeThread(hThread: isize, lpExitCode: *mut u32) -> i32;
}

#[repr(C)]
struct THREADENTRY32 {
    dwSize: u32,
    cntUsage: u32,
    th32ThreadID: u32,
    th32OwnerProcessID: u32,
    tpBasePri: i32,
    tpDeltaPri: i32,
    dwFlags: u32,
}

extern "system" {
    fn Thread32First(hSnapshot: isize, lpte: *mut THREADENTRY32) -> i32;
    fn Thread32Next(hSnapshot: isize, lpte: *mut THREADENTRY32) -> i32;
}

const THREAD_SUSPEND_RESUME: u32 = 0x0002;
const THREAD_GET_CONTEXT: u32 = 0x0008;
const THREAD_SET_CONTEXT: u32 = 0x0010;
const THREAD_QUERY_INFORMATION: u32 = 0x0040;
const CONTEXT_FULL: u32 = 0x1000B;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;

unsafe fn hijack_dump(vm: JavaVM) {
    log("[HIJACK] phase 2: hunting a genuine Java thread to borrow");
    let me = GetCurrentThreadId();
    let snap = CreateToolhelp32Snapshot(0x00000004, 0); // TH32CS_SNAPTHREAD
    if snap == -1 {
        log("[HIJACK] thread snapshot failed");
        return;
    }

    let mut victim: isize = 0;
    let mut saved_ctx: CONTEXT = std::mem::zeroed();
    let mut te: THREADENTRY32 = std::mem::zeroed();
    te.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let my_pid = GetCurrentProcessId();
    if Thread32First(snap, &mut te) != 0 {
        loop {
            if te.th32OwnerProcessID == my_pid && te.th32ThreadID != me {
                let h = OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_SET_CONTEXT | THREAD_QUERY_INFORMATION, 0, te.th32ThreadID);
                if h != 0 {
                    if SuspendThread(h) != u32::MAX {
                        let mut ctx: CONTEXT = std::mem::zeroed();
                        ctx.context_flags = CONTEXT_FULL;
                        if GetThreadContext(h, &mut ctx) != 0 {
                            let where_ = get_module_for_address(ctx.rip as usize);
                            if where_.contains("jvm.dll") {
                                log(&format!("[HIJACK] victim tid={} RIP={} RSP=0x{:X}", te.th32ThreadID, where_, ctx.rsp));
                                saved_ctx = ctx;
                                victim = h;
                                break;
                            }
                        }
                        ResumeThread(h);
                    }
                    CloseHandle(h);
                }
            }
            if Thread32Next(snap, &mut te) == 0 { break; }
        }
    }
    CloseHandle(snap);

    if victim == 0 {
        log("[HIJACK] no Java thread found to hijack (OpenThread blocked or none in jvm.dll)");
        return;
    }

    let stack = VirtualAlloc(ptr::null_mut(), 4 * 1024 * 1024, 0x3000, 0x04); // MEM_COMMIT|RESERVE, PAGE_READWRITE
    if stack.is_null() {
        log("[HIJACK] stack alloc failed");
        ResumeThread(victim);
        CloseHandle(victim);
        return;
    }

    let event = CreateEventW(ptr::null_mut(), 1, 0, ptr::null());
    let boxed_ctx = Box::new(saved_ctx);
    let orig_ptr = Box::into_raw(boxed_ctx);

    let mut new_ctx: CONTEXT = std::mem::zeroed();
    new_ctx.context_flags = CONTEXT_FULL;
    let top = (stack as usize + 4 * 1024 * 1024) & !0xF;
    new_ctx.rsp = (top - 0x100 - 8) as u64; // 16-byte alignment per ABI at entry
    new_ctx.rip = hijack_entry as usize as u64;
    new_ctx.rcx = vm as usize as u64;
    new_ctx.rdx = orig_ptr as u64;
    new_ctx.r8 = event as u64;

    if SetThreadContext(victim, &new_ctx) == 0 {
        log(&format!("[HIJACK] SetThreadContext failed, err={}", GetLastError()));
        ResumeThread(victim);
        CloseHandle(victim);
        return;
    }
    ResumeThread(victim);
    log("[HIJACK] context swapped, waiting for dump to finish (90s)");
    let wait = WaitForSingleObject(event, 90000);

    // Whatever happened, reclaim the victim. If it self-suspended at the end
    // of hijack_entry, extra resumes unwind the count to zero.
    loop {
        let c = SuspendThread(victim);
        SetThreadContext(victim, orig_ptr);
        ResumeThread(victim);
        if c <= 1 { break; }
    }
    let _ = Box::from_raw(orig_ptr);
    VirtualFree(stack, 0, 0x8000); // MEM_RELEASE
    if event != 0 { CloseHandle(event); }
    CloseHandle(victim);

    match wait {
        WAIT_OBJECT_0 => log(&format!("[HIJACK] completed successfully, classes on disk: {}", G_DUMPED)),
        WAIT_TIMEOUT => log("[HIJACK] timed out — victim restored, game unfrozen"),
        _ => log(&format!("[HIJACK] wait failed, err={}", GetLastError())),
    }
}

pub unsafe fn dumper_main() {
    log("=== Vibe Dumper v10 started ===");

    let h_snap = CreateToolhelp32Snapshot(0x00000008, 0);
    let mut real_jvm_handle: isize = 0;
    let mut real_jvm_size: usize = 0;

    if h_snap != -1 {
        let mut me: MODULEENTRY32A = std::mem::zeroed();
        me.dwSize = std::mem::size_of::<MODULEENTRY32A>() as u32;

        if Module32First(h_snap, &mut me) != 0 {
            loop {
                let mod_name = CStr::from_ptr(me.szModule.as_ptr() as *const c_char).to_string_lossy().to_lowercase();
                let mod_path = CStr::from_ptr(me.szExePath.as_ptr() as *const c_char).to_string_lossy().to_string();

                if mod_name == "jvm.dll" {
                    real_jvm_handle = me.hModule;
                    real_jvm_size = me.modBaseSize as usize;
                    log(&format!("[JVM MODULE FOUND] jvm.dll @ 0x{:X} (size: 0x{:X}) -> {}", real_jvm_handle, real_jvm_size, mod_path));
                }

                if Module32Next(h_snap, &mut me) == 0 {
                    break;
                }
            }
        }
        CloseHandle(h_snap);
    }

    if real_jvm_handle == 0 {
        let mod_name = CString::new("jvm.dll").unwrap();
        real_jvm_handle = GetModuleHandleA(mod_name.as_ptr() as *const u8);
        real_jvm_size = 0x2400000;
    }

    if real_jvm_handle == 0 {
        // Anticheat (Astraea) unlinks jvm.dll from the PEB module list, so both
        // Toolhelp and GetModuleHandle miss it. Fall back to a raw address-space
        // scan: jvm.dll is the only MEM_IMAGE mapping exporting JNI_CreateJavaVM.
        if let Some((base, size)) = find_jvm_by_pe_scan() {
            real_jvm_handle = base as isize;
            real_jvm_size = size;
            log(&format!("[OK] jvm.dll recovered via raw PE scan @ 0x{:X} (image size 0x{:X})", base, size));
        }
    }

    if real_jvm_handle == 0 {
        log("[ERROR] jvm.dll not found in process memory");
        return;
    }

    G_JVM_BASE = real_jvm_handle as usize;
    G_JVM_SIZE = if real_jvm_size != 0 { real_jvm_size } else { 0x2400000 };
    log(&format!("[INFO] jvm image recorded: base=0x{:X} size=0x{:X}", G_JVM_BASE, G_JVM_SIZE));

    // Try standard JNI_GetCreatedJavaVMs
    let mut vm: JavaVM = ptr::null_mut();
    let fn_name = CString::new("JNI_GetCreatedJavaVMs").unwrap();
    if let Some(get_vms_ptr) = GetProcAddress(real_jvm_handle, fn_name.as_ptr() as *const u8) {
        let get_created_java_vms: JNI_GetCreatedJavaVMs_fn = std::mem::transmute(get_vms_ptr);
        let mut vms: [JavaVM; 4] = [ptr::null_mut(); 4];
        let mut vm_count: JSize = 0;
        let rc = get_created_java_vms(vms.as_mut_ptr(), 4, &mut vm_count);
        if rc == JNI_OK && vm_count > 0 && !vms[0].is_null() {
            vm = vms[0];
            log(&format!("[OK] JNI_GetCreatedJavaVMs succeeded, found {} VMs", vm_count));
        } else {
            log(&format!("[WARN] JNI_GetCreatedJavaVMs returned {} VMs (anticheat stub/zeroed), activating direct memory scanner...", vm_count));
        }
    }

    if vm.is_null() {
        if let Some(scanned_vm) = scan_for_javavm(real_jvm_handle as usize, real_jvm_size) {
            vm = scanned_vm;
            log(&format!("[OK] Successfully recovered JavaVM from memory scan: 0x{:X}", vm as usize));
        }
    }

    if vm.is_null() {
        log("[ERROR] Could not obtain JavaVM pointer");
        return;
    }

    #[repr(C)]
    struct JavaVMAttachArgs {
        version: i32,
        name: *const c_char,
        group: *mut c_void,
    }

    let thread_name = CString::new("VibeDumperThread").unwrap();
    let mut attach_args = JavaVMAttachArgs {
        version: 0x00010008,
        name: thread_name.as_ptr(),
        group: ptr::null_mut(),
    };

    let iface = *vm;
    log(&format!("[DIAG VM] DestroyJavaVM: 0x{:X} -> {}", (*iface).DestroyJavaVM as usize, get_module_for_address((*iface).DestroyJavaVM as usize)));
    log(&format!("[DIAG VM] AttachCurrentThread: 0x{:X} -> {}", (*iface).AttachCurrentThread as usize, get_module_for_address((*iface).AttachCurrentThread as usize)));
    log(&format!("[DIAG VM] DetachCurrentThread: 0x{:X} -> {}", (*iface).DetachCurrentThread as usize, get_module_for_address((*iface).DetachCurrentThread as usize)));
    log(&format!("[DIAG VM] GetEnv: 0x{:X} -> {}", (*iface).GetEnv as usize, get_module_for_address((*iface).GetEnv as usize)));
    log(&format!("[DIAG VM] AttachCurrentThreadAsDaemon: 0x{:X} -> {}", (*iface).AttachCurrentThreadAsDaemon as usize, get_module_for_address((*iface).AttachCurrentThreadAsDaemon as usize)));

    let mut env_ptr: *mut c_void = ptr::null_mut();

    let get_env_fn = (*iface).GetEnv;
    let env_rc = get_env_fn(vm, &mut env_ptr, 0x00010008);
    log(&format!("[ATTACH] Initial GetEnv returned rc={}, env=0x{:X}", env_rc, env_ptr as usize));

    if env_rc != 0 || env_ptr.is_null() {
        // Try standard AttachCurrentThread with NULL args (spec-compliant for JDK 1.2..21)
        let attach_std = (*iface).AttachCurrentThread;
        let rc_std = attach_std(vm, &mut env_ptr, ptr::null_mut());
        log(&format!("[ATTACH] AttachCurrentThread(NULL) returned rc={}, env=0x{:X}", rc_std, env_ptr as usize));

        if rc_std != 0 || env_ptr.is_null() {
            let attach = (*iface).AttachCurrentThreadAsDaemon;
            let rc = attach(vm, &mut env_ptr, &mut attach_args as *mut _ as *mut c_void);
            log(&format!("[ATTACH] AttachCurrentThreadAsDaemon returned rc={}, env=0x{:X}", rc, env_ptr as usize));
        }
    }

    if env_ptr.is_null() {
        log("[ERROR] Could not attach to JVM thread: env_ptr is NULL");
        return;
    }

    // The hooked attach path may hand us a decoy env. JNIInvokeInterface is
    // still genuine, so if the attach was real, GetEnv now yields the true
    // env for our thread. If they differ, trust GetEnv.
    let mut env2: *mut c_void = ptr::null_mut();
    let rc2 = get_env_fn(vm, &mut env2, 0x00010008);
    if rc2 == 0 && !env2.is_null() && env2 as usize != env_ptr as usize {
        log(&format!("[ATTACH] GetEnv returned a DIFFERENT env 0x{:X} (attach gave 0x{:X}) — using GetEnv env", env2 as usize, env_ptr as usize));
        env_ptr = env2;
    } else {
        log(&format!("[ATTACH] env accepted (rc2={}, env2=0x{:X})", rc2, env2 as usize));
    }

    let env = env_ptr as JNIEnv;
    log(&format!("[OK] Successfully attached to JVM! JNIEnv=0x{:X}", env as usize));

    // Inspect JNIEnv structure and function table
    let vtable = *env;
    let vtable_addr = vtable as usize;
    log(&format!("[DIAG] *env (vtable pointer) = 0x{:X} -> {}", vtable_addr, get_module_for_address(vtable_addr)));

    for idx in 0..30 {
        let fn_p = *(vtable as *const *const c_void).add(idx);
        let fn_addr = fn_p as usize;
        log(&format!("  [vtable #{:02}] 0x{:X} -> {}", idx, fn_addr, get_module_for_address(fn_addr)));
    }

    let fn_get_version = *(vtable as *const *const c_void).add(JNI_GET_VERSION);
    let fn_find_class = *(vtable as *const *const c_void).add(JNI_FIND_CLASS);
    let fn_exc_occurred = *(vtable as *const *const c_void).add(JNI_EXCEPTION_OCCURRED);
    let fn_exc_clear = *(vtable as *const *const c_void).add(JNI_EXCEPTION_CLEAR);
    let fn_exc_check = *(vtable as *const *const c_void).add(JNI_EXCEPTION_CHECK);

    log(&format!("[DIAG] fn[4]  (GetVersion)    = 0x{:X} -> {}", fn_get_version as usize, get_module_for_address(fn_get_version as usize)));
    log(&format!("[DIAG] fn[6]  (FindClass)     = 0x{:X} -> {}", fn_find_class as usize, get_module_for_address(fn_find_class as usize)));
    log(&format!("[DIAG] fn[15] (ExcOccurred)   = 0x{:X} -> {}", fn_exc_occurred as usize, get_module_for_address(fn_exc_occurred as usize)));
    log(&format!("[DIAG] fn[17] (ExcClear)      = 0x{:X} -> {}", fn_exc_clear as usize, get_module_for_address(fn_exc_clear as usize)));
    log(&format!("[DIAG] fn[228](ExcCheck)      = 0x{:X} -> {}", fn_exc_check as usize, get_module_for_address(fn_exc_check as usize)));

    // Step 1: Call GetVersion (guarded — this exact call used to AV and kill
    // the host process on the Astraea-hooked JVM).
    log("[DIAG] Executing GetVersion(env) [guarded]...");
    match guarded1::<JInt>(fn_get_version as usize, env as usize) {
        Some(ver) => log(&format!("[DIAG] GetVersion returned 0x{:08X}", ver)),
        None => log("[GUARD] GetVersion trapped — env is hostile, continuing with recon"),
    }

    // Step 2: Call ExceptionCheck
    log("[DIAG] Executing ExceptionCheck(env) [guarded]...");
    match guarded1::<JBoolean>(fn_exc_check as usize, env as usize) {
        Some(has_exc) => log(&format!("[DIAG] ExceptionCheck returned {}", has_exc)),
        None => log("[GUARD] ExceptionCheck trapped"),
    }

    // Step 3: Test FindClass("java/lang/Object")
    log("[DIAG] Executing FindClass(env, 'java/lang/Object') [guarded]...");
    let c_obj = CString::new("java/lang/Object").unwrap();
    let obj_cls = match guarded2::<JClass>(fn_find_class as usize, env as usize, c_obj.as_ptr() as usize) {
        Some(c) => c,
        None => ptr::null_mut(),
    };
    match guarded1::<JBoolean>(fn_exc_check as usize, env as usize) {
        Some(exc_after_obj) => log(&format!("[DIAG] FindClass('java/lang/Object') = 0x{:X}, exc={}", obj_cls as usize, exc_after_obj)),
        None => log("[GUARD] post-FindClass ExceptionCheck trapped"),
    }

    // Step 4: Test FindClass("java/lang/Thread")
    log("[DIAG] Executing FindClass(env, 'java/lang/Thread') [guarded]...");
    let c_th = CString::new("java/lang/Thread").unwrap();
    let th_cls = match guarded2::<JClass>(fn_find_class as usize, env as usize, c_th.as_ptr() as usize) {
        Some(c) => c,
        None => ptr::null_mut(),
    };
    match guarded1::<JBoolean>(fn_exc_check as usize, env as usize) {
        Some(exc_after_th) => log(&format!("[DIAG] FindClass('java/lang/Thread') = 0x{:X}, exc={}", th_cls as usize, exc_after_th)),
        None => log("[GUARD] post-FindClass ExceptionCheck trapped"),
    }

    // Run dumpers
    if !try_jvmti_dump(vm, env) {
        jni_fallback_dump(env);
    }

    let detach = (*iface).DetachCurrentThread;
    if guarded1::<()>(detach as usize, vm as usize).is_none() {
        log("[GUARD] DetachCurrentThread trapped — leaving attach cleanup to DLL_THREAD_DETACH");
    }

    if G_DUMPED == 0 {
        log("[PHASE2] attached-thread dump yielded 0 classes (hostile env) — attempting thread hijack");
        hijack_dump(vm);
    }
    log("=== Vibe Dumper v10 finished successfully ===");
}

// ══════════════════════════════════════════════════════════════════════
// Win32 FFI & Entry Point
// ══════════════════════════════════════════════════════════════════════

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
struct M128A {
    low: u64,
    high: i64,
}

#[repr(C, align(16))]
struct CONTEXT {
    p1_home: u64,
    p2_home: u64,
    p3_home: u64,
    p4_home: u64,
    p5_home: u64,
    p6_home: u64,
    context_flags: u32,
    mx_csr: u32,
    seg_cs: u16,
    seg_ds: u16,
    seg_es: u16,
    seg_fs: u16,
    seg_gs: u16,
    seg_ss: u16,
    e_flags: u32,
    dr0: u64, dr1: u64, dr2: u64, dr3: u64, dr6: u64, dr7: u64,
    rax: u64,
    rcx: u64,
    rdx: u64,
    rbx: u64,
    rsp: u64,
    rbp: u64,
    rsi: u64,
    rdi: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    flt_save: [u8; 512],
    vector_register: [M128A; 26],
    vector_control: u64,
    debug_control: u64,
    last_branch_to_rip: u64,
    last_branch_from_rip: u64,
    last_exception_to_rip: u64,
    last_exception_from_rip: u64,
}

#[repr(C)]
struct EXCEPTION_POINTERS {
    ExceptionRecord: *mut EXCEPTION_RECORD,
    ContextRecord: *mut c_void,
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
    fn GetCurrentProcessId() -> u32;
    fn GetLastError() -> u32;
}

unsafe fn get_module_for_address(addr: usize) -> String {
    let snap = CreateToolhelp32Snapshot(0x00000008, 0);
    if snap == -1 {
        return format!("0x{:X}", addr);
    }
    let mut me: MODULEENTRY32A = std::mem::zeroed();
    me.dwSize = std::mem::size_of::<MODULEENTRY32A>() as u32;
    let mut result = format!("0x{:X} (unknown)", addr);
    if Module32First(snap, &mut me) != 0 {
        loop {
            let base = me.modBaseAddr as usize;
            let end = base + me.modBaseSize as usize;
            if addr >= base && addr < end {
                let name = CStr::from_ptr(me.szModule.as_ptr() as *const c_char).to_string_lossy();
                result = format!("{}!0x{:X} (base 0x{:X})", name, addr - base, base);
                break;
            }
            if Module32Next(snap, &mut me) == 0 {
                break;
            }
        }
    }
    CloseHandle(snap);
    result
}

unsafe extern "system" fn veh_crash_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    // Crash-proof gate: a guarded JNI call faulted (Astraea trap). Redirect
    // execution to vibe_guard_abort which "returns" into the gate with RAX=0.
    if G_GUARD_ON && !info.is_null() && !(*info).ContextRecord.is_null() {
        G_GUARD_ON = false;
        G_GUARD_FAULTED = true;
        let ctx = &mut *(*info).ContextRecord.cast::<CONTEXT>();
        ctx.rip = vibe_guard_abort as usize as u64;
        ctx.rsp = G_GUARD_RSP.wrapping_sub(8) as u64;
        ctx.rax = 0;
        // The faulting JNI frame was abandoned mid-prologue; restore the
        // callee-saved registers our caller still expects.
        let s = G_CALSAVE;
        ctx.rbx = s[0] as u64;
        ctx.rbp = s[1] as u64;
        ctx.rdi = s[2] as u64;
        ctx.rsi = s[3] as u64;
        ctx.r12 = s[4] as u64;
        ctx.r13 = s[5] as u64;
        ctx.r14 = s[6] as u64;
        ctx.r15 = s[7] as u64;
        return -1; // EXCEPTION_CONTINUE_EXECUTION
    }

    // HotSpot raises benign SEVs for implicit null checks / deoptimization
    // thousands of times per minute; its own handler deals with them. Log a
    // few for diagnostics, then stay silent.
    unsafe {
        if G_VEH_NOISE < 5 {
            G_VEH_NOISE += 1;
            log(&format!("[VEH NOISE] non-guard exception 0x{:08X} (HotSpot implicit check, passing through)",
                (*(*info).ExceptionRecord).ExceptionCode));
        }
    }
    EXCEPTION_CONTINUE_SEARCH
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    _hinst_dll: *mut c_void,
    fdw_reason: u32,
    _lpv_reserved: *mut c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;

    if fdw_reason == DLL_PROCESS_ATTACH {
        AddVectoredExceptionHandler(1, veh_crash_handler);
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            dumper_main();
        });
    }

    1
}
