use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::color::Rgba;

pub const DEFAULT_CONFIG_DIR: &str = "optix";

/// Parsed configuration with all colors resolved.
#[derive(Debug, Clone)]
pub struct Config {
    pub theme: ParsedTheme,
    pub font: Font,
    pub window: Window,
    pub scroll: Scroll,
    pub shell: Option<String>,
    pub working_directory: Option<PathBuf>,
    /// `"ctrl+shift+t"` -> action name.
    pub keybindings: HashMap<String, String>,
    /// Whether shell lines like `theme <name>` are intercepted by the terminal.
    pub magic_enabled: bool,
    /// Whether the `optix-msg` Unix-socket server is running.
    pub ipc_enabled: bool,
    /// Copy the selection to the clipboard when a mouse drag selection is
    /// released (kitty's `copy_on_select` behavior).
    pub copy_on_select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Font {
    /// Font family name as reported by fontconfig.
    pub family: String,
    /// Font size in points.
    pub size: f32,
    /// Optional italic variant family (falls back to `family`).
    pub family_italic: Option<String>,
    /// Optional bold variant family.
    pub family_bold: Option<String>,
    /// Horizontal padding around the terminal content in pixels.
    pub padding_x: f32,
    /// Vertical padding around the terminal content in pixels.
    pub padding_y: f32,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: "JetBrainsMono NF".to_string(),
            size: 12.0,
            family_italic: None,
            family_bold: None,
            padding_x: 12.0,
            padding_y: 10.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    /// Window opacity, `0.0` = fully transparent, `1.0` = opaque.
    pub opacity: f32,
    /// Whether the window uses a transparent (ARGB) surface so a compositor
    /// such as picom can composite the desktop wallpaper through the terminal.
    /// Requires a running compositor; without one the window appears black.
    pub transparent: bool,
    /// Path to a background image (PNG). Painted behind the terminal.
    pub background_image: Option<PathBuf>,
    /// Radius of rounded window/content corners in pixels.
    pub corner_radius: f32,
    /// Whether to render a subtle inner glow around the terminal area.
    pub glow: bool,
    /// Initial window width in pixels.
    pub width: u32,
    /// Initial window height in pixels.
    pub height: u32,
    pub title: String,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            opacity: 0.8,
            transparent: false,
            background_image: None,
            corner_radius: 12.0,
            glow: true,
            width: 1080,
            height: 680,
            title: "Optix".to_string(),
        }
    }
}

/// Scrollback browsing behavior. Only ever applies to the terminal's own
/// history (never to TUI apps on the alternate screen).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Scroll {
    /// Animate wheel scrollback motion instead of jumping line-by-line.
    pub smooth: bool,
    /// Draw the auto-hiding scrollbar overlay while browsing scrollback.
    pub scrollbar: bool,
    /// Keep coasting after a wheel flick (inertia).
    pub momentum: bool,
    /// Base lines per wheel notch used when the device sends line deltas.
    pub wheel_lines: f64,
}

impl Default for Scroll {
    fn default() -> Self {
        Self {
            smooth: true,
            scrollbar: true,
            momentum: true,
            wheel_lines: 3.0,
        }
    }
}

/// Raw (string) theme as it appears in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub cursor_text: Option<String>,
    pub selection_background: String,
    pub selection_foreground: Option<String>,
    pub normal: [String; 8],
    pub bright: [String; 8],
    pub dim: Option<[String; 8]>,
    pub split_border: String,
    pub split_active: String,
    pub search_background: String,
    pub search_foreground: String,
    pub search_match_background: String,
    pub search_match_foreground: Option<String>,
    /// Vertical gradient behind the terminal (used when no image is set).
    pub background_gradient: Option<Gradient>,
    /// Color the window flashes when the bell rings.
    pub bell: String,
    /// Scrollbar overlay colors. All optional; sensible colors are derived
    /// from the theme when omitted.
    pub scrollbar_thumb: Option<String>,
    pub scrollbar_track: Option<String>,
    pub scrollbar_thumb_hover: Option<String>,
    pub scrollbar_badge_background: Option<String>,
    pub scrollbar_badge_foreground: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gradient {
    pub top: String,
    pub bottom: String,
}

/// Config for magic shell-level commands (`theme <name>` etc.).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicCfg {
    /// Intercept magic commands instead of forwarding them to the shell.
    pub enabled: bool,
}

impl Default for MagicCfg {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Config for the `optix-msg` IPC server.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct IpcCfg {
    /// Serve commands over a Unix socket so `optix-msg theme ayu` can drive a
    /// running terminal (see `~/.config/optix/ipc.sock`).
    pub enabled: bool,
}

impl Default for IpcCfg {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for Theme {
    fn default() -> Self {
        // Catppuccin Mocha based default palette.
        Self {
            background: "#1e1e2e".into(),
            foreground: "#cdd6f4".into(),
            cursor: "#f5e0dc".into(),
            cursor_text: Some("#1e1e2e".into()),
            selection_background: "#585b70".into(),
            selection_foreground: None,
            normal: [
                "#45475a".into(),
                "#f38ba8".into(),
                "#a6e3a1".into(),
                "#f9e2af".into(),
                "#89b4fa".into(),
                "#f5c2e7".into(),
                "#94e2d5".into(),
                "#bac2de".into(),
            ],
            bright: [
                "#585b70".into(),
                "#f38ba8".into(),
                "#a6e3a1".into(),
                "#f9e2af".into(),
                "#89b4fa".into(),
                "#f5c2e7".into(),
                "#94e2d5".into(),
                "#a6adc8".into(),
            ],
            dim: None,
            split_border: "#45475a".into(),
            split_active: "#89b4fa".into(),
            search_background: "#313244".into(),
            search_foreground: "#cdd6f4".into(),
            search_match_background: "#89b4fa".into(),
            search_match_foreground: Some("#1e1e2e".into()),
            background_gradient: Some(Gradient {
                top: "#2b2b40".into(),
                bottom: "#16161d".into(),
            }),
            bell: "#f9e2af".into(),
            scrollbar_thumb: None,
            scrollbar_track: None,
            scrollbar_thumb_hover: None,
            scrollbar_badge_background: None,
            scrollbar_badge_foreground: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedTheme {
    pub background: Rgba,
    pub foreground: Rgba,
    pub cursor: Rgba,
    pub cursor_text: Option<Rgba>,
    pub selection_background: Rgba,
    pub selection_foreground: Option<Rgba>,
    pub normal: [Rgba; 8],
    pub bright: [Rgba; 8],
    pub dim: [Rgba; 8],
    pub split_border: Rgba,
    pub split_active: Rgba,
    pub search_background: Rgba,
    pub search_foreground: Rgba,
    pub search_match_background: Rgba,
    pub search_match_foreground: Option<Rgba>,
    pub background_gradient: Option<(Rgba, Rgba)>,
    pub bell: Rgba,
    pub scrollbar_thumb: Rgba,
    pub scrollbar_track: Rgba,
    pub scrollbar_thumb_hover: Rgba,
    pub scrollbar_badge_background: Rgba,
    pub scrollbar_badge_foreground: Rgba,
}

fn parse_hex(s: &str, fallback: Rgba) -> Rgba {
    Rgba::from_hex(s).unwrap_or_else(|err| {
        log::warn!("{err}; using fallback {fallback}");
        fallback
    })
}

impl Config {
    /// Default configuration.
    pub fn default_config() -> Self {
        Self {
            theme: ParsedTheme::from_raw(&Theme::default()),
            font: Font::default(),
            window: Window::default(),
            scroll: Scroll::default(),
            shell: None,
            working_directory: None,
            keybindings: default_keybindings(),
            magic_enabled: true,
            ipc_enabled: true,
            copy_on_select: true,
        }
    }

    /// Load configuration from `~/.config/optix/config.toml` merged with defaults.
    pub fn load() -> Self {
        let path = config_path();
        if !path.exists() {
            let default = Self::default_config();
            if let Err(err) = default.write_example_config(&path) {
                log::warn!("Failed to write default config to {}: {err}", path.display());
            }
            return default;
        }
        match Self::try_load() {
            Some(cfg) => cfg,
            None => {
                log::error!("Failed to parse {}; using defaults", path.display());
                Self::default_config()
            },
        }
    }

    /// Read and parse the config file, returning `None` if it is missing or
    /// malformed (used by the live-reload path to keep the old settings until
    /// the file is valid again).
    pub fn try_load() -> Option<Self> {
        let path = config_path();
        let text = fs::read_to_string(&path).ok()?;
        let toml_cfg = toml::from_str::<TomlConfig>(&text).ok()?;
        let keybindings = toml_cfg.keybindings.clone();
        let mut cfg = toml_cfg.into_config();
        cfg.keybindings.extend(keybindings);
        Some(cfg)
    }

    /// Write a commented example config to `path` (used on first launch).
    pub fn write_example_config(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, EXAMPLE_CONFIG)
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Directory holding all optix config files: `~/.config/optix`.
pub fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")).unwrap_or_default()
        });
    base.join(DEFAULT_CONFIG_DIR)
}

impl ParsedTheme {
    /// Parse a theme from a TOML string containing a `[theme]` section (the
    /// same layout as `config.toml`, and what `~/.config/optix/themes/*.toml`
    /// files use). Returns `None` if the file has no valid `[theme]` table.
    pub fn from_toml(raw: &str) -> Option<Self> {
        let cfg: TomlConfig = toml::from_str(raw).ok()?;
        cfg.theme.map(|theme| ParsedTheme::from_raw(&theme))
    }

    pub fn from_raw(raw: &Theme) -> Self {
        let dim_default = |i: usize| {
            let base = Rgba::from_hex(&raw.normal[i]).unwrap_or_default();
            Rgba { r: base.r * 0.66, g: base.g * 0.66, b: base.b * 0.66, a: 1.0 }
        };
        let dim = match &raw.dim {
            Some(d) => std::array::from_fn(|i| parse_hex(&d[i], dim_default(i))),
            None => std::array::from_fn(dim_default),
        };
        // Scrollbar colors default to theme-derived translucencies so every
        // theme (built-in or user) looks good without extra configuration.
        let foreground = parse_hex(&raw.foreground, Rgba::from_u8(205, 214, 244, 255));
        let search_background = parse_hex(&raw.search_background, Rgba::rgb(0.19, 0.2, 0.27));
        let search_foreground = parse_hex(&raw.search_foreground, Rgba::rgb(0.8, 0.84, 0.96));

        Self {
            background: parse_hex(&raw.background, Rgba::from_u8(30, 30, 46, 255)),
            foreground,
            cursor: parse_hex(&raw.cursor, Rgba::from_u8(245, 224, 220, 255)),
            cursor_text: raw.cursor_text.as_deref().and_then(|c| Rgba::from_hex(c).ok()),
            selection_background: parse_hex(&raw.selection_background, Rgba::from_u8(88, 91, 112, 255)),
            selection_foreground: raw.selection_foreground.as_deref().and_then(|c| Rgba::from_hex(c).ok()),
            normal: std::array::from_fn(|i| parse_hex(&raw.normal[i], Rgba::rgb(0.2, 0.2, 0.2))),
            bright: std::array::from_fn(|i| parse_hex(&raw.bright[i], Rgba::rgb(0.8, 0.8, 0.8))),
            dim,
            split_border: parse_hex(&raw.split_border, Rgba::rgb(0.27, 0.29, 0.35)),
            split_active: parse_hex(&raw.split_active, Rgba::rgb(0.54, 0.71, 0.98)),
            search_background,
            search_foreground,
            search_match_background: parse_hex(&raw.search_match_background, Rgba::rgb(0.54, 0.71, 0.98)),
            search_match_foreground: raw.search_match_foreground.as_deref().and_then(|c| Rgba::from_hex(c).ok()),
            background_gradient: raw.background_gradient.as_ref().map(|g| {
                (
                    parse_hex(&g.top, Rgba::rgb(0.17, 0.17, 0.25)),
                    parse_hex(&g.bottom, Rgba::rgb(0.086, 0.086, 0.11)),
                )
            }),
            bell: parse_hex(&raw.bell, Rgba::rgb(0.98, 0.85, 0.69)),
            scrollbar_thumb: raw
                .scrollbar_thumb
                .as_deref()
                .and_then(|c| Rgba::from_hex(c).ok())
                .unwrap_or_else(|| foreground.with_alpha(0.35)),
            scrollbar_track: raw
                .scrollbar_track
                .as_deref()
                .and_then(|c| Rgba::from_hex(c).ok())
                .unwrap_or_else(|| foreground.with_alpha(0.10)),
            scrollbar_thumb_hover: raw
                .scrollbar_thumb_hover
                .as_deref()
                .and_then(|c| Rgba::from_hex(c).ok())
                .unwrap_or_else(|| foreground.with_alpha(0.6)),
            scrollbar_badge_background: raw
                .scrollbar_badge_background
                .as_deref()
                .and_then(|c| Rgba::from_hex(c).ok())
                .unwrap_or(search_background),
            scrollbar_badge_foreground: raw
                .scrollbar_badge_foreground
                .as_deref()
                .and_then(|c| Rgba::from_hex(c).ok())
                .unwrap_or(search_foreground),
        }
    }
}

/// Mirrors the on-disk layout for serde; resolved into [`Config`] afterwards.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct TomlConfig {
    theme: Option<Theme>,
    font: Option<Font>,
    window: Option<Window>,
    scroll: Option<Scroll>,
    shell: Option<String>,
    working_directory: Option<String>,
    keybindings: HashMap<String, String>,
    magic: Option<MagicCfg>,
    ipc: Option<IpcCfg>,
    copy_on_select: Option<bool>,
}

impl TomlConfig {
    fn into_config(self) -> Config {
        let defaults = Config::default_config();
        Config {
            theme: ParsedTheme::from_raw(&self.theme.unwrap_or_default()),
            font: self.font.unwrap_or_default(),
            window: self.window.unwrap_or_default(),
            scroll: self.scroll.unwrap_or_default(),
            shell: self.shell,
            working_directory: self.working_directory.map(PathBuf::from),
            keybindings: defaults.keybindings,
            magic_enabled: self.magic.map(|m| m.enabled).unwrap_or(true),
            ipc_enabled: self.ipc.map(|i| i.enabled).unwrap_or(true),
            copy_on_select: self.copy_on_select.unwrap_or(true),
        }
    }
}

fn default_keybindings() -> HashMap<String, String> {
    [
        ("ctrl+shift+e", "split_right"),
        ("ctrl+shift+o", "split_below"),
        ("ctrl+shift+x", "close_pane"),
        ("ctrl+shift+]", "next_pane"),
        ("ctrl+shift+[", "prev_pane"),
        ("ctrl+alt+up", "focus_pane_up"),
        ("ctrl+alt+down", "focus_pane_down"),
        ("ctrl+alt+left", "focus_pane_left"),
        ("ctrl+alt+right", "focus_pane_right"),
        ("ctrl+shift+f", "search"),
        ("ctrl+shift+f2", "search"),
        ("ctrl+shift+p", "command"),
        ("ctrl+shift+enter", "search_next"),
        ("ctrl+shift+g", "search_next"),
        ("ctrl+shift+h", "search_prev"),
        ("ctrl+shift+up", "scroll_up"),
        ("ctrl+shift+down", "scroll_down"),
        ("ctrl+shift+pageup", "page_up"),
        ("ctrl+shift+pagedown", "page_down"),
        ("ctrl+shift+home", "scroll_top"),
        ("ctrl+shift+end", "scroll_bottom"),
        ("ctrl+shift+c", "copy"),
        ("ctrl+shift+v", "paste"),
        ("ctrl+plus", "font_increase"),
        ("ctrl+minus", "font_decrease"),
        ("ctrl+0", "font_reset"),
        ("ctrl+shift+q", "quit"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

const EXAMPLE_CONFIG: &str = r##"# Optix configuration.
# This file is regenerated on first launch; all fields are optional.

# Copy the selection to the clipboard when you release a mouse-drag selection,
# like kitty. Set to false to require Ctrl+Shift+C.
copy_on_select = true

[font]
family = "JetBrainsMono NF"   # any fontconfig family
size = 12.0
padding_x = 12.0
padding_y = 10.0

[window]
opacity = 0.8                 # 0.0 ..= 1.0
transparent = false           # true = ARGB window so picom shows the wallpaper through
                              # (needs a compositor running; combine with opacity < 1.0)
# background_image = "/path/to/image.png"
corner_radius = 12.0
glow = true
width = 1080
height = 680
title = "Optix"

[theme]
background = "#1e1e2e"
foreground = "#cdd6f4"
cursor = "#f5e0dc"
# cursor_text = "#1e1e2e"
selection_background = "#585b70"
# selection_foreground = "#cdd6f4"

# ANSI 16 palette
normal = ["#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de"]
bright = ["#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8"]
# dim = ["#313244", "#e78284", "#a6d189", "#e5c890", "#8caaee", "#f4b8e4", "#81c8be", "#a5adce"]

split_border = "#45475a"
split_active = "#89b4fa"

search_background = "#313244"
search_foreground = "#cdd6f4"
search_match_background = "#89b4fa"
# search_match_foreground = "#1e1e2e"
bell = "#f9e2af"

# Scrollbar overlay colors (all optional; derived from the theme when omitted).
# scrollbar_thumb = "#cdd6f4"
# scrollbar_track = "#cdd6f4"
# scrollbar_thumb_hover = "#ffffff"
# scrollbar_badge_background = "#313244"
# scrollbar_badge_foreground = "#cdd6f4"

# Vertical gradient painted behind the terminal when no image is set.
# Remove this section for a flat background.
[theme.background_gradient]
top = "#2b2b40"
bottom = "#16161d"

# Scrollback browsing. Only applies to the terminal's own history — TUI apps
# on the alternate screen (vim, less, atuin, ...) are never touched.
[scroll]
smooth = true            # glide scrollback motion instead of line jumps
scrollbar = true         # auto-hiding scrollbar overlay while browsing history
momentum = true          # keep coasting after a wheel flick
wheel_lines = 3.0        # base lines per wheel notch (line-delta mice)

# Magic commands: lines typed at a shell prompt that optix handles itself
# instead of forwarding to the shell. With this enabled, typing
#   theme ayu          switch to the "ayu" theme (no shell error, no /theme)
#   theme              list the available themes
# at a shell prompt changes the theme without the shell trying to run a
# `theme` program.
[magic]
enabled = true

# IPC: serve commands over a Unix socket so external tools can drive a running
# terminal, e.g. `optix-msg theme ayu`, `optix-msg themes`, `optix-msg ping`.
# This is the reliable way to switch themes on the fly (no /theme overlay, no
# shell cooperation needed).
[ipc]
enabled = true

# Keybindings: "mods+key" = "action"
# mods: ctrl, shift, alt, super (any order, separated by +)
# Actions:
#   split_right split_below close_pane next_pane prev_pane
#   focus_pane_up focus_pane_down focus_pane_left focus_pane_right
#   scroll_up scroll_down page_up page_down scroll_top scroll_bottom
#   search search_next search_prev
#   copy paste
#   font_increase font_decrease font_reset
#   command          open the /command line (e.g. "/theme ayu")
#   quit
[keybindings]
"ctrl+shift+e" = "split_right"
"ctrl+shift+o" = "split_below"
"ctrl+shift+x" = "close_pane"
"ctrl+shift+]" = "next_pane"
"ctrl+shift+[" = "prev_pane"
"ctrl+alt+up" = "focus_pane_up"
"ctrl+alt+down" = "focus_pane_down"
"ctrl+alt+left" = "focus_pane_left"
"ctrl+alt+right" = "focus_pane_right"
"ctrl+shift+f" = "search"
"ctrl+shift+enter" = "search_next"
"ctrl+shift+p" = "command"   # open the /command line, e.g. "/theme ayu"
"ctrl+shift+up" = "scroll_up"
"ctrl+shift+down" = "scroll_down"
"ctrl+shift+pageup" = "page_up"
"ctrl+shift+pagedown" = "page_down"
"ctrl+shift+home" = "scroll_top"
"ctrl+shift+end" = "scroll_bottom"
"ctrl+shift+c" = "copy"
"ctrl+shift+v" = "paste"
"ctrl+plus" = "font_increase"
"ctrl+minus" = "font_decrease"
"ctrl+0" = "font_reset"
"ctrl+shift+q" = "quit"
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keybindings_include_command_action() {
        let bindings = default_keybindings();
        assert_eq!(bindings.get("ctrl+shift+p").map(String::as_str), Some("command"));
        // The `command` action opens the overlay that powers `/theme`.
        assert_eq!(bindings.get("ctrl+shift+f").map(String::as_str), Some("search"));
    }

    #[test]
    fn example_config_parses_and_keeps_command_binding() {
        let cfg: TomlConfig = toml::from_str(EXAMPLE_CONFIG).expect("example config is valid TOML");
        let keybindings = cfg.keybindings;
        assert_eq!(keybindings.get("ctrl+shift+p").map(String::as_str), Some("command"));
        assert_eq!(cfg.magic.map(|m| m.enabled), Some(true));
    }

    #[test]
    fn magic_defaults_to_enabled_when_absent() {
        let cfg: TomlConfig = toml::from_str("[window]\nwidth = 640").unwrap();
        assert!(cfg.magic.is_none());
        let config = cfg.into_config();
        assert!(config.magic_enabled);
    }

    #[test]
    fn magic_can_be_disabled() {
        let cfg: TomlConfig =
            toml::from_str("[magic]\nenabled = false").expect("valid TOML");
        let config = cfg.into_config();
        assert!(!config.magic_enabled);
    }

    #[test]
    fn ipc_defaults_to_enabled_when_absent() {
        let cfg: TomlConfig = toml::from_str("[window]\nwidth = 640").unwrap();
        assert!(cfg.ipc.is_none());
        let config = cfg.into_config();
        assert!(config.ipc_enabled);
    }

    #[test]
    fn ipc_can_be_disabled() {
        let cfg: TomlConfig =
            toml::from_str("[ipc]\nenabled = false").expect("valid TOML");
        let config = cfg.into_config();
        assert!(!config.ipc_enabled);
    }

    #[test]
    fn scroll_defaults_when_section_absent() {
        // Config files written before the `[scroll]` section existed must still
        // load with working scroll defaults (smooth glide + auto-hiding bar).
        let cfg: TomlConfig = toml::from_str("[font]\nsize = 12.0").unwrap();
        assert!(cfg.scroll.is_none());
        let config = cfg.into_config();
        assert!(config.scroll.smooth, "smooth scrolling must default on");
        assert!(config.scroll.scrollbar, "scrollbar must default on");
        assert!(config.scroll.momentum, "momentum must default on");
        assert_eq!(config.scroll.wheel_lines, 3.0);
    }

    #[test]
    fn scroll_section_overrides_defaults() {
        let cfg: TomlConfig =
            toml::from_str("[scroll]\nsmooth = false\nmomentum = false\nscrollbar = false\nwheel_lines = 1.5")
                .expect("valid TOML");
        let config = cfg.into_config();
        assert!(!config.scroll.smooth);
        assert!(!config.scroll.momentum);
        assert!(!config.scroll.scrollbar);
        assert_eq!(config.scroll.wheel_lines, 1.5);
    }
}
