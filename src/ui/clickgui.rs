use std::collections::HashSet;
use crate::core::events::ClientEvent;
use crate::core::module::{ModuleCategory, SettingValue};
use crate::core::registry::ModuleRegistry;
use crate::core::snapshot::Vector2;
use crate::ui::theme::Theme;

#[derive(Debug, Clone)]
pub struct GuiWindow {
    pub category: ModuleCategory,
    pub pos: Vector2,
    pub is_dragging: bool,
    pub drag_offset: Vector2,
    pub is_collapsed: bool,
    pub scroll_offset: f32,
}

pub struct ClickGUI {
    pub is_open: bool,
    pub toggle_key: u32,
    pub windows: Vec<GuiWindow>,
    pub expanded_modules: HashSet<String>,
    pub binding_module_name: Option<String>,
    pub search_query: String,
    pub is_search_active: bool,
    pub mouse_pos: Vector2,
}

impl Default for ClickGUI {
    fn default() -> Self {
        Self::new()
    }
}

impl ClickGUI {
    pub fn new() -> Self {
        let categories = [
            ModuleCategory::Combat,
            ModuleCategory::Visual,
            ModuleCategory::Movement,
            ModuleCategory::Farm,
            ModuleCategory::World,
            ModuleCategory::Misc,
        ];

        let mut windows = Vec::new();
        let mut x = 40.0;
        let y = 60.0;
        let spacing = Theme::WINDOW_WIDTH + 18.0;

        for cat in categories {
            windows.push(GuiWindow {
                category: cat,
                pos: Vector2::new(x, y),
                is_dragging: false,
                drag_offset: Vector2::ZERO,
                is_collapsed: false,
                scroll_offset: 0.0,
            });
            x += spacing;
        }

        Self {
            is_open: false,
            toggle_key: 0xA1, // VK_RSHIFT (Right Shift)
            windows,
            expanded_modules: HashSet::new(),
            binding_module_name: None,
            search_query: String::new(),
            is_search_active: false,
            mouse_pos: Vector2::ZERO,
        }
    }

    pub fn toggle(&mut self) -> bool {
        self.is_open = !self.is_open;
        if !self.is_open {
            self.binding_module_name = None;
            self.is_search_active = false;
        }
        self.is_open
    }

    pub fn handle_mouse_click(
        &mut self,
        mouse_pos: Vector2,
        button: u32, // 0 = left, 1 = right
        pressed: bool,
        registry: &mut ModuleRegistry,
    ) -> bool {
        self.mouse_pos = mouse_pos;
        if !self.is_open {
            return false;
        }

        if !pressed {
            for win in &mut self.windows {
                win.is_dragging = false;
            }
            return false;
        }

        // Check search bar click
        let search_rect_min = Vector2::new(40.0, 15.0);
        let search_rect_max = Vector2::new(340.0, 45.0);
        if mouse_pos.x >= search_rect_min.x && mouse_pos.x <= search_rect_max.x
            && mouse_pos.y >= search_rect_min.y && mouse_pos.y <= search_rect_max.y {
            self.is_search_active = true;
            return true;
        } else {
            self.is_search_active = false;
        }

        for win in self.windows.iter_mut().rev() {
            let win_min = win.pos;
            let win_max = Vector2::new(win.pos.x + Theme::WINDOW_WIDTH, win.pos.y + Theme::HEADER_HEIGHT);

            if mouse_pos.x >= win_min.x && mouse_pos.x <= win_max.x
                && mouse_pos.y >= win_min.y && mouse_pos.y <= win_max.y {
                if button == 0 {
                    win.is_dragging = true;
                    win.drag_offset = Vector2::new(mouse_pos.x - win.pos.x, mouse_pos.y - win.pos.y);
                } else if button == 1 {
                    win.is_collapsed = !win.is_collapsed;
                }
                return true;
            }

            if win.is_collapsed {
                continue;
            }

            let mut current_y = win.pos.y + Theme::HEADER_HEIGHT + Theme::PADDING;
            let module_names: Vec<String> = registry
                .get_by_category(win.category)
                .into_iter()
                .map(|m| m.name().to_string())
                .collect();

            for mod_name in module_names {
                let item_min = Vector2::new(win.pos.x + Theme::PADDING, current_y);
                let item_max = Vector2::new(win.pos.x + Theme::WINDOW_WIDTH - Theme::PADDING, current_y + Theme::ITEM_HEIGHT);

                if mouse_pos.x >= item_min.x && mouse_pos.x <= item_max.x
                    && mouse_pos.y >= item_min.y && mouse_pos.y <= item_max.y {
                    if button == 0 {
                        // Left click: toggle module
                        if let Some(m) = registry.get_mut(&mod_name) {
                            m.toggle();
                        }
                    } else if button == 1 {
                        // Right click: expand settings
                        if self.expanded_modules.contains(&mod_name) {
                            self.expanded_modules.remove(&mod_name);
                        } else {
                            self.expanded_modules.insert(mod_name.clone());
                        }
                    }
                    return true;
                }

                current_y += Theme::ITEM_HEIGHT + 2.0;

                if self.expanded_modules.contains(&mod_name) {
                    if let Some(m) = registry.get_mut(&mod_name) {
                        for setting in m.settings_mut() {
                            let set_min = Vector2::new(win.pos.x + Theme::PADDING * 2.0, current_y);
                            let set_max = Vector2::new(win.pos.x + Theme::WINDOW_WIDTH - Theme::PADDING * 2.0, current_y + Theme::SETTING_HEIGHT);

                            if mouse_pos.x >= set_min.x && mouse_pos.x <= set_max.x
                                && mouse_pos.y >= set_min.y && mouse_pos.y <= set_max.y {
                                match &mut setting.value {
                                    SettingValue::Boolean(b) => *b = !*b,
                                    SettingValue::Mode { selected, options } => {
                                        if !options.is_empty() {
                                            *selected = (*selected + 1) % options.len();
                                        }
                                    }
                                    SettingValue::Float { value, min, max, step } => {
                                        let ratio = ((mouse_pos.x - set_min.x) / (set_max.x - set_min.x)).clamp(0.0, 1.0);
                                        let raw = *min + ratio * (*max - *min);
                                        let stepped = (raw / *step).round() * *step;
                                        *value = stepped.clamp(*min, *max);
                                    }
                                    SettingValue::Integer { value, min, max } => {
                                        let ratio = ((mouse_pos.x - set_min.x) / (set_max.x - set_min.x)).clamp(0.0, 1.0);
                                        let raw = *min as f32 + ratio * (*max - *min) as f32;
                                        *value = raw.round() as i32;
                                    }
                                    SettingValue::Keybind(_) => {
                                        self.binding_module_name = Some(mod_name.clone());
                                    }
                                }
                                return true;
                            }

                            current_y += Theme::SETTING_HEIGHT + 2.0;
                        }
                    }
                }
            }
        }

        false
    }

    pub fn handle_mouse_move(&mut self, mouse_pos: Vector2) {
        self.mouse_pos = mouse_pos;
        if !self.is_open {
            return;
        }

        for win in &mut self.windows {
            if win.is_dragging {
                win.pos.x = mouse_pos.x - win.drag_offset.x;
                win.pos.y = mouse_pos.y - win.drag_offset.y;
            }
        }
    }

    pub fn handle_key_press(&mut self, key_code: u32, registry: &mut ModuleRegistry) -> bool {
        if key_code == self.toggle_key {
            self.toggle();
            return true;
        }

        if !self.is_open {
            return false;
        }

        if let Some(target_mod) = &self.binding_module_name {
            if let Some(m) = registry.get_mut(target_mod) {
                if key_code == 0x1B { // VK_ESCAPE
                    m.set_keybind(0);
                } else {
                    m.set_keybind(key_code);
                }
            }
            self.binding_module_name = None;
            return true;
        }

        if self.is_search_active {
            if key_code == 0x08 { // Backspace
                self.search_query.pop();
                return true;
            } else if key_code == 0x1B || key_code == 0x0D { // Escape or Enter
                self.is_search_active = false;
                return true;
            } else if key_code >= 0x41 && key_code <= 0x5A { // A-Z
                let c = (key_code as u8) as char;
                self.search_query.push(c);
                return true;
            }
        }

        false
    }

    pub fn on_event(&mut self, event: &mut ClientEvent, registry: &mut ModuleRegistry) {
        match event {
            ClientEvent::KeyEvent { key_code, pressed } => {
                if *pressed {
                    self.handle_key_press(*key_code, registry);
                }
            }
            _ => {}
        }
    }
}
