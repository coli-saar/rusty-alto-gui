//! macOS presenter: a native top-of-screen bar via muda + AppKit upkeep.

use super::backend::MenuPresenter;
use super::model::{self, Item, ItemState, Node};
use super::{Accel, AccelKey, MenuAction, MenuContext, Predefined};
use muda::{
    AboutMetadata, CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use std::collections::HashMap;

const APP_NAME: &str = "Rusty Alto";

pub struct MacosMenu {
    installed: bool,
    checks: HashMap<MenuAction, CheckMenuItem>,
    ids: HashMap<MenuId, MenuAction>,
}

impl MacosMenu {
    pub fn new() -> Self {
        Self { installed: false, checks: HashMap::new(), ids: HashMap::new() }
    }
}

/// Translate our accelerator into muda's. `super_mod` maps to ⌘ (SUPER).
fn build_accelerator(accel: Accel) -> Accelerator {
    let mut mods = Modifiers::empty();
    if accel.super_mod {
        mods |= Modifiers::SUPER;
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

fn predefined_item(p: Predefined, label: &str) -> PredefinedMenuItem {
    match p {
        Predefined::About => PredefinedMenuItem::about(
            Some(label),
            Some(AboutMetadata { name: Some(APP_NAME.to_owned()), ..Default::default() }),
        ),
        Predefined::Services => PredefinedMenuItem::services(None),
        Predefined::Hide => PredefinedMenuItem::hide(Some(label)),
        Predefined::HideOthers => PredefinedMenuItem::hide_others(None),
        Predefined::ShowAll => PredefinedMenuItem::show_all(None),
        Predefined::CloseWindow => PredefinedMenuItem::close_window(None),
        Predefined::Minimize => PredefinedMenuItem::minimize(None),
        Predefined::Maximize => PredefinedMenuItem::maximize(None),
        Predefined::BringAllToFront => PredefinedMenuItem::bring_all_to_front(None),
        Predefined::Quit => PredefinedMenuItem::quit(Some(label)),
    }
}

impl MenuPresenter for MacosMenu {
    fn install(&mut self) {
        if self.installed {
            return;
        }
        let menu = Menu::new();
        for node in model::menu_bar() {
            if let Node::Submenu { title, children } = node {
                let submenu = Submenu::new(title, true);
                for child in children {
                    self.append(&submenu, child);
                }
                let _ = menu.append(&submenu);
            }
        }
        menu.init_for_nsapp();
        std::mem::forget(menu);
        self.installed = true;
    }

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
        suppress_window_tabbing();
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

    fn window_opened(&mut self, _id: iced::window::Id) {
        refresh_windows_menu();
    }
}

impl MacosMenu {
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
                    let _ = parent.append(&predefined_item(p, label));
                }
                MenuAction::SetView(_) => {
                    let item = CheckMenuItem::new(label, false, false, None);
                    self.ids.insert(item.id().clone(), action);
                    self.checks.insert(action, item.clone());
                    let _ = parent.append(&item);
                }
                _ => {
                    let accel = accel.map(build_accelerator);
                    let item = MenuItem::new(label, true, accel);
                    self.ids.insert(item.id().clone(), action);
                    let _ = parent.append(&item);
                }
            },
        }
    }
}

// --- AppKit upkeep: copied verbatim from the former platform_menu.rs ---

/// Suppress macOS's automatic window-tabbing UI and the items it injects.
pub fn suppress_window_tabbing() {
    use objc2_app_kit::{NSApplication, NSWindow};
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else { return };
    NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);

    let app = NSApplication::sharedApplication(mtm);
    let Some(main_menu) = (unsafe { app.mainMenu() }) else { return };

    let top_count = unsafe { main_menu.numberOfItems() };
    for top in 0..top_count {
        let Some(submenu) = (unsafe { main_menu.itemAtIndex(top) })
            .and_then(|item| unsafe { item.submenu() })
        else {
            continue;
        };
        let mut removed = false;
        let mut index = unsafe { submenu.numberOfItems() } - 1;
        while index >= 0 {
            if let Some(item) = unsafe { submenu.itemAtIndex(index) }
                && let Some(action) = unsafe { item.action() }
                && matches!(action.name(), "toggleTabBar:" | "toggleTabOverview:")
            {
                unsafe { submenu.removeItem(&item) };
                removed = true;
            }
            index -= 1;
        }
        if removed {
            trim_edge_separators(&submenu);
        }
    }
}

fn trim_edge_separators(submenu: &objc2_app_kit::NSMenu) {
    loop {
        let count = unsafe { submenu.numberOfItems() };
        if count == 0 { break; }
        let Some(item) = (unsafe { submenu.itemAtIndex(count - 1) }) else { break };
        if unsafe { item.isSeparatorItem() } {
            unsafe { submenu.removeItem(&item) };
        } else { break; }
    }
    loop {
        let count = unsafe { submenu.numberOfItems() };
        if count == 0 { break; }
        let Some(item) = (unsafe { submenu.itemAtIndex(0) }) else { break };
        if unsafe { item.isSeparatorItem() } {
            unsafe { submenu.removeItem(&item) };
        } else { break; }
    }
}

pub fn refresh_windows_menu() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);

    if let Some(main_menu) = unsafe { app.mainMenu() } {
        let count = unsafe { main_menu.numberOfItems() };
        if count > 1
            && let Some(submenu) = unsafe { main_menu.itemAtIndex(count - 2) }
                .and_then(|item| unsafe { item.submenu() })
        {
            unsafe { app.setWindowsMenu(Some(&submenu)) };
        }
    }
    let windows = app.windows();
    for i in 0..windows.count() {
        let window = unsafe { windows.objectAtIndex(i) };
        let title = window.title();
        unsafe { app.addWindowsItem_title_filename(&window, &title, false) };
    }
    suppress_window_tabbing();
}
