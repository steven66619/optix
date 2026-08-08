use cosmic_text::{Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping};

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
}

impl Fonts {
    pub fn new(family: &str, font_size: f32, dpi_scale: f32) -> Result<Self, String> {
        let mut font_system = FontSystem::new();

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
        };
        fonts.remeasure();
        Ok(fonts)
    }

    /// Recompute metrics for a new font size (in points, unscaled by dpi).
    pub fn set_font_size(&mut self, font_size: f32, dpi_scale: f32) {
        self.font_size_px = font_size * (96.0 / 72.0) * dpi_scale;
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
