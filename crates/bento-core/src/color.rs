//! Colors and the HSL based background model used by fence styles.

use serde::{Deserialize, Serialize};

/// 8-bit straight (non premultiplied) RGBA color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }

    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    pub const TRANSPARENT: Rgba = Rgba::new(0, 0, 0, 0);

    pub const fn with_alpha(self, a: u8) -> Self {
        Self::new(self.r, self.g, self.b, a)
    }

    /// Components as floats in `0.0..=1.0`.
    pub fn to_f32(self) -> [f32; 4] {
        [
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
            f32::from(self.a) / 255.0,
        ]
    }

    /// Relative luminance (sRGB approximation), `0.0..=1.0`.
    pub fn luminance(self) -> f32 {
        let [r, g, b, _] = self.to_f32();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Black or white, whichever reads better on top of `self`.
    pub fn contrasting_text(self) -> Rgba {
        if self.luminance() > 0.55 {
            Rgba::rgb(20, 20, 20)
        } else {
            Rgba::WHITE
        }
    }

    /// Linear interpolation towards `other` (`t` in `0.0..=1.0`).
    pub fn mix(self, other: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| -> u8 {
            let v = f32::from(a) + (f32::from(b) - f32::from(a)) * t;
            v.round().clamp(0.0, 255.0) as u8
        };
        Rgba::new(
            lerp(self.r, other.r),
            lerp(self.g, other.g),
            lerp(self.b, other.b),
            lerp(self.a, other.a),
        )
    }

    /// `#RRGGBB` or `#RRGGBBAA`.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    /// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the `#` is optional).
    pub fn from_hex(s: &str) -> Option<Rgba> {
        let s = s.trim().trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
        match s.len() {
            3 => {
                let nib = |i: usize| {
                    let v = u8::from_str_radix(s.get(i..=i)?, 16).ok()?;
                    Some(v * 17)
                };
                Some(Rgba::rgb(nib(0)?, nib(1)?, nib(2)?))
            }
            6 => Some(Rgba::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Rgba::new(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        }
    }

    /// Converts to hue (degrees), saturation and lightness (`0.0..=1.0`).
    pub fn to_hsl(self) -> (f32, f32, f32) {
        let [r, g, b, _] = self.to_f32();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < f32::EPSILON {
            return (0.0, 0.0, l);
        }
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if (max - r).abs() < f32::EPSILON {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if (max - g).abs() < f32::EPSILON {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h * 60.0, s, l)
    }

    /// Builds a color from hue (degrees), saturation and lightness.
    pub fn from_hsl(h: f32, s: f32, l: f32, a: u8) -> Rgba {
        let h = h.rem_euclid(360.0) / 360.0;
        let s = s.clamp(0.0, 1.0);
        let l = l.clamp(0.0, 1.0);
        let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        if s <= f32::EPSILON {
            let v = to_u8(l);
            return Rgba::new(v, v, v, a);
        }
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        let hue = |mut t: f32| {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        Rgba::new(
            to_u8(hue(h + 1.0 / 3.0)),
            to_u8(hue(h)),
            to_u8(hue(h - 1.0 / 3.0)),
            a,
        )
    }
}

/// Background color expressed with the four sliders Fences users know:
/// hue, saturation, intensity (lightness) and transparency.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tint {
    /// Hue in degrees, `0..360`.
    pub hue: u16,
    /// Saturation in percent.
    pub saturation: u8,
    /// Intensity (lightness) in percent.
    pub intensity: u8,
    /// Transparency in percent, 100 = fully transparent.
    pub transparency: u8,
}

impl Default for Tint {
    fn default() -> Self {
        // Neutral dark glass, close to the Windows 11 flyout look.
        Self {
            hue: 220,
            saturation: 12,
            intensity: 14,
            transparency: 45,
        }
    }
}

impl Tint {
    pub fn to_rgba(self) -> Rgba {
        let alpha = 255.0 * (1.0 - f32::from(self.transparency.min(100)) / 100.0);
        Rgba::from_hsl(
            f32::from(self.hue % 360),
            f32::from(self.saturation.min(100)) / 100.0,
            f32::from(self.intensity.min(100)) / 100.0,
            alpha.round().clamp(0.0, 255.0) as u8,
        )
    }

    /// Keeps the transparency, takes hue, saturation and intensity from `c`.
    pub fn from_rgba(c: Rgba, transparency: u8) -> Tint {
        let (h, s, l) = c.to_hsl();
        Tint {
            hue: (h.round() as i32).rem_euclid(360) as u16,
            saturation: (s * 100.0).round().clamp(0.0, 100.0) as u8,
            intensity: (l * 100.0).round().clamp(0.0, 100.0) as u8,
            transparency,
        }
    }
}

/// Average color of a premultiplied or straight BGRA buffer region.
/// Used by Chameleon to adapt a fence to the wallpaper behind it.
pub fn average_bgra(pixels: &[u8]) -> Option<Rgba> {
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    for px in pixels.chunks_exact(4) {
        sum[0] += u64::from(px[2]);
        sum[1] += u64::from(px[1]);
        sum[2] += u64::from(px[0]);
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let c = |v: u64| u8::try_from(v / n).unwrap_or(255);
    Some(Rgba::rgb(c(sum[0]), c(sum[1]), c(sum[2])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c = Rgba::new(0x12, 0xAB, 0xEF, 0x80);
        assert_eq!(Rgba::from_hex(&c.to_hex()), Some(c));
        assert_eq!(Rgba::from_hex("#fff"), Some(Rgba::WHITE));
        assert_eq!(Rgba::from_hex("zz"), None);
    }

    #[test]
    fn hsl_round_trip() {
        for c in [
            Rgba::rgb(255, 0, 0),
            Rgba::rgb(10, 200, 90),
            Rgba::rgb(30, 30, 30),
            Rgba::rgb(120, 60, 220),
        ] {
            let (h, s, l) = c.to_hsl();
            let back = Rgba::from_hsl(h, s, l, 255);
            assert!((i32::from(back.r) - i32::from(c.r)).abs() <= 1);
            assert!((i32::from(back.g) - i32::from(c.g)).abs() <= 1);
            assert!((i32::from(back.b) - i32::from(c.b)).abs() <= 1);
        }
    }

    #[test]
    fn tint_transparency_maps_to_alpha() {
        let t = Tint {
            transparency: 100,
            ..Tint::default()
        };
        assert_eq!(t.to_rgba().a, 0);
        let t = Tint {
            transparency: 0,
            ..Tint::default()
        };
        assert_eq!(t.to_rgba().a, 255);
    }

    #[test]
    fn average_of_buffer() {
        let buf = [0u8, 0, 255, 255, 255, 0, 0, 255];
        assert_eq!(average_bgra(&buf), Some(Rgba::rgb(127, 0, 127)));
        assert_eq!(average_bgra(&[]), None);
    }

    #[test]
    fn contrast_picks_dark_on_light() {
        assert_eq!(Rgba::WHITE.contrasting_text(), Rgba::rgb(20, 20, 20));
        assert_eq!(Rgba::BLACK.contrasting_text(), Rgba::WHITE);
    }
}
