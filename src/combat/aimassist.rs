use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{Entity, EntityType, GameSnapshot, Rotation, Vector3};

pub struct AimAssist {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    pub current_target_id: Option<u32>,
}

impl Default for AimAssist {
    fn default() -> Self {
        Self::new()
    }
}

impl AimAssist {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0x58, // 'X'
            settings: vec![
                Setting::new_float("FOV", "Maximum FOV angle to acquire target", 35.0, 5.0, 180.0, 1.0),
                Setting::new_float("Smooth", "Aim smoothing divisor (higher = slower/more human)", 6.5, 1.0, 30.0, 0.5),
                Setting::new_float("MaxDistance", "Maximum target distance in meters", 75.0, 5.0, 200.0, 5.0),
                Setting::new_mode("TargetBone", "Target priority bone", 0, &["Head", "Neck", "Pelvis"]),
                Setting::new_mode("Priority", "Target priority sorting", 0, &["Crosshair", "Distance", "LowestHP"]),
                Setting::new_bool("VisibleOnly", "Only target visible entities", true),
                Setting::new_bool("TargetSleepers", "Target sleeping players", false),
                Setting::new_bool("TargetNPCs", "Target scientist NPCs", true),
            ],
            current_target_id: None,
        }
    }

    pub fn find_best_target<'a>(&self, snapshot: &'a GameSnapshot) -> Option<(&'a Entity, Vector3)> {
        let max_fov = self.settings[0].as_float();
        let max_dist = self.settings[2].as_float() as f64;
        let bone_mode = self.settings[3].as_mode_index();
        let prio_mode = self.settings[4].as_mode_index();
        let target_sleepers = self.settings[6].as_bool();
        let target_npcs = self.settings[7].as_bool();

        let local_pos = snapshot.camera.position;
        let local_rot = snapshot.camera.rotation;

        let mut candidates: Vec<(&'a Entity, Vector3, f32, f64)> = Vec::new();

        for entity in &snapshot.entities {
            let is_valid_type = match entity.entity_type {
                EntityType::Player => entity.is_alive && (!entity.is_sleeping || target_sleepers),
                EntityType::Sleeper => target_sleepers,
                EntityType::Scientist | EntityType::HeavyScientist => target_npcs && entity.is_alive,
                _ => false,
            };

            if !is_valid_type || entity.id == snapshot.local_player.entity.id {
                continue;
            }

            let target_pos = match bone_mode {
                0 => entity.bounding_box.head_position(),
                1 => entity.bounding_box.neck_position(),
                _ => entity.bounding_box.pelvis_position(),
            };

            let dist = local_pos.distance_to(&target_pos);
            if dist > max_dist {
                continue;
            }

            let target_rot = local_pos.angle_to(&target_pos);
            let fov = local_rot.fov_to(&target_rot);
            if fov > max_fov {
                continue;
            }

            candidates.push((entity, target_pos, fov, dist));
        }

        if candidates.is_empty() {
            return None;
        }

        match prio_mode {
            0 => candidates.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap()), // Closest to crosshair
            1 => candidates.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap()), // Closest distance
            _ => candidates.sort_by(|a, b| a.0.health.partial_cmp(&b.0.health).unwrap()), // Lowest HP
        }

        let best = candidates[0];
        Some((best.0, best.1))
    }

    pub fn calculate_aim_step(&self, current_rot: Rotation, target_pos: Vector3, eye_pos: Vector3) -> Rotation {
        let desired_rot = eye_pos.angle_to(&target_pos);
        let smooth = self.settings[1].as_float().max(1.0);

        let delta = current_rot.delta_to(&desired_rot);

        let max_step = 8.0;
        let step_pitch = (delta.pitch / smooth).clamp(-max_step, max_step);
        let step_yaw = (delta.yaw / smooth).clamp(-max_step, max_step);

        Rotation::new(current_rot.pitch + step_pitch, current_rot.yaw + step_yaw)
    }
}

impl Module for AimAssist {
    fn name(&self) -> &'static str {
        "AimAssist"
    }

    fn description(&self) -> &'static str {
        "Smooth humanized target tracking with bone priority and FOV limit"
    }

    fn category(&self) -> ModuleCategory {
        ModuleCategory::Combat
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.current_target_id = None;
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
            if let Some((target, pos)) = self.find_best_target(snapshot) {
                self.current_target_id = Some(target.id);
                let _next_rot = self.calculate_aim_step(snapshot.camera.rotation, pos, snapshot.camera.position);
            } else {
                self.current_target_id = None;
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
