//! Magnet: edges of the area snap to lines on the screen (window borders, dividers, the edge of
//! a panel), so an area can be fitted to something exactly without pixel hunting.
//!
//! A still picture of the whole monitor is taken in the background (opening stays instant), and
//! an edge being dragged looks for the strongest line close to it. A line only counts if it runs
//! along most of that edge: the picture's colour must change across it on most rows (or
//! columns) the edge covers. Text and photos change colour too, but not in one straight line.
//!
//! The picture is retaken whenever a resize starts, since the page may have scrolled. Overlay
//! windows must stay out of it (or the frame would snap to its own border), so the magnet only
//! works where the platform keeps them out of captures.

use std::sync::mpsc::{self, Receiver};

use super::select::Geom;
use crate::platform;

/// How far an edge can jump to reach a line, in points.
const REACH_PT: f64 = 8.0;
/// A short edge is checked over at least this length (in points) around its middle, so a single
/// point (where a drag starts) or a thin area does not snap to noise.
const MIN_SPAN_PT: f64 = 24.0;
/// Two neighbouring pixels differ if any channel differs by more than this.
const CONTRAST: u8 = 20;
/// Share of the edge's length the line must run along.
const MIN_COVER: f64 = 0.6;
/// At most this many rows (or columns) are compared per candidate line.
const MAX_SAMPLES: i32 = 400;

/// A whole-monitor picture, RGBA, in physical pixels.
struct Picture {
    w: i32,
    h: i32,
    rgba: Vec<u8>,
}

impl Picture {
    fn px(&self, x: i32, y: i32) -> &[u8] {
        let i = ((y * self.w + x) * 4) as usize;
        &self.rgba[i..i + 3]
    }

    fn differ(&self, a: (i32, i32), b: (i32, i32)) -> bool {
        let (p, q) = (self.px(a.0, a.1), self.px(b.0, b.1));
        p.iter().zip(q).any(|(u, v)| u.abs_diff(*v) > CONTRAST)
    }
}

pub struct Magnet {
    on: bool,
    geom: Geom,
    picture: Option<Picture>,
    /// The picture being taken in the background.
    pending: Option<Receiver<Option<Picture>>>,
}

impl Magnet {
    /// Starts taking the first picture right away.
    pub fn new(geom: Geom, on: bool) -> Magnet {
        let mut m = Magnet { on, geom, picture: None, pending: None };
        m.refresh();
        m
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn set_on(&mut self, on: bool) {
        self.on = on;
        if on {
            self.refresh();
        }
    }

    /// Retakes the picture in the background. Until it is ready there is no snapping: the old
    /// picture could show lines that have scrolled away.
    pub fn refresh(&mut self) {
        // Where overlay windows show up in captures, the picture would contain the frame itself.
        if !self.on || !platform::capture_skips_overlay() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        // Monitors cannot move between threads on every platform: the thread finds this one
        // again by a point inside it (in the units xcap uses for positions).
        let (gw, gh) = (self.geom.width, self.geom.height);
        let unit = if cfg!(target_os = "macos") { self.geom.scale } else { 1.0 };
        let inside = (
            self.geom.origin.0 + (gw as f64 / unit / 2.0) as i32,
            self.geom.origin.1 + (gh as f64 / unit / 2.0) as i32,
        );
        std::thread::spawn(move || {
            let monitor = xcap::Monitor::from_point(inside.0, inside.1).ok();
            let picture = monitor.and_then(|m| m.capture_image().ok()).and_then(|img| {
                let (w, h) = (img.width() as i32, img.height() as i32);
                // Expect one pixel per physical pixel; otherwise positions would not match.
                ((w, h) == (gw, gh)).then(|| Picture { w, h, rgba: img.into_raw() })
            });
            let _ = tx.send(picture);
        });
        self.pending = Some(rx);
        self.picture = None;
    }

    /// Picks up the background picture if it is ready.
    fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(picture) => {
                self.picture = picture;
                self.pending = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
        }
    }

    /// A vertical edge at `x`, running from `y0` to `y1`: where it should be (physical pixels,
    /// monitor-relative). Unchanged when the magnet is off or no line is near.
    pub fn snap_x(&mut self, x: i32, y0: i32, y1: i32) -> i32 {
        self.snap(x, y0, y1, true)
    }

    /// A horizontal edge at `y`, running from `x0` to `x1`.
    pub fn snap_y(&mut self, y: i32, x0: i32, x1: i32) -> i32 {
        self.snap(y, x0, x1, false)
    }

    /// A single point (a corner before the area is drawn): snaps to nearby lines through it.
    pub fn snap_point(&mut self, x: i32, y: i32) -> (i32, i32) {
        (self.snap_x(x, y, y), self.snap_y(y, x, x))
    }

    fn snap(&mut self, at: i32, a: i32, b: i32, vertical: bool) -> i32 {
        if !self.on {
            return at;
        }
        self.poll();
        let Some(pic) = &self.picture else { return at };
        let step = self.geom.step();
        let reach = (REACH_PT * self.geom.scale).round() as i32;
        let min_span = (MIN_SPAN_PT * self.geom.scale).round() as i32;
        best_line(pic, at, a.min(b), a.max(b), vertical, step, reach, min_span).unwrap_or(at)
    }
}

/// The strongest line within `reach` of `at` (only on multiples of `step`, so captures still line
/// up with whole points), checked along `lo..hi` on the other axis. `None` if there is none.
#[allow(clippy::too_many_arguments)]
fn best_line(pic: &Picture, at: i32, lo: i32, hi: i32, vertical: bool, step: i32, reach: i32, min_span: i32) -> Option<i32> {
    let (len, across) = if vertical { (pic.h, pic.w) } else { (pic.w, pic.h) };
    // Widen a short edge around its middle, and keep it on the picture.
    let (mut lo, mut hi) = (lo, hi);
    if hi - lo < min_span {
        let mid = (lo + hi) / 2;
        (lo, hi) = (mid - min_span / 2, mid + min_span / 2);
    }
    let (lo, hi) = (lo.clamp(0, len), hi.clamp(0, len));
    if hi - lo < 2 {
        return None;
    }
    let every = ((hi - lo) / MAX_SAMPLES).max(1);

    let first = ((at - reach) as f64 / step as f64).ceil() as i32 * step;
    let mut best: Option<(f64, i32)> = None;
    let mut c = first.max(step);
    while c <= at + reach {
        // The boundary between pixel c-1 and pixel c.
        if c >= across {
            break;
        }
        let (mut hits, mut total) = (0, 0);
        let mut i = lo;
        while i < hi {
            let (p, q) = if vertical { ((c - 1, i), (c, i)) } else { ((i, c - 1), (i, c)) };
            hits += pic.differ(p, q) as i32;
            total += 1;
            i += every;
        }
        let cover = hits as f64 / total as f64;
        if cover >= MIN_COVER {
            // Stronger lines win; among equally strong ones, the nearest.
            let score = cover - 0.1 * (c - at).abs() as f64 / reach.max(1) as f64;
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, c));
            }
        }
        c += step;
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white `w` x `h` picture with a grey vertical line at columns `lx..lx+lw`, rows `ly0..ly1`.
    fn picture(w: i32, h: i32, lx: i32, lw: i32, ly0: i32, ly1: i32) -> Picture {
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for y in ly0..ly1 {
            for x in lx..lx + lw {
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 3].copy_from_slice(&[200, 200, 200]);
            }
        }
        Picture { w, h, rgba }
    }

    #[test]
    fn an_edge_near_a_line_snaps_onto_it() {
        let pic = picture(200, 200, 100, 2, 0, 200);
        // Left side of the line from the left, right side from the right.
        assert_eq!(best_line(&pic, 94, 20, 180, true, 2, 16, 48), Some(100));
        assert_eq!(best_line(&pic, 108, 20, 180, true, 2, 16, 48), Some(102));
        // Too far away: stays put.
        assert_eq!(best_line(&pic, 60, 20, 180, true, 2, 16, 48), None);
        // A horizontal edge finds nothing on a vertical line.
        assert_eq!(best_line(&pic, 100, 20, 180, false, 2, 16, 48), None);
    }

    #[test]
    fn a_line_must_run_along_most_of_the_edge() {
        // The line covers rows 0..50 only; an edge over rows 0..200 ignores it.
        let pic = picture(200, 200, 100, 2, 0, 50);
        assert_eq!(best_line(&pic, 96, 0, 200, true, 2, 16, 48), None);
        // But an edge over rows 0..60 finds it.
        assert_eq!(best_line(&pic, 96, 0, 60, true, 2, 16, 48), Some(100));
    }

    #[test]
    fn a_point_is_checked_over_a_minimum_span() {
        let pic = picture(200, 200, 100, 2, 0, 200);
        assert_eq!(best_line(&pic, 97, 80, 80, true, 2, 16, 48), Some(100));
    }
}
