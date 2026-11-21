//! One small borderless overlay window: a border strip, a handle, a readout, an invisible grab
//! area or the selector's mouse catcher.
//!
//! It remembers where it is and what it shows, so moving it only moves the window, and it is
//! repainted (and re-masked) only when its size or content changes. Overlay windows are kept out
//! of screen captures where the platform allows it.

use std::borrow::Cow;
use std::num::NonZeroU32;
use std::rc::Rc;

use softbuffer::{Context, Surface};
#[cfg(target_os = "macos")]
use winit::dpi::{LogicalPosition, LogicalSize};
#[cfg(not(target_os = "macos"))]
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{CursorIcon, Window, WindowId};

use super::select::Geom;
use crate::platform;

/// What the window shows.
#[derive(PartialEq)]
enum Content {
    /// One colour (0x00RRGGBB) over the whole window.
    Fill(u32),
    /// A pre-drawn picture `w` pixels wide: colour and opacity per pixel. The window is clipped
    /// to the opacity by the OS.
    Image { rgb: Vec<u32>, alpha: Vec<u8>, w: u32 },
}

pub(super) struct Piece {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    content: Content,
    /// Content changed since the last paint.
    dirty: bool,
    /// Last placement (physical pixels, monitor-relative) and painted size.
    at: Option<(i32, i32, i32, i32)>,
    painted: Option<(u32, u32)>,
    shown: bool,
    /// Window opacity, set again after each show (Windows forgets it).
    alpha: f64,
    /// Never takes activation when clicked (see `platform::no_activate`).
    no_activate: bool,
}

impl Piece {
    /// `alpha` is the window's opacity. `cursor` makes it take the mouse and show that cursor;
    /// without one, clicks and scrolls pass through.
    pub fn new(el: &ActiveEventLoop, color: u32, alpha: f64, cursor: Option<CursorIcon>) -> Option<Piece> {
        let attrs = platform::overlay_attrs().with_inner_size(winit::dpi::PhysicalSize::new(1u32, 1u32));
        let window = Rc::new(el.create_window(attrs).ok()?);
        match cursor {
            Some(icon) => {
                // Take the mouse even if the window is almost fully transparent.
                let _ = window.set_cursor_hittest(true);
                window.set_cursor(icon);
                // Hover still works while the app being captured is the active one.
                platform::track_mouse_always(&window);
            }
            None => window.set_cursor_hittest(false).ok()?,
        }
        platform::set_window_alpha(&window, alpha);
        platform::set_level(&window, platform::LEVEL_STRIPS);
        platform::on_all_spaces(&window);
        platform::no_shadow(&window);
        platform::exclude_from_capture(&window);

        let context = Context::new(window.clone()).ok()?;
        let surface = Surface::new(&context, window.clone()).ok()?;
        Some(Piece {
            window,
            surface,
            content: Content::Fill(color),
            dirty: true,
            at: None,
            painted: None,
            shown: false,
            alpha,
            no_activate: false,
        })
    }

    /// Clicks on this window leave the app being captured active (handles and grab areas).
    pub fn set_no_activate(&mut self) {
        self.no_activate = true;
        platform::no_activate(&self.window);
    }

    fn after_show(&self) {
        platform::after_show(&self.window, Some(self.alpha), self.no_activate);
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn set_fill(&mut self, color: u32) {
        let next = Content::Fill(color);
        if self.content != next {
            self.content = next;
            self.dirty = true;
        }
    }

    pub fn set_image(&mut self, rgb: Vec<u32>, alpha: Vec<u8>, w: u32) {
        let next = Content::Image { rgb, alpha, w };
        if self.content != next {
            self.content = next;
            self.dirty = true;
        }
    }

    pub fn hide(&mut self) {
        if self.shown {
            self.window.set_visible(false);
            self.shown = false;
        }
    }

    /// Shows a window hidden by `hide` without moving it (used around captures where overlay
    /// windows cannot be kept out of the picture).
    pub fn set_visible(&mut self, visible: bool) {
        if visible && self.at.is_some() && !self.shown {
            self.window.set_visible(true);
            self.shown = true;
            self.after_show();
        } else if !visible {
            self.hide();
        }
    }

    /// Puts the window at `(x, y, w, h)` physical pixels inside the monitor (clipped to it), and
    /// repaints only if needed. A zero-sized rectangle hides it.
    pub fn place(&mut self, geom: Geom, x: i32, y: i32, w: i32, h: i32) {
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(geom.width), (y + h).min(geom.height));
        let (x, y, w, h) = (x0, y0, x1 - x0, y1 - y0);
        if w <= 0 || h <= 0 {
            self.hide();
            self.at = None;
            return;
        }
        let moved = self.at != Some((x, y, w, h));
        if moved {
            self.at = Some((x, y, w, h));
            set_frame(&self.window, geom, x, y, w, h);
        }

        let size = self.window.inner_size();
        let (sw, sh) = (size.width, size.height);
        if self.dirty || self.painted != Some((sw, sh)) {
            self.paint(sw, sh);
        }

        // Show once: showing again would pull keyboard focus away from the toolbar.
        if !self.shown {
            self.shown = true;
            self.window.set_visible(true);
            self.after_show();
            // Showing a window is when macOS may push it below the menu bar, so set the frame
            // again now that it is on screen.
            set_frame(&self.window, geom, x, y, w, h);
        }
    }

    fn paint(&mut self, sw: u32, sh: u32) {
        let (Some(nw), Some(nh)) = (NonZeroU32::new(sw), NonZeroU32::new(sh)) else { return };
        if self.surface.resize(nw, nh).is_err() {
            return;
        }
        // If the OS made the window a slightly different size than the picture, the picture is
        // copied into the corner that fits and the rest is see-through, rather than left black.
        let fitted = match &self.content {
            Content::Image { rgb, alpha, w } => Some(fit(rgb, alpha, *w, sw, sh)),
            Content::Fill(_) => None,
        };
        if let Ok(mut buffer) = self.surface.buffer_mut() {
            match (&fitted, &self.content) {
                (Some((rgb, _)), _) => buffer.copy_from_slice(rgb),
                (None, Content::Fill(c)) => buffer.fill(*c),
                _ => {}
            }
            let _ = buffer.present();
        }
        if let Some((_, alpha)) = &fitted {
            platform::set_alpha_mask(&self.window, sw, sh, alpha);
        }
        self.painted = Some((sw, sh));
        self.dirty = false;
    }
}

/// Moves and resizes a window to `(x, y, w, h)` physical pixels relative to the monitor.
fn set_frame(window: &Window, geom: Geom, x: i32, y: i32, w: i32, h: i32) {
    // `geom.origin` is in points on macOS and pixels on Windows (the units xcap reports).
    #[cfg(target_os = "macos")]
    {
        let (px, py) = (geom.origin.0 as f64 + x as f64 / geom.scale, geom.origin.1 as f64 + y as f64 / geom.scale);
        let (pw, ph) = (w as f64 / geom.scale, h as f64 / geom.scale);
        if !platform::set_frame_points(window, px, py, pw, ph) {
            let _ = window.request_inner_size(LogicalSize::new(pw, ph));
            window.set_outer_position(LogicalPosition::new(px, py));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.request_inner_size(PhysicalSize::new(w as u32, h as u32));
        window.set_outer_position(PhysicalPosition::new(geom.origin.0 + x, geom.origin.1 + y));
    }
}

/// Copies an image `sw` pixels wide (colour and opacity) into a `dw` x `dh` one, top-left aligned.
/// Anything the source does not cover is see-through. Borrowed as-is when the sizes match (the
/// usual case).
fn fit<'a>(px: &'a [u32], alpha: &'a [u8], sw: u32, dw: u32, dh: u32) -> (Cow<'a, [u32]>, Cow<'a, [u8]>) {
    let sh = (px.len() as u32).checked_div(sw).unwrap_or(0);
    if (sw, sh) == (dw, dh) {
        return (Cow::Borrowed(px), Cow::Borrowed(alpha));
    }
    let mut out_px = vec![0u32; (dw * dh) as usize];
    let mut out_a = vec![0u8; (dw * dh) as usize];
    for y in 0..dh.min(sh) {
        for x in 0..dw.min(sw) {
            let (s, d) = ((y * sw + x) as usize, (y * dw + x) as usize);
            out_px[d] = px[s];
            out_a[d] = alpha[s];
        }
    }
    (Cow::Owned(out_px), Cow::Owned(out_a))
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn a_picture_fits_a_window_of_another_size() {
        let (px, a) = (vec![7u32; 6], vec![255u8; 6]); // 3 x 2
        let (bp, ba) = fit(&px, &a, 3, 4, 3); // bigger window
        assert_eq!(ba, vec![255, 255, 255, 0, 255, 255, 255, 0, 0, 0, 0, 0]);
        assert_eq!(bp[0], 7);
        let (_, sa) = fit(&px, &a, 3, 2, 1); // smaller window
        assert_eq!(sa, vec![255, 255]);
        let (_, same) = fit(&px, &a, 3, 3, 2);
        assert!(matches!(same, std::borrow::Cow::Borrowed(_)));
        assert_eq!(*same, a[..]);
    }
}
