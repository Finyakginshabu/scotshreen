//! The frame around the chosen area, over the live screen in its original colours. The same
//! frame is shown while the area is being drawn (Selecting) and after (Framing), so it looks
//! identical in both.
//!
//! Small borderless windows sit around the rectangle:
//! - four thin windows draw the white border, and four more a faint dark edge just outside it
//!   so the frame shows on white pages too (all click-through),
//! - white handles sit on the border: circles on the corners and pills on the middle of each
//!   edge. All 8 before frame 1, then only the top and bottom pills,
//! - invisible "grab" windows just outside the edges and corners (Framing only) resize the
//!   rectangle, like the handles,
//! - two readouts (see label.rs): the start corner's position and the size. Once frames are
//!   captured, the position readout shows the progress instead: frame count and total height.
//!
//! **Nothing covers the inside**, so scrolling and clicks go straight to the app underneath.
//! The rectangle cannot be moved, only resized. The first capture locks the width (R18); undoing
//! back to no frames unlocks it.
//!
//! With the magnet on, a dragged edge snaps to a line on the screen near it (see magnet.rs).

use std::rc::Rc;

use fontdue::Font;
use mouse_position::mouse_position::Mouse;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{CursorIcon, WindowId};

use super::label::{self, Readout, Side};
use super::magnet::Magnet;
use super::piece::Piece;
use super::select::{snap, Geom, Rect};
use crate::platform;

/// Opacity of invisible windows that take the mouse. Just above macOS's "fully transparent"
/// cut-off, so they still receive the mouse, but too faint to change any colour you can see.
pub(super) const GRAB_ALPHA: f64 = 0.005;
/// Thickness of the grab windows, in points.
const GRAB_PT: f64 = 10.0;
/// Handle sizes, in points: corner circles, and the pills on the middle of each edge.
const DOT_PT: f64 = 12.0;
const PILL_LEN_PT: f64 = 20.0;
const PILL_THICK_PT: f64 = 7.0;
/// Border thickness, in points.
const BORDER_PT: f64 = 2.0;
/// Colours, 0x00RRGGBB. The border briefly turns `FLASH` when a frame is captured.
const BORDER: u32 = 0x00FF_FFFF;
const FLASH: u32 = 0x00FF_7F1A;
/// A thin, see-through dark edge just outside the white border and around each handle, so the
/// frame still shows on white pages. Thickness in points, and opacity.
const EDGE_PT: f64 = 1.0;
const EDGE_ALPHA: f64 = 0.45;
/// Smallest allowed width / height, in physical pixels.
pub(super) const MIN_SIZE: i32 = 40;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role {
    BorderTop,
    BorderBottom,
    BorderLeft,
    BorderRight,
    EdgeTop,
    EdgeBottom,
    EdgeLeft,
    EdgeRight,
    GrabTop,
    GrabBottom,
    GrabLeft,
    GrabRight,
    GrabTopLeft,
    GrabTopRight,
    GrabBottomLeft,
    GrabBottomRight,
    DotTopLeft,
    DotTop,
    DotTopRight,
    DotLeft,
    DotRight,
    DotBottomLeft,
    DotBottom,
    DotBottomRight,
}

const BORDERS: [Role; 4] = [Role::BorderTop, Role::BorderBottom, Role::BorderLeft, Role::BorderRight];
const EDGES: [Role; 4] = [Role::EdgeTop, Role::EdgeBottom, Role::EdgeLeft, Role::EdgeRight];
const DOTS: [Role; 8] = [
    Role::DotTopLeft,
    Role::DotTop,
    Role::DotTopRight,
    Role::DotLeft,
    Role::DotRight,
    Role::DotBottomLeft,
    Role::DotBottom,
    Role::DotBottomRight,
];
const GRABS: [Role; 8] = [
    Role::GrabTop,
    Role::GrabBottom,
    Role::GrabLeft,
    Role::GrabRight,
    Role::GrabTopLeft,
    Role::GrabTopRight,
    Role::GrabBottomLeft,
    Role::GrabBottomRight,
];

impl Role {
    fn is_border(self) -> bool {
        BORDERS.contains(&self)
    }

    fn is_dot(self) -> bool {
        DOTS.contains(&self)
    }

    /// Still there after the rectangle is locked by the first capture.
    fn survives_lock(self) -> bool {
        !DOTS.contains(&self) && !GRABS.contains(&self)
            || matches!(self, Role::GrabTop | Role::GrabBottom | Role::DotTop | Role::DotBottom)
    }

    /// Which edges (left, top, right, bottom) dragging this window moves.
    fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Role::GrabTop | Role::DotTop => (false, true, false, false),
            Role::GrabBottom | Role::DotBottom => (false, false, false, true),
            Role::GrabLeft | Role::DotLeft => (true, false, false, false),
            Role::GrabRight | Role::DotRight => (false, false, true, false),
            Role::GrabTopLeft | Role::DotTopLeft => (true, true, false, false),
            Role::GrabTopRight | Role::DotTopRight => (false, true, true, false),
            Role::GrabBottomLeft | Role::DotBottomLeft => (true, false, false, true),
            Role::GrabBottomRight | Role::DotBottomRight => (false, false, true, true),
            _ => (false, false, false, false),
        }
    }

    fn cursor(self) -> Option<CursorIcon> {
        Some(match self.edges() {
            (false, false, false, false) => return None,
            (true, true, ..) | (_, _, true, true) => CursorIcon::NwseResize,
            (false, true, true, _) | (true, false, _, true) => CursorIcon::NeswResize,
            (true, ..) | (_, _, true, _) => CursorIcon::EwResize,
            _ => CursorIcon::NsResize,
        })
    }
}

#[derive(Clone, Copy)]
struct Drag {
    role: Role,
    /// Cursor (physical pixels, monitor-relative) and rectangle when the drag started.
    from: (i32, i32),
    start: Rect,
}

pub struct Frame {
    pieces: Vec<(Role, Piece)>,
    pos: Readout,
    size: Readout,
    font: Rc<Font>,
    geom: Geom,
    /// `None` until the first drag in the selector.
    rect: Option<Rect>,
    /// Which corner the selection drag started from: (on the left edge, on the top edge).
    anchor: (bool, bool),
    locked: bool,
    flash: bool,
    dragging: Option<Drag>,
    /// Snaps edges to lines on the screen. The selector uses it too.
    pub(super) magnet: Magnet,
    /// Frames captured and the joined image's height (physical pixels), once there are frames.
    progress: Option<(usize, u32)>,
}

impl Frame {
    /// Creates the frame's windows (hidden until there is a rectangle).
    pub fn new(el: &ActiveEventLoop, geom: Geom, magnet: Magnet) -> Option<Frame> {
        let px = |pt: f64| (pt * geom.scale).round() as i32;
        let e = px(EDGE_PT).max(1);
        let (d, len, thick) = (px(DOT_PT), px(PILL_LEN_PT), px(PILL_THICK_PT));
        let circle = handle_image(d, d, e);
        let pill_h = handle_image(len, thick, e);
        let pill_v = handle_image(thick, len, e);

        // Created bottom to top: border, dark edge, handles.
        let mut pieces = Vec::new();
        for role in BORDERS {
            pieces.push((role, Piece::new(el, BORDER, 1.0, None)?));
        }
        for role in EDGES {
            pieces.push((role, Piece::new(el, 0, EDGE_ALPHA, None)?));
        }
        for role in DOTS {
            let mut piece = Piece::new(el, 0, 1.0, role.cursor())?;
            piece.set_no_activate();
            // Above the dark edge strips: same-level windows can stack in any order.
            platform::set_level(piece.window(), platform::LEVEL_HANDLES);
            let img = match role {
                Role::DotTop | Role::DotBottom => &pill_h,
                Role::DotLeft | Role::DotRight => &pill_v,
                _ => &circle,
            };
            piece.set_image(img.2.clone(), img.3.clone(), img.0 as u32);
            pieces.push((role, piece));
        }

        Some(Frame {
            pieces,
            pos: Readout::new(el)?,
            size: Readout::new(el)?,
            font: super::font()?,
            geom,
            rect: None,
            anchor: (true, true),
            locked: false,
            flash: false,
            dragging: None,
            magnet,
            progress: None,
        })
    }

    /// Before the selection drag: no rectangle yet, only the position readout next to the cursor.
    pub fn follow_cursor(&mut self, p: (i32, i32)) {
        self.rect = None;
        for (_, piece) in &mut self.pieces {
            piece.hide();
        }
        self.size.hide();
        let text = label::position(p.0, p.1, self.geom.scale);
        self.pos.show(&self.font, text, p, Side::TopLeft, self.geom);
    }

    /// During the selection drag: the rectangle so far, and which corner the drag started from.
    pub fn select(&mut self, rect: Rect, anchor: (bool, bool)) {
        self.rect = Some(rect);
        self.anchor = anchor;
        self.relayout();
    }

    /// After the selection: adds the invisible grab windows along the edges and corners.
    pub fn make_interactive(&mut self, el: &ActiveEventLoop) {
        for role in GRABS {
            if let Some(mut piece) = Piece::new(el, 0, GRAB_ALPHA, role.cursor()) {
                piece.set_no_activate();
                self.pieces.push((role, piece));
            }
        }
        self.relayout();
    }

    /// The chosen rectangle in physical pixels, relative to the monitor.
    pub fn rect(&self) -> Rect {
        self.rect.unwrap_or(Rect { x: 0, y: 0, w: 0, h: 0 })
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.pieces.iter().any(|(_, p)| p.id() == id)
    }

    pub fn magnet_on(&self) -> bool {
        self.magnet.is_on()
    }

    pub fn set_magnet(&mut self, on: bool) {
        self.magnet.set_on(on);
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Called at the first capture: width and left/right position are fixed from now on.
    pub fn lock(&mut self) {
        self.set_locked(true);
    }

    /// Called when Undo goes back to no frames: the width can change again.
    pub fn unlock(&mut self) {
        self.set_locked(false);
    }

    fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.dragging = None;
        self.relayout();
    }

    /// Shows the progress (frame count, joined height in physical pixels) in place of the
    /// position readout, or the position again with `None`.
    pub fn set_progress(&mut self, progress: Option<(usize, u32)>) {
        if self.progress != progress {
            self.progress = progress;
            self.relayout();
        }
    }

    /// Turns the capture flash on or off (the border briefly changes colour).
    pub fn set_flash(&mut self, on: bool) {
        if self.flash != on {
            self.flash = on;
            let color = if on { FLASH } else { BORDER };
            for (_, piece) in self.pieces.iter_mut().filter(|(r, _)| r.is_border()) {
                piece.set_fill(color);
            }
            self.relayout();
        }
    }

    /// Hides or shows the handles and readouts, for platforms where overlay windows cannot be
    /// kept out of captures. They can overlap the inside of the rectangle.
    pub fn set_decor_visible(&mut self, visible: bool) {
        for (_, piece) in self.pieces.iter_mut().filter(|(r, _)| r.is_dot()) {
            piece.set_visible(visible);
        }
        self.pos.set_visible(visible);
        self.size.set_visible(visible);
    }

    /// Handles mouse events on the grab windows and the handles.
    pub fn handle(&mut self, id: WindowId, event: WindowEvent) {
        let Some(role) = self.pieces.iter().find(|(_, p)| p.id() == id).map(|(r, _)| *r) else {
            return;
        };
        let Some(icon) = role.cursor() else { return };
        match event {
            // Resize cursor on hover, anywhere along an edge or on a handle.
            WindowEvent::CursorEntered { .. } => platform::show_cursor(icon),
            WindowEvent::CursorLeft { .. } if self.dragging.is_none() => {
                platform::show_cursor(CursorIcon::Default)
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(from) = self.cursor() {
                    self.dragging = Some(Drag { role, from, start: self.rect() });
                    // The page may have scrolled since the last picture.
                    self.magnet.refresh();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.dragging = None;
            }
            WindowEvent::CursorMoved { .. } => {
                let (Some(drag), Some((x, y))) = (self.dragging, self.cursor()) else { return };
                let next = self.dragged(drag, x - drag.from.0, y - drag.from.1);
                let r = self.rect();
                if (next.x, next.y, next.w, next.h) != (r.x, r.y, r.w, r.h) {
                    self.rect = Some(next);
                    self.relayout();
                }
            }
            _ => {}
        }
    }

    /// The rectangle after moving the cursor by `(dx, dy)` since the drag started. Edges snap to
    /// whole points, so a capture lines up with the screen's pixels exactly, and then to a line
    /// on the screen near them if the magnet is on.
    fn dragged(&mut self, drag: Drag, dx: i32, dy: i32) -> Rect {
        let s = drag.start;
        let (mw, mh) = (self.geom.width, self.geom.height);
        let step = self.geom.step();
        let (el, et, er, eb) = drag.role.edges();
        let (el, er) = (el && !self.locked, er && !self.locked);
        let (mut l, mut t, mut r, mut b) = (s.x, s.y, s.x + s.w, s.y + s.h);
        if el {
            l = snap(l + dx, step);
        }
        if er {
            r = snap(r + dx, step);
        }
        if et {
            t = snap(t + dy, step);
        }
        if eb {
            b = snap(b + dy, step);
        }
        // Each moved edge snaps to a line along its whole length.
        let m = &mut self.magnet;
        if el {
            l = m.snap_x(l, t, b).clamp(0, r - MIN_SIZE);
        }
        if er {
            r = m.snap_x(r, t, b).clamp(l + MIN_SIZE, mw);
        }
        if et {
            t = m.snap_y(t, l, r).clamp(0, b - MIN_SIZE);
        }
        if eb {
            b = m.snap_y(b, l, r).clamp(t + MIN_SIZE, mh);
        }
        Rect { x: l, y: t, w: r - l, h: b - t }
    }

    /// True if the pointer is over the area or close to its border (its handles, resize edges).
    pub fn pointer_near(&self) -> bool {
        let (Some(r), Some((x, y))) = (self.rect, self.cursor()) else { return false };
        let m = (GRAB_PT * self.geom.scale).round() as i32;
        x >= r.x - m && x < r.x + r.w + m && y >= r.y - m && y < r.y + r.h + m
    }

    /// Cursor position in physical pixels, relative to the top-left of the monitor.
    pub fn cursor(&self) -> Option<(i32, i32)> {
        let Mouse::Position { x, y } = Mouse::get_mouse_position() else { return None };
        let (rx, ry) = ((x - self.geom.origin.0) as f64, (y - self.geom.origin.1) as f64);
        // The mouse position and the monitor origin are in points on macOS, pixels on Windows.
        #[cfg(target_os = "macos")]
        let (rx, ry) = (rx * self.geom.scale, ry * self.geom.scale);
        Some((rx.round() as i32, ry.round() as i32))
    }

    /// Moves every window to match the rectangle. Windows whose size and content did not change
    /// are only moved, not repainted (see piece.rs).
    fn relayout(&mut self) {
        let Some(Rect { x: l, y: t, w, h }) = self.rect else { return };
        let geom = self.geom;
        let (r, b) = (l + w, t + h);
        let px = |pt: f64| (pt * geom.scale).round() as i32;
        let th = px(BORDER_PT).max(1);
        let g = px(GRAB_PT);
        let e = px(EDGE_PT).max(1);
        let (d, len, thick) = (px(DOT_PT) + 2 * e, px(PILL_LEN_PT) + 2 * e, px(PILL_THICK_PT) + 2 * e);
        // A handle `hw` x `hh` centred on (cx, cy).
        let at = |cx: i32, cy: i32, hw: i32, hh: i32| (cx - hw / 2, cy - hh / 2, hw, hh);
        // Handles are centred on the middle of the border line, which sits just outside the area.
        let (bl, bt, br, bb) = (l - th / 2, t - th / 2, r + th / 2, b + th / 2);
        let (mx, my) = (l + w / 2, t + h / 2);

        for (role, piece) in &mut self.pieces {
            if self.locked && !role.survives_lock() {
                piece.hide();
                continue;
            }
            let (x, y, pw, ph) = match role {
                Role::BorderTop => (l - th, t - th, w + 2 * th, th),
                Role::BorderBottom => (l - th, b, w + 2 * th, th),
                Role::BorderLeft => (l - th, t, th, h),
                Role::BorderRight => (r, t, th, h),
                Role::EdgeTop => (l - th - e, t - th - e, w + 2 * (th + e), e),
                Role::EdgeBottom => (l - th - e, b + th, w + 2 * (th + e), e),
                Role::EdgeLeft => (l - th - e, t - th, e, h + 2 * th),
                Role::EdgeRight => (r + th, t - th, e, h + 2 * th),
                Role::GrabTop => (l, t - g, w, g),
                Role::GrabBottom => (l, b, w, g),
                Role::GrabLeft => (l - g, t, g, h),
                Role::GrabRight => (r, t, g, h),
                Role::GrabTopLeft => (l - g, t - g, g, g),
                Role::GrabTopRight => (r, t - g, g, g),
                Role::GrabBottomLeft => (l - g, b, g, g),
                Role::GrabBottomRight => (r, b, g, g),
                Role::DotTopLeft => at(bl, bt, d, d),
                Role::DotTopRight => at(br, bt, d, d),
                Role::DotBottomLeft => at(bl, bb, d, d),
                Role::DotBottomRight => at(br, bb, d, d),
                Role::DotTop => at(mx, bt, len, thick),
                Role::DotBottom => at(mx, bb, len, thick),
                Role::DotLeft => at(bl, my, thick, len),
                Role::DotRight => at(br, my, thick, len),
            };
            piece.place(geom, x, y, pw, ph);
        }

        // Readouts: position at the top-left of the start corner, size at the bottom-right of
        // the opposite corner (where the drag ended).
        let (al, at_top) = self.anchor;
        let start = (if al { l } else { r }, if at_top { t } else { b });
        let end = (if al { r } else { l }, if at_top { b } else { t });
        let pos = match self.progress {
            Some((n, total)) => label::progress(n, total, geom.scale),
            None => label::position(start.0, start.1, geom.scale),
        };
        self.pos.show(&self.font, pos, start, Side::TopLeft, geom);
        let size = label::size(w, h, geom.scale);
        self.size.show(&self.font, size, end, Side::BottomRight, geom);
    }
}

/// A white circle or pill `hw` x `hh` with a faint dark edge `e` around it (all pixels), as
/// (width, height, colour, opacity). The handle window is clipped to the opacity.
fn handle_image(hw: i32, hh: i32, e: i32) -> (i32, i32, Vec<u32>, Vec<u8>) {
    let (w, h) = (hw + 2 * e, hh + 2 * e);
    let (mut rgb, mut alpha) = (Vec::with_capacity((w * h) as usize), Vec::with_capacity((w * h) as usize));
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f64 + 0.5 - w as f64 / 2.0, y as f64 + 0.5 - h as f64 / 2.0);
            let (core, outer) = shape_cover(px, py, hw as f64, hh as f64, e as f64);
            let a = core + (1.0 - core) * outer * EDGE_ALPHA;
            // White over black, not premultiplied.
            let v = if a > 0.0 { (core / a * 255.0).round() as u32 } else { 0 };
            rgb.push(v << 16 | v << 8 | v);
            alpha.push((a * 255.0).round() as u8);
        }
    }
    (w, h, rgb, alpha)
}

/// How much of the pixel at `(px, py)` (relative to the centre of a `hw` x `hh` circle or pill)
/// the white shape covers, and how much the shape grown by `edge` covers.
fn shape_cover(px: f64, py: f64, hw: f64, hh: f64, edge: f64) -> (f64, f64) {
    let r = hw.min(hh) / 2.0;
    // The shape is every point within `r` of the segment through its middle.
    let (ex, ey) = ((hw / 2.0 - r).max(0.0), (hh / 2.0 - r).max(0.0));
    let (qx, qy) = (px - px.clamp(-ex, ex), py - py.clamp(-ey, ey));
    let dist = (qx * qx + qy * qy).sqrt();
    ((r - dist + 0.5).clamp(0.0, 1.0), (r + edge - dist + 0.5).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_image_is_white_inside_with_a_soft_dark_edge() {
        let (w, h, rgb, alpha) = handle_image(24, 24, 2);
        assert_eq!((w, h), (28, 28));
        let centre = (14 * w + 14) as usize;
        assert_eq!((rgb[centre], alpha[centre]), (0x00FF_FFFF, 255));
        // Corner of the square is outside the circle and its edge: see-through.
        assert_eq!(alpha[0], 0);
        // Just outside the white circle: dark and partly see-through.
        let rim = (14 * w + 1) as usize;
        assert!(alpha[rim] > 0 && alpha[rim] < 200 && rgb[rim] >> 16 < 128);
    }

    #[test]
    fn locking_keeps_only_the_top_and_bottom_handles() {
        let kept: Vec<Role> = DOTS.into_iter().chain(GRABS).filter(|r| r.survives_lock()).collect();
        assert_eq!(kept, vec![Role::DotTop, Role::DotBottom, Role::GrabTop, Role::GrabBottom]);
        assert!(BORDERS.into_iter().chain(EDGES).all(|r| r.survives_lock()));
    }

    #[test]
    fn handles_show_the_matching_resize_cursor() {
        assert_eq!(Role::DotTop.cursor(), Some(CursorIcon::NsResize));
        assert_eq!(Role::GrabRight.cursor(), Some(CursorIcon::EwResize));
        assert_eq!(Role::DotTopLeft.cursor(), Some(CursorIcon::NwseResize));
        assert_eq!(Role::GrabBottomRight.cursor(), Some(CursorIcon::NwseResize));
        assert_eq!(Role::DotTopRight.cursor(), Some(CursorIcon::NeswResize));
        assert_eq!(Role::GrabBottomLeft.cursor(), Some(CursorIcon::NeswResize));
        assert_eq!(Role::BorderTop.cursor(), None);
    }
}
