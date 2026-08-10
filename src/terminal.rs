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
    pub fn new(
        id: usize,
        pty_opts: &PtyOptions,
        cols: usize,
        lines: usize,
        cell_w: u16,
        cell_h: u16,
        tx: mpsc::Sender<PaneEvent>,
        el_wakeup: Option<winit::event_loop::EventLoopProxy<()>>,
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
    pub fn scroll(&mut self, scroll: Scroll) {
        let mut guard = self.term.lock();
        if guard.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        guard.scroll_display(scroll);
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
