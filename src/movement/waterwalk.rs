use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::Vector3;

pub struct WaterWalk {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
}

impl Default for WaterWalk {
    fn default() -> Self {
        Self::new()
    }
}

impl WaterWalk {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_mode("Mode", "Water surface interaction mode", 0, &["Solid", "Bounce", "Dolphin"]),
                Setting::new_float("VerticalOffset", "Height offset above water surface", 0.05, 0.0, 0.5, 0.05),
            ],
        }
    }

    pub fn modify_water_physics(&self, is_in_water: bool, mut current_vel: Vector3) -> (Vector3, bool) {
        if !self.enabled || !is_in_water {
            return (current_vel, false);
        }

        let mode = self.settings[0].as_mode_index();
        let offset = self.settings[1].as_float() as f64;

        match mode {
            0 => {
                // Solid: keep vertical velocity at or above zero, pretend to be on ground
                if current_vel.y < 0.0 {
                    current_vel.y = offset;
                }
                (current_vel, true)
            }
            1 => {
                // Bounce: apply small hop upon hitting water surface
                current_vel.y = 0.28;
                (current_vel, false)
            }
            _ => {
                // Dolphin: rapid swimming speed boost
                current_vel.x *= 1.4;
                current_vel.z *= 1.4;
                (current_vel, false)
            }
        }
    }
}

impl Module for WaterWalk {
    fn name(&self) -> &'static str {
        "WaterWalk"
    }

    fn description(&self) -> &'static str {
        "Allows walking across water surfaces as if they were solid blocks"
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
