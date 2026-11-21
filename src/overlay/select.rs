//! Selecting state: drag to draw the area on the live screen.
//!
//! An invisible window covers the monitor under the mouse to take the drag (it is far too faint
//! to see), and the real frame (frame.rs) is drawn while dragging, so the area looks the same
//! before and after the mouse is released. Releasing confirms it. Nothing is dimmed and no
//! screenshot is taken, so opening is quick and the screen stays live.
//!
//! With the magnet on (M toggles it), corners snap to lines on the screen (see magnet.rs).
//!
//! A click without a drag picks the window under the cursor (like Space in macOS ⌘⇧4); its
//! edges can then be adjusted like a drawn area.

use mouse_position::mouse_position::Mouse;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{CursorIcon, WindowId};

use super::frame::{Frame, GRAB_ALPHA, MIN_SIZE};
use super::magnet::Magnet;
use super::piece::Piece;
use crate::platform;

/// Why capture mode could not open.
#[derive(Debug)]
pub enum Blocked {
    /// macOS: Screen Recording is not allowed for Scotshreen. System Settings has been opened.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Permission,
    /// The screen or the mouse could not be read, or a window could not be made.
    Failed,
}

/// Result of handling one window event.
pub enum Outcome {
    Pending,
    Cancelled,
    /// The mouse was released over a real rectangle. The frame (already showing it) is handed
    /// over to the Framing state.
    Confirmed(Box<Frame>, Geom, xcap::Monitor),
}

/// Where the chosen monitor is, so later states can place windows on it.
#[derive(Clone, Copy, Debug)]
pub struct Geom {
    /// Top-left of the monitor in the units xcap reports (points on macOS, pixels on Windows).
    pub origin: (i32, i32),
    /// Size in physical pixels.
    pub width: i32,
    pub height: i32,
    pub scale: f64,
}

impl Geom {
    /// Pixels per point, as a whole number: rectangle edges snap to multiples of it.
    pub fn step(&self) -> i32 {
        (self.scale.round() as i32).max(1)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Rounds `v` to the nearest multiple of `step`.
pub fn snap(v: i32, step: i32) -> i32 {
    ((v as f64 / step as f64).round() as i32) * step
}

pub struct Selector {
    /// Invisible full-monitor window that takes the mouse and the keyboard.
    catcher: Piece,
    /// Shows the rectangle and the readouts. `None` once handed over.
    frame: Option<Frame>,
    geom: Geom,
    monitor: xcap::Monitor,
    /// Cursor in physical pixels, relative to the monitor.
    cursor: (i32, i32),
    /// Where the drag started, once the mouse is pressed.
    start: Option<(i32, i32)>,
    rect: Option<Rect>,
    /// Where the mouse went down (not snapped), to tell a click from a drag.
    press: Option<(i32, i32)>,
}

impl Selector {
    /// Opens on the monitor under the mouse, with the magnet on or off.
    pub fn new(el: &ActiveEventLoop, magnet: bool) -> Result<Selector, Blocked> {
        // Without Screen Recording permission macOS returns only the wallpaper, so stop here.
        #[cfg(target_os = "macos")]
        {
            use core_graphics::access::ScreenCaptureAccess;
            if !ScreenCaptureAccess.preflight() {
                ScreenCaptureAccess.request(); // shows the system prompt the first time
                let _ = std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
                    .spawn();
                return Err(Blocked::Permission);
            }
        }
        Self::open(el, magnet).ok_or(Blocked::Failed)
    }

    fn open(el: &ActiveEventLoop, magnet: bool) -> Option<Selector> {
        let (mx, my) = match Mouse::get_mouse_position() {
            Mouse::Position { x, y } => (x, y),
            Mouse::Error => return None,
        };
        let monitor = xcap::Monitor::from_point(mx, my).ok()?;
        let (ox, oy) = (monitor.x().ok()?, monitor.y().ok()?);
        let (mw, mh) = (monitor.width().ok()? as f64, monitor.height().ok()? as f64);
        let scale = monitor.scale_factor().ok()? as f64;
        // xcap reports points on macOS and pixels on Windows.
        #[cfg(target_os = "macos")]
        let (mw, mh, cx, cy) = (mw * scale, mh * scale, (mx - ox) as f64 * scale, (my - oy) as f64 * scale);
        #[cfg(not(target_os = "macos"))]
        let (cx, cy) = ((mx - ox) as f64, (my - oy) as f64);
        let geom = Geom { origin: (ox, oy), width: mw.round() as i32, height: mh.round() as i32, scale };

        // Become the active app first (closing capture mode hides it to hand focus back), so
        // the windows show, Esc reaches us, and the crosshair is not replaced by the cursor of
        // the app underneath.
        platform::activate_app();

        let mut catcher = Piece::new(el, 0, GRAB_ALPHA, Some(CursorIcon::Crosshair))?;
        catcher.place(geom, 0, 0, geom.width, geom.height);
        let cursor = (cx.round() as i32, cy.round() as i32);
        let magnet = Magnet::new(geom, magnet);
        let mut frame = Frame::new(el, geom, magnet)?;
        frame.follow_cursor(cursor);

        catcher.window().focus_window();
        platform::show_cursor(CursorIcon::Crosshair);

        Some(Selector { catcher, frame: Some(frame), geom, monitor, cursor, start: None, rect: None, press: None })
    }

    pub fn window_id(&self) -> WindowId {
        self.catcher.id()
    }

    pub fn magnet_on(&self) -> bool {
        self.frame.as_ref().is_some_and(|f| f.magnet.is_on())
    }

    pub fn handle(&mut self, event: WindowEvent) -> Outcome {
        match event {
            WindowEvent::CloseRequested => return Outcome::Cancelled,
            WindowEvent::CursorMoved { position, .. } => {
                // The catcher covers the monitor, so its coordinates are monitor-relative.
                self.cursor = (position.x as i32, position.y as i32);
                self.on_move();
                // Keep the crosshair even if macOS hands the cursor to the app underneath.
                platform::show_cursor(CursorIcon::Crosshair);
            }
            WindowEvent::CursorEntered { .. } => platform::show_cursor(CursorIcon::Crosshair),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    self.press = Some(self.clamped());
                    self.start = self.point();
                    self.on_move();
                }
                ElementState::Released => {
                    if let Some(out) = self.on_release() {
                        return out;
                    }
                }
            },
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                return Outcome::Cancelled;
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("m")) =>
            {
                if let Some(frame) = &mut self.frame {
                    frame.magnet.set_on(!frame.magnet.is_on());
                }
                self.on_move();
            }
            _ => {}
        }
        Outcome::Pending
    }

    fn clamped(&self) -> (i32, i32) {
        (self.cursor.0.clamp(0, self.geom.width), self.cursor.1.clamp(0, self.geom.height))
    }

    /// The cursor on whole points, snapped to a nearby line before the drag starts.
    fn point(&mut self) -> Option<(i32, i32)> {
        let step = self.geom.step();
        let (x, y) = self.clamped();
        let frame = self.frame.as_mut()?;
        Some(frame.magnet.snap_point(snap(x, step), snap(y, step)))
    }

    fn on_move(&mut self) {
        let Some((ax, ay)) = self.start else {
            if let Some(p) = self.point()
                && let Some(frame) = &mut self.frame
            {
                frame.follow_cursor(p);
            }
            return;
        };
        let step = self.geom.step();
        let (x, y) = self.clamped();
        let (x, y) = (snap(x, step), snap(y, step));
        let Some(frame) = &mut self.frame else { return };
        // The moving corner's two edges: each snaps to a line along its whole length.
        let x = frame.magnet.snap_x(x, ay, y);
        let y = frame.magnet.snap_y(y, ax, x);
        let rect = Rect { x: ax.min(x), y: ay.min(y), w: (x - ax).abs(), h: (y - ay).abs() };
        self.rect = Some(rect);
        // Which corner the drag started from: the position readout stays there.
        frame.select(rect, (ax <= x, ay <= y));
    }

    /// Confirms the rectangle as soon as the mouse is released. A click picks the window under
    /// the cursor; a tiny drag starts over.
    fn on_release(&mut self) -> Option<Outcome> {
        self.start = None;
        let rect = self.rect.take();
        let clicked = self.press.take().is_some_and(|(px, py)| {
            let (x, y) = self.clamped();
            let slop = 4 * self.geom.step();
            (x - px).abs() <= slop && (y - py).abs() <= slop
        });
        let rect = match rect {
            Some(r) if r.w >= MIN_SIZE && r.h >= MIN_SIZE => Some(r),
            _ if clicked => self.window_at(self.clamped()),
            _ => None,
        };
        match rect {
            Some(r) => {
                let mut frame = self.frame.take()?;
                frame.select(r, (true, true));
                Some(Outcome::Confirmed(Box::new(frame), self.geom, self.monitor.clone()))
            }
            None => {
                self.on_move();
                None
            }
        }
    }

    /// The frontmost window of another app under `p` (physical pixels, monitor-relative), cut
    /// to the monitor and snapped to whole points.
    fn window_at(&self, p: (i32, i32)) -> Option<Rect> {
        // System surfaces that sit over everything but are not windows anyone means to capture.
        const SKIP: [&str; 5] = ["Window Server", "Dock", "Control Center", "SystemUIServer", "Notification Center"];
        let me = std::process::id();
        let g = self.geom;
        // xcap reports windows in points on macOS, pixels elsewhere.
        let unit = if cfg!(target_os = "macos") { g.scale } else { 1.0 };
        let to_px = |v: i32, origin: i32| ((v - origin) as f64 * unit).round() as i32;
        let step = g.step();
        // Front to back: the first window that contains the point is the one on top.
        for w in xcap::Window::all().ok()? {
            if w.pid().ok() == Some(me) || w.is_minimized().unwrap_or(false) {
                continue;
            }
            if w.app_name().is_ok_and(|n| SKIP.contains(&n.as_str())) {
                continue;
            }
            let (Ok(x), Ok(y), Ok(ww), Ok(wh)) = (w.x(), w.y(), w.width(), w.height()) else { continue };
            let (l, t) = (to_px(x, g.origin.0), to_px(y, g.origin.1));
            let (r, b) = (l + (ww as f64 * unit).round() as i32, t + (wh as f64 * unit).round() as i32);
            if p.0 < l || p.0 >= r || p.1 < t || p.1 >= b {
                continue;
            }
            let (l, t) = (snap(l.max(0), step), snap(t.max(0), step));
            let (r, b) = (snap(r.min(g.width), step), snap(b.min(g.height), step));
            if r - l < MIN_SIZE || b - t < MIN_SIZE {
                return None;
            }
            return Some(Rect { x: l, y: t, w: r - l, h: b - t });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_snap_to_whole_points() {
        assert_eq!(snap(101, 2), 102);
        assert_eq!(snap(100, 2), 100);
        assert_eq!(snap(99, 2), 100);
        assert_eq!(snap(7, 1), 7);
        let g = Geom { origin: (0, 0), width: 100, height: 100, scale: 2.0 };
        assert_eq!(g.step(), 2);
    }
}
