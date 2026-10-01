use std::collections::HashMap;
use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory};

pub struct ModuleRegistry {
    modules: Vec<Box<dyn Module>>,
    name_index: HashMap<String, usize>,
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
            name_index: HashMap::new(),
        }
    }

    pub fn register<M: Module + 'static>(&mut self, module: M) {
        let name = module.name().to_string();
        let idx = self.modules.len();
        self.modules.push(Box::new(module));
        self.name_index.insert(name.to_lowercase(), idx);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Module> {
        self.name_index.get(&name.to_lowercase()).map(|&idx| self.modules[idx].as_ref())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Box<dyn Module>> {
        if let Some(&idx) = self.name_index.get(&name.to_lowercase()) {
            Some(&mut self.modules[idx])
        } else {
            None
        }
    }

    pub fn all(&self) -> &[Box<dyn Module>] {
        &self.modules
    }

    pub fn all_mut(&mut self) -> &mut [Box<dyn Module>] {
        &mut self.modules
    }

    pub fn get_by_category(&self, category: ModuleCategory) -> Vec<&dyn Module> {
        self.modules
            .iter()
            .filter(|m| m.category() == category)
            .map(|m| m.as_ref())
            .collect()
    }

    pub fn handle_key(&mut self, key_code: u32, pressed: bool) -> Vec<(&'static str, bool)> {
        if !pressed || key_code == 0 {
            return Vec::new();
        }

        let mut toggled = Vec::new();
        for module in &mut self.modules {
            if module.keybind() == key_code {
                let state = module.toggle();
                toggled.push((module.name(), state));
            }
        }
        toggled
    }

    pub fn dispatch_event(&mut self, event: &mut ClientEvent) {
        for module in &mut self.modules {
            if module.is_enabled() {
                module.on_event(event);
            }
        }
    }

    pub fn search(&self, query: &str) -> Vec<&dyn Module> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.modules.iter().map(|m| m.as_ref()).collect();
        }

        self.modules
            .iter()
            .filter(|m| {
                m.name().to_lowercase().contains(&q)
                    || m.description().to_lowercase().contains(&q)
                    || m.category().display_name().to_lowercase().contains(&q)
            })
            .map(|m| m.as_ref())
            .collect()
    }
}
