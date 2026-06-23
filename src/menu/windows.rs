//! Windows presenter: a native per-window menu bar via muda + HWND.

use super::backend::MenuPresenter;
use super::model::{self, Item, ItemState, Node};
use super::{Accel, AccelKey, MenuAction, MenuContext, Predefined};
use muda::{
    CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use std::collections::HashMap;

pub struct WindowsMenu {
    menu: Menu,
    checks: HashMap<MenuAction, CheckMenuItem>,
    ids: HashMap<MenuId, MenuAction>,
}

fn accelerator(accel: Accel) -> Accelerator {
    let mut mods = Modifiers::empty();
    if accel.super_mod {
        mods |= Modifiers::CONTROL; // ⌘-equivalent on Windows is Ctrl
    }
    if accel.shift {
        mods |= Modifiers::SHIFT;
    }
    let code = match accel.key {
        AccelKey::Char('o') => Code::KeyO,
        AccelKey::Char('p') => Code::KeyP,
        AccelKey::Char('w') => Code::KeyW,
        AccelKey::Char(_) => Code::KeyO,
        AccelKey::Slash => Code::Slash,
    };
    Accelerator::new(Some(mods), code)
}

/// Windows shows none of macOS's app-menu items. We keep only Quit (as "Exit",
/// mapped to closing all windows) and drop the rest.
fn predefined_label(p: Predefined) -> Option<(&'static str, MenuAction)> {
    match p {
        Predefined::Quit => Some(("Exit", MenuAction::CloseAllWindows)),
        _ => None,
    }
}

impl WindowsMenu {
    pub fn new() -> Self {
        let mut this = Self { menu: Menu::new(), checks: HashMap::new(), ids: HashMap::new() };
        for node in model::menu_bar() {
            if let Node::Submenu { title, children } = node {
                // Skip the macOS app menu and the macOS-only Window menu.
                if title == "Rusty Alto" || title == "Window" {
                    continue;
                }
                let submenu = Submenu::new(title, true);
                for child in children {
                    this.append(&submenu, child);
                }
                let _ = this.menu.append(&submenu);
            }
        }
        this
    }

    pub fn menu_handle(&self) -> Menu {
        self.menu.clone()
    }

    fn append(&mut self, parent: &Submenu, node: Node) {
        match node {
            Node::Separator => {
                let _ = parent.append(&PredefinedMenuItem::separator());
            }
            Node::Submenu { title, children } => {
                let sub = Submenu::new(title, true);
                for child in children {
                    self.append(&sub, child);
                }
                let _ = parent.append(&sub);
            }
            Node::Item(Item { action, label, accel }) => match action {
                MenuAction::Predefined(p) => {
                    if let Some((label, mapped)) = predefined_label(p) {
                        let item = MenuItem::new(label, true, None);
                        self.ids.insert(item.id().clone(), mapped);
                        let _ = parent.append(&item);
                    }
                }
                MenuAction::SetView(_) => {
                    let item = CheckMenuItem::new(label, false, false, None);
                    self.ids.insert(item.id().clone(), action);
                    self.checks.insert(action, item.clone());
                    let _ = parent.append(&item);
                }
                _ => {
                    let item = MenuItem::new(label, true, accel.map(accelerator));
                    self.ids.insert(item.id().clone(), action);
                    let _ = parent.append(&item);
                }
            },
        }
    }
}

impl MenuPresenter for WindowsMenu {
    fn install(&mut self) {}

    fn sync(&mut self, ctx: &MenuContext) {
        for (action, handle) in &self.checks {
            match model::item_state(action, ctx) {
                ItemState::Check(checked) => {
                    handle.set_enabled(true);
                    handle.set_checked(checked);
                }
                ItemState::Disabled => {
                    handle.set_enabled(false);
                    handle.set_checked(false);
                }
                ItemState::Normal => handle.set_enabled(true),
            }
        }
    }

    fn poll(&mut self) -> Vec<MenuAction> {
        let mut out = Vec::new();
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            if let Some(action) = self.ids.get(&event.id) {
                out.push(*action);
            }
        }
        out
    }

    fn windows_menu(&self) -> Option<muda::Menu> {
        Some(self.menu_handle())
    }
}
