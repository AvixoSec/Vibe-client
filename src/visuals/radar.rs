use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{EntityType, GameSnapshot, Vector2};

#[derive(Debug, Clone)]
pub struct RadarBlip {
    pub offset_x: f32,
    pub offset_y: f32,
    pub color_rgba: [f32; 4],
    pub entity_type: EntityType,
}

pub struct Radar {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    pub cached_blips: Vec<RadarBlip>,
}

impl Default for Radar {
    fn default() -> Self {
        Self::new()
    }
}

impl Radar {
    pub fn new() -> Self {
        Self {
            enabled: true,
            keybind: 0,
            settings: vec![
                Setting::new_float("RadarRadius", "Radius of radar HUD in pixels", 75.0, 40.0, 150.0, 5.0),
                Setting::new_float("MaxRange", "World range in meters covered by radar", 120.0, 30.0, 300.0, 10.0),
                Setting::new_bool("RotateWithPlayer", "Rotate radar so player faces UP", true),
                Setting::new_float("PosX", "Radar X position on screen", 100.0, 20.0, 1920.0, 10.0),
                Setting::new_float("PosY", "Radar Y position on screen", 100.0, 20.0, 1080.0, 10.0),
            ],
            cached_blips: Vec::new(),
        }
    }

    pub fn update_blips(&mut self, snapshot: &GameSnapshot) {
        if !self.enabled {
            self.cached_blips.clear();
            return;
        }

        let radius = self.settings[0].as_float();
        let max_range = self.settings[1].as_float() as f64;
        let rotate_player = self.settings[2].as_bool();

        let local_pos = snapshot.local_player.entity.position;
        let player_yaw_rad = if rotate_player {
            snapshot.camera.rotation.yaw.to_radians()
        } else {
            0.0
        };

        let cos_yaw = (player_yaw_rad as f64).cos();
        let sin_yaw = (player_yaw_rad as f64).sin();

        let mut blips = Vec::new();

        for entity in &snapshot.entities {
            if entity.id == snapshot.local_player.entity.id {
                continue;
            }

            let dx = entity.position.x - local_pos.x;
            let dz = entity.position.z - local_pos.z;

            let rot_x = dx * cos_yaw - dz * sin_yaw;
            let rot_z = dx * sin_yaw + dz * cos_yaw;

            let dist = (rot_x * rot_x + rot_z * rot_z).sqrt();
            if dist > max_range {
                continue;
            }

            let scale = radius as f64 / max_range;
            let px = (rot_x * scale) as f32;
            let py = (rot_z * scale) as f32;

            let color = match entity.entity_type {
                EntityType::Player => [0.0, 0.9, 0.74, 1.0], // Turquoise
                EntityType::Sleeper => [0.4, 0.6, 0.9, 0.8], // Soft blue
                EntityType::Corpse => [0.9, 0.2, 0.2, 0.9], // Red
                EntityType::Backpack => [1.0, 0.8, 0.2, 0.9], // Amber
                EntityType::Scientist | EntityType::HeavyScientist => [0.2, 0.4, 0.9, 1.0],
                EntityType::Airdrop => [0.9, 0.1, 0.9, 1.0],
                _ => [0.7, 0.7, 0.7, 0.7],
            };

            blips.push(RadarBlip {
                offset_x: px,
                offset_y: py,
                color_rgba: color,
                entity_type: entity.entity_type,
            });
        }

        self.cached_blips = blips;
    }

    pub fn get_center(&self) -> Vector2 {
        Vector2::new(self.settings[3].as_float(), self.settings[4].as_float())
    }

    pub fn get_radius(&self) -> f32 {
        self.settings[0].as_float()
    }
}

impl Module for Radar {
    fn name(&self) -> &'static str {
        "Radar"
    }

    fn description(&self) -> &'static str {
        "2D HUD compass radar with player rotation and entity tracking"
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
            self.cached_blips.clear();
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
            self.update_blips(snapshot);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
