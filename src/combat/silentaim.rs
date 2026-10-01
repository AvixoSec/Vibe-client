use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{EntityType, GameSnapshot, Rotation};

pub struct SilentAim {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
}

impl Default for SilentAim {
    fn default() -> Self {
        Self::new()
    }
}

impl SilentAim {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_float("MaxFOV", "Silent aim maximum FOV radius", 45.0, 5.0, 180.0, 1.0),
                Setting::new_float("MaxDistance", "Maximum target distance in meters", 100.0, 5.0, 250.0, 5.0),
                Setting::new_mode("TargetBone", "Target priority bone", 0, &["Head", "Neck", "Pelvis"]),
                Setting::new_float("HitChance", "Hit probability percentage", 100.0, 10.0, 100.0, 5.0),
            ],
        }
    }

    pub fn calculate_silent_angles(&self, snapshot: &GameSnapshot) -> Option<(u32, Rotation)> {
        if !self.enabled {
            return None;
        }

        let max_fov = self.settings[0].as_float();
        let max_dist = self.settings[1].as_float() as f64;
        let bone_mode = self.settings[2].as_mode_index();
        let hit_chance = self.settings[3].as_float();

        let eye_pos = snapshot.camera.position;
        let cam_rot = snapshot.camera.rotation;

        let mut closest_target: Option<(u32, Rotation, f32)> = None;

        for entity in &snapshot.entities {
            if entity.id == snapshot.local_player.entity.id {
                continue;
            }

            let is_target = match entity.entity_type {
                EntityType::Player => entity.is_alive && !entity.is_sleeping,
                EntityType::Scientist | EntityType::HeavyScientist => entity.is_alive,
                _ => false,
            };

            if !is_target {
                continue;
            }

            let target_pos = match bone_mode {
                0 => entity.bounding_box.head_position(),
                1 => entity.bounding_box.neck_position(),
                _ => entity.bounding_box.pelvis_position(),
            };

            let dist = eye_pos.distance_to(&target_pos);
            if dist > max_dist {
                continue;
            }

            let desired_rot = eye_pos.angle_to(&target_pos);
            let fov = cam_rot.fov_to(&desired_rot);

            if fov <= max_fov {
                if let Some((_, _, best_fov)) = closest_target {
                    if fov < best_fov {
                        closest_target = Some((entity.id, desired_rot, fov));
                    }
                } else {
                    closest_target = Some((entity.id, desired_rot, fov));
                }
            }
        }

        if let Some((id, rot, _)) = closest_target {
            if hit_chance >= 99.0 || (snapshot.tick_count % 100) as f32 <= hit_chance {
                Some((id, rot))
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl Module for SilentAim {
    fn name(&self) -> &'static str {
        "SilentAim"
    }

    fn description(&self) -> &'static str {
        "Directs bullet trajectories towards target bone without client camera displacement"
    }

    fn category(&self) -> ModuleCategory {
        ModuleCategory::Combat
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
