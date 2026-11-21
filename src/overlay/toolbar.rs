//! The toolbar: a compact pill with
//! ⠿ | ◉ Capture  ↶ (with the frame count under it)  magnet | ⧉ ⤓ | ✕
//!
//! Drawn with `tiny-skia` (vector icons) and `fontdue` (text) into a `softbuffer` window.
//! The pill shape comes from the window itself (rounded by the OS), because softbuffer has
//! no per-pixel transparency.
//!
//! It opens at the top centre of the screen and can be dragged anywhere. Hovering a button for a moment shows a tooltip with its shortcut.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fontdue::Font;
use softbuffer::{Context, Surface};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform,
};
#[cfg(target_os = "macos")]
use winit::dpi::{LogicalPosition, LogicalSize};
#[cfg(not(target_os = "macos"))]
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Theme, Window, WindowId};

use super::select::{Geom, Rect};
use super::toast::Spot;
use crate::platform;

/// What the user asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Capture,
    Undo,
    Copy,
    Save,
    Cancel,
    /// Turn snapping to lines on the screen on or off.
    ToggleMagnet,
}

impl Action {
    /// The id used for the toolbar's accessible buttons.
    pub fn id(self) -> &'static str {
        match self {
            Action::Capture => "capture",
            Action::Undo => "undo",
            Action::Copy => "copy",
            Action::Save => "save",
            Action::Cancel => "cancel",
            Action::ToggleMagnet => "magnet",
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        [Action::Capture, Action::Undo, Action::Copy, Action::Save, Action::Cancel, Action::ToggleMagnet]
            .into_iter()
            .find(|a| a.id() == id)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Handle,
    Capture,
    Undo,
    Magnet,
    Copy,
    Save,
    Cancel,
}

struct Item {
    kind: Kind,
    x: f32,
    w: f32,
    sep_before: bool,
}

struct Palette {
    bg: [u8; 3],
    fg: [u8; 3],
    hover: ([u8; 3], f32), // colour and strength
    accent: [u8; 3],
    accent_hover: [u8; 3],
    accent_pressed: [u8; 3],
    line: [u8; 3],
}

const DARK: Palette = Palette {
    bg: [34, 34, 38],
    fg: [236, 236, 240],
    hover: ([255, 255, 255], 0.14),
    accent: [255, 127, 26],
    accent_hover: [255, 152, 64],
    accent_pressed: [214, 100, 10],
    line: [72, 72, 78],
};
const LIGHT: Palette = Palette {
    bg: [246, 246, 248],
    fg: [28, 28, 32],
    hover: ([0, 0, 0], 0.09),
    accent: [255, 127, 26],
    accent_hover: [255, 152, 64],
    accent_pressed: [214, 100, 10],
    line: [208, 208, 214],
};

/// Toolbar height in points / logical pixels.
const HEIGHT: f32 = 40.0;
pub(super) const GLASS_TINT: f64 = 0.1;
/// Space kept free at the top of the screen (the menu bar), in points.
const TOP_MARGIN: f32 = 48.0;
/// Width of the icon buttons, and the space around a separator line (points). The hover circle is
/// wider than the button, so neighbours' circles overlap a little: only one shows at a time.
const ICON_BTN: f32 = 28.0;
const SEP_GAP: f32 = 8.0;
/// Space between the Capture button and Undo, whose hover circle reaches 2pt past its button.
const CAPTURE_GAP: f32 = 5.0;
/// Distance between the toolbar and a message under it, in points.
const MESSAGE_GAP: f64 = 6.0;
/// How long the pointer rests on a button before its tooltip shows.
const TIP_DELAY: Duration = Duration::from_millis(600);

/// Accessible buttons, in this order (the frame count and the handle are not buttons).
const ACCESS: [(Kind, Action, &str); 6] = [
    (Kind::Capture, Action::Capture, "Capture frame"),
    (Kind::Undo, Action::Undo, "Undo last frame"),
    (Kind::Magnet, Action::ToggleMagnet, "Magnet: snap edges to lines"),
    (Kind::Copy, Action::Copy, "Copy"),
    (Kind::Save, Action::Save, "Save"),
    (Kind::Cancel, Action::Cancel, "Cancel"),
];

pub struct Toolbar {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    font: Rc<Font>,
    items: Vec<Item>,
    w: u32,
    h: u32,
    scale: f32,
    dark: bool,
    hover: Option<Kind>,
    /// When the pointer came to rest on `hover` (for the tooltip). `None` after a click.
    hover_since: Option<Instant>,
    pressed: Option<Kind>,
    frames: usize,
    magnet: bool,
    /// Copy / Save is finishing: every button is dimmed.
    busy: bool,
    mods: ModifiersState,
    glass: Option<platform::Glass>,
    access: Option<platform::Access>,
}

impl Toolbar {
    /// Opens the toolbar at the top centre of the monitor described by `geom`.
    pub fn new(el: &ActiveEventLoop, geom: Geom, magnet: bool) -> Option<Toolbar> {
        let font = super::font()?;
        let scale = geom.scale as f32;
        let (items, total) = layout(&font, scale);
        let (w, h) = (total.ceil() as u32, (HEIGHT * scale).round() as u32);
        // Top centre of the monitor; the user can drag it anywhere.
        let (px, py) = ((geom.width - w as i32) / 2, (TOP_MARGIN * scale).round() as i32);

        let attrs = platform::overlay_attrs();
        // `geom.origin` is in points on macOS and pixels on Windows (the units xcap reports).
        #[cfg(target_os = "macos")]
        let attrs = {
            let (lw, lh) = (w as f64 / geom.scale, h as f64 / geom.scale);
            attrs
                .with_position(LogicalPosition::new(
                    geom.origin.0 as f64 + px as f64 / geom.scale,
                    geom.origin.1 as f64 + py as f64 / geom.scale,
                ))
                .with_inner_size(LogicalSize::new(lw, lh))
        };
        #[cfg(not(target_os = "macos"))]
        let attrs = attrs
            .with_position(PhysicalPosition::new(geom.origin.0 + px, geom.origin.1 + py))
            .with_inner_size(PhysicalSize::new(w, h));

        let window = Rc::new(el.create_window(attrs).ok()?);
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        let context = Context::new(window.clone()).ok()?;
        let mut surface = Surface::new(&context, window.clone()).ok()?;
        surface
            .resize(NonZeroU32::new(w)?, NonZeroU32::new(h)?)
            .ok()?;

        platform::round_window(&window, (HEIGHT / 2.0) as f64);
        let glass = platform::Glass::install(&window, (HEIGHT / 2.0) as f64);
        platform::set_level(&window, platform::LEVEL_TOOLBAR);
        platform::on_all_spaces(&window);
        // No drop shadow: around the see-through pill it reads as a dark outline on white pages.
        platform::no_shadow(&window);
        // Never in a captured frame, even when dragged over the area (R15).
        platform::exclude_from_capture(&window);
        // Hover (and so clicks, which need it) still works while another app is active.
        platform::track_mouse_always(&window);
        platform::no_activate(&window);

        // Screen readers get one button per item, since the bar is drawn as one picture.
        let access_buttons: Vec<platform::AccessButton> = ACCESS
            .iter()
            .filter_map(|(kind, action, label)| {
                let item = items.iter().find(|i| i.kind == *kind)?;
                Some(platform::AccessButton {
                    id: action.id(),
                    label,
                    x: (item.x / scale) as f64,
                    w: (item.w / scale) as f64,
                })
            })
            .collect();
        let access = platform::Access::install(&window, &access_buttons, HEIGHT as f64);

        let mut t = Toolbar {
            dark: window.theme() != Some(Theme::Light),
            window,
            surface,
            font,
            items,
            w,
            h,
            scale,
            hover: None,
            hover_since: None,
            pressed: None,
            frames: 0,
            magnet,
            busy: false,
            mods: ModifiersState::empty(),
            glass,
            access,
        };
        t.render();
        t.window.set_visible(true);
        // Showing the window can turn the shadow back on (and resets styles on Windows).
        platform::after_show(&t.window, None, true);
        // No focus: the app being captured keeps it (the shortcuts are global, see hotkey.rs).
        Some(t)
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn show(&self) {
        self.window.set_visible(true);
        platform::after_show(&self.window, None, true);
    }

    /// The toolbar's place on screen, physical pixels relative to `geom`'s monitor.
    fn monitor_rect(&self, geom: Geom) -> Option<Rect> {
        let pos = self.window.outer_position().ok()?;
        let size = self.window.outer_size();
        #[cfg(target_os = "macos")]
        let (x, y) = {
            let p = pos.to_logical::<f64>(self.window.scale_factor());
            (
                ((p.x - geom.origin.0 as f64) * geom.scale) as i32,
                ((p.y - geom.origin.1 as f64) * geom.scale) as i32,
            )
        };
        #[cfg(not(target_os = "macos"))]
        let (x, y) = (pos.x - geom.origin.0, pos.y - geom.origin.1);
        Some(Rect { x, y, w: size.width as i32, h: size.height as i32 })
    }

    /// True if the toolbar could appear in a capture of `rect` (with a margin for its shadow).
    pub fn overlaps(&self, rect: Rect, geom: Geom) -> bool {
        let Some(b) = self.monitor_rect(geom) else { return true };
        let m = (24.0 * geom.scale) as i32;
        b.x - m < rect.x + rect.w && b.x + b.w + m > rect.x && b.y - m < rect.y + rect.h && b.y + b.h + m > rect.y
    }

    /// True if `p` (physical pixels, monitor-relative) is on the toolbar.
    pub fn contains(&self, p: (i32, i32), geom: Geom) -> bool {
        self.monitor_rect(geom)
            .is_some_and(|b| p.0 >= b.x && p.0 < b.x + b.w && p.1 >= b.y && p.1 < b.y + b.h)
    }

    /// Top-left and size of the toolbar in screen units (points on macOS, pixels elsewhere).
    fn screen_box(&self) -> Option<(f64, f64, f64, f64)> {
        let pos = self.window.outer_position().ok()?;
        let size = self.window.outer_size();
        if cfg!(target_os = "macos") {
            let f = self.window.scale_factor();
            Some((pos.x as f64 / f, pos.y as f64 / f, size.width as f64 / f, size.height as f64 / f))
        } else {
            Some((pos.x as f64, pos.y as f64, size.width as f64, size.height as f64))
        }
    }

    fn units_per_px(&self) -> f64 {
        if cfg!(target_os = "macos") { 1.0 / self.scale as f64 } else { 1.0 }
    }

    /// Where a message goes: centred under the toolbar.
    pub fn anchor(&self) -> Option<Spot> {
        let (x, y, w, h) = self.screen_box()?;
        let gap = MESSAGE_GAP * self.scale as f64 * self.units_per_px();
        Some(Spot { cx: x + w / 2.0, top: y + h + gap, scale: self.scale as f64, dark: self.dark })
    }

    /// The tooltip to show now, if the pointer has rested on a button long enough: its text and
    /// where it goes (under that button).
    pub fn tip(&self, now: Instant) -> Option<(String, Spot)> {
        let kind = self.hover?;
        if now < self.hover_since? + TIP_DELAY {
            return None;
        }
        let text = self.tip_text(kind)?;
        let item = self.items.iter().find(|i| i.kind == kind)?;
        let (x, y, _, h) = self.screen_box()?;
        let u = self.units_per_px();
        let gap = MESSAGE_GAP * self.scale as f64 * u;
        let cx = x + (item.x + item.w / 2.0) as f64 * u;
        Some((text, Spot { cx, top: y + h + gap, scale: self.scale as f64, dark: self.dark }))
    }

    /// When a tooltip is due, so the event loop wakes up for it.
    pub fn tip_deadline(&self) -> Option<Instant> {
        self.hover?;
        Some(self.hover_since? + TIP_DELAY)
    }

    fn tip_text(&self, kind: Kind) -> Option<String> {
        let cmd = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
        Some(match kind {
            Kind::Handle => "Drag to move".into(),
            Kind::Capture => "Capture frame (Space)".into(),
            Kind::Undo => format!("Undo last frame ({cmd}+Z)"),
            Kind::Magnet if self.magnet => "Magnet on: edges snap to lines (M while drawing)".into(),
            Kind::Magnet => "Magnet off: edges move freely (M while drawing)".into(),
            Kind::Copy => "Copy (Enter)".into(),
            Kind::Save => format!("Save to Pictures/Scotshreen ({cmd}+S)"),
            Kind::Cancel => "Cancel (Esc)".into(),
        })
    }

    pub fn set_frames(&mut self, n: usize) {
        self.frames = n;
        self.window.request_redraw();
    }

    /// Shows the magnet button as on or off.
    pub fn set_magnet(&mut self, on: bool) {
        self.magnet = on;
        // Drawn now, not on the next redraw, so the button changes the moment it is clicked.
        self.render();
    }

    /// Dims every button while Copy / Save finishes and capture mode is about to close.
    pub fn set_busy(&mut self, busy: bool) {
        if self.busy != busy {
            self.busy = busy;
            self.render();
        }
    }

    fn enabled(&self, kind: Kind) -> bool {
        if self.busy {
            return false;
        }
        match kind {
            Kind::Undo | Kind::Copy | Kind::Save => self.frames > 0,
            Kind::Capture | Kind::Cancel | Kind::Magnet => true,
            Kind::Handle => false,
        }
    }

    fn action_of(kind: Kind) -> Option<Action> {
        match kind {
            Kind::Capture => Some(Action::Capture),
            Kind::Undo => Some(Action::Undo),
            Kind::Copy => Some(Action::Copy),
            Kind::Save => Some(Action::Save),
            Kind::Cancel => Some(Action::Cancel),
            Kind::Magnet => Some(Action::ToggleMagnet),
            Kind::Handle => None,
        }
    }

    fn item_at(&self, x: f64) -> Option<Kind> {
        let x = x as f32;
        self.items.iter().find(|i| x >= i.x && x < i.x + i.w).map(|i| i.kind)
    }

    pub fn handle(&mut self, event: WindowEvent) -> Option<Action> {
        match event {
            WindowEvent::CloseRequested => return Some(Action::Cancel),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::ThemeChanged(theme) => {
                self.dark = theme != Theme::Light;
                self.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let hover = self.item_at(position.x);
                if hover != self.hover {
                    self.hover = hover;
                    self.hover_since = Some(Instant::now());
                    self.window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.hover = None;
                self.hover_since = None;
                self.window.request_redraw();
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    // No tooltip once the button is used (until the pointer moves to another).
                    self.hover_since = None;
                    if self.hover == Some(Kind::Handle) {
                        let _ = self.window.drag_window();
                    } else {
                        self.pressed = self.hover;
                    }
                    self.window.request_redraw();
                }
                ElementState::Released => {
                    let clicked = self.pressed.take().filter(|k| Some(*k) == self.hover);
                    self.window.request_redraw();
                    if let Some(kind) = clicked
                        && self.enabled(kind)
                    {
                        return Self::action_of(kind);
                    }
                }
            },
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                let command = if cfg!(target_os = "macos") {
                    self.mods.super_key()
                } else {
                    self.mods.control_key()
                };
                // Undo / Copy / Save without frames are ignored by the caller (`App::on_action`),
                // which checks every action whatever its source.
                return match &event.logical_key {
                    Key::Named(NamedKey::Escape) => Some(Action::Cancel),
                    Key::Named(NamedKey::Space) => Some(Action::Capture),
                    Key::Named(NamedKey::Enter) => Some(Action::Copy),
                    Key::Character(c) if command && c.eq_ignore_ascii_case("s") => Some(Action::Save),
                    Key::Character(c) if command && c.eq_ignore_ascii_case("z") => Some(Action::Undo),
                    Key::Character(c) if !command && c.eq_ignore_ascii_case("m") => Some(Action::ToggleMagnet),
                    _ => None,
                };
            }
            _ => {}
        }
        None
    }

    // ---------- drawing ----------

    fn render(&mut self) {
        let Some(mut pm) = Pixmap::new(self.w, self.h) else { return };
        let pal = if self.dark { &DARK } else { &LIGHT };
        let s = self.scale;
        let h = self.h as f32;
        let glass = self.glass.is_some();
        // On glass the system draws the backdrop and the pixmap stays transparent.
        let line_a = if glass { 0.55 } else { 1.0 };
        if !glass {
            pm.fill(rgb(pal.bg, 1.0));
        }
        // Keep the glass in the theme's tone, so it does not turn white over white pages.
        if let Some(g) = &self.glass {
            g.set_base_tint(pal.bg, GLASS_TINT);
        }

        // Thin outline so the pill reads on both light and dark backgrounds.
        // (On glass the system draws its own edge highlight, like the Dock.)
        if !glass && let Some(p) = rrect(0.5, 0.5, self.w as f32 - 1.0, h - 1.0, h / 2.0) {
            stroke(&mut pm, &p, pal.line, 1.0, 1.0);
        }

        let (btn_y, btn_h) = (4.0 * s, h - 8.0 * s);
        for item in &self.items {
            let enabled = self.enabled(item.kind);
            let hovered = self.hover == Some(item.kind) && enabled;
            let pressed = self.pressed == Some(item.kind) && hovered;
            let cy = h / 2.0;
            let labelled = item.kind == Kind::Capture;

            if item.sep_before {
                let x = item.x - SEP_GAP / 2.0 * s;
                if let Some(p) = line(&[(x, 11.0 * s), (x, h - 11.0 * s)]) {
                    stroke(&mut pm, &p, pal.line, 1.0, line_a);
                }
            }

            // Button background. The Capture button is a solid accent pill drawn here, not tinted
            // system glass: the toolbar never has focus (the app being captured keeps it), and
            // glass in a window without focus turns grey.
            let bg = if item.kind == Kind::Capture {
                let c = if pressed {
                    pal.accent_pressed
                } else if hovered {
                    pal.accent_hover
                } else {
                    pal.accent
                };
                Some((c, if enabled { 1.0 } else { 0.45 }))
            } else if hovered {
                Some((pal.hover.0, pal.hover.1 * if pressed { 1.7 } else { 1.0 }))
            } else {
                None
            };
            if let Some((c, a)) = bg {
                // Icon buttons get a round hover as wide as the bar is tall, which reaches a little
                // past the button into the gap. Buttons with a label get a pill as wide as they are.
                let (bx, bw) = if labelled {
                    (item.x, item.w)
                } else {
                    (item.x + item.w / 2.0 - btn_h / 2.0, btn_h)
                };
                if let Some(p) = rrect(bx, btn_y, bw, btn_h, btn_h / 2.0) {
                    fill(&mut pm, &p, c, a);
                }
            }

            let fg = match item.kind {
                Kind::Capture => [255, 255, 255],
                Kind::Magnet if self.magnet => pal.accent,
                _ => pal.fg,
            };
            let alpha = if enabled || item.kind == Kind::Handle { 1.0 } else { 0.32 };
            let px = 14.0 * s;
            let icon = 16.0 * s;
            match item.kind {
                Kind::Handle => {
                    let cx = item.x + item.w / 2.0;
                    for col in [-1.0, 1.0] {
                        for row in [-1.0, 0.0, 1.0] {
                            if let Some(p) =
                                PathBuilder::from_circle(cx + col * 3.0 * s, cy + row * 5.0 * s, 1.3 * s)
                            {
                                fill(&mut pm, &p, pal.fg, 0.45);
                            }
                        }
                    }
                }
                Kind::Capture => {
                    let a = if self.busy { 0.6 } else { 1.0 };
                    let ix = item.x + 12.0 * s + icon / 2.0;
                    draw_icon(&mut pm, Kind::Capture, false, ix, cy, icon, fg, a);
                    draw_text(&mut pm, &self.font, "Capture", px, ix + icon / 2.0 + 6.0 * s, cy, fg, a);
                }
                Kind::Magnet => {
                    // Off: faded, like a disabled button, but still clickable.
                    let a = if self.magnet { alpha } else { alpha * 0.55 };
                    draw_icon(&mut pm, Kind::Magnet, self.magnet, item.x + item.w / 2.0, cy, icon, fg, a);
                }
                Kind::Undo => {
                    // The icon sits a little high, with the frame count small underneath it.
                    let cx = item.x + item.w / 2.0;
                    draw_icon(&mut pm, Kind::Undo, false, cx, cy - 3.0 * s, icon, fg, alpha);
                    let text = self.frames.to_string();
                    let npx = 10.0 * s;
                    let tw = text_width(&self.font, &text, npx);
                    draw_text(&mut pm, &self.font, &text, npx, cx - tw / 2.0, cy + 7.5 * s, fg, alpha);
                }
                Kind::Copy | Kind::Save | Kind::Cancel => {
                    draw_icon(&mut pm, item.kind, false, item.x + item.w / 2.0, cy, icon, fg, alpha);
                }
            }
        }

        if let Some(access) = &self.access {
            for (i, (kind, _, _)) in ACCESS.iter().enumerate() {
                access.set_enabled(i, self.enabled(*kind));
            }
        }

        if let Some(g) = &self.glass {
            g.set_image(self.w, self.h, pm.data(), self.scale as f64);
        } else {
            present(&mut self.surface, &pm);
        }
    }
}

/// Copies an opaque pixmap into a softbuffer window (0x00RRGGBB per pixel) and shows it.
pub(super) fn present(surface: &mut Surface<Rc<Window>, Rc<Window>>, pm: &Pixmap) -> Option<()> {
    let mut buffer = surface.buffer_mut().ok()?;
    for (dst, src) in buffer.iter_mut().zip(pm.data().as_chunks::<4>().0) {
        *dst = (src[0] as u32) << 16 | (src[1] as u32) << 8 | src[2] as u32;
    }
    buffer.present().ok()
}

// ---------- layout ----------

/// Item positions (physical pixels) and total width.
fn layout(font: &Font, s: f32) -> (Vec<Item>, f32) {
    let px = 14.0 * s;
    let tw = |t: &str| text_width(font, t, px);
    // (kind, width, separator before)
    let spec: [(Kind, f32, bool); 7] = [
        (Kind::Handle, 24.0 * s, false),
        (Kind::Capture, 12.0 * s + 16.0 * s + 6.0 * s + tw("Capture") + 14.0 * s, true),
        (Kind::Undo, ICON_BTN * s, false),
        (Kind::Magnet, ICON_BTN * s, false),
        (Kind::Copy, ICON_BTN * s, true),
        (Kind::Save, ICON_BTN * s, false),
        (Kind::Cancel, ICON_BTN * s, true),
    ];
    let mut x = 6.0 * s;
    let mut items = Vec::new();
    for (i, (kind, w, sep)) in spec.into_iter().enumerate() {
        if i > 0 {
            x += if sep {
                SEP_GAP * s
            } else if kind == Kind::Undo {
                CAPTURE_GAP * s
            } else {
                0.0
            };
        }
        items.push(Item { kind, x, w, sep_before: sep });
        x += w;
    }
    (items, x + 6.0 * s)
}

// ---------- tiny-skia helpers ----------

pub(super) fn rgb(c: [u8; 3], a: f32) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub(super) fn fill(pm: &mut Pixmap, path: &Path, c: [u8; 3], a: f32) {
    let mut paint = Paint::default();
    paint.set_color(rgb(c, a));
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroke(pm: &mut Pixmap, path: &Path, c: [u8; 3], width: f32, a: f32) {
    let mut paint = Paint::default();
    paint.set_color(rgb(c, a));
    paint.anti_alias = true;
    let st = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    pm.stroke_path(path, &paint, &st, Transform::identity(), None);
}

fn line(points: &[(f32, f32)]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].0, points[0].1);
    for p in &points[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.finish()
}

pub(super) fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = r * 0.5523;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// The toolbar icons are glyphs of `assets/icons.ttf` (built by `tools/build_icon_font.py`).
/// Each glyph fills the whole em box, so an icon of size `sz` is drawn at font size `sz`.
/// Keep the code points in sync with `GLYPHS` in that script.
fn icon_char(kind: Kind, magnet_on: bool) -> Option<char> {
    Some(match kind {
        Kind::Capture => '\u{E000}',
        Kind::Undo => '\u{E001}',
        Kind::Copy => '\u{E002}',
        Kind::Save => '\u{E003}',
        Kind::Cancel => '\u{E004}',
        Kind::Magnet if magnet_on => '\u{E005}',
        Kind::Magnet => '\u{E006}',
        Kind::Handle => return None,
    })
}

/// Draws an icon centred on `(cx, cy)` in a `sz` square. `magnet_on` picks the magnet with or
/// without its bolt.
#[allow(clippy::too_many_arguments)]
fn draw_icon(pm: &mut Pixmap, kind: Kind, magnet_on: bool, cx: f32, cy: f32, sz: f32, c: [u8; 3], a: f32) {
    let (Some(ch), Some(font)) = (icon_char(kind, magnet_on), super::icon_font()) else { return };
    let (m, cov) = font.rasterize(ch, sz);
    // The em box is centred on (cx, cy): its baseline is half a box below the centre.
    let gx = (cx - sz / 2.0 + m.xmin as f32).round() as i32;
    let gy = (cy + sz / 2.0 - m.height as f32 - m.ymin as f32).round() as i32;
    blit(pm, gx, gy, m.width, &cov, c, a);
}

// ---------- text ----------

pub(super) fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// Draws `text` starting at `x`, vertically centred on `cy` (based on digit height).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_text(pm: &mut Pixmap, font: &Font, text: &str, px: f32, x: f32, cy: f32, c: [u8; 3], a: f32) {
    let baseline = cy + font.metrics('H', px).height as f32 / 2.0;
    let mut pen = x;
    for ch in text.chars() {
        let (m, cov) = font.rasterize(ch, px);
        let gx = (pen + m.xmin as f32).round() as i32;
        let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        blit(pm, gx, gy, m.width, &cov, c, a);
        pen += m.advance_width;
    }
}

/// Paints a glyph coverage bitmap (`w` wide, row-major) with its top-left at `(gx, gy)`, in colour
/// `c` at opacity `a`. The pixmap is premultiplied: this is source-over, so on an opaque
/// background it is the plain blend and on a transparent one alpha grows. Clipped to the pixmap.
#[allow(clippy::too_many_arguments)]
fn blit(pm: &mut Pixmap, gx: i32, gy: i32, w: usize, cov: &[u8], c: [u8; 3], a: f32) {
    if w == 0 {
        return;
    }
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let h = (cov.len() / w) as i32;
    let (c0, c1) = ((-gx).max(0), (pw - gx).min(w as i32));
    let (r0, r1) = ((-gy).max(0), (ph - gy).min(h));
    let data = pm.data_mut();
    for row in r0..r1 {
        let src = &cov[row as usize * w..][..w];
        for col in c0..c1 {
            let k = src[col as usize] as f32 / 255.0 * a;
            if k <= 0.0 {
                continue;
            }
            let i = (((gy + row) * pw + gx + col) * 4) as usize;
            let px = &mut data[i..i + 4];
            for ch in 0..3 {
                px[ch] = (c[ch] as f32 * k + px[ch] as f32 * (1.0 - k)) as u8;
            }
            px[3] = (255.0 * k + px[3] as f32 * (1.0 - k)) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_ids_round_trip() {
        for (_, a, _) in ACCESS {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
    }

    #[test]
    fn every_icon_glyph_draws_inside_its_box() {
        let font = super::super::icon_font().expect("icon font loads");
        let mut sheet = Pixmap::new(7 * 80, 80).unwrap();
        sheet.fill(Color::WHITE);
        let cases = [
            (Kind::Capture, false),
            (Kind::Undo, false),
            (Kind::Copy, false),
            (Kind::Save, false),
            (Kind::Cancel, false),
            (Kind::Magnet, true),
            (Kind::Magnet, false),
        ];
        for (i, (kind, on)) in cases.into_iter().enumerate() {
            let ch = icon_char(kind, on).unwrap();
            let (m, cov) = font.rasterize(ch, 64.0);
            assert!(cov.iter().any(|&c| c > 200), "{ch:?} is empty");
            assert!(m.width <= 66 && m.height <= 66, "{ch:?} is bigger than its box");
            draw_icon(&mut sheet, kind, on, i as f32 * 80.0 + 40.0, 40.0, 64.0, [0, 0, 0], 1.0);
        }
        if let Ok(dir) = std::env::var("ICON_SHEET_DIR") {
            sheet.save_png(std::path::Path::new(&dir).join("sheet.png")).unwrap();
        }
    }
}
