use crate::core::arbiter::{ActionArbiter, ActionPayload, ActionPriority};
use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{EntityType, GameSnapshot, Vector3};

pub struct TriggerBot {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    last_trigger_tick: u64,
}

impl Default for TriggerBot {
    fn default() -> Self {
        Self::new()
    }
}

impl TriggerBot {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0x5A, // 'Z'
            settings: vec![
                Setting::new_float("MaxDistance", "Maximum trigger distance in meters", 60.0, 5.0, 150.0, 5.0),
                Setting::new_int("DelayTicks", "Click delay in ticks before firing", 1, 0, 10),
                Setting::new_bool("HeadshotsOnly", "Only trigger when crosshair is on head", false),
                Setting::new_bool("TriggerNPCs", "Trigger on scientist NPCs", true),
            ],
            last_trigger_tick: 0,
        }
    }

    pub fn check_trigger(
        &mut self,
        snapshot: &GameSnapshot,
        arbiter: &mut ActionArbiter,
        current_tick: u64,
    ) -> Option<u32> {
        if !self.enabled {
            return None;
        }

        let max_dist = self.settings[0].as_float() as f64;
        let delay_ticks = self.settings[1].as_int() as u64;
        let headshots_only = self.settings[2].as_bool();
        let trigger_npcs = self.settings[3].as_bool();

        if self.last_trigger_tick > 0 && current_tick < self.last_trigger_tick + delay_ticks {
            return None;
        }

        let eye_pos = snapshot.camera.position;
        let rot = snapshot.camera.rotation;

        let yaw_rad = rot.yaw.to_radians();
        let pitch_rad = rot.pitch.to_radians();

        let dir = Vector3::new(
            -pitch_rad.cos() as f64 * yaw_rad.sin() as f64,
            -pitch_rad.sin() as f64,
            pitch_rad.cos() as f64 * yaw_rad.cos() as f64,
        ).normalize();

        for entity in &snapshot.entities {
            let is_valid = match entity.entity_type {
                EntityType::Player => entity.is_alive && entity.id != snapshot.local_player.entity.id,
                EntityType::Scientist | EntityType::HeavyScientist => trigger_npcs && entity.is_alive,
                _ => false,
            };

            if !is_valid {
                continue;
            }

            if eye_pos.distance_to(&entity.position) > max_dist {
                continue;
            }

            let hit = if headshots_only {
                let head_pos = entity.bounding_box.head_position();
                let head_bbox = crate::core::snapshot::BoundingBox::from_center_and_size(head_pos, 0.35, 0.35);
                head_bbox.intersects_ray(eye_pos, dir, max_dist)
            } else {
                entity.bounding_box.intersects_ray(eye_pos, dir, max_dist)
            };

            if hit {
                self.last_trigger_tick = current_tick;
                arbiter.submit_action(
                    ActionPriority::Combat,
                    ActionPayload::AttackEntity {
                        target_id: entity.id,
                        is_headshot: headshots_only,
                    },
                );
                return Some(entity.id);
            }
        }

        None
    }
}

impl Module for TriggerBot {
    fn name(&self) -> &'static str {
        "TriggerBot"
    }

    fn description(&self) -> &'static str {
        "Automatically fires when crosshair intersects an enemy hitbox"
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
