use crate::core::events::ClientEvent;
use crate::core::module::{Module, ModuleCategory, Setting};
use crate::core::snapshot::Rotation;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecoilPatternPoint {
    pub pitch_offset: f32,
    pub yaw_offset: f32,
    pub duration_ms: u32,
}

#[derive(Debug, Clone)]
pub struct WeaponRecoilProfile {
    pub name: &'static str,
    pub rpm: u32,
    pub pattern: Vec<RecoilPatternPoint>,
}

impl WeaponRecoilProfile {
    pub fn get_sks() -> Self {
        Self {
            name: "SKS",
            rpm: 380,
            pattern: vec![
                RecoilPatternPoint { pitch_offset: 3.8, yaw_offset: -0.4, duration_ms: 150 },
                RecoilPatternPoint { pitch_offset: 4.1, yaw_offset: 0.6, duration_ms: 150 },
                RecoilPatternPoint { pitch_offset: 4.3, yaw_offset: -0.8, duration_ms: 150 },
                RecoilPatternPoint { pitch_offset: 4.5, yaw_offset: 0.9, duration_ms: 150 },
            ],
        }
    }

    pub fn get_custom_smg() -> Self {
        Self {
            name: "Custom SMG",
            rpm: 600,
            pattern: vec![
                RecoilPatternPoint { pitch_offset: 1.8, yaw_offset: -0.8, duration_ms: 100 },
                RecoilPatternPoint { pitch_offset: 2.2, yaw_offset: -1.2, duration_ms: 100 },
                RecoilPatternPoint { pitch_offset: 2.5, yaw_offset: 1.0, duration_ms: 100 },
                RecoilPatternPoint { pitch_offset: 2.7, yaw_offset: 1.5, duration_ms: 100 },
                RecoilPatternPoint { pitch_offset: 2.6, yaw_offset: -1.1, duration_ms: 100 },
                RecoilPatternPoint { pitch_offset: 2.8, yaw_offset: -1.4, duration_ms: 100 },
            ],
        }
    }

    pub fn get_heavy_revolver() -> Self {
        Self {
            name: "Heavy Revolver",
            rpm: 340,
            pattern: vec![
                RecoilPatternPoint { pitch_offset: 5.2, yaw_offset: 0.5, duration_ms: 175 },
                RecoilPatternPoint { pitch_offset: 5.4, yaw_offset: -0.7, duration_ms: 175 },
                RecoilPatternPoint { pitch_offset: 5.5, yaw_offset: 0.8, duration_ms: 175 },
            ],
        }
    }

    pub fn get_ak47() -> Self {
        Self {
            name: "AK-47",
            rpm: 450,
            pattern: vec![
                RecoilPatternPoint { pitch_offset: 2.6, yaw_offset: -1.5, duration_ms: 133 },
                RecoilPatternPoint { pitch_offset: 3.0, yaw_offset: -2.0, duration_ms: 133 },
                RecoilPatternPoint { pitch_offset: 3.3, yaw_offset: -1.8, duration_ms: 133 },
                RecoilPatternPoint { pitch_offset: 3.2, yaw_offset: 1.4, duration_ms: 133 },
                RecoilPatternPoint { pitch_offset: 3.4, yaw_offset: 2.2, duration_ms: 133 },
                RecoilPatternPoint { pitch_offset: 3.1, yaw_offset: 2.4, duration_ms: 133 },
                RecoilPatternPoint { pitch_offset: 2.9, yaw_offset: -1.0, duration_ms: 133 },
                RecoilPatternPoint { pitch_offset: 2.8, yaw_offset: -1.8, duration_ms: 133 },
            ],
        }
    }

    pub fn get_thompson() -> Self {
        Self {
            name: "Thompson",
            rpm: 460,
            pattern: vec![
                RecoilPatternPoint { pitch_offset: 2.0, yaw_offset: 0.7, duration_ms: 130 },
                RecoilPatternPoint { pitch_offset: 2.4, yaw_offset: 1.1, duration_ms: 130 },
                RecoilPatternPoint { pitch_offset: 2.5, yaw_offset: -0.9, duration_ms: 130 },
                RecoilPatternPoint { pitch_offset: 2.7, yaw_offset: -1.2, duration_ms: 130 },
            ],
        }
    }

    pub fn get_nailgun() -> Self {
        Self {
            name: "Nailgun",
            rpm: 400,
            pattern: vec![
                RecoilPatternPoint { pitch_offset: 1.2, yaw_offset: 0.3, duration_ms: 150 },
                RecoilPatternPoint { pitch_offset: 1.4, yaw_offset: -0.4, duration_ms: 150 },
            ],
        }
    }
}

pub struct RecoilControl {
    enabled: bool,
    keybind: u32,
    settings: Vec<Setting>,
    pub shot_count: usize,
    pub is_firing: bool,
    active_profile: WeaponRecoilProfile,
}

impl Default for RecoilControl {
    fn default() -> Self {
        Self::new()
    }
}

impl RecoilControl {
    pub fn new() -> Self {
        Self {
            enabled: false,
            keybind: 0,
            settings: vec![
                Setting::new_float("PitchCompensation", "Vertical recoil reduction percentage", 90.0, 0.0, 100.0, 5.0),
                Setting::new_float("YawCompensation", "Horizontal recoil reduction percentage", 85.0, 0.0, 100.0, 5.0),
                Setting::new_float("RandomJitter", "Random humanization jitter percentage", 5.0, 0.0, 20.0, 1.0),
                Setting::new_mode("WeaponProfile", "Active weapon recoil profile", 0, &[
                    "AutoDetect",
                    "AK-47",
                    "SKS",
                    "Custom SMG",
                    "Heavy Revolver",
                    "Thompson",
                    "Nailgun",
                ]),
            ],
            shot_count: 0,
            is_firing: false,
            active_profile: WeaponRecoilProfile::get_ak47(),
        }
    }

    pub fn set_weapon(&mut self, weapon_name: &str) {
        let clean = weapon_name.to_lowercase();
        if clean.contains("sks") {
            self.active_profile = WeaponRecoilProfile::get_sks();
        } else if clean.contains("smg") || clean.contains("кустарн") {
            self.active_profile = WeaponRecoilProfile::get_custom_smg();
        } else if clean.contains("revolver") || clean.contains("револьвер") {
            self.active_profile = WeaponRecoilProfile::get_heavy_revolver();
        } else if clean.contains("thompson") || clean.contains("томпсон") {
            self.active_profile = WeaponRecoilProfile::get_thompson();
        } else if clean.contains("nail") || clean.contains("гвоздо") {
            self.active_profile = WeaponRecoilProfile::get_nailgun();
        } else {
            self.active_profile = WeaponRecoilProfile::get_ak47();
        }
    }

    pub fn on_shot_fired(&mut self) -> Rotation {
        if !self.enabled || self.active_profile.pattern.is_empty() {
            return Rotation::ZERO;
        }

        let pitch_comp = self.settings[0].as_float() / 100.0;
        let yaw_comp = self.settings[1].as_float() / 100.0;
        let jitter = self.settings[2].as_float() / 100.0;

        let idx = self.shot_count.min(self.active_profile.pattern.len() - 1);
        let point = self.active_profile.pattern[idx];

        let jitter_factor_pitch = 1.0 + ((self.shot_count as f32 % 3.0) - 1.0) * jitter * 0.5;
        let jitter_factor_yaw = 1.0 + (((self.shot_count + 1) as f32 % 3.0) - 1.0) * jitter * 0.5;

        let comp_pitch = -point.pitch_offset * pitch_comp * jitter_factor_pitch;
        let comp_yaw = -point.yaw_offset * yaw_comp * jitter_factor_yaw;

        self.shot_count += 1;
        Rotation::new(comp_pitch, comp_yaw)
    }

    pub fn reset_shots(&mut self) {
        self.shot_count = 0;
        self.is_firing = false;
    }
}

impl Module for RecoilControl {
    fn name(&self) -> &'static str {
        "RecoilControl"
    }

    fn description(&self) -> &'static str {
        "Compensates weapon recoil using custom RustMe recoil tables"
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
            self.reset_shots();
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
            if let Some(held) = &snapshot.local_player.entity.held_item {
                let mode = self.settings[3].as_mode_index();
                if mode == 0 {
                    self.set_weapon(&held.name);
                }
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
