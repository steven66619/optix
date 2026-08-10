//! Minimal implementation of the Kitty graphics protocol.
//!
//! Applications (notably fastfetch with `--logo-type kitty-direct`) transmit
//! images to the terminal using APC escape sequences (`ESC _` ... `ESC \`).
//! The stock `vte` parser safely ignores APC payloads, so the PTY reader
//! scans the byte stream here and turns valid kitty sequences into image
//! placements that the renderer draws as textured quads.

use std::collections::HashMap;
use std::path::PathBuf;

/// Invisible sentinel char written into the grid cells an image placement
/// covers. A placement is only drawn while every cell of its box still holds
/// this marker; as soon as a cell is overwritten (text, a screen clear, or a
/// scroll) the placement is dropped so stale images don't linger on screen
/// when a new program redraws the pane.
pub const KITTY_MARKER: char = '\u{E000}';

/// Decoded RGBA image ready to be uploaded as a texture.
pub struct KittyImage {
    pub width: u32,
    pub height: u32,
    /// Straight RGBA8, `width * height * 4` bytes, row-major.
    pub rgba: Vec<u8>,
    /// Cell size given at transmit time (used by later placements that omit it).
    pub cells_w: Option<f32>,
    pub cells_h: Option<f32>,
    /// Bumped whenever the image for this id is replaced so stale GPU
    /// textures can be invalidated.
    pub gen: u64,
}

/// A request to draw an image anchored to a grid cell.
pub struct Placement {
    pub image_id: u64,
    pub gen: u64,
    pub col: usize,
    pub row: isize,
    /// Width/height in cells. At least one is `Some` for sized placements.
    pub cells_w: Option<f32>,
    pub cells_h: Option<f32>,
    /// Pixel size, used by explicit `w`/`h` placements.
    pub px_w: Option<f32>,
    pub px_h: Option<f32>,
    pub z: i32,
}

/// An in-progress chunked transmission (`m=1` ... `m=0`).
struct Pending {
    image_id: u64,
    format: u32,
    zlib: bool,
    /// `t=f`: the payload is the base64 of a file path, not image data.
    file: bool,
    size: Option<(u32, u32)>,
    /// Raw base64 payload accumulated across chunks.
    data: Vec<u8>,
    cells_w: Option<f32>,
    cells_h: Option<f32>,
    px_w: Option<f32>,
    px_h: Option<f32>,
    z: i32,
}

/// Outcome of processing one APC payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyAction {
    /// Nothing interesting happened.
    None,
    /// A capability query (`a=q`) was received; the caller should answer.
    Respond,
    /// The store changed and the UI should redraw.
    Changed,
    /// A transmit+display (`a=T`) image was placed at `(col, row)` covering
    /// `cols x rows` cells. The caller should allocate those cells and advance
    /// the cursor so subsequent text flows around the image.
    Place { col: usize, row: isize, cols: usize, rows: usize },
    /// An anchor (`a=p`) placement covers `cols x rows` cells at `(col, row)`.
    /// The caller should mark those cells without moving the cursor.
    MarkCells { col: usize, row: isize, cols: usize, rows: usize },
}

/// Per-pane store of decoded images and active placements.
#[derive(Default)]
pub struct KittyStore {
    pub images: HashMap<u64, KittyImage>,
    pub placements: Vec<Placement>,
    pending: Option<Pending>,
    gen_counter: u64,
}

/// Parse the `key=value,key=value` control data. Values may themselves
/// contain commas (e.g. `s=w,h`), which are rejoined into the previous key.
fn parse_control(control: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut last_key: Option<String> = None;
    for token in control.split(',') {
        if let Some((k, v)) = token.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
            last_key = Some(k.trim().to_string());
        } else if let Some(key) = last_key.as_ref() {
            if let Some(v) = out.get_mut(key) {
                v.push(',');
                v.push_str(token);
            }
        }
    }
    out
}

fn num<T: std::str::FromStr>(kv: &HashMap<String, String>, key: &str) -> Option<T> {
    kv.get(key).and_then(|s| s.parse().ok())
}

impl KittyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one APC payload (`ESC _` <payload> `ESC \`).
    ///
    /// `cursor` is the grid cell `(column, line)` where the image should be
    /// anchored for cursor placements. `cell_w`/`cell_h` are the current cell
    /// dimensions, used to size cell-based placements.
    pub fn handle(&mut self, payload: &[u8], cursor: (usize, isize)) -> KittyAction {
        // The kitty graphics protocol prefixes the payload with a `G` marker.
        let payload = payload.strip_prefix(b"G").unwrap_or(payload);
        let (control, data) = match payload.iter().position(|&b| b == b';') {
            Some(idx) => (&payload[..idx], &payload[idx + 1..]),
            None => (payload, &payload[..0]),
        };
        let control = String::from_utf8_lossy(control);
        let kv = parse_control(&control);

        let action = kv.get("a").map(String::as_str).unwrap_or("t");
        let more = kv.get("m").map(String::as_str) == Some("1");

        match action {
            "t" | "T" => {
                if self.pending.is_none() {
                    let size = parse_size(&kv);
                    self.pending = Some(Pending {
                        image_id: num(&kv, "i").unwrap_or(0),
                        format: num(&kv, "f").unwrap_or(100),
                        zlib: kv.get("o").map(String::as_str) == Some("z"),
                        file: kv.get("t").map(String::as_str) == Some("f"),
                        size,
                        data: Vec::new(),
                        cells_w: num(&kv, "c"),
                        cells_h: num(&kv, "r"),
                        px_w: num(&kv, "w"),
                        px_h: num(&kv, "h"),
                        z: num(&kv, "z").unwrap_or(0),
                    });
                }
                if let Some(pending) = self.pending.as_mut() {
                    pending.data.extend_from_slice(data);
                }

                if more {
                    return KittyAction::None;
                }

                let pending = match self.pending.take() {
                    Some(pending) => pending,
                    None => return KittyAction::None,
                };
                let display = action == "T";
                self.finish_transmit(pending, cursor, display).unwrap_or(KittyAction::None)
            },
            "q" => KittyAction::Respond,
            "p" => {
                let image_id = num(&kv, "i").unwrap_or(0);
                if !self.images.contains_key(&image_id) {
                    return KittyAction::None;
                }
                let gen = self.images[&image_id].gen;
                let placement = self.build_placement(image_id, gen, &kv, cursor);
                let (col, row, cols, rows) = {
                    let img = &self.images[&image_id];
                    let (cols, rows) = image_box(img, placement.cells_w, placement.cells_h, placement.px_w, placement.px_h);
                    (placement.col, placement.row, cols, rows)
                };
                self.placements.push(placement);
                KittyAction::MarkCells { col, row, cols, rows }
            },
            "d" => {
                if let Some(id) = num::<u64>(&kv, "i") {
                    self.images.remove(&id);
                    self.placements.retain(|p| p.image_id != id);
                } else {
                    self.placements.clear();
                }
                KittyAction::Changed
            },
            _ => KittyAction::None,
        }
    }

    /// Drop all images and placements (e.g. when a pane is closed).
    pub fn clear(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.pending = None;
    }

    fn finish_transmit(&mut self, pending: Pending, cursor: (usize, isize), display: bool) -> Option<KittyAction> {
        let Pending { image_id, format, zlib, file, size, mut data, cells_w, cells_h, px_w, px_h, z } = pending;

        // Transmission from file: the payload is the base64 of a local path.
        if file {
            let Some(path) = decode_path(&data) else { return None };
            let Ok(bytes) = std::fs::read(&path) else { return None };
            return self.insert_and_maybe_display(
                image_id,
                bytes,
                format,
                size,
                cursor,
                display,
                cells_w,
                cells_h,
                px_w,
                px_h,
                z,
            );
        }

        // Inline transmission: the payload is base64, optionally zlib-compressed.
        let Some(decoded) = b64_decode(&data) else { return None };
        data = if zlib {
            match zlib_decompress(&decoded) {
                Some(d) => d,
                None => return None,
            }
        } else {
            decoded
        };

        self.insert_and_maybe_display(image_id, data, format, size, cursor, display, cells_w, cells_h, px_w, px_h, z)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_and_maybe_display(
        &mut self,
        image_id: u64,
        bytes: Vec<u8>,
        format: u32,
        size: Option<(u32, u32)>,
        cursor: (usize, isize),
        display: bool,
        cells_w: Option<f32>,
        cells_h: Option<f32>,
        px_w: Option<f32>,
        px_h: Option<f32>,
        z: i32,
    ) -> Option<KittyAction> {
        let Some(image) = decode_image(&bytes, format, size) else { return None };
        let gen = self.gen_counter;
        self.gen_counter += 1;

        if !display {
            self.images.insert(
                image_id,
                KittyImage { cells_w, cells_h, gen, ..image },
            );
            return Some(KittyAction::Changed);
        }

        // A transmit+display image is placed inline: report the covered cell box
        // so the caller can clear those cells and advance the cursor.
        let box_ = image_box(&image, cells_w, cells_h, px_w, px_h);
        self.images.insert(
            image_id,
            KittyImage { cells_w, cells_h, gen, ..image },
        );
        self.placements.push(Placement {
            image_id,
            gen,
            col: cursor.0,
            row: cursor.1,
            cells_w,
            cells_h,
            px_w,
            px_h,
            z,
        });
        Some(KittyAction::Place {
            col: cursor.0,
            row: cursor.1,
            cols: box_.0,
            rows: box_.1,
        })
    }

    fn build_placement(
        &self,
        image_id: u64,
        gen: u64,
        kv: &HashMap<String, String>,
        cursor: (usize, isize),
    ) -> Placement {
        let at_cursor = kv.get("C").map(String::as_str) == Some("1") || kv.get("X").is_none() && kv.get("Y").is_none();
        let (col, row) = if at_cursor {
            cursor
        } else {
            let col = num::<f32>(kv, "X").map(|x| x as usize).unwrap_or(cursor.0);
            let row = num::<f32>(kv, "Y").map(|y| y as isize).unwrap_or(cursor.1);
            (col, row)
        };
        // A placement may omit its size; fall back to the transmit-time size.
        let (cells_w, cells_h) = self
            .images
            .get(&image_id)
            .map(|img| (img.cells_w, img.cells_h))
            .unwrap_or((None, None));
        Placement {
            image_id,
            gen,
            col,
            row,
            cells_w: num(kv, "c").or(cells_w),
            cells_h: num(kv, "r").or(cells_h),
            px_w: num(kv, "w"),
            px_h: num(kv, "h"),
            z: num(kv, "z").unwrap_or(0),
        }
    }
}

/// Extract `(w, h)` from the `s=w,h` key, falling back to `v` for the height.
fn parse_size(kv: &HashMap<String, String>) -> Option<(u32, u32)> {
    let s = kv.get("s")?;
    let mut parts = s.split(',');
    let w: u32 = parts.next()?.parse().ok()?;
    let h: u32 = parts
        .next()
        .map(str::parse)
        .or_else(|| kv.get("v").map(String::as_str).map(str::parse))
        .and_then(Result::ok)?;
    Some((w, h))
}

/// Compute the cell box `(cols, rows)` an image placement covers, matching how
/// the renderer sizes the quad. Uses the requested cell size when given, the
/// pixel size with a nominal 10px cell otherwise, and the image aspect ratio
/// (with a nominal cell width/height ratio of 0.5) to derive the missing
/// dimension.
fn image_box(img: &KittyImage, cells_w: Option<f32>, cells_h: Option<f32>, px_w: Option<f32>, px_h: Option<f32>) -> (usize, usize) {
    let aspect = if img.width > 0 { img.height as f32 / img.width as f32 } else { 1.0 };
    let inv = if img.height > 0 { img.width as f32 / img.height as f32 } else { 1.0 };
    const CELL_ASPECT: f32 = 0.5; // nominal cell width / height

    let (cols, rows) = match (cells_w, cells_h) {
        (Some(c), Some(r)) => (c, r),
        (Some(c), None) => (c, c * aspect * CELL_ASPECT),
        (None, Some(r)) => (r * inv / CELL_ASPECT, r),
        (None, None) => {
            let w = px_w.unwrap_or(img.width as f32);
            let h = px_h.unwrap_or(img.height as f32);
            let c = w / 10.0;
            (c, c * h / w / CELL_ASPECT)
        },
    };
    (cols.max(1.0).ceil() as usize, rows.max(1.0).ceil() as usize)
}

/// Public wrapper over [`image_box`], for callers that need the cell box a
/// placement covers (e.g. to validate its cells are still intact).
pub fn placement_box(p: &Placement, img: &KittyImage) -> (usize, usize) {
    image_box(img, p.cells_w, p.cells_h, p.px_w, p.px_h)
}

fn decode_path(data: &[u8]) -> Option<PathBuf> {
    let decoded = b64_decode(data)?;
    let path = String::from_utf8(decoded).ok()?;
    Some(PathBuf::from(path))
}

fn b64_decode(data: &[u8]) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

fn zlib_decompress(data: &[u8]) -> Option<Vec<u8>> {
    match miniz_oxide::inflate::decompress_to_vec_zlib(data) {
        Ok(out) => Some(out),
        Err(_) => None,
    }
}

fn decode_image(bytes: &[u8], format: u32, size: Option<(u32, u32)>) -> Option<KittyImage> {
    match format {
        // Raw RGB24/RGBA32.
        24 | 32 => {
            let (w, h) = size?;
            let bpp = if format == 24 { 3 } else { 4 };
            let expected = w as usize * h as usize * bpp;
            if bytes.len() < expected {
                return None;
            }
            let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
            if format == 24 {
                for px in bytes[..expected].chunks_exact(3) {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            } else {
                rgba.extend_from_slice(&bytes[..expected]);
            }
            Some(KittyImage { width: w, height: h, rgba, cells_w: None, cells_h: None, gen: 0 })
        },
        // Compressed formats: PNG/JPEG/GIF/...
        _ => {
            let img = image::load_from_memory(bytes).ok()?;
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            Some(KittyImage { width: w, height: h, rgba: img.into_raw(), cells_w: None, cells_h: None, gen: 0 })
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

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
    fn fastfetch_style_inline_transmit_and_place() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(tiny_png());
        let payload = format!("Ga=T,f=100,i=1,c=4,r=3,m=0;{b64}");

        let mut store = KittyStore::new();
        let action = store.handle(payload.as_bytes(), (0, 2));
        assert!(matches!(action, KittyAction::Place { col: 0, row: 2, cols: 4, rows: 3 }));
        assert_eq!(store.images.len(), 1);
        assert_eq!(store.placements.len(), 1);

        let p = &store.placements[0];
        assert_eq!(p.col, 0);
        assert_eq!(p.row, 2);
        assert_eq!(p.cells_w, Some(4.0));
        assert_eq!(p.cells_h, Some(3.0));

        let img = &store.images[&1];
        assert_eq!((img.width, img.height), (4, 3));
    }

    #[test]
    fn fastfetch_style_file_transmit_then_place() {
        let dir = std::env::temp_dir().join("optix-kitty-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("logo.png");
        std::fs::write(&path, tiny_png()).unwrap();
        let b64_path = base64::engine::general_purpose::STANDARD.encode(path.to_str().unwrap());

        let mut store = KittyStore::new();

        // Transmit-only from file, recording the cell size at transmit time.
        let transmit = format!("Ga=t,f=100,t=f,c=40,r=25,i=9,z=-1,m=0;{b64_path}");
        assert_eq!(store.handle(transmit.as_bytes(), (0, 2)), KittyAction::Changed);
        assert_eq!(store.images.len(), 1);
        assert!(store.placements.is_empty(), "a=t transmits but does not place");

        // Place at the cursor; size comes from the transmit-time c/r.
        assert_eq!(store.handle(b"Ga=p,i=9,q=2,m=0", (0, 3)), KittyAction::MarkCells { col: 0, row: 3, cols: 40, rows: 25 });
        assert_eq!(store.placements.len(), 1);
        let p = &store.placements[0];
        assert_eq!(p.col, 0);
        assert_eq!(p.row, 3);
        assert_eq!(p.cells_w, Some(40.0));
        assert_eq!(p.cells_h, Some(25.0));

        // Transmit + display in one shot (a=T): places at the cursor.
        let display = format!("Ga=T,f=100,t=f,c=20,r=10,i=10,z=-1,m=0;{b64_path}");
        assert!(matches!(
            store.handle(display.as_bytes(), (0, 5)),
            KittyAction::Place { col: 0, row: 5, cols: 20, rows: 10 }
        ));
        let p2 = &store.placements[1];
        assert_eq!((p2.col, p2.row), (0, 5));
        assert_eq!(p2.cells_w, Some(20.0));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn chunked_transmit_reassembles_payload() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(tiny_png());
        let (a, b) = b64.split_at(b64.len() / 2);

        let mut store = KittyStore::new();
        let p1 = format!("a=t,f=100,s=4,3,i=7,m=1;{a}");
        assert_eq!(store.handle(p1.as_bytes(), (0, 0)), KittyAction::None);
        assert!(store.images.is_empty());

        let p2 = format!("m=0;{b}");
        assert_eq!(store.handle(p2.as_bytes(), (0, 0)), KittyAction::Changed);
        assert_eq!(store.images.len(), 1);
        assert_eq!(store.images[&7].width, 4);
    }

    #[test]
    fn capability_query_requests_response() {
        let mut store = KittyStore::new();
        assert_eq!(store.handle(b"a=q,response=0", (0, 0)), KittyAction::Respond);
    }

    #[test]
    fn delete_removes_image_and_placements() {
        let mut store = KittyStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode(tiny_png());
        store.handle(format!("a=t,f=100,i=1,m=0;{b64}").as_bytes(), (0, 0));
        store.handle(format!("a=p,i=1,c=1,r=1,m=0").as_bytes(), (0, 0));
        assert_eq!(store.placements.len(), 1);

        assert_eq!(store.handle(b"a=d,i=1,m=0", (0, 0)), KittyAction::Changed);
        assert!(store.images.is_empty());
        assert!(store.placements.is_empty());
    }
}
