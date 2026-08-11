//! Browser-grade scrolling for terminal panes.
//!
//! Alacritty's grid can only be scrolled in whole lines, which makes wheel
//! input feel abrupt. This module adds the missing pieces:
//!
//! - **Smooth motion** — the scroll position is tracked as a fractional number
//!   of lines and animated toward the target each frame. The terminal grid is
//!   advanced in whole-line steps while the fractional remainder is turned
//!   into a pixel shift that the renderer applies while drawing rows, so text
//!   glides instead of jumping.
//! - **Momentum** — wheel deltas are injected as velocity that decays over
//!   time (friction), giving the "flick and coast" feel of a web browser.
//! - **Scrollbar overlay** — geometry helpers shared between the renderer and
//!   the mouse-hit-testing so the thumb, track clicks and pills agree.

use std::time::{Duration, Instant};

/// Velocity added per wheel line (lines/sec of impulse).
const IMPULSE: f64 = 16.0;
/// Exponential velocity decay (1/sec). Higher = shorter coast.
const FRICTION: f64 = 14.0;
/// Maximum glide speed, so a fast flick never feels out of control.
const MAX_VEL: f64 = 90.0;
/// Below this velocity the scroll settles and animation stops.
const SETTLE_VEL: f64 = 0.05;
/// Scrollbar fade-in/out rate (1/sec).
const BAR_FADE_K: f64 = 10.0;
/// How long the scrollbar stays visible after the last interaction.
const BAR_HOLD: Duration = Duration::from_millis(1200);

/// Per-pane scroll state.
#[derive(Debug)]
pub struct ScrollState {
    /// Fractional scroll offset: lines scrolled up from the bottom.
    /// `0.0` is the prompt, `history_size` is the top of the scrollback.
    pub pos: f64,
    /// Momentum velocity in lines/sec (positive = toward older content).
    pub vel: f64,
    /// Whole-line offset currently applied to the terminal grid.
    pub applied: f64,
    /// Fractional remainder from `pos`, as a pixel shift for this frame.
    /// Positive shifts content down (scrolling to older content).
    pub shift: f32,
    /// Whether wheel input animates (`false` = instant line jumps).
    pub smooth: bool,
    /// Scrollbar overlay alpha (0..1), animated.
    pub bar_alpha: f32,
    /// The thumb is being dragged.
    pub dragging: bool,
    /// Pixel offset from the thumb's top edge to the grab point while dragging.
    pub drag_grab: f32,
    /// Pointer is hovering the scrollbar track.
    pub hover: bool,
    /// When the user last scrolled or grabbed the bar (drives auto-hide).
    pub last_active: Instant,
}

impl ScrollState {
    pub fn new(smooth: bool) -> Self {
        Self {
            pos: 0.0,
            vel: 0.0,
            applied: 0.0,
            shift: 0.0,
            smooth,
            bar_alpha: 0.0,
            dragging: false,
            drag_grab: 0.0,
            hover: false,
            last_active: Instant::now(),
        }
    }

    /// A wheel/keyboard delta in lines; positive = older, negative = newer.
    /// When momentum is enabled (and smooth motion is on) the delta becomes
    /// an impulse on the glide velocity; otherwise it moves the position
    /// directly for an instant, snappy step.
    pub fn input(&mut self, delta: f64, momentum: bool) {
        self.last_active = Instant::now();
        if self.smooth && momentum {
            self.vel = (self.vel + delta * IMPULSE).clamp(-MAX_VEL, MAX_VEL);
        } else {
            self.pos += delta;
            self.vel = 0.0;
        }
    }

    /// Jump to an absolute scroll position (page/top/bottom, thumb drag).
    pub fn jump(&mut self, pos: f64) {
        self.pos = pos;
        self.vel = 0.0;
        self.last_active = Instant::now();
    }

    /// Advance the animation by `dt` seconds given the current history length.
    ///
    /// Returns `(needs_draw, grid_delta)` where `grid_delta` is the number of
    /// whole lines the terminal grid still has to move this frame. The caller
    /// applies it with `grid.scroll_display(Scroll::Delta(delta))` and then
    /// recomputes the fractional `shift` from `pos`.
    pub fn tick(&mut self, dt: f64, history: usize) -> (bool, Option<i32>) {
        let max = history as f64;
        // Stay within the scrollable range (history grows/shrinks over time).
        self.pos = self.pos.clamp(0.0, max);

        if !self.smooth {
            // Discrete mode: pos is the position, no physics.
            let rounded = self.pos.round();
            let delta = (rounded - self.applied) as i32;
            if delta != 0 {
                self.applied = rounded;
            }
            let fading = self.fade_bar(dt);
            return (fading || delta != 0, if delta != 0 { Some(delta) } else { None });
        }

        if dt > 0.0 {
            // Momentum: velocity decays while it carries the position.
            self.vel *= (-FRICTION * dt).exp();
            self.pos += self.vel * dt;
        }

        // Hard bounds: stick to the edge and kill outward velocity.
        if self.pos <= 0.0 {
            self.pos = 0.0;
            self.vel = self.vel.max(0.0);
        } else if self.pos >= max {
            self.pos = max;
            self.vel = self.vel.min(0.0);
        }

        // Move the grid whenever the position crosses a whole-line boundary.
        let rounded = self.pos.round();
        let delta = (rounded - self.applied) as i32;
        if delta != 0 {
            self.applied = rounded;
        }

        // Everything is at rest when velocity died and we sit on a line.
        let moving = self.vel.abs() > SETTLE_VEL || (self.pos - rounded).abs() > 0.02;
        let fading = self.fade_bar(dt);

        (moving || fading || delta != 0, if delta != 0 { Some(delta) } else { None })
    }

    /// Re-sync state after something else moved the grid (search jump, resize).
    pub fn resync(&mut self, grid_offset: usize) {
        self.applied = grid_offset as f64;
        self.pos = self.applied;
        self.vel = 0.0;
        self.shift = 0.0;
    }

    /// Ease the scrollbar alpha toward visible/hidden; returns true while fading.
    fn fade_bar(&mut self, dt: f64) -> bool {
        let active = self.dragging || self.hover || self.last_active.elapsed() < BAR_HOLD;
        let target = if active { 1.0f32 } else { 0.0 };
        if dt > 0.0 {
            let k = 1.0 - (-BAR_FADE_K * dt).exp();
            self.bar_alpha += (target - self.bar_alpha) * k as f32;
            if (target - self.bar_alpha).abs() < 0.01 {
                self.bar_alpha = target;
            }
        } else {
            self.bar_alpha = target;
        }
        (self.bar_alpha - target).abs() > 0.01
    }

    /// Whether the scroll position is meaningfully off the bottom.
    pub fn is_scrolled_up(&self) -> bool {
        self.pos > 0.5
    }

    /// Whether the scroll position is at (or essentially at) the top.
    pub fn is_at_top(&self, history: usize) -> bool {
        history > 0 && self.pos >= history as f64 - 0.5
    }
}

/// Geometry of the on-pane scrollbar overlay.
#[derive(Debug, Clone, Copy)]
pub struct BarGeom {
    /// Full-width slot the pointer can hit (bar width + a small margin).
    pub hit_x: f32,
    pub hit_w: f32,
    /// Track (visible scrollbar column).
    pub x: f32,
    pub w: f32,
    pub track_y: f32,
    pub track_h: f32,
    pub thumb_y: f32,
    pub thumb_h: f32,
}

impl BarGeom {
    /// Whether a point (in pane-local window coordinates) is on the scrollbar.
    pub fn hit_test(&self, px: f32, py: f32) -> bool {
        px >= self.hit_x
            && px <= self.hit_x + self.hit_w
            && py >= self.track_y - 6.0
            && py <= self.track_y + self.track_h + 6.0
    }

    /// Whether a point is on the thumb itself.
    pub fn on_thumb(&self, px: f32, py: f32) -> bool {
        self.hit_test(px, py) && py >= self.thumb_y && py <= self.thumb_y + self.thumb_h
    }
}

/// Scrollbar track/thumb geometry for one pane.
///
/// * `rect` — pane rectangle (window coordinates).
/// * `dpi` — display scale factor.
/// * `pos` — current scroll offset in lines.
/// * `history` — total scrollback lines.
/// * `screen_lines` — visible lines.
pub fn bar_geom(rect: (f32, f32, f32, f32), dpi: f32, pos: f64, history: f64, screen_lines: usize) -> BarGeom {
    let (rx, ry, rw, rh) = rect;
    let w = (8.0 * dpi).min(rw * 0.35);
    let x = rx + rw - w - 4.0 * dpi;
    let hit_w = (w + 8.0 * dpi).min(rw);
    let hit_x = rx + rw - hit_w;

    let track_y = ry + 4.0 * dpi;
    let track_h = (rh - 8.0 * dpi).max(1.0);

    if history <= 0.0 {
        return BarGeom {
            hit_x,
            hit_w,
            x,
            w,
            track_y,
            track_h,
            thumb_y: track_y,
            thumb_h: track_h,
        };
    }

    let total = history + screen_lines as f64;
    let view_frac = (screen_lines as f64 / total).clamp(0.0, 1.0);
    let thumb_h = (track_h as f64 * view_frac).clamp(20.0 * dpi as f64, track_h as f64) as f32;
    let frac = (pos / history).clamp(0.0, 1.0);
    let thumb_y = track_y + (track_h - thumb_h) * frac as f32;

    BarGeom { hit_x, hit_w, x, w, track_y, track_h, thumb_y, thumb_h }
}

/// A floating pill (used for "back to bottom" / "back to top").
#[derive(Debug, Clone, Copy)]
pub struct PillGeom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PillGeom {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// Bottom-center pill shown while scrolled up; clicking returns to the prompt.
pub fn bottom_pill(rect: (f32, f32, f32, f32), dpi: f32) -> PillGeom {
    let (rx, ry, rw, rh) = rect;
    let h = 24.0 * dpi;
    let w = 130.0 * dpi;
    let x = rx + (rw - w) * 0.5;
    let y = ry + rh - h - 12.0 * dpi;
    PillGeom { x, y, w, h }
}

/// Top-center pill shown while at the very top of the scrollback.
pub fn top_pill(rect: (f32, f32, f32, f32), dpi: f32) -> PillGeom {
    let (rx, ry, rw, _rh) = rect;
    let h = 24.0 * dpi;
    let w = 130.0 * dpi;
    let x = rx + (rw - w) * 0.5;
    let y = ry + 12.0 * dpi;
    PillGeom { x, y, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smooth(history: usize) -> ScrollState {
        let mut s = ScrollState::new(true);
        // Large first tick advances physics without racing; call tick to settle.
        let _ = s.tick(0.0, history);
        s
    }

    #[test]
    fn stepped_input_moves_immediately() {
        let mut s = ScrollState::new(false);
        s.input(3.0, false);
        let (_, delta) = s.tick(1.0 / 60.0, 100);
        assert_eq!(delta, Some(3));
        assert_eq!(s.pos, 3.0);
    }

    #[test]
    fn glide_impulse_carries_past_the_impulse() {
        // Momentum: a 3-line notch coasts several lines before settling.
        let mut s = smooth(1000);
        s.input(3.0, true);
        for _ in 0..240 {
            let _ = s.tick(1.0 / 60.0, 1000);
            if s.vel.abs() <= SETTLE_VEL && (s.pos - s.pos.round()).abs() < 0.02 {
                break;
            }
        }
        assert!(s.vel.abs() <= SETTLE_VEL, "velocity must settle");
        // Total travel is more than the raw 3-line notch (momentum coast).
        assert!(s.pos > 3.0, "expected momentum coast, got {}", s.pos);
    }

    #[test]
    fn glide_respects_history_bounds() {
        // A flick with enough energy to cross the whole (small) history must
        // clamp at the top; an equal flick down clamps at the bottom.
        let mut s = smooth(3);
        s.input(100.0, true); // enormous flick
        for _ in 0..300 {
            let _ = s.tick(1.0 / 60.0, 3);
        }
        assert_eq!(s.pos, 3.0);

        s.input(-100.0, true);
        for _ in 0..300 {
            let _ = s.tick(1.0 / 60.0, 3);
        }
        assert_eq!(s.pos, 0.0);
    }

    #[test]
    fn grid_delta_accumulates_in_whole_lines() {
        let mut s = smooth(50);
        s.input(5.0, true);
        let mut total = 0i32;
        for _ in 0..240 {
            let (_, delta) = s.tick(1.0 / 60.0, 50);
            if let Some(d) = delta {
                total += d;
            }
            // Stop once the glide has fully settled.
            if s.vel.abs() <= SETTLE_VEL && (s.pos - s.pos.round()).abs() < 0.02 {
                break;
            }
        }
        assert_eq!(s.pos.round() as i32, total, "applied grid delta matches pos");
    }

    #[test]
    fn jump_lands_exactly() {
        let mut s = smooth(50);
        s.jump(42.0);
        let (needs, delta) = s.tick(0.0, 50);
        assert_eq!(delta, Some(42));
        assert!(needs);
        // No physics: position stays pinned.
        let (needs2, delta2) = s.tick(1.0 / 60.0, 50);
        assert_eq!(delta2, None);
        assert!(!needs2);
    }

    #[test]
    fn resync_after_external_scroll() {
        let mut s = smooth(50);
        s.input(7.0, false);
        // Something else (search) moved the grid directly.
        s.resync(12);
        let (_, delta) = s.tick(0.0, 50);
        assert_eq!(delta, None, "resynced state produces no grid delta");
        assert_eq!(s.pos, 12.0);
    }

    #[test]
    fn bar_fades_in_and_out() {
        let mut s = smooth(0);
        s.hover = true;
        for _ in 0..120 {
            let _ = s.tick(1.0 / 60.0, 0);
        }
        assert_eq!(s.bar_alpha, 1.0);
        s.hover = false;
        s.last_active = Instant::now() - Duration::from_secs(10);
        for _ in 0..120 {
            let _ = s.tick(1.0 / 60.0, 0);
        }
        assert_eq!(s.bar_alpha, 0.0);
    }

    #[test]
    fn geometry_thumb_stays_within_track() {
        let g = bar_geom((0.0, 0.0, 800.0, 600.0), 1.0, 0.0, 100.0, 37);
        assert!(g.thumb_y >= g.track_y);
        assert!(g.thumb_y + g.thumb_h <= g.track_y + g.track_h + 0.01);

        let g = bar_geom((0.0, 0.0, 800.0, 600.0), 1.0, 100.0, 100.0, 37);
        assert!((g.thumb_y - (g.track_y + g.track_h - g.thumb_h)).abs() < 0.01);

        // No history: thumb fills the track and hit testing still works.
        let g = bar_geom((0.0, 0.0, 800.0, 600.0), 1.0, 0.0, 0.0, 37);
        assert!(g.hit_test(790.0, 300.0));
    }

    #[test]
    fn pills_are_centered_and_clickable() {
        let p = bottom_pill((100.0, 50.0, 800.0, 600.0), 1.0);
        assert!((p.x + p.w / 2.0 - 500.0).abs() < 0.01, "pill centered");
        assert!(p.contains(500.0, p.y + p.h / 2.0));
        assert!(!p.contains(100.0, 50.0));
    }
}
