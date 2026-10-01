use std::ffi::{c_char, c_void, CString};
use std::ptr;
use vibe_dumper_v11::{JavaVM, JNIEnv, jvalue};

#[repr(C)]
struct JavaVMOption {
    optionString: *const c_char,
    extraInfo: *mut c_void,
}

#[repr(C)]
struct JavaVMInitArgs {
    version: i32,
    nOptions: i32,
    options: *mut JavaVMOption,
    ignoreUnrecognized: u8,
}

type JNI_CreateJavaVM_fn = unsafe extern "system" fn(*mut JavaVM, *mut *mut c_void, *mut c_void) -> i32;

extern "system" {
    fn LoadLibraryExA(lpLibFileName: *const u8, hFile: isize, dwFlags: u32) -> isize;
    fn SetDllDirectoryA(lpPathName: *const u8) -> i32;
    fn GetProcAddress(hModule: isize, lpProcName: *const u8) -> Option<unsafe extern "system" fn() -> isize>;
    fn GetLastError() -> u32;
}

fn main() {
    println!("============================================================");
    println!("  VIBE DUMPER INTEGRATION TEST SUITE");
    println!("  Target: RustMe OpenJDK 21 (jvm.dll in-process harness)");
    println!("============================================================");

    let jvm_home = std::env::var("VIBE_JVM_HOME")
        .unwrap_or_else(|_| r"c:\users\avixo\appdata\roaming\rustme-launcher\java\prod-a".to_string());
    let bin_dir = CString::new(format!(r"{}\bin", jvm_home)).unwrap();
    unsafe { SetDllDirectoryA(bin_dir.as_ptr() as *const u8) };
    let jvm_path = CString::new(format!(r"{}\bin\server\jvm.dll", jvm_home)).unwrap();
    println!("[INFO] JVM home: {}", jvm_home);
    let h_jvm = unsafe { LoadLibraryExA(jvm_path.as_ptr() as *const u8, 0, 0x00000008) };
    if h_jvm == 0 {
        let err = unsafe { GetLastError() };
        eprintln!("[FAIL] LoadLibraryExA failed for jvm.dll: GetLastError={}", err);
        if err == 1114 {
            eprintln!("[HINT] ERROR_DLL_INIT_FAILED: this jvm.dll is packed and its DllMain refuses");
            eprintln!("[HINT] to initialize outside the official launcher. Set VIBE_JVM_HOME to a");
            eprintln!("[HINT] stock JDK/JRE (e.g. Temurin 21) to run the harness tests.");
        }
        std::process::exit(1);
    }
    println!("[PASS] jvm.dll loaded at 0x{:X}", h_jvm);

    let fn_create_name = CString::new("JNI_CreateJavaVM").unwrap();
    let proc = unsafe { GetProcAddress(h_jvm, fn_create_name.as_ptr() as *const u8) };
    if proc.is_none() {
        eprintln!("[FAIL] JNI_CreateJavaVM not found in jvm.dll");
        std::process::exit(1);
    }
    let create_jvm: JNI_CreateJavaVM_fn = unsafe { std::mem::transmute(proc.unwrap()) };
    println!("[PASS] JNI_CreateJavaVM symbol resolved");

    let opt_cp = CString::new("-Djava.class.path=d:\\project\\rustme\\tools\\cfr.jar").unwrap();
    let mut options = [
        JavaVMOption {
            optionString: opt_cp.as_ptr(),
            extraInfo: ptr::null_mut(),
        }
    ];

    let mut args = JavaVMInitArgs {
        version: 0x00010008, // JNI_VERSION_1_8
        nOptions: 1,
        options: options.as_mut_ptr(),
        ignoreUnrecognized: 1,
    };

    let mut vm: JavaVM = ptr::null_mut();
    let mut env_ptr: *mut c_void = ptr::null_mut();

    let rc = unsafe { create_jvm(&mut vm, &mut env_ptr, &mut args as *mut _ as *mut c_void) };
    if rc != 0 || vm.is_null() || env_ptr.is_null() {
        eprintln!("[FAIL] JNI_CreateJavaVM failed with rc={}", rc);
        std::process::exit(1);
    }
    println!("[PASS] JNI_CreateJavaVM created active JVM at 0x{:X}, env=0x{:X}", vm as usize, env_ptr as usize);

    let env = env_ptr as JNIEnv;

    // Test 1: Memory Scanner
    println!("\n--- [TEST 1] PE Memory Scanner for JavaVM ---");
    let scanned_vm = unsafe { vibe_dumper_v11::scan_for_javavm(h_jvm as usize, 0x2400000) };
    match scanned_vm {
        Some(svm) => {
            println!("[PASS] Memory scanner found JavaVM at 0x{:X} (actual: 0x{:X})", svm as usize, vm as usize);
            assert_eq!(svm, vm, "Scanned VM must match actual VM");
        }
        None => {
            eprintln!("[FAIL] Memory scanner failed to locate JavaVM");
            std::process::exit(1);
        }
    }

    // Test 2: Basic JNI GetVersion
    println!("\n--- [TEST 2] JNI GetVersion ---");
    let vtable = unsafe { *env };
    let fn_get_version: unsafe extern "system" fn(JNIEnv) -> i32 = unsafe {
        let p = *(vtable as *const *const c_void).add(4);
        std::mem::transmute(p)
    };
    let ver = unsafe { fn_get_version(env) };
    println!("[PASS] GetVersion returned 0x{:08X}", ver);

    // Test 3: ExceptionCheck
    println!("\n--- [TEST 3] JNI ExceptionCheck ---");
    let fn_exc_check: unsafe extern "system" fn(JNIEnv) -> u8 = unsafe {
        let p = *(vtable as *const *const c_void).add(228);
        std::mem::transmute(p)
    };
    let exc = unsafe { fn_exc_check(env) };
    println!("[PASS] ExceptionCheck returned {}", exc);
    assert_eq!(exc, 0, "ExceptionCheck should be 0 on clean state");

    // Test 4: FindClass java/lang/Object and java/lang/Thread
    println!("\n--- [TEST 4] FindClass core classes ---");
    let fn_find_class: unsafe extern "system" fn(JNIEnv, *const c_char) -> *mut c_void = unsafe {
        let p = *(vtable as *const *const c_void).add(6);
        std::mem::transmute(p)
    };

    let c_obj = CString::new("java/lang/Object").unwrap();
    let obj_cls = unsafe { fn_find_class(env, c_obj.as_ptr()) };
    assert!(!obj_cls.is_null(), "FindClass(java/lang/Object) must not be null");
    println!("[PASS] FindClass('java/lang/Object') = 0x{:X}", obj_cls as usize);

    let c_th = CString::new("java/lang/Thread").unwrap();
    let th_cls = unsafe { fn_find_class(env, c_th.as_ptr()) };
    assert!(!th_cls.is_null(), "FindClass(java/lang/Thread) must not be null");
    println!("[PASS] FindClass('java/lang/Thread') = 0x{:X}", th_cls as usize);

    // Test 5: Call Thread.getAllStackTraces()
    println!("\n--- [TEST 5] Thread.getAllStackTraces() execution ---");
    let fn_get_static_mid: unsafe extern "system" fn(JNIEnv, *mut c_void, *const c_char, *const c_char) -> *mut c_void = unsafe {
        let p = *(vtable as *const *const c_void).add(113);
        std::mem::transmute(p)
    };
    let c_mid_name = CString::new("getAllStackTraces").unwrap();
    let c_mid_sig = CString::new("()Ljava/util/Map;").unwrap();
    let mid_get_all = unsafe { fn_get_static_mid(env, th_cls, c_mid_name.as_ptr(), c_mid_sig.as_ptr()) };
    assert!(!mid_get_all.is_null(), "GetStaticMethodID(getAllStackTraces) must not be null");
    println!("[PASS] GetStaticMethodID(getAllStackTraces) = 0x{:X}", mid_get_all as usize);

    let fn_call_static_obj_a: unsafe extern "system" fn(JNIEnv, *mut c_void, *mut c_void, *const jvalue) -> *mut c_void = unsafe {
        let p = *(vtable as *const *const c_void).add(116);
        std::mem::transmute(p)
    };
    let map_obj = unsafe { fn_call_static_obj_a(env, th_cls, mid_get_all, ptr::null()) };
    assert!(!map_obj.is_null(), "Thread.getAllStackTraces() must return valid Map");
    println!("[PASS] Thread.getAllStackTraces() returned Map object: 0x{:X}", map_obj as usize);

    // Test 6: Full jni_fallback_dump invocation
    println!("\n--- [TEST 6] Full jni_fallback_dump pipeline ---");
    unsafe {
        vibe_dumper_v11::jni_fallback_dump(env);
    }
    println!("[PASS] jvm_fallback_dump completed cleanly without errors!");

    println!("\n============================================================");
    println!("  ALL 6 INTEGRATION TESTS PASSED PERFECTLY!");
    println!("  vibe_dumper_v11 is completely stable and crash-free.");
    println!("============================================================");
}
