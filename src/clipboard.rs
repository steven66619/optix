//! OS clipboard access, modelled after alacritty's `clipboard` module.
//!
//! Alacritty's approach (which we copy here) is deliberately simple and
//! robust for long-running terminal apps:
//!
//! - The clipboard backend is created **once at startup**, not lazily per
//!   operation. A failed init is logged and the app keeps running with a
//!   no-op clipboard instead of failing every paste.
//! - Every `store` failure is logged (`warn!`) instead of being swallowed.
//! - `load` never fails: an empty string is returned on error. This matters
//!   for OSC 52 **queries**: an app that asks the terminal for the clipboard
//!   content (e.g. a TUI paste helper) must always get an answer — an empty
//!   payload tells it the clipboard is empty, whereas no answer at all can
//!   leave it hanging.
//! - On Linux the CLIPBOARD and PRIMARY selections are both served through
//!   the single arboard backend using `LinuxClipboardKind`; on other
//!   platforms `ClipboardType::Selection` is a no-op, exactly like
//!   alacritty's `selection: None` case.

use log::{debug, warn};

use alacritty_terminal::term::ClipboardType;
use arboard::Clipboard as ArboardClipboard;

/// The terminal's clipboard: one backend, two selections on Linux.
pub struct Clipboard {
    clipboard: Option<ArboardClipboard>,
}

impl Clipboard {
    /// Create the clipboard backend eagerly, like alacritty does at startup.
    ///
    /// On Wayland this uses arboard's `wayland-data-control` feature (the
    /// same wl-clipboard support alacritty ships); on X11 arboard spawns its
    /// selection-serving thread here. If no backend is available the
    /// clipboard stays `None` and operations become no-ops.
    pub fn new() -> Self {
        let clipboard = match ArboardClipboard::new() {
            Ok(clipboard) => Some(clipboard),
            Err(err) => {
                warn!("Unable to initialize clipboard: {err}");
                None
            },
        };
        Self { clipboard }
    }

    /// Store text into the given clipboard selection.
    ///
    /// Mirrors alacritty's `Clipboard::store`: errors are logged, never
    /// panicked or silently dropped.
    pub fn store(&mut self, ty: ClipboardType, text: impl Into<String>) {
        let Some(clipboard) = &mut self.clipboard else { return };

        let result = match ty {
            ClipboardType::Clipboard => clipboard.set_text(text.into()),
            #[cfg(target_os = "linux")]
            ClipboardType::Selection => {
                use arboard::{LinuxClipboardKind, SetExtLinux};
                clipboard.set().clipboard(LinuxClipboardKind::Primary).text(text.into())
            },
            // No PRIMARY selection on non-Linux platforms (alacritty's
            // `(ClipboardType::Selection, None) => return`).
            #[cfg(not(target_os = "linux"))]
            ClipboardType::Selection => return,
        };

        if let Err(err) = result {
            warn!("Unable to store text in clipboard: {err}");
        }
    }

    /// Load text from the given clipboard selection.
    ///
    /// Mirrors alacritty's `Clipboard::load`: never fails, always returns a
    /// `String` so OSC 52 queries can be answered even when the clipboard is
    /// empty or the backend errored.
    pub fn load(&mut self, ty: ClipboardType) -> String {
        let Some(clipboard) = &mut self.clipboard else { return String::new() };

        let result = match ty {
            ClipboardType::Clipboard => clipboard.get_text(),
            #[cfg(target_os = "linux")]
            ClipboardType::Selection => {
                use arboard::{GetExtLinux, LinuxClipboardKind};
                clipboard.get().clipboard(LinuxClipboardKind::Primary).text()
            },
            #[cfg(not(target_os = "linux"))]
            ClipboardType::Selection => return String::new(),
        };

        match result {
            Ok(text) => text,
            Err(err) => {
                debug!("Unable to load text from clipboard: {err}");
                String::new()
            },
        }
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}
