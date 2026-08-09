use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

use crate::color::{from_ansi_rgb, Rgba};
use crate::config::ParsedTheme;

/// Resolved terminal palette used to turn cell colors into RGBA.
#[derive(Clone)]
pub struct Palette {
    pub normal: [Rgba; 8],
    pub bright: [Rgba; 8],
    pub dim: [Rgba; 8],
    pub foreground: Rgba,
    pub background: Rgba,
    pub cursor: Rgba,
    pub cursor_text: Option<Rgba>,
    /// Dynamically modified colors from OSC 4/10/11/12 sequences.
    pub dynamic: Colors,
}

impl Palette {
    pub fn from_theme(theme: &ParsedTheme) -> Self {
        Self {
            normal: theme.normal,
            bright: theme.bright,
            dim: theme.dim,
            foreground: theme.foreground,
            background: theme.background,
            cursor: theme.cursor,
            cursor_text: theme.cursor_text,
            dynamic: Colors::default(),
        }
    }

    /// Resolve the 256-color cube/grayscale entry (already checked for dynamic override).
    fn indexed_cube(&self, idx: u8) -> Rgba {
        let idx = idx as usize;
        if idx < 8 {
            return self.normal[idx];
        }
        if idx < 16 {
            return self.bright[idx - 8];
        }
        if idx < 232 {
            let i = idx - 16;
            let level = |v: usize| -> u8 { if v == 0 { 0 } else { 55 + v as u8 * 40 } };
            Rgba::from_u8(level(i / 36), level((i / 6) % 6), level(i % 6), 255)
        } else {
            let v = (8 + (idx - 232) * 10).clamp(0, 255) as u8;
            Rgba::from_u8(v, v, v, 255)
        }
    }

    fn named(&self, named: NamedColor) -> Option<Rgba> {
        let idx = named as usize;
        match idx {
            0..=7 => self.normal.get(idx).copied(),
            8..=15 => self.bright.get(idx - 8).copied(),
            16 => Some(self.foreground),
            17 => Some(self.background),
            18 => Some(self.cursor),
            19..=26 => self.dim.get(idx - 19).copied(),
            27 => Some(self.bright[7]),
            28 => Some(self.dim[7]),
            _ => None,
        }
    }

    /// Resolve a terminal cell color to RGBA, honoring dynamic (OSC) overrides.
    pub fn resolve(&self, color: &Color, dynamic: &Colors) -> Rgba {
        match color {
            Color::Spec(rgb) => from_ansi_rgb(*rgb),
            Color::Named(named) => {
                let idx = *named as usize;
                if idx < 16 {
                    if let Some(rgb) = dynamic[idx] {
                        return from_ansi_rgb(rgb);
                    }
                }
                self.named(*named).unwrap_or(self.foreground)
            },
            Color::Indexed(idx) => {
                if let Some(rgb) = dynamic[*idx as usize] {
                    return from_ansi_rgb(rgb);
                }
                self.indexed_cube(*idx)
            },
        }
    }

    /// Convert an ANSI RGB to our type (kept for dynamic color formatting).
    pub fn rgb(rgb: Rgb) -> Rgba {
        from_ansi_rgb(rgb)
    }
}
