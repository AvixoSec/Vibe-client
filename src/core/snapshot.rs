use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vector3 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn distance_to(&self, other: &Self) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2) + (self.z - other.z).powi(2)).sqrt()
    }

    pub fn distance_sq(&self, other: &Self) -> f64 {
        (self.x - other.x).powi(2) + (self.y - other.y).powi(2) + (self.z - other.z).powi(2)
    }

    pub fn length(&self) -> f64 {
        (self.x.powi(2) + self.y.powi(2) + self.z.powi(2)).sqrt()
    }

    pub fn length_horizontal(&self) -> f64 {
        (self.x.powi(2) + self.z.powi(2)).sqrt()
    }

    pub fn normalize(&self) -> Self {
        let len = self.length();
        if len > 1e-6 {
            Self { x: self.x / len, y: self.y / len, z: self.z / len }
        } else {
            Self::ZERO
        }
    }

    pub fn dot(&self, other: &Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(&self, other: &Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    pub fn lerp(&self, other: &Self, t: f64) -> Self {
        let factor = t.clamp(0.0, 1.0);
        Self {
            x: self.x + (other.x - self.x) * factor,
            y: self.y + (other.y - self.y) * factor,
            z: self.z + (other.z - self.z) * factor,
        }
    }

    pub fn angle_to(&self, target: &Self) -> Rotation {
        let delta = Self {
            x: target.x - self.x,
            y: target.y - self.y,
            z: target.z - self.z,
        };
        let hyp = (delta.x * delta.x + delta.z * delta.z).sqrt();
        let yaw = (-delta.x).atan2(delta.z).to_degrees() as f32;
        let pitch = (-(delta.y)).atan2(hyp).to_degrees() as f32;
        Rotation::new(pitch, yaw)
    }
}

impl std::ops::Add for Vector3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self { x: self.x + rhs.x, y: self.y + rhs.y, z: self.z + rhs.z }
    }
}

impl std::ops::Sub for Vector3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self { x: self.x - rhs.x, y: self.y - rhs.y, z: self.z - rhs.z }
    }
}

impl std::ops::Mul<f64> for Vector3 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self { x: self.x * rhs, y: self.y * rhs, z: self.z * rhs }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance_to(&self, other: &Self) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }

    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let factor = t.clamp(0.0, 1.0);
        Self {
            x: self.x + (other.x - self.x) * factor,
            y: self.y + (other.y - self.y) * factor,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rotation {
    pub pitch: f32,
    pub yaw: f32,
}

impl Rotation {
    pub const ZERO: Self = Self { pitch: 0.0, yaw: 0.0 };

    pub fn new(pitch: f32, yaw: f32) -> Self {
        Self { pitch, yaw }.clamped()
    }

    pub fn clamped(&self) -> Self {
        let pitch = self.pitch.clamp(-89.9, 89.9);
        let mut yaw = self.yaw % 360.0;
        if yaw > 180.0 {
            yaw -= 360.0;
        } else if yaw < -180.0 {
            yaw += 360.0;
        }
        Self { pitch, yaw }
    }

    pub fn delta_to(&self, other: &Self) -> Self {
        let pitch_diff = other.pitch - self.pitch;
        let mut yaw_diff = (other.yaw - self.yaw) % 360.0;
        if yaw_diff > 180.0 {
            yaw_diff -= 360.0;
        } else if yaw_diff < -180.0 {
            yaw_diff += 360.0;
        }
        Self { pitch: pitch_diff, yaw: yaw_diff }
    }

    pub fn fov_to(&self, other: &Self) -> f32 {
        let delta = self.delta_to(other);
        (delta.pitch.powi(2) + delta.yaw.powi(2)).sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: Vector3,
    pub max: Vector3,
}

impl BoundingBox {
    pub fn new(min: Vector3, max: Vector3) -> Self {
        Self { min, max }
    }

    pub fn from_center_and_size(center: Vector3, width: f64, height: f64) -> Self {
        let half_w = width * 0.5;
        Self {
            min: Vector3::new(center.x - half_w, center.y, center.z - half_w),
            max: Vector3::new(center.x + half_w, center.y + height, center.z + half_w),
        }
    }

    pub fn center(&self) -> Vector3 {
        Vector3::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    pub fn head_position(&self) -> Vector3 {
        Vector3::new(
            (self.min.x + self.max.x) * 0.5,
            self.max.y - 0.2,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    pub fn neck_position(&self) -> Vector3 {
        Vector3::new(
            (self.min.x + self.max.x) * 0.5,
            self.max.y - 0.45,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    pub fn pelvis_position(&self) -> Vector3 {
        Vector3::new(
            (self.min.x + self.max.x) * 0.5,
            self.min.y + (self.max.y - self.min.y) * 0.45,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    pub fn intersects_ray(&self, origin: Vector3, direction: Vector3, max_dist: f64) -> bool {
        let dir = direction.normalize();
        let inv_x = if dir.x != 0.0 { 1.0 / dir.x } else { f64::INFINITY };
        let inv_y = if dir.y != 0.0 { 1.0 / dir.y } else { f64::INFINITY };
        let inv_z = if dir.z != 0.0 { 1.0 / dir.z } else { f64::INFINITY };

        let t1 = (self.min.x - origin.x) * inv_x;
        let t2 = (self.max.x - origin.x) * inv_x;
        let t3 = (self.min.y - origin.y) * inv_y;
        let t4 = (self.max.y - origin.y) * inv_y;
        let t5 = (self.min.z - origin.z) * inv_z;
        let t6 = (self.max.z - origin.z) * inv_z;

        let tmin = t1.min(t2).max(t3.min(t4)).max(t5.min(t6));
        let tmax = t1.max(t2).min(t3.max(t4)).min(t5.max(t6));

        if tmax < 0.0 || tmin > tmax {
            return false;
        }

        tmin <= max_dist
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityType {
    Player,
    Sleeper,
    Corpse,
    Backpack,
    Scientist,
    HeavyScientist,
    Zombie,
    Helicopter,
    Car,
    Boat,
    ResourceOre,
    WoodTree,
    Crate,
    Airdrop,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: u32,
    pub entity_type: EntityType,
    pub name: String,
    pub position: Vector3,
    pub prev_position: Vector3,
    pub velocity: Vector3,
    pub rotation: Rotation,
    pub health: f32,
    pub max_health: f32,
    pub is_alive: bool,
    pub is_sleeping: bool,
    pub is_dead: bool,
    pub despawn_timer_ticks: u32,
    pub bounding_box: BoundingBox,
    pub held_item: Option<Item>,
    pub armor_slots: [Option<Item>; 7],
}

impl Entity {
    pub fn new_player(id: u32, name: &str, pos: Vector3, health: f32) -> Self {
        let bbox = BoundingBox::from_center_and_size(pos, 0.6, 1.8);
        Self {
            id,
            entity_type: EntityType::Player,
            name: name.to_string(),
            position: pos,
            prev_position: pos,
            velocity: Vector3::ZERO,
            rotation: Rotation::ZERO,
            health,
            max_health: 100.0,
            is_alive: health > 0.0,
            is_sleeping: false,
            is_dead: health <= 0.0,
            despawn_timer_ticks: 0,
            bounding_box: bbox,
            held_item: None,
            armor_slots: [None, None, None, None, None, None, None],
        }
    }

    pub fn new_corpse(id: u32, player_name: &str, pos: Vector3) -> Self {
        let bbox = BoundingBox::from_center_and_size(pos, 0.8, 0.35);
        Self {
            id,
            entity_type: EntityType::Corpse,
            name: format!("Corpse ({})", player_name),
            position: pos,
            prev_position: pos,
            velocity: Vector3::ZERO,
            rotation: Rotation::ZERO,
            health: 0.0,
            max_health: 100.0,
            is_alive: false,
            is_sleeping: false,
            is_dead: true,
            despawn_timer_ticks: 12000,
            bounding_box: bbox,
            held_item: None,
            armor_slots: [None, None, None, None, None, None, None],
        }
    }

    pub fn new_backpack(id: u32, pos: Vector3, remaining_ticks: u32) -> Self {
        let bbox = BoundingBox::from_center_and_size(pos, 0.5, 0.4);
        Self {
            id,
            entity_type: EntityType::Backpack,
            name: "Backpack".to_string(),
            position: pos,
            prev_position: pos,
            velocity: Vector3::ZERO,
            rotation: Rotation::ZERO,
            health: 0.0,
            max_health: 50.0,
            is_alive: false,
            is_sleeping: false,
            is_dead: true,
            despawn_timer_ticks: remaining_ticks,
            bounding_box: bbox,
            held_item: None,
            armor_slots: [None, None, None, None, None, None, None],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemCategory {
    Weapon,
    Ammo,
    Armor,
    Resource,
    Tool,
    Medical,
    Deployable,
    Misc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    pub name: String,
    pub category: ItemCategory,
    pub count: u32,
    pub slot_index: u16,
    pub durability: f32,
    pub max_durability: f32,
    pub damage: f32,
    pub protection_value: f32,
}

impl Item {
    pub fn new(id: u32, name: &str, category: ItemCategory, count: u32, slot: u16) -> Self {
        Self {
            id,
            name: name.to_string(),
            category,
            count,
            slot_index: slot,
            durability: 100.0,
            max_durability: 100.0,
            damage: 0.0,
            protection_value: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub items: Vec<Item>,
    pub hotbar: [Option<Item>; 9],
    pub armor: [Option<Item>; 7],
    pub container_items: Vec<Item>,
    pub is_container_open: bool,
    pub container_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub position: Vector3,
    pub rotation: Rotation,
    pub fov: f32,
    pub view_matrix: [[f32; 4]; 4],
    pub projection_matrix: [[f32; 4]; 4],
    pub viewport_width: u32,
    pub viewport_height: u32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vector3::ZERO,
            rotation: Rotation::ZERO,
            fov: 70.0,
            view_matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            projection_matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            viewport_width: 1920,
            viewport_height: 1080,
        }
    }
}

impl Camera {
    pub fn world_to_screen(&self, world_pos: Vector3) -> Option<Vector2> {
        let rel_x = (world_pos.x - self.position.x) as f32;
        let rel_y = (world_pos.y - self.position.y) as f32;
        let rel_z = (world_pos.z - self.position.z) as f32;

        let yaw_rad = self.rotation.yaw.to_radians();
        let pitch_rad = self.rotation.pitch.to_radians();

        let cos_yaw = yaw_rad.cos();
        let sin_yaw = yaw_rad.sin();
        let cos_pitch = pitch_rad.cos();
        let sin_pitch = pitch_rad.sin();

        let x1 = rel_x * cos_yaw + rel_z * sin_yaw;
        let z1 = -rel_x * sin_yaw + rel_z * cos_yaw;

        let y2 = rel_y * cos_pitch - z1 * sin_pitch;
        let z2 = rel_y * sin_pitch + z1 * cos_pitch;

        if z2 <= 0.05 {
            return None;
        }

        let half_fov_rad = (self.fov * 0.5).to_radians();
        let tan_half_fov = half_fov_rad.tan();
        if tan_half_fov == 0.0 {
            return None;
        }

        let aspect = (self.viewport_width as f32) / (self.viewport_height as f32);

        let screen_x = (self.viewport_width as f32 * 0.5) * (1.0 + (x1 / (z2 * tan_half_fov * aspect)));
        let screen_y = (self.viewport_height as f32 * 0.5) * (1.0 - (y2 / (z2 * tan_half_fov)));

        Some(Vector2::new(screen_x, screen_y))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalPlayer {
    pub entity: Entity,
    pub is_grounded: bool,
    pub is_in_water: bool,
    pub is_sprinting: bool,
    pub is_sneaking: bool,
    pub fall_distance: f32,
    pub hunger: f32,
    pub thirst: f32,
}

impl Default for LocalPlayer {
    fn default() -> Self {
        Self {
            entity: Entity::new_player(1, "LocalPlayer", Vector3::ZERO, 100.0),
            is_grounded: true,
            is_in_water: false,
            is_sprinting: false,
            is_sneaking: false,
            fall_distance: 0.0,
            hunger: 100.0,
            thirst: 100.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub timestamp_ms: u64,
    pub tick_count: u64,
    pub local_player: LocalPlayer,
    pub entities: Vec<Entity>,
    pub inventory: Inventory,
    pub camera: Camera,
    pub world_time: u64,
}

impl GameSnapshot {
    pub fn find_entity(&self, id: u32) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id)
    }

    pub fn get_alive_players(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter().filter(|e| e.entity_type == EntityType::Player && e.is_alive)
    }

    pub fn get_lootable_entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter().filter(|e| {
            matches!(
                e.entity_type,
                EntityType::Corpse | EntityType::Sleeper | EntityType::Backpack | EntityType::Crate | EntityType::Airdrop
            )
        })
    }
}
