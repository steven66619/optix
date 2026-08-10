use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::ClipboardType;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::color::{from_ansi_rgb, Rgba};
use crate::config::{Config, ParsedTheme};
use crate::event::{PaneEvent, PaneEventKind};
use crate::fonts::Fonts;
use crate::input::{self, Mods};
use crate::kitty::{KittyImage, Placement, KITTY_MARKER};
use crate::layout::{self, Layout, Orientation, PaneId, Rect};
use crate::palette::Palette;
use crate::render::{Frame, Renderer};
use crate::terminal::{is_visible, TerminalPane};

/// Active search overlay state (bound to the focused pane).
struct SearchOverlay {
    query: String,
}

enum SearchAction {
    Close,
    NextAndClose,
    Backspace,
    Push(String),
}

/// What to do with the in-progress `/command` buffer after a key press.
enum CommandKey {
    /// Buffer updated; keep composing.
    Keep,
    /// Enter: run the command (or flush it to the shell).
    Execute,
    /// Escape/backspace-to-empty: drop the buffer, send nothing.
    Cancel,
    /// Any other key: send what was typed to the shell, then process normally.
    Flush,
}

/// The terminal application.
pub struct OptixApp {
    config: Config,
    window: Option<Window>,
    renderer: Option<Renderer>,
    fonts: Option<Fonts>,
    panes: HashMap<PaneId, TerminalPane>,
    layout: Layout,
    next_pane_id: PaneId,
    event_tx: mpsc::Sender<PaneEvent>,
    event_rx: mpsc::Receiver<PaneEvent>,
    el_wakeup: winit::event_loop::EventLoopProxy<()>,
    palette: Palette,
    search: Option<SearchOverlay>,
    bell_flash: Option<Instant>,
    focused: bool,
    mods: Mods,
    base_font_size: f32,
    font_size: f32,
    dpi_scale: f32,
    mouse_pos: (f64, f64),
    dragging: bool,
    quit: bool,
    clipboard: Option<arboard::Clipboard>,
    default_title: String,
    /// Last-seen mtime of the config file, for live reload.
    config_mtime: Option<std::time::SystemTime>,
    /// In-progress `/command` line (only composed while `Some`).
    command: Option<String>,
    /// Heuristic: the next key press starts a fresh shell line.
    line_start: bool,
    /// Text forwarded to the shell since the last Enter, shadowing the shell's
    /// line editor so magic commands (`theme ayu`) can be recognized at Enter.
    pending_line: String,
    /// Transient feedback shown after running a command, e.g. "Unknown theme".
    command_message: Option<(String, Instant)>,
    /// IPC socket channel: requests from `optix-msg` arrive here.
    ipc_tx: mpsc::Sender<crate::ipc::IpcRequest>,
    ipc_rx: mpsc::Receiver<crate::ipc::IpcRequest>,
}

impl OptixApp {
    pub fn new(
        config: Config,
        event_tx: mpsc::Sender<PaneEvent>,
        event_rx: mpsc::Receiver<PaneEvent>,
        el_wakeup: winit::event_loop::EventLoopProxy<()>,
    ) -> Self {
        let palette = Palette::from_theme(&config.theme);
        let base_font_size = config.font.size;
        let config_mtime = std::fs::metadata(crate::config::config_path())
            .and_then(|m| m.modified())
            .ok();
        let (ipc_tx, ipc_rx) = mpsc::channel::<crate::ipc::IpcRequest>();
        if config.ipc_enabled {
            // Serve `optix-msg` on a background thread; commands arrive on
            // `ipc_rx` and are executed in `handle_ipc`.
            crate::ipc::spawn(ipc_tx.clone(), el_wakeup.clone());
        }
        Self {
            palette,
            base_font_size,
            font_size: base_font_size,
            dpi_scale: 1.0,
            mouse_pos: (0.0, 0.0),
            dragging: false,
            quit: false,
            clipboard: None,
            default_title: config.window.title.clone(),
            layout: Layout::new(0),
            panes: HashMap::new(),
            next_pane_id: 0,
            event_tx,
            event_rx,
            el_wakeup,
            search: None,
            bell_flash: None,
            focused: true,
            mods: Mods::default(),
            window: None,
            renderer: None,
            fonts: None,
            config,
            config_mtime,
            command: None,
            line_start: true,
            pending_line: String::new(),
            command_message: None,
            ipc_tx,
            ipc_rx,
        }
    }

    fn window(&self) -> &Window {
        self.window.as_ref().expect("window exists during event handling")
    }

    fn padding(&self) -> (f32, f32) {
        (
            self.config.font.padding_x * self.dpi_scale,
            self.config.font.padding_y * self.dpi_scale,
        )
    }

    fn window_size(&self) -> (f32, f32) {
        self.renderer.as_ref().map(|r| (r.size.width as f32, r.size.height as f32)).unwrap_or((1.0, 1.0))
    }

    fn client_area(&self) -> Rect {
        let (w, h) = self.window_size();
        Rect { x: 0.0, y: 0.0, w, h }
    }

    fn pty_options(&self) -> PtyOptions {
        let shell = self.config.shell.clone().map(|s| Shell::new(s, Vec::new()));
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        PtyOptions {
            shell,
            working_directory: self.config.working_directory.clone(),
            drain_on_exit: false,
            env,
        }
    }

    fn spawn_pane(&mut self, cols: usize, lines: usize) -> Option<PaneId> {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let opts = self.pty_options();
        let fonts = self.fonts.as_ref().expect("fonts ready");
        let cell_w = fonts.cell_w as u16;
        let cell_h = fonts.cell_h as u16;
        match TerminalPane::new(id, &opts, cols, lines, cell_w, cell_h, self.event_tx.clone(), Some(self.el_wakeup.clone())) {
            Ok(pane) => {
                self.panes.insert(id, pane);
                Some(id)
            },
            Err(err) => {
                log::error!("failed to spawn shell pane {id}: {err}");
                None
            },
        }
    }

    fn cols_lines_for(&self, rect: &Rect) -> (usize, usize) {
        let (pad_x, pad_y) = self.padding();
        let cell_w = self.fonts.as_ref().map(|f| f.cell_w).unwrap_or(8.0);
        let cell_h = self.fonts.as_ref().map(|f| f.cell_h).unwrap_or(16.0);
        let cols = ((rect.w - 2.0 * pad_x) / cell_w).floor().max(1.0) as usize;
        let lines = ((rect.h - 2.0 * pad_y) / cell_h).floor().max(1.0) as usize;
        (cols, lines)
    }

    fn recompute_pane_sizes(&mut self) {
        let Some(fonts) = &self.fonts else { return };
        let cell_w = fonts.cell_w as u16;
        let cell_h = fonts.cell_h as u16;
        let area = self.client_area();
        let rects = self.layout.tab().layout_rects(area);
        let sizes = rects.iter().map(|(_, r)| self.cols_lines_for(r)).collect::<Vec<_>>();
        for ((id, _), (cols, lines)) in rects.iter().zip(sizes) {
            if let Some(pane) = self.panes.get_mut(id) {
                pane.resize(cols, lines, cell_w, cell_h);
            }
        }
    }

    fn split(&mut self, orientation: Orientation) {
        let focused = self.layout.focused();
        let Some(frect) = self
            .layout
            .tab()
            .layout_rects(self.client_area())
            .into_iter()
            .find(|(id, _)| *id == focused)
            .map(|(_, r)| r)
        else {
            return;
        };
        let (cols, lines) = match orientation {
            Orientation::Horizontal => {
                let half = Rect { w: frect.w * 0.5, ..frect };
                let (cols, lines) = self.cols_lines_for(&half);
                (cols.max(1), lines)
            },
            Orientation::Vertical => {
                let half = Rect { h: frect.h * 0.5, ..frect };
                let (cols, lines) = self.cols_lines_for(&half);
                (cols, lines.max(1))
            },
        };
        if let Some(new_id) = self.spawn_pane(cols, lines) {
            self.layout.tab_mut().split(focused, new_id, orientation);
            self.layout.tab_mut().focus(new_id);
            self.recompute_pane_sizes();
        }
    }

    fn remove_pane(&mut self, id: PaneId) {
        if !self.layout.contains(id) {
            return;
        }
        let last_pane = self.layout.tab_mut().remove(id);
        if last_pane {
            self.quit = true;
        }
        self.layout.refresh_focus();
        if let Some(pane) = self.panes.remove(&id) {
            let mut pane = pane;
            pane.quit();
        }
    }

    fn close_pane(&mut self) {
        let id = self.layout.focused();
        self.remove_pane(id);
    }

    fn focused_pane(&mut self) -> Option<&mut TerminalPane> {
        let id = self.layout.focused();
        self.panes.get_mut(&id)
    }

    fn change_font_to(&mut self, size: f32) {
        if size == self.font_size {
            return;
        }
        self.font_size = size;
        if let Some(fonts) = &mut self.fonts {
            fonts.set_font_size(self.font_size, self.dpi_scale);
        }
        self.recompute_pane_sizes();
    }

    fn copy(&mut self) {
        let text = self
            .panes
            .get(&self.layout.focused())
            .and_then(|p| p.selection_text())
            .filter(|t| !t.is_empty());
        if let Some(text) = text {
            self.store_clipboard(&text);
            self.store_selection(&text);
        }
    }

    fn paste(&mut self) {
        if let Some(text) = self.load_clipboard() {
            if let Some(pane) = self.focused_pane() {
                pane.write_str(&text);
            }
        }
    }

    fn paste_selection(&mut self) {
        if let Some(text) = self.load_selection() {
            if let Some(pane) = self.focused_pane() {
                pane.write_str(&text);
            }
        }
    }

    fn store_clipboard(&mut self, text: &str) {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        if let Some(c) = &mut self.clipboard {
            let _ = c.set_text(text.to_string());
        }
    }

    fn load_clipboard(&mut self) -> Option<String> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut().and_then(|c| c.get_text().ok())
    }

    /// Store text in the PRIMARY selection buffer (X11 middle-click paste).
    fn store_selection(&mut self, text: &str) {
        #[cfg(target_os = "linux")]
        {
            use arboard::{LinuxClipboardKind, SetExtLinux};
            if self.clipboard.is_none() {
                self.clipboard = arboard::Clipboard::new().ok();
            }
            if let Some(c) = &mut self.clipboard {
                let _ = c.set().clipboard(LinuxClipboardKind::Primary).text(text.to_string());
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = text;
    }

    /// Load text from the PRIMARY selection buffer (X11 middle-click paste).
    fn load_selection(&mut self) -> Option<String> {
        #[cfg(target_os = "linux")]
        {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            if self.clipboard.is_none() {
                self.clipboard = arboard::Clipboard::new().ok();
            }
            self.clipboard
                .as_mut()
                .and_then(|c| c.get().clipboard(LinuxClipboardKind::Primary).text().ok())
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    fn open_search(&mut self) {
        if let Some(pane) = self.focused_pane() {
            pane.close_search();
        }
        self.search = Some(SearchOverlay { query: String::new() });
    }

    /// Open the internal `/command` overlay regardless of shell line state.
    ///
    /// The plain-`/` trigger only fires while `line_start` is true, which goes
    /// stale after a TUI app exits or a line is interrupted (no Enter reaches
    /// the shell). A keybinding gives a deterministic way to reach `/theme`
    /// and future commands.
    fn open_command(&mut self) {
        self.command = Some("/".to_string());
        self.line_start = false;
        self.command_message = None;
        self.pending_line.clear();
        self.window().request_redraw();
    }

    fn update_search(&mut self) {
        let query = self.search.as_ref().map(|s| s.query.clone()).unwrap_or_default();
        if let Some(pane) = self.focused_pane() {
            if query.is_empty() {
                pane.close_search();
            } else if pane.start_search(&query) {
                pane.collect_matches();
            }
        }
    }

    fn handle_action(&mut self, action: &str) {
        match action {
            "split_right" => self.split(Orientation::Horizontal),
            "split_below" => self.split(Orientation::Vertical),
            "close_pane" => self.close_pane(),
            "next_pane" => self.layout.tab_mut().focus_next(true),
            "prev_pane" => self.layout.tab_mut().focus_prev(true),
            "focus_pane_up" => self.layout.tab_mut().focus_direction(layout::Direction::Up),
            "focus_pane_down" => self.layout.tab_mut().focus_direction(layout::Direction::Down),
            "focus_pane_left" => self.layout.tab_mut().focus_direction(layout::Direction::Left),
            "focus_pane_right" => self.layout.tab_mut().focus_direction(layout::Direction::Right),
            "scroll_up" => {
                if let Some(pane) = self.focused_pane() {
                    pane.scroll(Scroll::Delta(3));
                }
            },
            "scroll_down" => {
                if let Some(pane) = self.focused_pane() {
                    pane.scroll(Scroll::Delta(-3));
                }
            },
            "page_up" => {
                if let Some(pane) = self.focused_pane() {
                    pane.scroll(Scroll::PageUp);
                }
            },
            "page_down" => {
                if let Some(pane) = self.focused_pane() {
                    pane.scroll(Scroll::PageDown);
                }
            },
            "scroll_top" => {
                if let Some(pane) = self.focused_pane() {
                    pane.scroll(Scroll::Top);
                }
            },
            "scroll_bottom" => {
                if let Some(pane) = self.focused_pane() {
                    pane.scroll(Scroll::Bottom);
                }
            },
            "search" => self.open_search(),
            "command" => self.open_command(),
            "search_next" => {
                if let Some(pane) = self.focused_pane() {
                    pane.search_next();
                }
            },
            "search_prev" => {
                if let Some(pane) = self.focused_pane() {
                    pane.search_prev();
                }
            },
            "copy" => self.copy(),
            "paste" => self.paste(),
            "font_increase" => self.change_font_to(self.font_size + 1.0),
            "font_decrease" => self.change_font_to(self.font_size - 1.0),
            "font_reset" => self.change_font_to(self.base_font_size),
            "quit" => self.quit = true,
            _ => {},
        }
    }

    fn on_key(&mut self, event: KeyEvent) {
        // Search overlay consumes most input while open.
        if self.search.is_some() {
            self.on_search_key(&event);
            return;
        }

        // Start composing an internal `/command` when `/` begins a fresh line.
        if self.command.is_none()
            && command_trigger(self.mods, self.line_start, event.state, event.repeat, &event.logical_key)
        {
            self.command = Some("/".to_string());
            self.line_start = false;
            self.pending_line.clear();
            self.window().request_redraw();
            return;
        }

        // While composing, keys edit the buffer instead of reaching the shell.
        if let Some(mut cmd) = self.command.take() {
            match self.on_command_key(&mut cmd, &event) {
                CommandKey::Keep => {
                    self.command = Some(cmd);
                    self.window().request_redraw();
                    return;
                },
                CommandKey::Execute => {
                    self.line_start = true;
                    self.command_message = None;
                    if cmd.trim_start().starts_with("/theme") {
                        self.run_theme_command(&cmd);
                    } else {
                        // Not an internal command: hand the whole line to the shell.
                        self.write_to_shell(&cmd);
                        self.write_to_shell("\r");
                    }
                    self.pending_line.clear();
                    self.window().request_redraw();
                    return;
                },
                CommandKey::Cancel => {
                    // Nothing was sent to the shell while composing, so the
                    // shell is still sitting at its prompt.
                    self.line_start = true;
                    self.pending_line.clear();
                    self.window().request_redraw();
                    return;
                },
                CommandKey::Flush => {
                    // An unexpected key (arrows, shortcuts, ...) ended the
                    // command: let the shell see what was typed so far, then
                    // process this key normally below.
                    self.line_start = false;
                    self.pending_line.clear();
                    self.write_to_shell(&cmd);
                },
            }
        }

        // Resolve keybindings once per physical press; ignore auto-repeat so
        // actions like paste do not fire on key release or while held.
        if event.state == ElementState::Pressed && !event.repeat {
            match input::resolve(&event, self.mods, &self.config.keybindings) {
                input::BindingResult::Action(action) => {
                    self.handle_action(&action);
                    return;
                },
                input::BindingResult::Passthrough => {},
            }
        }

        if event.state != ElementState::Pressed {
            // Forward releases only when the kitty keyboard protocol requests
            // them; otherwise ignore them here.
            let id = self.layout.focused();
            let report_events = self
                .panes
                .get(&id)
                .map(|pane| pane.term.lock().mode().contains(TermMode::REPORT_EVENT_TYPES))
                .unwrap_or(false);
            if !report_events {
                return;
            }
        }

        let mods = self.mods;
        let mode = {
            let id = self.layout.focused();
            if let Some(pane) = self.panes.get(&id) {
                *pane.term.lock().mode()
            } else {
                TermMode::default()
            }
        };

        // A plain Enter submits the shell line. Detect it from the key event
        // rather than the encoded bytes: the kitty keyboard protocol encodes
        // Enter as `CSI 13 u`, not a literal `\r`, so the old byte check made
        // `line_start` go stale and `/theme` stopped triggering.
        let enter_pressed = event.state == ElementState::Pressed
            && !event.repeat
            && matches!(&event.logical_key, Key::Named(NamedKey::Enter));

        // Swallow magic shell lines (e.g. `theme ayu`) before Enter reaches
        // the shell, so the shell never sees (or errors on) the command.
        if enter_pressed && self.try_magic_line() {
            self.line_start = true;
            self.window().request_redraw();
            return;
        }

        let bytes = input::encode_key(&event, mods, mode);
        if !bytes.is_empty() {
            if let Some(pane) = self.focused_pane() {
                pane.write(&bytes);
            }
            if enter_pressed {
                // A line was submitted: the next key starts a fresh one.
                self.pending_line.clear();
                self.line_start = true;
            } else if mods.ctrl {
                // Line-editing keys (Ctrl-C, Ctrl-A, Ctrl-K, ...) change the
                // shell's buffer in ways we cannot shadow reliably; reset the
                // pending line rather than guess wrong.
                self.pending_line.clear();
                self.line_start = false;
            } else {
                // Plain text on the current line: shadow it and leave the
                // line-start heuristic so `/` only opens a command at the
                // very beginning of a line.
                self.update_pending_line(&bytes);
                self.line_start = false;
            }
        }
    }

    fn on_search_key(&mut self, event: &KeyEvent) {
        let action = match &event.logical_key {
            Key::Named(NamedKey::Escape) => SearchAction::Close,
            Key::Named(NamedKey::Enter) if event.state == ElementState::Pressed => SearchAction::NextAndClose,
            Key::Named(NamedKey::Backspace) if event.state == ElementState::Pressed => SearchAction::Backspace,
            Key::Character(ch) if event.state == ElementState::Pressed && !event.repeat => {
                SearchAction::Push(ch.to_string())
            },
            _ => return,
        };
        match action {
            SearchAction::Close => {
                self.search = None;
                self.update_search();
            },
            SearchAction::NextAndClose => {
                if let Some(pane) = self.focused_pane() {
                    pane.search_next();
                }
                self.search = None;
            },
            SearchAction::Backspace => {
                if let Some(search) = self.search.as_mut() {
                    search.query.pop();
                }
                self.update_search();
            },
            SearchAction::Push(ch) => {
                if let Some(search) = self.search.as_mut() {
                    search.query.push_str(&ch);
                }
                self.update_search();
            },
        }
    }

    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        let cell_h = self.fonts.as_ref().map(|f| f.cell_h).unwrap_or(16.0);
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i32,
            MouseScrollDelta::PixelDelta(pos) => (pos.y / cell_h as f64) as i32,
        };
        if lines != 0 {
            if let Some(pane) = self.focused_pane() {
                pane.scroll(Scroll::Delta(lines));
            }
        }
    }

    /// Edit an in-progress `/command` buffer; returns what to do with it.
    fn on_command_key(&self, cmd: &mut String, event: &KeyEvent) -> CommandKey {
        command_edit(cmd, self.mods, event.state, event.repeat, &event.logical_key)
    }

    /// Execute a `/theme <name>` command; no argument lists available themes.
    fn run_theme_command(&mut self, line: &str) {
        let args: Vec<&str> = line.split_whitespace().collect();
        if args.len() < 2 {
            self.list_themes();
            return;
        }
        let name = args[1];
        self.set_theme(name);
    }

    /// Apply a theme by name. Returns the feedback text, which is also shown
    /// in the overlay so `/theme` and `optix-msg` share one code path.
    fn set_theme(&mut self, name: &str) -> String {
        match crate::themes::by_name(name) {
            Some(theme) => {
                self.palette = Palette::from_theme(&theme);
                log::info!("theme set to `{name}`");
                let msg = format!("theme set to {name}");
                self.command_message = Some((msg.clone(), Instant::now()));
                msg
            },
            None => {
                let msg = format!("error: unknown theme `{name}`");
                self.command_message = Some((msg.clone(), Instant::now()));
                msg
            },
        }
    }

    /// List the available themes. Returns the feedback text (shared with the
    /// overlay and `optix-msg themes`).
    fn list_themes(&mut self) -> String {
        let names = crate::themes::names();
        let msg = format!(
            "themes: {} — try `theme <name>`, or drop <name>.toml in ~/.config/optix/themes/",
            names.join(", ")
        );
        self.command_message = Some((msg.clone(), Instant::now()));
        msg
    }

    /// Drain IPC requests from `optix-msg`, execute them, and send the reply
    /// back to the waiting client. Returns true if the UI should redraw.
    fn handle_ipc(&mut self) -> bool {
        let mut dirty = false;
        while let Ok(request) = self.ipc_rx.try_recv() {
            dirty = true;
            let reply = self.execute_ipc(&request.command);
            let _ = request.reply.send(reply);
        }
        dirty
    }

    /// Execute a single IPC command line and produce the reply text.
    fn execute_ipc(&mut self, line: &str) -> String {
        match crate::ipc::IpcCommand::parse(line) {
            crate::ipc::IpcCommand::Ping => "pong".to_string(),
            crate::ipc::IpcCommand::Themes => self.list_themes(),
            crate::ipc::IpcCommand::ThemeSet(name) => self.set_theme(&name),
            crate::ipc::IpcCommand::Quit => {
                self.quit = true;
                "ok: quitting".to_string()
            },
            crate::ipc::IpcCommand::Unknown(cmd) => {
                format!("error: unknown command `{cmd}` (try `theme <name>`, `themes`, `ping`, `quit`)")
            },
        }
    }

    /// If the text typed since the last Enter is a magic command (e.g. `theme
    /// ayu`), swallow it here: clear the shell's line editor and run the
    /// action internally. Returns true when the line was consumed, in which
    /// case the caller must not forward the Enter to the shell.
    fn try_magic_line(&mut self) -> bool {
        if !self.config.magic_enabled {
            return false;
        }
        let Some(cmd) = crate::magic::parse(&self.pending_line) else { return false };
        // The line is consumed by the terminal either way; clear the shadow
        // buffer so a later Enter on an empty line cannot re-run it.
        self.pending_line.clear();
        // The shell already echoed the line and holds it in its line editor;
        // cancel it (Ctrl-U = kill-to-line-start in bash/zsh readline default
        // bindings) so it is never executed on a later Enter.
        self.write_to_shell("\x15");
        match cmd {
            crate::magic::Magic::ThemeList => self.list_themes(),
            crate::magic::Magic::ThemeSet(name) => self.set_theme(&name),
        };
        true
    }

    /// Track text forwarded to the shell so the current line can be recognized
    /// as a magic command when Enter is pressed. Called for non-Enter keys.
    fn update_pending_line(&mut self, bytes: &[u8]) {
        track_pending_line(&mut self.pending_line, bytes);
    }

    /// Write raw text into the focused pane's PTY (as if typed).
    fn write_to_shell(&mut self, text: &str) {
        if let Some(pane) = self.focused_pane() {
            pane.write_str(text);
        }
    }

    fn pane_at(&self, pos: (f64, f64)) -> Option<(PaneId, Rect)> {
        let (x, y) = pos;
        let area = self.client_area();
        self.layout
            .tab()
            .layout_rects(area)
            .into_iter()
            .find(|(_, r)| r.contains(x as f32, y as f32))
    }

    fn on_mouse(&mut self, state: ElementState, button: MouseButton) {
        match button {
            MouseButton::Left => self.on_mouse_left(state),
            // X11 convention: middle-click pastes the PRIMARY selection. Fire on
            // press only, otherwise the click's press + release would paste twice.
            MouseButton::Middle if state == ElementState::Pressed => self.paste_selection(),
            _ => {},
        }
    }

    fn on_mouse_left(&mut self, state: ElementState) {
        let pos = self.mouse_pos;

        match state {
            ElementState::Pressed => {
                if let Some((id, rect)) = self.pane_at(pos) {
                    self.layout.tab_mut().focus(id);
                    let (pad_x, pad_y) = self.padding();
                    let cell_w = self.fonts.as_ref().map(|f| f.cell_w).unwrap_or(8.0);
                    let cell_h = self.fonts.as_ref().map(|f| f.cell_h).unwrap_or(16.0);
                    if let Some(pane) = self.panes.get_mut(&id) {
                        let point = pane.point_at(
                            (pos.0 - rect.x as f64) as f32,
                            (pos.1 - rect.y as f64) as f32,
                            cell_w,
                            cell_h,
                            pad_x,
                            pad_y,
                        );
                        pane.start_selection(SelectionType::Simple, point);
                        self.dragging = true;
                    }
                }
            },
            ElementState::Released => {
                self.dragging = false;
            },
        }
    }

    fn on_mouse_move(&mut self, position: PhysicalPosition<f64>) {
        self.mouse_pos = (position.x, position.y);
        if !self.dragging {
            return;
        }
        let id = self.layout.focused();
        if let Some((_, rect)) = self.pane_at(self.mouse_pos) {
            let (pad_x, pad_y) = self.padding();
            let cell_w = self.fonts.as_ref().map(|f| f.cell_w).unwrap_or(8.0);
            let cell_h = self.fonts.as_ref().map(|f| f.cell_h).unwrap_or(16.0);
            if let Some(pane) = self.panes.get_mut(&id) {
                let point = pane.point_at(
                    (self.mouse_pos.0 - rect.x as f64) as f32,
                    (self.mouse_pos.1 - rect.y as f64) as f32,
                    cell_w,
                    cell_h,
                    pad_x,
                    pad_y,
                );
                pane.update_selection(point);
            }
        }
    }

    /// Re-read the config file if its mtime changed since we last looked.
    /// Returns true if settings were re-applied (caller should redraw).
    fn reload_config_if_changed(&mut self) -> bool {
        // Nothing to apply live until the window + renderer exist.
        if self.window.is_none() {
            return false;
        }
        let mtime = std::fs::metadata(crate::config::config_path())
            .and_then(|m| m.modified())
            .ok();
        if mtime.is_some() && mtime == self.config_mtime {
            return false;
        }
        self.config_mtime = mtime;
        // If the file is currently malformed (mid-save), keep the old settings;
        // the watcher will re-trigger on the next write.
        let Some(new_cfg) = Config::try_load() else { return false };
        log::info!("config changed; applying live reload");
        self.apply_config(new_cfg);
        true
    }

    /// Apply a freshly loaded config to live, running state.
    fn apply_config(&mut self, new_cfg: Config) {
        // Capture old values needed for comparison before swapping.
        let old_family = self.config.font.family.clone();
        let old_base_size = self.base_font_size;
        let old_pad = (self.config.font.padding_x, self.config.font.padding_y);
        let old_corner = self.config.window.corner_radius;
        let old_img = self.config.window.background_image.clone();
        let old_title = self.default_title.clone();
        let old_size = (self.config.window.width, self.config.window.height);
        let old_transparent = self.config.window.transparent;

        // Colors are re-resolved from the new theme; the terminal palette is
        // re-derived so every pane immediately uses the new scheme.
        self.palette = Palette::from_theme(&new_cfg.theme);
        self.config = new_cfg;

        let font_changed = self.config.font.family != old_family
            || self.config.font.size != old_base_size;
        let pad_changed = (self.config.font.padding_x, self.config.font.padding_y) != old_pad;
        if font_changed {
            // Rebuild the font stack at the current (possibly zoomed) size so a
            // family/size edit takes effect immediately. Keep zoom on failure.
            let size = if self.config.font.size == old_base_size { self.font_size } else { self.config.font.size };
            match Fonts::new(&self.config.font.family, size, self.dpi_scale) {
                Ok(f) => {
                    self.fonts = Some(f);
                    self.base_font_size = self.config.font.size;
                    if self.config.font.size != old_base_size {
                        self.font_size = self.config.font.size;
                    }
                },
                Err(err) => log::warn!("font reload failed: {err}"),
            }
        }
        if font_changed || pad_changed {
            self.recompute_pane_sizes();
        }

        if let Some(renderer) = &mut self.renderer {
            if self.config.window.corner_radius != old_corner {
                renderer.set_corner_radius(self.config.window.corner_radius);
            }
            let new_img = self.config.window.background_image.clone();
            if new_img != old_img {
                match &new_img {
                    Some(path) => {
                        if let Err(err) = renderer.load_background_image(path) {
                            log::warn!("{err}");
                        }
                    },
                    None => renderer.clear_background_image(),
                }
            }
        }

        // Window-level settings that can be applied without recreating the window.
        if self.config.window.title != old_title {
            self.default_title = self.config.window.title.clone();
            if self.window().title() == old_title {
                self.window().set_title(&self.default_title);
            }
        }
        if (self.config.window.width, self.config.window.height) != old_size {
            let size = winit::dpi::PhysicalSize::new(
                self.config.window.width.max(1),
                self.config.window.height.max(1),
            );
            let _ = self.window().request_inner_size(size);
        }
        if self.config.window.transparent != old_transparent {
            log::warn!("changing `window.transparent` requires a restart");
        }
    }

    fn handle_events(&mut self) -> bool {
        let mut dirty = false;
        let mut exited: Option<PaneId> = None;
        while let Ok(ev) = self.event_rx.try_recv() {
            dirty = true;
            match ev.kind {
                PaneEventKind::Wakeup | PaneEventKind::CursorBlinkingChange => {},
                PaneEventKind::Title(title) => {
                    if let Some(pane) = self.panes.get_mut(&ev.pane_id) {
                        pane.title = title.clone().unwrap_or_default();
                    }
                    if let Some(title) = title {
                        self.window().set_title(&title);
                    } else {
                        self.window().set_title(&self.default_title);
                    }
                },
                PaneEventKind::Bell => {
                    self.bell_flash = Some(Instant::now());
                },
                PaneEventKind::ClipboardStore(ty, text) => {
                    match ty {
                        ClipboardType::Clipboard => self.store_clipboard(&text),
                        ClipboardType::Selection => self.store_selection(&text),
                    }
                },
                PaneEventKind::ClipboardLoad(ty, formatter) => {
                    let text = match ty {
                        ClipboardType::Clipboard => self.load_clipboard(),
                        ClipboardType::Selection => self.load_selection(),
                    };
                    if let Some(text) = text {
                        let formatted = formatter(&text);
                        if let Some(pane) = self.panes.get_mut(&ev.pane_id) {
                            pane.write_str(&formatted);
                        }
                    }
                },
                PaneEventKind::ColorRequest(idx, formatter) => {
                    let rgb = self.palette.dynamic[idx].unwrap_or_else(|| self.theme_color_at(idx));
                    let formatted = formatter(rgb);
                    if let Some(pane) = self.panes.get_mut(&ev.pane_id) {
                        pane.write_str(&formatted);
                    }
                },
                PaneEventKind::TextAreaSizeRequest(formatter) => {
                    if let Some(pane) = self.panes.get_mut(&ev.pane_id) {
                        let fonts = self.fonts.as_ref().expect("fonts ready");
                        let size = WindowSize {
                            num_lines: pane.lines as u16,
                            num_cols: pane.cols as u16,
                            cell_width: fonts.cell_w as u16,
                            cell_height: fonts.cell_h as u16,
                        };
                        let formatted = formatter(size);
                        pane.write_str(&formatted);
                    }
                },
                PaneEventKind::PtyWrite(text) => {
                    if let Some(pane) = self.panes.get_mut(&ev.pane_id) {
                        pane.write_str(&text);
                    }
                },
                PaneEventKind::Exit(status) => {
                    log::info!("pane {} exited ({status})", ev.pane_id);
                    exited = Some(ev.pane_id);
                },
            }
        }
        if let Some(id) = exited {
            self.remove_pane(id);
        }
        dirty
    }

    fn theme_color_at(&self, idx: usize) -> Rgb {
        let c = match idx {
            0..=7 => self.palette.normal[idx],
            8..=15 => self.palette.bright[idx - 8],
            16 => self.palette.foreground,
            17 => self.palette.background,
            18 => self.palette.cursor,
            19..=27 => self.palette.dim[idx - 19],
            _ => self.palette.foreground,
        };
        Rgb { r: (c.r * 255.0).round() as u8, g: (c.g * 255.0).round() as u8, b: (c.b * 255.0).round() as u8 }
    }

    fn draw(&mut self) {
        let area = self.client_area();
        let (pad_x, pad_y) = self.padding();
        let (w, h) = self.window_size();

        let Some(renderer) = self.renderer.as_mut() else { return };
        let Some(fonts) = self.fonts.as_mut() else { return };
        let theme = &self.config.theme;
        let palette = &self.palette;
        let opacity = self.config.window.opacity;

        let mut frame = Frame::default();

        // Window background: image, gradient, or flat color. The whole window
        // is faded to `opacity` so a transparent (ARGB) surface lets picom
        // composite the desktop wallpaper through it.
        if renderer.has_background_image() {
            frame.image_quad(0.0, 0.0, w, h, renderer.background_uv(), Rgba::from_rgba(1.0, 1.0, 1.0, opacity));
        } else if let Some((top, bottom)) = theme.background_gradient {
            let slices = 32.0;
            for i in 0..slices as usize {
                let t = i as f32 / slices;
                let y = h * t;
                frame.rect(0.0, y, w, h / slices + 1.0, top.lerp(bottom, t).with_alpha(opacity));
            }
        } else {
            frame.rect(0.0, 0.0, w, h, palette.background.with_alpha(opacity));
        }

        let rects = self.layout.tab().layout_rects(area);
        let focused_id = self.layout.focused();

        // Recollect search matches for the focused pane before locking.
        if self.search.is_some() {
            if let Some(pane) = self.panes.get_mut(&focused_id) {
                if let Some(state) = pane.search.as_mut() {
                    if state.dirty {
                        pane.collect_matches();
                    }
                }
            }
        }

        for (id, rect) in rects.iter().copied() {
            if let Some(pane) = self.panes.get_mut(&id) {
                render_pane(
                    &mut frame,
                    pane,
                    &rect,
                    fonts,
                    palette,
                    theme,
                    id == focused_id && self.focused,
                    opacity,
                    pad_x,
                    pad_y,
                );
            }
        }

        // Upload kitty-graphics textures and push their placement quads.
        let mut keep = std::collections::HashSet::new();
        for (id, rect) in rects.iter().copied() {
            let Some(pane) = self.panes.get_mut(&id) else { continue };
            let cell_w = fonts.cell_w;
            let cell_h = fonts.cell_h;
            let store = pane.kitty.lock().unwrap_or_else(|e| e.into_inner());
            for img in store.images.values() {
                keep.insert(img.gen);
                renderer.upload_image(img.gen, img.width, img.height, &img.rgba);
            }
            for p in &store.placements {
                let Some(img) = store.images.get(&p.image_id) else { continue };
                if let Some((x, y, w, h)) = placement_quad(p, img, &rect, pad_x, pad_y, cell_w, cell_h) {
                    log::debug!(
                        "kitty quad {}x{} at ({:.0},{:.0}) cell={:.2}x{:.2} rect={rect:?} src={}x{} cells={:?}x{:?}",
                        w,
                        h,
                        x,
                        y,
                        cell_w,
                        cell_h,
                        img.width,
                        img.height,
                        p.cells_w,
                        p.cells_h
                    );
                    frame.kitty_quad(x, y, w, h, p.gen);
                }
            }
        }
        renderer.prune_images(&keep);

        for border in self.layout.tab().split_borders(area) {
            frame.rect(border.x, border.y, border.w, border.h, theme.split_border);
        }

        if let Some((_, focused_rect)) = rects.iter().find(|(id, _)| *id == focused_id) {
            frame.rect(focused_rect.x, focused_rect.y, focused_rect.w, 1.0, theme.split_active);
        }

        if self.search.is_some() {
            if let Some((_, pane_rect)) = rects.iter().find(|(id, _)| *id == focused_id) {
                draw_search_overlay(&mut frame, fonts, &self.search, theme, pane_rect, 30.0 * self.dpi_scale);
            }
        }

        // In-progress `/command` and its transient result message.
        if let Some(cmd) = &self.command {
            if let Some((_, pane_rect)) = rects.iter().find(|(id, _)| *id == focused_id) {
                draw_command_overlay(&mut frame, fonts, &format!("{cmd}█"), theme, pane_rect, 30.0 * self.dpi_scale);
            }
        }
        if let Some((msg, when)) = &self.command_message {
            if when.elapsed() < Duration::from_secs(4) {
                if let Some((_, pane_rect)) = rects.iter().find(|(id, _)| *id == focused_id) {
                    draw_command_overlay(&mut frame, fonts, msg, theme, pane_rect, 30.0 * self.dpi_scale);
                }
            } else {
                self.command_message = None;
            }
        }

        if let Some(flash) = self.bell_flash {
            if flash.elapsed() < Duration::from_millis(250) {
                frame.rect(0.0, 0.0, w, h, theme.bell.with_alpha(0.12));
            } else {
                self.bell_flash = None;
            }
        }

        log::debug!(
            "frame: w={w:.0} h={h:.0} area={area:?} {} rects {} glyphs",
            frame.rects.len(),
            frame.glyphs.len()
        );

        if let Err(err) = renderer.render(&frame, fonts) {
            log::error!("render failed: {err}");
        }
    }
}

fn draw_search_overlay(
    frame: &mut Frame,
    fonts: &mut Fonts,
    search: &Option<SearchOverlay>,
    theme: &ParsedTheme,
    pane_rect: &Rect,
    height: f32,
) {
    let Some(search) = search else { return };
    let text = format!("Search: {}█", search.query);
    draw_command_overlay(frame, fonts, &text, theme, pane_rect, height);
}

/// Draw a text bar across the top of the focused pane (search or `/command`).
fn draw_command_overlay(
    frame: &mut Frame,
    fonts: &mut Fonts,
    text: &str,
    theme: &ParsedTheme,
    pane_rect: &Rect,
    height: f32,
) {
    frame.rect(pane_rect.x, pane_rect.y, pane_rect.w, height, theme.search_background);
    let glyphs = fonts.layout_paragraph(text, Some(pane_rect.w - 20.0), false, height * 0.5);
    for g in glyphs {
        frame.glyph(pane_rect.x + 10.0 + g.x, pane_rect.y + g.y, g.cache_key, theme.search_foreground);
    }
}

/// Compute the screen-space quad for a kitty placement, preserving the image's
/// aspect ratio when only one dimension is specified.
fn placement_quad(
    p: &Placement,
    img: &KittyImage,
    rect: &Rect,
    pad_x: f32,
    pad_y: f32,
    cell_w: f32,
    cell_h: f32,
) -> Option<(f32, f32, f32, f32)> {
    let src_w = img.width as f32;
    let src_h = img.height as f32;
    if src_w <= 0.0 || src_h <= 0.0 {
        return None;
    }
    let (w, h) = match (p.cells_w, p.cells_h) {
        (Some(c), Some(r)) => (c * cell_w, r * cell_h),
        (Some(c), None) => {
            let w = c * cell_w;
            (w, w * src_h / src_w)
        },
        (None, Some(r)) => {
            let h = r * cell_h;
            (h * src_w / src_h, h)
        },
        (None, None) => match (p.px_w, p.px_h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * src_h / src_w),
            (None, Some(h)) => (h * src_w / src_h, h),
            (None, None) => (src_w, src_h),
        },
    };
    let x = rect.x + pad_x + p.col as f32 * cell_w;
    let y = rect.y + pad_y + p.row as f32 * cell_h;
    Some((x, y, w, h))
}

/// Draw a single pane's grid, cursor, and search highlights into the frame.
#[allow(clippy::too_many_arguments)]
fn render_pane(
    frame: &mut Frame,
    pane: &mut TerminalPane,
    rect: &Rect,
    fonts: &mut Fonts,
    palette: &Palette,
    theme: &ParsedTheme,
    focused: bool,
    opacity: f32,
    pad_x: f32,
    pad_y: f32,
) {
    let cell_w = fonts.cell_w;
    let cell_h = fonts.cell_h;

    // Pane background at window opacity so the gradient/image shows through.
    frame.rect(rect.x, rect.y, rect.w, rect.h, palette.background.with_alpha(opacity));

    let guard = pane.term.lock();
    let content = guard.renderable_content();
    let display_offset = content.display_offset as i32;
    let dynamic = content.colors;
    let cursor = content.cursor;
    let selection = guard.selection.as_ref().and_then(|s| s.to_range(&guard));
    log::debug!("pane {} rect={rect:?} cols={} lines={} cursor_shape={:?} at={:?}", pane.id, pane.cols, pane.lines, cursor.shape, cursor.point);

    let mut cell_count = 0usize;

    for indexed in content.display_iter {
        let cell = indexed.cell;
        let point = indexed.point;
        if !is_visible(cell) {
            continue;
        }
        let row = point.line.0 + display_offset;
        let col = point.column.0;
        if row < 0 || row >= pane.lines as i32 || col >= pane.cols {
            continue;
        }
        cell_count += 1;
        if cell.c != ' ' && cell_count <= 80 {
            log::debug!("cell ({row},{col}) {:?} fg={:?} bg={:?}", cell.c, cell.fg, cell.bg);
        }
        let x = rect.x + pad_x + col as f32 * cell_w;
        let y = rect.y + pad_y + row as f32 * cell_h;

        let mut fg = cell.fg;
        let mut bg = cell.bg;
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }

        let cell_fg = if cell.flags.contains(Flags::DIM) {
            if let Color::Named(n) = fg {
                let idx = n as usize;
                if idx < 8 {
                    if let Some(rgb) = dynamic[259 + idx] {
                        from_ansi_rgb(rgb)
                    } else {
                        palette.dim[idx]
                    }
                } else {
                    palette.resolve(&fg, dynamic)
                }
            } else {
                palette.resolve(&fg, dynamic)
            }
        } else {
            palette.resolve(&fg, dynamic)
        };

        let bg_rgba = palette.resolve(&bg, dynamic);
        let is_default_bg = matches!(bg, Color::Named(n) if n as usize == NamedColor::Background as usize);
        if !is_default_bg {
            frame.rect(x, y, cell_w, cell_h, bg_rgba);
        }

        if pane.search_active {
            let in_match = pane
                .search
                .as_ref()
                .map(|s| s.matches.iter().any(|m| m.contains(&point)))
                .unwrap_or(false);
            if in_match {
                frame.rect(x, y, cell_w, cell_h, theme.search_match_background);
            }
        }

        if selection.as_ref().map(|r| r.contains(point)).unwrap_or(false) {
            frame.rect(x, y, cell_w, cell_h, theme.selection_background);
        }

        if cell.c != ' ' && cell.c != KITTY_MARKER {
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            for g in fonts.layout_cell(cell.c, bold, italic) {
                frame.glyph(x + g.x, y + g.y, g.cache_key, cell_fg);
            }
        }

        if cell.flags.contains(Flags::UNDERLINE) {
            frame.rect(x, y + cell_h - 2.0, cell_w, 1.5, cell_fg);
        }
        if cell.flags.contains(Flags::STRIKEOUT) {
            frame.rect(x, y + cell_h * 0.55, cell_w, 1.0, cell_fg);
        }
    }
    log::debug!("pane {} visible cells: {cell_count}", pane.id);

    // Cursor.
    if cursor.shape != CursorShape::Hidden && focused {
        let row = cursor.point.line.0 + display_offset;
        let col = cursor.point.column.0;
        if row >= 0 && (row as usize) < pane.lines && col < pane.cols {
            let cx = rect.x + pad_x + col as f32 * cell_w;
            let cy = rect.y + pad_y + row as f32 * cell_h;
            let cursor_color = dynamic[NamedColor::Cursor as usize].map(from_ansi_rgb).unwrap_or(palette.cursor);
            let blinking = guard.cursor_style().blinking;
            if !blinking || (Instant::now().elapsed().as_millis() / 500).is_multiple_of(2) {
                match cursor.shape {
                    CursorShape::Block => {
                        frame.rect(cx, cy, cell_w, cell_h, cursor_color);
                        if let Some(ct) = palette.cursor_text {
                            let ch = guard.grid()[cursor.point.line][cursor.point.column].c;
                            if ch != ' ' && ch != KITTY_MARKER {
                                let glyphs = fonts.layout_line(&ch.to_string(), false, false);
                                for g in glyphs {
                                    frame.glyph(cx + g.x, cy + g.y, g.cache_key, ct);
                                }
                            }
                        }
                    },
                    CursorShape::Underline => {
                        frame.rect(cx, cy + cell_h - 2.0, cell_w, 2.0, cursor_color);
                    },
                    CursorShape::Beam => {
                        frame.rect(cx, cy, 2.0, cell_h, cursor_color);
                    },
                    CursorShape::HollowBlock => {
                        frame.rect(cx, cy, cell_w, 1.5, cursor_color);
                        frame.rect(cx, cy + cell_h - 1.5, cell_w, 1.5, cursor_color);
                        frame.rect(cx, cy, 1.5, cell_h, cursor_color);
                        frame.rect(cx + cell_w - 1.5, cy, 1.5, cell_h, cursor_color);
                    },
                    CursorShape::Hidden => {},
                }
            }
        }
    }
}

impl ApplicationHandler for OptixApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let width = self.config.window.width;
        let height = self.config.window.height;
        let attrs = Window::default_attributes()
            .with_title(&self.config.window.title)
            .with_transparent(self.config.window.transparent)
            .with_inner_size(PhysicalSize::new(width, height));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(err) => {
                log::error!("failed to create window: {err}");
                event_loop.exit();
                return;
            },
        };
        let size = window.inner_size();
        let dpi = window.scale_factor() as f32;
        self.dpi_scale = dpi;
        self.default_title = window.title();

        let mut renderer = match Renderer::new(&window, size, dpi, self.config.window.corner_radius, self.config.window.transparent) {
            Ok(r) => r,
            Err(err) => {
                log::error!("failed to initialize renderer: {err}");
                event_loop.exit();
                return;
            },
        };
        if let Some(path) = &self.config.window.background_image {
            if let Err(err) = renderer.load_background_image(path) {
                log::warn!("{err}");
            }
        }
        let fonts = match Fonts::new(&self.config.font.family, self.config.font.size, dpi) {
            Ok(f) => f,
            Err(err) => {
                log::error!("failed to initialize fonts: {err}");
                event_loop.exit();
                return;
            },
        };

        self.renderer = Some(renderer);
        self.fonts = Some(fonts);

        // Spawn the initial pane sized for the full tab area.
        let area = self.client_area();
        let (cols, lines) = self.cols_lines_for(&area);
        let pane_id = self.spawn_pane(cols.max(1), lines.max(1)).unwrap_or_else(|| {
            log::error!("no shell pane could be spawned");
            event_loop.exit();
            std::process::exit(1);
        });
        self.layout.tab_mut().focused = pane_id;

        self.window = Some(window);
        self.window().request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size, self.dpi_scale);
                }
                self.recompute_pane_sizes();
                self.window().request_redraw();
            },
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.dpi_scale = scale_factor as f32;
                if let Some(fonts) = &mut self.fonts {
                    fonts.set_font_size(self.font_size, self.dpi_scale);
                }
                self.recompute_pane_sizes();
            },
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                let id = self.layout.focused();
                if let Some(pane) = self.panes.get_mut(&id) {
                    pane.set_focus(focused);
                }
                self.window().request_redraw();
            },
            WindowEvent::KeyboardInput { event, .. } => {
                self.on_key(event);
                self.window().request_redraw();
            },
            WindowEvent::ModifiersChanged(modifiers) => {
                self.mods = Mods::from_state(modifiers.state());
            },
            WindowEvent::MouseWheel { delta, .. } => {
                self.on_wheel(delta);
                self.window().request_redraw();
            },
            WindowEvent::MouseInput { state, button, .. } => {
                self.on_mouse(state, button);
                self.window().request_redraw();
            },
            WindowEvent::CursorMoved { position, .. } => {
                let was_dragging = self.dragging;
                self.on_mouse_move(position);
                if was_dragging {
                    self.window().request_redraw();
                }
            },
            WindowEvent::RedrawRequested => {
                self.draw();
            },
            _ => {},
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
            return;
        }
        // Live reload: pick up config.toml edits (colors, fonts, window opts).
        let reloaded = self.reload_config_if_changed();
        // Commands arriving over the IPC socket (`optix-msg theme ayu`, ...).
        let ipc = self.handle_ipc();
        if self.handle_events() || reloaded || ipc {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

/// Pure `CommandKey` edit step, factored out for unit testing.
fn command_edit(cmd: &mut String, mods: Mods, state: ElementState, repeat: bool, key: &Key) -> CommandKey {
    if state != ElementState::Pressed {
        return CommandKey::Keep;
    }
    match key {
        Key::Named(NamedKey::Enter) => CommandKey::Execute,
        Key::Named(NamedKey::Escape) => CommandKey::Cancel,
        Key::Named(NamedKey::Backspace) => {
            cmd.pop();
            if cmd.is_empty() {
                CommandKey::Cancel
            } else {
                CommandKey::Keep
            }
        },
        Key::Character(ch) if !repeat => {
            if mods.is_empty() {
                cmd.push_str(ch);
                CommandKey::Keep
            } else {
                CommandKey::Flush
            }
        },
        _ => CommandKey::Flush,
    }
}

/// Whether a key press should open a `/command` (a plain `/` at line start).
fn command_trigger(mods: Mods, line_start: bool, state: ElementState, repeat: bool, key: &Key) -> bool {
    line_start
        && state == ElementState::Pressed
        && !repeat
        && mods.is_empty()
        && matches!(key, Key::Character(ch) if ch == "/")
}

/// Mirror the shell's line buffer as plain text so magic commands (`theme
/// ayu`) can be recognized when Enter is pressed. `bytes` are what was
/// forwarded to the shell for a single non-Enter key press.
fn track_pending_line(buf: &mut String, bytes: &[u8]) {
    if bytes == b"\x7f" || bytes == b"\x08" {
        // Backspace erases the last character of the pending line.
        buf.pop();
        return;
    }
    // Control bytes (Tab, classically-encoded arrows, ...) don't add text.
    if bytes.iter().any(|b| *b < 0x20 || *b == 0x7f) {
        return;
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        // Magic commands are short; cap the shadow line defensively.
        if buf.len() < 256 {
            buf.push_str(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::Key;

    fn no_mods() -> Mods {
        Mods { ctrl: false, shift: false, alt: false, super_: false }
    }

    #[test]
    fn trigger_only_on_plain_slash_at_line_start() {
        let pressed = ElementState::Pressed;
        let slash = Key::Character("/".into());
        assert!(command_trigger(no_mods(), true, pressed, false, &slash));
        assert!(!command_trigger(no_mods(), false, pressed, false, &slash));
        assert!(!command_trigger(no_mods(), true, ElementState::Released, false, &slash));
        assert!(!command_trigger(no_mods(), true, pressed, true, &slash));

        let ctrl = Mods { ctrl: true, shift: false, alt: false, super_: false };
        assert!(!command_trigger(ctrl, true, pressed, false, &slash));

        let other = Key::Character("x".into());
        assert!(!command_trigger(no_mods(), true, pressed, false, &other));
    }

    #[test]
    fn composing_builds_the_command() {
        let mut cmd = "/".to_string();
        for c in "theme gruvbox".chars() {
            let key = Key::Character(c.to_string().into());
            assert!(matches!(command_edit(&mut cmd, no_mods(), ElementState::Pressed, false, &key), CommandKey::Keep));
        }
        assert_eq!(cmd, "/theme gruvbox");
    }

    #[test]
    fn enter_executes_escape_cancels() {
        let mut cmd = "/theme dracula".to_string();
        let enter = Key::Named(NamedKey::Enter);
        assert!(matches!(command_edit(&mut cmd, no_mods(), ElementState::Pressed, false, &enter), CommandKey::Execute));

        let mut cmd2 = "/theme".to_string();
        let esc = Key::Named(NamedKey::Escape);
        assert!(matches!(command_edit(&mut cmd2, no_mods(), ElementState::Pressed, false, &esc), CommandKey::Cancel));
        assert_eq!(cmd2, "/theme");
    }

    #[test]
    fn backspace_edits_and_empty_cancels() {
        let bs = Key::Named(NamedKey::Backspace);
        let pressed = ElementState::Pressed;

        let mut cmd = "/th".to_string();
        assert!(matches!(command_edit(&mut cmd, no_mods(), pressed, false, &bs), CommandKey::Keep));
        assert_eq!(cmd, "/t");

        let mut cmd = "/t".to_string();
        assert!(matches!(command_edit(&mut cmd, no_mods(), pressed, false, &bs), CommandKey::Keep));
        assert_eq!(cmd, "/");

        let mut cmd = "/".to_string();
        assert!(matches!(command_edit(&mut cmd, no_mods(), pressed, false, &bs), CommandKey::Cancel));
    }

    #[test]
    fn modified_or_other_keys_flush() {
        let arrow = Key::Named(NamedKey::ArrowUp);
        let mut cmd = "/theme".to_string();
        assert!(matches!(command_edit(&mut cmd, no_mods(), ElementState::Pressed, false, &arrow), CommandKey::Flush));

        let ctrl_c = Key::Character("c".into());
        let ctrl = Mods { ctrl: true, shift: false, alt: false, super_: false };
        let mut cmd2 = "/theme".to_string();
        assert!(matches!(command_edit(&mut cmd2, ctrl, ElementState::Pressed, false, &ctrl_c), CommandKey::Flush));
    }

    #[test]
    fn pending_line_shadows_typed_text() {
        let mut buf = String::new();
        for &b in b"theme ayu".iter() {
            track_pending_line(&mut buf, &[b]);
        }
        assert_eq!(buf, "theme ayu");
    }

    #[test]
    fn pending_line_backspace_pops() {
        let mut buf = String::new();
        for &b in b"themea".iter() {
            track_pending_line(&mut buf, &[b]);
        }
        track_pending_line(&mut buf, b"\x7f");
        assert_eq!(buf, "theme");
    }

    #[test]
    fn pending_line_ignores_control_bytes_and_caps_length() {
        let mut buf = String::new();
        track_pending_line(&mut buf, b"\x1b[A"); // arrow escape
        assert!(buf.is_empty());
        track_pending_line(&mut buf, b"\t");
        assert!(buf.is_empty());

        for b in b"a".repeat(300) {
            track_pending_line(&mut buf, &[b]);
        }
        assert_eq!(buf.len(), 256);
    }

    #[test]
    fn magic_lines_and_shell_lines_round_trip() {
        // The pure detection: "theme ayu" is magic, other lines are not.
        assert!(matches!(crate::magic::parse("theme ayu"), Some(crate::magic::Magic::ThemeSet(_))));
        assert!(crate::magic::parse("ls -la").is_none());
        assert!(crate::magic::parse("theme ayu extra").is_none());
    }
}
