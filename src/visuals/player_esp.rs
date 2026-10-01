use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{Camera, Entity, EntityType, GameSnapshot, Vector2, Vector3};

#[derive(Debug, Clone)]
pub struct EspRenderItem {
    pub entity_id: u32,
    pub entity_type: EntityType,
    pub name: String,
    pub screen_box: Option<(Vector2, Vector2)>, // top_left, bottom_right
    pub health_fraction: f32,
    pub distance_m: f32,
    pub held_item_name: Option<String>,
    pub extra_info: Option<String>,
    pub color_rgba: [f32; 4],
}

pub struct PlayerESP {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    pub cached_render_list: Vec<EspRenderItem>,
}

impl Default for PlayerESP {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerESP {
    pub fn new() -> Self {
        Self {
            enabled: true,
            keybind: 0x4F, // 'O'
            settings: vec![
                Setting::new_mode("BoxStyle", "Bounding box visual style", 0, &["Corner", "Full", "None"]),
                Setting::new_bool("HealthBar", "Show dynamic health bar", true),
                Setting::new_bool("Distance", "Show distance in meters", true),
                Setting::new_bool("NameTags", "Show entity nickname", true),
                Setting::new_bool("HeldWeapon", "Show active held weapon", true),
                Setting::new_bool("Sleepers", "Show sleeping players", true),
                Setting::new_bool("Corpses", "Show player corpses with 10min timer", true),
                Setting::new_bool("Backpacks", "Show backpacks with 30min timer", true),
                Setting::new_bool("Scientists", "Show NPC scientists", true),
                Setting::new_float("MaxDistance", "Maximum ESP render distance", 250.0, 20.0, 500.0, 10.0),
            ],
            cached_render_list: Vec::new(),
        }
    }

    pub fn calculate_screen_bounds(&self, entity: &Entity, camera: &Camera) -> Option<(Vector2, Vector2)> {
        let bbox = entity.bounding_box;

        let corners = [
            Vector3::new(bbox.min.x, bbox.min.y, bbox.min.z),
            Vector3::new(bbox.max.x, bbox.min.y, bbox.min.z),
            Vector3::new(bbox.min.x, bbox.min.y, bbox.max.z),
            Vector3::new(bbox.max.x, bbox.min.y, bbox.max.z),
            Vector3::new(bbox.min.x, bbox.max.y, bbox.min.z),
            Vector3::new(bbox.max.x, bbox.max.y, bbox.min.z),
            Vector3::new(bbox.min.x, bbox.max.y, bbox.max.z),
            Vector3::new(bbox.max.x, bbox.max.y, bbox.max.z),
        ];

        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;

        let mut any_visible = false;

        for corner in &corners {
            if let Some(screen_pt) = camera.world_to_screen(*corner) {
                any_visible = true;
                min_x = min_x.min(screen_pt.x);
                min_y = min_y.min(screen_pt.y);
                max_x = max_x.max(screen_pt.x);
                max_y = max_y.max(screen_pt.y);
            }
        }

        if !any_visible || max_x <= min_x || max_y <= min_y {
            return None;
        }

        Some((Vector2::new(min_x, min_y), Vector2::new(max_x, max_y)))
    }

    pub fn update_render_list(&mut self, snapshot: &GameSnapshot) {
        if !self.enabled {
            self.cached_render_list.clear();
            return;
        }

        let show_sleepers = self.settings[5].as_bool();
        let show_corpses = self.settings[6].as_bool();
        let show_backpacks = self.settings[7].as_bool();
        let show_scientists = self.settings[8].as_bool();
        let max_dist = self.settings[9].as_float() as f64;

        let mut list = Vec::new();
        let cam_pos = snapshot.camera.position;

        for entity in &snapshot.entities {
            if entity.id == snapshot.local_player.entity.id {
                continue;
            }

            let dist = cam_pos.distance_to(&entity.position);
            if dist > max_dist {
                continue;
            }

            let should_render = match entity.entity_type {
                EntityType::Player => true,
                EntityType::Sleeper => show_sleepers,
                EntityType::Corpse => show_corpses,
                EntityType::Backpack => show_backpacks,
                EntityType::Scientist | EntityType::HeavyScientist => show_scientists,
                _ => false,
            };

            if !should_render {
                continue;
            }

            let screen_box = self.calculate_screen_bounds(entity, &snapshot.camera);

            let color = match entity.entity_type {
                EntityType::Player => [0.0, 0.9, 0.74, 1.0], // Turquoise
                EntityType::Sleeper => [0.4, 0.6, 0.9, 0.85], // Soft blue
                EntityType::Corpse => [0.9, 0.2, 0.2, 0.9], // Red
                EntityType::Backpack => [1.0, 0.8, 0.2, 0.9], // Golden amber
                EntityType::Scientist | EntityType::HeavyScientist => [0.2, 0.4, 0.9, 1.0], // Deep Blue
                _ => [1.0, 1.0, 1.0, 0.8],
            };

            let extra_info = match entity.entity_type {
                EntityType::Corpse => crate::visuals::classifier::EntityClassifier::get_despawn_text(
                    EntityType::Corpse,
                    entity.despawn_timer_ticks,
                ),
                EntityType::Backpack => crate::visuals::classifier::EntityClassifier::get_despawn_text(
                    EntityType::Backpack,
                    entity.despawn_timer_ticks,
                ),
                EntityType::Sleeper => Some("Sleeping".to_string()),
                _ => None,
            };

            list.push(EspRenderItem {
                entity_id: entity.id,
                entity_type: entity.entity_type,
                name: entity.name.clone(),
                screen_box,
                health_fraction: (entity.health / entity.max_health).clamp(0.0, 1.0),
                distance_m: dist as f32,
                held_item_name: entity.held_item.as_ref().map(|i| i.name.clone()),
                extra_info,
                color_rgba: color,
            });
        }

        self.cached_render_list = list;
    }
}

impl Module for PlayerESP {
    fn name(&self) -> &'static str {
        "PlayerESP"
    }

    fn description(&self) -> &'static str {
        "2D bounding boxes, health, held weapon, sleepers, corpses and backpacks"
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
            self.cached_render_list.clear();
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
            self.update_render_list(snapshot);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
