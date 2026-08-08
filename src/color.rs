use std::fmt;

/// RGBA color in linear-free float space (0.0..=1.0), straight alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn from_rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
    }

    /// Parse `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    pub fn from_hex(input: &str) -> Result<Self, String> {
        let hex = input.trim().trim_start_matches('#');
        let parse = |off: usize| -> Result<u8, String> {
            u8::from_str_radix(&hex[off..off + 2], 16)
                .map_err(|_| format!("invalid hex color `{input}`"))
        };
        let (r, g, b, a) = match hex.len() {
            3 | 4 => {
                let d = |i: usize| -> Result<u8, String> {
                    let c = hex.chars().nth(i).ok_or_else(|| format!("invalid hex color `{input}`"))?;
                    u8::from_str_radix(&format!("{c}{c}"), 16).map_err(|_| format!("invalid hex color `{input}`"))
                };
                let a = if hex.len() == 4 { d(3)? } else { 255 };
                (d(0)?, d(1)?, d(2)?, a)
            },
            6 | 8 => {
                let a = if hex.len() == 8 { parse(6)? } else { 255 };
                (parse(0)?, parse(2)?, parse(4)?, a)
            },
            _ => return Err(format!("invalid hex color `{input}`")),
        };
        Ok(Self::from_u8(r, g, b, a))
    }

    /// Blend this color over `other` (this is the top color).
    pub fn over(self, other: Self) -> Self {
        let a = self.a + other.a * (1.0 - self.a);
        if a <= 0.0 {
            return Self::from_rgba(0.0, 0.0, 0.0, 0.0);
        }
        Self {
            r: (self.r * self.a + other.r * other.a * (1.0 - self.a)) / a,
            g: (self.g * self.a + other.g * other.a * (1.0 - self.a)) / a,
            b: (self.b * self.a + other.b * other.a * (1.0 - self.a)) / a,
            a,
        }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Linear interpolation toward `other`.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    pub fn to_wgpu(self) -> wgpu::Color {
        wgpu::Color { r: self.r as f64, g: self.g as f64, b: self.b as f64, a: self.a as f64 }
    }

    pub fn to_u32(self) -> u32 {
        let r = (self.r * 255.0).round().clamp(0.0, 255.0) as u32;
        let g = (self.g * 255.0).round().clamp(0.0, 255.0) as u32;
        let b = (self.b * 255.0).round().clamp(0.0, 255.0) as u32;
        (r << 16) | (g << 8) | b
    }
}

impl Default for Rgba {
    fn default() -> Self {
        Self::rgb(0.0, 0.0, 0.0)
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.to_u32())
    }
}

/// Convert an `alacritty_terminal` RGB into our color type.
pub fn from_ansi_rgb(rgb: alacritty_terminal::vte::ansi::Rgb) -> Rgba {
    Rgba::from_u8(rgb.r, rgb.g, rgb.b, 255)
}
