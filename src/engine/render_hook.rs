#![allow(dead_code)]
//! OpenGL overlay renderer — hooks wglSwapBuffers to draw the cheat UI.

use std::ffi::{c_void, CString};
use std::sync::Mutex;

use crate::engine::renderer::DrawCommand;

// ══════════════════════════════════════════════════════════════════════
// OpenGL Constants
// ══════════════════════════════════════════════════════════════════════

const GL_QUADS: u32 = 0x0007;
const GL_LINES: u32 = 0x0001;
const GL_LINE_LOOP: u32 = 0x0002;
const GL_BLEND: u32 = 0x0BE2;
const GL_SRC_ALPHA: u32 = 0x0302;
const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_DEPTH_TEST: u32 = 0x0B71;
const GL_PROJECTION: u32 = 0x1701;
const GL_MODELVIEW: u32 = 0x1700;
const GL_ALL_ATTRIB_BITS: u32 = 0x000FFFFF;
const GL_CURRENT_PROGRAM: u32 = 0x8B8D;
const GL_VERTEX_ARRAY_BINDING: u32 = 0x85B5;
const GL_ARRAY_BUFFER_BINDING: u32 = 0x8894;
const GL_ARRAY_BUFFER: u32 = 0x8892;

// ══════════════════════════════════════════════════════════════════════
// OpenGL Function Pointers
// ══════════════════════════════════════════════════════════════════════

#[allow(non_snake_case)]
struct GlFunctions {
    glBegin: unsafe extern "system" fn(u32),
    glEnd: unsafe extern "system" fn(),
    glVertex2f: unsafe extern "system" fn(f32, f32),
    glColor4f: unsafe extern "system" fn(f32, f32, f32, f32),
    glLineWidth: unsafe extern "system" fn(f32),
    glEnable: unsafe extern "system" fn(u32),
    glDisable: unsafe extern "system" fn(u32),
    glBlendFunc: unsafe extern "system" fn(u32, u32),
    glPushMatrix: unsafe extern "system" fn(),
    glPopMatrix: unsafe extern "system" fn(),
    glPushAttrib: unsafe extern "system" fn(u32),
    glPopAttrib: unsafe extern "system" fn(),
    glMatrixMode: unsafe extern "system" fn(u32),
    glLoadIdentity: unsafe extern "system" fn(),
    glOrtho: unsafe extern "system" fn(f64, f64, f64, f64, f64, f64),
    glGetIntegerv: unsafe extern "system" fn(u32, *mut i32),
    glViewport: unsafe extern "system" fn(i32, i32, i32, i32),
    // Bitmap font
    glRasterPos2f: unsafe extern "system" fn(f32, f32),
    glListBase: unsafe extern "system" fn(u32),
    glCallLists: unsafe extern "system" fn(i32, u32, *const c_void),
    // Modern OpenGL state unbinding
    glUseProgram: Option<unsafe extern "system" fn(u32)>,
    glBindVertexArray: Option<unsafe extern "system" fn(u32)>,
    glBindBuffer: Option<unsafe extern "system" fn(u32, u32)>,
}

static mut GL: Option<GlFunctions> = None;
static mut FONT_LIST_BASE: u32 = 0;
static mut HOOK_INSTALLED: bool = false;
static mut ORIGINAL_WGL_SWAP: usize = 0;
static mut ORIGINAL_BYTES: [u8; 14] = [0; 14]; // for x64 hook

pub static RENDER_COMMANDS: Mutex<RenderBuffer> = Mutex::new(RenderBuffer {
    commands: Vec::new(),
    strings: Vec::new(),
});

pub static VIEWPORT_WIDTH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1920);
pub static VIEWPORT_HEIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1080);

pub struct RenderBuffer {
    pub commands: Vec<DrawCommand>,
    pub strings: Vec<String>,
}

extern "system" {
    fn GetModuleHandleA(name: *const u8) -> isize;
    fn GetProcAddress(hModule: isize, lpProcName: *const u8) -> Option<unsafe extern "system" fn() -> isize>;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn VirtualAlloc(addr: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
    fn GetCurrentProcess() -> isize;
    fn FlushInstructionCache(hProcess: isize, lpBaseAddress: *const c_void, dwSize: usize) -> i32;
}

unsafe fn load_gl_fn<T: Copy>(module: isize, name: &str) -> T {
    let cname = CString::new(name).unwrap();
    let proc = GetProcAddress(module, cname.as_ptr() as *const u8);
    let raw = proc.map(|f| f as usize).unwrap_or(0);
    std::mem::transmute_copy(&raw)
}

type FnWglGetCurrentDC = unsafe extern "system" fn() -> isize;
type FnWglUseFontBitmapsA = unsafe extern "system" fn(isize, u32, u32, u32) -> i32;
type FnWglGetProcAddress = unsafe extern "system" fn(*const u8) -> *const c_void;

unsafe fn init_gl() -> bool {
    let gl32 = {
        let n = CString::new("opengl32.dll").unwrap();
        GetModuleHandleA(n.as_ptr() as *const u8)
    };
    if gl32 == 0 { return false; }

    let wgl_gpa: FnWglGetProcAddress = load_gl_fn(gl32, "wglGetProcAddress");
    let load_ext = |name: &str| -> Option<usize> {
        let cname = CString::new(name).ok()?;
        let p = wgl_gpa(cname.as_ptr() as *const u8);
        if p.is_null() || p as usize == 1 || p as usize == 2 || p as usize == 3 || p as usize == usize::MAX {
            None
        } else {
            Some(p as usize)
        }
    };

    let fn_use_prog: Option<unsafe extern "system" fn(u32)> = load_ext("glUseProgram").map(|p| std::mem::transmute(p));
    let fn_bind_vao: Option<unsafe extern "system" fn(u32)> = load_ext("glBindVertexArray").map(|p| std::mem::transmute(p));
    let fn_bind_buf: Option<unsafe extern "system" fn(u32, u32)> = load_ext("glBindBuffer").map(|p| std::mem::transmute(p));

    GL = Some(GlFunctions {
        glBegin: load_gl_fn(gl32, "glBegin"),
        glEnd: load_gl_fn(gl32, "glEnd"),
        glVertex2f: load_gl_fn(gl32, "glVertex2f"),
        glColor4f: load_gl_fn(gl32, "glColor4f"),
        glLineWidth: load_gl_fn(gl32, "glLineWidth"),
        glEnable: load_gl_fn(gl32, "glEnable"),
        glDisable: load_gl_fn(gl32, "glDisable"),
        glBlendFunc: load_gl_fn(gl32, "glBlendFunc"),
        glPushMatrix: load_gl_fn(gl32, "glPushMatrix"),
        glPopMatrix: load_gl_fn(gl32, "glPopMatrix"),
        glPushAttrib: load_gl_fn(gl32, "glPushAttrib"),
        glPopAttrib: load_gl_fn(gl32, "glPopAttrib"),
        glMatrixMode: load_gl_fn(gl32, "glMatrixMode"),
        glLoadIdentity: load_gl_fn(gl32, "glLoadIdentity"),
        glOrtho: load_gl_fn(gl32, "glOrtho"),
        glGetIntegerv: load_gl_fn(gl32, "glGetIntegerv"),
        glViewport: load_gl_fn(gl32, "glViewport"),
        glRasterPos2f: load_gl_fn(gl32, "glRasterPos2f"),
        glListBase: load_gl_fn(gl32, "glListBase"),
        glCallLists: load_gl_fn(gl32, "glCallLists"),
        glUseProgram: fn_use_prog,
        glBindVertexArray: fn_bind_vao,
        glBindBuffer: fn_bind_buf,
    });

    // Init bitmap font dynamically
    let wgl_get_dc: FnWglGetCurrentDC = load_gl_fn(gl32, "wglGetCurrentDC");
    let wgl_use_font_bitmaps: FnWglUseFontBitmapsA = load_gl_fn(gl32, "wglUseFontBitmapsA");
    let hdc = wgl_get_dc();
    if hdc != 0 {
        let ok = wgl_use_font_bitmaps(hdc, 0, 256, 1000);
        if ok != 0 {
            FONT_LIST_BASE = 1000;
            crate::engine::jni_bridge::log_msg("[RENDER] Font bitmaps created (base 1000)");
        } else {
            crate::engine::jni_bridge::log_msg("[RENDER] wglUseFontBitmapsA returned 0, text rendering disabled");
        }
    }

    true
}

// ══════════════════════════════════════════════════════════════════════
// Overlay Rendering
// ══════════════════════════════════════════════════════════════════════

type FnWglGetCurrentContext = unsafe extern "system" fn() -> isize;

unsafe fn render_overlay() {
    let gl32 = match GetModuleHandleA(b"opengl32.dll\0".as_ptr()) {
        0 => return,
        h => h,
    };
    let wgl_get_ctx: FnWglGetCurrentContext = load_gl_fn(gl32, "wglGetCurrentContext");
    if wgl_get_ctx() == 0 {
        return; // No active OpenGL context on this thread yet
    }

    let gl = match unsafe { (*(&raw const GL)).as_ref() } {
        Some(g) => g,
        None => {
            if !init_gl() { return; }
            match unsafe { (*(&raw const GL)).as_ref() } {
                Some(g) => g,
                None => return,
            }
        }
    };

    // Get viewport
    let mut viewport = [0i32; 4];
    (gl.glGetIntegerv)(0x0BA2, viewport.as_mut_ptr()); // GL_VIEWPORT
    let w = viewport[2] as f64;
    let h = viewport[3] as f64;
    if w < 1.0 || h < 1.0 { return; }

    VIEWPORT_WIDTH.store(viewport[2] as u32, std::sync::atomic::Ordering::Relaxed);
    VIEWPORT_HEIGHT.store(viewport[3] as u32, std::sync::atomic::Ordering::Relaxed);

    // Lock and clone the command buffer (try_lock to avoid blocking game thread)
    let (commands, strings) = {
        let lock = match RENDER_COMMANDS.try_lock() {
            Ok(l) => l,
            Err(_) => return,
        };
        (lock.commands.clone(), lock.strings.clone())
    };
    if commands.is_empty() { return; }

    // Save and unbind modern OpenGL pipeline state to avoid conflicts in AMD/NVIDIA drivers
    let mut last_prog: i32 = 0;
    let mut last_vao: i32 = 0;
    let mut last_vbo: i32 = 0;

    (gl.glGetIntegerv)(GL_CURRENT_PROGRAM, &mut last_prog);
    (gl.glGetIntegerv)(GL_VERTEX_ARRAY_BINDING, &mut last_vao);
    (gl.glGetIntegerv)(GL_ARRAY_BUFFER_BINDING, &mut last_vbo);

    if let Some(f) = gl.glUseProgram {
        if last_prog != 0 { f(0); }
    }
    if let Some(f) = gl.glBindVertexArray {
        if last_vao != 0 { f(0); }
    }
    if let Some(f) = gl.glBindBuffer {
        if last_vbo != 0 { f(GL_ARRAY_BUFFER, 0); }
    }

    // Save state
    (gl.glPushAttrib)(GL_ALL_ATTRIB_BITS);
    (gl.glMatrixMode)(GL_PROJECTION);
    (gl.glPushMatrix)();
    (gl.glLoadIdentity)();
    (gl.glOrtho)(0.0, w, h, 0.0, -1.0, 1.0);
    (gl.glMatrixMode)(GL_MODELVIEW);
    (gl.glPushMatrix)();
    (gl.glLoadIdentity)();

    (gl.glDisable)(GL_DEPTH_TEST);
    (gl.glDisable)(GL_TEXTURE_2D);
    (gl.glEnable)(GL_BLEND);
    (gl.glBlendFunc)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

    // Draw commands
    for cmd in &commands {
        match cmd {
            DrawCommand::Rect { min, max, color } => {
                (gl.glColor4f)(color.r, color.g, color.b, color.a);
                (gl.glBegin)(GL_QUADS);
                (gl.glVertex2f)(min.x, min.y);
                (gl.glVertex2f)(max.x, min.y);
                (gl.glVertex2f)(max.x, max.y);
                (gl.glVertex2f)(min.x, max.y);
                (gl.glEnd)();
            }
            DrawCommand::RectOutline { min, max, thickness, color } => {
                (gl.glColor4f)(color.r, color.g, color.b, color.a);
                (gl.glLineWidth)(*thickness);
                (gl.glBegin)(GL_LINE_LOOP);
                (gl.glVertex2f)(min.x, min.y);
                (gl.glVertex2f)(max.x, min.y);
                (gl.glVertex2f)(max.x, max.y);
                (gl.glVertex2f)(min.x, max.y);
                (gl.glEnd)();
            }
            DrawCommand::Line { start, end, thickness, color } => {
                (gl.glColor4f)(color.r, color.g, color.b, color.a);
                (gl.glLineWidth)(*thickness);
                (gl.glBegin)(GL_LINES);
                (gl.glVertex2f)(start.x, start.y);
                (gl.glVertex2f)(end.x, end.y);
                (gl.glEnd)();
            }
            DrawCommand::Circle { center, radius, color } => {
                (gl.glColor4f)(color.r, color.g, color.b, color.a);
                (gl.glBegin)(GL_LINE_LOOP);
                for i in 0..24 {
                    let angle = (i as f32) * std::f32::consts::TAU / 24.0;
                    (gl.glVertex2f)(center.x + radius * angle.cos(), center.y + radius * angle.sin());
                }
                (gl.glEnd)();
            }
            DrawCommand::CornerBox { min, max, length, color } => {
                (gl.glColor4f)(color.r, color.g, color.b, color.a);
                (gl.glLineWidth)(1.5);
                let l = *length;
                (gl.glBegin)(GL_LINES);
                // Top-left corner
                (gl.glVertex2f)(min.x, min.y); (gl.glVertex2f)(min.x + l, min.y);
                (gl.glVertex2f)(min.x, min.y); (gl.glVertex2f)(min.x, min.y + l);
                // Top-right corner
                (gl.glVertex2f)(max.x, min.y); (gl.glVertex2f)(max.x - l, min.y);
                (gl.glVertex2f)(max.x, min.y); (gl.glVertex2f)(max.x, min.y + l);
                // Bottom-left corner
                (gl.glVertex2f)(min.x, max.y); (gl.glVertex2f)(min.x + l, max.y);
                (gl.glVertex2f)(min.x, max.y); (gl.glVertex2f)(min.x, max.y - l);
                // Bottom-right corner
                (gl.glVertex2f)(max.x, max.y); (gl.glVertex2f)(max.x - l, max.y);
                (gl.glVertex2f)(max.x, max.y); (gl.glVertex2f)(max.x, max.y - l);
                (gl.glEnd)();
            }
            DrawCommand::Text { text_idx, pos, color } => {
                let base = *(&raw const FONT_LIST_BASE);
                if base != 0 {
                    if let Some(text) = strings.get(*text_idx) {
                        (gl.glColor4f)(color.r, color.g, color.b, color.a);
                        (gl.glRasterPos2f)(pos.x, pos.y + 12.0);
                        (gl.glListBase)(base);
                        (gl.glCallLists)(text.len() as i32, 0x1401, text.as_ptr() as *const c_void); // GL_UNSIGNED_BYTE
                    }
                }
            }
        }
    }

    // Restore state
    (gl.glMatrixMode)(GL_MODELVIEW);
    (gl.glPopMatrix)();
    (gl.glMatrixMode)(GL_PROJECTION);
    (gl.glPopMatrix)();
    (gl.glPopAttrib)();

    // Restore modern OpenGL state
    if let Some(f) = gl.glBindBuffer {
        if last_vbo != 0 { f(GL_ARRAY_BUFFER, last_vbo as u32); }
    }
    if let Some(f) = gl.glBindVertexArray {
        if last_vao != 0 { f(last_vao as u32); }
    }
    if let Some(f) = gl.glUseProgram {
        if last_prog != 0 { f(last_prog as u32); }
    }
}

// ══════════════════════════════════════════════════════════════════════
// wglSwapBuffers Hook (x64 absolute JMP via stolen bytes + RIP fixup)
// ══════════════════════════════════════════════════════════════════════

type FnSwapBuffers = unsafe extern "system" fn(isize) -> i32;

static mut OVERLAY_DISABLED: bool = false;
static FRAME_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

unsafe extern "C" fn render_overlay_entry() {
    render_overlay();
}

unsafe extern "system" fn hooked_wgl_swap_buffers(hdc: isize) -> i32 {
    let f = FRAME_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if f == 0 || f == 60 || f == 300 {
        crate::engine::jni_bridge::log_msg(&format!("[RENDER] wglSwapBuffers frame {}", f));
    }

    if !*(&raw const OVERLAY_DISABLED) {
        let ok = crate::engine::jni_bridge::guarded_render_call(render_overlay_entry);
        if !ok {
            OVERLAY_DISABLED = true;
            crate::engine::jni_bridge::log_msg("[RENDER] Hardware exception caught — overlay disabled to preserve game stability");
        }
    }

    let orig = *(&raw const ORIGINAL_WGL_SWAP);
    if orig != 0 {
        let original: FnSwapBuffers = std::mem::transmute(orig);
        original(hdc)
    } else {
        0
    }
}

/// Install the wglSwapBuffers hook with RIP-relative instruction relocation.
pub unsafe fn install_hook() -> bool {
    if *(&raw const HOOK_INSTALLED) { return true; }

    let gl32 = match GetModuleHandleA(b"opengl32.dll\0".as_ptr()) {
        0 => return false,
        h => h,
    };

    let swap_name = CString::new("wglSwapBuffers").unwrap();
    let swap_addr = match GetProcAddress(gl32, swap_name.as_ptr() as *const u8) {
        Some(f) => f as usize,
        None => return false,
    };
    if swap_addr == 0 { return false; }

    // Inspect first 14 bytes of wglSwapBuffers
    let swap_bytes = std::slice::from_raw_parts(swap_addr as *const u8, 14);

    // Check if already hooked (starts with jmp [rip+0]: FF 25)
    if swap_bytes[0] == 0xFF && swap_bytes[1] == 0x25 {
        crate::engine::jni_bridge::log_msg("[RENDER] wglSwapBuffers is already hooked, skipping installation");
        HOOK_INSTALLED = true;
        return true;
    }

    // Allocate trampoline buffer (executable)
    let trampoline = VirtualAlloc(
        std::ptr::null_mut(),
        64,
        0x3000, // MEM_COMMIT | MEM_RESERVE
        0x40,   // PAGE_EXECUTE_READWRITE
    );
    if trampoline.is_null() { return false; }

    std::ptr::copy_nonoverlapping(swap_addr as *const u8, (&raw mut ORIGINAL_BYTES) as *mut u8, 14);

    let mut tramp_len: usize = 14;

    // Check if byte 7 is: 48 8B 05 <disp32> (mov rax, [rip + disp32])
    // This is the standard Windows opengl32!wglSwapBuffers instruction.
    if swap_bytes[7] == 0x48 && swap_bytes[8] == 0x8B && swap_bytes[9] == 0x05 {
        let disp = i32::from_le_bytes(swap_bytes[10..14].try_into().unwrap()) as isize;
        let target_var = (swap_addr + 14).wrapping_add(disp as usize);
        crate::engine::jni_bridge::log_msg(&format!(
            "[RENDER] Relocating RIP-relative instruction: target=0x{:X}",
            target_var
        ));

        // Copy first 7 bytes: 40 55 57 48 83 EC 58
        std::ptr::copy_nonoverlapping(swap_addr as *const u8, trampoline as *mut u8, 7);

        // At offset 7, emit absolute load:
        // mov rax, target_var  => 48 B8 <8 bytes> (10 bytes)
        // mov rax, [rax]       => 48 8B 00        (3 bytes)
        let t = (trampoline as usize + 7) as *mut u8;
        *t = 0x48;
        *t.add(1) = 0xB8;
        *(t.add(2) as *mut u64) = target_var as u64;
        *t.add(10) = 0x48;
        *t.add(11) = 0x8B;
        *t.add(12) = 0x00;

        tramp_len = 20; // 7 + 10 + 3
    } else {
        std::ptr::copy_nonoverlapping((&raw const ORIGINAL_BYTES) as *const u8, trampoline as *mut u8, 14);
    }

    // Add jump back to swap_addr + 14
    let ret_addr = swap_addr + 14;
    let tramp_jmp = (trampoline as usize + tramp_len) as *mut u8;
    *tramp_jmp = 0xFF;
    *tramp_jmp.add(1) = 0x25;
    *(tramp_jmp.add(2) as *mut u32) = 0;
    *(tramp_jmp.add(6) as *mut u64) = ret_addr as u64;

    let tramp_addr = trampoline as usize;
    ORIGINAL_WGL_SWAP = tramp_addr;

    // Overwrite wglSwapBuffers entry with JMP to our hook: FF 25 00 00 00 00 <hooked_wgl_swap_buffers>
    let mut old_protect: u32 = 0;
    VirtualProtect(swap_addr as *mut c_void, 14, 0x40, &mut old_protect);

    let hook_target = swap_addr as *mut u8;
    *hook_target = 0xFF;
    *hook_target.add(1) = 0x25;
    *(hook_target.add(2) as *mut u32) = 0;
    *(hook_target.add(6) as *mut u64) = hooked_wgl_swap_buffers as *const () as u64;

    VirtualProtect(swap_addr as *mut c_void, 14, old_protect, &mut old_protect);

    let cur_proc = GetCurrentProcess();
    FlushInstructionCache(cur_proc, swap_addr as *const c_void, 14);
    FlushInstructionCache(cur_proc, trampoline, 64);

    HOOK_INSTALLED = true;
    crate::engine::jni_bridge::log_msg(&format!(
        "[RENDER] wglSwapBuffers hook installed safely (trampoline @ 0x{:X})",
        tramp_addr
    ));
    true
}

/// Submit a frame of draw commands for the render hook to display.
pub fn submit_frame(commands: &[DrawCommand], strings: &[String]) {
    if let Ok(mut lock) = RENDER_COMMANDS.lock() {
        lock.commands.clear();
        lock.commands.extend_from_slice(commands);
        lock.strings.clear();
        lock.strings.extend_from_slice(strings);
    }
}
