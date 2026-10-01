use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{EntityType, GameSnapshot, Vector2};

#[derive(Debug, Clone)]
pub struct WorldEspItem {
    pub name: String,
    pub screen_pos: Vector2,
    pub distance_m: f32,
    pub color_rgba: [f32; 4],
}

pub struct WorldESP {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    pub cached_items: Vec<WorldEspItem>,
}

impl Default for WorldESP {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldESP {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_bool("SulfurOre", "Show sulfur resource nodes", true),
                Setting::new_bool("MetalOre", "Show metal resource nodes", true),
                Setting::new_bool("StoneOre", "Show stone resource nodes", false),
                Setting::new_bool("Crates", "Show loot crates and barrels", true),
                Setting::new_bool("Airdrops", "Show incoming airdrop supply drops", true),
                Setting::new_float("MaxDistance", "Maximum world ESP render distance", 200.0, 20.0, 500.0, 10.0),
            ],
            cached_items: Vec::new(),
        }
    }

    pub fn update_world_items(&mut self, snapshot: &GameSnapshot) {
        if !self.enabled {
            self.cached_items.clear();
            return;
        }

        let show_sulfur = self.settings[0].as_bool();
        let show_metal = self.settings[1].as_bool();
        let show_stone = self.settings[2].as_bool();
        let show_crates = self.settings[3].as_bool();
        let show_airdrops = self.settings[4].as_bool();
        let max_dist = self.settings[5].as_float() as f64;

        let mut list = Vec::new();
        let cam_pos = snapshot.camera.position;

        for entity in &snapshot.entities {
            let dist = cam_pos.distance_to(&entity.position);
            if dist > max_dist {
                continue;
            }

            let name_lower = entity.name.to_lowercase();

            let (should_show, color) = match entity.entity_type {
                EntityType::ResourceOre => {
                    if name_lower.contains("sulfur") || name_lower.contains("сера") {
                        (show_sulfur, [1.0, 0.9, 0.1, 0.9]) // Bright Yellow
                    } else if name_lower.contains("metal") || name_lower.contains("железо") {
                        (show_metal, [0.8, 0.5, 0.2, 0.9]) // Copper Orange
                    } else {
                        (show_stone, [0.6, 0.6, 0.6, 0.8]) // Stone Gray
                    }
                }
                EntityType::Crate => (show_crates, [0.2, 0.8, 0.3, 0.9]), // Green
                EntityType::Airdrop => (show_airdrops, [0.9, 0.1, 0.9, 1.0]), // Bright Magenta
                _ => (false, [1.0, 1.0, 1.0, 1.0]),
            };

            if should_show {
                if let Some(screen_pos) = snapshot.camera.world_to_screen(entity.position) {
                    list.push(WorldEspItem {
                        name: entity.name.clone(),
                        screen_pos,
                        distance_m: dist as f32,
                        color_rgba: color,
                    });
                }
            }
        }

        self.cached_items = list;
    }
}

impl Module for WorldESP {
    fn name(&self) -> &'static str {
        "WorldESP"
    }

    fn description(&self) -> &'static str {
        "ESP overlay for Sulfur, Metal, Stone ores, Crates, and Airdrops"
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
            self.cached_items.clear();
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

    fn on_event(&mut self, event: &mut ClientEvent) {
        if let ClientEvent::GameTick { snapshot, .. } = event {
            self.update_world_items(snapshot);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
