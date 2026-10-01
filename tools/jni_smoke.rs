//! Standalone smoke test against a fresh, ordinary Linux JVM, not a game process.
//! Usage: rustc --edition=2021 tools/jni_smoke.rs -o jni-smoke
//!        ./jni-smoke /absolute/path/to/lib/server/libjvm.so
#[path = "../src/engine/jni_api.rs"]
mod jni_api;

#[cfg(target_os = "linux")]
fn main() {
    use jni_api::*;
    use std::ffi::{c_char, c_void, CString};
    use std::ptr;

    #[link(name = "dl")]
    extern "C" {
        fn dlopen(path: *const c_char, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    #[repr(C)]
    struct VmOption {
        option_string: *mut c_char,
        extra_info: *mut c_void,
    }
    #[repr(C)]
    struct InitArgs {
        version: Jint,
        n_options: Jint,
        options: *mut c_void,
        ignore_unrecognized: Jboolean,
    }

    let path = CString::new(std::env::args().nth(1).expect("Pass a libjvm.so path")).unwrap();
    unsafe {
        let library = dlopen(path.as_ptr(), 2 | 256); // RTLD_NOW | RTLD_GLOBAL
        assert!(!library.is_null(), "Could not load test JVM");
        let address = dlsym(library, b"JNI_CreateJavaVM\0".as_ptr().cast());
        assert!(!address.is_null(), "JNI_CreateJavaVM missing");
        let create: unsafe extern "system" fn(
            *mut JavaVM,
            *mut *mut c_void,
            *mut InitArgs,
        ) -> Jint = std::mem::transmute(address);
        let mut vm: JavaVM = ptr::null_mut();
        let mut raw_env: *mut c_void = ptr::null_mut();
        let check_jni = CString::new("-Xcheck:jni").unwrap();
        let mut option = VmOption {
            option_string: check_jni.as_ptr() as *mut c_char,
            extra_info: ptr::null_mut(),
        };
        let mut init = InitArgs {
            version: JNI_VERSION_1_8,
            n_options: 1,
            options: &mut option as *mut _ as *mut c_void,
            ignore_unrecognized: 0,
        };
        assert_eq!(create(&mut vm, &mut raw_env, &mut init), JNI_OK);
        assert!(!vm.is_null() && !raw_env.is_null());
        let env = raw_env as JNIEnv;
        assert!(jni_get_version(env) >= JNI_VERSION_1_8);

        let address = dlsym(library, b"JNI_GetCreatedJavaVMs\0".as_ptr().cast());
        assert!(!address.is_null());
        let get_vms: unsafe extern "system" fn(*mut JavaVM, Jsize, *mut Jsize) -> Jint =
            std::mem::transmute(address);
        let mut vms = [ptr::null_mut(); 2];
        let mut count = 0;
        assert_eq!(get_vms(vms.as_mut_ptr(), 2, &mut count), JNI_OK);
        assert_eq!(count, 1);
        assert_eq!(vms[0], vm);
        let mut verified = ptr::null_mut();
        assert_eq!(
            ((*(*vm)).get_env)(vm, &mut verified, JNI_VERSION_1_8),
            JNI_OK
        );
        assert_eq!(verified, raw_env);

        assert_eq!(jni_push_local_frame(env, 32), JNI_OK);
        let object = jni_find_class(env, "java/lang/Object");
        assert!(!object.is_null());
        // Failure must clear its Java exception so subsequent JNI calls work.
        assert!(jni_find_class(env, "vibe/test/DefinitelyMissingClass").is_null());
        assert!(!jni_find_class(env, "java/lang/Object").is_null());
        let text = jni_new_string_utf(env, "JNI smoke test");
        assert_eq!(
            jni_get_string_utf(env, text).as_deref(),
            Some("JNI smoke test")
        );

        let thread_class = jni_find_class(env, "java/lang/Thread");
        assert!(!thread_class.is_null());
        let current =
            jni_get_static_method_id(env, thread_class, "currentThread", "()Ljava/lang/Thread;");
        assert!(!current.is_null());
        let thread = jni_call_static_object(env, thread_class, current, &[]);
        assert!(!thread.is_null());
        let get_name = jni_get_method_id(env, thread_class, "getName", "()Ljava/lang/String;");
        assert!(!get_name.is_null());
        let name = jni_call_object(env, thread, get_name, &[]);
        assert!(!jni_get_string_utf(env, name).unwrap().is_empty());

        let float_class = jni_find_class(env, "java/lang/Float");
        let float_factory =
            jni_get_static_method_id(env, float_class, "valueOf", "(F)Ljava/lang/Float;");
        let float_object =
            jni_call_static_object(env, float_class, float_factory, &[jvalue { f: 1.25 }]);
        let float_value = jni_get_field_id(env, float_class, "value", "F");
        assert!(!float_object.is_null() && !float_value.is_null());
        jni_set_float_field(env, float_object, float_value, -2.5);
        assert_eq!(jni_get_float_field(env, float_object, float_value), -2.5);

        let double_class = jni_find_class(env, "java/lang/Double");
        let double_factory =
            jni_get_static_method_id(env, double_class, "valueOf", "(D)Ljava/lang/Double;");
        let double_object =
            jni_call_static_object(env, double_class, double_factory, &[jvalue { d: 1.25 }]);
        let double_value = jni_get_field_id(env, double_class, "value", "D");
        assert!(!double_object.is_null() && !double_value.is_null());
        jni_set_double_field(env, double_object, double_value, -123.5);
        assert_eq!(
            jni_get_double_field(env, double_object, double_value),
            -123.5
        );

        // JavaVM is shareable; JNIEnv is not. Obtain a separate JNIEnv on the
        // worker, verify it, and detach only that worker's own attachment.
        let vm_address = vm as usize;
        std::thread::spawn(move || {
            let vm = vm_address as JavaVM;
            let interface = *vm;
            let mut worker_env = ptr::null_mut();
            assert_eq!(
                ((*interface).get_env)(vm, &mut worker_env, JNI_VERSION_1_8),
                JNI_EDETACHED
            );
            assert_eq!(
                ((*interface).attach_current_thread_as_daemon)(
                    vm,
                    &mut worker_env,
                    ptr::null_mut()
                ),
                JNI_OK
            );
            assert!(!worker_env.is_null());
            let mut verified = ptr::null_mut();
            assert_eq!(
                ((*interface).get_env)(vm, &mut verified, JNI_VERSION_1_8),
                JNI_OK
            );
            assert_eq!(verified, worker_env);
            let object = jni_find_class(worker_env as JNIEnv, "java/lang/Object");
            assert!(!object.is_null());
            jni_delete_local_ref(worker_env as JNIEnv, object);
            assert_eq!(((*interface).detach_current_thread)(vm), JNI_OK);
            assert_eq!(
                ((*interface).get_env)(vm, &mut verified, JNI_VERSION_1_8),
                JNI_EDETACHED
            );
        })
        .join()
        .unwrap();

        jni_pop_local_frame(env, ptr::null_mut());
        assert_eq!(((*(*vm)).destroy_java_vm)(vm), JNI_OK);
        println!("PASS: standard JVM discovery, version, FindClass, exception clearing, strings, method calls, float/double fields and worker attach/detach");
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This standalone smoke test is for a fresh Linux JVM only.");
    std::process::exit(1);
}
