use crate::{
    model::{
        ChartDocument, DerivationPresentation, DocumentTab, GrammarDocument, InputField,
        RuleColumn, RuleRow, StrategyChoice,
    },
    theme,
    tree_canvas::tree_view,
    workers::{self, LanguageEvent, LanguageWorker},
};
use iced::{
    Alignment, Element, Event, Length, Subscription, Task, event,
    keyboard::{Key, Modifiers, key::Named},
    widget::{
        Column, Row, button, checkbox, column, container, horizontal_rule, horizontal_space,
        pick_list, rich_text, row, scrollable, span, stack, text, text_input, vertical_rule,
    },
    window,
};
use rusty_alto::LanguageCardinality;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

pub fn run() -> iced::Result {
    iced::daemon(app_title, app_update, app_view)
        .theme(|_app, _id| iced::Theme::Light)
        .font(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice())
        .default_font(iced::Font {
            family: iced::font::Family::Name("Inter"),
            weight: iced::font::Weight::Medium,
            ..iced::Font::DEFAULT
        })
        .subscription(app_subscription)
        .run_with(|| {
            let mut app = App::default();
            let (id, open) = window::open(window_settings());
            app.windows.insert(id, Workbench::default());
            (app, open.map(AppMsg::WindowOpened))
        })
}

fn window_settings() -> window::Settings {
    window::Settings {
        size: iced::Size::new(1440.0, 900.0),
        min_size: Some(iced::Size::new(1050.0, 680.0)),
        ..Default::default()
    }
}

/// Top-level daemon state: one [`Workbench`] per open grammar window.
#[derive(Default)]
struct App {
    windows: BTreeMap<window::Id, Workbench>,
    #[cfg(target_os = "macos")]
    menu_installed: bool,
    /// Last window to gain focus — the target for app-level menu actions.
    #[cfg(target_os = "macos")]
    focused: Option<window::Id>,
}

#[derive(Debug, Clone)]
enum AppMsg {
    /// A message originating from a specific window's view/shortcuts.
    Window(window::Id, Message),
    WindowOpened(window::Id),
    CloseWindow(window::Id),
    GrammarPicked(window::Id, Option<PathBuf>),
    GrammarLoaded(window::Id, Result<GrammarDocument, String>),
    Poll,
    #[cfg(target_os = "macos")]
    WindowFocused(window::Id),
    #[cfg(target_os = "macos")]
    MenuPoll,
}

fn app_title(app: &App, id: window::Id) -> String {
    app.windows
        .get(&id)
        .map(window_title)
        .unwrap_or_else(|| "Rusty Alto".to_owned())
}

fn window_title(state: &Workbench) -> String {
    match &state.grammar {
        Some(grammar) => display_name(&grammar.path),
        None => "Rusty Alto".to_owned(),
    }
}

fn app_view(app: &App, id: window::Id) -> Element<'_, AppMsg> {
    match app.windows.get(&id) {
        Some(window) => view(window).map(move |message| AppMsg::Window(id, message)),
        None => horizontal_space().into(),
    }
}

fn app_update(app: &mut App, message: AppMsg) -> Task<AppMsg> {
    match message {
        // Opening a grammar is an app-level action: it may spawn a new window.
        AppMsg::Window(id, Message::OpenGrammar | Message::ShortcutOpenGrammar) => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("IRTG grammar", &["irtg"])
                    .pick_file()
                    .await
                    .map(|handle| handle.path().to_owned())
            },
            move |path| AppMsg::GrammarPicked(id, path),
        ),
        AppMsg::Window(id, message) => match app.windows.get_mut(&id) {
            Some(window) => update(window, message).map(move |m| AppMsg::Window(id, m)),
            None => Task::none(),
        },
        AppMsg::GrammarPicked(_, None) => Task::none(),
        AppMsg::GrammarPicked(asking_id, Some(path)) => {
            // Load into the asking window if it's still empty, otherwise open a
            // fresh window so every grammar gets its own window.
            let needs_new_window = app
                .windows
                .get(&asking_id)
                .is_none_or(|window| window.grammar.is_some());
            let (target, open_task) = if needs_new_window {
                let (new_id, open) = window::open(window_settings());
                app.windows.insert(new_id, Workbench::default());
                (new_id, open.map(AppMsg::WindowOpened))
            } else {
                (asking_id, Task::none())
            };
            if let Some(window) = app.windows.get_mut(&target) {
                window.busy = Some(format!("Loading {}…", display_name(&path)));
                window.error = None;
            }
            let load = Task::perform(
                async move { workers::load_grammar(path) },
                move |result| AppMsg::GrammarLoaded(target, result),
            );
            Task::batch([open_task, load])
        }
        AppMsg::GrammarLoaded(id, result) => {
            if let Some(window) = app.windows.get_mut(&id) {
                window.apply_grammar(result);
            }
            Task::none()
        }
        AppMsg::WindowOpened(id) => {
            // Install the native menu bar once, now that NSApp is running on the
            // main thread (this update runs on the winit/main thread).
            #[cfg(target_os = "macos")]
            if !app.menu_installed {
                app.menu_installed = true;
                macos_menu::install();
            }
            window::gain_focus(id)
        }
        AppMsg::CloseWindow(id) => {
            app.windows.remove(&id);
            if app.windows.is_empty() {
                iced::exit()
            } else {
                window::close(id)
            }
        }
        AppMsg::Poll => {
            for window in app.windows.values_mut() {
                window.poll();
            }
            Task::none()
        }
        #[cfg(target_os = "macos")]
        AppMsg::WindowFocused(id) => {
            app.focused = Some(id);
            Task::none()
        }
        #[cfg(target_os = "macos")]
        AppMsg::MenuPoll => {
            let mut tasks = Vec::new();
            while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
                match event.id.0.as_str() {
                    macos_menu::OPEN_GRAMMAR_ID => {
                        // Open into the focused window (its handler decides
                        // whether to reuse the window or spawn a new one).
                        let target = app
                            .focused
                            .or_else(|| app.windows.keys().next().copied());
                        if let Some(id) = target {
                            tasks.push(app_update(app, AppMsg::Window(id, Message::OpenGrammar)));
                        }
                    }
                    macos_menu::CLOSE_ALL_ID => {
                        app.windows.clear();
                        tasks.push(iced::exit());
                    }
                    _ => {}
                }
            }
            Task::batch(tasks)
        }
    }
}

fn app_subscription(app: &App) -> Subscription<AppMsg> {
    let needs_poll = app.windows.values().any(Workbench::has_pending_language);
    let polling = if needs_poll {
        iced::time::every(Duration::from_millis(80)).map(|_| AppMsg::Poll)
    } else {
        Subscription::none()
    };
    // Route keyboard shortcuts to whichever window currently has focus, and
    // remember the focused window for app-level menu actions (macOS).
    let events = event::listen_with(|event, _status, id| match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            keyboard_shortcut(key, modifiers).map(|message| AppMsg::Window(id, message))
        }
        #[cfg(target_os = "macos")]
        Event::Window(window::Event::Focused) => Some(AppMsg::WindowFocused(id)),
        _ => None,
    });
    let closes = window::close_requests().map(AppMsg::CloseWindow);
    let mut subscriptions = vec![polling, events, closes];
    // Drain native menu activations (Open grammar / Close All Windows).
    #[cfg(target_os = "macos")]
    subscriptions.push(iced::time::every(Duration::from_millis(120)).map(|_| AppMsg::MenuPoll));
    Subscription::batch(subscriptions)
}

/// Native macOS menu bar via muda. The predefined items (Quit, Hide, Minimize,
/// and the auto-populated window list) are handled by AppKit itself, so no menu
/// events need routing back into iced. A top-of-screen menu bar is a macOS
/// concept; the muda API is cross-platform, so adding Windows/Linux later is an
/// extra init call rather than a rewrite.
#[cfg(target_os = "macos")]
mod macos_menu {
    use muda::{
        AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    };

    pub const OPEN_GRAMMAR_ID: &str = "open-grammar";
    pub const CLOSE_ALL_ID: &str = "close-all";
    const APP_NAME: &str = "Rusty Alto";

    pub fn install() {
        let menu = Menu::new();

        // The application menu (its bold name comes from the process name).
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

        // File: custom items routed back through muda's event channel; Close
        // Window is the native ⌘W item.
        let file_menu = Submenu::new("File", true);
        let open_grammar = MenuItem::with_id(OPEN_GRAMMAR_ID, "Open grammar…", true, None);
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
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
            &close_all,
        ]);

        // Edit: standard items handled natively by AppKit.
        let edit_menu = Submenu::new("Edit", true);
        let _ = edit_menu.append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ]);

        // Window: AppKit auto-populates this with the open windows.
        let window_menu = Submenu::new("Window", true);
        let _ = window_menu.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ]);

        let _ = menu.append_items(&[&app_menu, &file_menu, &edit_menu, &window_menu]);
        menu.init_for_nsapp();
        window_menu.set_as_windows_menu_for_nsapp();

        // AppKit retains the NSMenu, but keep muda's wrappers alive for the
        // process lifetime so the menu isn't torn down.
        std::mem::forget(menu);
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenGrammar,
    SelectGrammar,
    SelectParse(u64),
    RemoveParse(u64),
    NewParse,
    SelectTab(DocumentTab),
    SortGrammar(RuleColumn),
    SortChart(u64, RuleColumn),
    InputChanged(usize, String),
    StrategyChanged(StrategyChoice),
    StopAtFirstGoal(bool),
    BeamChanged(String),
    Parse,
    Parsed(Result<ChartDocument, String>),
    PreviousDerivation,
    NextDerivation,
    SelectOutput(usize),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ShortcutOpenGrammar,
    ShortcutPrevious,
    ShortcutNext,
}

pub struct Workbench {
    grammar: Option<GrammarDocument>,
    grammar_language: Option<LanguageSession>,
    parses: Vec<ParseSession>,
    next_parse_id: u64,
    selection: Selection,
    active_tab: DocumentTab,
    inputs: Vec<InputField>,
    strategy: StrategyChoice,
    stop_at_first_goal: bool,
    beam: String,
    pending_label: Option<String>,
    busy: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selection {
    Grammar,
    Parse(u64),
    NewParse,
}

struct ParseSession {
    id: u64,
    label: String,
    chart: ChartDocument,
    language: LanguageSession,
}

struct LanguageSession {
    status: LanguageStatus,
    receiver: Option<mpsc::Receiver<LanguageEvent>>,
    worker: Option<LanguageWorker>,
    derivations: Vec<Arc<DerivationPresentation>>,
    derivation_index: usize,
    output_index: usize,
    zoom: f32,
}

#[derive(Debug, Clone)]
enum LanguageStatus {
    Preparing,
    Ready(LanguageCardinality),
    Error(String),
}

impl Default for Workbench {
    fn default() -> Self {
        Self {
            grammar: None,
            grammar_language: None,
            parses: Vec::new(),
            next_parse_id: 1,
            selection: Selection::Grammar,
            active_tab: DocumentTab::Primary,
            inputs: Vec::new(),
            strategy: StrategyChoice::TopDown,
            stop_at_first_goal: false,
            beam: String::new(),
            pending_label: None,
            busy: None,
            error: None,
        }
    }
}

impl LanguageSession {
    fn preparing(worker: LanguageWorker, receiver: mpsc::Receiver<LanguageEvent>) -> Self {
        Self {
            status: LanguageStatus::Preparing,
            receiver: Some(receiver),
            worker: Some(worker),
            derivations: Vec::new(),
            derivation_index: 0,
            output_index: 0,
            zoom: 1.0,
        }
    }

    fn ready(&self) -> bool {
        matches!(self.status, LanguageStatus::Ready(_))
    }

    fn has_next(&self) -> bool {
        match self.status {
            LanguageStatus::Ready(LanguageCardinality::Finite(size)) => {
                self.derivation_index + 1 < size
            }
            LanguageStatus::Ready(
                LanguageCardinality::Infinite | LanguageCardinality::TooLarge,
            ) => true,
            _ => false,
        }
    }

    fn size_label(&self) -> String {
        match self.status {
            LanguageStatus::Ready(LanguageCardinality::Finite(size)) => size.to_string(),
            LanguageStatus::Ready(LanguageCardinality::Infinite) => "∞".into(),
            LanguageStatus::Ready(LanguageCardinality::TooLarge) => "many".into(),
            _ => "…".into(),
        }
    }

    fn sidebar_status(&self) -> String {
        match &self.status {
            LanguageStatus::Preparing => "Preparing language…".into(),
            LanguageStatus::Ready(LanguageCardinality::Finite(0)) => "Empty language".into(),
            LanguageStatus::Ready(LanguageCardinality::Finite(1)) => "1 derivation".into(),
            LanguageStatus::Ready(LanguageCardinality::Finite(size)) => {
                format!("{size} derivations")
            }
            LanguageStatus::Ready(LanguageCardinality::Infinite) => "∞ derivations".into(),
            LanguageStatus::Ready(LanguageCardinality::TooLarge) => "Many derivations".into(),
            LanguageStatus::Error(error) => format!("Language error: {error}"),
        }
    }
}

fn update(state: &mut Workbench, message: Message) -> Task<Message> {
    match message {
        // Opening a grammar is handled at the app level (it may open a window).
        Message::OpenGrammar | Message::ShortcutOpenGrammar => {}
        Message::SelectGrammar => {
            if state.grammar.is_some() {
                state.selection = Selection::Grammar;
                state.active_tab = DocumentTab::Primary;
                state.error = None;
            }
        }
        Message::SelectParse(id) => {
            if state.parse(id).is_some() {
                state.selection = Selection::Parse(id);
                state.active_tab = DocumentTab::Primary;
                state.error = None;
            }
        }
        Message::RemoveParse(id) => {
            if let Some(index) = state.parses.iter().position(|parse| parse.id == id) {
                state.parses.remove(index);
            }
            if state.selection == Selection::Parse(id) {
                state.selection = Selection::Grammar;
                state.active_tab = DocumentTab::Primary;
            }
        }
        Message::NewParse => {
            if let Some(grammar) = &state.grammar {
                state.inputs = input_fields(grammar);
                state.strategy = StrategyChoice::TopDown;
                state.stop_at_first_goal = false;
                state.beam.clear();
                state.selection = Selection::NewParse;
                state.active_tab = DocumentTab::Primary;
                state.error = None;
            }
        }
        Message::SelectTab(tab) => {
            if tab == DocumentTab::Primary
                || state.active_language().is_some_and(|lang| lang.ready())
            {
                state.active_tab = tab;
            }
        }
        Message::SortGrammar(column) => {
            if let Some(grammar) = &mut state.grammar {
                sort_rows(&mut grammar.rules, column);
            }
        }
        Message::SortChart(id, column) => {
            if let Some(parse) = state.parse_mut(id) {
                sort_rows(&mut parse.chart.rules, column);
            }
        }
        Message::InputChanged(index, value) => {
            if let Some(field) = state.inputs.get_mut(index) {
                field.value = value;
            }
        }
        Message::StrategyChanged(strategy) => state.strategy = strategy,
        Message::StopAtFirstGoal(value) => state.stop_at_first_goal = value,
        Message::BeamChanged(value) => state.beam = value,
        Message::Parse => {
            let Some(grammar) = state
                .grammar
                .as_ref()
                .map(|document| document.grammar.clone())
            else {
                return Task::none();
            };
            let inputs = state
                .inputs
                .iter()
                .filter(|input| !input.value.trim().is_empty())
                .map(|input| (input.name.clone(), input.value.trim().to_owned()))
                .collect::<Vec<_>>();
            if inputs.is_empty() {
                state.fail("Enter input for at least one interpretation.".into());
                return Task::none();
            }
            let beam = if state.beam.trim().is_empty() {
                None
            } else {
                match state.beam.trim().parse::<f64>() {
                    Ok(value) if value.is_finite() && value > 0.0 => Some(value),
                    _ => {
                        state.fail("Beam must be a positive finite number.".into());
                        return Task::none();
                    }
                }
            };
            state.pending_label = Some(parse_label(&inputs));
            state.busy = Some("Computing parse chart…".into());
            state.error = None;
            let strategy = state.strategy.to_strategy(state.stop_at_first_goal, beam);
            return Task::perform(
                async move { workers::parse(grammar, inputs, strategy) },
                Message::Parsed,
            );
        }
        Message::Parsed(result) => {
            state.busy = None;
            match result {
                Ok(chart) => {
                    let Some(grammar) = &state.grammar else {
                        return Task::none();
                    };
                    let id = state.next_parse_id;
                    state.next_parse_id += 1;
                    let (sender, receiver) = mpsc::channel();
                    let worker = workers::start_chart_language_worker(
                        grammar.grammar.clone(),
                        chart.automaton.clone(),
                        sender,
                    );
                    state.parses.push(ParseSession {
                        id,
                        label: state
                            .pending_label
                            .take()
                            .unwrap_or_else(|| "Parsed input".into()),
                        chart,
                        language: LanguageSession::preparing(worker, receiver),
                    });
                    state.selection = Selection::Parse(id);
                    state.active_tab = DocumentTab::Primary;
                    state.error = None;
                }
                Err(error) => {
                    state.pending_label = None;
                    state.fail(format!("Parsing failed: {error}"));
                }
            }
        }
        Message::PreviousDerivation => {
            if let Some(language) = state.active_language_mut() {
                language.derivation_index = language.derivation_index.saturating_sub(1);
            }
        }
        Message::NextDerivation => {
            if let Some(language) = state.active_language_mut() {
                if language.derivation_index + 1 < language.derivations.len() {
                    language.derivation_index += 1;
                } else if language.has_next() {
                    if let Some(worker) = &language.worker {
                        worker.request(language.derivation_index + 1);
                    }
                }
            }
        }
        Message::SelectOutput(index) => {
            if let Some(language) = state.active_language_mut() {
                language.output_index = index;
            }
        }
        Message::ZoomIn => {
            if let Some(language) = state.active_language_mut() {
                language.zoom = (language.zoom + 0.15).min(2.5);
            }
        }
        Message::ZoomOut => {
            if let Some(language) = state.active_language_mut() {
                language.zoom = (language.zoom - 0.15).max(0.35);
            }
        }
        Message::ZoomReset => {
            if let Some(language) = state.active_language_mut() {
                language.zoom = 1.0;
            }
        }
        Message::ShortcutPrevious => {
            if state.active_tab == DocumentTab::Language {
                return update(state, Message::PreviousDerivation);
            }
        }
        Message::ShortcutNext => {
            if state.active_tab == DocumentTab::Language {
                return update(state, Message::NextDerivation);
            }
        }
    }
    Task::none()
}

fn poll_language(language: &mut LanguageSession) {
    let events = language
        .receiver
        .as_ref()
        .map(|receiver| receiver.try_iter().take(32).collect::<Vec<_>>())
        .unwrap_or_default();
    for event in events {
        match event {
            LanguageEvent::Ready(size) => {
                language.status = LanguageStatus::Ready(size);
                if size != LanguageCardinality::Finite(0)
                    && let Some(worker) = &language.worker
                {
                    worker.request(0);
                }
            }
            LanguageEvent::Derivation(item) => {
                let index = item.index;
                if index == language.derivations.len() {
                    language.derivations.push(item);
                } else if let Some(slot) = language.derivations.get_mut(index) {
                    *slot = item;
                }
                if index == language.derivation_index + 1 {
                    language.derivation_index = index;
                }
            }
            LanguageEvent::EndOfLanguage(count) => {
                language.status = LanguageStatus::Ready(LanguageCardinality::Finite(count));
            }
            LanguageEvent::Error(error) => {
                language.status = LanguageStatus::Error(error);
            }
        }
    }
}

fn view(state: &Workbench) -> Element<'_, Message> {
    let body = row![sidebar(state), vertical_rule(1), workspace(state)].height(Length::Fill);

    container(column![body, status_bar(state)])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::workspace)
        .into()
}

fn sidebar(state: &Workbench) -> Element<'_, Message> {
    let grammar_row: Element<'_, Message> = if let Some(grammar) = &state.grammar {
        document_button(
            column![
                text(display_name(&grammar.path)).size(14),
                text(format!(
                    "{} rules · {} states",
                    grammar.summary.rule_count, grammar.summary.state_count
                ))
                .size(10)
                .color(theme::MUTED),
            ]
            .spacing(3),
            state.selection == Selection::Grammar,
            Message::SelectGrammar,
        )
    } else {
        container(
            column![
                text("No grammar open").size(13).color(theme::MUTED),
                text("Use “Open grammar…” below")
                    .size(10)
                    .color(theme::MUTED),
            ]
            .spacing(3),
        )
        .padding(10)
        .width(Length::Fill)
        .into()
    };

    let mut documents = Column::new()
        .spacing(4)
        .push(text("DOCUMENTS").size(10).color(theme::MUTED))
        .push(grammar_row);
    for parse in &state.parses {
        let id = parse.id;
        let content = column![
            container(
                text(format!("#{}  {}", parse.id, parse.label))
                    .size(12)
                    .wrapping(text::Wrapping::None),
            )
            .clip(true)
            .width(Length::Fill),
            text(parse.language.sidebar_status())
                .size(10)
                .color(match parse.language.status {
                    LanguageStatus::Error(_) => theme::DANGER,
                    _ => theme::MUTED,
                }),
        ]
        .spacing(3)
        .padding(iced::Padding {
            top: 0.0,
            right: 22.0,
            bottom: 0.0,
            left: 0.0,
        });
        let select = document_button(
            content,
            state.selection == Selection::Parse(id),
            Message::SelectParse(id),
        );
        let remove = container(
            button(text("×").size(14))
                .padding([1, 7])
                .style(theme::quiet_button)
                .on_press(Message::RemoveParse(id)),
        )
        .align_right(Length::Fill)
        .center_y(Length::Fill)
        .padding([0, 6]);
        documents = documents.push(stack![select, remove]);
    }

    let parse_button = button(text("+ Parse").size(13))
        .width(Length::Fill)
        .padding([10, 18])
        .style(theme::parse_button);
    let parse_button = if state.grammar.is_some() && state.busy.is_none() {
        parse_button.on_press(Message::NewParse)
    } else {
        parse_button
    };

    container(
        column![
            scrollable(documents.padding([12, 0])).height(Length::Fill),
            parse_button,
        ]
        .padding([12, 10])
        .spacing(8)
        .height(Length::Fill),
    )
    .width(theme::SIDEBAR_WIDTH)
    .height(Length::Fill)
    .style(theme::sidebar)
    .into()
}

fn document_button<'a>(
    content: impl Into<Element<'a, Message>>,
    selected: bool,
    message: Message,
) -> Element<'a, Message> {
    button(content)
        .width(Length::Fill)
        .padding([8, 10])
        .style(if selected {
            theme::selected_button
        } else {
            theme::quiet_button
        })
        .on_press(message)
        .into()
}

/// The merged view bar: the primary selector on the left plus, on the Language
/// view, the interpretation tabs and zoom controls. Sits below the page heading
/// and directly above the content it controls, with a baseline rule.
fn view_bar<'a>(
    primary_label: &'a str,
    active_tab: DocumentTab,
    language_ready: bool,
    extra: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut bar = row![view_selector(primary_label, active_tab, language_ready)]
        .align_y(Alignment::Center)
        .spacing(24);
    if let Some(extra) = extra {
        bar = bar.push(extra);
    }
    bar.into()
}

/// Prominent two-segment toggle for the primary Grammar/Chart ↔ Language switch.
fn view_selector<'a>(
    primary_label: &'a str,
    active_tab: DocumentTab,
    language_ready: bool,
) -> Element<'a, Message> {
    const R: f32 = 6.0;
    const SEGMENT_WIDTH: f32 = 104.0;
    let segment_label = |label: &str| {
        text(label.to_owned())
            .size(13)
            .width(Length::Fill)
            .align_x(Alignment::Center)
    };
    let primary = button(segment_label(primary_label))
        .width(Length::Fixed(SEGMENT_WIDTH))
        .padding([7, 10])
        .style(theme::segment(active_tab == DocumentTab::Primary, [R, 0.0, 0.0, R]))
        .on_press(Message::SelectTab(DocumentTab::Primary));

    let language_active = language_ready && active_tab == DocumentTab::Language;
    let language = button(segment_label("Language"))
        .width(Length::Fixed(SEGMENT_WIDTH))
        .padding([7, 10])
        .style(theme::segment(language_active, [0.0, R, R, 0.0]));
    let language = if language_ready {
        language.on_press(Message::SelectTab(DocumentTab::Language))
    } else {
        language
    };

    row![primary, language].into()
}

fn workspace(state: &Workbench) -> Element<'_, Message> {
    match state.selection {
        Selection::NewParse => parse_page(state),
        Selection::Grammar => match state.active_tab {
            DocumentTab::Primary => grammar_page(state),
            DocumentTab::Language => state
                .grammar_language
                .as_ref()
                .map(|language| {
                    language_page(
                        language,
                        state
                            .grammar
                            .as_ref()
                            .map(|grammar| format!("Grammar: {}", display_name(&grammar.path)))
                            .unwrap_or_else(|| "Grammar".into()),
                        "Grammar",
                    )
                })
                .unwrap_or_else(|| empty_state("No language", "Open a grammar first.", None)),
        },
        Selection::Parse(id) => {
            let Some(parse) = state.parse(id) else {
                return empty_state("Parse removed", "Choose another document.", None);
            };
            match state.active_tab {
                DocumentTab::Primary => chart_page(parse),
                DocumentTab::Language => language_page(
                    &parse.language,
                    format!("#{}  {}", parse.id, parse.label),
                    "Chart",
                ),
            }
        }
    }
}

fn grammar_page(state: &Workbench) -> Element<'_, Message> {
    let Some(grammar) = &state.grammar else {
        return empty_state(
            "Open a grammar",
            "Load an IRTG grammar to begin.",
            Some(("Open grammar…", Message::OpenGrammar)),
        );
    };
    let ready = state.active_language().is_some_and(LanguageSession::ready);
    page(
        column![
            page_heading(
                format!("Grammar: {}", display_name(&grammar.path)),
                format!(
                    "{} rules · {} states · maximum rank {}",
                    grammar.summary.rule_count,
                    grammar.summary.state_count,
                    grammar.summary.maximum_rank
                ),
            ),
            view_bar("Grammar", DocumentTab::Primary, ready, None),
            rule_table(
                &grammar.rules,
                &grammar.interpretation_names,
                false,
                Message::SortGrammar,
            ),
        ]
        .spacing(theme::SECTION_SPACING),
    )
}

fn chart_page(parse: &ParseSession) -> Element<'_, Message> {
    let id = parse.id;
    page(
        column![
            page_heading(
                format!("#{}  {}", parse.id, parse.label),
                format!(
                    "{} chart rules · {} states · built in {:.2?}",
                    parse.chart.summary.rule_count,
                    parse.chart.summary.state_count,
                    parse.chart.elapsed
                ),
            ),
            view_bar("Chart", DocumentTab::Primary, parse.language.ready(), None),
            rule_table(&parse.chart.rules, &[], true, move |column| {
                Message::SortChart(id, column)
            }),
        ]
        .spacing(theme::SECTION_SPACING),
    )
}

fn parse_page(state: &Workbench) -> Element<'_, Message> {
    let mut fields = Column::new().spacing(10);
    for (index, input) in state.inputs.iter().enumerate() {
        let mut field = text_input("Optional interpretation input", &input.value)
            .on_input(move |value| Message::InputChanged(index, value))
            .padding(9)
            .size(13);
        if state.busy.is_none() {
            field = field.on_submit(Message::Parse);
        }
        fields = fields.push(
            column![text(&input.name).size(12).color(theme::MUTED), field].spacing(5),
        );
    }
    let mut options = column![
        text("Parsing algorithm").size(12).color(theme::MUTED),
        pick_list(
            StrategyChoice::ALL,
            Some(state.strategy),
            Message::StrategyChanged
        )
        .width(Length::Fill),
    ]
    .spacing(6);
    if state.strategy == StrategyChoice::Astar {
        let mut beam = text_input("Optional beam, e.g. 0.001", &state.beam)
            .on_input(Message::BeamChanged)
            .padding(9);
        if state.busy.is_none() {
            beam = beam.on_submit(Message::Parse);
        }
        options = options
            .push(
                checkbox("Stop after first goal", state.stop_at_first_goal)
                    .on_toggle(Message::StopAtFirstGoal)
                    .size(15),
            )
            .push(beam);
    }
    let parse_button = button(text(if state.busy.is_some() {
        "Parsing…"
    } else {
        "Run parser"
    }))
    .padding([9, 16])
    .style(button::primary);
    let parse_button = if state.busy.is_none() {
        parse_button.on_press(Message::Parse)
    } else {
        parse_button
    };
    page(
        column![
            page_heading(
                "Parse new input",
                "Provide one or more interpretation values, then choose a chart construction strategy.",
            ),
            container(fields)
                .padding(14)
                .width(Length::Fill)
                .style(theme::raised),
            container(options)
                .padding(14)
                .width(Length::Fill)
                .style(theme::raised),
            row![horizontal_space(), parse_button],
        ]
        .spacing(theme::SECTION_SPACING)
        .max_width(760),
    )
}

fn language_page<'a>(
    language: &'a LanguageSession,
    title: String,
    primary_label: &'a str,
) -> Element<'a, Message> {
    let derivation = match &language.status {
        LanguageStatus::Ready(LanguageCardinality::Finite(0)) => None,
        LanguageStatus::Ready(_) => language.derivations.get(language.derivation_index),
        _ => None,
    };

    // Each state resolves to a subtitle, optional derivation nav, optional
    // interpretation toolbar (tabs + zoom), and the body panel.
    type Bits<'b> = (
        String,
        Option<Element<'b, Message>>,
        Option<Element<'b, Message>>,
        Element<'b, Message>,
    );
    let (subtitle, nav, extra, body): Bits<'a> = match (&language.status, derivation) {
        (LanguageStatus::Preparing, _) => (
            "Preparing…".into(),
            None,
            None,
            message_panel(
                "Preparing language…",
                "Initializing the sorted language iterator in the background.",
            ),
        ),
        (LanguageStatus::Error(error), _) => (
            "Could not prepare".into(),
            None,
            None,
            message_panel("Could not prepare language", error),
        ),
        (LanguageStatus::Ready(LanguageCardinality::Finite(0)), _) => (
            "Empty language".into(),
            None,
            None,
            message_panel(
                "Language is empty",
                "This automaton accepts no derivation trees.",
            ),
        ),
        (LanguageStatus::Ready(_), None) => (
            "Loading…".into(),
            None,
            None,
            message_panel(
                "Loading first derivation…",
                "Evaluating interpretations and preparing the derivation tree.",
            ),
        ),
        (LanguageStatus::Ready(_), Some(derivation)) => {
            let output_index = language
                .output_index
                .min(derivation.views.len().saturating_sub(1));
            let output = &derivation.views[output_index];
            let value_is_tree = output.tree.is_some();
            let value: Element<'a, Message> = if let Some(layout) = &output.tree {
                tree_view(layout.clone(), language.zoom)
            } else {
                container(text(&output.value).size(15))
                    .width(Length::Fill)
                    .into()
            };
            let body: Element<'a, Message> = if let Some(term) = &output.term {
                // A string value only needs its own height; a tree value shares
                // the panel with the term tree.
                let (value_height, term_height) = if value_is_tree {
                    (Length::FillPortion(3), Length::FillPortion(2))
                } else {
                    (Length::Shrink, Length::Fill)
                };
                container(
                    column![
                        panel_section("Value", value, value_height),
                        horizontal_rule(1).style(theme::separator),
                        panel_section("Term", tree_view(term.clone(), language.zoom), term_height),
                    ]
                    .height(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::raised)
                .into()
            } else {
                container(value)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(theme::raised)
                    .into()
            };

            let previous = button(text("‹").size(18)).style(theme::quiet_button);
            let previous = if language.derivation_index > 0 {
                previous.on_press(Message::PreviousDerivation)
            } else {
                previous
            };
            let next = button(text("›").size(18)).style(theme::quiet_button);
            let next = if language.has_next() {
                next.on_press(Message::NextDerivation)
            } else {
                next
            };
            let nav = row![previous, next].align_y(Alignment::Center).spacing(2);

            // Interpretation-view tabs sit in the bar, right above the tree.
            let mut tabs = Row::new().spacing(4).align_y(Alignment::Center);
            for (index, item) in derivation.views.iter().enumerate() {
                tabs = tabs.push(
                    button(text(&item.name).size(12))
                        .padding([6, 12])
                        .style(if index == output_index {
                            theme::selected_button
                        } else {
                            theme::quiet_button
                        })
                        .on_press(Message::SelectOutput(index)),
                );
            }
            let zoom = row![
                button(text("−").size(15))
                    .style(theme::quiet_button)
                    .on_press(Message::ZoomOut),
                button(text(format!("{}%", (language.zoom * 100.0).round() as i32)).size(12))
                    .style(theme::quiet_button)
                    .on_press(Message::ZoomReset),
                button(text("+").size(15))
                    .style(theme::quiet_button)
                    .on_press(Message::ZoomIn),
            ]
            .spacing(2)
            .align_y(Alignment::Center);
            let extra = row![tabs, horizontal_space(), zoom]
                .width(Length::Fill)
                .align_y(Alignment::Center);

            (
                format!(
                    "#{} of {} · weight {:.6}",
                    language.derivation_index + 1,
                    language.size_label(),
                    derivation.weight
                ),
                Some(nav.into()),
                Some(extra.into()),
                body,
            )
        }
    };

    let mut heading = row![page_heading(title, subtitle), horizontal_space()]
        .align_y(Alignment::Center)
        .spacing(5);
    if let Some(nav) = nav {
        heading = heading.push(nav);
    }

    page(
        column![
            heading,
            view_bar(primary_label, DocumentTab::Language, true, extra),
            body,
        ]
        .spacing(theme::SECTION_SPACING),
    )
}

/// A labeled section ("Value" / "Term") inside the language content panel.
/// The body should fill width; pass `Length::Shrink` for a content-sized
/// section (e.g. a string value) or `Length::Fill`/`FillPortion` for a tree.
fn panel_section<'a>(
    title: &'a str,
    body: Element<'a, Message>,
    height: Length,
) -> Element<'a, Message> {
    column![
        text(title.to_uppercase()).size(10).color(theme::MUTED),
        body,
    ]
    .spacing(6)
    .padding([10, 14])
    .width(Length::Fill)
    .height(height)
    .into()
}

/// A centered status message filling the content panel (loading / error / empty).
fn message_panel<'a>(title: &'a str, detail: &'a str) -> Element<'a, Message> {
    container(
        column![
            text(title).size(18),
            text(detail).size(13).color(theme::MUTED),
        ]
        .align_x(Alignment::Center)
        .spacing(8),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(theme::raised)
    .into()
}

fn status_bar(state: &Workbench) -> Element<'_, Message> {
    let (marker, status, color) = if let Some(busy) = &state.busy {
        ("●", busy.as_str(), theme::ACCENT)
    } else if let Some(error) = &state.error {
        ("●", error.as_str(), theme::DANGER)
    } else {
        ("●", "Ready", theme::SUCCESS)
    };
    container(
        row![text(marker).size(10).color(color), text(status).size(11)]
            .align_y(Alignment::Center)
            .spacing(7)
            .padding([0, 12]),
    )
    .center_y(26)
    .width(Length::Fill)
    .style(theme::flat)
    .into()
}

fn rule_table<'a>(
    rows: &'a [RuleRow],
    interpretations: &'a [String],
    mute_spans: bool,
    sort: impl Fn(RuleColumn) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    let rule_portion = if interpretations.is_empty() { 9 } else { 6 };
    const WEIGHT_PORTION: u16 = 1;
    const INTERP_PORTION: u16 = 3;

    let mut header = row![
        table_header("Rule", rule_portion, RuleColumn::Rule, sort),
        table_header("Weight", WEIGHT_PORTION, RuleColumn::Weight, sort),
    ]
    .spacing(12)
    .padding([0, 10])
    .align_y(Alignment::Center);
    for name in interpretations {
        header = header.push(
            container(text(name).size(12).color(theme::MUTED))
                .width(Length::FillPortion(INTERP_PORTION)),
        );
    }

    let mut body = Column::new().spacing(0);
    for (index, item) in rows.iter().enumerate() {
        let mut cells = row![
            container(rule_cell(&item.parent, &item.rhs, mute_spans))
                .width(Length::FillPortion(rule_portion)),
            table_cell_owned(format_weight(item.weight), WEIGHT_PORTION),
        ]
        .spacing(12)
        .padding([0, 10])
        .align_y(Alignment::Center);
        for column in 0..interpretations.len() {
            let term = item.interpretations.get(column).map(String::as_str).unwrap_or("");
            cells = cells.push(table_cell(term, INTERP_PORTION));
        }
        body = body.push(
            container(cells)
                .center_y(theme::TABLE_ROW_HEIGHT)
                .width(Length::Fill)
                .style(move |_| iced::widget::container::Style {
                    background: Some(if index % 2 == 0 { theme::BG } else { theme::SURFACE }.into()),
                    text_color: Some(theme::TEXT),
                    ..Default::default()
                }),
        );
    }
    container(column![
        container(header)
            .center_y(34)
            .width(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(theme::SURFACE.into()),
                ..Default::default()
            }),
        horizontal_rule(1).style(theme::separator),
        scrollable(body).height(Length::Fill),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    // Inset content past the 1px border and clip to the rounded corners so the
    // zebra rows don't paint over the panel edge.
    .padding(1)
    .clip(true)
    .style(theme::panel)
    .into()
}

/// "parent → rhs" for a rule. On the chart, the `[i,j]` position spans are
/// muted so the rule structure stands out.
fn rule_cell<'a>(parent: &str, rhs: &str, mute_spans: bool) -> Element<'a, Message> {
    let full = format!("{parent}  →  {rhs}");
    if !mute_spans {
        return text(full).size(14).into();
    }
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut buf_muted = false;
    let mut depth: u32 = 0;
    for ch in full.chars() {
        let muted = match ch {
            '[' => {
                depth += 1;
                true
            }
            ']' => {
                let was = depth > 0;
                depth = depth.saturating_sub(1);
                was
            }
            _ => depth > 0,
        };
        if !buf.is_empty() && muted != buf_muted {
            let color = if buf_muted { theme::MUTED } else { theme::TEXT };
            spans.push(span(std::mem::take(&mut buf)).color(color));
        }
        buf_muted = muted;
        buf.push(ch);
    }
    if !buf.is_empty() {
        let color = if buf_muted { theme::MUTED } else { theme::TEXT };
        spans.push(span(buf).color(color));
    }
    rich_text(spans).size(14).into()
}

fn format_weight(weight: f64) -> String {
    if weight.fract() == 0.0 {
        format!("{}", weight as i64)
    } else {
        format!("{weight:.4}")
    }
}

fn table_header<'a>(
    label: &'a str,
    width: u16,
    column: RuleColumn,
    sort: impl Fn(RuleColumn) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    button(text(format!("{label}  ↕")).size(12).color(theme::MUTED))
        .width(Length::FillPortion(width))
        .padding(0)
        .style(theme::quiet_button)
        .on_press(sort(column))
        .into()
}

fn table_cell<'a>(value: &'a str, width: u16) -> Element<'a, Message> {
    container(text(value).size(14))
        .width(Length::FillPortion(width))
        .into()
}

fn table_cell_owned(value: String, width: u16) -> Element<'static, Message> {
    container(text(value).size(14))
        .width(Length::FillPortion(width))
        .into()
}

fn page<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        // Smaller bottom inset so the panel bottom lines up with the sidebar's
        // "+ Parse" button and sits closer to the status bar.
        .padding(iced::Padding {
            top: theme::PAGE_PADDING,
            right: theme::PAGE_PADDING,
            bottom: 12.0,
            left: theme::PAGE_PADDING,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::workspace)
        .into()
}

fn page_heading<'a>(title: impl Into<String>, subtitle: impl Into<String>) -> Element<'a, Message> {
    column![
        text(title.into()).size(19),
        text(subtitle.into()).size(12).color(theme::MUTED),
    ]
    .spacing(3)
    .into()
}

fn empty_state<'a>(
    title: &'a str,
    detail: &'a str,
    action: Option<(&'a str, Message)>,
) -> Element<'a, Message> {
    let mut content = column![
        text("◇").size(30).color(theme::ACCENT),
        text(title).size(20),
        text(detail).size(13).color(theme::MUTED),
    ]
    .align_x(Alignment::Center)
    .spacing(8);
    if let Some((label, message)) = action {
        content = content.push(
            button(text(label))
                .padding([9, 14])
                .style(button::primary)
                .on_press(message),
        );
    }
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(theme::workspace)
        .into()
}

impl Workbench {
    /// Apply a loaded grammar (or its error) to this window.
    fn apply_grammar(&mut self, result: Result<GrammarDocument, String>) {
        self.busy = None;
        match result {
            Ok(document) => {
                let (sender, receiver) = mpsc::channel();
                let worker =
                    workers::start_grammar_language_worker(document.grammar.clone(), sender);
                self.inputs = input_fields(&document);
                self.grammar = Some(document);
                self.grammar_language = Some(LanguageSession::preparing(worker, receiver));
                self.parses.clear();
                self.next_parse_id = 1;
                self.selection = Selection::Grammar;
                self.active_tab = DocumentTab::Primary;
                self.error = None;
            }
            Err(error) => self.fail(format!("Could not load grammar: {error}")),
        }
    }

    /// Drain background language events for this window's grammar and parses.
    fn poll(&mut self) {
        if let Some(language) = &mut self.grammar_language {
            poll_language(language);
        }
        for parse in &mut self.parses {
            poll_language(&mut parse.language);
        }
    }

    /// Whether any language iterator is still feeding this window events.
    fn has_pending_language(&self) -> bool {
        self.grammar_language
            .as_ref()
            .is_some_and(|language| language.receiver.is_some())
            || self
                .parses
                .iter()
                .any(|parse| parse.language.receiver.is_some())
    }

    fn parse(&self, id: u64) -> Option<&ParseSession> {
        self.parses.iter().find(|parse| parse.id == id)
    }

    fn parse_mut(&mut self, id: u64) -> Option<&mut ParseSession> {
        self.parses.iter_mut().find(|parse| parse.id == id)
    }

    fn active_language(&self) -> Option<&LanguageSession> {
        match self.selection {
            Selection::Grammar => self.grammar_language.as_ref(),
            Selection::Parse(id) => self.parse(id).map(|parse| &parse.language),
            Selection::NewParse => None,
        }
    }

    fn active_language_mut(&mut self) -> Option<&mut LanguageSession> {
        match self.selection {
            Selection::Grammar => self.grammar_language.as_mut(),
            Selection::Parse(id) => self.parse_mut(id).map(|parse| &mut parse.language),
            Selection::NewParse => None,
        }
    }

    fn fail(&mut self, message: String) {
        self.error = Some(message);
    }
}

fn input_fields(grammar: &GrammarDocument) -> Vec<InputField> {
    grammar
        .interpretations
        .iter()
        .filter(|info| info.input_capable)
        .map(|info| InputField {
            name: info.name.clone(),
            value: String::new(),
        })
        .collect()
}

fn parse_label(inputs: &[(String, String)]) -> String {
    inputs
        .iter()
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn sort_rows(rows: &mut [RuleRow], column: RuleColumn) {
    rows.sort_by(|a, b| match column {
        RuleColumn::Rule => a.parent.cmp(&b.parent).then_with(|| a.rhs.cmp(&b.rhs)),
        RuleColumn::Weight => a.weight.total_cmp(&b.weight),
    });
}

fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("<grammar>"))
        .to_owned()
}

fn keyboard_shortcut(key: Key, modifiers: Modifiers) -> Option<Message> {
    match key.as_ref() {
        Key::Character("o") if modifiers.command() => Some(Message::ShortcutOpenGrammar),
        Key::Named(Named::ArrowLeft) if modifiers.is_empty() => Some(Message::ShortcutPrevious),
        Key::Named(Named::ArrowRight) if modifiers.is_empty() => Some(Message::ShortcutNext),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_labels_preserve_the_users_input() {
        let inputs = vec![
            ("english".into(), "john watches".into()),
            ("german".into(), "hans betrachtet".into()),
        ];
        assert_eq!(parse_label(&inputs), "john watches · hans betrachtet");
    }

    #[test]
    fn standard_keyboard_shortcuts_are_cross_platform() {
        assert!(matches!(
            keyboard_shortcut(Key::Character("o".into()), Modifiers::COMMAND),
            Some(Message::ShortcutOpenGrammar)
        ));
        assert!(matches!(
            keyboard_shortcut(Key::Named(Named::ArrowLeft), Modifiers::empty()),
            Some(Message::ShortcutPrevious)
        ));
    }
}
