//! Menu bar / tray icon and its menu.

use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::event_loop::EventLoopProxy;

use crate::{login, UserEvent};

pub struct Tray {
    _icon: TrayIcon,
    login: CheckMenuItem,
}

impl Tray {
    /// `hotkey_ok`: the global hotkey could be registered. If not, the menu says so, since the
    /// hotkey would otherwise just silently do nothing.
    pub fn new(proxy: EventLoopProxy<UserEvent>, hotkey_ok: bool) -> Self {
        #[cfg(target_os = "macos")]
        let command = Modifiers::META;
        #[cfg(not(target_os = "macos"))]
        let command = Modifiers::CONTROL;
        // Shown next to the item, so the shortcut can be found without the docs.
        let shortcut = hotkey_ok.then(|| Accelerator::new(command | Modifiers::SHIFT, Code::Digit1));
        let capture = MenuItem::new("Capture", true, shortcut);
        let taken = MenuItem::new(
            if cfg!(target_os = "macos") {
                "Shortcut \u{2318}\u{21E7}1 is used by another app"
            } else {
                "Shortcut Ctrl+Shift+1 is used by another app"
            },
            false,
            None,
        );
        let open = MenuItem::new("Open folder", true, None);
        let login = CheckMenuItem::new("Open at login", true, login::is_enabled(), None);
        let quit = MenuItem::new("Quit", true, None);

        let menu = Menu::new();
        menu.append(&capture).expect("failed to build menu");
        if !hotkey_ok {
            menu.append(&taken).expect("failed to build menu");
        }
        menu.append(&open).expect("failed to build menu");
        if login::SUPPORTED {
            menu.append(&login).expect("failed to build menu");
        }
        menu.append_items(&[&PredefinedMenuItem::separator(), &quit])
            .expect("failed to build menu");

        let (capture_id, open_id, login_id, quit_id) =
            (capture.id().clone(), open.id().clone(), login.id().clone(), quit.id().clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let ev = if e.id == capture_id {
                UserEvent::Capture
            } else if e.id == open_id {
                UserEvent::OpenFolder
            } else if e.id == login_id {
                UserEvent::ToggleLogin
            } else if e.id == quit_id {
                UserEvent::Quit
            } else {
                return;
            };
            let _ = proxy.send_event(ev);
        }));

        let builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Scotshreen");
        // macOS: a template image adapts to the light / dark menu bar.
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(make_icon());
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(make_icon());
        let icon = builder.build().expect("failed to create tray icon");

        Tray { _icon: icon, login }
    }

    /// Turns "Open at login" on or off, and shows the result in the menu.
    pub fn toggle_login(&self) {
        let want = !login::is_enabled();
        if let Err(e) = login::set_enabled(want) {
            eprintln!("open at login: {e}");
        }
        // The menu toggles its check mark itself: show what actually happened.
        self.login.set_checked(login::is_enabled());
    }
}

/// The tray icon, rendered from `assets/scotshreen.svg`.
///
/// On macOS it is a template (only its shape counts). Elsewhere it is drawn white with a dark
/// outline, so it shows on both dark and light taskbars.
fn make_icon() -> Icon {
    const S: u32 = 64;
    // The artwork spans about 190..890 of the 1080 box. Show a window centred on it that is
    // larger than the artwork, so the icon keeps some margin and matches other menu bar icons.
    const CROP_SIZE: f32 = 900.0;
    const CROP_MIN: f32 = 540.0 - CROP_SIZE / 2.0;

    let svg = include_bytes!("../assets/scotshreen.svg");
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .expect("bad tray icon svg");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(S, S).expect("pixmap");
    let k = S as f32 / CROP_SIZE;
    let transform = resvg::tiny_skia::Transform::from_scale(k, k)
        .pre_translate(-CROP_MIN, -CROP_MIN);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Only the alpha (the shape) is used; the colour is chosen per platform below.
    let alpha: Vec<u8> = pixmap.data().as_chunks::<4>().0.iter().map(|p| p[3]).collect();
    let template = cfg!(target_os = "macos");
    let n = S as usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let i = (y * n + x) * 4;
            let a = alpha[y * n + x];
            if template {
                rgba[i + 3] = a;
            } else if a > 0 {
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, a]);
            } else {
                // A dark outline (2 px at this size) around the white shape.
                let near = (-2i32..=2).any(|dy| {
                    (-2i32..=2).any(|dx| {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        (0..n as i32).contains(&nx)
                            && (0..n as i32).contains(&ny)
                            && alpha[ny as usize * n + nx as usize] > 127
                    })
                });
                if near {
                    rgba[i..i + 4].copy_from_slice(&[0, 0, 0, 200]);
                }
            }
        }
    }

    Icon::from_rgba(rgba, S, S).expect("bad icon")
}

/// Open `Pictures/Scotshreen` in Finder / Explorer, creating it if needed.
pub fn open_save_folder() {
    let Some(dir) = crate::output::save_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);

    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let cmd = "xdg-open";

    let _ = std::process::Command::new(cmd).arg(dir).spawn();
}
