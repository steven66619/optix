//! Headless reproduction of the opencode crash: feeds captured opencode
//! output through the real alacritty parser, then runs the same CPU-side
//! rendering logic as `render_pane` (color resolution + glyph layout).

use std::sync::mpsc;
use std::sync::Arc;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::{EventLoop, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

use crate::color::Rgba;
use crate::config::Config;
use crate::event::{PaneEvent, PaneProxy};
use crate::fonts::Fonts;
use crate::palette::Palette;
use crate::terminal::is_visible;

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

fn setup() -> (Arc<FairMutex<Term<PaneProxy>>>, alacritty_terminal::event_loop::EventLoopSender) {
    let (tx, _rx) = mpsc::channel::<PaneEvent>();
    let proxy_a = PaneProxy { pane_id: 0, tx: tx.clone() };
    let proxy_b = PaneProxy { pane_id: 0, tx };
    let term_config = TermConfig { kitty_keyboard: false, ..TermConfig::default() };
    let dims = PaneSize { columns: 95, screen_lines: 46 };
    let term = Arc::new(FairMutex::new(Term::new(term_config, &dims, proxy_a)));
    let pty = tty::new(
        &tty::Options {
            shell: None,
            working_directory: None,
            drain_on_exit: false,
            env: std::collections::HashMap::new(),
        },
        WindowSize { num_lines: 46, num_cols: 95, cell_width: 10, cell_height: 22 },
        0,
    )
    .expect("pty");
    let el = EventLoop::new(term.clone(), proxy_b, pty, false, false).expect("event loop");
    let sender = el.channel();
    el.spawn();
    (term, sender)
}

fn feed(term: &mut Term<PaneProxy>, bytes: &[u8]) {
    let mut parser = Processor::<StdSyncHandler>::new();
    parser.advance(term, bytes);
}

fn sample_render(term: &mut Term<PaneProxy>, fonts: &mut Fonts, palette: &Palette) {
    let content = term.renderable_content();
    let display_offset = content.display_offset as i32;
    let cursor = content.cursor;
    let dynamic = content.colors;
    let lines = term.screen_lines() as i32;
    let cols = term.columns() as i32;
    let mut glyphs = 0usize;
    for indexed in content.display_iter {
        let cell = indexed.cell;
        if !is_visible(cell) {
            continue;
        }
        let row = indexed.point.line.0 + display_offset;
        let col = indexed.point.column.0 as i32;
        if row < 0 || row >= lines || col >= cols {
            continue;
        }
        let mut fg = cell.fg;
        let mut bg = cell.bg;
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let cell_fg = if cell.flags.contains(Flags::DIM) {
            if let alacritty_terminal::vte::ansi::Color::Named(n) = fg {
                let idx = n as usize;
                if idx < 8 {
                    if let Some(rgb) = dynamic[259 + idx] {
                        Rgba::from_u8(rgb.r, rgb.g, rgb.b, 255)
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
        let _bg_rgba = palette.resolve(&bg, dynamic);
        if cell.c != ' ' {
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            let g = fonts.layout_line(&cell.c.to_string(), bold, italic);
            glyphs += g.len();
        }
    }
    if cursor.shape != alacritty_terminal::vte::ansi::CursorShape::Hidden {
        let row = cursor.point.line.0 + display_offset;
        let col = cursor.point.column.0 as i32;
        if row >= 0 && row < lines && col >= 0 && col < cols {
            let ch = term.grid()[cursor.point.line][cursor.point.column].c;
            let _ = ch;
        }
    }
    eprintln!("sampled: {glyphs} glyphs laid out");
}

#[test]
fn minimal_fonts_cover_tui_glyphs() {
    let cfg = Config::load();
    let mut fonts = Fonts::new(&cfg.font.family, cfg.font.size, 1.0).expect("fonts");
    let glyphs = fonts.layout_line("· ┃ ╹ ▀ ■ ▣ ⬝ \u{2800}\u{2801}\u{28FF} box ⛰", false, false);
    assert!(!glyphs.is_empty(), "minimal font set must shape TUI glyphs");
    let bad = glyphs
        .iter()
        .filter(|g| g.cache_key.glyph_id == 0)
        .count();
    if bad > 0 {
        let mut missing = String::new();
        let chars: Vec<char> = "· ┃ ╹ ▀ ■ ▣ ⬝ \u{2800}\u{2801}\u{28FF} box ⛰".chars().collect();
        for (i, g) in glyphs.iter().enumerate() {
            if g.cache_key.glyph_id == 0 && i < chars.len() {
                missing.push(chars[i]);
            }
        }
        eprintln!("missing glyphs in minimal set: {:?}", missing);
    }
    assert_eq!(bad, 0, "{bad} glyphs were not found in the minimal font set");
}

#[test]
fn startup_timing() {
    let t0 = std::time::Instant::now();
    let cfg = Config::load();
    let t_cfg = t0.elapsed();
    let t1 = std::time::Instant::now();
    let fonts = Fonts::new(&cfg.font.family, cfg.font.size, 1.0).expect("fonts");
    let t_fonts = t1.elapsed();
    let t2 = std::time::Instant::now();
    let palette = Palette::from_theme(&cfg.theme);
    let t_palette = t2.elapsed();
    let t3 = std::time::Instant::now();
    let (term_arc, sender) = setup();
    let t_term = t3.elapsed();
    eprintln!(
        "config={:?} fonts={:?} palette={:?} term+pty={:?} total={:?} font_cell={:.2}x{:.2} faces={}",
        t_cfg,
        t_fonts,
        t_palette,
        t_term,
        t0.elapsed(),
        fonts.cell_w,
        fonts.cell_h,
        fonts.font_system.db().len()
    );
    let _ = (term_arc, sender, palette);
}

#[test]
fn opencode_stream_renders_without_panic() {
    let bytes = std::fs::read("/tmp/opencode/oc2.bin").expect("captured opencode output");

    let mut cfg = Config::default_config();
    cfg.font.size = 12.0;
    let palette = Palette::from_theme(&cfg.theme);
    let mut fonts = Fonts::new(&cfg.font.family, cfg.font.size, 1.0).expect("fonts");

    let (term_arc, sender) = setup();

    // Feed the data in chunks as a PTY would.
    for chunk in bytes.chunks(512) {
        let mut guard = term_arc.lock();
        feed(&mut guard, chunk);
    }

    let mut guard = term_arc.lock();
    for _ in 0..2 {
        sample_render(&mut guard, &mut fonts, &palette);
    }
    let _ = sender;
}
