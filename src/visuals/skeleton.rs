use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{Camera, Entity, EntityType, GameSnapshot, Vector2, Vector3};

#[derive(Debug, Clone)]
pub struct SkeletonBoneLine {
    pub start: Vector2,
    pub end: Vector2,
}

#[derive(Debug, Clone)]
pub struct EntitySkeletonRender {
    pub entity_id: u32,
    pub bones: Vec<SkeletonBoneLine>,
    pub tracer_line: Option<(Vector2, Vector2)>,
    pub color_rgba: [f32; 4],
}

pub struct SkeletonESP {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    pub cached_skeletons: Vec<EntitySkeletonRender>,
}

impl Default for SkeletonESP {
    fn default() -> Self {
        Self::new()
    }
}

impl SkeletonESP {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_bool("RenderBones", "Render skeletal bone lines", true),
                Setting::new_bool("RenderTracers", "Render tracer lines from screen bottom", false),
                Setting::new_mode("TracerOrigin", "Tracer origin point on screen", 0, &["Bottom", "Crosshair"]),
                Setting::new_float("MaxDistance", "Maximum skeleton render distance", 100.0, 10.0, 200.0, 5.0),
            ],
            cached_skeletons: Vec::new(),
        }
    }

    pub fn build_skeleton_for_entity(
        entity: &Entity,
        camera: &Camera,
        render_bones: bool,
        render_tracers: bool,
        tracer_origin_mode: usize,
    ) -> Option<EntitySkeletonRender> {
        let pos = entity.position;
        let h = entity.bounding_box.max.y - entity.bounding_box.min.y;

        let head = Vector3::new(pos.x, pos.y + h * 0.92, pos.z);
        let neck = Vector3::new(pos.x, pos.y + h * 0.80, pos.z);
        let l_shoulder = Vector3::new(pos.x - 0.22, pos.y + h * 0.78, pos.z);
        let r_shoulder = Vector3::new(pos.x + 0.22, pos.y + h * 0.78, pos.z);
        let l_hand = Vector3::new(pos.x - 0.32, pos.y + h * 0.45, pos.z);
        let r_hand = Vector3::new(pos.x + 0.32, pos.y + h * 0.45, pos.z);
        let pelvis = Vector3::new(pos.x, pos.y + h * 0.48, pos.z);
        let l_knee = Vector3::new(pos.x - 0.16, pos.y + h * 0.25, pos.z);
        let r_knee = Vector3::new(pos.x + 0.16, pos.y + h * 0.25, pos.z);
        let l_foot = Vector3::new(pos.x - 0.18, pos.y, pos.z);
        let r_foot = Vector3::new(pos.x + 0.18, pos.y, pos.z);

        let pairs = [
            (head, neck),
            (neck, l_shoulder),
            (neck, r_shoulder),
            (l_shoulder, l_hand),
            (r_shoulder, r_hand),
            (neck, pelvis),
            (pelvis, l_knee),
            (pelvis, r_knee),
            (l_knee, l_foot),
            (r_knee, r_foot),
        ];

        let mut bones = Vec::new();
        if render_bones {
            for (p1, p2) in &pairs {
                if let (Some(s1), Some(s2)) = (camera.world_to_screen(*p1), camera.world_to_screen(*p2)) {
                    bones.push(SkeletonBoneLine { start: s1, end: s2 });
                }
            }
        }

        let mut tracer = None;
        if render_tracers {
            if let Some(target_screen) = camera.world_to_screen(pelvis) {
                let origin = match tracer_origin_mode {
                    0 => Vector2::new(camera.viewport_width as f32 * 0.5, camera.viewport_height as f32),
                    _ => Vector2::new(camera.viewport_width as f32 * 0.5, camera.viewport_height as f32 * 0.5),
                };
                tracer = Some((origin, target_screen));
            }
        }

        if bones.is_empty() && tracer.is_none() {
            return None;
        }

        Some(EntitySkeletonRender {
            entity_id: entity.id,
            bones,
            tracer_line: tracer,
            color_rgba: [0.0, 0.9, 0.74, 0.85],
        })
    }

    pub fn update_skeletons(&mut self, snapshot: &GameSnapshot) {
        if !self.enabled {
            self.cached_skeletons.clear();
            return;
        }

        let render_bones = self.settings[0].as_bool();
        let render_tracers = self.settings[1].as_bool();
        let tracer_mode = self.settings[2].as_mode_index();
        let max_dist = self.settings[3].as_float() as f64;

        let mut list = Vec::new();

        for entity in &snapshot.entities {
            if entity.id == snapshot.local_player.entity.id || !entity.is_alive {
                continue;
            }

            if entity.entity_type != EntityType::Player && entity.entity_type != EntityType::Scientist {
                continue;
            }

            if snapshot.camera.position.distance_to(&entity.position) > max_dist {
                continue;
            }

            if let Some(skel) = Self::build_skeleton_for_entity(
                entity,
                &snapshot.camera,
                render_bones,
                render_tracers,
                tracer_mode,
            ) {
                list.push(skel);
            }
        }

        self.cached_skeletons = list;
    }
}

impl Module for SkeletonESP {
    fn name(&self) -> &'static str {
        "SkeletonESP"
    }

    fn description(&self) -> &'static str {
        "Projects 3D player bone hierarchy and snapline tracers to screen"
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
            self.cached_skeletons.clear();
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
            self.update_skeletons(snapshot);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
