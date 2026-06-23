//! The single source of truth for menu structure (`menu_bar`) and per-item
//! dynamic state (`item_state`). Pure data: no muda, no objc2, no iced.

use super::{Accel, AccelKey, MenuAction, MenuContext, Predefined};
use crate::model::PresentationMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemState {
    Normal,
    Disabled,
    Check(bool),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub action: MenuAction,
    pub label: &'static str,
    pub accel: Option<Accel>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Item(Item),
    Separator,
    Submenu {
        title: &'static str,
        children: Vec<Node>,
    },
}

fn item(action: MenuAction, label: &'static str, accel: Option<Accel>) -> Node {
    Node::Item(Item { action, label, accel })
}

fn predefined(p: Predefined, label: &'static str) -> Node {
    Node::Item(Item { action: MenuAction::Predefined(p), label, accel: None })
}

const CMD: fn(char) -> Accel = |c| Accel { super_mod: true, shift: false, key: AccelKey::Char(c) };

/// The whole menu bar, top level first. Backends render the subset they
/// support (e.g. the macOS app menu's predefined items are skipped elsewhere).
pub fn menu_bar() -> Vec<Node> {
    vec![
        Node::Submenu {
            title: "Rusty Alto",
            children: vec![
                predefined(Predefined::About, "About Rusty Alto"),
                Node::Separator,
                predefined(Predefined::Services, "Services"),
                Node::Separator,
                predefined(Predefined::Hide, "Hide Rusty Alto"),
                predefined(Predefined::HideOthers, "Hide Others"),
                predefined(Predefined::ShowAll, "Show All"),
                Node::Separator,
                predefined(Predefined::Quit, "Quit Rusty Alto"),
            ],
        },
        Node::Submenu {
            title: "File",
            children: vec![
                item(MenuAction::OpenGrammar, "Open grammar…", Some(CMD('o'))),
                item(MenuAction::NewParse, "Parse…", Some(CMD('p'))),
                Node::Separator,
                predefined(Predefined::CloseWindow, "Close Window"),
                item(
                    MenuAction::CloseAllWindows,
                    "Close All Windows",
                    Some(Accel { super_mod: true, shift: true, key: AccelKey::Char('w') }),
                ),
            ],
        },
        Node::Submenu {
            title: "View",
            children: vec![
                item(MenuAction::SetView(PresentationMode::Tag), "TAG", None),
                item(MenuAction::SetView(PresentationMode::RawIrtg), "IRTG", None),
            ],
        },
        Node::Submenu {
            title: "Window",
            children: vec![
                predefined(Predefined::Minimize, "Minimize"),
                predefined(Predefined::Maximize, "Zoom"),
                Node::Separator,
                predefined(Predefined::BringAllToFront, "Bring All to Front"),
            ],
        },
        Node::Submenu {
            title: "Help",
            children: vec![item(
                MenuAction::KeyboardShortcuts,
                "Keyboard Shortcuts",
                Some(Accel { super_mod: false, shift: true, key: AccelKey::Slash }),
            )],
        },
    ]
}

/// Dynamic state for a given action, mirroring the old `update_view_mode`.
pub fn item_state(action: &MenuAction, ctx: &MenuContext) -> ItemState {
    match action {
        MenuAction::SetView(PresentationMode::Tag) => {
            if !ctx.tag_available {
                ItemState::Disabled
            } else {
                ItemState::Check(ctx.mode == Some(PresentationMode::Tag))
            }
        }
        MenuAction::SetView(PresentationMode::RawIrtg) => {
            if !ctx.grammar_loaded {
                ItemState::Disabled
            } else {
                ItemState::Check(ctx.mode == Some(PresentationMode::RawIrtg))
            }
        }
        _ => ItemState::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_items(nodes: &[Node], out: &mut Vec<Item>) {
        for node in nodes {
            match node {
                Node::Item(i) => out.push(i.clone()),
                Node::Submenu { children, .. } => collect_items(children, out),
                Node::Separator => {}
            }
        }
    }

    #[test]
    fn every_item_has_a_nonempty_label() {
        let mut items = Vec::new();
        collect_items(&menu_bar(), &mut items);
        assert!(!items.is_empty());
        assert!(items.iter().all(|i| !i.label.is_empty()));
    }

    #[test]
    fn the_core_actions_are_present_exactly_once() {
        let mut items = Vec::new();
        collect_items(&menu_bar(), &mut items);
        let count = |a: MenuAction| items.iter().filter(|i| i.action == a).count();
        assert_eq!(count(MenuAction::OpenGrammar), 1);
        assert_eq!(count(MenuAction::NewParse), 1);
        assert_eq!(count(MenuAction::CloseAllWindows), 1);
        assert_eq!(count(MenuAction::KeyboardShortcuts), 1);
        assert_eq!(count(MenuAction::SetView(PresentationMode::Tag)), 1);
        assert_eq!(count(MenuAction::SetView(PresentationMode::RawIrtg)), 1);
    }

    #[test]
    fn tag_is_disabled_until_available_then_checks_with_mode() {
        let base = MenuContext { grammar_loaded: true, tag_available: false, mode: None };
        assert_eq!(
            item_state(&MenuAction::SetView(PresentationMode::Tag), &base),
            ItemState::Disabled
        );
        let tag_on = MenuContext {
            grammar_loaded: true,
            tag_available: true,
            mode: Some(PresentationMode::Tag),
        };
        assert_eq!(
            item_state(&MenuAction::SetView(PresentationMode::Tag), &tag_on),
            ItemState::Check(true)
        );
    }

    #[test]
    fn irtg_is_disabled_without_a_grammar() {
        let none = MenuContext { grammar_loaded: false, tag_available: false, mode: None };
        assert_eq!(
            item_state(&MenuAction::SetView(PresentationMode::RawIrtg), &none),
            ItemState::Disabled
        );
    }
}
