#![allow(non_camel_case_types, dead_code, clippy::missing_safety_doc)]
//! Typed standard JNI dispatch. Testable without injecting a DLL or loading a game.
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;

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
pub const JNI_EDETACHED: Jint = -2;
pub const JNI_VERSION_1_8: Jint = 0x00010008;

#[repr(C)]
pub struct JNIInvokeInterface {
    pub reserved0: *mut c_void,
    pub reserved1: *mut c_void,
    pub reserved2: *mut c_void,
    pub destroy_java_vm: unsafe extern "system" fn(JavaVM) -> Jint,
    pub attach_current_thread:
        unsafe extern "system" fn(JavaVM, *mut *mut c_void, *mut c_void) -> Jint,
    pub detach_current_thread: unsafe extern "system" fn(JavaVM) -> Jint,
    pub get_env: unsafe extern "system" fn(JavaVM, *mut *mut c_void, Jint) -> Jint,
    pub attach_current_thread_as_daemon:
        unsafe extern "system" fn(JavaVM, *mut *mut c_void, *mut c_void) -> Jint,
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
const JNI_CALL_STATIC_INT_METHOD_A: usize = 131;
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

// Only invoke the exact function supplied by the JVM's JNIEnv table.
// No machine-code scans, stub unwrapping, or synthetic exception recovery.
macro_rules! jni_invoke {
    ($env:expr, $slot:expr, ($($ty:ty),*) -> $ret:ty, $($arg:expr),* $(,)?) => {{
        let env = $env;
        if env.is_null() || (*env).is_null() { None } else {
            let table = *env as *const *const c_void;
            let address = *table.add($slot);
            if address.is_null() { None } else {
                let function: unsafe extern "system" fn($($ty),*) -> $ret =
                    std::mem::transmute(address);
                Some(function($($arg),*))
            }
        }
    }};
}

unsafe fn jni_clear_exception(env: JNIEnv) -> bool {
    let had = jni_invoke!(env, JNI_EXCEPTION_CHECK, (JNIEnv) -> Jboolean, env).unwrap_or(0) != 0;
    if had {
        let _ = jni_invoke!(env, JNI_EXCEPTION_CLEAR, (JNIEnv) -> (), env);
    }
    had
}

pub unsafe fn jni_get_version(env: JNIEnv) -> Jint {
    jni_invoke!(env, JNI_GET_VERSION, (JNIEnv) -> Jint, env).unwrap_or(0)
}

pub unsafe fn jni_find_class(env: JNIEnv, name: &str) -> Jclass {
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let res =
        jni_invoke!(env, JNI_FIND_CLASS, (JNIEnv, *const c_char) -> Jclass, env, cname.as_ptr())
            .unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    res
}

pub unsafe fn jni_get_method_id(env: JNIEnv, cls: Jclass, name: &str, sig: &str) -> JmethodID {
    if cls.is_null() {
        return ptr::null_mut();
    }
    let cn = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let cs = match CString::new(sig) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let r = jni_invoke!(env, JNI_GET_METHOD_ID, (JNIEnv, Jclass, *const c_char, *const c_char) -> JmethodID, env, cls, cn.as_ptr(), cs.as_ptr()).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_get_static_method_id(
    env: JNIEnv,
    cls: Jclass,
    name: &str,
    sig: &str,
) -> JmethodID {
    if cls.is_null() {
        return ptr::null_mut();
    }
    let cn = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let cs = match CString::new(sig) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let r = jni_invoke!(env, JNI_GET_STATIC_METHOD_ID, (JNIEnv, Jclass, *const c_char, *const c_char) -> JmethodID, env, cls, cn.as_ptr(), cs.as_ptr()).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_get_field_id(env: JNIEnv, cls: Jclass, name: &str, sig: &str) -> JfieldID {
    if cls.is_null() {
        return ptr::null_mut();
    }
    let cn = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let cs = match CString::new(sig) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let r = jni_invoke!(env, JNI_GET_FIELD_ID, (JNIEnv, Jclass, *const c_char, *const c_char) -> JfieldID, env, cls, cn.as_ptr(), cs.as_ptr()).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_get_static_field_id(env: JNIEnv, cls: Jclass, name: &str, sig: &str) -> JfieldID {
    if cls.is_null() {
        return ptr::null_mut();
    }
    let cn = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let cs = match CString::new(sig) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let r = jni_invoke!(env, JNI_GET_STATIC_FIELD_ID, (JNIEnv, Jclass, *const c_char, *const c_char) -> JfieldID, env, cls, cn.as_ptr(), cs.as_ptr()).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_get_object_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> Jobject {
    if obj.is_null() || fid.is_null() {
        return ptr::null_mut();
    }
    let r = jni_invoke!(env, JNI_GET_OBJECT_FIELD, (JNIEnv, Jobject, JfieldID) -> Jobject, env, obj, fid).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_get_double_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> f64 {
    if obj.is_null() || fid.is_null() {
        return 0.0;
    }
    let r =
        jni_invoke!(env, JNI_GET_DOUBLE_FIELD, (JNIEnv, Jobject, JfieldID) -> f64, env, obj, fid)
            .unwrap_or(0.0);
    if jni_clear_exception(env) {
        return 0.0;
    }
    r
}

pub unsafe fn jni_get_float_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> f32 {
    if obj.is_null() || fid.is_null() {
        return 0.0;
    }
    let r =
        jni_invoke!(env, JNI_GET_FLOAT_FIELD, (JNIEnv, Jobject, JfieldID) -> f32, env, obj, fid)
            .unwrap_or(0.0);
    if jni_clear_exception(env) {
        return 0.0;
    }
    r
}

pub unsafe fn jni_get_int_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> Jint {
    if obj.is_null() || fid.is_null() {
        return 0;
    }
    let r = jni_invoke!(env, JNI_GET_INT_FIELD, (JNIEnv, Jobject, JfieldID) -> Jint, env, obj, fid)
        .unwrap_or(0);
    if jni_clear_exception(env) {
        return 0;
    }
    r
}

pub unsafe fn jni_get_boolean_field(env: JNIEnv, obj: Jobject, fid: JfieldID) -> bool {
    if obj.is_null() || fid.is_null() {
        return false;
    }
    let r = jni_invoke!(env, JNI_GET_BOOLEAN_FIELD, (JNIEnv, Jobject, JfieldID) -> Jboolean, env, obj, fid).unwrap_or(0);
    if jni_clear_exception(env) {
        return false;
    }
    r != 0
}

pub unsafe fn jni_set_float_field(env: JNIEnv, obj: Jobject, fid: JfieldID, val: f32) {
    if obj.is_null() || fid.is_null() {
        return;
    }
    let _ = jni_invoke!(env, JNI_SET_FLOAT_FIELD, (JNIEnv, Jobject, JfieldID, Jfloat) -> (), env, obj, fid, val);
    jni_clear_exception(env);
}

pub unsafe fn jni_set_double_field(env: JNIEnv, obj: Jobject, fid: JfieldID, val: f64) {
    if obj.is_null() || fid.is_null() {
        return;
    }
    let _ = jni_invoke!(env, JNI_SET_DOUBLE_FIELD, (JNIEnv, Jobject, JfieldID, Jdouble) -> (), env, obj, fid, val);
    jni_clear_exception(env);
}

pub unsafe fn jni_set_boolean_field(env: JNIEnv, obj: Jobject, fid: JfieldID, val: bool) {
    if obj.is_null() || fid.is_null() {
        return;
    }
    let _ = jni_invoke!(env, JNI_SET_BOOLEAN_FIELD, (JNIEnv, Jobject, JfieldID, Jboolean) -> (), env, obj, fid, u8::from(val));
    jni_clear_exception(env);
}

pub unsafe fn jni_call_object(
    env: JNIEnv,
    obj: Jobject,
    mid: JmethodID,
    args: &[jvalue],
) -> Jobject {
    if obj.is_null() || mid.is_null() {
        return ptr::null_mut();
    }
    let p = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    let r = jni_invoke!(env, JNI_CALL_OBJECT_METHOD_A, (JNIEnv, Jobject, JmethodID, *const jvalue) -> Jobject, env, obj, mid, p).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_call_float(env: JNIEnv, obj: Jobject, mid: JmethodID, args: &[jvalue]) -> f32 {
    if obj.is_null() || mid.is_null() {
        return 0.0;
    }
    let p = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    let r = jni_invoke!(env, JNI_CALL_FLOAT_METHOD_A, (JNIEnv, Jobject, JmethodID, *const jvalue) -> f32, env, obj, mid, p).unwrap_or(0.0);
    if jni_clear_exception(env) {
        return 0.0;
    }
    r
}

pub unsafe fn jni_call_int(env: JNIEnv, obj: Jobject, mid: JmethodID, args: &[jvalue]) -> Jint {
    if obj.is_null() || mid.is_null() {
        return 0;
    }
    let p = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    let r = jni_invoke!(env, JNI_CALL_INT_METHOD_A, (JNIEnv, Jobject, JmethodID, *const jvalue) -> Jint, env, obj, mid, p).unwrap_or(0);
    if jni_clear_exception(env) {
        return 0;
    }
    r
}

pub unsafe fn jni_call_void(env: JNIEnv, obj: Jobject, mid: JmethodID, args: &[jvalue]) {
    if obj.is_null() || mid.is_null() {
        return;
    }
    let p = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    let _ = jni_invoke!(env, JNI_CALL_VOID_METHOD_A, (JNIEnv, Jobject, JmethodID, *const jvalue) -> (), env, obj, mid, p);
    jni_clear_exception(env);
}

pub unsafe fn jni_call_static_object(
    env: JNIEnv,
    cls: Jclass,
    mid: JmethodID,
    args: &[jvalue],
) -> Jobject {
    if cls.is_null() || mid.is_null() {
        return ptr::null_mut();
    }
    let p = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    let r = jni_invoke!(env, JNI_CALL_STATIC_OBJECT_METHOD_A, (JNIEnv, Jclass, JmethodID, *const jvalue) -> Jobject, env, cls, mid, p).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_call_static_void(env: JNIEnv, cls: Jclass, mid: JmethodID, args: &[jvalue]) {
    if cls.is_null() || mid.is_null() {
        return;
    }
    let p = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    let _ = jni_invoke!(env, JNI_CALL_STATIC_VOID_METHOD_A, (JNIEnv, Jclass, JmethodID, *const jvalue) -> (), env, cls, mid, p);
    jni_clear_exception(env);
}

pub unsafe fn jni_get_static_object_field(env: JNIEnv, cls: Jclass, fid: JfieldID) -> Jobject {
    if cls.is_null() || fid.is_null() {
        return ptr::null_mut();
    }
    let r = jni_invoke!(env, JNI_GET_STATIC_OBJECT_FIELD, (JNIEnv, Jclass, JfieldID) -> Jobject, env, cls, fid).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_get_array_length(env: JNIEnv, array: Jarray) -> Jsize {
    if array.is_null() {
        return 0;
    }
    let r =
        jni_invoke!(env, JNI_GET_ARRAY_LENGTH, (JNIEnv, Jarray) -> Jsize, env, array).unwrap_or(0);
    if jni_clear_exception(env) {
        return 0;
    }
    r
}

pub unsafe fn jni_get_object_array_element(env: JNIEnv, array: Jarray, idx: Jsize) -> Jobject {
    if array.is_null() {
        return ptr::null_mut();
    }
    let r = jni_invoke!(env, JNI_GET_OBJECT_ARRAY_ELEMENT, (JNIEnv, Jarray, Jsize) -> Jobject, env, array, idx).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_new_string_utf(env: JNIEnv, s: &str) -> Jstring {
    let cs = match CString::new(s) {
        Ok(c) => c,
        Err(_) => return ptr::null_mut(),
    };
    let r =
        jni_invoke!(env, JNI_NEW_STRING_UTF, (JNIEnv, *const c_char) -> Jstring, env, cs.as_ptr())
            .unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

pub unsafe fn jni_get_string_utf(env: JNIEnv, s: Jstring) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let chars = jni_invoke!(env, JNI_GET_STRING_UTF_CHARS,
        (JNIEnv, Jstring, *mut Jboolean) -> *const c_char, env, s, ptr::null_mut())?;
    if chars.is_null() {
        jni_clear_exception(env);
        return None;
    }
    let text = CStr::from_ptr(chars).to_string_lossy().to_string();
    let _ = jni_invoke!(env, JNI_RELEASE_STRING_UTF_CHARS,
        (JNIEnv, Jstring, *const c_char) -> (), env, s, chars);
    Some(text)
}

pub unsafe fn jni_push_local_frame(env: JNIEnv, capacity: Jint) -> Jint {
    jni_invoke!(env, JNI_PUSH_LOCAL_FRAME, (JNIEnv, Jint) -> Jint, env, capacity).unwrap_or(-1)
}

pub unsafe fn jni_pop_local_frame(env: JNIEnv, result: Jobject) -> Jobject {
    jni_invoke!(env, JNI_POP_LOCAL_FRAME, (JNIEnv, Jobject) -> Jobject, env, result)
        .unwrap_or(ptr::null_mut())
}

pub unsafe fn jni_new_global_ref(env: JNIEnv, obj: Jobject) -> Jobject {
    if obj.is_null() {
        return ptr::null_mut();
    }
    jni_invoke!(env, JNI_NEW_GLOBAL_REF, (JNIEnv, Jobject) -> Jobject, env, obj)
        .unwrap_or(ptr::null_mut())
}

pub unsafe fn jni_delete_global_ref(env: JNIEnv, obj: Jobject) {
    if obj.is_null() {
        return;
    }
    let _ = jni_invoke!(env, JNI_DELETE_GLOBAL_REF, (JNIEnv, Jobject) -> (), env, obj);
}

pub unsafe fn jni_delete_local_ref(env: JNIEnv, obj: Jobject) {
    if obj.is_null() {
        return;
    }
    let _ = jni_invoke!(env, JNI_DELETE_LOCAL_REF, (JNIEnv, Jobject) -> (), env, obj);
}

pub unsafe fn jni_is_same_object(env: JNIEnv, ref1: Jobject, ref2: Jobject) -> bool {
    jni_invoke!(env, JNI_IS_SAME_OBJECT, (JNIEnv, Jobject, Jobject) -> Jboolean, env, ref1, ref2)
        .unwrap_or(0)
        != 0
}

pub unsafe fn jni_new_object_array(
    env: JNIEnv,
    len: Jsize,
    element_class: Jclass,
    initial_element: Jobject,
) -> Jarray {
    let r = jni_invoke!(env, JNI_NEW_OBJECT_ARRAY, (JNIEnv, Jsize, Jclass, Jobject) -> Jarray, env, len, element_class, initial_element).unwrap_or(ptr::null_mut());
    if jni_clear_exception(env) {
        return ptr::null_mut();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Observed {
        env: usize,
        name: String,
        is_copy_was_null: bool,
        released: bool,
        float: f32,
        double: f64,
        object: usize,
        field: usize,
        empty_args: bool,
        argument: i32,
        pending: bool,
        clears: usize,
    }

    thread_local! {
        static OBSERVED: RefCell<Observed> = RefCell::new(Observed::default());
    }

    struct MockEnv {
        table: Box<[*const c_void; 229]>,
        env: Box<*const c_void>,
    }

    impl MockEnv {
        fn new() -> Self {
            OBSERVED.with(|o| *o.borrow_mut() = Observed::default());
            let mut table = Box::new([ptr::null(); 229]);
            table[JNI_EXCEPTION_CHECK] = check as *const c_void;
            table[JNI_EXCEPTION_CLEAR] = clear as *const c_void;
            let env = Box::new(table.as_ptr() as *const c_void);
            Self { table, env }
        }

        fn ptr(&mut self) -> JNIEnv {
            &mut *self.env as JNIEnv
        }
    }

    unsafe extern "system" fn check(_: JNIEnv) -> Jboolean {
        OBSERVED.with(|o| u8::from(o.borrow().pending))
    }

    unsafe extern "system" fn clear(_: JNIEnv) {
        OBSERVED.with(|o| {
            let mut o = o.borrow_mut();
            o.pending = false;
            o.clears += 1;
        });
    }

    unsafe extern "system" fn find_class(env: JNIEnv, name: *const c_char) -> Jclass {
        OBSERVED.with(|o| {
            let mut o = o.borrow_mut();
            o.env = env as usize;
            o.name = CStr::from_ptr(name).to_string_lossy().into_owned();
        });
        0x1234usize as Jclass // opaque test handle, never dereferenced
    }

    unsafe extern "system" fn get_utf(
        _: JNIEnv,
        _: Jstring,
        is_copy: *mut Jboolean,
    ) -> *const c_char {
        OBSERVED.with(|o| o.borrow_mut().is_copy_was_null = is_copy.is_null());
        b"hello\0".as_ptr() as *const c_char
    }

    unsafe extern "system" fn release_utf(_: JNIEnv, _: Jstring, _: *const c_char) {
        OBSERVED.with(|o| o.borrow_mut().released = true);
    }

    unsafe extern "system" fn set_float(_: JNIEnv, obj: Jobject, field: JfieldID, value: Jfloat) {
        OBSERVED.with(|o| {
            let mut o = o.borrow_mut();
            o.float = value;
            o.object = obj as usize;
            o.field = field as usize;
        });
    }

    unsafe extern "system" fn set_double(_: JNIEnv, obj: Jobject, field: JfieldID, value: Jdouble) {
        OBSERVED.with(|o| {
            let mut o = o.borrow_mut();
            o.double = value;
            o.object = obj as usize;
            o.field = field as usize;
        });
    }

    unsafe extern "system" fn call_object(
        _: JNIEnv,
        _: Jobject,
        _: JmethodID,
        args: *const jvalue,
    ) -> Jobject {
        OBSERVED.with(|o| {
            let mut o = o.borrow_mut();
            o.empty_args = args.is_null();
            if !args.is_null() {
                o.argument = (*args).i;
            }
        });
        0x5678usize as Jobject
    }

    #[test]
    fn find_class_receives_env_and_name_in_correct_argument_positions() {
        let mut mock = MockEnv::new();
        mock.table[JNI_FIND_CLASS] = find_class as *const c_void;
        let env = mock.ptr();
        let result = unsafe { jni_find_class(env, "java/lang/Object") };
        assert_eq!(result as usize, 0x1234);
        OBSERVED.with(|o| {
            let o = o.borrow();
            assert_eq!(o.env, env as usize);
            assert_eq!(o.name, "java/lang/Object");
        });
    }

    #[test]
    fn get_utf_passes_explicit_null_third_argument_and_releases_chars() {
        let mut mock = MockEnv::new();
        mock.table[JNI_GET_STRING_UTF_CHARS] = get_utf as *const c_void;
        mock.table[JNI_RELEASE_STRING_UTF_CHARS] = release_utf as *const c_void;
        assert_eq!(
            unsafe { jni_get_string_utf(mock.ptr(), 1usize as Jstring) },
            Some("hello".into())
        );
        OBSERVED.with(|o| {
            assert!(o.borrow().is_copy_was_null);
            assert!(o.borrow().released);
        });
    }

    #[test]
    fn setters_pass_actual_floating_point_values_not_integer_bits() {
        let mut mock = MockEnv::new();
        mock.table[JNI_SET_FLOAT_FIELD] = set_float as *const c_void;
        mock.table[JNI_SET_DOUBLE_FIELD] = set_double as *const c_void;
        unsafe {
            jni_set_float_field(mock.ptr(), 1usize as Jobject, 2usize as JfieldID, 1.25);
            jni_set_double_field(mock.ptr(), 1usize as Jobject, 2usize as JfieldID, -123.5);
        }
        OBSERVED.with(|o| {
            let o = o.borrow();
            assert_eq!(o.float, 1.25);
            assert_eq!(o.double, -123.5);
            assert_eq!((o.object, o.field), (1, 2));
        });
    }

    #[test]
    fn method_a_passes_null_for_empty_args_and_typed_array_for_nonempty_args() {
        let mut mock = MockEnv::new();
        mock.table[JNI_CALL_OBJECT_METHOD_A] = call_object as *const c_void;
        unsafe {
            jni_call_object(mock.ptr(), 1usize as Jobject, 2usize as JmethodID, &[]);
        }
        OBSERVED.with(|o| assert!(o.borrow().empty_args));
        unsafe {
            jni_call_object(
                mock.ptr(),
                1usize as Jobject,
                2usize as JmethodID,
                &[jvalue { i: -42 }],
            );
        }
        OBSERVED.with(|o| assert_eq!(o.borrow().argument, -42));
    }

    #[test]
    fn exception_result_is_discarded_and_exception_is_cleared() {
        let mut mock = MockEnv::new();
        mock.table[JNI_FIND_CLASS] = find_class as *const c_void;
        OBSERVED.with(|o| o.borrow_mut().pending = true);
        assert!(unsafe { jni_find_class(mock.ptr(), "missing/Class") }.is_null());
        OBSERVED.with(|o| {
            assert!(!o.borrow().pending);
            assert_eq!(o.borrow().clears, 1);
        });
    }

    #[test]
    fn null_env_null_slot_and_embedded_nul_do_not_dispatch() {
        let mut mock = MockEnv::new();
        assert!(unsafe { jni_find_class(ptr::null_mut(), "java/lang/Object") }.is_null());
        assert!(unsafe { jni_find_class(mock.ptr(), "java/lang/Object") }.is_null());
        mock.table[JNI_FIND_CLASS] = find_class as *const c_void;
        assert!(unsafe { jni_find_class(mock.ptr(), "bad\0name") }.is_null());
        OBSERVED.with(|o| assert!(o.borrow().name.is_empty()));
    }
}
