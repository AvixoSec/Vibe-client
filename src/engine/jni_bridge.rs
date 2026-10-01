#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::missing_safety_doc)]

//! Standard JNI bridge for the Vibe Client.
//! Uses JVM-provided function pointers; unsupported initialization fails closed.

use std::ffi::{c_char, c_void, CString};
use std::ptr;

use crate::core::snapshot::*;

// ══════════════════════════════════════════════════════════════════════
// JNI Type Aliases
// ══════════════════════════════════════════════════════════════════════

pub use crate::engine::jni_api::*;

#[repr(C)]
struct JavaVMAttachArgs {
    version: i32,
    name: *const c_char,
    group: *mut c_void,
}

// ══════════════════════════════════════════════════════════════════════
// VEH Crash-Proof Gate
// ══════════════════════════════════════════════════════════════════════

pub static mut G_RENDER_GUARD: bool = false;
pub static mut G_RENDER_RSP: usize = 0;
pub static mut G_RENDER_CALSAVE: [usize; 8] = [0; 8];
pub static mut G_RENDER_TID: u32 = 0;
pub static mut G_RENDER_FAULTED: bool = false;
static mut G_VEH_INSTALLED: bool = false;

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

// ══════════════════════════════════════════════════════════════════════
// Win32 FFI
// ══════════════════════════════════════════════════════════════════════

extern "system" {
    fn GetModuleHandleA(lpModuleName: *const u8) -> isize;
    fn GetProcAddress(hModule: isize, lpProcName: *const u8) -> Option<unsafe extern "system" fn() -> isize>;
    fn AddVectoredExceptionHandler(First: u32, Handler: unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32) -> *mut c_void;
    fn GetCurrentThreadId() -> u32;
}

type JNI_GetCreatedJavaVMs_fn = unsafe extern "system" fn(*mut JavaVM, Jsize, *mut Jsize) -> Jint;

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

    log_msg("[JNI] Resolver: FindClass(java/lang/Class)");
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

    // The attached worker can only use its own context loader and loaders
    // obtained from known entrypoint classes. Do not enumerate every thread or
    // request all Java stack traces during native initialization.
    let thread_cls = jni_find_class(env, "java/lang/Thread");
    if !thread_cls.is_null() {
        let current = jni_get_static_method_id(env, thread_cls, "currentThread", "()Ljava/lang/Thread;");
        let context = jni_get_method_id(env, thread_cls, "getContextClassLoader", "()Ljava/lang/ClassLoader;");
        if !current.is_null() && !context.is_null() {
            let thread = jni_call_static_object(env, thread_cls, current, &[]);
            if !thread.is_null() {
                let loader = jni_call_object(env, thread, context, &[]);
                if !loader.is_null() {
                    add_loader(loader, "current thread contextClassLoader");
                    jni_delete_local_ref(env, loader);
                }
                jni_delete_local_ref(env, thread);
            }
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

pub fn log_msg(msg: &str) {
    crate::engine::diagnostics::log_msg(msg);
}

// ══════════════════════════════════════════════════════════════════════
// JniBridge
// ══════════════════════════════════════════════════════════════════════

pub struct JniBridge {
    pub is_connected: bool,
    pub jvm_ptr: JavaVM,
    pub env_ptr: *mut c_void,
    cached_ids: Option<CachedIds>,
    owner_thread: Option<std::thread::ThreadId>,
    owns_attachment: bool,
    mc_instance: Jobject, // cached Minecraft singleton
}

// Raw JVM handles are deliberately not Send or Sync: JNIEnv is thread-local.

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
            owner_thread: None,
            owns_attachment: false,
            mc_instance: ptr::null_mut(),
        }
    }

    pub fn env(&self) -> JNIEnv {
        if self.owner_thread != Some(std::thread::current().id()) {
            return ptr::null_mut();
        }
        self.env_ptr as JNIEnv
    }

    fn detach_owned_thread(&mut self) {
        if self.owner_thread != Some(std::thread::current().id()) { return; }
        if self.owns_attachment && !self.jvm_ptr.is_null() {
            unsafe { ((*(*self.jvm_ptr)).detach_current_thread)(self.jvm_ptr); }
        }
        self.is_connected = false;
        self.owns_attachment = false;
        self.owner_thread = None;
        self.env_ptr = ptr::null_mut();
        self.jvm_ptr = ptr::null_mut();
    }

    #[cfg(target_os = "windows")]
    pub fn attach_to_game_process(&mut self) -> Result<(), &'static str> {
        unsafe {
            if self.is_connected {
                return if self.env().is_null() { Err("JNIEnv belongs to another thread") } else { Ok(()) };
            }
            // Only the standard exported Invocation API may supply a JavaVM.
            // Guessing one from arbitrary memory can accept a false vtable.
            let jvm_handle = GetModuleHandleA(b"jvm.dll\0".as_ptr());
            if jvm_handle == 0 { return Err("jvm.dll unavailable through standard loader API; initialization stopped"); }
            let proc = GetProcAddress(jvm_handle, b"JNI_GetCreatedJavaVMs\0".as_ptr())
                .ok_or("JNI_GetCreatedJavaVMs unavailable; memory-scan fallback disabled")?;
            let get_vms: JNI_GetCreatedJavaVMs_fn = std::mem::transmute(proc);
            let mut vms: [JavaVM; 2] = [ptr::null_mut(); 2];
            let mut count: Jsize = 0;
            let rc = get_vms(vms.as_mut_ptr(), 2, &mut count);
            if rc != JNI_OK || count != 1 || vms[0].is_null() {
                log_msg(&format!("[JNI] GetCreatedJavaVMs rejected: rc={} count={}", rc, count));
                return Err("Standard Invocation API did not return exactly one JavaVM");
            }
            let vm = vms[0];
            let iface = *vm;
            if iface.is_null() { return Err("JavaVM has no Invocation API table"); }
            let mut env: *mut c_void = ptr::null_mut();
            let rc = ((*iface).get_env)(vm, &mut env, JNI_VERSION_1_8);
            let mut owns_attachment = false;
            match rc {
                JNI_OK if !env.is_null() => {},
                JNI_EDETACHED => { // not an unsupported-version/other error
                    let thread_name = CString::new("VibeClientThread").unwrap();
                    let mut args = JavaVMAttachArgs {
                        version: JNI_VERSION_1_8, name: thread_name.as_ptr(), group: ptr::null_mut(),
                    };
                    let rc = ((*iface).attach_current_thread_as_daemon)(
                        vm, &mut env, &mut args as *mut _ as *mut c_void,
                    );
                    if rc != JNI_OK { return Err("AttachCurrentThreadAsDaemon failed"); }
                    owns_attachment = true;
                },
                _ => return Err("GetEnv failed; attachment not attempted for this error"),
            }
            let mut verified: *mut c_void = ptr::null_mut();
            let rc = ((*iface).get_env)(vm, &mut verified, JNI_VERSION_1_8);
            if rc != JNI_OK || env.is_null() || verified != env {
                if owns_attachment { ((*iface).detach_current_thread)(vm); }
                return Err("GetEnv could not verify the attached thread's JNIEnv");
            }
            self.jvm_ptr = vm;
            self.env_ptr = env;
            self.owner_thread = Some(std::thread::current().id());
            self.owns_attachment = owns_attachment;
            let e = self.env();
            log_msg(&format!("[JNI] Attached using standard Invocation API; env=0x{:X}", env as usize));
            log_msg("[JNI] Bootstrap: GetVersion");
            let version = jni_get_version(e);
            if version < JNI_VERSION_1_8 {
                self.detach_owned_thread();
                return Err("JNI GetVersion returned an unsupported version");
            }
            log_msg("[JNI] Bootstrap: FindClass(java/lang/Object)");
            let object = jni_find_class(e, "java/lang/Object");
            if object.is_null() {
                self.detach_owned_thread();
                return Err("Standard FindClass bootstrap probe failed");
            }
            jni_delete_local_ref(e, object);
            if jni_push_local_frame(e, 256) != JNI_OK {
                self.detach_owned_thread();
                return Err("PushLocalFrame failed during initialization");
            }
            log_msg("[JNI] Bootstrap: resolve game class/field/method IDs");
            self.cached_ids = CachedIds::resolve(e);
            if self.cached_ids.is_none() {
                jni_pop_local_frame(e, ptr::null_mut());
                self.detach_owned_thread();
                return Err("Game IDs unavailable through standard JNI; initialization stopped");
            }
            log_msg("[JNI] All class/field/method IDs resolved successfully");

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

            jni_pop_local_frame(e, ptr::null_mut());
            self.is_connected = true;
            G_JAVAVM = vm;
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

    pub fn detach(&mut self) { self.detach_owned_thread(); }

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
