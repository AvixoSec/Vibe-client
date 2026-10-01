use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::registry::ModuleRegistry;
use crate::core::snapshot::GameSnapshot;
use crate::ui::theme::{Color, Theme};

#[derive(Debug, Clone)]
pub struct HudModuleEntry {
    pub name: &'static str,
    pub color: Color,
}

pub struct HUD {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    pub cached_active_modules: Vec<HudModuleEntry>,
    pub watermark_text: String,
    pub coords_text: String,
}

impl Default for HUD {
    fn default() -> Self {
        Self::new()
    }
}

impl HUD {
    pub fn new() -> Self {
        Self {
            enabled: true,
            keybind: 0,
            settings: vec![
                Setting::new_bool("Watermark", "Display client watermark and build info", true),
                Setting::new_bool("ArrayList", "Display list of active modules", true),
                Setting::new_bool("Coordinates", "Display position coordinates and biome", true),
                Setting::new_bool("Keybinds", "Display active keybinds monitor", true),
                Setting::new_mode("ColorMode", "ArrayList color scheme", 0, &["TurquoiseGradient", "Rainbow", "Static"]),
            ],
            cached_active_modules: Vec::new(),
            watermark_text: "Vibe v0.1.0 | RustMe Vanilla | 60 FPS".to_string(),
            coords_text: String::new(),
        }
    }

    pub fn update_hud_state(&mut self, registry: &ModuleRegistry, snapshot: &GameSnapshot) {
        if !self.enabled {
            self.cached_active_modules.clear();
            return;
        }

        let show_arraylist = self.settings[1].as_bool();
        let show_coords = self.settings[2].as_bool();
        let color_mode = self.settings[4].as_mode_index();

        if show_coords {
            let pos = snapshot.local_player.entity.position;
            let rot = snapshot.camera.rotation;
            self.coords_text = format!(
                "XYZ: {:.1} / {:.1} / {:.1} | Yaw: {:.1} Pitch: {:.1}",
                pos.x, pos.y, pos.z, rot.yaw, rot.pitch
            );
        }

        if show_arraylist {
            let mut active: Vec<&'static str> = registry
                .all()
                .iter()
                .filter(|m| m.is_enabled() && m.name() != "HUD")
                .map(|m| m.name())
                .collect();

            // Sort by string length descending
            active.sort_by(|a, b| b.len().cmp(&a.len()));

            let count = active.len();
            let mut list = Vec::new();

            for (i, name) in active.into_iter().enumerate() {
                let color = match color_mode {
                    0 => {
                        // Turquoise gradient
                        let factor = if count > 1 { i as f32 / (count - 1) as f32 } else { 0.0 };
                        Color::new(
                            0.0 * (1.0 - factor) + 0.15 * factor,
                            0.9 * (1.0 - factor) + 0.65 * factor,
                            0.74 * (1.0 - factor) + 0.95 * factor,
                            1.0,
                        )
                    }
                    1 => {
                        // Rainbow
                        let hue = ((i as f32 * 35.0 + snapshot.tick_count as f32 * 2.0) % 360.0) / 360.0;
                        let (r, g, b) = hsv_to_rgb(hue, 0.8, 1.0);
                        Color::new(r, g, b, 1.0)
                    }
                    _ => Theme::ACCENT,
                };

                list.push(HudModuleEntry { name, color });
            }

            self.cached_active_modules = list;
        }
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);

    match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

impl Module for HUD {
    fn name(&self) -> &'static str {
        "HUD"
    }

    fn description(&self) -> &'static str {
        "In-game HUD overlay with watermark, arraylist and coordinates"
    }

    fn category(&self) -> ModuleCategory {
        ModuleCategory::Visual
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.cached_active_modules.clear();
        }
    }

    fn keybind(&self) -> u32 {
        self.keybind
    }

    fn set_keybind(&mut self, key: u32) {
        self.keybind = key;
    }

    fn settings(&self) -> &[Setting] {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut [Setting] {
        &mut self.settings
    }

    fn on_event(&mut self, _event: &mut ClientEvent) {}

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
