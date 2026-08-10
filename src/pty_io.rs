//! PTY reader/writer thread for one terminal pane.
//!
//! Replaces `alacritty_terminal::event_loop::EventLoop` so that kitty-graphics
//! APC sequences can be intercepted before they reach the emulator. The stock
//! `vte` parser ignores APC payloads outright, so we scan the raw byte stream
//! for them here and pass everything else straight through to the parser.

use std::io::{Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty::{self, EventedReadWrite as _};
use alacritty_terminal::vte::ansi::Processor;

use crate::event::PaneProxy;
use crate::kitty::{KittyAction, KittyStore, KITTY_MARKER, placement_box};

/// Commands from the UI thread to the PTY thread.
pub enum PtyMsg {
    Write(Vec<u8>),
    Resize(WindowSize),
    Shutdown,
}

/// Handle to a spawned pane I/O thread.
pub struct PtyIo {
    tx: Sender<PtyMsg>,
    thread: Option<JoinHandle<()>>,
}

impl PtyIo {
    /// Spawn the I/O thread, taking ownership of the PTY and the terminal.
    pub fn spawn(
        pty: tty::Pty,
        term: Arc<FairMutex<Term<PaneProxy>>>,
        kitty: Arc<Mutex<KittyStore>>,
        proxy: PaneProxy,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<PtyMsg>();
        let thread = thread::Builder::new()
            .name(format!("optix-pty-{}", proxy.pane_id))
            .spawn(move || {
                run_io_thread(pty, term, kitty, proxy, rx);
            })
            .expect("failed to spawn pty thread");
        Self { tx, thread: Some(thread) }
    }

    pub fn send(&self, msg: PtyMsg) {
        let _ = self.tx.send(msg);
    }
}

impl Drop for PtyIo {
    fn drop(&mut self) {
        let _ = self.tx.send(PtyMsg::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// Tracks a partially-received APC (`ESC _` ... `ESC \`) across read chunks.
#[derive(Default)]
struct ApcScanner {
    in_apc: bool,
    esc_seen: bool,
    payload: Vec<u8>,
}

impl ApcScanner {
    /// Scan a chunk of raw bytes. Completed APCs are pushed as `(offset, payload)`
    /// where `offset` is the index in this chunk just past the `ESC \` terminator,
    /// i.e. where the next non-APC byte begins.
    fn scan(&mut self, bytes: &[u8], out: &mut Vec<(usize, Vec<u8>)>) {
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if self.in_apc {
                if b == 0x1b {
                    self.esc_seen = true;
                    i += 1;
                } else if self.esc_seen {
                    self.esc_seen = false;
                    if b == b'\\' {
                        self.in_apc = false;
                        out.push((i + 1, std::mem::take(&mut self.payload)));
                    } else {
                        self.payload.push(0x1b);
                        self.payload.push(b);
                    }
                    i += 1;
                } else {
                    self.payload.push(b);
                    i += 1;
                }
            } else if b == 0x1b {
                self.esc_seen = true;
                i += 1;
            } else if self.esc_seen {
                self.esc_seen = false;
                if b == b'_' {
                    self.in_apc = true;
                    self.payload.clear();
                } else {
                    // Not an APC introducer; the parser will see these bytes.
                }
                i += 1;
            } else {
                i += 1;
            }
        }
    }
}

fn set_nonblocking(fd: i32) {
    // SAFETY: `fd` is a valid pty master fd owned by this thread.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags >= 0 {
        let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }
}

fn resize_pty(fd: i32, size: WindowSize) {
    // SAFETY: `fd` is a valid pty master; `winsize` is POD with no pointers.
    let ws = libc::winsize {
        ws_row: size.num_lines,
        ws_col: size.num_cols,
        ws_xpixel: size.cell_width,
        ws_ypixel: size.cell_height,
    };
    unsafe {
        libc::ioctl(fd, libc::TIOCSWINSZ, &ws);
    }
}

/// Allocate `cols x rows` cells for an inline (`a=T`) image anchored at the
/// grid position `(col, row)`: write `KITTY_MARKER` cells over the box and
/// leave the cursor just past the image's bottom-right cell so text flows
/// around it. The marker cells are what the renderer's reconciliation uses to
/// know the placement is still intact.
fn allocate_kitty_cells(
    parser: &mut Processor,
    term: &mut Term<PaneProxy>,
    col: usize,
    row: isize,
    cols: usize,
    rows: usize,
) {
    use alacritty_terminal::grid::Dimensions;

    if row < 0 {
        return;
    }
    let row = row as usize;
    let columns = term.columns();
    let screen_lines = term.screen_lines();
    if col >= columns || row >= screen_lines {
        return;
    }
    let cols = cols.min(columns - col);
    let rows = rows.min(screen_lines - row);
    if cols == 0 || rows == 0 {
        return;
    }

    // Position each covered row explicitly (CUP) and blank it out, so the box
    // is cleared even when the image hugs the right edge (no wrap ambiguity).
    let marker_line: String = KITTY_MARKER.to_string().repeat(cols);
    let mut bytes = Vec::with_capacity((cols * 3 + 8) * rows);
    let t0 = std::time::Instant::now();
    for r in 0..rows {
        bytes.extend_from_slice(format!("\x1b[{};{}H", row + 1 + r, col + 1).as_bytes());
        bytes.extend_from_slice(marker_line.as_bytes());
    }
    parser.advance(term, &bytes);
    log::debug!(
        "kitty allocated {}x{} at ({},{}) ({:.2}ms)",
        cols,
        rows,
        col,
        row,
        t0.elapsed().as_secs_f64() * 1000.0
    );
}

/// Drop kitty placements whose covered cells were overwritten since they were
/// placed (text output, a screen clear, or scrolling). Every placement is
/// anchored by the `KITTY_MARKER` cells written over its box, so it dies the
/// moment any covered cell is replaced — this is what stops stale images from
/// following the user into other programs. Returns true if anything was removed.
fn reconcile_kitty(kitty: &Mutex<KittyStore>, term: &Term<PaneProxy>) -> bool {
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point};

    let columns = term.columns();
    let lines = term.screen_lines();
    let mut store = kitty.lock().unwrap_or_else(|e| e.into_inner());

    // Precompute the on-screen box for each placement up front so the closure
    // below never borrows the store mutably.
    struct Check {
        col0: usize,
        row0: isize,
        col_end: usize,
        row_end: isize,
    }
    let checks: Vec<Option<Check>> = {
        let images = &store.images;
        store
            .placements
            .iter()
            .map(|p| {
                let img = images.get(&p.image_id)?;
                let (cols, rows) = placement_box(p, img);
                let col0 = p.col;
                let row0 = p.row.max(0);
                Some(Check {
                    col0,
                    row0,
                    // Only the part of the box that is on screen can be
                    // overwritten (or inspected); out-of-grid rows/cols are
                    // ignored.
                    col_end: col0.saturating_add(cols).min(columns),
                    row_end: row0.saturating_add(rows as isize).min(lines as isize),
                })
            })
            .collect()
    };

    let before = store.placements.len();
    let mut i = 0;
    store.placements.retain(|_| {
        let check = checks[i].as_ref();
        i += 1;
        let Some(check) = check else { return false };
        if check.col_end <= check.col0 || check.row_end <= check.row0 {
            return true;
        }
        for r in check.row0..check.row_end {
            for c in check.col0..check.col_end {
                let ch = term.grid()[Point { column: Column(c), line: Line(r as i32) }].c;
                if ch != KITTY_MARKER {
                    return false;
                }
            }
        }
        true
    });

    before != store.placements.len()
}

/// Like [`allocate_kitty_cells`], but for anchor (`a=p`) placements: mark the
/// covered box with `KITTY_MARKER` cells and then restore the cursor to where
/// it was, since an explicit placement does not move the cursor.
fn mark_kitty_cells(
    parser: &mut Processor,
    term: &mut Term<PaneProxy>,
    col: usize,
    row: isize,
    cols: usize,
    rows: usize,
) {
    let point = term.grid().cursor.point;
    allocate_kitty_cells(parser, term, col, row, cols, rows);
    let restore = format!("\x1b[{};{}H", point.line.0 + 1, point.column.0 + 1);
    parser.advance(term, restore.as_bytes());
}

/// The I/O loop: poll the PTY, parse bytes, service the command channel.
fn run_io_thread(
    mut pty: tty::Pty,
    term: Arc<FairMutex<Term<PaneProxy>>>,
    kitty: Arc<Mutex<KittyStore>>,
    proxy: PaneProxy,
    rx: Receiver<PtyMsg>,
) {
    let fd = pty.file().as_raw_fd();
    set_nonblocking(fd);

    let mut parser: Processor = Processor::new();
    let mut scanner = ApcScanner::default();
    let mut buf = [0u8; 65536];
    let mut apc_buf: Vec<(usize, Vec<u8>)> = Vec::new();

    let wake = |proxy: &PaneProxy| {
        proxy.send_event(Event::Wakeup);
    };

    loop {
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` points to valid stack memory; fd is valid.
        let n = unsafe { libc::poll(&mut pollfd, 1, 20) };

        let mut dirty = false;
        let mut exited = false;

        if n < 0 {
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                log::error!("poll(pty) failed");
                break;
            }
        } else if n > 0 && pollfd.revents & libc::POLLIN != 0 {
            loop {
                match pty.file().read(&mut buf) {
                    Ok(0) => {
                        // All slave fds closed: the child exited.
                        exited = true;
                        break;
                    },
                    Ok(got) => {
                        dirty = true;
                        apc_buf.clear();
                        scanner.scan(&buf[..got], &mut apc_buf);

                        // Capture the grid cursor at the moment each APC arrives and
                        // handle each APC inline: a preceding `CSI CUP` in the same
                        // chunk is honored, and a `Place` can allocate its covered
                        // cells before the bytes that follow it in this chunk parse.
                        let mut actions: Vec<KittyAction> = Vec::with_capacity(apc_buf.len());
                        {
                            let mut guard = term.lock();
                            let mut prev = 0;
                            for (offset, payload) in &apc_buf {
                                parser.advance(&mut *guard, &buf[prev..*offset]);
                                prev = *offset;
                                let point = guard.grid().cursor.point;
                                let cursor = (point.column.0, point.line.0 as isize);
                                let action = kitty
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .handle(payload, cursor);
                                log::debug!(
                                    "kitty APC cursor=({},{}) payload={} -> {:?}",
                                    cursor.0,
                                    cursor.1,
                                    String::from_utf8_lossy(&payload[..payload.len().min(60)])
                                        .replace('\x1b', "\\e"),
                                    action
                                );
                                if let KittyAction::Place { col, row, cols, rows } = action {
                                    log::debug!(
                                        "kitty place {}x{} at ({},{}) allocating cells",
                                        cols,
                                        rows,
                                        col,
                                        row
                                    );
                                    allocate_kitty_cells(&mut parser, &mut guard, col, row, cols, rows);
                                }
                                if let KittyAction::MarkCells { col, row, cols, rows } = action {
                                    log::debug!(
                                        "kitty a=p mark {}x{} at ({},{})",
                                        cols,
                                        rows,
                                        col,
                                        row
                                    );
                                    mark_kitty_cells(&mut parser, &mut guard, col, row, cols, rows);
                                }
                                actions.push(action);
                            }
                            parser.advance(&mut *guard, &buf[prev..got]);
                        }

                        // Any bytes in this chunk may have overwritten image
                        // cells; drop placements whose box is no longer intact.
                        {
                            let guard = term.lock();
                            if reconcile_kitty(&kitty, &guard) {
                                log::debug!("kitty reconcile: dropped stale placement(s)");
                            }
                        }

                        for action in actions {
                            match action {
                                KittyAction::Respond => {
                                    log::debug!("kitty respond OK");
                                    let _ = pty.writer().write_all(b"\x1b_Gi=1;OK\x1b\\");
                                },
                                KittyAction::Changed | KittyAction::Place { .. } | KittyAction::MarkCells { .. } => dirty = true,
                                KittyAction::None => {},
                            }
                        }
                    },
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) => {
                        log::error!("pty read failed: {err}");
                        exited = true;
                        break;
                    },
                }
            }
        }

        // Service commands from the UI thread.
        let mut shutdown = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                PtyMsg::Write(bytes) => {
                    if let Err(err) = pty.writer().write_all(&bytes) {
                        log::error!("pty write failed: {err}");
                    }
                },
                PtyMsg::Resize(size) => resize_pty(fd, size),
                PtyMsg::Shutdown => {
                    shutdown = true;
                    break;
                },
            }
        }

        if dirty {
            wake(&proxy);
        }
        if exited {
            proxy.send_event(Event::Exit);
            break;
        }
        if shutdown {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use alacritty_terminal::event::WindowSize;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use crate::event::{PaneEvent, PaneEventKind, PaneProxy};
    use crate::kitty::{KittyAction, KittyStore};

    use super::{allocate_kitty_cells, mark_kitty_cells, reconcile_kitty, ApcScanner};

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

    #[test]
    fn fastfetch_kitty_direct_advances_cursor() {
        // Mirror fastfetch's `--logo-type kitty-direct` byte stream:
        // clear, position the cursor, transmit+display a file image (`a=T, t=f, c=40`),
        // then query the cursor position with `\e[6n`.
        let dir = std::env::temp_dir().join("optix-kitty-e2e");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("logo.png");
        let mut img = image::RgbaImage::new(200, 145); // aspect ratio like a fastfetch logo
        for px in img.pixels_mut() {
            *px = image::Rgba([30, 120, 200, 255]);
        }
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        std::fs::write(&path, &buf).unwrap();
        use base64::Engine as _;
        let b64_path = base64::engine::general_purpose::STANDARD.encode(path.to_str().unwrap());
        let stream = format!("\x1b[2J\x1b[3J\x1b[3;3H\x1b_Ga=T,f=100,t=f,c=40;{b64_path}\x1b\\\x1b[6n");

        let (tx, rx) = mpsc::channel::<PaneEvent>();
        let term_config = TermConfig { kitty_keyboard: false, ..TermConfig::default() };
        let dims = PaneSize { columns: 95, screen_lines: 46 };
        let mut term = Term::new(term_config, &dims, PaneProxy { pane_id: 0, tx, el_wakeup: None });
        let mut parser = Processor::<StdSyncHandler>::new();
        let mut kitty = KittyStore::new();

        // Replicate the run_io_thread scan+handle block for one APC.
        let mut scanner = ApcScanner::default();
        let mut apc_buf = Vec::new();
        scanner.scan(stream.as_bytes(), &mut apc_buf);
        assert_eq!(apc_buf.len(), 1);
        let (offset, payload) = &apc_buf[0];

        parser.advance(&mut term, &stream.as_bytes()[..*offset]);
        let point = term.grid().cursor.point;
        let cursor = (point.column.0, point.line.0 as isize);
        let action = kitty.handle(payload, cursor);
        assert!(matches!(action, KittyAction::Place { col: 2, row: 2, cols: 40, rows }));
        if let KittyAction::Place { col, row, cols, rows } = action {
            allocate_kitty_cells(&mut parser, &mut term, col, row, cols, rows);
        }
        parser.advance(&mut term, &stream.as_bytes()[*offset..]);

        // The cursor should now sit just past the image's bottom-right cell:
        // col 2 + 40 = 42 (0-based).
        assert_eq!(term.grid().cursor.point.column.0, 42);

        // The `\e[6n` response must report that column (1-based -> 43) and the
        // bottom row of the image (row 2 + 15 rows - 1 = 16 -> 17).
        let response = rx.try_recv().unwrap();
        match response.kind {
            PaneEventKind::PtyWrite(text) => {
                assert_eq!(text, "\x1b[17;43R");
            },
            _ => panic!("expected PtyWrite response to \\e[6n"),
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn tiny_png() -> Vec<u8> {
        let mut img = image::RgbaImage::new(4, 3);
        for px in img.pixels_mut() {
            *px = image::Rgba([200, 50, 90, 255]);
        }
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn reconcile_drops_stale_placements() {
        use base64::Engine as _;

        let (tx, _rx) = mpsc::channel::<PaneEvent>();
        let term_config = TermConfig { kitty_keyboard: false, ..TermConfig::default() };
        let dims = PaneSize { columns: 80, screen_lines: 24 };
        let mut term = Term::new(term_config, &dims, PaneProxy { pane_id: 0, tx, el_wakeup: None });
        let mut parser = Processor::<StdSyncHandler>::new();
        let kitty = std::sync::Mutex::new(KittyStore::new());

        let b64 = base64::engine::general_purpose::STANDARD.encode(tiny_png());
        let payload = format!("Ga=T,f=100,i=1,c=10,r=3,m=0;{b64}");
        let action = kitty
            .lock()
            .unwrap()
            .handle(payload.as_bytes(), (5, 5));
        let (col, row, cols, rows) = match action {
            KittyAction::Place { col, row, cols, rows } => (col, row, cols, rows),
            _ => panic!("expected Place"),
        };
        allocate_kitty_cells(&mut parser, &mut term, col, row, cols, rows);
        assert_eq!(kitty.lock().unwrap().placements.len(), 1);
        assert!(!reconcile_kitty(&kitty, &term), "intact placement must survive reconcile");
        assert_eq!(kitty.lock().unwrap().placements.len(), 1);

        // Text written over the box erases the placement.
        parser.advance(&mut term, b"\x1b[7;6Hoverwritten");
        assert!(reconcile_kitty(&kitty, &term), "overwritten placement must be dropped");
        assert!(kitty.lock().unwrap().placements.is_empty());

        // Re-place, then a full-screen clear erases it too.
        let action = kitty
            .lock()
            .unwrap()
            .handle(payload.as_bytes(), (5, 5));
        let (col, row, cols, rows) = match action {
            KittyAction::Place { col, row, cols, rows } => (col, row, cols, rows),
            _ => panic!("expected Place"),
        };
        allocate_kitty_cells(&mut parser, &mut term, col, row, cols, rows);
        assert_eq!(kitty.lock().unwrap().placements.len(), 1);
        parser.advance(&mut term, b"\x1b[2J");
        assert!(reconcile_kitty(&kitty, &term), "cleared placement must be dropped");
        assert!(kitty.lock().unwrap().placements.is_empty());

        // Scrolling the content away erases it too.
        let action = kitty
            .lock()
            .unwrap()
            .handle(payload.as_bytes(), (5, 5));
        let (col, row, cols, rows) = match action {
            KittyAction::Place { col, row, cols, rows } => (col, row, cols, rows),
            _ => panic!("expected Place"),
        };
        allocate_kitty_cells(&mut parser, &mut term, col, row, cols, rows);
        assert_eq!(kitty.lock().unwrap().placements.len(), 1);
        // Jump to the bottom line and emit LF to force a scroll-up by one.
        parser.advance(&mut term, b"\x1b[24;1H\n");
        assert!(reconcile_kitty(&kitty, &term), "scrolled placement must be dropped");
        assert!(kitty.lock().unwrap().placements.is_empty());

        // Anchor (`a=p`) placements are marked too, so they die the same way.
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(tiny_png());
        let mut store = kitty.lock().unwrap();
        store.handle(format!("a=t,f=100,i=2,c=4,r=2,m=0;{b64}").as_bytes(), (0, 0));
        let action = store.handle(b"a=p,i=2,m=0", (10, 10));
        drop(store);
        let (col, row, cols, rows) = match action {
            KittyAction::MarkCells { col, row, cols, rows } => (col, row, cols, rows),
            _ => panic!("expected MarkCells"),
        };
        mark_kitty_cells(&mut parser, &mut term, col, row, cols, rows);
        assert_eq!(kitty.lock().unwrap().placements.len(), 1);
        assert!(!reconcile_kitty(&kitty, &term), "intact a=p placement must survive");
        parser.advance(&mut term, b"\x1b[2J");
        assert!(reconcile_kitty(&kitty, &term), "cleared a=p placement must be dropped");
        assert!(kitty.lock().unwrap().placements.is_empty());
    }

    #[test]
    fn scanner_finds_apc_split_across_chunks() {
        let mut s = ApcScanner::default();
        let mut out = Vec::new();

        // Cursor move, then the start of an APC mid-chunk.
        s.scan(b"\x1b[3;1H\x1b_Ga=T;abc", &mut out);
        assert!(out.is_empty());

        // APC body continues, terminates, then ordinary bytes follow.
        s.scan(b"def\x1b\\rest", &mut out);
        assert_eq!(out.len(), 1);
        let (offset, payload) = &out[0];
        assert_eq!(&payload[..], b"Ga=T;abcdef");
        // `\\` is index 4; offset points just past it.
        assert_eq!(*offset, 5);
    }

    #[test]
    fn scanner_ignores_ordinary_esc_sequences() {
        let mut s = ApcScanner::default();
        let mut out = Vec::new();
        s.scan(b"\x1b[2J\x1b[31mhi \x1b[0m", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn scanner_handles_terminator_across_boundary() {
        let mut s = ApcScanner::default();
        let mut out = Vec::new();
        s.scan(b"pre\x1b_abc\x1b", &mut out);
        assert!(out.is_empty());
        s.scan(b"\\post", &mut out);
        assert_eq!(out.len(), 1);
        let (offset, payload) = &out[0];
        assert_eq!(&payload[..], b"abc");
        assert_eq!(*offset, 1);
    }
}
