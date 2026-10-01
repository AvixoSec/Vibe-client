use crate::core::snapshot::EntityType;

pub struct EntityClassifier;

impl EntityClassifier {
    pub fn classify_from_model_name(model_name: &str) -> EntityType {
        let name = model_name.to_lowercase();

        if name.contains("heavy_scientist") {
            EntityType::HeavyScientist
        } else if name.contains("scientist") || name.contains("missile_silo_scientist") {
            EntityType::Scientist
        } else if name.contains("sleeper") {
            EntityType::Sleeper
        } else if name.contains("dead") || name.contains("corpse") {
            EntityType::Corpse
        } else if name.contains("backpack") {
            EntityType::Backpack
        } else if name.contains("zombie") || name.contains("gingerbread") {
            EntityType::Zombie
        } else if name.contains("minicopter") || name.contains("helicopter") || name.contains("scrap_heli") {
            EntityType::Helicopter
        } else if name.contains("car") || name.contains("cockpit_vehicle") {
            EntityType::Car
        } else if name.contains("boat") || name.contains("rhib") {
            EntityType::Boat
        } else if name.contains("airdrop") || name.contains("supply_drop") {
            EntityType::Airdrop
        } else if name.contains("crate") || name.contains("barrel") {
            EntityType::Crate
        } else if name.contains("ore") || name.contains("sulfur") || name.contains("metal") {
            EntityType::ResourceOre
        } else if name.contains("tree") || name.contains("wood") {
            EntityType::WoodTree
        } else if name.contains("player") {
            EntityType::Player
        } else {
            EntityType::Unknown
        }
    }

    pub fn get_despawn_text(entity_type: EntityType, despawn_ticks: u32) -> Option<String> {
        match entity_type {
            EntityType::Corpse => {
                let seconds = (despawn_ticks / 20) % 60;
                let minutes = despawn_ticks / 20 / 60;
                Some(format!("Turns to bag in: {:02}:{:02}", minutes, seconds))
            }
            EntityType::Backpack => {
                let seconds = (despawn_ticks / 20) % 60;
                let minutes = despawn_ticks / 20 / 60;
                Some(format!("Despawns in: {:02}:{:02}", minutes, seconds))
            }
            _ => None,
        }
    }
}
