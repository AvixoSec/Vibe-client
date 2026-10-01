use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::core::module::SettingValue;
use crate::core::registry::ModuleRegistry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleConfigData {
    pub enabled: bool,
    pub keybind: u32,
    pub settings: HashMap<String, SettingValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientPreset {
    pub name: String,
    pub description: String,
    pub modules: HashMap<String, ModuleConfigData>,
}

impl ClientPreset {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            modules: HashMap::new(),
        }
    }
}

pub struct ConfigManager;

impl ConfigManager {
    pub fn export_preset(name: &str, description: &str, registry: &ModuleRegistry) -> ClientPreset {
        let mut preset = ClientPreset::new(name, description);

        for module in registry.all() {
            let mut settings_map = HashMap::new();
            for setting in module.settings() {
                settings_map.insert(setting.name.clone(), setting.value.clone());
            }

            preset.modules.insert(
                module.name().to_string(),
                ModuleConfigData {
                    enabled: module.is_enabled(),
                    keybind: module.keybind(),
                    settings: settings_map,
                },
            );
        }

        preset
    }

    pub fn apply_preset(preset: &ClientPreset, registry: &mut ModuleRegistry) {
        for (name, data) in &preset.modules {
            if let Some(module) = registry.get_mut(name) {
                module.set_enabled(data.enabled);
                module.set_keybind(data.keybind);

                for setting in module.settings_mut() {
                    if let Some(saved_val) = data.settings.get(&setting.name) {
                        setting.value = saved_val.clone();
                    }
                }
            }
        }
    }

    pub fn to_json(preset: &ClientPreset) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(preset)
    }

    pub fn from_json(json: &str) -> Result<ClientPreset, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn get_default_preset(preset_type: &str) -> ClientPreset {
        match preset_type.to_lowercase().as_str() {
            "visual" => ClientPreset {
                name: "Visual".to_string(),
                description: "Clean visual overlay only (ESP, HUD, Radar)".to_string(),
                modules: {
                    let mut m = HashMap::new();
                    m.insert("PlayerESP".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0x4F, // 'O'
                        settings: HashMap::new(),
                    });
                    m.insert("HUD".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m.insert("Radar".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m
                },
            },
            "combat" => ClientPreset {
                name: "Combat".to_string(),
                description: "PVP assistance with humanized aim and recoil control".to_string(),
                modules: {
                    let mut m = HashMap::new();
                    m.insert("PlayerESP".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0x4F,
                        settings: HashMap::new(),
                    });
                    m.insert("AimAssist".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0x58, // 'X'
                        settings: HashMap::new(),
                    });
                    m.insert("RecoilControl".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m.insert("TriggerBot".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0x5A, // 'Z'
                        settings: HashMap::new(),
                    });
                    m
                },
            },
            "farm" => ClientPreset {
                name: "Farm".to_string(),
                description: "Automated looting and resource node harvesting".to_string(),
                modules: {
                    let mut m = HashMap::new();
                    m.insert("AutoLoot".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0x4C, // 'L'
                        settings: HashMap::new(),
                    });
                    m.insert("AutoArmor".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m.insert("AutoFarm".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0x4A, // 'J'
                        settings: HashMap::new(),
                    });
                    m
                },
            },
            _ => ClientPreset {
                name: "Experimental".to_string(),
                description: "Aggressive features for testing and research".to_string(),
                modules: {
                    let mut m = HashMap::new();
                    m.insert("SilentAim".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m.insert("Speed".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m.insert("Fly".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m.insert("NoFall".to_string(), ModuleConfigData {
                        enabled: true,
                        keybind: 0,
                        settings: HashMap::new(),
                    });
                    m
                },
            },
        }
    }
}
