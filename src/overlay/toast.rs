//! A small message pill: the line under the toolbar ("Copied", "Saved", the gap warning), the
//! toolbar's tooltips, and messages shown when capture mode cannot open.
//!
//! On macOS it sits on the same system glass as the toolbar. It ignores the mouse, and fades
//! out (see `set_opacity`).

use std::num::NonZeroU32;
use std::rc::Rc;

use softbuffer::{Context, Surface};
use tiny_skia::Pixmap;
#[cfg(target_os = "macos")]
use winit::dpi::{LogicalPosition, LogicalSize};
#[cfg(not(target_os = "macos"))]
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use super::toolbar::{draw_text, fill, present, rgb, rrect, text_width, GLASS_TINT};
use crate::platform;

const PX: f32 = 13.0;

/// Where a message goes: its top centre, in screen units (points on macOS, pixels elsewhere,
/// the units winit uses for positions there).
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    pub cx: f64,
    pub top: f64,
    /// Pixels per point of the screen it is on.
    pub scale: f64,
    pub dark: bool,
}

pub struct Toast {
    // Dropping the window closes it.
    window: Rc<Window>,
    // Kept alive so the surface stays valid.
    _surface: Surface<Rc<Window>, Rc<Window>>,
    _glass: Option<platform::Glass>,
    text: String,
}

impl Toast {
    /// Shows `text` with its top centre at `spot`.
    pub fn new(el: &ActiveEventLoop, text: &str, spot: Spot) -> Option<Toast> {
        let font = super::font()?;
        let scale = spot.scale;
        let s = scale as f32;
        let px = PX * s;
        let (pad, h) = (14.0 * s, 28.0 * s);
        let w = (text_width(&font, text, px) + 2.0 * pad).ceil() as u32;
        let h = h.ceil() as u32;

        let attrs = platform::overlay_attrs();
        #[cfg(target_os = "macos")]
        let attrs = {
            let (lw, lh) = (w as f64 / scale, h as f64 / scale);
            attrs
                .with_position(LogicalPosition::new(spot.cx - lw / 2.0, spot.top))
                .with_inner_size(LogicalSize::new(lw, lh))
        };
        #[cfg(not(target_os = "macos"))]
        let attrs = attrs
            .with_position(PhysicalPosition::new((spot.cx - w as f64 / 2.0).round() as i32, spot.top.round() as i32))
            .with_inner_size(PhysicalSize::new(w, h));
        let window = Rc::new(el.create_window(attrs).ok()?);
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        let context = Context::new(window.clone()).ok()?;
        let mut surface = Surface::new(&context, window.clone()).ok()?;
        surface.resize(NonZeroU32::new(w)?, NonZeroU32::new(h)?).ok()?;
        let radius = (h as f64 / scale) / 2.0;
        platform::round_window(&window, radius);
        let glass = platform::Glass::install(&window, radius);
        // After the glass, which makes the window take the mouse: clicks go to whatever is below.
        let _ = window.set_cursor_hittest(false);
        platform::set_window_alpha(&window, 1.0);
        platform::no_activate(&window);
        platform::set_level(&window, platform::LEVEL_TOOLBAR);
        platform::on_all_spaces(&window);
        platform::exclude_from_capture(&window);

        let (bg, fg) = if spot.dark { ([34, 34, 38], [236, 236, 240]) } else { ([246, 246, 248], [28, 28, 32]) };
        let mut pm = Pixmap::new(w, h)?;
        // On glass the system draws the backdrop: only the text is drawn.
        if glass.is_none() {
            pm.fill(rgb(bg, 1.0));
            if let Some(p) = rrect(0.0, 0.0, w as f32, h as f32, h as f32 / 2.0) {
                fill(&mut pm, &p, bg, 1.0);
            }
        }
        draw_text(&mut pm, &font, text, px, pad, h as f32 / 2.0, fg, 1.0);
        if let Some(g) = &glass {
            g.set_base_tint(bg, GLASS_TINT);
            g.set_image(w, h, pm.data(), scale);
        } else {
            present(&mut surface, &pm)?;
        }
        window.set_visible(true);
        platform::after_show(&window, Some(1.0), true);
        Some(Toast { window, _surface: surface, _glass: glass, text: text.to_string() })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Fades the whole message (0.0 gone .. 1.0 solid). Where window opacity is not supported
    /// it stays solid until it closes.
    pub fn set_opacity(&self, a: f64) {
        platform::set_window_alpha(&self.window, a.clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn font_has_every_glyph_the_messages_use() {
        let font = fontdue::Font::from_bytes(
            include_bytes!("../../assets/font.ttf") as &[u8],
            fontdue::FontSettings::default(),
        )
        .unwrap();
        let messages = "Copied Saved Saving failed 0123456789 x - No overlap found, frames may have \
                        a gap. Undo and scroll less px to Pictures/Scotshreen (copy failed) Capture \
                        Press Esc or click X again to discard Allow Screen Recording for \
                        Scotshreen in System Settings, then try again. Try again or Copy. \
                        Could not open capture mode Undo last frame (Cmd+Z) Ctrl+Z Space Enter \
                        Cmd+S Ctrl+S Magnet: snap edges to lines (M while drawing) Cancel Esc";
        for c in messages.chars() {
            assert!(font.lookup_glyph_index(c) != 0, "missing glyph {c:?}");
        }
    }
}
