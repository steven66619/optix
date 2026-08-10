//! Theme presets selectable at runtime with `/theme <name>` or by typing
//! `theme <name>` directly at a shell prompt (see the `magic` module).
//!
//! Built-in palettes live here as TOML snippets matching the `[theme]` config
//! section. Users can drop their own `<name>.toml` files (same `[theme]` layout)
//! into `~/.config/optix/themes/` and they take precedence over the presets.

use crate::config::ParsedTheme;

/// Built-in theme names, in listing order.
const BUILTIN_NAMES: &[&str] = &[
    "catppuccin",
    "gruvbox",
    "dracula",
    "nord",
    "solarized",
    "tokyonight",
];

fn parse_toml(raw: &str) -> Option<ParsedTheme> {
    ParsedTheme::from_toml(raw)
}

fn builtin(name: &str) -> Option<ParsedTheme> {
    let raw = match name {
        "catppuccin" => CATPPUCCIN,
        "gruvbox" => GRUVBOX,
        "dracula" => DRACULA,
        "nord" => NORD,
        "solarized" => SOLARIZED,
        "tokyonight" => TOKYONIGHT,
        _ => return None,
    };
    parse_toml(raw)
}

/// Look up a theme: user themes in `~/.config/optix/themes/` first, then presets.
pub fn by_name(name: &str) -> Option<ParsedTheme> {
    let name = name.trim().to_ascii_lowercase();

    let path = crate::config::config_dir().join("themes").join(format!("{name}.toml"));
    if path.exists() {
        match std::fs::read_to_string(&path).ok().and_then(|t| parse_toml(&t)) {
            Some(theme) => return Some(theme),
            None => log::warn!("theme file {path:?} is malformed; ignoring"),
        }
    }

    builtin(&name)
}

/// All theme names: built-ins plus any user files found on disk.
pub fn names() -> Vec<String> {
    let mut out: Vec<String> = BUILTIN_NAMES.iter().map(|s| s.to_string()).collect();
    let dir = crate::config::config_dir().join("themes");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(stem) = entry.path().file_stem().map(|s| s.to_string_lossy().into_owned()) {
                if !out.contains(&stem) {
                    out.push(stem);
                }
            }
        }
    }
    out
}

const CATPPUCCIN: &str = r##"[theme]
background = "#1e1e2e"
foreground = "#cdd6f4"
cursor = "#f5e0dc"
cursor_text = "#1e1e2e"
selection_background = "#585b70"
normal = ["#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de"]
bright = ["#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8"]
split_border = "#45475a"
split_active = "#89b4fa"
search_background = "#313244"
search_foreground = "#cdd6f4"
search_match_background = "#89b4fa"
bell = "#f9e2af"
[theme.background_gradient]
top = "#2b2b40"
bottom = "#16161d"
"##;

const GRUVBOX: &str = r##"[theme]
background = "#282828"
foreground = "#ebdbb2"
cursor = "#ebdbb2"
cursor_text = "#282828"
selection_background = "#504945"
normal = ["#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984"]
bright = ["#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2"]
split_border = "#3c3836"
split_active = "#83a598"
search_background = "#3c3836"
search_foreground = "#ebdbb2"
search_match_background = "#fabd2f"
bell = "#d79921"
[theme.background_gradient]
top = "#32302f"
bottom = "#1d2021"
"##;

const DRACULA: &str = r##"[theme]
background = "#282a36"
foreground = "#f8f8f2"
cursor = "#f8f8f2"
cursor_text = "#282a36"
selection_background = "#44475a"
normal = ["#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2"]
bright = ["#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff"]
split_border = "#44475a"
split_active = "#bd93f9"
search_background = "#44475a"
search_foreground = "#f8f8f2"
search_match_background = "#bd93f9"
bell = "#f1fa8c"
[theme.background_gradient]
top = "#2d2f3d"
bottom = "#21222c"
"##;

const NORD: &str = r##"[theme]
background = "#2e3440"
foreground = "#d8dee9"
cursor = "#d8dee9"
cursor_text = "#2e3440"
selection_background = "#434c5e"
normal = ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0"]
bright = ["#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]
split_border = "#3b4252"
split_active = "#88c0d0"
search_background = "#3b4252"
search_foreground = "#d8dee9"
search_match_background = "#81a1c1"
bell = "#ebcb8b"
[theme.background_gradient]
top = "#353b4a"
bottom = "#242933"
"##;

const SOLARIZED: &str = r##"[theme]
background = "#002b36"
foreground = "#839496"
cursor = "#93a1a1"
cursor_text = "#002b36"
selection_background = "#073642"
normal = ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5"]
bright = ["#586e75", "#cb4b16", "#93a1a1", "#657b83", "#6c71c4", "#d33682", "#2aa198", "#fdf6e3"]
split_border = "#073642"
split_active = "#268bd2"
search_background = "#073642"
search_foreground = "#93a1a1"
search_match_background = "#b58900"
bell = "#b58900"
[theme.background_gradient]
top = "#003442"
bottom = "#00202b"
"##;

const TOKYONIGHT: &str = r##"[theme]
background = "#1a1b26"
foreground = "#c0caf5"
cursor = "#c0caf5"
cursor_text = "#1a1b26"
selection_background = "#283457"
normal = ["#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6"]
bright = ["#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5"]
split_border = "#1f2335"
split_active = "#7aa2f7"
search_background = "#1f2335"
search_foreground = "#c0caf5"
search_match_background = "#7aa2f7"
bell = "#e0af68"
[theme.background_gradient]
top = "#1f2335"
bottom = "#14151f"
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_themes_parse() {
        for name in BUILTIN_NAMES {
            let theme = builtin(name);
            assert!(theme.is_some(), "theme `{name}` should parse");
        }
    }

    #[test]
    fn by_name_is_case_insensitive_and_handles_unknown() {
        assert!(by_name("CATPPUCCIN").is_some());
        assert!(by_name("gruvbox").is_some());
        assert!(by_name("no-such-theme").is_none());
    }

    #[test]
    fn themes_have_distinct_backgrounds() {
        let cat = by_name("catppuccin").unwrap();
        let gruv = by_name("gruvbox").unwrap();
        assert_ne!(cat.background, gruv.background);
    }
}
