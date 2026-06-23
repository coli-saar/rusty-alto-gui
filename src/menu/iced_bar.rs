//! The iced-drawn menu bar used on Linux (and a portable fallback).
//!
//! Linux has no OS menu server, so a "native" bar is just an in-window widget —
//! which is exactly what this is. The module compiles on every platform so the
//! macOS build type-checks it; `super::bar_view` decides where it is rendered.

use super::model::{self, Item, ItemState, Node};
use super::{Accel, AccelKey, MenuAction, Predefined};
use crate::app::AppMsg;
use crate::theme;
use iced::widget::{button, column, container, row, space, text};
use iced::{Element, Length};

/// Render an accelerator as a hint string. `super_mod` is Ctrl on Linux.
fn accel_label(accel: Accel) -> String {
    let mut mods: Vec<&str> = Vec::new();
    if accel.super_mod {
        mods.push("Ctrl");
    }
    if accel.shift {
        mods.push("Shift");
    }
    let key = match accel.key {
        AccelKey::Char(c) => c.to_ascii_uppercase().to_string(),
        AccelKey::Slash => "/".to_string(),
    };
    if mods.is_empty() {
        key
    } else {
        format!("{}+{}", mods.join("+"), key)
    }
}

/// A row of top-level menu buttons; the open one drops its children below.
pub fn view<'a>(open: Option<usize>, ctx: &super::MenuContext) -> Element<'a, AppMsg> {
    let tops = model::menu_bar();
    let mut bar = row![].spacing(2);
    let mut dropdown: Option<Element<'a, AppMsg>> = None;

    for (index, node) in tops.iter().enumerate() {
        if let Node::Submenu { title, children } = node {
            // Skip the macOS app menu; its only portable item (Quit) is surfaced
            // under File as a normal item below.
            if *title == "Rusty Alto" {
                continue;
            }
            bar = bar.push(
                button(text(title.to_string()).size(13))
                    .style(theme::quiet_button)
                    .on_press(AppMsg::MenuToggle(index)),
            );
            if open == Some(index) {
                dropdown = Some(dropdown_view(children, ctx));
            }
        }
    }

    let bar = container(bar)
        .width(Length::Fill)
        .padding(4)
        .style(theme::workspace);

    match dropdown {
        Some(menu) => column![bar, menu].into(),
        None => bar.into(),
    }
}

fn dropdown_view<'a>(children: &[Node], ctx: &super::MenuContext) -> Element<'a, AppMsg> {
    let mut items = column![].spacing(1);
    for node in children {
        match node {
            Node::Separator => {
                items = items.push(container(text(" ").size(6)));
            }
            Node::Item(Item { action, label, accel }) => {
                // Only surface Quit among predefined items; the rest are macOS
                // concepts with no portable equivalent.
                if matches!(action, MenuAction::Predefined(p) if *p != Predefined::Quit) {
                    continue;
                }
                let state = model::item_state(action, ctx);
                let prefix = match state {
                    ItemState::Check(true) => "✓ ",
                    _ => "   ",
                };
                let on_press = if matches!(action, MenuAction::Predefined(Predefined::Quit)) {
                    AppMsg::Menu(MenuAction::CloseAllWindows)
                } else {
                    AppMsg::Menu(*action)
                };
                // Label on the left, accelerator hint right-aligned (like a
                // native menu), separated by a flexible space.
                let hint = accel.map(accel_label).unwrap_or_default();
                let content = row![
                    text(format!("{prefix}{label}")).size(13),
                    space::horizontal(),
                    text(hint).size(13).style(theme::muted_text),
                ]
                .spacing(16)
                .align_y(iced::Alignment::Center);
                let mut b = button(content)
                    .width(Length::Fill)
                    .style(theme::quiet_button);
                if !matches!(state, ItemState::Disabled) {
                    b = b.on_press(on_press);
                }
                items = items.push(b);
            }
            Node::Submenu { .. } => {} // no nested submenus in this bar
        }
    }
    container(items).padding(4).style(theme::workspace).into()
}
