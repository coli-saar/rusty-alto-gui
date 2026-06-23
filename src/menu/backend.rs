//! The presenter abstraction the app talks to. One impl per platform plus a
//! no-op fallback so the app compiles on any target.

use super::{MenuAction, MenuContext};
use iced::window;

pub trait MenuPresenter {
    /// Build/attach the bar. Safe to call once the event loop is running.
    fn install(&mut self);
    /// Push current enabled/checked state into a native bar (no-op for the
    /// iced-drawn bar, which reads context at view time).
    fn sync(&mut self, ctx: &MenuContext);
    /// Drain pending native activations into abstract actions.
    fn poll(&mut self) -> Vec<MenuAction>;
    /// A new window appeared (used by the Windows backend to attach a bar).
    fn window_opened(&mut self, _id: window::Id) {}

    /// The native menu to attach to a freshly created window (Windows). `None`
    /// where the bar isn't per-window.
    #[cfg(target_os = "windows")]
    fn windows_menu(&self) -> Option<muda::Menu> {
        None
    }
}

/// Used on platforms whose bar is drawn in iced, or before a real backend is
/// wired in. Does nothing and yields no actions.
#[allow(dead_code)]
pub struct NullPresenter;

impl MenuPresenter for NullPresenter {
    fn install(&mut self) {}
    fn sync(&mut self, _ctx: &MenuContext) {}
    fn poll(&mut self) -> Vec<MenuAction> {
        Vec::new()
    }
}
