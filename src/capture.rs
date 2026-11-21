//! Grabs the pixels inside the chosen rectangle from the live screen.

use crate::overlay::select::Rect;

/// One captured frame: RGBA, 8 bits per channel, no padding between rows.
pub struct Shot {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Captures `r` (physical pixels, relative to the monitor). Only that area is read when the
/// rectangle lines up with whole points (it normally does, see `select::snap`); otherwise the
/// whole monitor is read and cut. Returns `None` if the screen can't be read.
pub fn grab(monitor: &xcap::Monitor, r: Rect, scale: f64) -> Option<Shot> {
    if r.w <= 0 || r.h <= 0 {
        return None;
    }
    if let Some(shot) = grab_region(monitor, r, scale) {
        return Some(shot);
    }
    let img = match monitor.capture_image() {
        Ok(img) => img,
        Err(e) => {
            eprintln!("screen capture failed: {e}");
            return None;
        }
    };
    crop(img.width() as i32, img.height() as i32, img.as_raw(), r)
}

/// Reads only the rectangle. xcap takes the region in the units it reports for the monitor
/// (points on macOS, pixels on Windows).
fn grab_region(monitor: &xcap::Monitor, r: Rect, scale: f64) -> Option<Shot> {
    let unit = if cfg!(target_os = "macos") { scale } else { 1.0 };
    let to_units = |v: i32| {
        let u = v as f64 / unit;
        (u.fract() == 0.0 && u >= 0.0).then_some(u as u32)
    };
    let (x, y, w, h) = (to_units(r.x)?, to_units(r.y)?, to_units(r.w)?, to_units(r.h)?);
    let img = monitor.capture_region(x, y, w, h).ok()?;
    // Expect exactly the rectangle; anything else (an unusual scale) goes through the full path.
    if (img.width() as i32, img.height() as i32) != (r.w, r.h) {
        return None;
    }
    Some(Shot { w: img.width(), h: img.height(), rgba: img.into_raw() })
}

/// Cuts `r` out of a whole-monitor RGBA image `iw` x `ih`.
fn crop(iw: i32, ih: i32, raw: &[u8], r: Rect) -> Option<Shot> {
    let (x0, y0) = (r.x.clamp(0, iw), r.y.clamp(0, ih));
    let (x1, y1) = ((r.x + r.w).clamp(0, iw), (r.y + r.h).clamp(0, ih));
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y1 {
        let start = ((y * iw + x0) * 4) as usize;
        rgba.extend_from_slice(&raw[start..start + (w * 4) as usize]);
    }
    Some(Shot { w: w as u32, h: h as u32, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_cuts_the_rectangle_and_clips_to_the_image() {
        // 4 x 3 image where each pixel's red value is its index.
        let raw: Vec<u8> = (0..12u8).flat_map(|i| [i, 0, 0, 255]).collect();
        let s = crop(4, 3, &raw, Rect { x: 1, y: 1, w: 2, h: 2 }).unwrap();
        assert_eq!((s.w, s.h), (2, 2));
        let reds: Vec<u8> = s.rgba.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, vec![5, 6, 9, 10]);
        let clipped = crop(4, 3, &raw, Rect { x: 3, y: 2, w: 5, h: 5 }).unwrap();
        assert_eq!((clipped.w, clipped.h), (1, 1));
        assert!(crop(4, 3, &raw, Rect { x: 9, y: 9, w: 2, h: 2 }).is_none());
    }
}
