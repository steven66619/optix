use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cosmic_text::{fontdb, Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping};

/// Glyph placed in a terminal line, with coordinates relative to the line's top-left
/// in the pane's client area (already offset by the row's baseline).
#[derive(Debug, Clone, Copy)]
pub struct GlyphDraw {
    pub cache_key: CacheKey,
    pub x: f32,
    pub y: f32,
}

/// Font handling: system font discovery, metrics, and per-line shaping.
pub struct Fonts {
    pub font_system: FontSystem,
    buffer: Buffer,
    pub metrics: Metrics,
    pub font_size_px: f32,
    /// Advance width of one monospace cell.
    pub cell_w: f32,
    /// Height of one terminal row.
    pub cell_h: f32,
    /// Y offset (in a row) of the text baseline.
    pub baseline: f32,
    pub family: String,
    /// Shaped glyphs for a single terminal cell, keyed by (char, bold, italic).
    glyph_cache: HashMap<(char, bool, bool), Vec<GlyphDraw>>,
}

impl Fonts {
    pub fn new(family: &str, font_size: f32, dpi_scale: f32) -> Result<Self, String> {
        let db = load_minimal_font_database(family);
        let mut font_system = FontSystem::new_with_locale_and_db(current_locale(), db);

        // Resolve the requested family (fall back to any monospace).
        let resolved = {
            let db = font_system.db();
            let found = db.query(&cosmic_text::fontdb::Query {
                families: &[cosmic_text::fontdb::Family::Name(family)],
                ..Default::default()
            });
            if found.is_some() {
                family.to_string()
            } else {
                log::warn!("Font family `{family}` not found; falling back to system monospace");
                "monospace".to_string()
            }
        };

        // Font size given in points -> pixels.
        let font_size_px = font_size * (96.0 / 72.0) * dpi_scale;

        let mut buffer = Buffer::new(&mut font_system, Metrics::new(font_size_px, font_size_px * 1.35));
        buffer.set_size(None, None);

        let mut fonts = Self {
            font_system,
            buffer,
            metrics: Metrics::new(font_size_px, font_size_px * 1.35),
            font_size_px,
            cell_w: 0.0,
            cell_h: 0.0,
            baseline: 0.0,
            family: resolved,
            glyph_cache: HashMap::new(),
        };
        fonts.remeasure();
        Ok(fonts)
    }

    /// Recompute metrics for a new font size (in points, unscaled by dpi).
    pub fn set_font_size(&mut self, font_size: f32, dpi_scale: f32) {
        self.font_size_px = font_size * (96.0 / 72.0) * dpi_scale;
        self.glyph_cache.clear();
        self.remeasure();
    }

    fn remeasure(&mut self) {
        let sample = "WWWWWWWWWWWWWWWW";
        self.buffer.set_text(
            sample,
            &Attrs::new().family(Family::Name(&self.family)),
            Shaping::Advanced,
            None,
        );
        self.buffer.set_size(None, None);
        self.buffer.shape_until_scroll(&mut self.font_system, true);

        let mut run_opt = None;
        if let Some(run) = self.buffer.layout_runs().next() {
            run_opt = Some((run.line_w, run.line_height, run.line_y));
        }
        let (line_w, line_height, line_y) = run_opt.unwrap_or((0.0, self.font_size_px, 0.0));
        let cell_w = (line_w / sample.len() as f32 * 2.0).round() / 2.0;
        let cell_h = line_height.ceil().max(1.0);

        self.cell_w = cell_w;
        self.cell_h = cell_h;
        self.metrics = Metrics::new(self.font_size_px, cell_h);
        self.buffer.set_metrics(self.metrics);
        self.buffer.shape_until_scroll(&mut self.font_system, true);
        self.baseline = self
            .buffer
            .layout_runs()
            .next()
            .map(|run| run.line_y)
            .unwrap_or(line_y);
    }

    /// Layout one terminal cell; returns glyphs positioned relative to the cell's
    /// top-left corner. Shaping is done once per distinct (char, bold, italic).
    pub fn layout_cell(&mut self, c: char, bold: bool, italic: bool) -> &[GlyphDraw] {
        let key = (c, bold, italic);
        if !self.glyph_cache.contains_key(&key) {
            let glyphs = self.shape_cell(c, bold, italic);
            self.glyph_cache.insert(key, glyphs);
        }
        &self.glyph_cache[&key]
    }

    fn shape_cell(&mut self, c: char, bold: bool, italic: bool) -> Vec<GlyphDraw> {
        let mut attrs = Attrs::new().family(Family::Name(&self.family));
        if bold {
            attrs = attrs.weight(cosmic_text::fontdb::Weight::BOLD);
        }
        if italic {
            attrs = attrs.style(cosmic_text::fontdb::Style::Italic);
        }
        self.buffer.set_text(&c.to_string(), &attrs, Shaping::Advanced, None);
        self.buffer.set_size(None, None);
        self.buffer.shape_until_scroll(&mut self.font_system, true);

        let baseline = self.baseline;
        let mut out = Vec::new();
        for run in self.buffer.layout_runs() {
            for glyph in run.glyphs {
                let phys = glyph.physical((0.0, 0.0), 1.0);
                out.push(GlyphDraw {
                    cache_key: phys.cache_key,
                    x: phys.x as f32,
                    y: baseline + phys.y as f32,
                });
            }
        }
        out
    }

    /// Layout one line of text; returns glyphs positioned relative to the line's
    /// top-left corner (glyph.y is the offset from the row's baseline).
    pub fn layout_line(&mut self, text: &str, bold: bool, italic: bool) -> Vec<GlyphDraw> {
        let mut attrs = Attrs::new().family(Family::Name(&self.family));
        if bold {
            attrs = attrs.weight(cosmic_text::fontdb::Weight::BOLD);
        }
        if italic {
            attrs = attrs.style(cosmic_text::fontdb::Style::Italic);
        }
        self.buffer.set_text(text, &attrs, Shaping::Advanced, None);
        self.buffer.set_size(None, None);
        self.buffer.shape_until_scroll(&mut self.font_system, true);

        let baseline = self.baseline;
        let mut out = Vec::new();
        for run in self.buffer.layout_runs() {
            for glyph in run.glyphs {
                let phys = glyph.physical((0.0, 0.0), 1.0);
                out.push(GlyphDraw {
                    cache_key: phys.cache_key,
                    x: phys.x as f32,
                    y: baseline + phys.y as f32,
                });
            }
        }
        out
    }

    /// Measure the width of a piece of text in pixels.
    pub fn measure(&mut self, text: &str) -> f32 {
        self.buffer.set_text(text, &Attrs::new().family(Family::Name(&self.family)), Shaping::Advanced, None);
        self.buffer.set_size(None, None);
        self.buffer.shape_until_scroll(&mut self.font_system, true);
        self.buffer.layout_runs().next().map(|run| run.line_w).unwrap_or(0.0)
    }

    /// Layout a multi-line string into glyphs with an explicit max width; used for
    /// the tab bar and search overlay. `wrap: true` wraps at `width`.
    pub fn layout_paragraph(
        &mut self,
        text: &str,
        width: Option<f32>,
        bold: bool,
        baseline_offset: f32,
    ) -> Vec<GlyphDraw> {
        let mut attrs = Attrs::new().family(Family::Name(&self.family));
        if bold {
            attrs = attrs.weight(cosmic_text::fontdb::Weight::BOLD);
        }
        self.buffer
            .set_text(text, &attrs, Shaping::Advanced, None);
        self.buffer.set_size(width, None);
        self.buffer.shape_until_scroll(&mut self.font_system, true);

        let mut out = Vec::new();
        for run in self.buffer.layout_runs() {
            for glyph in run.glyphs {
                let phys = glyph.physical((0.0, 0.0), 1.0);
                out.push(GlyphDraw {
                    cache_key: phys.cache_key,
                    x: phys.x as f32,
                    y: baseline_offset + run.line_y + phys.y as f32,
                });
            }
        }
        out
    }
}

/// Standard directories where Linux distributions install fonts.
fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".fonts"));
        dirs.push(PathBuf::from(&home).join(".local/share/fonts"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg).join("fonts"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_DIRS") {
        for dir in xdg.split(':') {
            if !dir.is_empty() {
                dirs.push(PathBuf::from(dir).join("fonts"));
            }
        }
    }
    dirs
}

/// Recursively collect font files under `dir`.
fn collect_font_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_files(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(
                ext.to_lowercase().as_str(),
                "ttf" | "ttc" | "otf" | "otc"
            ) {
                out.push(path);
            }
        }
    }
}

/// Best-effort locale from the environment (fontconfig-style `en_US.UTF-8`).
fn current_locale() -> String {
    for var in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            let lang = val.split('.').next().unwrap_or(&val).replace('_', "-");
            if !lang.is_empty() {
                return lang;
            }
        }
    }
    "en-US".to_string()
}

/// Build a font database with only the requested family plus a few well-known
/// fallbacks that cover box-drawing/braille (TUI glyphs), CJK and emoji. Loading
/// the whole system font set can take seconds, so this keeps startup fast while
/// still resolving the configured family. If the family can't be found, falls
/// back to a full system scan so rendering is never broken.
fn load_minimal_font_database(family: &str) -> fontdb::Database {
    let mut files = Vec::new();
    for dir in system_font_dirs() {
        collect_font_files(&dir, &mut files);
    }

    // Keywords for the configured family: every significant word plus the name
    // with all separators stripped.
    let mut family_keys: Vec<String> = Vec::new();
    let family_norm: String = family
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    for token in family.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        if token.len() >= 3 {
            family_keys.push(token.to_string());
        }
    }
    family_keys.push(family_norm);

    // (keyword, exclude-substring) fallback sets.
    let fallbacks: &[(&str, &str)] = &[
        ("dejavusansmono", ""), // box drawing, braille, block elements
        ("dejavusans", ""), // symbols, box drawing, braille
        ("adwaitamono", ""), // default GNOME mono: braille, boxes, symbols
        ("notosansmono", "cjk"), // Latin/Cyrillic/Greek monospace
        ("notosanscjk", ""), // CJK variable font
        ("notosanssymbols2", ""), // braille, geometric shapes
        ("notosanssymbols", ""), // arrows and symbols
        ("notosansmath", ""), // math symbols
        ("colrv1", ""), // Noto Color Emoji
        ("liberationmono", ""), // metric-compatible generic mono
        ("opensymbol", ""), // misc symbols
    ];

    let mut db = fontdb::Database::new();
    for file in &files {
        let lower = file.to_string_lossy().to_lowercase();

        // Fallback fonts cover glyphs the primary family lacks.
        let fallback = fallbacks.iter().any(|(key, exclude)| {
            lower.contains(key) && (exclude.is_empty() || !lower.contains(exclude))
        });

        // Family files, skipping Nerd Font variant families (Mono/Propo/NL)
        // that only share the base name with the requested family.
        let family_match = family_keys.iter().any(|k| lower.contains(k.as_str()))
            && !(lower.contains("nerdfontmono")
                || lower.contains("nerdfontpropo")
                || lower.contains("nlnerdfont"));

        if fallback || family_match {
            let _ = db.load_font_file(file);
        }
    }

    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        ..Default::default()
    };
    if db.query(&query).is_some() {
        // Keep the generic monospace family resolvable for cosmic-text's
        // internal fallback, mirroring what FontSystem::new() sets up.
        if db
            .query(&fontdb::Query {
                families: &[fontdb::Family::Monospace],
                ..Default::default()
            })
            .is_none()
        {
            db.set_monospace_family(family);
        }
    } else {
        log::warn!("font `{family}` not found in minimal font set; loading all system fonts");
        let mut full = fontdb::Database::new();
        full.load_system_fonts();
        db = full;
    }

    log::debug!("loaded {} font faces for `{family}`", db.len());
    db
}
