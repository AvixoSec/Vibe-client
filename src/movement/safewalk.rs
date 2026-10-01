use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::Vector3;

pub struct SafeWalk {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
}

impl Default for SafeWalk {
    fn default() -> Self {
        Self::new()
    }
}

impl SafeWalk {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_bool("EdgeSneak", "Automatically simulate sneak when approaching cliff edge", true),
                Setting::new_float("EdgeThreshold", "Edge detection distance in meters", 0.45, 0.2, 0.8, 0.05),
            ],
        }
    }

    pub fn check_edge(&self, pos: Vector3, velocity: Vector3, is_grounded: bool) -> bool {
        if !self.enabled || !is_grounded {
            return false;
        }

        let threshold = self.settings[1].as_float() as f64;
        let next_x = pos.x + velocity.x.clamp(-threshold, threshold);
        let next_z = pos.z + velocity.z.clamp(-threshold, threshold);

        // Simple edge check: if moving off horizontal block boundary
        let cur_block_x = pos.x.floor();
        let cur_block_z = pos.z.floor();
        let next_block_x = next_x.floor();
        let next_block_z = next_z.floor();

        (cur_block_x != next_block_x || cur_block_z != next_block_z) && velocity.y <= 0.0
    }
}

impl Module for SafeWalk {
    fn name(&self) -> &'static str {
        "SafeWalk"
    }

    fn description(&self) -> &'static str {
        "Prevents stepping off edges and falling from heights"
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
