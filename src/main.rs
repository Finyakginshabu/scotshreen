//! Scotshreen: long screenshots. Select an area, capture it, scroll, capture again.

// Windows release builds: no console window next to the tray icon.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod capture;
mod hotkey;
mod login;
mod output;
mod overlay;
mod platform;
mod stitch;
mod tray;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use hotkey::Shortcuts;
use overlay::frame::Frame;
use overlay::select::{Blocked, Geom, Outcome, Selector};
use overlay::toast::{Spot, Toast};
use overlay::toolbar::{Action, Toolbar};
use stitch::Stitcher;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Theme, WindowId};
use winit::application::ApplicationHandler;

/// How long messages stay, and how long the border flashes after a capture.
const MESSAGE: Duration = Duration::from_millis(900);
const WARNING: Duration = Duration::from_millis(2500);
/// Messages shown when capture mode cannot open: long enough to read and act on.
const BANNER: Duration = Duration::from_millis(6000);
const FLASH: Duration = Duration::from_millis(150);
/// Messages fade out over their last moments, redrawn at about 60 Hz.
const FADE: Duration = Duration::from_millis(220);
const FADE_TICK: Duration = Duration::from_millis(16);
/// After handing activation back to the app being captured, how long to wait before a capture,
/// so its window has redrawn with its focused look (coloured traffic lights).
const REFOCUS: Duration = Duration::from_millis(200);
/// While an area is open, how often to check whether the pointer is over it (the shortcuts
/// follow the pointer, see hotkey.rs).
const POINTER_CHECK: Duration = Duration::from_millis(100);
/// Cancel with at least this many frames asks to be pressed twice.
const CONFIRM_FRAMES: usize = 2;

/// A save that did not work: why, and the pixels, so the session can go on. Not `Clone`: it
/// carries the whole image.
pub struct SaveFailed {
    pub error: String,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for SaveFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SaveFailed({})", self.error)
    }
}

/// Things that wake the event loop from outside (tray menu, hotkey, the save thread).
#[derive(Debug)]
pub enum UserEvent {
    /// The hotkey or the tray's Capture: opens capture mode, or discards the open area and starts a new one.
    Capture,
    OpenFolder,
    ToggleLogin,
    Quit,
    /// The PNG was written (its path) or could not be.
    Saved(Result<std::path::PathBuf, SaveFailed>),
    /// A toolbar shortcut, pressed while another app has the keyboard (or a screen reader's
    /// press on a toolbar button).
    Shortcut(Action),
}

/// A message and when it goes away. It fades out just before.
struct Notice {
    toast: Option<Toast>,
    until: Instant,
}

impl Notice {
    fn new(toast: Option<Toast>, how_long: Duration) -> Notice {
        Notice { toast, until: Instant::now() + how_long }
    }

    /// Fades the message if its time is nearly up. Returns when it next needs attention.
    fn tick(&self, now: Instant) -> Instant {
        let fade_from = self.until.checked_sub(FADE).unwrap_or(self.until);
        if now < fade_from {
            return fade_from;
        }
        if let Some(t) = &self.toast {
            let left = self.until.saturating_duration_since(now).as_secs_f64();
            t.set_opacity(left / FADE.as_secs_f64());
        }
        (now + FADE_TICK).min(self.until)
    }
}

/// Everything that exists while capture mode is open after the area has been chosen.
struct Session {
    frame: Frame,
    toolbar: Toolbar,
    monitor: xcap::Monitor,
    geom: Geom,
    /// Frames joined so far, top to bottom in capture order.
    stitcher: Stitcher,
    /// A message under the toolbar.
    notice: Option<Notice>,
    /// The tooltip of the toolbar button under the pointer.
    tip: Option<Toast>,
    /// When the capture flash ends.
    flash_until: Option<Instant>,
    /// A capture waiting for the captured app to look active again.
    capture_at: Option<Instant>,
    /// After Copy / Save: when capture mode closes (`None` while the PNG is still being saved).
    closing: Option<Option<Instant>>,
    /// Cancel was pressed once with several frames: a second press before this discards them.
    discard_armed: Option<Instant>,
    /// Size of the result, and whether it reached the clipboard, for the "Saved" message.
    result: (u32, u32),
    copied: bool,
}

struct App {
    // Kept alive for the whole run: dropping them removes the icon / hotkey.
    tray: Option<tray::Tray>,
    hotkey: Option<hotkey::Hotkey>,
    proxy: EventLoopProxy<UserEvent>,
    /// `Some` while the area is being drawn (Selecting state).
    selector: Option<Selector>,
    /// `Some` after the area is confirmed (Framing state). Dropping it closes all its windows.
    session: Option<Session>,
    /// A message shown outside capture mode (why it could not open).
    banner: Option<Notice>,
    /// Capture mode was open: when it closes, the keyboard goes back to the previous app.
    was_open: bool,
    /// Edges snap to lines on the screen. Kept between captures while the app runs.
    magnet: bool,
}

impl Session {
    /// Captures the area, once the captured app is the active one. If Scotshreen took activation
    /// (a click on the toolbar), it is handed back first and the capture waits for the redraw.
    fn request_capture(&mut self, el: &ActiveEventLoop) {
        if self.capture_at.is_some() {
            return;
        }
        if platform::return_focus() {
            self.capture_at = Some(Instant::now() + REFOCUS);
        } else {
            self.capture(el);
        }
    }

    /// Captures the area now and joins it to the frames so far.
    fn capture(&mut self, el: &ActiveEventLoop) {
        // The first capture fixes the width.
        if !self.frame.is_locked() {
            self.frame.lock();
        }
        let rect = self.frame.rect();

        // Where overlay windows cannot be kept out of captures, hide them for the moment.
        let hide = !platform::capture_skips_overlay();
        if hide {
            if self.toolbar.overlaps(rect, self.geom) {
                self.toolbar.hide();
            }
            self.frame.set_decor_visible(false);
            std::thread::sleep(Duration::from_millis(120));
        }
        let shot = capture::grab(&self.monitor, rect, self.geom.scale);
        if hide {
            self.frame.set_decor_visible(true);
            self.toolbar.show();
        }

        let Some(shot) = shot else {
            eprintln!("capture failed");
            if self.stitcher.frames() == 0 {
                self.frame.unlock();
            }
            self.notify(el, "Capture failed. Try again", WARNING);
            return;
        };
        let n = self.stitcher.frames() + 1;
        println!("frame {n}: {} x {} px", shot.w, shot.h);
        // Debugging aid: SCOTSHREEN_DEBUG_DIR=/some/folder saves every frame as a PNG.
        if let Ok(dir) = std::env::var("SCOTSHREEN_DEBUG_DIR") {
            let _ = std::fs::create_dir_all(&dir);
            if let Err(e) = output::save_to(format!("{dir}/frame-{n}.png"), shot.w, shot.h, &shot.rgba) {
                eprintln!("debug save failed: {e}");
            }
        }

        let joined = self.stitcher.push(&shot);
        self.frames_changed();
        self.frame.set_flash(true);
        self.flash_until = Some(Instant::now() + FLASH);
        if !joined {
            // Said now, while the user can still Undo and scroll less (R21).
            self.notify(el, "No overlap found. Undo and scroll less", WARNING);
        }
    }

    fn undo(&mut self) {
        self.stitcher.pop();
        self.frames_changed();
        // Back to no frames: the width can be changed again.
        if self.stitcher.frames() == 0 {
            self.frame.unlock();
        }
    }

    /// Shows the frame count on the toolbar and the progress next to the area.
    fn frames_changed(&mut self) {
        let n = self.stitcher.frames();
        self.toolbar.set_frames(n);
        self.frame.set_progress(self.stitcher.size().map(|(_, h)| (n, h)));
    }

    fn notify(&mut self, el: &ActiveEventLoop, text: &str, how_long: Duration) {
        // The message goes where the tooltip is.
        self.tip = None;
        let toast = self.toolbar.anchor().and_then(|spot| Toast::new(el, text, spot));
        self.notice = Some(Notice::new(toast, how_long));
    }

    /// Copies the joined image (and saves it, on a background thread), then closes.
    fn finish(&mut self, el: &ActiveEventLoop, save: bool, proxy: &EventLoopProxy<UserEvent>) {
        if self.closing.is_some() {
            return;
        }
        let Some((w, h)) = self.stitcher.size() else { return };
        self.result = (w, h);
        let gaps = self.stitcher.gaps();
        println!("{} frames -> {w} x {h} px, {gaps} gap(s)", self.stitcher.frames());
        self.toolbar.set_busy(true);

        self.copied = match output::copy(w, h, self.stitcher.rgba()) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("clipboard failed: {e}");
                false
            }
        };
        if save {
            // Encoding a tall PNG can take a moment: do it off the main thread. The pixels are
            // moved, not copied, and come back if the save fails.
            let rgba = self.stitcher.take_rgba();
            let proxy = proxy.clone();
            std::thread::spawn(move || {
                let result = match output::save(w, h, &rgba) {
                    Ok(path) => Ok(path),
                    Err(error) => Err(SaveFailed { error, rgba }),
                };
                let _ = proxy.send_event(UserEvent::Saved(result));
            });
            self.notify(el, "Saving", BANNER);
            self.closing = Some(None);
        } else {
            let text = if self.copied { format!("Copied {w} x {h} px") } else { "Copy failed".into() };
            if self.copied {
                self.close_after(el, &text, gaps);
            } else {
                // Nothing was kept: stay open so the user can try again or Save.
                self.toolbar.set_busy(false);
                self.notify(el, "Copy failed. Try again or Save", WARNING);
            }
        }
    }

    /// The PNG was written, or not.
    fn saved(&mut self, el: &ActiveEventLoop, result: Result<std::path::PathBuf, SaveFailed>) {
        match result {
            Ok(path) => {
                println!("saved {}", path.display());
                let (w, h) = self.result;
                let mut text = format!("Saved {w} x {h} px to {}", output::short_dir(&path));
                if !self.copied {
                    text.push_str(" (copy failed)");
                }
                let gaps = self.stitcher.gaps();
                self.close_after(el, &text, gaps);
            }
            Err(failed) => {
                eprintln!("save failed: {}", failed.error);
                // Keep the frames: the user can try again, or Copy.
                self.stitcher.restore_rgba(failed.rgba);
                self.closing = None;
                self.toolbar.set_busy(false);
                self.notify(el, "Save failed. Try again or Copy", WARNING);
            }
        }
    }

    /// Shows the final message and closes capture mode after it.
    fn close_after(&mut self, el: &ActiveEventLoop, text: &str, gaps: usize) {
        let (text, how_long) = if gaps > 0 {
            (format!("{text} - frames may have a gap"), WARNING)
        } else {
            (text.to_string(), MESSAGE)
        };
        self.notify(el, &text, how_long);
        self.closing = Some(Some(Instant::now() + how_long));
    }

    /// True if the pointer is over the area (or its edges) or the toolbar: the toolbar's
    /// shortcuts are taken only then.
    fn pointer_here(&self) -> bool {
        self.frame.pointer_near() || self.frame.cursor().is_some_and(|p| self.toolbar.contains(p, self.geom))
    }

    /// Shows, keeps or removes the tooltip of the toolbar button under the pointer.
    fn update_tip(&mut self, el: &ActiveEventLoop, now: Instant) {
        let want = if self.notice.is_none() && self.closing.is_none() { self.toolbar.tip(now) } else { None };
        match want {
            Some((text, _)) if self.tip.as_ref().is_some_and(|t| t.text() == text) => {}
            Some((text, spot)) => self.tip = Toast::new(el, &text, spot),
            None => self.tip = None,
        }
    }

    /// The next moment something has to happen (a message fading or ending, a flash ending, a
    /// tooltip, closing, the pointer check).
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let tip = if self.tip.is_none() { self.toolbar.tip_deadline().filter(|t| *t > now) } else { None };
        [
            self.notice.as_ref().map(|n| n.tick(now)),
            self.flash_until,
            self.capture_at,
            self.closing.flatten(),
            tip,
            Some(now + POINTER_CHECK),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

impl App {
    fn on_action(&mut self, el: &ActiveEventLoop, action: Action) {
        let Some(session) = &mut self.session else { return };
        if let Some(closing) = session.closing {
            // While saving, wait for the file (so its message shows). After that, Cancel (Esc)
            // closes right away.
            if action == Action::Cancel && closing.is_some() {
                self.session = None;
            }
            return;
        }
        // No frames yet: nothing to undo, copy or save (shortcuts arrive here unchecked).
        let needs_frames = matches!(action, Action::Undo | Action::Copy | Action::Save);
        if needs_frames && session.stitcher.frames() == 0 {
            return;
        }
        if action != Action::Cancel {
            session.discard_armed = None;
        }
        match action {
            Action::Capture => session.request_capture(el),
            Action::Undo => session.undo(),
            Action::Copy => session.finish(el, false, &self.proxy),
            Action::Save => session.finish(el, true, &self.proxy),
            Action::Cancel => {
                // Several frames: one stray Esc should not throw them away.
                let n = session.stitcher.frames();
                let armed = session.discard_armed.is_some_and(|t| Instant::now() < t);
                if n >= CONFIRM_FRAMES && !armed {
                    session.discard_armed = Some(Instant::now() + WARNING);
                    session.notify(el, &format!("Press Esc or click X again to discard {n} frames"), WARNING);
                } else {
                    self.session = None;
                }
            }
            Action::ToggleMagnet => {
                self.magnet = !session.frame.magnet_on();
                session.frame.set_magnet(self.magnet);
                session.toolbar.set_magnet(self.magnet);
            }
        }
    }

    /// Opens capture mode, or says why it cannot open.
    fn open_selector(&mut self, el: &ActiveEventLoop) {
        match Selector::new(el, self.magnet) {
            Ok(selector) => {
                self.banner = None;
                self.selector = Some(selector);
            }
            Err(Blocked::Permission) => self.show_banner(
                el,
                "Allow Screen Recording for Scotshreen in System Settings, then try again",
            ),
            Err(Blocked::Failed) => self.show_banner(el, "Could not open capture mode"),
        }
    }

    /// A message at the top centre of the screen under the mouse.
    fn show_banner(&mut self, el: &ActiveEventLoop, text: &str) {
        eprintln!("{text}");
        let dark = el.system_theme() != Some(Theme::Light);
        let spot = banner_spot(dark);
        let toast = spot.and_then(|spot| Toast::new(el, text, spot));
        self.banner = Some(Notice::new(toast, BANNER));
    }
}

/// Top centre of the monitor under the mouse, in screen units.
fn banner_spot(dark: bool) -> Option<Spot> {
    use mouse_position::mouse_position::Mouse;
    let Mouse::Position { x, y } = Mouse::get_mouse_position() else { return None };
    let m = xcap::Monitor::from_point(x, y).ok()?;
    let (ox, oy, w) = (m.x().ok()? as f64, m.y().ok()? as f64, m.width().ok()? as f64);
    let scale = m.scale_factor().ok()? as f64;
    // xcap and winit positions are both in points on macOS, pixels elsewhere.
    let unit = if cfg!(target_os = "macos") { 1.0 } else { scale };
    Some(Spot { cx: ox + w / 2.0, top: oy + 48.0 * unit, scale, dark })
}

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, _el: &ActiveEventLoop, cause: StartCause) {
        // Tray icon and hotkey must be created on the main thread after the loop starts.
        if cause == StartCause::Init {
            self.hotkey = hotkey::Hotkey::new(self.proxy.clone());
            let hotkey_ok = self.hotkey.as_ref().is_some_and(hotkey::Hotkey::main_ok);
            self.tray = Some(tray::Tray::new(self.proxy.clone(), hotkey_ok));
            // A screen reader's press on a toolbar button acts like its shortcut.
            let proxy = Mutex::new(self.proxy.clone());
            platform::set_press_handler(move |id| {
                if let (Some(action), Ok(p)) = (Action::from_id(id), proxy.lock()) {
                    let _ = p.send_event(UserEvent::Shortcut(action));
                }
            });
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let mut deadline = None;
        if let Some(b) = &self.banner {
            if now >= b.until {
                self.banner = None;
            } else {
                deadline = Some(b.tick(now));
            }
        }
        if let Some(s) = &mut self.session {
            if s.capture_at.is_some_and(|t| now >= t) {
                s.capture_at = None;
                if s.closing.is_none() {
                    s.capture(el);
                }
            }
            if s.flash_until.is_some_and(|t| now >= t) {
                s.flash_until = None;
                s.frame.set_flash(false);
            }
            if s.notice.as_ref().is_some_and(|n| now >= n.until) {
                s.notice = None;
            }
            s.update_tip(el, now);
            if s.closing.flatten().is_some_and(|t| now >= t) {
                self.session = None;
            }
        }

        // The toolbar's shortcuts are global only while an area is open, and most of them only
        // while the pointer is over it or the toolbar.
        if let Some(h) = &self.hotkey {
            h.set_shortcuts(match &self.session {
                None => Shortcuts::Off,
                Some(s) if s.pointer_here() => Shortcuts::All,
                Some(_) => Shortcuts::EscOnly,
            });
        }

        let open = self.selector.is_some() || self.session.is_some();
        if self.was_open && !open {
            // Capture mode just closed: give the keyboard back to the app the user was in.
            platform::yield_focus();
        }
        self.was_open = open;

        // Sleep until the next deadline, or until something happens: ~0% CPU while idle.
        let session = self.session.as_ref().and_then(|s| s.next_deadline(now));
        match [deadline, session].into_iter().flatten().min() {
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }

    fn resumed(&mut self, _el: &ActiveEventLoop) {}

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Capture => {
                // While an area is open, the hotkey (or the tray's Capture) throws it away
                // and opens capture mode again. Space and the toolbar take the next frame.
                if let Some(session) = &self.session {
                    // Saving: wait for the file, like every other action.
                    if session.closing.is_some() {
                        return;
                    }
                    self.session = None;
                }
                if self.selector.is_none() {
                    self.open_selector(el);
                }
            }
            UserEvent::Shortcut(action) => self.on_action(el, action),
            UserEvent::OpenFolder => tray::open_save_folder(),
            UserEvent::ToggleLogin => {
                if let Some(t) = &self.tray {
                    t.toggle_login();
                }
            }
            UserEvent::Quit => el.exit(),
            UserEvent::Saved(result) => {
                if let Some(s) = &mut self.session {
                    s.saved(el, result);
                }
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Framing state.
        if let Some(session) = &mut self.session {
            let ours = session.toolbar.window_id() == id || session.frame.owns(id);
            if !ours {
                return;
            }
            // The app being captured stays the active one. A click on a Scotshreen window
            // activates Scotshreen, so remember that app while the mouse comes over, and hand
            // activation back after the click.
            let entered = matches!(event, WindowEvent::CursorEntered { .. });
            let released = matches!(
                event,
                WindowEvent::MouseInput { state: winit::event::ElementState::Released, .. }
            );
            if entered {
                platform::remember_front_app();
            }
            let is_key = matches!(event, WindowEvent::KeyboardInput { .. } | WindowEvent::ModifiersChanged(_));
            let action = if session.toolbar.window_id() == id || is_key {
                session.toolbar.handle(event)
            } else {
                session.frame.handle(id, event);
                None
            };
            // A Capture click hands activation back itself, and waits for the redraw.
            if released && action != Some(Action::Capture) {
                platform::return_focus();
            }
            if let Some(action) = action {
                self.on_action(el, action);
            }
            return;
        }

        // Selecting state.
        let Some(selector) = &mut self.selector else { return };
        if selector.window_id() != id {
            return;
        }
        let outcome = selector.handle(event);
        self.magnet = selector.magnet_on();
        match outcome {
            Outcome::Pending => {}
            Outcome::Cancelled => self.selector = None,
            Outcome::Confirmed(mut frame, geom, monitor) => {
                // The frame already shows the area: add its resize handles and the toolbar, then
                // close the selector's mouse catcher.
                frame.make_interactive(el);
                if let Some(toolbar) = Toolbar::new(el, geom, frame.magnet_on()) {
                    self.session = Some(Session {
                        frame: *frame,
                        toolbar,
                        monitor,
                        geom,
                        // The right edge (scrollbar) is left out of the overlap search.
                        stitcher: Stitcher::new((24.0 * geom.scale) as u32),
                        notice: None,
                        tip: None,
                        flash_until: None,
                        capture_at: None,
                        closing: None,
                        discard_armed: None,
                        result: (0, 0),
                        copied: false,
                    });
                    // The area is chosen: the app being captured gets activation back, so its
                    // window looks focused in the captures.
                    platform::return_focus();
                }
                self.selector = None;
            }
        }
    }
}

fn main() {
    let mut builder = EventLoop::<UserEvent>::with_user_event();

    // macOS: menu bar only, no Dock icon.
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }

    let event_loop = builder.build().expect("failed to create event loop");
    // Sleep until something happens: no polling, ~0% CPU while idle.
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App {
        tray: None,
        hotkey: None,
        proxy: event_loop.create_proxy(),
        selector: None,
        session: None,
        banner: None,
        was_open: false,
        magnet: true,
    };
    event_loop.run_app(&mut app).expect("event loop error");
}
