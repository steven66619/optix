use std::ops::RangeInclusive;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::search::{RegexIter, RegexSearch};
use alacritty_terminal::term::{cell::Flags, Config as TermConfig, Term, TermMode};
use alacritty_terminal::tty::{self, Options as PtyOptions};

use crate::event::{PaneEvent, PaneProxy};
use crate::kitty::KittyStore;
use crate::pty_io::{PtyIo, PtyMsg};
use crate::scroll::ScrollState;

/// Search state for one terminal pane.
pub struct SearchState {
    pub regex: RegexSearch,
    /// All matches intersecting the current viewport.
    pub matches: Vec<RangeInclusive<Point>>,
    /// Index into `matches` that the cursor is positioned on.
    pub selected: usize,
    /// Set when content/query changed and matches need recollecting.
    pub dirty: bool,
}

/// A single running terminal session (PTY + emulator + search/selection state).
pub struct TerminalPane {
    pub id: usize,
    pub term: Arc<FairMutex<Term<PaneProxy>>>,
    pub pty: PtyIo,
    /// Per-pane kitty-graphics store, populated by the PTY thread.
    pub kitty: Arc<Mutex<KittyStore>>,
    pub title: String,
    pub search: Option<SearchState>,
    /// Whether the terminal had damage since the last frame.
    pub dirty: bool,
    pub cols: usize,
    pub lines: usize,
    /// True while a search is active and matches must be redrawn.
    pub search_active: bool,
    /// Smooth scrollback browsing state (position, momentum, scrollbar fade).
    pub scroll: ScrollState,
}

struct PaneSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for PaneSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

impl TerminalPane {
    /// Spawn a shell inside a fresh PTY with the given grid size.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: usize,
        pty_opts: &PtyOptions,
        cols: usize,
        lines: usize,
        cell_w: u16,
        cell_h: u16,
        tx: mpsc::Sender<PaneEvent>,
        el_wakeup: Option<winit::event_loop::EventLoopProxy<()>>,
        smooth_scroll: bool,
    ) -> std::io::Result<Self> {
        let term_config = TermConfig { kitty_keyboard: true, ..TermConfig::default() };

        let dims = PaneSize { columns: cols, screen_lines: lines };
        let proxy_a = PaneProxy { pane_id: id, tx: tx.clone(), el_wakeup: el_wakeup.clone() };
        let proxy_b = PaneProxy { pane_id: id, tx, el_wakeup };
        let term = Arc::new(FairMutex::new(Term::new(term_config, &dims, proxy_a)));

        let kitty = Arc::new(Mutex::new(KittyStore::new()));

        let window_size = WindowSize {
            num_lines: lines as u16,
            num_cols: cols as u16,
            cell_width: cell_w,
            cell_height: cell_h,
        };

        let pty = tty::new(pty_opts, window_size, id as u64)?;
        let io = PtyIo::spawn(pty, term.clone(), kitty.clone(), proxy_b);

        Ok(Self {
            id,
            term,
            pty: io,
            kitty,
            title: String::new(),
            search: None,
            dirty: true,
            cols,
            lines,
            search_active: false,
            scroll: ScrollState::new(smooth_scroll),
        })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.pty.send(PtyMsg::Write(bytes.to_vec()));
    }

    pub fn write_str(&mut self, s: &str) {
        log::debug!("term write_str {:?}", s);
        self.write(s.as_bytes());
    }

    /// Resize both the emulator and the PTY.
    pub fn resize(&mut self, cols: usize, lines: usize, cell_w: u16, cell_h: u16) {
        if cols == self.cols && lines == self.lines {
            return;
        }
        self.cols = cols;
        self.lines = lines;
        let size = PaneSize { columns: cols, screen_lines: lines };
        let mut guard = self.term.lock();
        guard.resize(size);
        drop(guard);
        self.pty.send(PtyMsg::Resize(WindowSize { num_lines: lines as u16, num_cols: cols as u16, cell_width: cell_w, cell_height: cell_h }));
    }

    /// Ask the PTY thread to shut down.
    pub fn quit(&mut self) {
        self.pty.send(PtyMsg::Shutdown);
    }

    pub fn set_focus(&mut self, focused: bool) {
        let mut guard = self.term.lock();
        guard.is_focused = focused;
    }

    /// Scroll the viewport (ignored while an app is on the alternate screen).
    /// Keyboard-driven actions are applied as instant line steps; wheel input
    /// goes through [`TerminalPane::scroll_wheel`] so it can glide.
    pub fn scroll(&mut self, scroll: Scroll) {
        match scroll {
            Scroll::Delta(lines) => self.scroll_lines(lines as f64, false),
            Scroll::PageUp => self.scroll_lines(self.lines as f64, false),
            Scroll::PageDown => self.scroll_lines(-(self.lines as f64), false),
            Scroll::Top => self.scroll_jump(f64::INFINITY),
            Scroll::Bottom => self.scroll_jump(0.0),
        }
    }

    /// Scroll by `delta` lines. Positive scrolls toward older content.
    /// When `momentum` is set (wheel input) and smooth scrolling is enabled,
    /// the delta is applied as an impulse so the viewport glides.
    pub fn scroll_lines(&mut self, delta: f64, momentum: bool) {
        if self.on_alt_screen() {
            return;
        }
        self.scroll.input(delta, momentum);
    }

    /// Jump to an absolute scroll position (in lines from the bottom).
    pub fn scroll_jump(&mut self, to: f64) {
        if self.on_alt_screen() {
            return;
        }
        let history = self.history_lines();
        self.scroll.jump(to.clamp(0.0, history as f64));
    }

    /// Wheel input with momentum feel; only ever touches the scrollback.
    pub fn scroll_wheel(&mut self, delta: f64, momentum: bool) {
        self.scroll_lines(delta, momentum);
    }

    /// Drop straight back to the prompt after browsing scrollback (e.g. the
    /// user typed something). Immediate, never animated.
    pub fn scroll_to_bottom(&mut self) {
        if self.on_alt_screen() {
            return;
        }
        let mut guard = self.term.lock();
        guard.grid_mut().scroll_display(Scroll::Bottom);
        drop(guard);
        self.scroll.jump(0.0);
        self.scroll.shift = 0.0;
    }

    /// Advance the smooth-scroll animation by `dt` seconds, moving the grid in
    /// whole-line steps. Returns true when another frame is needed.
    pub fn scroll_tick(&mut self, dt: f32, cell_h: f32) -> bool {
        let mut guard = self.term.lock();
        if guard.mode().contains(TermMode::ALT_SCREEN) {
            // Never animate, fade, or touch the grid while a TUI app owns the
            // screen (vim, less, atuin, ...). They scroll themselves.
            self.scroll.resync(0);
            return false;
        }
        // Something else may have moved the grid (search jump, input auto
        // scroll-to-bottom); resync so position and grid never disagree.
        let actual = guard.grid().display_offset();
        if (actual as f64 - self.scroll.applied).abs() > 0.5 {
            self.scroll.resync(actual);
        }
        let history = guard.grid().history_size();
        let (needs_draw, delta) = self.scroll.tick(dt as f64, history);
        if let Some(delta) = delta {
            guard.grid_mut().scroll_display(Scroll::Delta(delta));
            self.scroll.applied = guard.grid().display_offset() as f64;
        }
        // Fractional remainder -> pixel shift for the renderer.
        let frac = self.scroll.pos - self.scroll.pos.round();
        self.scroll.shift = frac as f32 * cell_h;
        needs_draw
    }

    /// Number of scrollback lines available right now (0 on the alt screen).
    pub fn history_lines(&self) -> usize {
        let guard = self.term.lock();
        if guard.mode().contains(TermMode::ALT_SCREEN) {
            return 0;
        }
        guard.grid().history_size()
    }

    /// Whether the pane is on the alternate screen (a TUI app is running).
    pub fn on_alt_screen(&self) -> bool {
        let guard = self.term.lock();
        guard.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Whether the app has enabled any mouse-reporting mode. When true, mouse
    /// events must be forwarded to the PTY (SGR/X10) instead of being consumed
    /// for optix's own scrollback/selection UI.
    pub fn mouse_reporting(&self) -> bool {
        let guard = self.term.lock();
        crate::input::mouse_reporting_active(*guard.mode())
    }

    /// Whether SGR encoding should be used for mouse reports (X10 otherwise).
    pub fn mouse_sgr(&self) -> bool {
        let guard = self.term.lock();
        guard.mode().contains(TermMode::SGR_MOUSE)
    }

    /// Whether the app wants motion (not just click) reports: 1002 drag or
    /// 1003 any-motion reporting.
    pub fn mouse_motion_reports(&self) -> bool {
        let guard = self.term.lock();
        guard.mode().intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
    }

    /// Write a mouse-report sequence to the app's PTY.
    pub fn write_mouse(&mut self, button: u16, col: usize, row: usize, mods: crate::input::Mods) {
        let sgr = self.mouse_sgr();
        let bytes = crate::input::encode_mouse(button, col, row, mods, sgr);
        if !bytes.is_empty() {
            self.write(&bytes);
        }
    }

    /// Write a wheel mouse-report to the app's PTY (discrete steps).
    pub fn write_mouse_wheel(&mut self, delta_lines: f64, col: usize, row: usize, mods: crate::input::Mods) {
        let sgr = self.mouse_sgr();
        let bytes = crate::input::encode_mouse_wheel(delta_lines, col, row, mods, sgr);
        if !bytes.is_empty() {
            self.write(&bytes);
        }
    }

    /// Convert window coordinates (already mapped into the pane's client area, pixels) to a grid point.
    pub fn point_at(&self, x: f32, y: f32, cell_w: f32, cell_h: f32, padding_x: f32, padding_y: f32) -> Point {
        let col = ((x - padding_x) / cell_w).floor().max(0.0) as usize;
        let row = ((y - padding_y) / cell_h).floor().max(0.0) as usize;
        let guard = self.term.lock();
        let display_offset = guard.grid().display_offset();
        let line = (row as i32 - display_offset as i32).max(-(display_offset as i32));
        let col = col.min(self.cols.saturating_sub(1));
        Point::new(Line(line), Column(col))
    }

    /// The visible grid cell under a window point, as 0-based (col, row) in the
    /// current viewport. Used for mouse-report coordinates (SGR/X10), which are
    /// relative to the on-screen grid regardless of scrollback offset.
    pub fn cell_at(&self, x: f32, y: f32, cell_w: f32, cell_h: f32, padding_x: f32, padding_y: f32) -> (usize, usize) {
        let col = ((x - padding_x) / cell_w).floor().max(0.0) as usize;
        let row = ((y - padding_y) / cell_h).floor().max(0.0) as usize;
        let col = col.min(self.cols.saturating_sub(1));
        let row = row.min(self.lines.saturating_sub(1));
        (col, row)
    }

    pub fn clear_selection(&mut self) {
        let mut guard = self.term.lock();
        guard.selection = None;
    }

    pub fn start_selection(&mut self, sel_type: SelectionType, point: Point) {
        let mut guard = self.term.lock();
        guard.selection = Some(Selection::new(sel_type, point, cell_side(point)));
    }

    pub fn update_selection(&mut self, point: Point) {
        let mut guard = self.term.lock();
        if let Some(selection) = guard.selection.as_mut() {
            selection.update(point, cell_side(point));
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        let guard = self.term.lock();
        guard.selection_to_string()
    }

    /// Begin a new search with the given query.
    pub fn start_search(&mut self, query: &str) -> bool {
        match RegexSearch::new(query) {
            Ok(regex) => {
                self.search = Some(SearchState { regex, matches: Vec::new(), selected: 0, dirty: true });
                self.search_active = true;
                true
            },
            Err(err) => {
                log::debug!("invalid search regex: {err}");
                false
            },
        }
    }

    pub fn close_search(&mut self) {
        self.search = None;
        self.search_active = false;
    }

    /// Recollect all matches intersecting the current viewport.
    pub fn collect_matches(&mut self) {
        let Some(state) = self.search.as_mut() else { return };
        let guard = self.term.lock();
        let display_offset = guard.grid().display_offset();
        let lines = guard.screen_lines() as i32;
        let last_col = guard.last_column();
        let start = Point::new(Line(-(display_offset as i32)), Column(0));
        let end = Point::new(Line(lines - 1 - display_offset as i32), last_col);
        let mut matches = Vec::new();
        let iter = RegexIter::new(start, end, Direction::Right, &guard, &mut state.regex);
        for m in iter {
            matches.push(m);
        }
        drop(guard);
        state.matches = matches;
        state.selected = 0;
        state.dirty = false;
    }

    /// Jump the viewport to the next search match.
    pub fn search_next(&mut self) {
        let Some(state) = self.search.as_mut() else { return };
        let mut guard = self.term.lock();
        let origin = guard.grid().cursor.point;
        if let Some(m) = guard.search_next(&mut state.regex, origin, Direction::Right, Side::Right, None) {
            let point = *m.start();
            guard.scroll_to_point(point);
        }
    }

    pub fn search_prev(&mut self) {
        let Some(state) = self.search.as_mut() else { return };
        let mut guard = self.term.lock();
        let origin = guard.grid().cursor.point;
        if let Some(m) = guard.search_next(&mut state.regex, origin, Direction::Left, Side::Left, None) {
            let point = *m.start();
            guard.scroll_to_point(point);
        }
    }
}

fn cell_side(point: Point) -> Side {
    // Side of the cell (left/right) used as the selection boundary; defaults to left.
    let _ = point;
    Side::Left
}

/// Skip wide-char spacers when iterating visible cells.
pub fn is_visible(cell: &alacritty_terminal::term::cell::Cell) -> bool {
    !cell.flags.contains(Flags::WIDE_CHAR_SPACER)
}
