use crate::core::arbiter::{ActionArbiter, ActionPayload, ActionPriority};
use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{GameSnapshot, Item, ItemCategory};

pub struct AutoArmor {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    last_equip_tick: u64,
}

impl Default for AutoArmor {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoArmor {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_int("EquipDelayTicks", "Delay between equipping individual armor pieces", 3, 1, 10),
                Setting::new_bool("PreferMetalArmor", "Always prioritize metal/high tier over hazmat", true),
                Setting::new_bool("AutoReplaceDamaged", "Replace broken or low durability armor", true),
            ],
            last_equip_tick: 0,
        }
    }

    pub fn classify_armor_piece(item: &Item) -> Option<(u8, f32)> {
        let name = item.name.to_lowercase();
        if item.category != ItemCategory::Armor && !name.contains("helmet") && !name.contains("mask") && !name.contains("chestplate") && !name.contains("kilt") && !name.contains("jacket") && !name.contains("pants") && !name.contains("boots") && !name.contains("hazmat") {
            return None;
        }

        // Slot mapping: 0=Head, 1=Face, 2=Chest, 3=Legs, 4=Feet, 5=Hands, 6=Suit
        if name.contains("metal facemask") || name.contains("heavy plate helmet") {
            Some((0, 50.0))
        } else if name.contains("coffee can") {
            Some((0, 35.0))
        } else if name.contains("riot helmet") || name.contains("bucket") || name.contains("wooden helmet") {
            Some((0, 20.0))
        } else if name.contains("bandana") || name.contains("wooden mask") {
            Some((1, 15.0))
        } else if name.contains("metal chestplate") || name.contains("heavy plate jacket") {
            Some((2, 50.0))
        } else if name.contains("roadsign jacket") || name.contains("road sign") {
            Some((2, 40.0))
        } else if name.contains("bone armor") || name.contains("jacket") || name.contains("hoodie") {
            Some((2, 25.0))
        } else if name.contains("roadsign kilt") {
            Some((3, 35.0))
        } else if name.contains("pants") {
            Some((3, 20.0))
        } else if name.contains("boots") {
            Some((4, 20.0))
        } else if name.contains("gloves") {
            Some((5, 10.0))
        } else if name.contains("hazmat") {
            Some((6, 30.0))
        } else {
            None
        }
    }

    pub fn check_and_equip(
        &mut self,
        snapshot: &GameSnapshot,
        arbiter: &mut ActionArbiter,
        current_tick: u64,
    ) -> bool {
        if !self.enabled {
            return false;
        }

        let delay = self.settings[0].as_int() as u64;
        if self.last_equip_tick > 0 && current_tick < self.last_equip_tick + delay {
            return false;
        }

        for item in &snapshot.inventory.items {
            if let Some((target_slot_idx, protection)) = Self::classify_armor_piece(item) {
                let current_slot = &snapshot.inventory.armor[target_slot_idx as usize];

                let should_equip = match current_slot {
                    None => true,
                    Some(current) => {
                        let cur_prot = Self::classify_armor_piece(current).map(|(_, p)| p).unwrap_or(0.0);
                        protection > cur_prot
                    }
                };

                if should_equip {
                    self.last_equip_tick = current_tick;
                    arbiter.submit_action(
                        ActionPriority::Inventory,
                        ActionPayload::EquipArmor {
                            slot_from: item.slot_index,
                            armor_slot_idx: target_slot_idx,
                        },
                    );
                    return true;
                }
            }
        }

        false
    }
}

impl Module for AutoArmor {
    fn name(&self) -> &'static str {
        "AutoArmor"
    }

    fn description(&self) -> &'static str {
        "Automatically equips best armor pieces into RustMe's 7 flexible slots"
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
