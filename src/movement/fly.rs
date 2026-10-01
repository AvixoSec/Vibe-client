use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::Vector3;

pub struct Fly {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
}

impl Default for Fly {
    fn default() -> Self {
        Self::new()
    }
}

impl Fly {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_float("VerticalSpeed", "Vertical climb/descend speed", 0.5, 0.1, 2.0, 0.1),
                Setting::new_float("HorizontalSpeed", "Horizontal flying speed", 0.8, 0.2, 3.0, 0.1),
                Setting::new_bool("Glide", "Slow descent instead of rigid hover", false),
            ],
        }
    }

    pub fn calculate_fly_velocity(&self, input_forward: f32, input_strafe: f32, input_up: f32, input_down: f32) -> Vector3 {
        if !self.enabled {
            return Vector3::ZERO;
        }

        let v_speed = self.settings[0].as_float() as f64;
        let h_speed = self.settings[1].as_float() as f64;
        let glide = self.settings[2].as_bool();

        let mut vy = (input_up as f64 - input_down as f64) * v_speed;
        if vy == 0.0 && glide {
            vy = -0.05;
        }

        let vx = (input_forward as f64) * h_speed;
        let vz = (input_strafe as f64) * h_speed;

        Vector3::new(vx, vy, vz)
    }
}

impl Module for Fly {
    fn name(&self) -> &'static str {
        "Fly"
    }

    fn description(&self) -> &'static str {
        "Enables controlled flight and aerial maneuvering"
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
