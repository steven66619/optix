use std::process::ExitStatus;
use std::sync::mpsc;
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::term::ClipboardType;
use alacritty_terminal::vte::ansi::Rgb;

/// Kind of event flowing from a terminal's PTY thread back to the UI thread.
pub enum PaneEventKind {
    /// New terminal content is available; schedule a redraw.
    Wakeup,
    /// Window title changed (`None` resets to default).
    Title(Option<String>),
    /// Terminal bell rang.
    Bell,
    /// Store text in a clipboard.
    ClipboardStore(ClipboardType, String),
    /// Read the clipboard and write the formatted result to the PTY.
    ClipboardLoad(ClipboardType, Arc<dyn Fn(&str) -> String + Send + Sync + 'static>),
    /// Respond to an OSC 4 color request.
    ColorRequest(usize, Arc<dyn Fn(Rgb) -> String + Send + Sync + 'static>),
    /// Respond to a text area size request.
    TextAreaSizeRequest(Arc<dyn Fn(WindowSize) -> String + Send + Sync + 'static>),
    /// Terminal wants us to write raw bytes to the PTY (e.g. bracketed paste passthrough).
    PtyWrite(String),
    /// Child process exited.
    Exit(ExitStatus),
    /// Cursor blinking state changed.
    CursorBlinkingChange,
}

/// Event routed from a terminal's PTY thread to the UI thread.
pub struct PaneEvent {
    pub pane_id: usize,
    pub kind: PaneEventKind,
}

/// `EventListener` handed to each `Term` so its event loop can reach the UI thread.
pub struct PaneProxy {
    pub pane_id: usize,
    pub tx: mpsc::Sender<PaneEvent>,
}

impl EventListener for PaneProxy {
    fn send_event(&self, event: Event) {
        let kind = match event {
            Event::MouseCursorDirty | Event::Wakeup => PaneEventKind::Wakeup,
            Event::Title(title) => PaneEventKind::Title(Some(title)),
            Event::ResetTitle => PaneEventKind::Title(None),
            Event::ClipboardStore(ty, text) => PaneEventKind::ClipboardStore(ty, text),
            Event::ClipboardLoad(ty, formatter) => PaneEventKind::ClipboardLoad(ty, formatter),
            Event::ColorRequest(idx, formatter) => PaneEventKind::ColorRequest(idx, formatter),
            Event::PtyWrite(text) => PaneEventKind::PtyWrite(text),
            Event::TextAreaSizeRequest(formatter) => PaneEventKind::TextAreaSizeRequest(formatter),
            Event::CursorBlinkingChange => PaneEventKind::CursorBlinkingChange,
            Event::Bell => PaneEventKind::Bell,
            Event::Exit => PaneEventKind::Exit(ExitStatus::default()),
            Event::ChildExit(status) => PaneEventKind::Exit(status),
        };
        let _ = self.tx.send(PaneEvent { pane_id: self.pane_id, kind });
    }
}
