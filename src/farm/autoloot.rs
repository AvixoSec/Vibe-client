use crate::core::arbiter::{ActionArbiter, ActionPayload, ActionPriority};
use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::{GameSnapshot, ItemCategory};

pub struct AutoLoot {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    last_loot_tick: u64,
}

impl Default for AutoLoot {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoLoot {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0x4C, // 'L'
            settings: vec![
                Setting::new_mode("FilterMode", "Item looting filter", 1, &["All", "ValuablesOnly", "CustomWhitelist"]),
                Setting::new_int("LootDelayTicks", "Delay between taking individual slots", 2, 1, 10),
                Setting::new_bool("TakeWeapons", "Loot weapons", true),
                Setting::new_bool("TakeAmmo", "Loot ammunition", true),
                Setting::new_bool("TakeMeds", "Loot medical supplies", true),
                Setting::new_bool("TakeSulfur", "Loot sulfur and explosives", true),
                Setting::new_bool("TakeScrap", "Loot scrap and components", true),
            ],
            last_loot_tick: 0,
        }
    }

    pub fn process_container_loot(
        &mut self,
        snapshot: &GameSnapshot,
        arbiter: &mut ActionArbiter,
        current_tick: u64,
    ) -> bool {
        if !self.enabled || !snapshot.inventory.is_container_open {
            return false;
        }

        let delay_ticks = self.settings[1].as_int() as u64;
        if self.last_loot_tick > 0 && current_tick < self.last_loot_tick + delay_ticks {
            return false;
        }

        let filter_mode = self.settings[0].as_mode_index();
        let take_weapons = self.settings[2].as_bool();
        let take_ammo = self.settings[3].as_bool();
        let take_meds = self.settings[4].as_bool();
        let take_sulfur = self.settings[5].as_bool();
        let take_scrap = self.settings[6].as_bool();

        for item in &snapshot.inventory.container_items {
            let is_match = match filter_mode {
                0 => true, // All
                _ => {
                    let name_lower = item.name.to_lowercase();
                    match item.category {
                        ItemCategory::Weapon => take_weapons,
                        ItemCategory::Ammo => take_ammo,
                        ItemCategory::Medical => take_meds,
                        ItemCategory::Resource => {
                            if name_lower.contains("sulfur") || name_lower.contains("сера") || name_lower.contains("gunpowder") {
                                take_sulfur
                            } else if name_lower.contains("scrap") || name_lower.contains("скрап") || name_lower.contains("gear") || name_lower.contains("spring") {
                                take_scrap
                            } else {
                                false
                            }
                        }
                        _ => false,
                    }
                }
            };

            if is_match {
                self.last_loot_tick = current_tick;
                arbiter.submit_action(
                    ActionPriority::Inventory,
                    ActionPayload::MoveSlot {
                        from_container: true,
                        slot_from: item.slot_index,
                        slot_to: 0, // Auto-deposit into first available slot
                        item_id: item.id,
                    },
                );
                return true;
            }
        }

        false
    }
}

impl Module for AutoLoot {
    fn name(&self) -> &'static str {
        "AutoLoot"
    }

    fn description(&self) -> &'static str {
        "Transfers items from opened chests/corpses/sleepers with priority filtering"
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
