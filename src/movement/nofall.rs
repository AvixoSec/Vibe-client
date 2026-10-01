use crate::core::arbiter::{ActionArbiter, ActionPayload, ActionPriority};
use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::GameSnapshot;

pub struct NoFall {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
}

impl Default for NoFall {
    fn default() -> Self {
        Self::new()
    }
}

impl NoFall {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_float("TriggerDistance", "Fall distance threshold to trigger spoof", 2.8, 1.5, 5.0, 0.1),
                Setting::new_mode("Mode", "NoFall spoof technique", 0, &["PacketGround", "Catch"]),
            ],
        }
    }

    pub fn check_and_apply(&self, snapshot: &GameSnapshot, arbiter: &mut ActionArbiter) -> bool {
        if !self.enabled {
            return false;
        }

        let threshold = self.settings[0].as_float();
        if snapshot.local_player.fall_distance > threshold {
            arbiter.submit_action(
                ActionPriority::Emergency,
                ActionPayload::GroundSpoof { on_ground: true },
            );
            return true;
        }

        false
    }
}

impl Module for NoFall {
    fn name(&self) -> &'static str {
        "NoFall"
    }

    fn description(&self) -> &'static str {
        "Prevents taking fall damage by spoofing ground packets before impact"
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
