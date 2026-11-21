//! Small native helpers that winit does not offer.

use winit::window::Window;

#[cfg(target_os = "macos")]
fn ns_view(window: &Window) -> Option<&objc2_app_kit::NSView> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    // SAFETY: winit gives us a valid NSView pointer that lives as long as the window.
    Some(unsafe { h.ns_view.cast::<objc2_app_kit::NSView>().as_ref() })
}

/// Makes the whole window see-through by `alpha` (0.0 invisible .. 1.0 solid).
/// Returns `false` if this platform is not supported yet.
#[cfg(target_os = "macos")]
pub fn set_window_alpha(window: &Window, alpha: f64) -> bool {
    match ns_view(window).and_then(|v| v.window()) {
        Some(w) => {
            w.setAlphaValue(alpha);
            true
        }
        None => false,
    }
}

/// Clips the window to rounded corners (radius in points). Used for the pill-shaped toolbar.
#[cfg(target_os = "macos")]
pub fn round_window(window: &Window, radius: f64) {
    use objc2_app_kit::NSColor;

    let Some(view) = ns_view(window) else { return };
    if let Some(w) = view.window() {
        w.setOpaque(false);
        w.setBackgroundColor(Some(&NSColor::clearColor()));
    }
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(true);
    }
}

#[cfg(target_os = "windows")]
fn hwnd(window: &Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return None };
    Some(h.hwnd.get() as windows_sys::Win32::Foundation::HWND)
}

/// Adds extended window styles. winit rewrites them whenever its own flags change (showing,
/// hiding, hit-testing), so callers set them again after that (see `after_show`).
#[cfg(target_os = "windows")]
fn add_ex_style(hwnd: windows_sys::Win32::Foundation::HWND, bits: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE};
    // SAFETY: a valid window handle owned by this thread.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex & bits as isize != bits as isize {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | bits as isize);
        }
    }
}

/// Windows: a layered window with one opacity for the whole window.
#[cfg(target_os = "windows")]
pub fn set_window_alpha(window: &Window, alpha: f64) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA, WS_EX_LAYERED};

    let Some(h) = hwnd(window) else { return false };
    add_ex_style(h, WS_EX_LAYERED);
    // A window at opacity 0 stops taking the mouse; the invisible grab windows need it.
    let a = if alpha > 0.0 { (alpha * 255.0).round().clamp(1.0, 255.0) as u8 } else { 0 };
    // SAFETY: a valid layered window handle.
    unsafe { SetLayeredWindowAttributes(h, 0, a, LWA_ALPHA) != 0 }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn set_window_alpha(_window: &Window, _alpha: f64) -> bool {
    false
}

/// Windows: clips the window to a pill / rounded rectangle with a window region (hard edged:
/// regions have no anti-aliasing).
#[cfg(target_os = "windows")]
pub fn round_window(window: &Window, radius: f64) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};

    let Some(h) = hwnd(window) else { return };
    let size = window.inner_size();
    let d = (2.0 * radius * window.scale_factor()).round() as i32;
    // SAFETY: the system owns the region after SetWindowRgn succeeds.
    unsafe {
        let rgn = CreateRoundRectRgn(0, 0, size.width as i32 + 1, size.height as i32 + 1, d, d);
        if !rgn.is_null() {
            SetWindowRgn(h, rgn, 1);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn round_window(_window: &Window, _radius: f64) {}

/// Keeps a window from taking activation when clicked, so the app being captured stays the
/// active one (macOS gets the same by handing activation back, see `return_focus`).
#[cfg(target_os = "windows")]
pub fn no_activate(window: &Window) {
    use windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE;
    if let Some(h) = hwnd(window) {
        add_ex_style(h, WS_EX_NOACTIVATE);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn no_activate(_window: &Window) {}

/// Window attributes every overlay window shares: borderless, always on top, created hidden and
/// without taking activation. Windows: no taskbar button.
pub fn overlay_attrs() -> winit::window::WindowAttributes {
    let attrs = Window::default_attributes()
        .with_title("Scotshreen")
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
        .with_active(false)
        .with_visible(false);
    #[cfg(target_os = "windows")]
    let attrs = {
        use winit::platform::windows::WindowAttributesExtWindows;
        attrs.with_skip_taskbar(true)
    };
    attrs
}

/// Call after showing an overlay window. macOS: showing can turn the drop shadow back on.
/// Windows: showing makes winit rewrite the extended styles, so the opacity (`alpha`) and
/// no-activate style are set again.
pub fn after_show(window: &Window, alpha: Option<f64>, no_activate_too: bool) {
    no_shadow(window);
    #[cfg(target_os = "windows")]
    {
        if let Some(a) = alpha {
            set_window_alpha(window, a);
        }
        if no_activate_too {
            no_activate(window);
        }
    }
    let _ = (alpha, no_activate_too);
}

/// A see-through glass backdrop behind a window's content. On macOS 26 this is the system
/// "Liquid Glass" (`NSGlassEffectView`); on older systems it falls back to a blurred
/// `NSVisualEffectView`. The content is a transparent view that shows an RGBA image on top.
pub struct Glass {
    #[cfg(target_os = "macos")]
    content: objc2::rc::Retained<objc2::runtime::AnyObject>,
    /// The pill's own glass (macOS 26 only), tinted with the theme colour so it keeps its tone
    /// over any background.
    #[cfg(target_os = "macos")]
    main: Option<objc2::rc::Retained<objc2::runtime::AnyObject>>,
}

#[cfg(target_os = "macos")]
impl Glass {
    /// Adds the glass (rounded by `radius` points) to the window's view.
    ///
    /// macOS 26 layout: an `NSGlassEffectContainerView` holding the pill's glass view, with the
    /// content image (icons, text, the blue Capture button) inside it.
    pub fn install(window: &Window, radius: f64) -> Option<Glass> {
        use objc2::rc::Retained;
        use objc2::runtime::{AnyClass, AnyObject, Bool};
        use objc2::{class, msg_send};
        use objc2_app_kit::NSColor;

        let view = ns_view(window)?;
        if let Some(w) = view.window() {
            w.setOpaque(false);
            // A nearly invisible fill, not fully clear: macOS lets clicks fall through the fully
            // transparent pixels of a borderless window, and the glass itself does not count.
            w.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.004)));
            // Setting this explicitly to NO makes the whole window take the mouse, transparent
            // or not.
            w.setIgnoresMouseEvents(false);
        }
        let bounds = view.bounds();
        // SAFETY: plain AppKit calls on the main thread; every object is retained by its parent
        // view (or by `Glass`) after the calls.
        unsafe {
            let new_view = |cls: &AnyClass| -> Retained<AnyObject> {
                let v: Retained<AnyObject> = msg_send![cls, new];
                let _: () = msg_send![&*v, setFrame: bounds];
                let _: () = msg_send![&*v, setAutoresizingMask: 18usize]; // width | height
                v
            };
            let content = new_view(class!(NSView));
            let _: () = msg_send![&*content, setWantsLayer: Bool::YES];

            let mut main_glass: Option<Retained<AnyObject>> = None;
            if let Some(glass_cls) = AnyClass::get(c"NSGlassEffectView") {
                let host = new_view(class!(NSView));
                let main = new_view(glass_cls);
                let _: () = msg_send![&*main, setCornerRadius: radius];
                // 0 = regular, 1 = clear. SCOTSHREEN_GLASS=clear to compare.
                let style: isize = if std::env::var("SCOTSHREEN_GLASS").as_deref() == Ok("clear") { 1 } else { 0 };
                let ok: Bool = msg_send![&*main, respondsToSelector: objc2::sel!(setStyle:)];
                if ok.as_bool() {
                    let _: () = msg_send![&*main, setStyle: style];
                }
                // Glass draws above plain sibling views, so it shows its own content through
                // `contentView`.
                let _: () = msg_send![&*main, setContentView: &*content];
                let _: () = msg_send![&*host, addSubview: &*main];
                main_glass = Some(main);

                let root: Retained<AnyObject> = match AnyClass::get(c"NSGlassEffectContainerView") {
                    Some(cls) => {
                        let c = new_view(cls);
                        let _: () = msg_send![&*c, setContentView: &*host];
                        c
                    }
                    None => host,
                };
                pass_mouse_through(&root);
                let _: () = msg_send![view, addSubview: &*root];
            } else {
                let cls = AnyClass::get(c"NSVisualEffectView")?;
                let g = new_view(cls);
                let _: () = msg_send![&*g, setMaterial: 13isize]; // HUD window
                let _: () = msg_send![&*g, setBlendingMode: 0isize]; // behind window
                let _: () = msg_send![&*g, setState: 1isize]; // always active
                let _: () = msg_send![&*g, addSubview: &*content];
                pass_mouse_through(&g);
                let _: () = msg_send![view, addSubview: &*g];
            }
            Some(Glass { content, main: main_glass })
        }
    }

    /// Tints the pill's glass with `rgb` at `alpha`. Without a tint, the glass follows what is
    /// behind it and turns white over white pages, hiding light icons.
    pub fn set_base_tint(&self, rgb: [u8; 3], alpha: f64) {
        use objc2::msg_send;
        use objc2_app_kit::NSColor;

        let Some(m) = &self.main else { return };
        let c = |v: u8| v as f64 / 255.0;
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(c(rgb[0]), c(rgb[1]), c(rgb[2]), alpha);
        // SAFETY: plain AppKit setter on the main thread.
        unsafe {
            let _: () = msg_send![&**m, setTintColor: &*color];
        }
    }

    /// Shows a premultiplied RGBA image (`w` x `h` physical pixels) on top of the glass.
    pub fn set_image(&self, w: u32, h: u32, rgba: &[u8], scale: f64) {
        show_image(&self.content, w, h, rgba, scale);
    }
}

/// A `w` x `h` premultiplied RGBA image that owns `rgba`.
#[cfg(target_os = "macos")]
fn cg_image(w: u32, h: u32, rgba: Vec<u8>) -> core_graphics::image::CGImage {
    use core_graphics::base::{kCGImageAlphaPremultipliedLast, kCGRenderingIntentDefault};
    use core_graphics::color_space::CGColorSpace;
    use core_graphics::data_provider::CGDataProvider;
    use core_graphics::image::CGImage;

    let provider = CGDataProvider::from_buffer(std::sync::Arc::new(rgba));
    CGImage::new(
        w as usize,
        h as usize,
        8,
        32,
        w as usize * 4,
        &CGColorSpace::create_device_rgb(),
        kCGImageAlphaPremultipliedLast,
        &provider,
        false,
        kCGRenderingIntentDefault,
    )
}

#[cfg(target_os = "macos")]
fn show_image(view: &objc2::runtime::AnyObject, w: u32, h: u32, rgba: &[u8], scale: f64) {
    use foreign_types::ForeignType;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    if w == 0 || h == 0 || rgba.len() != (w * h * 4) as usize {
        return;
    }
    let image = cg_image(w, h, rgba.to_vec());
    // SAFETY: plain CALayer calls on the main thread; the layer retains the image.
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        if let Some(layer) = layer.as_ref() {
            let _: () = msg_send![layer, setContentsScale: scale];
            let _: () = msg_send![layer, setContents: image.as_ptr() as *mut AnyObject];
        }
    }
}

/// Makes `view` and everything inside it invisible to the mouse, so clicks and hover go to the
/// window's own view (winit's) underneath. Done by switching the object to a subclass of its own
/// class whose `hitTest:` returns nil.
#[cfg(target_os = "macos")]
fn pass_mouse_through(view: &objc2::runtime::AnyObject) {
    use std::ffi::CString;

    use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
    use objc2::sel;
    use objc2_foundation::NSPoint;

    unsafe extern "C-unwind" fn hit_test(_this: *mut AnyObject, _cmd: Sel, _p: NSPoint) -> *mut AnyObject {
        std::ptr::null_mut()
    }

    let base = view.class();
    let Ok(name) = CString::new(format!("ScotshreenPassThrough_{}", base.name().to_string_lossy())) else {
        return;
    };
    let cls: &AnyClass = match AnyClass::get(&name) {
        Some(c) => c,
        None => {
            let Some(mut b) = ClassBuilder::new(&name, base) else { return };
            // SAFETY: the signature matches `-[NSView hitTest:]`.
            unsafe {
                b.add_method(
                    sel!(hitTest:),
                    hit_test as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSPoint) -> *mut AnyObject,
                );
            }
            b.register()
        }
    };
    // SAFETY: the subclass adds no ivars, so the object's layout is unchanged.
    unsafe {
        AnyObject::set_class(view, cls);
    }
}

#[cfg(not(target_os = "macos"))]
impl Glass {
    pub fn install(_window: &Window, _radius: f64) -> Option<Glass> {
        None
    }
    pub fn set_image(&self, _w: u32, _h: u32, _rgba: &[u8], _scale: f64) {}
    pub fn set_base_tint(&self, _rgb: [u8; 3], _alpha: f64) {}
}

/// Shows the window on every Space (desktop), including full-screen apps, so the frame and
/// toolbar come along when you swipe to another desktop.
#[cfg(target_os = "macos")]
pub fn on_all_spaces(window: &Window) {
    use objc2_app_kit::NSWindowCollectionBehavior as B;

    if let Some(w) = ns_view(window).and_then(|v| v.window()) {
        w.setCollectionBehavior(B::CanJoinAllSpaces | B::FullScreenAuxiliary | B::Stationary);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn on_all_spaces(_window: &Window) {}

/// Delivers mouse moves (and enter / leave) to the window even while another app is active.
/// macOS normally sends them only to the active app, and the app being captured stays active
/// while Scotshreen's toolbar and handles are used.
#[cfg(target_os = "macos")]
pub fn track_mouse_always(window: &Window) {
    use objc2::msg_send;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSRect;

    // NSTrackingMouseEnteredAndExited | NSTrackingMouseMoved | NSTrackingActiveAlways |
    // NSTrackingInVisibleRect (the area follows the view's size).
    const OPTIONS: usize = 0x01 | 0x02 | 0x80 | 0x200;
    let Some(view) = ns_view(window) else { return };
    let Some(class) = AnyClass::get(c"NSTrackingArea") else { return };
    // SAFETY: plain AppKit calls on the main thread. The view retains the tracking area; its
    // owner (the view) handles mouseMoved: / mouseEntered: / mouseExited: as winit events.
    unsafe {
        let area: Allocated<AnyObject> = msg_send![class, alloc];
        let area: Option<Retained<AnyObject>> = msg_send![
            area,
            initWithRect: NSRect::ZERO,
            options: OPTIONS,
            owner: view,
            userInfo: std::ptr::null::<AnyObject>()
        ];
        if let Some(area) = area {
            let _: () = msg_send![view, addTrackingArea: &*area];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn track_mouse_always(_window: &Window) {}

/// Turns off the drop shadow macOS draws under borderless windows. Without this, the shadows of
/// the dim strips darken the strips next to them and show up as dark bars.
#[cfg(target_os = "macos")]
pub fn no_shadow(window: &Window) {
    if let Some(w) = ns_view(window).and_then(|v| v.window()) {
        w.setHasShadow(false);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn no_shadow(_window: &Window) {}

/// Puts the window at exactly this rectangle, in points, measured from the top-left of the
/// primary screen. Used for the dim / grab strips because `request_inner_size` was leaving the
/// thin grab bands much taller than asked.
#[cfg(target_os = "macos")]
pub fn set_frame_points(window: &Window, x: f64, y: f64, w: f64, h: f64) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let Some(mtm) = MainThreadMarker::new() else { return false };
    let Some(win) = ns_view(window).and_then(|v| v.window()) else { return false };
    let Some(primary) = NSScreen::screens(mtm).firstObject() else { return false };
    let screen_h = primary.frame().size.height;
    // Cocoa measures y upward from the bottom of the primary screen.
    let frame = NSRect::new(NSPoint::new(x, screen_h - y - h), NSSize::new(w, h));
    win.setFrame_display(frame, false);
    true
}

/// Window levels above the menu bar, so macOS does not push overlay windows down below it
/// (that is what made the top dim window overlap the rectangle). The toolbar and its message
/// sit above the dim / border strips. The frame's handles get a level of their own between the
/// two, so the dark edge strips never draw over the white handles.
pub const LEVEL_STRIPS: isize = 101; // kCGPopUpMenuWindowLevel
pub const LEVEL_HANDLES: isize = 102;
pub const LEVEL_TOOLBAR: isize = 103;

#[cfg(target_os = "macos")]
pub fn set_level(window: &Window, level: isize) {
    if let Some(w) = ns_view(window).and_then(|v| v.window()) {
        w.setLevel(level);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_level(_window: &Window, _level: isize) {}

/// Shows a system cursor right now. macOS only applies a window's own cursor while that window
/// has keyboard focus, and the frame's edge windows never do (the toolbar keeps it for the
/// shortcuts), so on hover the cursor is set directly.
#[cfg(target_os = "macos")]
pub fn show_cursor(icon: winit::window::CursorIcon) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
    use winit::window::CursorIcon;

    // The diagonal ones are Apple's private cursors (the ones Finder uses); arrow if missing.
    let name = match icon {
        CursorIcon::NsResize => c"resizeUpDownCursor",
        CursorIcon::EwResize => c"resizeLeftRightCursor",
        CursorIcon::NwseResize => c"_windowResizeNorthWestSouthEastCursor",
        CursorIcon::NeswResize => c"_windowResizeNorthEastSouthWestCursor",
        CursorIcon::Crosshair => c"crosshairCursor",
        _ => c"arrowCursor",
    };
    let Some(class) = AnyClass::get(c"NSCursor") else { return };
    let mut sel = Sel::register(name);
    // SAFETY: plain class-method calls on NSCursor that take no arguments and return a cursor.
    unsafe {
        let ok: Bool = msg_send![class, respondsToSelector: sel];
        if !ok.as_bool() {
            sel = Sel::register(c"arrowCursor");
        }
        let cursor: *mut AnyObject = msg_send![class, performSelector: sel];
        if let Some(cursor) = cursor.as_ref() {
            let _: () = msg_send![cursor, set];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn show_cursor(_icon: winit::window::CursorIcon) {}

/// Clips the window to an alpha mask (`w` x `h` pixels, one byte per pixel, 0 = see-through).
/// softbuffer has no per-pixel transparency, so the readouts use this to show only their
/// outlined text, with no box around it.
#[cfg(target_os = "macos")]
pub fn set_alpha_mask(window: &Window, w: u32, h: u32, alpha: &[u8]) {
    use foreign_types::ForeignType;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_app_kit::NSColor;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    if alpha.len() != (w * h) as usize || w == 0 || h == 0 {
        return;
    }
    let Some(view) = ns_view(window) else { return };
    if let Some(win) = view.window() {
        win.setOpaque(false);
        win.setBackgroundColor(Some(&NSColor::clearColor()));
    }
    view.setWantsLayer(true);
    let Some(layer) = view.layer() else { return };

    // Only the alpha of the mask matters; premultiplied, so colour = alpha.
    let image = cg_image(w, h, alpha.iter().flat_map(|&a| [a, a, a, a]).collect());
    let bounds = view.bounds();
    // SAFETY: plain CALayer calls on the main thread; the mask layer keeps its own reference to
    // the image, and the view's layer keeps the mask.
    unsafe {
        let mask: Retained<AnyObject> = msg_send![class!(CALayer), new];
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(bounds.size.width, bounds.size.height));
        let _: () = msg_send![&*mask, setFrame: frame];
        let _: () = msg_send![&*mask, setContents: image.as_ptr() as *mut AnyObject];
        let _: () = msg_send![&*layer, setMask: &*mask];
    }
}

/// Windows: clips the window to the pixels of the mask that are at least half opaque, with a
/// window region (no soft edges: regions have no anti-aliasing).
#[cfg(target_os = "windows")]
pub fn set_alpha_mask(window: &Window, w: u32, h: u32, alpha: &[u8]) {
    use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR};

    if alpha.len() != (w * h) as usize || w == 0 || h == 0 {
        return;
    }
    let Some(hw) = hwnd(window) else { return };
    // SAFETY: GDI regions created and freed here; the system owns `rgn` after SetWindowRgn.
    unsafe {
        let rgn = CreateRectRgn(0, 0, 0, 0);
        if rgn.is_null() {
            return;
        }
        for y in 0..h as i32 {
            let row = &alpha[(y as u32 * w) as usize..][..w as usize];
            let mut x = 0usize;
            while x < row.len() {
                if row[x] < 128 {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < row.len() && row[x] >= 128 {
                    x += 1;
                }
                let run = CreateRectRgn(start as i32, y, x as i32, y + 1);
                if !run.is_null() {
                    CombineRgn(rgn, rgn, run, RGN_OR);
                    DeleteObject(run);
                }
            }
        }
        if SetWindowRgn(hw, rgn, 1) == 0 {
            DeleteObject(rgn);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn set_alpha_mask(_window: &Window, _w: u32, _h: u32, _alpha: &[u8]) {}

/// Keeps the window out of screen captures, so the frame, handles, readouts and toolbar never
/// end up in a frame and do not have to be hidden for each capture (R15). Returns `false` where
/// this is not available, and the caller hides its windows instead.
#[cfg(target_os = "macos")]
pub fn exclude_from_capture(window: &Window) -> bool {
    use objc2_app_kit::NSWindowSharingType;
    match ns_view(window).and_then(|v| v.window()) {
        Some(w) => {
            w.setSharingType(NSWindowSharingType::None);
            true
        }
        None => false,
    }
}

/// Windows 10 2004 and later: `WDA_EXCLUDEFROMCAPTURE`. If it fails (older Windows), overlay
/// windows are hidden around each capture instead (see `capture_skips_overlay`).
#[cfg(target_os = "windows")]
pub fn exclude_from_capture(window: &Window) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};

    // SAFETY: a valid window handle.
    let ok = hwnd(window).is_some_and(|h| unsafe { SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE) != 0 });
    if !ok {
        EXCLUDE_FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    ok
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn exclude_from_capture(_window: &Window) -> bool {
    EXCLUDE_FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
    false
}

/// Set once any overlay window could not be kept out of captures.
static EXCLUDE_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True if overlay windows are kept out of captures (so they need not be hidden for each one).
pub fn capture_skips_overlay() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
        && !EXCLUDE_FAILED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_os = "macos")]
thread_local! {
    /// The app that was in front before Scotshreen took over: it gets activation back.
    static FRONT_APP: std::cell::RefCell<Option<objc2::rc::Retained<objc2::runtime::AnyObject>>> =
        const { std::cell::RefCell::new(None) };
}

/// Remembers the app in front right now (unless it is Scotshreen), so `return_focus` can hand
/// activation back to it. Called before Scotshreen activates itself, and whenever the mouse
/// enters one of its windows (just before a click there can activate it).
#[cfg(target_os = "macos")]
pub fn remember_front_app() {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    let Some(class) = AnyClass::get(c"NSWorkspace") else { return };
    // SAFETY: plain NSWorkspace / NSRunningApplication getters on the main thread.
    unsafe {
        let ws: *mut AnyObject = msg_send![class, sharedWorkspace];
        let Some(ws) = ws.as_ref() else { return };
        let front: Option<Retained<AnyObject>> = msg_send![ws, frontmostApplication];
        let Some(front) = front else { return };
        let ours: Bool = msg_send![&*front, isEqual: &*current_app()];
        if !ours.as_bool() {
            FRONT_APP.with(|f| *f.borrow_mut() = Some(front));
        }
    }
}

/// This process as an `NSRunningApplication`.
#[cfg(target_os = "macos")]
fn current_app() -> objc2::rc::Retained<objc2::runtime::AnyObject> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};

    // SAFETY: class method that always returns the current process.
    unsafe {
        let class = AnyClass::get(c"NSRunningApplication").expect("NSRunningApplication");
        let app: Option<Retained<AnyObject>> = msg_send![class, currentApplication];
        app.expect("currentApplication")
    }
}

/// If Scotshreen is the active app (the area was just chosen, or one of its windows was clicked),
/// makes the remembered app active again, so the window being captured keeps its focused look
/// (coloured traffic lights) and the keyboard. Scotshreen's windows stay on top. Returns `true`
/// if activation was handed back, so a capture can wait for that window to redraw.
#[cfg(target_os = "macos")]
pub fn return_focus() -> bool {
    use objc2::msg_send;
    use objc2::runtime::Bool;

    let Some(app) = FRONT_APP.with(|f| f.borrow().clone()) else { return false };
    // SAFETY: plain NSRunningApplication calls on the main thread.
    unsafe {
        let active: Bool = msg_send![&*current_app(), isActive];
        if !active.as_bool() {
            return false;
        }
        let terminated: Bool = msg_send![&*app, isTerminated];
        if terminated.as_bool() {
            return false;
        }
        // Allowed because Scotshreen is the active app, so it can pass activation on.
        let ok: Bool = msg_send![&*app, activateWithOptions: 0usize];
        ok.as_bool()
    }
}

/// Makes Scotshreen the active app, so its windows get the keyboard (Esc) and the crosshair
/// while the area is drawn. A menu-bar app is not made active just by opening a window. The app
/// in front before is remembered and gets activation back once the area is chosen.
#[cfg(target_os = "macos")]
pub fn activate_app() {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    remember_front_app();
    let Some(class) = AnyClass::get(c"NSApplication") else { return };
    // SAFETY: plain NSApplication calls on the main thread.
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        let Some(app) = app.as_ref() else { return };
        let _: () = msg_send![app, unhideWithoutActivation];
        // `activate` is the macOS 14+ way; older systems only have the forced variant.
        let modern: Bool = msg_send![app, respondsToSelector: objc2::sel!(activate)];
        if modern.as_bool() {
            let _: () = msg_send![app, activate];
        } else {
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
        }
    }
}

/// Gives the keyboard back to the app that was in front before capture mode opened.
#[cfg(target_os = "macos")]
pub fn yield_focus() {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    let Some(class) = AnyClass::get(c"NSApplication") else { return };
    // SAFETY: plain NSApplication calls on the main thread. Hiding a menu-bar app with no windows
    // only hands activation to the next app; the menu bar icon stays.
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        if let Some(app) = app.as_ref() {
            let _: () = msg_send![app, hide: std::ptr::null::<AnyObject>()];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn activate_app() {}

#[cfg(not(target_os = "macos"))]
pub fn remember_front_app() {}

#[cfg(not(target_os = "macos"))]
pub fn return_focus() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn yield_focus() {}

// ---------- accessibility ----------

#[cfg(target_os = "macos")]
type PressHandler = Box<dyn Fn(&str) + Send + Sync>;

/// The handler VoiceOver's "press" on a toolbar button calls, with the button's id.
#[cfg(target_os = "macos")]
static PRESS: std::sync::OnceLock<PressHandler> = std::sync::OnceLock::new();

/// Sets what pressing a button through the accessibility API (VoiceOver) does. Set once.
pub fn set_press_handler(f: impl Fn(&str) + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    let _ = PRESS.set(Box::new(f));
    #[cfg(not(target_os = "macos"))]
    let _ = f;
}

/// One accessible button: id (passed to the press handler), spoken label, and its place in the
/// window (points from the left, width in points).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct AccessButton<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub x: f64,
    pub w: f64,
}

/// Accessible buttons for a window whose content is drawn as one picture (the toolbar), so
/// screen readers can name and press them.
pub struct Access {
    #[cfg(target_os = "macos")]
    elements: Vec<objc2::rc::Retained<objc2::runtime::AnyObject>>,
}

#[cfg(target_os = "macos")]
impl Access {
    pub fn install(window: &Window, buttons: &[AccessButton], height: f64) -> Option<Access> {
        use objc2::msg_send;
        use objc2::rc::Retained;
        use objc2::runtime::{AnyObject, Bool};
        use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

        let view = ns_view(window)?;
        let cls = access_class()?;
        let role = NSString::from_str("AXButton");
        let mut elements: Vec<Retained<AnyObject>> = Vec::new();
        // SAFETY: plain NSAccessibility setters on the main thread; the view keeps the elements
        // through its children array, and `Access` keeps them too.
        unsafe {
            for b in buttons {
                let el: Retained<AnyObject> = msg_send![cls, new];
                let _: () = msg_send![&*el, setAccessibilityRole: &*role];
                let _: () = msg_send![&*el, setAccessibilityLabel: &*NSString::from_str(b.label)];
                let _: () = msg_send![&*el, setAccessibilityIdentifier: &*NSString::from_str(b.id)];
                let _: () = msg_send![&*el, setAccessibilityParent: view];
                let frame = NSRect::new(NSPoint::new(b.x, 0.0), NSSize::new(b.w, height));
                let _: () = msg_send![&*el, setAccessibilityFrameInParentSpace: frame];
                let _: () = msg_send![&*el, setAccessibilityEnabled: Bool::YES];
                elements.push(el);
            }
            let array = NSArray::from_retained_slice(&elements);
            let _: () = msg_send![view, setAccessibilityChildren: &*array];
        }
        Some(Access { elements })
    }

    /// Marks button `i` as enabled or dimmed.
    pub fn set_enabled(&self, i: usize, on: bool) {
        use objc2::msg_send;
        use objc2::runtime::Bool;
        if let Some(el) = self.elements.get(i) {
            // SAFETY: plain NSAccessibility setter on the main thread.
            unsafe {
                let _: () = msg_send![&**el, setAccessibilityEnabled: Bool::new(on)];
            }
        }
    }
}

/// A subclass of `NSAccessibilityElement` whose press calls the handler with its identifier.
#[cfg(target_os = "macos")]
fn access_class() -> Option<&'static objc2::runtime::AnyClass> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
    use objc2::sel;
    use objc2_foundation::NSString;

    unsafe extern "C-unwind" fn press(this: *mut AnyObject, _cmd: Sel) -> Bool {
        // SAFETY: `this` is a live accessibility element; the getter returns an NSString.
        let id: Option<Retained<NSString>> = unsafe { msg_send![this, accessibilityIdentifier] };
        if let (Some(id), Some(handler)) = (id, PRESS.get()) {
            handler(&id.to_string());
        }
        Bool::YES
    }

    let name = c"ScotshreenAccessButton";
    if let Some(c) = AnyClass::get(name) {
        return Some(c);
    }
    let base = AnyClass::get(c"NSAccessibilityElement")?;
    let mut b = ClassBuilder::new(name, base)?;
    // SAFETY: the signature matches `-[NSAccessibilityElement accessibilityPerformPress]`.
    unsafe {
        b.add_method(
            sel!(accessibilityPerformPress),
            press as unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool,
        );
    }
    Some(b.register())
}

/// Not available on this platform yet.
#[cfg(not(target_os = "macos"))]
impl Access {
    pub fn install(_window: &Window, _buttons: &[AccessButton], _height: f64) -> Option<Access> {
        None
    }
    pub fn set_enabled(&self, _i: usize, _on: bool) {}
}
