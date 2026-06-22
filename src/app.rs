use crate::{
    app_state::{
        CopyMenu, LanguageSession, LanguageStatus, ParseJob, ParseSession, Selection, Workbench,
    },
    feature_canvas::feature_structure_view,
    model::{
        ChartDocument, DocumentTab, GrammarDocument, HeuristicChoice, InputField, PresentationMode,
        RuleColumn, RuleRow, StrategyChoice, ValuePresentation, ViewContent,
    },
    theme,
    tree_canvas::tree_view,
    workers::{self, LanguageEvent},
};
use iced::{
    Alignment, Element, Event, Length, Point, Subscription, Task, clipboard, event,
    keyboard::{Key, Modifiers, key::Named},
    mouse,
    widget::{
        Column, Row, button, checkbox, column, container, mouse_area, pick_list, rich_text, row,
        rule, scrollable, space, span, stack, text, text_input, tooltip,
    },
    window,
};
use rusty_alto::{
    CodecMetadata, EvaluatedAlgebraValue, InputCodecRegistry, Irtg, LanguageCardinality,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

pub fn run() -> iced::Result {
    iced::daemon(
        || {
            let mut app = App::default();
            let (id, open) = window::open(window_settings());
            app.windows.insert(id, Workbench::default());
            (app, open.map(AppMsg::WindowOpened))
        },
        app_update,
        app_view,
    )
    .title(app_title)
    .theme(iced::Theme::Light)
    .font(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice())
    .default_font(iced::Font {
        family: iced::font::Family::Name("Inter"),
        weight: iced::font::Weight::Medium,
        ..iced::Font::DEFAULT
    })
    .subscription(app_subscription)
    .run()
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
    GrammarLoaded {
        asking: window::Id,
        target: window::Id,
        opened_new: bool,
        result: Result<GrammarDocument, String>,
    },
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
        None => space::horizontal().into(),
    }
}

fn app_update(app: &mut App, message: AppMsg) -> Task<AppMsg> {
    match message {
        // Opening a grammar is an app-level action: it may spawn a new window.
        AppMsg::Window(id, Message::OpenGrammar | Message::ShortcutOpenGrammar) => {
            let extensions = InputCodecRegistry::standard()
                .codecs_for::<Irtg>()
                .iter()
                .filter_map(|codec| codec.metadata().extension)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .add_filter("IRTG grammars", &extensions)
                        .pick_file()
                        .await
                        .map(|handle| handle.path().to_owned())
                },
                move |path| AppMsg::GrammarPicked(id, path),
            )
        }
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
            let (target, opened_new, open_task) = if needs_new_window {
                let (new_id, open) = window::open(window_settings());
                app.windows.insert(new_id, Workbench::default());
                (new_id, true, open.map(AppMsg::WindowOpened))
            } else {
                (asking_id, false, Task::none())
            };
            if let Some(window) = app.windows.get_mut(&target) {
                window.busy = Some(format!("Loading {}…", display_name(&path)));
                window.error = None;
            }
            let load = Task::perform(async move { workers::load_grammar(path) }, move |result| {
                AppMsg::GrammarLoaded {
                    asking: asking_id,
                    target,
                    opened_new,
                    result,
                }
            });
            Task::batch([open_task, load])
        }
        AppMsg::GrammarLoaded {
            asking,
            target,
            opened_new,
            result,
        } => match result {
            Ok(document) => {
                if let Some(window) = app.windows.get_mut(&target) {
                    window.apply_grammar(Ok(document));
                }
                #[cfg(target_os = "macos")]
                sync_view_menu(app);
                Task::none()
            }
            Err(error) if opened_new => {
                app.windows.remove(&target);
                if let Some(window) = app.windows.get_mut(&asking) {
                    window.busy = None;
                    window.fail(format!("Could not load grammar: {error}"));
                }
                window::close(target)
            }
            Err(error) => {
                if let Some(window) = app.windows.get_mut(&target) {
                    window.apply_grammar(Err(error));
                }
                Task::none()
            }
        },
        AppMsg::WindowOpened(id) => {
            // Install the native menu bar once, now that NSApp is running on the
            // main thread (this update runs on the winit/main thread), and add
            // the new window to the Window menu.
            #[cfg(target_os = "macos")]
            {
                if !app.menu_installed {
                    app.menu_installed = true;
                    crate::platform_menu::install();
                }
                crate::platform_menu::refresh_windows_menu();
                sync_view_menu(app);
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
            sync_view_menu(app);
            Task::none()
        }
        #[cfg(target_os = "macos")]
        AppMsg::MenuPoll => {
            let mut tasks = Vec::new();
            while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
                match event.id.0.as_str() {
                    crate::platform_menu::OPEN_GRAMMAR_ID => {
                        // Open into the focused window (its handler decides
                        // whether to reuse the window or spawn a new one).
                        let target = app.focused.or_else(|| app.windows.keys().next().copied());
                        if let Some(id) = target {
                            tasks.push(app_update(app, AppMsg::Window(id, Message::OpenGrammar)));
                        }
                    }
                    crate::platform_menu::NEW_PARSE_ID => {
                        if let Some(id) = app.focused.or_else(|| app.windows.keys().next().copied())
                        {
                            tasks.push(app_update(app, AppMsg::Window(id, Message::NewParse)));
                        }
                    }
                    crate::platform_menu::CLOSE_ALL_ID => {
                        app.windows.clear();
                        tasks.push(iced::exit());
                    }
                    crate::platform_menu::KEYBOARD_SHORTCUTS_ID => {
                        if let Some(id) = app.focused.or_else(|| app.windows.keys().next().copied())
                        {
                            tasks.push(app_update(app, AppMsg::Window(id, Message::ShowShortcuts)));
                        }
                    }
                    crate::platform_menu::VIEW_TAG_ID => {
                        if let Some(id) = app.focused.or_else(|| app.windows.keys().next().copied())
                        {
                            tasks.push(app_update(
                                app,
                                AppMsg::Window(
                                    id,
                                    Message::SetPresentationMode(PresentationMode::Tag),
                                ),
                            ));
                            sync_view_menu(app);
                        }
                    }
                    crate::platform_menu::VIEW_IRTG_ID => {
                        if let Some(id) = app.focused.or_else(|| app.windows.keys().next().copied())
                        {
                            tasks.push(app_update(
                                app,
                                AppMsg::Window(
                                    id,
                                    Message::SetPresentationMode(PresentationMode::RawIrtg),
                                ),
                            ));
                            sync_view_menu(app);
                        }
                    }
                    _ => {}
                }
            }
            Task::batch(tasks)
        }
    }
}

#[cfg(target_os = "macos")]
fn sync_view_menu(app: &App) {
    let state = app
        .focused
        .and_then(|id| app.windows.get(&id))
        .or_else(|| app.windows.values().next());
    let tag_available = state
        .and_then(|state| state.grammar.as_ref())
        .is_some_and(|grammar| grammar.detected_mode == PresentationMode::Tag);
    let grammar_loaded = state.is_some_and(|state| state.grammar.is_some());
    let mode = state.map(|state| state.presentation_mode);
    crate::platform_menu::update_view_mode(
        grammar_loaded,
        tag_available,
        mode == Some(PresentationMode::Tag),
        mode == Some(PresentationMode::RawIrtg),
    );
}

fn app_subscription(app: &App) -> Subscription<AppMsg> {
    let needs_poll = app
        .windows
        .values()
        .any(|window| window.has_pending_language() || window.active_parse.is_some());
    let polling = if needs_poll {
        iced::time::every(Duration::from_millis(80)).map(|_| AppMsg::Poll)
    } else {
        Subscription::none()
    };
    // Route keyboard shortcuts to whichever window currently has focus, and
    // remember the focused window for app-level menu actions (macOS).
    let events = event::listen_with(|event, status, id| match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            if status == event::Status::Captured {
                None
            } else if key == Key::Named(Named::Escape) {
                Some(AppMsg::Window(id, Message::CloseCopyMenu))
            } else {
                keyboard_shortcut(key, modifiers).map(|message| AppMsg::Window(id, message))
            }
        }
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(AppMsg::Window(id, Message::CursorMoved(position)))
        }
        Event::Window(window::Event::Resized(size)) => {
            Some(AppMsg::Window(id, Message::WindowResized(size)))
        }
        #[cfg(target_os = "macos")]
        Event::Window(window::Event::Focused) => Some(AppMsg::WindowFocused(id)),
        _ => None,
    });
    let closes = window::close_requests().map(AppMsg::CloseWindow);
    // `mut` is only used on macOS, where the menu poll is pushed below.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut subscriptions = vec![polling, events, closes];
    // Drain native menu activations (Open grammar / Close All Windows).
    #[cfg(target_os = "macos")]
    subscriptions.push(iced::time::every(Duration::from_millis(120)).map(|_| AppMsg::MenuPoll));
    Subscription::batch(subscriptions)
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
    RequireValidChanged(usize, bool),
    StrategyChanged(StrategyChoice),
    HeuristicChanged(HeuristicChoice),
    StopAtFirstGoal(bool),
    SetPresentationMode(PresentationMode),
    ShowTechnicalNodes(bool),
    FocusNext,
    FocusPrevious,
    Parse,
    CancelParse,
    Parsed(u64, Result<ChartDocument, String>),
    PreviousDerivation,
    NextDerivation,
    SelectOutput(usize),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ShortcutOpenGrammar,
    ShortcutPrevious,
    ShortcutNext,
    CursorMoved(Point),
    WindowResized(iced::Size),
    OpenCopyMenu(Arc<EvaluatedAlgebraValue>, Vec<CodecMetadata>),
    CloseCopyMenu,
    CopyWithCodec(String),
    ShowShortcuts,
    HideShortcuts,
}

fn update(state: &mut Workbench, message: Message) -> Task<Message> {
    if !matches!(
        message,
        Message::CursorMoved(_)
            | Message::WindowResized(_)
            | Message::OpenCopyMenu(_, _)
            | Message::CloseCopyMenu
            | Message::CopyWithCodec(_)
            | Message::ShowShortcuts
            | Message::HideShortcuts
    ) {
        state.copy_menu = None;
    }
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
                state.inputs = input_fields(grammar, state.presentation_mode);
                state.strategy = StrategyChoice::TopDown;
                state.heuristic = HeuristicChoice::Zero;
                state.stop_at_first_goal = false;
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
                if !field.value.trim().is_empty() {
                    field.require_valid = false;
                }
            }
            state.disable_unsupported_early_stop();
        }
        Message::RequireValidChanged(index, value) => {
            if let Some(field) = state.inputs.get_mut(index)
                && field.non_null_filter_capable
                && field.value.trim().is_empty()
            {
                field.require_valid = value;
            }
            state.disable_unsupported_early_stop();
        }
        Message::StrategyChanged(strategy) => state.strategy = strategy,
        Message::HeuristicChanged(heuristic) => state.heuristic = heuristic,
        Message::StopAtFirstGoal(value) => {
            state.stop_at_first_goal = value && state.constraint_count() <= 1;
        }
        Message::SetPresentationMode(mode) => {
            let compatible = state.grammar.as_ref().is_some_and(|grammar| {
                mode == PresentationMode::RawIrtg || grammar.detected_mode == PresentationMode::Tag
            });
            if compatible && state.presentation_mode != mode {
                state.presentation_mode = mode;
                state.show_technical_nodes = false;
                if let Some(grammar) = &state.grammar {
                    state.inputs = input_fields(grammar, mode);
                }
                if mode == PresentationMode::Tag {
                    state.strategy = StrategyChoice::TopDown;
                    state.heuristic = HeuristicChoice::Zero;
                    state.stop_at_first_goal = false;
                }
                if let Some(language) = &mut state.grammar_language {
                    language.output_index = 0;
                }
                for parse in &mut state.parses {
                    parse.language.output_index = 0;
                }
            }
        }
        Message::ShowTechnicalNodes(value) => state.show_technical_nodes = value,
        Message::FocusNext => return iced::widget::operation::focus_next(),
        Message::FocusPrevious => return iced::widget::operation::focus_previous(),
        Message::Parse => {
            if state.active_parse.is_some() {
                return Task::none();
            }
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
            let required_valid = state
                .inputs
                .iter()
                .filter(|input| input.require_valid && input.value.trim().is_empty())
                .map(|input| input.name.clone())
                .collect::<Vec<_>>();
            let mut required_valid = required_valid;
            if state.presentation_mode == PresentationMode::Tag
                && state.grammar.as_ref().is_some_and(|grammar| {
                    grammar
                        .interpretations
                        .iter()
                        .any(|info| info.name == "ft" && info.non_null_filter_capable)
                })
            {
                required_valid.push("ft".into());
            }
            if state.presentation_mode == PresentationMode::Tag && inputs.is_empty() {
                state.fail("Enter a sentence to parse.".into());
                return Task::none();
            }
            if inputs.is_empty() && required_valid.is_empty() {
                state.fail(
                    "Enter an interpretation input or require at least one valid value.".into(),
                );
                return Task::none();
            }
            state.pending_label = Some(if state.presentation_mode == PresentationMode::Tag {
                parse_label(&inputs, &[])
            } else {
                parse_label(&inputs, &required_valid)
            });
            state.error = None;
            let job_id = state.next_parse_job_id;
            state.next_parse_job_id += 1;
            let control = rusty_alto::ParseControl::new();
            state.active_parse = Some(ParseJob {
                id: job_id,
                started: Instant::now(),
                control: control.clone(),
            });
            let (strategy, heuristic, stop_at_first_goal) =
                if state.presentation_mode == PresentationMode::Tag {
                    (StrategyChoice::TopDown, HeuristicChoice::Zero, false)
                } else {
                    (
                        state.strategy,
                        state.heuristic,
                        state.stop_at_first_goal && state.constraint_count() <= 1,
                    )
                };
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        workers::parse_controlled(
                            grammar,
                            inputs,
                            required_valid,
                            strategy,
                            heuristic,
                            stop_at_first_goal,
                            control,
                        )
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(format!("The parser worker stopped unexpectedly: {error}"))
                    })
                },
                move |result| Message::Parsed(job_id, result),
            );
        }
        Message::CancelParse => {
            if let Some(job) = &state.active_parse {
                job.control.cancel();
            }
            state.active_parse = None;
            state.pending_label = None;
        }
        Message::Parsed(job_id, result) => {
            if state
                .active_parse
                .as_ref()
                .is_none_or(|job| job.id != job_id)
            {
                return Task::none();
            }
            state.active_parse = None;
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
                } else if language.has_next()
                    && !language.request_pending
                    && let Some(worker) = &language.worker
                {
                    language.request_pending = true;
                    worker.request(language.derivation_index + 1);
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
        Message::CursorMoved(position) => state.cursor_position = position,
        Message::WindowResized(size) => state.viewport_size = size,
        Message::OpenCopyMenu(value, codecs) => {
            let position =
                clamp_menu_position(state.cursor_position, state.viewport_size, codecs.len());
            state.copy_menu = Some(CopyMenu {
                position,
                value,
                codecs,
            });
        }
        Message::CloseCopyMenu => {
            state.copy_menu = None;
            state.show_shortcuts = false;
        }
        Message::CopyWithCodec(codec_name) => {
            let encoded = state
                .copy_menu
                .as_ref()
                .map(|menu| menu.value.encode(&codec_name));
            state.copy_menu = None;
            match encoded {
                Some(Ok(text)) => return clipboard::write(text),
                Some(Err(error)) => state.fail(format!("Could not copy value: {error}")),
                None => {}
            }
        }
        Message::ShowShortcuts => {
            state.copy_menu = None;
            state.show_shortcuts = true;
        }
        Message::HideShortcuts => state.show_shortcuts = false,
    }
    Task::none()
}

const COPY_MENU_WIDTH: f32 = 250.0;

fn clamp_menu_position(cursor: Point, viewport: iced::Size, item_count: usize) -> Point {
    let margin = 8.0;
    let menu_height = 34.0 + item_count as f32 * 36.0;
    Point::new(
        (cursor.x + 4.0)
            .min((viewport.width - COPY_MENU_WIDTH - margin).max(margin))
            .max(margin),
        (cursor.y + 4.0)
            .min((viewport.height - menu_height - margin).max(margin))
            .max(margin),
    )
}

fn poll_language(language: &mut LanguageSession) -> bool {
    if !language.polling() {
        return false;
    }
    let mut disconnected = false;
    let mut events = Vec::new();
    if let Some(receiver) = &language.receiver {
        for _ in 0..32 {
            match receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
    }
    let changed = !events.is_empty();
    for event in events {
        match event {
            LanguageEvent::Ready(size) => {
                language.status = LanguageStatus::Ready(size);
                if size == LanguageCardinality::Finite(0) {
                    language.receiver = None;
                    language.worker = None;
                } else if let Some(worker) = &language.worker {
                    language.request_pending = true;
                    worker.request(0);
                }
            }
            LanguageEvent::Derivation(item) => {
                language.request_pending = false;
                let index = item.index;
                if index == language.derivations.len() {
                    language.derivations.push(item);
                } else if let Some(slot) = language.derivations.get_mut(index) {
                    *slot = item;
                }
                if index == language.derivation_index + 1 {
                    language.derivation_index = index;
                }
                if matches!(
                    language.status,
                    LanguageStatus::Ready(LanguageCardinality::Finite(size))
                        if language.derivations.len() >= size
                ) {
                    language.receiver = None;
                    language.worker = None;
                }
            }
            LanguageEvent::EndOfLanguage(count) => {
                language.status = LanguageStatus::Ready(LanguageCardinality::Finite(count));
                language.request_pending = false;
                language.receiver = None;
                language.worker = None;
            }
            LanguageEvent::Failed(error) => {
                language.status = LanguageStatus::Failed(error);
                language.request_pending = false;
                language.receiver = None;
                language.worker = None;
            }
        }
    }
    if disconnected && language.receiver.is_some() {
        language.status = LanguageStatus::Failed(
            "The background language worker disconnected unexpectedly.".into(),
        );
        language.request_pending = false;
        language.receiver = None;
        language.worker = None;
        return true;
    }
    changed
}

fn view(state: &Workbench) -> Element<'_, Message> {
    let body = row![sidebar(state), rule::vertical(1), workspace(state)].height(Length::Fill);

    let content = container(column![body, status_bar(state)])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::workspace);
    if state.show_shortcuts {
        stack![content, shortcuts_overlay()].into()
    } else if let Some(menu) = &state.copy_menu {
        stack![content, copy_menu_overlay(menu)].into()
    } else {
        content.into()
    }
}

fn shortcuts_overlay() -> Element<'static, Message> {
    let dismiss = mouse_area(
        container(space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::HideShortcuts);
    let shortcuts = [
        ("Open grammar", "⌘/Ctrl O"),
        ("New parse", "⌘/Ctrl P"),
        ("Next field", "Tab"),
        ("Previous field", "Shift Tab"),
        ("Previous derivation", "←"),
        ("Next derivation", "→"),
        ("Show this dialog", "?"),
        ("Dismiss dialog or menu", "Esc"),
    ];
    let mut rows = Column::new().spacing(8);
    for (action, keys) in shortcuts {
        rows = rows.push(
            row![
                text(action).size(13).width(Length::Fill),
                text(keys).size(12).color(theme::MUTED),
            ]
            .align_y(Alignment::Center),
        );
    }
    let panel = container(
        column![
            row![
                text("Keyboard shortcuts").size(19),
                space::horizontal(),
                button(text("Close").size(12))
                    .style(theme::quiet_button)
                    .on_press(Message::HideShortcuts),
            ]
            .align_y(Alignment::Center),
            text("Shortcuts apply to the focused Rusty Alto window.")
                .size(12)
                .color(theme::MUTED),
            rule::horizontal(1).style(theme::separator),
            rows,
        ]
        .spacing(12),
    )
    .width(Length::Fixed(430.0))
    .padding(18)
    .style(theme::raised);
    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);
    stack![dismiss, centered].into()
}

fn copy_menu_overlay(menu: &CopyMenu) -> Element<'_, Message> {
    let dismiss = mouse_area(
        container(space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::CloseCopyMenu)
    .on_right_press(Message::CloseCopyMenu);

    let mut items = Column::new()
        .spacing(2)
        .push(text("COPY VALUE").size(10).color(theme::MUTED));
    for codec in &menu.codecs {
        items = items.push(
            button(text(format!("Copy as {}", codec.description)).size(13))
                .padding([7, 10])
                .width(Length::Fill)
                .style(theme::quiet_button)
                .on_press(Message::CopyWithCodec(codec.name.to_owned())),
        );
    }
    let menu_panel = container(items)
        .width(Length::Fixed(COPY_MENU_WIDTH))
        .padding(8)
        .style(theme::raised);
    let positioned = column![
        space::vertical().height(Length::Fixed(menu.position.y)),
        row![
            space::horizontal().width(Length::Fixed(menu.position.x)),
            menu_panel
        ]
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    stack![dismiss, positioned].into()
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
                .color(theme::MUTED),
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
    let parse_button =
        if state.grammar.is_some() && state.busy.is_none() && state.active_parse.is_none() {
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
        .style(theme::segment(
            active_tab == DocumentTab::Primary,
            [R, 0.0, 0.0, R],
        ))
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
                        state.presentation_mode,
                        state.show_technical_nodes,
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
                    state.presentation_mode,
                    state.show_technical_nodes,
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
        let has_exact_input = !input.value.trim().is_empty();
        let mut row = Column::new().spacing(5).push(
            text(if state.presentation_mode == PresentationMode::Tag {
                "Sentence"
            } else {
                &input.name
            })
            .size(12)
            .color(theme::MUTED),
        );
        if input.input_capable {
            let mut field = text_input(
                if state.presentation_mode == PresentationMode::Tag {
                    "Enter a sentence"
                } else {
                    "Optional interpretation input"
                },
                &input.value,
            )
            .id(input.id.clone())
            .on_input(move |value| Message::InputChanged(index, value))
            .on_paste(move |value| Message::InputChanged(index, value))
            .padding(9)
            .size(13);
            if state.busy.is_none() && state.active_parse.is_none() {
                field = field.on_submit(Message::Parse);
            }
            row = row.push(field);
        } else {
            row = row.push(
                text("Output-only interpretation")
                    .size(11)
                    .color(theme::MUTED),
            );
        }
        if input.non_null_filter_capable {
            let checked = has_exact_input || input.require_valid;
            let can_toggle =
                state.busy.is_none() && state.active_parse.is_none() && !has_exact_input;
            let mut validity = checkbox(checked).label("Require valid value").size(15);
            if can_toggle {
                validity =
                    validity.on_toggle(move |value| Message::RequireValidChanged(index, value));
            }
            row = row.push(validity);
        }
        fields = fields.push(row);
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
        let early_stop_supported = state.constraint_count() <= 1;
        let mut stop = checkbox(state.stop_at_first_goal)
            .label("Stop after first goal")
            .size(15);
        if early_stop_supported {
            stop = stop.on_toggle(Message::StopAtFirstGoal);
        }
        options = options
            .push(text("Heuristic").size(12).color(theme::MUTED))
            .push(
                pick_list(
                    HeuristicChoice::ALL,
                    Some(state.heuristic),
                    Message::HeuristicChanged,
                )
                .width(Length::Fill),
            )
            .push(stop);
        if !early_stop_supported {
            options = options.push(
                text("Early stopping is unavailable with multiple interpretation constraints.")
                    .size(11)
                    .color(theme::MUTED),
            );
        }
    }
    let parse_button = button(text(if let Some(job) = &state.active_parse {
        format!("Cancel ({:.1}s)", job.started.elapsed().as_secs_f32())
    } else {
        "Run parser".into()
    }))
    .padding([9, 16])
    .style(button::primary);
    let parse_button = if state.active_parse.is_some() {
        parse_button.on_press(Message::CancelParse)
    } else if state.busy.is_none() {
        parse_button.on_press(Message::Parse)
    } else {
        parse_button
    };
    let mut content = column![
        page_heading(
            "Parse new input",
            if state.presentation_mode == PresentationMode::Tag {
                "Enter the sentence to parse."
            } else {
                "Provide one or more interpretation values, then choose a chart construction strategy."
            },
        ),
        container(fields)
            .padding(14)
            .width(Length::Fill)
            .style(theme::raised),
    ]
    .spacing(theme::SECTION_SPACING)
    .max_width(760);
    if state.presentation_mode != PresentationMode::Tag {
        content = content.push(
            container(options)
                .padding(14)
                .width(Length::Fill)
                .style(theme::raised),
        );
    }
    content = content.push(row![space::horizontal(), parse_button]);
    page(content)
}

fn language_page<'a>(
    language: &'a LanguageSession,
    title: String,
    primary_label: &'a str,
    mode: PresentationMode,
    show_technical_nodes: bool,
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
        (LanguageStatus::Failed(error), _) => (
            "Unavailable".into(),
            None,
            None,
            message_panel("Language preparation failed", error),
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
            let tag_presentation = (mode == PresentationMode::Tag)
                .then_some(derivation.tag.as_ref())
                .flatten();
            let tag_derivation: Option<&ViewContent> = tag_presentation.map(|tag| {
                if show_technical_nodes {
                    &tag.derivation_with_technical
                } else {
                    &tag.derivation
                }
            });
            let visible_views: Vec<&ViewContent> =
                if let (Some(tag), Some(tag_derivation)) = (tag_presentation, tag_derivation) {
                    vec![&tag.derived_tree, tag_derivation]
                } else {
                    derivation.views.iter().collect()
                };
            let output_index = language
                .output_index
                .min(visible_views.len().saturating_sub(1));
            let output = visible_views[output_index];
            let value_is_structured = matches!(
                output.value,
                ValuePresentation::Tree(_) | ValuePresentation::FeatureStructure(_)
            );
            let value: Element<'a, Message> = match &output.value {
                ValuePresentation::Empty => space::horizontal().into(),
                ValuePresentation::Error(error) => {
                    message_panel("Interpretation did not evaluate", error)
                }
                ValuePresentation::Text(value) => {
                    container(text(value).size(15)).width(Length::Fill).into()
                }
                ValuePresentation::Tree(layout) => tree_view(layout.clone(), language.zoom),
                ValuePresentation::FeatureStructure(layout) => {
                    feature_structure_view(layout.clone(), language.zoom)
                }
            };
            let value = if let Some(evaluated) = &output.evaluated {
                mouse_area(value)
                    .on_right_press(Message::OpenCopyMenu(
                        evaluated.clone(),
                        output.codecs.clone(),
                    ))
                    .into()
            } else {
                value
            };
            let value: Element<'a, Message> = if let Some(warning) = &output.warning {
                column![
                    container(text(warning).size(11).color(theme::MUTED))
                        .padding([7, 10])
                        .width(Length::Fill),
                    value,
                ]
                .spacing(4)
                .height(Length::Fill)
                .into()
            } else {
                value
            };
            let body: Element<'a, Message> = if let Some(term) = &output.term {
                // Text values only need their content height. Errors need a
                // definite region: a Fill-height message inside a Shrink
                // section otherwise collapses to an empty VALUE panel.
                let (value_height, term_height) = match &output.value {
                    ValuePresentation::Error(_) => (Length::Fixed(132.0), Length::Fill),
                    _ if value_is_structured => (Length::FillPortion(3), Length::FillPortion(2)),
                    _ => (Length::Shrink, Length::Fill),
                };
                container(
                    column![
                        panel_section("Value", value, value_height),
                        rule::horizontal(1).style(theme::separator),
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
            let nav = row![
                tooltip(
                    previous,
                    text("Previous derivation").size(12),
                    tooltip::Position::Bottom,
                ),
                tooltip(
                    next,
                    text("Next derivation").size(12),
                    tooltip::Position::Bottom,
                ),
            ]
            .align_y(Alignment::Center)
            .spacing(2);

            // Interpretation-view tabs sit in the bar, right above the tree.
            let mut tabs = Row::new().spacing(4).align_y(Alignment::Center);
            for (index, item) in visible_views.iter().enumerate() {
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
            if tag_presentation.is_some() && output_index == 1 {
                tabs = tabs.push(
                    checkbox(show_technical_nodes)
                        .label("Show technical nodes")
                        .size(13)
                        .on_toggle(Message::ShowTechnicalNodes),
                );
            }
            let zoom = row![
                tooltip(
                    button(text("−").size(15))
                        .style(theme::quiet_button)
                        .on_press(Message::ZoomOut),
                    text("Zoom out").size(12),
                    tooltip::Position::Bottom,
                ),
                tooltip(
                    button(text(format!("{}%", (language.zoom * 100.0).round() as i32)).size(12))
                        .style(theme::quiet_button)
                        .on_press(Message::ZoomReset),
                    text("Reset zoom").size(12),
                    tooltip::Position::Bottom,
                ),
                tooltip(
                    button(text("+").size(15))
                        .style(theme::quiet_button)
                        .on_press(Message::ZoomIn),
                    text("Zoom in").size(12),
                    tooltip::Position::Bottom,
                ),
            ]
            .spacing(2)
            .align_y(Alignment::Center);
            let extra = row![tabs, space::horizontal(), zoom]
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

    let mut heading = row![page_heading(title, subtitle), space::horizontal()]
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
    } else if let Some(job) = &state.active_parse {
        (
            "●",
            if job.started.elapsed() < Duration::from_secs(1) {
                "Computing parse chart…"
            } else {
                "Computing parse chart… Cancel is available in the parse form."
            },
            theme::ACCENT,
        )
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
    let header = container(header)
        .center_y(34)
        .width(Length::Fill)
        .style(|_| iced::widget::container::Style {
            background: Some(theme::SURFACE.into()),
            ..Default::default()
        });

    let mut body = Column::new().spacing(0);
    for (index, item) in rows.iter().enumerate() {
        let mut cells = row![
            container(rule_cell(item, mute_spans)).width(Length::FillPortion(rule_portion)),
            table_cell_owned(format_weight(item.weight), WEIGHT_PORTION),
        ]
        .spacing(12)
        .padding([0, 10])
        .align_y(Alignment::Center);
        for column in 0..interpretations.len() {
            let term = item
                .interpretations
                .get(column)
                .map(String::as_str)
                .unwrap_or("");
            cells = cells.push(table_cell(term, INTERP_PORTION));
        }
        body = body.push(
            container(cells)
                .center_y(theme::TABLE_ROW_HEIGHT)
                .width(Length::Fill)
                .style(move |_| iced::widget::container::Style {
                    background: Some(
                        if index % 2 == 0 {
                            theme::BG
                        } else {
                            theme::SURFACE
                        }
                        .into(),
                    ),
                    text_color: Some(theme::TEXT),
                    ..Default::default()
                }),
        );
    }
    let table =
        column![header, rule::horizontal(1).style(theme::separator), body,].width(Length::Fill);
    container(scrollable(table).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        // Inset content past the 1px border and clip to the rounded corners so the
        // zebra rows don't paint over the panel edge.
        .padding(1)
        .clip(true)
        .style(theme::panel)
        .into()
}

/// "parent → rhs" for a rule. Chart-state components are colored by the
/// automaton that contributed them: grammar state first, then one component per
/// decomposition/filter automaton.
fn rule_cell<'a>(rule: &RuleRow, style_state_parts: bool) -> Element<'a, Message> {
    if !style_state_parts {
        return text(format!("{}  →  {}", rule.parent, rule.rhs))
            .size(14)
            .into();
    }

    let mut spans: Vec<iced::widget::text::Span<'_, ()>> = Vec::new();
    push_state_spans(&mut spans, &rule.parent, &rule.parent_parts);
    push_rule_span(&mut spans, "  →  ", 0);
    push_rule_span(&mut spans, rule.symbol.clone(), 0);
    if !rule.children.is_empty() {
        push_rule_span(&mut spans, "(", 0);
        for (index, child) in rule.children.iter().enumerate() {
            if index > 0 {
                push_rule_span(&mut spans, ", ", 0);
            }
            push_state_spans(
                &mut spans,
                child,
                rule.child_parts
                    .get(index)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            );
        }
        push_rule_span(&mut spans, ")", 0);
    }
    rich_text(spans).size(14).into()
}

fn push_state_spans<'a>(
    spans: &mut Vec<iced::widget::text::Span<'a, ()>>,
    display: &str,
    parts: &[String],
) {
    for (text, color_index) in state_text_segments(display, parts) {
        push_rule_span(spans, text, color_index);
    }
}

fn push_rule_span<'a>(
    spans: &mut Vec<iced::widget::text::Span<'a, ()>>,
    text: impl Into<String>,
    color_index: usize,
) {
    const PART_COLORS: [iced::Color; 5] = [
        theme::TEXT,
        theme::MUTED,
        theme::ACCENT,
        theme::STATE_PART_PURPLE,
        theme::STATE_PART_TEAL,
    ];
    spans.push(span(text.into()).color(PART_COLORS[color_index % PART_COLORS.len()]));
}

fn state_text_segments(display: &str, parts: &[String]) -> Vec<(String, usize)> {
    if parts.is_empty() {
        return vec![(display.to_owned(), 0)];
    }

    let mut segments = Vec::new();
    let mut cursor = 0;
    for (index, part) in parts.iter().enumerate() {
        let Some(relative_start) = display[cursor..].find(part) else {
            return vec![(display.to_owned(), 0)];
        };
        let start = cursor + relative_start;
        if start > cursor {
            segments.push((display[cursor..start].to_owned(), index));
        }
        let end = start + part.len();
        segments.push((display[start..end].to_owned(), index));
        cursor = end;
    }
    if cursor < display.len() {
        segments.push((display[cursor..].to_owned(), 0));
    }
    segments
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
    fn constraint_count(&self) -> usize {
        self.inputs
            .iter()
            .filter(|input| !input.value.trim().is_empty() || input.require_valid)
            .count()
    }

    fn disable_unsupported_early_stop(&mut self) {
        if self.constraint_count() > 1 {
            self.stop_at_first_goal = false;
        }
    }

    /// Apply a loaded grammar (or its error) to this window.
    fn apply_grammar(&mut self, result: Result<GrammarDocument, String>) {
        self.busy = None;
        match result {
            Ok(document) => {
                let (sender, receiver) = mpsc::channel();
                let worker =
                    workers::start_grammar_language_worker(document.grammar.clone(), sender);
                self.presentation_mode = document.detected_mode;
                self.show_technical_nodes = false;
                self.inputs = input_fields(&document, self.presentation_mode);
                self.strategy = StrategyChoice::TopDown;
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
        let mut changed = false;
        if let Some(language) = &mut self.grammar_language {
            changed |= poll_language(language);
        }
        for parse in &mut self.parses {
            changed |= poll_language(&mut parse.language);
        }
        if changed {
            self.copy_menu = None;
        }
    }

    /// Whether any language iterator is still feeding this window events.
    fn has_pending_language(&self) -> bool {
        self.grammar_language
            .as_ref()
            .is_some_and(LanguageSession::polling)
            || self.parses.iter().any(|parse| parse.language.polling())
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

fn input_fields(grammar: &GrammarDocument, mode: PresentationMode) -> Vec<InputField> {
    grammar
        .interpretations
        .iter()
        .filter(|info| mode != PresentationMode::Tag || info.name == "string")
        .map(|info| InputField {
            name: info.name.clone(),
            value: String::new(),
            id: iced::widget::Id::unique(),
            input_capable: info.input_capable,
            non_null_filter_capable: info.non_null_filter_capable,
            require_valid: false,
        })
        .collect()
}

fn parse_label(inputs: &[(String, String)], required_valid: &[String]) -> String {
    let mut parts = inputs
        .iter()
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(" · ");
    for name in required_valid {
        if !parts.is_empty() {
            parts.push_str(" · ");
        }
        parts.push_str(&format!("{name} defined"));
    }
    parts
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
    // `modifiers.command()` is ⌘ on macOS and Ctrl elsewhere.
    match key.as_ref() {
        Key::Character("o") if modifiers.command() => Some(Message::ShortcutOpenGrammar),
        Key::Character("p") if modifiers.command() => Some(Message::NewParse),
        Key::Named(Named::Tab) if modifiers.shift() => Some(Message::FocusPrevious),
        Key::Named(Named::Tab) => Some(Message::FocusNext),
        Key::Named(Named::ArrowLeft) if modifiers.is_empty() => Some(Message::ShortcutPrevious),
        Key::Named(Named::ArrowRight) if modifiers.is_empty() => Some(Message::ShortcutNext),
        Key::Character("?") => Some(Message::ShowShortcuts),
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
        assert_eq!(
            parse_label(&inputs, &["ft".into()]),
            "john watches · hans betrachtet · ft defined"
        );
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
        assert!(matches!(
            keyboard_shortcut(Key::Named(Named::Tab), Modifiers::empty()),
            Some(Message::FocusNext)
        ));
        assert!(matches!(
            keyboard_shortcut(Key::Named(Named::Tab), Modifiers::SHIFT),
            Some(Message::FocusPrevious)
        ));
        assert!(
            keyboard_shortcut(Key::Character("a".into()), Modifiers::COMMAND).is_none(),
            "Cmd/Ctrl-A must reach the focused text input for select-all"
        );
        assert!(matches!(
            keyboard_shortcut(Key::Character("?".into()), Modifiers::SHIFT),
            Some(Message::ShowShortcuts)
        ));
    }

    #[test]
    fn copy_menu_position_is_clamped_to_each_window_edge() {
        let viewport = iced::Size::new(800.0, 600.0);
        assert_eq!(
            clamp_menu_position(Point::new(-20.0, -30.0), viewport, 2),
            Point::new(8.0, 8.0)
        );
        let bottom_right = clamp_menu_position(Point::new(799.0, 599.0), viewport, 2);
        assert!(bottom_right.x + COPY_MENU_WIDTH <= viewport.width - 8.0);
        assert!(bottom_right.y + 106.0 <= viewport.height - 8.0);
    }

    #[test]
    fn language_failure_is_terminal_and_stops_polling() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(LanguageEvent::Failed("test failure".into()))
            .unwrap();
        let mut language = LanguageSession {
            status: LanguageStatus::Preparing,
            receiver: Some(receiver),
            worker: None,
            derivations: Vec::new(),
            derivation_index: 0,
            output_index: 0,
            zoom: 1.0,
            request_pending: false,
        };
        assert!(poll_language(&mut language));
        assert!(matches!(language.status, LanguageStatus::Failed(_)));
        assert!(!language.polling());
        assert!(language.receiver.is_none());
    }

    #[test]
    fn disconnected_language_worker_becomes_a_terminal_failure() {
        let (sender, receiver) = mpsc::channel::<LanguageEvent>();
        drop(sender);
        let mut language = LanguageSession {
            status: LanguageStatus::Preparing,
            receiver: Some(receiver),
            worker: None,
            derivations: Vec::new(),
            derivation_index: 0,
            output_index: 0,
            zoom: 1.0,
            request_pending: false,
        };
        assert!(poll_language(&mut language));
        assert!(matches!(language.status, LanguageStatus::Failed(_)));
        assert!(!language.polling());
    }

    #[test]
    fn finite_language_completion_releases_worker_resources() {
        let (sender, receiver) = mpsc::channel();
        sender.send(LanguageEvent::EndOfLanguage(3)).unwrap();
        let mut language = LanguageSession {
            status: LanguageStatus::Ready(LanguageCardinality::TooLarge),
            receiver: Some(receiver),
            worker: None,
            derivations: Vec::new(),
            derivation_index: 2,
            output_index: 0,
            zoom: 1.0,
            request_pending: true,
        };
        assert!(poll_language(&mut language));
        assert!(matches!(
            language.status,
            LanguageStatus::Ready(LanguageCardinality::Finite(3))
        ));
        assert!(!language.polling());
        assert!(language.receiver.is_none());
    }

    #[test]
    fn canceled_and_stale_parse_results_are_ignored() {
        let control = rusty_alto::ParseControl::new();
        let mut state = Workbench {
            active_parse: Some(ParseJob {
                id: 7,
                started: Instant::now(),
                control: control.clone(),
            }),
            pending_label: Some("input".into()),
            ..Workbench::default()
        };
        let _ = update(&mut state, Message::Parsed(6, Err("stale".into())));
        assert_eq!(state.active_parse.as_ref().map(|job| job.id), Some(7));
        assert!(state.error.is_none());
        let _ = update(&mut state, Message::CancelParse);
        assert!(control.is_cancelled());
        assert!(state.active_parse.is_none());
        assert!(state.pending_label.is_none());
        let _ = update(&mut state, Message::Parsed(7, Err("late".into())));
        assert!(state.error.is_none());
    }

    #[test]
    fn failed_load_removes_only_the_new_target_window() {
        let asking = window::Id::unique();
        let target = window::Id::unique();
        let mut app = App::default();
        app.windows.insert(asking, Workbench::default());
        app.windows.insert(target, Workbench::default());
        let _ = app_update(
            &mut app,
            AppMsg::GrammarLoaded {
                asking,
                target,
                opened_new: true,
                result: Err("bad grammar".into()),
            },
        );
        assert!(app.windows.contains_key(&asking));
        assert!(!app.windows.contains_key(&target));
        assert!(
            app.windows
                .get(&asking)
                .and_then(|window| window.error.as_deref())
                .is_some_and(|error| error.contains("bad grammar"))
        );
    }

    #[test]
    fn multiple_constraints_disable_astar_early_stop() {
        let mut state = Workbench {
            strategy: StrategyChoice::Astar,
            stop_at_first_goal: true,
            inputs: vec![
                InputField {
                    name: "string".into(),
                    value: "words".into(),
                    id: iced::widget::Id::unique(),
                    input_capable: true,
                    non_null_filter_capable: false,
                    require_valid: false,
                },
                InputField {
                    name: "ft".into(),
                    value: String::new(),
                    id: iced::widget::Id::unique(),
                    input_capable: false,
                    non_null_filter_capable: true,
                    require_valid: true,
                },
            ],
            ..Workbench::default()
        };
        assert_eq!(state.constraint_count(), 2);
        state.disable_unsupported_early_stop();
        assert!(!state.stop_at_first_goal);
    }

    #[test]
    fn exact_input_is_one_constraint_and_clears_redundant_validity() {
        let mut state = Workbench {
            inputs: vec![InputField {
                name: "ft".into(),
                value: String::new(),
                id: iced::widget::Id::unique(),
                input_capable: true,
                non_null_filter_capable: true,
                require_valid: true,
            }],
            ..Workbench::default()
        };
        let _ = update(&mut state, Message::InputChanged(0, "[case: nom]".into()));
        assert_eq!(state.constraint_count(), 1);
        assert!(!state.inputs[0].require_valid);
    }

    #[test]
    fn parsing_rows_include_output_only_and_filter_capable_interpretations() {
        let directory =
            std::env::temp_dir().join(format!("rusty_alto_gui_rows_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("rows.irtg");
        std::fs::write(
            &path,
            r#"
interpretation string: de.up.ling.irtg.algebra.StringAlgebra
interpretation tree: de.up.ling.irtg.algebra.TreeWithAritiesAlgebra
interpretation ft: de.up.ling.irtg.algebra.FeatureStructureAlgebra
S! -> value
  [string] word
  [tree] word_0
  [ft] "[case: nom]"
"#,
        )
        .unwrap();
        let document = workers::load_grammar(path).unwrap();
        let rows = input_fields(&document, PresentationMode::RawIrtg);
        assert_eq!(rows.len(), 3);
        let ft = rows.iter().find(|row| row.name == "ft").unwrap();
        let string = rows.iter().find(|row| row.name == "string").unwrap();
        let tree = rows.iter().find(|row| row.name == "tree").unwrap();
        assert!(ft.non_null_filter_capable);
        assert!(!ft.require_valid);
        assert!(string.input_capable);
        assert!(!tree.input_capable);
    }

    #[test]
    fn tag_mode_is_detected_and_can_switch_to_raw_irtg() {
        let directory =
            std::env::temp_dir().join(format!("rusty_alto_gui_tag_mode_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("mode.tag");
        std::fs::write(
            &path,
            r#"
tree sentence:
  S @NA { V+ }
word 'sleeps': sentence
"#,
        )
        .unwrap();
        let document = workers::load_grammar(path).unwrap();
        assert_eq!(document.detected_mode, PresentationMode::Tag);

        let mut state = Workbench::default();
        state.apply_grammar(Ok(document));
        assert_eq!(state.presentation_mode, PresentationMode::Tag);
        assert_eq!(state.strategy, StrategyChoice::TopDown);
        assert_eq!(state.inputs.len(), 1);
        assert_eq!(state.inputs[0].name, "string");

        let _ = update(
            &mut state,
            Message::SetPresentationMode(PresentationMode::RawIrtg),
        );
        assert_eq!(state.presentation_mode, PresentationMode::RawIrtg);
        assert!(state.inputs.iter().any(|input| input.name == "tree"));
    }

    #[test]
    fn state_text_segments_preserve_arbitrary_display_parts() {
        assert_eq!(
            state_text_segments(
                "NP × [0-2, 3-5] × q7!",
                &["NP".into(), "[0-2, 3-5]".into(), "q7".into()]
            ),
            vec![
                ("NP".into(), 0),
                (" × ".into(), 1),
                ("[0-2, 3-5]".into(), 1),
                (" × ".into(), 2),
                ("q7".into(), 2),
                ("!".into(), 0),
            ]
        );
        assert_eq!(
            state_text_segments("NP[0-2]", &["NP".into(), "[0-2]".into()]),
            vec![("NP".into(), 0), ("[0-2]".into(), 1)]
        );
    }
}
