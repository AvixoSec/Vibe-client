//! Input handler — polls keyboard state for module toggles and GUI interaction.

use std::collections::HashMap;

extern "system" {
    fn GetAsyncKeyState(vKey: i32) -> i16;
    fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    fn GetForegroundWindow() -> isize;
    fn FindWindowA(lpClassName: *const u8, lpWindowName: *const u8) -> isize;
    fn ScreenToClient(hWnd: isize, lpPoint: *mut POINT) -> i32;
}

#[repr(C)]
struct POINT { x: i32, y: i32 }

// Virtual key codes
pub const VK_INSERT: i32 = 0x2D;
pub const VK_DELETE: i32 = 0x2E;
pub const VK_RSHIFT: i32 = 0xA1;
pub const VK_HOME: i32 = 0x24;
pub const VK_END: i32 = 0x23;
pub const VK_F1: i32 = 0x70;
pub const VK_F2: i32 = 0x71;
pub const VK_F3: i32 = 0x72;
pub const VK_F4: i32 = 0x73;
pub const VK_LBUTTON: i32 = 0x01;
pub const VK_RBUTTON: i32 = 0x02;

#[derive(Debug, Clone)]
pub enum InputEvent {
    ToggleGUI,
    ToggleModule(String),
    Panic, // kill switch
    MousePosition(f32, f32),
    MouseClick { button: u32, pressed: bool },
    KeyPress(u32),
}

pub struct InputHandler {
    keybinds: HashMap<i32, String>,     // VK -> module name
    prev_key_state: HashMap<i32, bool>, // for edge detection
    gui_key: i32,
    panic_key: i32,
}

impl Default for InputHandler {
    fn default() -> Self { Self::new() }
}

impl InputHandler {
    pub fn new() -> Self {
        let mut keybinds = HashMap::new();
        // Default keybinds
        keybinds.insert(VK_F1, "PlayerESP".to_string());
        keybinds.insert(VK_F2, "AimAssist".to_string());
        keybinds.insert(VK_F3, "TriggerBot".to_string());
        keybinds.insert(VK_F4, "NoFall".to_string());
        keybinds.insert(VK_HOME, "Speed".to_string());
        keybinds.insert(VK_END, "Fly".to_string());

        Self {
            keybinds,
            prev_key_state: HashMap::new(),
            gui_key: VK_INSERT,
            panic_key: VK_DELETE,
        }
    }

    pub fn set_keybind(&mut self, vk: i32, module_name: &str) {
        self.keybinds.insert(vk, module_name.to_string());
    }

    /// Poll input and return events since last call.
    pub fn poll(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();

        // Check GUI toggle (INSERT or RSHIFT)
        if self.key_pressed(self.gui_key) || self.key_pressed(VK_RSHIFT) {
            events.push(InputEvent::ToggleGUI);
        }

        // Check panic key (DELETE)
        if self.key_pressed(self.panic_key) {
            events.push(InputEvent::Panic);
        }

        // Check module keybinds
        let binds: Vec<(i32, String)> = self.keybinds.iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        for (vk, module) in binds {
            if self.key_pressed(vk) {
                events.push(InputEvent::ToggleModule(module));
            }
        }

        // Mouse position (converted to game window client coordinates)
        unsafe {
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt) != 0 {
                let mut hwnd = GetForegroundWindow();
                let mc_hwnd = FindWindowA(b"LWJGL\0".as_ptr(), std::ptr::null());
                if mc_hwnd != 0 {
                    hwnd = mc_hwnd;
                }
                let glfw_hwnd = FindWindowA(b"GLFW30\0".as_ptr(), std::ptr::null());
                if glfw_hwnd != 0 {
                    hwnd = glfw_hwnd;
                }
                if hwnd != 0 {
                    ScreenToClient(hwnd, &mut pt);
                }
                events.push(InputEvent::MousePosition(pt.x as f32, pt.y as f32));
            }
        }

        // Mouse buttons edge detection
        let l_down = self.is_key_down(VK_LBUTTON);
        let l_was_down = *self.prev_key_state.get(&VK_LBUTTON).unwrap_or(&false);
        if l_down != l_was_down {
            self.prev_key_state.insert(VK_LBUTTON, l_down);
            events.push(InputEvent::MouseClick { button: 0, pressed: l_down });
        }

        let r_down = self.is_key_down(VK_RBUTTON);
        let r_was_down = *self.prev_key_state.get(&VK_RBUTTON).unwrap_or(&false);
        if r_down != r_was_down {
            self.prev_key_state.insert(VK_RBUTTON, r_down);
            events.push(InputEvent::MouseClick { button: 1, pressed: r_down });
        }

        events
    }

    /// Returns true on the rising edge (key just pressed this frame).
    fn key_pressed(&mut self, vk: i32) -> bool {
        let down = self.is_key_down(vk);
        let was_down = *self.prev_key_state.get(&vk).unwrap_or(&false);
        self.prev_key_state.insert(vk, down);
        down && !was_down
    }

    fn is_key_down(&self, vk: i32) -> bool {
        unsafe { GetAsyncKeyState(vk) & (1 << 15) as i16 != 0 }
    }
}
