//! The readouts next to the cursor, like macOS ⌘⇧4: two stacked numbers in white text with a
//! black outline, and no box around them.
//!
//! - Position (x, y) at the top-left of the cursor. Once you press, it stays at the start point.
//! - Size (width, height) at the bottom-right of the cursor while dragging, and stays after.
//!
//! Values are in points, like macOS shows them. Each readout is a small window clipped to its
//! outlined text, so nothing around the numbers covers the screen.

use fontdue::Font;
use tiny_skia::{Pixmap, PremultipliedColorU8};
use winit::event_loop::ActiveEventLoop;

use super::piece::Piece;
use super::select::Geom;
use super::toolbar::text_width;

/// Sizes in points.
const TEXT_PT: f32 = 13.0;
const LINE_PT: f32 = 15.0;
/// The embedded font is thin, so the letters are thickened by this much to keep the white
/// readable inside the outline.
const BOLD_PT: f32 = 0.4;
/// Thickness of the black outline around the (thickened) letters.
const OUTLINE_PT: f32 = 0.75;
/// Distance between the cursor point and the text, in points.
const GAP_PT: f64 = 8.0;

/// Which side of the point the box goes.
#[derive(Clone, Copy)]
pub enum Side {
    TopLeft,
    BottomRight,
}

/// Draws two lines of white text with a black outline. Everything else is transparent.
pub fn paint(font: &Font, lines: [&str; 2], scale: f64) -> Option<Pixmap> {
    let s = scale as f32;
    let px = TEXT_PT * s;
    let bold = BOLD_PT * s;
    let rr = (OUTLINE_PT * s).max(1.0);
    let pad = (bold + rr).ceil() + 1.0;
    let tw = lines.iter().map(|t| text_width(font, t, px)).fold(0.0, f32::max);
    // Whole points, so the window macOS makes for it has exactly this many pixels.
    let step = scale.ceil().max(1.0) as i32;
    let whole = |v: f32| (v.ceil() as i32 + step - 1) / step * step;
    let (w, h) = (whole(tw + 2.0 * pad), whole(2.0 * LINE_PT * s + 2.0 * pad));

    // Coverage of the letters themselves.
    let mut text = vec![0f32; (w * h) as usize];
    let baseline_drop = font.metrics('8', px).height as f32 / 2.0;
    for (i, line) in lines.iter().enumerate() {
        let cy = pad + LINE_PT * s * (i as f32 + 0.5);
        let mut pen = pad;
        for ch in line.chars() {
            let (m, cov) = font.rasterize(ch, px);
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (cy + baseline_drop - m.height as f32 - m.ymin as f32).round() as i32;
            for row in 0..m.height as i32 {
                for col in 0..m.width as i32 {
                    let (x, y) = (gx + col, gy + row);
                    if x >= 0 && y >= 0 && x < w && y < h {
                        let c = cov[(row * m.width as i32 + col) as usize] as f32 / 255.0;
                        let k = (y * w + x) as usize;
                        text[k] = text[k].max(c);
                    }
                }
            }
            pen += m.advance_width;
        }
    }

    // Thicker letters, then the outline: each grown in every direction, with a soft edge.
    let text = grow(&text, w, h, bold);
    let outline = grow(&text, w, h, rr);

    // White letters over the black outline, premultiplied.
    let mut pm = Pixmap::new(w as u32, h as u32)?;
    for (k, px) in pm.pixels_mut().iter_mut().enumerate() {
        let (t, o) = (text[k], outline[k]);
        let a = t + o * (1.0 - t);
        let v = (t * 255.0).round() as u8; // white * t, black adds nothing
        let a8 = (a * 255.0).round() as u8;
        *px = PremultipliedColorU8::from_rgba(v.min(a8), v.min(a8), v.min(a8), a8).unwrap_or(*px);
    }
    Some(pm)
}

/// `mask` grown by `r` pixels in every direction (a soft-edged dilation). `r` of 0 keeps it.
fn grow(mask: &[f32], w: i32, h: i32, r: f32) -> Vec<f32> {
    if r <= 0.0 {
        return mask.to_vec();
    }
    let reach = r.ceil() as i32;
    // Weight of each neighbour (soft round edge), worked out once; zero weights are dropped.
    let kernel: Vec<(i32, i32, f32)> = (-reach..=reach)
        .flat_map(|dy| (-reach..=reach).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| (dx, dy, (r + 0.5 - ((dx * dx + dy * dy) as f32).sqrt()).clamp(0.0, 1.0)))
        .filter(|&(_, _, k)| k > 0.0)
        .collect();
    let mut out = vec![0f32; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let mut best = 0f32;
            for &(dx, dy, k) in &kernel {
                let (sx, sy) = (x + dx, y + dy);
                if sx >= 0 && sy >= 0 && sx < w && sy < h {
                    best = best.max(mask[(sy * w + sx) as usize] * k);
                }
            }
            out[(y * w + x) as usize] = best;
        }
    }
    out
}

/// The position text for a point in physical pixels: two lines, x and y in points.
pub fn position(x: i32, y: i32, scale: f64) -> [String; 2] {
    [pt(x, scale).to_string(), pt(y, scale).to_string()]
}

/// The size text for a rectangle in physical pixels: two lines, width and height in points.
pub fn size(w: i32, h: i32, scale: f64) -> [String; 2] {
    position(w, h, scale)
}

/// The progress text once frames are captured: "3 frames" over the joined height in points.
pub fn progress(frames: usize, height_px: u32, scale: f64) -> [String; 2] {
    let noun = if frames == 1 { "frame" } else { "frames" };
    [format!("{frames} {noun}"), format!("{} pt tall", pt(height_px as i32, scale))]
}

fn pt(v: i32, scale: f64) -> i32 {
    (v as f64 / scale).round() as i32
}

/// Top-left of a `w` x `h` box placed on `side` of the point `(px, py)`, kept on the monitor
/// (`mw` x `mh`). All in physical pixels.
#[allow(clippy::too_many_arguments)]
pub fn origin(px: i32, py: i32, w: i32, h: i32, side: Side, scale: f64, mw: i32, mh: i32) -> (i32, i32) {
    let gap = (GAP_PT * scale).round() as i32;
    let (x, y) = match side {
        Side::TopLeft => (px - gap - w, py - gap - h),
        Side::BottomRight => (px + gap, py + gap),
    };
    (x.clamp(0, (mw - w).max(0)), y.clamp(0, (mh - h).max(0)))
}

/// The colour of each pixel (0x00RRGGBB, not premultiplied) and its opacity, for a window that
/// is clipped to the opacity by the OS (see `platform::set_alpha_mask`).
pub fn to_pixels(pm: &Pixmap) -> (Vec<u32>, Vec<u8>) {
    pm.pixels()
        .iter()
        .map(|p| {
            let c = p.demultiply();
            ((c.red() as u32) << 16 | (c.green() as u32) << 8 | c.blue() as u32, c.alpha())
        })
        .unzip()
}

/// A readout window. It is repainted only when its text changes; otherwise it is just moved.
pub(super) struct Readout {
    piece: Piece,
    text: Option<[String; 2]>,
    size: (i32, i32),
}

impl Readout {
    pub fn new(el: &ActiveEventLoop) -> Option<Readout> {
        Some(Readout { piece: Piece::new(el, 0, 1.0, None)?, text: None, size: (0, 0) })
    }

    /// Shows `text` on `side` of `point` (physical pixels, monitor-relative).
    pub fn show(&mut self, font: &Font, text: [String; 2], point: (i32, i32), side: Side, geom: Geom) {
        if self.text.as_ref() != Some(&text) {
            let Some(pm) = paint(font, [&text[0], &text[1]], geom.scale) else { return };
            self.size = (pm.width() as i32, pm.height() as i32);
            let (rgb, alpha) = to_pixels(&pm);
            self.piece.set_image(rgb, alpha, pm.width());
            self.text = Some(text);
        }
        let (w, h) = self.size;
        let (x, y) = origin(point.0, point.1, w, h, side, geom.scale, geom.width, geom.height);
        self.piece.place(geom, x, y, w, h);
    }

    pub fn hide(&mut self) {
        self.piece.hide();
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.piece.set_visible(visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_sit_on_the_right_side_of_the_point_and_stay_on_the_monitor() {
        // 2x screen: gap is 16 px.
        assert_eq!(origin(500, 400, 60, 40, Side::TopLeft, 2.0, 1000, 800), (424, 344));
        assert_eq!(origin(500, 400, 60, 40, Side::BottomRight, 2.0, 1000, 800), (516, 416));
        // Near the edges they are pushed back inside.
        assert_eq!(origin(5, 5, 60, 40, Side::TopLeft, 2.0, 1000, 800), (0, 0));
        assert_eq!(origin(990, 790, 60, 40, Side::BottomRight, 2.0, 1000, 800), (940, 760));
    }

    #[test]
    fn progress_counts_frames_and_points() {
        assert_eq!(progress(1, 400, 2.0), ["1 frame".to_string(), "200 pt tall".to_string()]);
        assert_eq!(progress(3, 900, 1.0), ["3 frames".to_string(), "900 pt tall".to_string()]);
    }

    #[test]
    fn values_are_in_points() {
        assert_eq!(position(1001, 400, 2.0), ["501".to_string(), "200".to_string()]);
        assert_eq!(size(300, 200, 1.0), ["300".to_string(), "200".to_string()]);
    }

    #[test]
    fn the_text_is_drawn_white_with_a_dark_outline() {
        let font = Font::from_bytes(
            include_bytes!("../../assets/font.ttf") as &[u8],
            fontdue::FontSettings::default(),
        )
        .unwrap();
        let pm = paint(&font, ["1234", "567"], 2.0).unwrap();
        assert!(pm.width() > 40 && pm.height() > 40);
        // Whole points on a 2x screen.
        for t in [["1", "7"], ["415", "260"], ["117", "63"], ["1234", "567"]] {
            let p = paint(&font, t, 2.0).unwrap();
            assert!(p.width().is_multiple_of(2) && p.height().is_multiple_of(2), "{t:?}");
        }
        let (rgb, alpha) = to_pixels(&pm);
        assert_eq!(rgb.len(), (pm.width() * pm.height()) as usize);
        // Some pixels are white text, some are outline only, the corners are see-through.
        assert!(rgb.iter().zip(&alpha).any(|(&c, &a)| c == 0x00FF_FFFF && a == 255));
        assert!(rgb.iter().zip(&alpha).any(|(&c, &a)| (c >> 16) < 64 && a > 128));
        assert_eq!(alpha[0], 0);
    }
}
