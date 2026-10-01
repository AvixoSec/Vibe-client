use crate::core::module::{Module, SettingValue};
use crate::core::registry::ModuleRegistry;
use crate::core::snapshot::Vector2;
use crate::ui::clickgui::ClickGUI;
use crate::ui::hud::HUD;
use crate::ui::theme::{Color, Theme};
use crate::visuals::player_esp::PlayerESP;
use crate::visuals::radar::Radar;
use crate::visuals::skeleton::SkeletonESP;
use crate::visuals::world_esp::WorldESP;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawCommand {
    Rect { min: Vector2, max: Vector2, color: Color },
    RectOutline { min: Vector2, max: Vector2, thickness: f32, color: Color },
    Line { start: Vector2, end: Vector2, thickness: f32, color: Color },
    Circle { center: Vector2, radius: f32, color: Color },
    CornerBox { min: Vector2, max: Vector2, length: f32, color: Color },
    Text { text_idx: usize, pos: Vector2, color: Color },
}

pub struct Renderer {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub command_buffer: Vec<DrawCommand>,
    pub string_pool: Vec<String>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(1920.0, 1080.0)
    }
}

impl Renderer {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            viewport_width: width,
            viewport_height: height,
            command_buffer: Vec::with_capacity(2048),
            string_pool: Vec::with_capacity(256),
        }
    }

    pub fn begin_frame(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.command_buffer.clear();
        self.string_pool.clear();
    }

    pub fn draw_rect(&mut self, min: Vector2, max: Vector2, color: Color) {
        self.command_buffer.push(DrawCommand::Rect { min, max, color });
    }

    pub fn draw_rect_outline(&mut self, min: Vector2, max: Vector2, thickness: f32, color: Color) {
        self.command_buffer.push(DrawCommand::RectOutline { min, max, thickness, color });
    }

    pub fn draw_corner_box(&mut self, min: Vector2, max: Vector2, length: f32, color: Color) {
        self.command_buffer.push(DrawCommand::CornerBox { min, max, length, color });
    }

    pub fn draw_line(&mut self, start: Vector2, end: Vector2, thickness: f32, color: Color) {
        self.command_buffer.push(DrawCommand::Line { start, end, thickness, color });
    }

    pub fn draw_circle(&mut self, center: Vector2, radius: f32, color: Color) {
        self.command_buffer.push(DrawCommand::Circle { center, radius, color });
    }

    pub fn draw_text(&mut self, text: &str, pos: Vector2, color: Color) {
        let idx = self.string_pool.len();
        self.string_pool.push(text.to_string());
        self.command_buffer.push(DrawCommand::Text { text_idx: idx, pos, color });
    }

    pub fn draw_progress_bar(&mut self, min: Vector2, max: Vector2, fraction: f32, fill_color: Color, bg_color: Color) {
        self.draw_rect(min, max, bg_color);
        let fill_w = (max.x - min.x) * fraction.clamp(0.0, 1.0);
        let fill_max = Vector2::new(min.x + fill_w, max.y);
        self.draw_rect(min, fill_max, fill_color);
    }

    pub fn render_player_esp(&mut self, esp: &PlayerESP) {
        if !esp.is_enabled() {
            return;
        }

        let box_mode = esp.settings()[0].as_mode_index();
        let show_hp = esp.settings()[1].as_bool();
        let show_dist = esp.settings()[2].as_bool();
        let show_names = esp.settings()[3].as_bool();
        let show_weapon = esp.settings()[4].as_bool();

        for item in &esp.cached_render_list {
            if let Some((min, max)) = item.screen_box {
                let col = Color::new(item.color_rgba[0], item.color_rgba[1], item.color_rgba[2], item.color_rgba[3]);

                // Box
                match box_mode {
                    0 => self.draw_corner_box(min, max, 8.0, col),
                    1 => self.draw_rect_outline(min, max, 1.5, col),
                    _ => {}
                }

                // Health bar (vertical left of box)
                if show_hp {
                    let hp_min = Vector2::new(min.x - 6.0, min.y);
                    let hp_max = Vector2::new(min.x - 2.0, max.y);
                    let hp_col = Color::new(
                        (1.0 - item.health_fraction) * 1.0 + item.health_fraction * 0.1,
                        item.health_fraction * 0.9 + (1.0 - item.health_fraction) * 0.1,
                        0.2,
                        1.0,
                    );
                    self.draw_rect(hp_min, hp_max, Color::new(0.1, 0.1, 0.1, 0.8));
                    let cur_hp_y = max.y - (max.y - min.y) * item.health_fraction;
                    self.draw_rect(Vector2::new(hp_min.x, cur_hp_y), hp_max, hp_col);
                }

                // Name & Distance
                if show_names {
                    self.draw_text(&item.name, Vector2::new(min.x, min.y - 14.0), Theme::TEXT_PRIMARY);
                }

                if show_dist {
                    let dist_str = format!("{:.0}m", item.distance_m);
                    self.draw_text(&dist_str, Vector2::new(max.x + 4.0, min.y), Theme::TEXT_SECONDARY);
                }

                // Held weapon
                if show_weapon {
                    if let Some(weapon) = &item.held_item_name {
                        self.draw_text(weapon, Vector2::new(min.x, max.y + 2.0), Theme::ACCENT);
                    }
                }

                // Extra info (corpse / backpack timers)
                if let Some(extra) = &item.extra_info {
                    self.draw_text(extra, Vector2::new(min.x, max.y + 14.0), Theme::WARNING);
                }
            }
        }
    }

    pub fn render_skeletons(&mut self, skel: &SkeletonESP) {
        if !skel.is_enabled() {
            return;
        }

        for item in &skel.cached_skeletons {
            let col = Color::new(item.color_rgba[0], item.color_rgba[1], item.color_rgba[2], item.color_rgba[3]);

            for bone in &item.bones {
                self.draw_line(bone.start, bone.end, 1.2, col);
            }

            if let Some((origin, target)) = item.tracer_line {
                self.draw_line(origin, target, 1.0, Color::new(col.r, col.g, col.b, 0.45));
            }
        }
    }

    pub fn render_world_esp(&mut self, world: &WorldESP) {
        if !world.is_enabled() {
            return;
        }

        for item in &world.cached_items {
            let col = Color::new(item.color_rgba[0], item.color_rgba[1], item.color_rgba[2], item.color_rgba[3]);
            let label = format!("{} [{:.0}m]", item.name, item.distance_m);
            self.draw_circle(item.screen_pos, 3.0, col);
            self.draw_text(&label, Vector2::new(item.screen_pos.x + 5.0, item.screen_pos.y - 4.0), col);
        }
    }

    pub fn render_radar(&mut self, radar: &Radar) {
        if !radar.is_enabled() {
            return;
        }

        let center = radar.get_center();
        let radius = radar.get_radius();

        // Background circle & border
        self.draw_circle(center, radius, Color::new(0.08, 0.08, 0.1, 0.75));
        self.draw_circle(center, 3.0, Theme::ACCENT); // Local player dot

        // Crosshairs
        self.draw_line(
            Vector2::new(center.x - radius, center.y),
            Vector2::new(center.x + radius, center.y),
            1.0,
            Color::new(0.25, 0.25, 0.3, 0.6),
        );
        self.draw_line(
            Vector2::new(center.x, center.y - radius),
            Vector2::new(center.x, center.y + radius),
            1.0,
            Color::new(0.25, 0.25, 0.3, 0.6),
        );

        // Blips
        for blip in &radar.cached_blips {
            let blip_pos = Vector2::new(center.x + blip.offset_x, center.y + blip.offset_y);
            let col = Color::new(blip.color_rgba[0], blip.color_rgba[1], blip.color_rgba[2], blip.color_rgba[3]);
            self.draw_circle(blip_pos, 2.5, col);
        }
    }

    pub fn render_hud(&mut self, hud: &HUD) {
        if !hud.is_enabled() {
            return;
        }

        // Watermark top left
        self.draw_rect(
            Vector2::new(10.0, 10.0),
            Vector2::new(280.0, 32.0),
            Color::new(0.1, 0.11, 0.13, 0.9),
        );
        self.draw_rect(
            Vector2::new(10.0, 10.0),
            Vector2::new(13.0, 32.0),
            Theme::ACCENT,
        );
        self.draw_text(&hud.watermark_text, Vector2::new(20.0, 16.0), Theme::TEXT_PRIMARY);

        // Coordinates bottom left
        if !hud.coords_text.is_empty() {
            self.draw_text(
                &hud.coords_text,
                Vector2::new(10.0, self.viewport_height - 20.0),
                Theme::TEXT_SECONDARY,
            );
        }

        // ArrayList top right
        let mut y = 10.0;
        let right_x = self.viewport_width - 10.0;

        for entry in &hud.cached_active_modules {
            let text_w = entry.name.len() as f32 * 8.0 + 12.0;
            let min = Vector2::new(right_x - text_w, y);
            let max = Vector2::new(right_x, y + 18.0);

            self.draw_rect(min, max, Color::new(0.08, 0.09, 0.11, 0.85));
            self.draw_rect(Vector2::new(max.x - 2.0, min.y), max, entry.color);
            self.draw_text(entry.name, Vector2::new(min.x + 4.0, min.y + 3.0), entry.color);

            y += 19.0;
        }
    }

    pub fn render_clickgui(&mut self, gui: &ClickGUI, registry: &ModuleRegistry) {
        if !gui.is_open {
            return;
        }

        // Dim background
        self.draw_rect(
            Vector2::ZERO,
            Vector2::new(self.viewport_width, self.viewport_height),
            Color::new(0.0, 0.0, 0.0, 0.45),
        );

        // Search bar
        let search_min = Vector2::new(40.0, 15.0);
        let search_max = Vector2::new(340.0, 45.0);
        self.draw_rect(search_min, search_max, Theme::BG_INPUT);
        self.draw_rect_outline(
            search_min,
            search_max,
            1.0,
            if gui.is_search_active { Theme::ACCENT } else { Theme::BG_HEADER },
        );
        let search_display = if gui.search_query.is_empty() {
            "Search modules..."
        } else {
            &gui.search_query
        };
        self.draw_text(
            search_display,
            Vector2::new(search_min.x + 8.0, search_min.y + 8.0),
            if gui.search_query.is_empty() { Theme::TEXT_DISABLED } else { Theme::TEXT_PRIMARY },
        );

        // Windows
        for win in &gui.windows {
            let win_min = win.pos;
            let win_max = Vector2::new(win.pos.x + Theme::WINDOW_WIDTH, win.pos.y + Theme::HEADER_HEIGHT);

            // Header
            self.draw_rect(win_min, win_max, Theme::BG_HEADER);
            self.draw_rect(
                Vector2::new(win_min.x, win_max.y - 2.0),
                win_max,
                Theme::ACCENT,
            );
            self.draw_text(
                win.category.display_name(),
                Vector2::new(win_min.x + 10.0, win_min.y + 9.0),
                Theme::TEXT_PRIMARY,
            );

            if win.is_collapsed {
                continue;
            }

            let mut y = win_max.y + Theme::PADDING;
            let modules = registry.get_by_category(win.category);

            for m in modules {
                let item_min = Vector2::new(win.pos.x + Theme::PADDING, y);
                let item_max = Vector2::new(win.pos.x + Theme::WINDOW_WIDTH - Theme::PADDING, y + Theme::ITEM_HEIGHT);

                let is_hovered = gui.mouse_pos.x >= item_min.x && gui.mouse_pos.x <= item_max.x
                    && gui.mouse_pos.y >= item_min.y && gui.mouse_pos.y <= item_max.y;

                let bg_col = if m.is_enabled() {
                    if is_hovered {
                        Color::new(0.0, 0.898, 0.737, 0.32)
                    } else {
                        Color::new(0.0, 0.898, 0.737, 0.18)
                    }
                } else if is_hovered {
                    Color::new(0.18, 0.20, 0.24, 0.95)
                } else {
                    Theme::BG_PANEL
                };
                self.draw_rect(item_min, item_max, bg_col);

                let text_col = if m.is_enabled() {
                    Theme::ACCENT
                } else {
                    Theme::TEXT_SECONDARY
                };
                self.draw_text(m.name(), Vector2::new(item_min.x + 8.0, item_min.y + 6.0), text_col);

                // Right indicator [ON] / [OFF]
                let state_str = if m.is_enabled() { "[ON]" } else { "[OFF]" };
                let state_col = if m.is_enabled() { Theme::ACCENT } else { Theme::TEXT_DISABLED };
                self.draw_text(state_str, Vector2::new(item_max.x - 36.0, item_min.y + 6.0), state_col);

                y += Theme::ITEM_HEIGHT + 2.0;

                if gui.expanded_modules.contains(m.name()) {
                    for set in m.settings() {
                        let set_min = Vector2::new(win.pos.x + Theme::PADDING * 2.0, y);
                        let set_max = Vector2::new(win.pos.x + Theme::WINDOW_WIDTH - Theme::PADDING * 2.0, y + Theme::SETTING_HEIGHT);

                        let set_hovered = gui.mouse_pos.x >= set_min.x && gui.mouse_pos.x <= set_max.x
                            && gui.mouse_pos.y >= set_min.y && gui.mouse_pos.y <= set_max.y;

                        self.draw_rect(set_min, set_max, if set_hovered { Color::new(0.16, 0.18, 0.22, 0.95) } else { Theme::BG_INPUT });

                        match &set.value {
                            SettingValue::Boolean(b) => {
                                self.draw_text(&set.name, Vector2::new(set_min.x + 4.0, set_min.y + 4.0), Theme::TEXT_SECONDARY);
                                let b_str = if *b { "[ON]" } else { "[OFF]" };
                                let b_col = if *b { Theme::ACCENT } else { Theme::TEXT_DISABLED };
                                self.draw_text(b_str, Vector2::new(set_max.x - 34.0, set_min.y + 4.0), b_col);
                            }
                            SettingValue::Float { value, min, max, .. } => {
                                let ratio = if max > min { ((*value - *min) / (*max - *min)).clamp(0.0, 1.0) } else { 0.0 };
                                let bar_w = (set_max.x - set_min.x) * ratio;
                                self.draw_rect(
                                    Vector2::new(set_min.x, set_min.y),
                                    Vector2::new(set_min.x + bar_w, set_max.y),
                                    Color::new(0.0, 0.898, 0.737, 0.25),
                                );
                                let label = format!("{}: {:.1}", set.name, value);
                                self.draw_text(&label, Vector2::new(set_min.x + 4.0, set_min.y + 4.0), Theme::TEXT_PRIMARY);
                            }
                            SettingValue::Integer { value, min, max } => {
                                let ratio = if max > min { ((*value as f32 - *min as f32) / (*max as f32 - *min as f32)).clamp(0.0, 1.0) } else { 0.0 };
                                let bar_w = (set_max.x - set_min.x) * ratio;
                                self.draw_rect(
                                    Vector2::new(set_min.x, set_min.y),
                                    Vector2::new(set_min.x + bar_w, set_max.y),
                                    Color::new(0.0, 0.898, 0.737, 0.25),
                                );
                                let label = format!("{}: {}", set.name, value);
                                self.draw_text(&label, Vector2::new(set_min.x + 4.0, set_min.y + 4.0), Theme::TEXT_PRIMARY);
                            }
                            SettingValue::Mode { selected, options } => {
                                let opt = options.get(*selected).map(|s| s.as_str()).unwrap_or("?");
                                let label = format!("{}: <{}>", set.name, opt);
                                self.draw_text(&label, Vector2::new(set_min.x + 4.0, set_min.y + 4.0), Theme::ACCENT);
                            }
                            SettingValue::Keybind(k) => {
                                let label = format!("{}: [0x{:X}]", set.name, k);
                                self.draw_text(&label, Vector2::new(set_min.x + 4.0, set_min.y + 4.0), Theme::TEXT_SECONDARY);
                            }
                        }

                        y += Theme::SETTING_HEIGHT + 2.0;
                    }
                }
            }
        }

        // Draw custom cursor arrow at gui.mouse_pos
        let mx = gui.mouse_pos.x;
        let my = gui.mouse_pos.y;
        if mx > 0.0 || my > 0.0 {
            // Dark outline
            self.draw_line(Vector2::new(mx, my), Vector2::new(mx, my + 13.0), 2.5, Color::new(0.0, 0.0, 0.0, 0.9));
            self.draw_line(Vector2::new(mx, my), Vector2::new(mx + 10.0, my + 10.0), 2.5, Color::new(0.0, 0.0, 0.0, 0.9));
            self.draw_line(Vector2::new(mx, my + 13.0), Vector2::new(mx + 4.0, my + 10.0), 2.5, Color::new(0.0, 0.0, 0.0, 0.9));
            self.draw_line(Vector2::new(mx + 4.0, my + 10.0), Vector2::new(mx + 10.0, my + 10.0), 2.5, Color::new(0.0, 0.0, 0.0, 0.9));
            // Inner cyan body
            self.draw_line(Vector2::new(mx, my), Vector2::new(mx, my + 12.0), 1.5, Theme::ACCENT);
            self.draw_line(Vector2::new(mx, my), Vector2::new(mx + 9.0, my + 9.0), 1.5, Theme::ACCENT);
            self.draw_line(Vector2::new(mx, my + 12.0), Vector2::new(mx + 4.0, my + 9.0), 1.5, Theme::ACCENT);
            self.draw_line(Vector2::new(mx + 4.0, my + 9.0), Vector2::new(mx + 9.0, my + 9.0), 1.5, Theme::ACCENT);
        }
    }
}
