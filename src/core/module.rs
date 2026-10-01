use serde::{Deserialize, Serialize};
use crate::core::events::ClientEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleCategory {
    Combat,
    Visual,
    Movement,
    Farm,
    World,
    Misc,
}

impl ModuleCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Combat => "Combat",
            Self::Visual => "Visual",
            Self::Movement => "Movement",
            Self::Farm => "Farm & Loot",
            Self::World => "World",
            Self::Misc => "Misc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettingValue {
    Boolean(bool),
    Float { value: f32, min: f32, max: f32, step: f32 },
    Integer { value: i32, min: i32, max: i32 },
    Keybind(u32),
    Mode { selected: usize, options: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub name: String,
    pub description: String,
    pub value: SettingValue,
}

impl Setting {
    pub fn new_bool(name: &str, description: &str, default: bool) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            value: SettingValue::Boolean(default),
        }
    }

    pub fn new_float(name: &str, description: &str, default: f32, min: f32, max: f32, step: f32) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            value: SettingValue::Float { value: default, min, max, step },
        }
    }

    pub fn new_int(name: &str, description: &str, default: i32, min: i32, max: i32) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            value: SettingValue::Integer { value: default, min, max },
        }
    }

    pub fn new_keybind(name: &str, description: &str, default_key: u32) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            value: SettingValue::Keybind(default_key),
        }
    }

    pub fn new_mode(name: &str, description: &str, selected: usize, options: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            value: SettingValue::Mode {
                selected,
                options: options.iter().map(|s| s.to_string()).collect(),
            },
        }
    }

    pub fn as_bool(&self) -> bool {
        match &self.value {
            SettingValue::Boolean(b) => *b,
            _ => false,
        }
    }

    pub fn as_float(&self) -> f32 {
        match &self.value {
            SettingValue::Float { value, .. } => *value,
            _ => 0.0,
        }
    }

    pub fn as_int(&self) -> i32 {
        match &self.value {
            SettingValue::Integer { value, .. } => *value,
            _ => 0,
        }
    }

    pub fn as_keybind(&self) -> u32 {
        match &self.value {
            SettingValue::Keybind(k) => *k,
            _ => 0,
        }
    }

    pub fn as_mode_index(&self) -> usize {
        match &self.value {
            SettingValue::Mode { selected, .. } => *selected,
            _ => 0,
        }
    }

    pub fn as_mode_str(&self) -> Option<&str> {
        match &self.value {
            SettingValue::Mode { selected, options } => options.get(*selected).map(|s| s.as_str()),
            _ => None,
        }
    }
}

pub trait Module: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn category(&self) -> ModuleCategory;
    fn is_enabled(&self) -> bool;
    fn set_enabled(&mut self, enabled: bool);
    fn toggle(&mut self) -> bool {
        let new_state = !self.is_enabled();
        self.set_enabled(new_state);
        new_state
    }
    fn keybind(&self) -> u32;
    fn set_keybind(&mut self, key: u32);
    fn settings(&self) -> &[Setting];
    fn settings_mut(&mut self) -> &mut [Setting];
    fn on_event(&mut self, event: &mut ClientEvent);
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
