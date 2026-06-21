use muda::{
    AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};

pub const OPEN_GRAMMAR_ID: &str = "open-grammar";
pub const NEW_PARSE_ID: &str = "new-parse";
pub const CLOSE_ALL_ID: &str = "close-all";
pub const KEYBOARD_SHORTCUTS_ID: &str = "keyboard-shortcuts";
const APP_NAME: &str = "Rusty Alto";

fn cmd(code: Code) -> Accelerator {
    Accelerator::new(Some(Modifiers::SUPER), code)
}

pub fn install() {
    let menu = Menu::new();
    let app_menu = Submenu::new(APP_NAME, true);
    let about = PredefinedMenuItem::about(
        Some(&format!("About {APP_NAME}")),
        Some(AboutMetadata {
            name: Some(APP_NAME.to_owned()),
            ..Default::default()
        }),
    );
    let hide = PredefinedMenuItem::hide(Some(&format!("Hide {APP_NAME}")));
    let quit = PredefinedMenuItem::quit(Some(&format!("Quit {APP_NAME}")));
    let _ = app_menu.append_items(&[
        &about,
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &hide,
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &quit,
    ]);

    let file_menu = Submenu::new("File", true);
    let open_grammar = MenuItem::with_id(
        OPEN_GRAMMAR_ID,
        "Open grammar…",
        true,
        Some(cmd(Code::KeyO)),
    );
    let new_parse = MenuItem::with_id(NEW_PARSE_ID, "Parse…", true, Some(cmd(Code::KeyP)));
    let close_all = MenuItem::with_id(
        CLOSE_ALL_ID,
        "Close All Windows",
        true,
        Some(Accelerator::new(
            Some(Modifiers::SUPER | Modifiers::SHIFT),
            Code::KeyW,
        )),
    );
    let _ = file_menu.append_items(&[
        &open_grammar,
        &new_parse,
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::close_window(None),
        &close_all,
    ]);

    let window_menu = Submenu::new("Window", true);
    let _ = window_menu.append_items(&[
        &PredefinedMenuItem::minimize(None),
        &PredefinedMenuItem::maximize(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::bring_all_to_front(None),
    ]);

    let help_menu = Submenu::new("Help", true);
    let shortcuts = MenuItem::with_id(
        KEYBOARD_SHORTCUTS_ID,
        "Keyboard Shortcuts",
        true,
        Some(Accelerator::new(Some(Modifiers::SHIFT), Code::Slash)),
    );
    let _ = help_menu.append(&shortcuts);

    let _ = menu.append_items(&[&app_menu, &file_menu, &window_menu, &help_menu]);
    menu.init_for_nsapp();
    std::mem::forget(menu);
}

pub fn refresh_windows_menu() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
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
}
