use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::color::from_ansi_rgb;
use crate::config::{Config, ParsedTheme};
use crate::event::{PaneEvent, PaneEventKind};
use crate::fonts::Fonts;
use crate::input::{self, Mods};
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

/// The terminal application.
pub struct OtermApp {
    config: Config,
    window: Option<Window>,
    renderer: Option<Renderer>,
    fonts: Option<Fonts>,
    panes: HashMap<PaneId, TerminalPane>,
    layout: Layout,
    next_pane_id: PaneId,
    event_tx: mpsc::Sender<PaneEvent>,
    event_rx: mpsc::Receiver<PaneEvent>,
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
}

impl OtermApp {
    pub fn new(config: Config, event_tx: mpsc::Sender<PaneEvent>, event_rx: mpsc::Receiver<PaneEvent>) -> Self {
        let palette = Palette::from_theme(&config.theme);
        let base_font_size = config.font.size;
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
            search: None,
            bell_flash: None,
            focused: true,
            mods: Mods::default(),
            window: None,
            renderer: None,
            fonts: None,
            config,
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
        match TerminalPane::new(id, &opts, cols, lines, cell_w, cell_h, self.event_tx.clone()) {
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
            let _ = pane.pty.send(Msg::Shutdown);
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
        }
    }

    fn paste(&mut self) {
        if let Some(text) = self.load_clipboard() {
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

    fn open_search(&mut self) {
        if let Some(pane) = self.focused_pane() {
            pane.close_search();
        }
        self.search = Some(SearchOverlay { query: String::new() });
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

        match input::resolve(&event, self.mods, &self.config.keybindings) {
            input::BindingResult::Action(action) => {
                self.handle_action(&action);
                return;
            },
            input::BindingResult::Passthrough => {},
        }

        if event.state != ElementState::Pressed {
            return;
        }

        let mods = self.mods;
        let (app_cursor, app_keypad) = {
            let id = self.layout.focused();
            if let Some(pane) = self.panes.get(&id) {
                let guard = pane.term.lock();
                let mode = guard.mode();
                (mode.contains(TermMode::APP_CURSOR), mode.contains(TermMode::APP_KEYPAD))
            } else {
                (false, false)
            }
        };

        let bytes = input::encode_key(&event, mods, app_cursor, app_keypad);
        if !bytes.is_empty() {
            if let Some(pane) = self.focused_pane() {
                pane.write(&bytes);
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
        if button != MouseButton::Left {
            return;
        }
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
                        pane.start_selection(point);
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
                PaneEventKind::ClipboardStore(_, text) => {
                    self.store_clipboard(&text);
                },
                PaneEventKind::ClipboardLoad(_, formatter) => {
                    if let Some(text) = self.load_clipboard() {
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

        // Window background: image, gradient, or flat color.
        if renderer.has_background_image() {
            frame.image_quad(0.0, 0.0, w, h, renderer.background_uv());
        } else if let Some((top, bottom)) = theme.background_gradient {
            let slices = 32.0;
            for i in 0..slices as usize {
                let t = i as f32 / slices;
                let y = h * t;
                frame.rect(0.0, y, w, h / slices + 1.0, top.lerp(bottom, t));
            }
        } else {
            frame.rect(0.0, 0.0, w, h, palette.background);
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
    frame.rect(pane_rect.x, pane_rect.y, pane_rect.w, height, theme.search_background);
    let glyphs = fonts.layout_paragraph(&text, Some(pane_rect.w - 20.0), false, height * 0.5);
    for g in glyphs {
        frame.glyph(pane_rect.x + 10.0 + g.x, pane_rect.y + g.y, g.cache_key, theme.search_foreground);
    }
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

        if cell.c != ' ' {
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            let glyphs = fonts.layout_line(&cell.c.to_string(), bold, italic);
            for g in glyphs {
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
                            if ch != ' ' {
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

impl ApplicationHandler for OtermApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let width = self.config.window.width;
        let height = self.config.window.height;
        let attrs = Window::default_attributes()
            .with_title(&self.config.window.title)
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

        let mut renderer = match Renderer::new(&window, size, dpi, self.config.window.corner_radius) {
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
                self.on_mouse_move(position);
                self.window().request_redraw();
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
        if self.handle_events() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}
