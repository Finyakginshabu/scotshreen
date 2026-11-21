pub mod frame;
pub mod label;
pub mod magnet;
mod piece;
pub mod select;
pub mod toast;
pub mod toolbar;

use std::rc::Rc;

use fontdue::{Font, FontSettings};

thread_local! {
    static FONT: Option<Rc<Font>> =
        Font::from_bytes(include_bytes!("../../assets/font.ttf") as &[u8], FontSettings::default())
            .ok()
            .map(Rc::new);
}

thread_local! {
    static ICON_FONT: Option<Rc<Font>> =
        Font::from_bytes(include_bytes!("../../assets/icons.ttf") as &[u8], FontSettings::default())
            .ok()
            .map(Rc::new);
}

/// The toolbar icon font (`assets/icons.ttf`), parsed once.
pub fn icon_font() -> Option<Rc<Font>> {
    ICON_FONT.with(|f| f.clone())
}

/// The embedded font, parsed once and shared by every overlay window.
pub fn font() -> Option<Rc<Font>> {
    FONT.with(|f| f.clone())
}
