//! Magic shell-level commands handled by the terminal itself.
//!
//! When a line typed at a shell prompt matches a magic command, optix swallows
//! it: the text is never handed to the shell (so the shell never complains
//! about a missing `theme` program), and the action runs inside the terminal
//! instead, with feedback shown in the overlay.
//!
//! These complement the `/command` overlay (`/theme ayu`). The overlay only
//! triggers when the line-start heuristic says the next key begins a fresh
//! line, which goes stale after a TUI app exits or a line is interrupted.
//! Magic lines are recognized at Enter time from whatever was actually typed,
//! so they work even when that heuristic is wrong.

/// A recognized magic command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Magic {
    /// `theme` with no arguments: list the available themes.
    ThemeList,
    /// `theme <name>`: switch to that theme.
    ThemeSet(String),
}

/// Names of the magic commands, for docs and future completion UIs.
pub const NAMES: &[&str] = &["theme"];

/// Parse the text typed since the last Enter as a magic command.
///
/// Returns `None` when the line is not a magic command and should be passed
/// through to the shell untouched. Leading/trailing whitespace and multiple
/// spaces between words are tolerated; anything with more than one argument
/// is deliberately *not* treated as a magic command so an arbitrary shell
/// line can never be swallowed by mistake.
pub fn parse(line: &str) -> Option<Magic> {
    let mut parts = line.split_whitespace();
    let command = parts.next()?;
    if !command.eq_ignore_ascii_case("theme") {
        return None;
    }
    let arg = parts.next();
    if parts.next().is_some() {
        // More than one argument: not something we can interpret.
        return None;
    }
    match arg {
        None => Some(Magic::ThemeList),
        // "help" is the same as no argument.
        Some(arg) if arg.eq_ignore_ascii_case("help") || arg == "-h" || arg == "--help" => {
            Some(Magic::ThemeList)
        },
        Some(name) => Some(Magic::ThemeSet(name.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_with_name_sets_that_theme() {
        assert_eq!(parse("theme ayu"), Some(Magic::ThemeSet("ayu".into())));
        assert_eq!(parse("theme tokyonight"), Some(Magic::ThemeSet("tokyonight".into())));
    }

    #[test]
    fn theme_alone_lists_themes() {
        assert_eq!(parse("theme"), Some(Magic::ThemeList));
        assert_eq!(parse("theme   "), Some(Magic::ThemeList));
        assert_eq!(parse("theme help"), Some(Magic::ThemeList));
        assert_eq!(parse("theme --help"), Some(Magic::ThemeList));
        assert_eq!(parse("theme -h"), Some(Magic::ThemeList));
    }

    #[test]
    fn command_word_is_case_insensitive() {
        assert_eq!(parse("THEME ayu"), Some(Magic::ThemeSet("ayu".into())));
        assert_eq!(parse("Theme Ayu"), Some(Magic::ThemeSet("Ayu".into())));
    }

    #[test]
    fn extra_whitespace_is_fine() {
        assert_eq!(parse("  theme   ayu  "), Some(Magic::ThemeSet("ayu".into())));
    }

    #[test]
    fn non_magic_lines_pass_through() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("ls -la"), None);
        assert_eq!(parse("themes ayu"), None);
        assert_eq!(parse("theme ayu extra"), None);
        assert_eq!(parse("echo theme ayu"), None);
    }
}
