use crate::core::arbiter::{ActionArbiter, ActionPayload, ActionPriority};
use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{EntityType, GameSnapshot, Vector3};

pub struct AutoFarm {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    last_hit_tick: u64,
}

impl Default for AutoFarm {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoFarm {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0x4A, // 'J'
            settings: vec![
                Setting::new_float("ReachDistance", "Maximum farming reach distance", 4.5, 2.0, 7.0, 0.2),
                Setting::new_bool("TargetTrees", "Harvest wood from trees", true),
                Setting::new_bool("TargetOres", "Mine stone, sulfur and metal ores", true),
                Setting::new_bool("SweetSpotOnly", "Only hit the critical bonus sweet spot marker", true),
                Setting::new_int("SwingDelayTicks", "Ticks between tool swings", 10, 5, 25),
            ],
            last_hit_tick: 0,
        }
    }

    pub fn process_farming(
        &mut self,
        snapshot: &GameSnapshot,
        arbiter: &mut ActionArbiter,
        current_tick: u64,
    ) -> bool {
        if !self.enabled {
            return false;
        }

        let delay = self.settings[4].as_int() as u64;
        if self.last_hit_tick > 0 && current_tick < self.last_hit_tick + delay {
            return false;
        }

        let reach = self.settings[0].as_float() as f64;
        let target_trees = self.settings[1].as_bool();
        let target_ores = self.settings[2].as_bool();
        let sweet_spot = self.settings[3].as_bool();

        let player_pos = snapshot.camera.position;

        for entity in &snapshot.entities {
            let is_match = match entity.entity_type {
                EntityType::WoodTree => target_trees,
                EntityType::ResourceOre => target_ores,
                _ => false,
            };

            if !is_match {
                continue;
            }

            let dist = player_pos.distance_to(&entity.position);
            if dist > reach {
                continue;
            }

            // Calculate sweet spot target: on RustMe, sweet spot is a displaced point on the node surface
            let target_point = if sweet_spot {
                let offset_y = if entity.entity_type == EntityType::WoodTree { 1.3 } else { 0.6 };
                Vector3::new(entity.position.x + 0.2, entity.position.y + offset_y, entity.position.z)
            } else {
                entity.position
            };

            self.last_hit_tick = current_tick;
            arbiter.submit_action(
                ActionPriority::Farm,
                ActionPayload::MineBlock {
                    position: [
                        target_point.x.floor() as i32,
                        target_point.y.floor() as i32,
                        target_point.z.floor() as i32,
                    ],
                    tool_slot: 0,
                },
            );
            return true;
        }

        false
    }
}

impl Module for AutoFarm {
    fn name(&self) -> &'static str {
        "AutoFarm"
    }

    fn description(&self) -> &'static str {
        "Automatically mines ore nodes and cuts trees with sweet-spot targeting"
    }

    fn category(&self) -> ModuleCategory {
        ModuleCategory::Farm
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
