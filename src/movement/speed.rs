use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::Vector3;

pub struct Speed {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
}

impl Default for Speed {
    fn default() -> Self {
        Self::new()
    }
}

impl Speed {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_mode("Mode", "Movement speed algorithm", 0, &["VanillaSafe", "Bhop", "Rage"]),
                Setting::new_float("Multiplier", "Speed multiplier", 1.25, 1.05, 3.5, 0.05),
                Setting::new_bool("OnlyOnGround", "Only apply boost when player is grounded", true),
            ],
        }
    }

    pub fn calculate_modified_velocity(&self, current_vel: Vector3, is_grounded: bool) -> Vector3 {
        if !self.enabled {
            return current_vel;
        }

        let only_ground = self.settings[2].as_bool();
        if only_ground && !is_grounded {
            return current_vel;
        }

        let mult = self.settings[1].as_float() as f64;
        let mode = self.settings[0].as_mode_index();

        match mode {
            0 => {
                // VanillaSafe: modest horizontal acceleration, clamp max to stay under RMAC threshold
                let max_h_speed = 0.35 * mult;
                let cur_h = current_vel.length_horizontal();
                if cur_h > 0.01 {
                    let factor = (cur_h * mult).min(max_h_speed) / cur_h;
                    Vector3::new(current_vel.x * factor, current_vel.y, current_vel.z * factor)
                } else {
                    current_vel
                }
            }
            1 => {
                // Bhop: add small vertical hop when moving on ground
                if is_grounded && current_vel.length_horizontal() > 0.05 {
                    Vector3::new(current_vel.x * mult, 0.38, current_vel.z * mult)
                } else {
                    Vector3::new(current_vel.x * (mult * 0.95), current_vel.y, current_vel.z * (mult * 0.95))
                }
            }
            _ => {
                // Rage: direct multiplier
                Vector3::new(current_vel.x * mult, current_vel.y, current_vel.z * mult)
            }
        }
    }
}

impl Module for Speed {
    fn name(&self) -> &'static str {
        "Speed"
    }

    fn description(&self) -> &'static str {
        "Modifies player movement velocity with safe and bhop modes"
    }

    fn category(&self) -> ModuleCategory {
        ModuleCategory::Movement
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
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
