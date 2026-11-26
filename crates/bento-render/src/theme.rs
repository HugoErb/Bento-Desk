//! Light and dark palettes following the Windows 11 (Fluent) look.

use bento_core::color::Rgba;

/// Font used for the UI text.
pub const UI_FONT: &str = "Segoe UI Variable Text";
/// Icon font available on Windows 10 and 11.
pub const ICON_FONT: &str = "Segoe MDL2 Assets";

/// Colors of menus and overlays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub accent: Rgba,
    pub surface: Rgba,
    pub surface_border: Rgba,
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub hover: Rgba,
    pub pressed: Rgba,
    pub separator: Rgba,
    pub danger: Rgba,
}

impl Theme {
    pub fn new(dark: bool, accent: Rgba) -> Theme {
        if dark {
            Theme {
                dark,
                accent,
                surface: Rgba::rgb(0x2C, 0x2C, 0x2C),
                surface_border: Rgba::new(255, 255, 255, 0x1F),
                text: Rgba::rgb(0xFF, 0xFF, 0xFF),
                text_secondary: Rgba::new(0xFF, 0xFF, 0xFF, 0xA8),
                hover: Rgba::new(255, 255, 255, 0x14),
                pressed: Rgba::new(255, 255, 255, 0x0C),
                separator: Rgba::new(255, 255, 255, 0x18),
                danger: Rgba::rgb(0xFF, 0x99, 0xA4),
            }
        } else {
            Theme {
                dark,
                accent,
                surface: Rgba::rgb(0xF9, 0xF9, 0xF9),
                surface_border: Rgba::new(0, 0, 0, 0x1A),
                text: Rgba::rgb(0x1A, 0x1A, 0x1A),
                text_secondary: Rgba::new(0, 0, 0, 0x9E),
                hover: Rgba::new(0, 0, 0, 0x0A),
                pressed: Rgba::new(0, 0, 0, 0x06),
                separator: Rgba::new(0, 0, 0, 0x14),
                danger: Rgba::rgb(0xC4, 0x2B, 0x1C),
            }
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::new(true, Rgba::rgb(0x00, 0x78, 0xD4))
    }
}

/// Scale factor for a DPI.
pub fn scale(dpi: u32) -> f32 {
    dpi.max(96) as f32 / 96.0
}
