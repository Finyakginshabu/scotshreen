//! Global hotkeys.
//!
//! - Cmd+Shift+1 (macOS) / Ctrl+Shift+1 (Windows, Linux) opens capture mode, or discards the
//!   open area (and its frames) and starts a new one.
//! - While an area is open, the toolbar's shortcuts are global too: the app being captured keeps
//!   the keyboard and stays active (its window keeps its focused look in the captures), so these
//!   keys never reach a Scotshreen window. To keep typing in other apps possible:
//!   - Esc is taken for the whole of capture mode (R4),
//!   - Space, Enter, Cmd+S and Cmd+Z only while the pointer is over the area or the toolbar,
//!   - the magnet has no global key (M works while the area is being drawn, and on the button).
//!
//!   All of them are released as soon as capture mode closes.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use winit::event_loop::EventLoopProxy;

use crate::overlay::toolbar::Action;
use crate::UserEvent;

#[cfg(target_os = "macos")]
const COMMAND: Modifiers = Modifiers::META;
#[cfg(not(target_os = "macos"))]
const COMMAND: Modifiers = Modifiers::CONTROL;

/// The toolbar's shortcuts, and whether each one needs the pointer over the area or toolbar.
fn shortcuts() -> [(HotKey, Action, bool); 5] {
    [
        (HotKey::new(None, Code::Escape), Action::Cancel, false),
        (HotKey::new(None, Code::Space), Action::Capture, true),
        (HotKey::new(None, Code::Enter), Action::Copy, true),
        (HotKey::new(Some(COMMAND), Code::KeyS), Action::Save, true),
        (HotKey::new(Some(COMMAND), Code::KeyZ), Action::Undo, true),
    ]
}

/// Which of the toolbar's shortcuts should be taken right now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shortcuts {
    /// Capture mode is closed: none.
    Off,
    /// An area is open but the pointer is elsewhere: only Esc.
    EscOnly,
    /// The pointer is over the area or the toolbar: all of them.
    All,
}

pub struct Hotkey {
    manager: GlobalHotKeyManager,
    /// Cmd+Shift+1 could be registered (another app may own it).
    main_ok: bool,
    /// Shortcut hotkeys registered right now, by id. Shared with the event handler, which may
    /// run on another thread.
    active: Arc<Mutex<HashMap<u32, Action>>>,
    /// Only touched on the main thread.
    state: Cell<Shortcuts>,
}

impl Hotkey {
    /// Returns `None` only if no hotkey can be registered at all. If Cmd+Shift+1 is taken by
    /// another app, the hotkey still exists (for the toolbar's shortcuts) and `main_ok` is false.
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Option<Self> {
        let manager = GlobalHotKeyManager::new().ok()?;
        let hotkey = HotKey::new(Some(COMMAND | Modifiers::SHIFT), Code::Digit1);
        let main_ok = match manager.register(hotkey) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("could not register the hotkey: {e}");
                false
            }
        };

        let id = hotkey.id();
        let active: Arc<Mutex<HashMap<u32, Action>>> = Arc::default();
        let shared = active.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state != HotKeyState::Pressed {
                return;
            }
            if e.id == id {
                let _ = proxy.send_event(UserEvent::Capture);
            } else if let Some(action) = shared.lock().ok().and_then(|m| m.get(&e.id).copied()) {
                let _ = proxy.send_event(UserEvent::Shortcut(action));
            }
        }));

        Some(Hotkey { manager, main_ok, active, state: Cell::new(Shortcuts::Off) })
    }

    pub fn main_ok(&self) -> bool {
        self.main_ok
    }

    /// Takes the toolbar's shortcuts that `want` asks for, and gives the others back.
    pub fn set_shortcuts(&self, want: Shortcuts) {
        if self.state.replace(want) == want {
            return;
        }
        let Ok(mut active) = self.active.lock() else { return };
        for (hotkey, action, needs_pointer) in shortcuts() {
            let on = match want {
                Shortcuts::Off => false,
                Shortcuts::EscOnly => !needs_pointer,
                Shortcuts::All => true,
            };
            let has = active.contains_key(&hotkey.id());
            if on && !has {
                match self.manager.register(hotkey) {
                    Ok(()) => {
                        active.insert(hotkey.id(), action);
                    }
                    Err(e) => eprintln!("could not register the {action:?} shortcut: {e}"),
                }
            } else if !on && has {
                let _ = self.manager.unregister(hotkey);
                active.remove(&hotkey.id());
            }
        }
    }
}
