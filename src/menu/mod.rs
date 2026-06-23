//! Platform-neutral menu definitions and the presenter abstraction.
//!
//! The menu *structure* and *behavior* live here and in [`model`]; the
//! platform-specific way a bar is shown lives in the backend modules. On
//! Linux a "native" menu bar is itself just an in-window widget, so we draw
//! one in iced rather than asking GTK — anything GTK would give us as
//! in-window UI we provide by drawing in iced instead. (This does NOT cover
//! OS-protocol integrations such as the PDF drag-out, which need the real
//! X11/Wayland protocol and have no in-window equivalent.)

pub mod model;

use crate::model::PresentationMode;

/// An abstract menu activation. Every backend maps its own widgets to this,
/// so action handling in `app.rs` is written once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuAction {
    OpenGrammar,
    NewParse,
    CloseAllWindows,
    KeyboardShortcuts,
    SetView(PresentationMode),
    /// A platform-provided item (About/Quit/…). Backends that have no native
    /// equivalent simply skip it.
    Predefined(Predefined),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Predefined {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    CloseWindow,
    Minimize,
    Maximize,
    BringAllToFront,
    Quit,
}

/// A keyboard accelerator shown alongside an item. `super_mod` is ⌘ on macOS
/// and Ctrl elsewhere (matches `modifiers.command()` semantics in iced).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accel {
    pub super_mod: bool,
    pub shift: bool,
    pub key: AccelKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelKey {
    Char(char),
    Slash,
}

/// A snapshot of the focused window used to compute enabled/checked state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuContext {
    pub grammar_loaded: bool,
    pub tag_available: bool,
    pub mode: Option<PresentationMode>,
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
pub mod backend;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod iced_bar;

#[allow(unused_imports)]
pub use backend::{MenuPresenter, NullPresenter};

/// Construct the platform's presenter.
pub fn presenter() -> Box<dyn MenuPresenter> {
    #[cfg(target_os = "macos")]
    { Box::new(macos::MacosMenu::new()) }
    #[cfg(target_os = "windows")]
    { Box::new(windows::WindowsMenu::new()) }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { Box::new(NullPresenter) }
}

/// The in-window menu bar element. Empty except on Linux, where the OS has no
/// menu server and we draw our own.
pub fn bar_view<'a>(
    _open: Option<usize>,
    _ctx: &MenuContext,
) -> iced::Element<'a, crate::app::AppMsg> {
    #[cfg(target_os = "linux")]
    {
        iced_bar::view(_open, _ctx)
    }
    #[cfg(not(target_os = "linux"))]
    {
        iced::widget::column![].into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::backend::MenuPresenter;

    #[test]
    fn null_presenter_yields_no_actions() {
        let mut p = NullPresenter;
        p.install();
        p.sync(&MenuContext { grammar_loaded: false, tag_available: false, mode: None });
        assert!(p.poll().is_empty());
    }
}
