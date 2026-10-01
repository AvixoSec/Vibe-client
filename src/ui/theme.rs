use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_hex(hex: u32, alpha: f32) -> Self {
        let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
        let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
        let b = (hex & 0xFF) as f32 / 255.0;
        Self { r, g, b, a: alpha }
    }

    pub fn to_rgba_u32(&self) -> u32 {
        let r = (self.r.clamp(0.0, 1.0) * 255.0) as u32;
        let g = (self.g.clamp(0.0, 1.0) * 255.0) as u32;
        let b = (self.b.clamp(0.0, 1.0) * 255.0) as u32;
        let a = (self.a.clamp(0.0, 1.0) * 255.0) as u32;
        (a << 24) | (r << 16) | (g << 8) | b
    }
}

pub struct Theme;

impl Theme {
    // Background: Dark Graphite
    pub const BG_MAIN: Color = Color::new(0.094, 0.098, 0.110, 0.95); // #18191c
    pub const BG_PANEL: Color = Color::new(0.133, 0.141, 0.157, 0.95); // #222428
    pub const BG_HEADER: Color = Color::new(0.169, 0.176, 0.196, 1.0); // #2b2d32
    pub const BG_INPUT: Color = Color::new(0.110, 0.118, 0.133, 1.0);

    // Accent: Electric Turquoise
    pub const ACCENT: Color = Color::new(0.0, 0.898, 0.737, 1.0); // #00e5bc
    pub const ACCENT_HOVER: Color = Color::new(0.15, 0.95, 0.80, 1.0);
    pub const ACCENT_MUTED: Color = Color::new(0.0, 0.898, 0.737, 0.25);

    // Text
    pub const TEXT_PRIMARY: Color = Color::new(1.0, 1.0, 1.0, 1.0);
    pub const TEXT_SECONDARY: Color = Color::new(0.627, 0.647, 0.690, 1.0); // #a0a5b0
    pub const TEXT_DISABLED: Color = Color::new(0.40, 0.42, 0.46, 1.0);

    // Status
    pub const SUCCESS: Color = Color::new(0.180, 0.800, 0.443, 1.0);
    pub const DANGER: Color = Color::new(1.0, 0.278, 0.341, 1.0); // #ff4757
    pub const WARNING: Color = Color::new(1.0, 0.647, 0.008, 1.0); // #ffa502

    // Layout Metrics
    pub const WINDOW_WIDTH: f32 = 220.0;
    pub const HEADER_HEIGHT: f32 = 34.0;
    pub const ITEM_HEIGHT: f32 = 28.0;
    pub const SETTING_HEIGHT: f32 = 24.0;
    pub const PADDING: f32 = 6.0;
    pub const CORNER_RADIUS: f32 = 5.0;
}
