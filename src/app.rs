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
    Alignment, Element, Length, Subscription, Task,
    keyboard::{Key, Modifiers, key::Named},
    widget::{
        Column, Row, button, checkbox, column, container, horizontal_space, pick_list, row,
        scrollable, text, text_input, vertical_rule,
    },
};
use rusty_alto::LanguageCardinality;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

pub fn run() -> iced::Result {
    iced::application("Rusty Alto Workbench", update, view)
        .theme(theme::app_theme)
        .subscription(subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 900.0),
            min_size: Some(iced::Size::new(1050.0, 680.0)),
            ..Default::default()
        })
        .run_with(|| (Workbench::default(), Task::none()))
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenGrammar,
    GrammarPicked(Option<PathBuf>),
    GrammarLoaded(Result<GrammarDocument, String>),
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
    PollLanguages,
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
        Message::OpenGrammar => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("IRTG grammar", &["irtg"])
                        .pick_file()
                        .await
                        .map(|handle| handle.path().to_owned())
                },
                Message::GrammarPicked,
            );
        }
        Message::GrammarPicked(Some(path)) => {
            state.busy = Some(format!("Loading {}…", display_name(&path)));
            state.error = None;
            return Task::perform(
                async move { workers::load_grammar(path) },
                Message::GrammarLoaded,
            );
        }
        Message::GrammarPicked(None) => {}
        Message::GrammarLoaded(result) => {
            state.busy = None;
            match result {
                Ok(document) => {
                    let (sender, receiver) = mpsc::channel();
                    let worker =
                        workers::start_grammar_language_worker(document.grammar.clone(), sender);
                    state.inputs = input_fields(&document);
                    state.grammar = Some(document);
                    state.grammar_language = Some(LanguageSession::preparing(worker, receiver));
                    state.parses.clear();
                    state.next_parse_id = 1;
                    state.selection = Selection::Grammar;
                    state.active_tab = DocumentTab::Primary;
                    state.error = None;
                }
                Err(error) => state.fail(format!("Could not load grammar: {error}")),
            }
        }
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
        Message::PollLanguages => {
            if let Some(language) = &mut state.grammar_language {
                poll_language(language);
            }
            for parse in &mut state.parses {
                poll_language(&mut parse.language);
            }
        }
        Message::PreviousDerivation => {
            if let Some(language) = state.active_language_mut() {
                language.derivation_index = language.derivation_index.saturating_sub(1);
                language.output_index = 0;
            }
        }
        Message::NextDerivation => {
            if let Some(language) = state.active_language_mut() {
                if language.derivation_index + 1 < language.derivations.len() {
                    language.derivation_index += 1;
                    language.output_index = 0;
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
        Message::ShortcutOpenGrammar => return update(state, Message::OpenGrammar),
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
                    language.output_index = 0;
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

fn subscription(state: &Workbench) -> Subscription<Message> {
    let has_language = state
        .grammar_language
        .as_ref()
        .is_some_and(|language| language.receiver.is_some())
        || state
            .parses
            .iter()
            .any(|parse| parse.language.receiver.is_some());
    let polling = if has_language {
        iced::time::every(Duration::from_millis(80)).map(|_| Message::PollLanguages)
    } else {
        Subscription::none()
    };
    Subscription::batch([polling, iced::keyboard::on_key_press(keyboard_shortcut)])
}

fn view(state: &Workbench) -> Element<'_, Message> {
    let body = row![
        sidebar(state),
        vertical_rule(1),
        column![context_tabs(state), workspace(state)].width(Length::Fill),
    ]
    .height(Length::Fill);

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
            text(format!("#{}  {}", parse.id, parse.label))
                .size(12)
                .wrapping(text::Wrapping::None),
            text(parse.language.sidebar_status())
                .size(10)
                .color(match parse.language.status {
                    LanguageStatus::Error(_) => theme::DANGER,
                    _ => theme::MUTED,
                }),
        ]
        .spacing(3);
        let select = document_button(
            content,
            state.selection == Selection::Parse(id),
            Message::SelectParse(id),
        );
        let remove = button(text("×").size(15))
            .padding([6, 9])
            .style(theme::quiet_button)
            .on_press(Message::RemoveParse(id));
        documents = documents.push(
            row![select, remove]
                .spacing(3)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        );
    }

    let parse_button = button(text("+ Parse").size(13))
        .width(170)
        .padding([10, 18])
        .style(theme::parse_button);
    let parse_button = if state.grammar.is_some() && state.busy.is_none() {
        parse_button.on_press(Message::NewParse)
    } else {
        parse_button
    };

    container(
        column![
            button(text("Open grammar…").size(13))
                .width(Length::Fill)
                .padding([9, 12])
                .style(button::secondary)
                .on_press(Message::OpenGrammar),
            scrollable(documents.padding([12, 0])).height(Length::Fill),
            container(parse_button)
                .padding([10, 18])
                .width(Length::Fill)
                .center_x(Length::Fill),
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

fn context_tabs(state: &Workbench) -> Element<'_, Message> {
    if state.selection == Selection::NewParse || state.grammar.is_none() {
        return container(horizontal_space())
            .height(theme::TAB_HEIGHT + 16.0)
            .width(Length::Fill)
            .style(theme::tab_strip)
            .into();
    }
    let primary_label = match state.selection {
        Selection::Grammar => "Grammar",
        Selection::Parse(_) => "Chart",
        Selection::NewParse => unreachable!(),
    };
    let primary = button(text(primary_label).size(12))
        .height(theme::TAB_HEIGHT)
        .padding([0, 15])
        .style(if state.active_tab == DocumentTab::Primary {
            theme::selected_button
        } else {
            theme::quiet_button
        })
        .on_press(Message::SelectTab(DocumentTab::Primary));

    let ready = state.active_language().is_some_and(LanguageSession::ready);
    let language_text =
        text("Language")
            .size(12)
            .color(if ready { theme::TEXT } else { theme::MUTED });
    let language = button(language_text)
        .height(theme::TAB_HEIGHT)
        .padding([0, 15])
        .style(if ready && state.active_tab == DocumentTab::Language {
            theme::selected_button
        } else {
            theme::quiet_button
        });
    let language = if ready {
        language.on_press(Message::SelectTab(DocumentTab::Language))
    } else {
        language
    };

    container(row![primary, language].spacing(4).align_y(Alignment::End))
        .padding(iced::Padding {
            top: 10.0,
            right: 18.0,
            bottom: 0.0,
            left: 18.0,
        })
        .height(theme::TAB_HEIGHT + 16.0)
        .width(Length::Fill)
        .style(theme::tab_strip)
        .into()
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
                            .map(|grammar| display_name(&grammar.path))
                            .unwrap_or_else(|| "Grammar".into()),
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
                DocumentTab::Language => {
                    language_page(&parse.language, format!("#{}  {}", parse.id, parse.label))
                }
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
    page(
        column![
            page_heading(
                "Grammar rules",
                format!(
                    "{} rules · {} states · maximum rank {}",
                    grammar.summary.rule_count,
                    grammar.summary.state_count,
                    grammar.summary.maximum_rank
                ),
            ),
            rule_table(&grammar.rules, Message::SortGrammar),
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
            rule_table(&parse.chart.rules, move |column| Message::SortChart(
                id, column
            )),
        ]
        .spacing(theme::SECTION_SPACING),
    )
}

fn parse_page(state: &Workbench) -> Element<'_, Message> {
    let mut fields = Column::new().spacing(10);
    for (index, input) in state.inputs.iter().enumerate() {
        fields = fields.push(
            column![
                text(&input.name).size(12).color(theme::MUTED),
                text_input("Optional interpretation input", &input.value)
                    .on_input(move |value| Message::InputChanged(index, value))
                    .padding(9)
                    .size(13),
            ]
            .spacing(5),
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
        options = options
            .push(
                checkbox("Stop after first goal", state.stop_at_first_goal)
                    .on_toggle(Message::StopAtFirstGoal)
                    .size(15),
            )
            .push(
                text_input("Optional beam, e.g. 0.001", &state.beam)
                    .on_input(Message::BeamChanged)
                    .padding(9),
            );
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

fn language_page(language: &LanguageSession, title: String) -> Element<'_, Message> {
    match &language.status {
        LanguageStatus::Preparing => {
            return empty_state(
                "Preparing language…",
                "Initializing the sorted language iterator in the background.",
                None,
            );
        }
        LanguageStatus::Error(error) => {
            return empty_state("Could not prepare language", error, None);
        }
        LanguageStatus::Ready(LanguageCardinality::Finite(0)) => {
            return empty_state(
                "Language is empty",
                "This automaton accepts no derivation trees.",
                None,
            );
        }
        LanguageStatus::Ready(_) => {}
    }
    let Some(derivation) = language.derivations.get(language.derivation_index) else {
        return empty_state(
            "Loading first derivation…",
            "Evaluating interpretations and preparing the derivation tree.",
            None,
        );
    };
    let output_index = language
        .output_index
        .min(derivation.views.len().saturating_sub(1));
    let output = &derivation.views[output_index];
    let mut choices = Row::new().spacing(4);
    for (index, item) in derivation.views.iter().enumerate() {
        choices = choices.push(
            button(text(&item.name).size(12))
                .padding([7, 10])
                .style(if index == output_index {
                    theme::selected_button
                } else {
                    theme::quiet_button
                })
                .on_press(Message::SelectOutput(index)),
        );
    }
    let value: Element<'_, Message> = if let Some(layout) = &output.tree {
        tree_view(layout.clone(), language.zoom)
    } else {
        scrollable(
            container(text(&output.value).size(16))
                .padding(20)
                .width(Length::Fill),
        )
        .into()
    };
    let content = if let Some(term) = &output.term {
        column![
            value,
            container(
                column![
                    text("TERM").size(10).color(theme::MUTED),
                    text(term).size(13),
                ]
                .spacing(4)
            )
            .padding(12)
            .width(Length::Fill)
            .style(theme::flat),
        ]
        .height(Length::Fill)
    } else {
        column![value].height(Length::Fill)
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
    page(
        column![
            column![
                text(title).size(19),
                row![
                    text(format!(
                        "#{} of {} · weight {:.6}",
                        language.derivation_index + 1,
                        language.size_label(),
                        derivation.weight
                    ))
                    .size(12)
                    .color(theme::MUTED),
                    horizontal_space(),
                    previous,
                    next,
                ]
                .align_y(Alignment::Center)
                .spacing(5),
            ]
            .spacing(4),
            row![
                scrollable(choices).direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::default()
                )),
                horizontal_space(),
                button(text("−"))
                    .style(theme::quiet_button)
                    .on_press(Message::ZoomOut),
                button(text(format!("{}%", (language.zoom * 100.0).round() as i32)))
                    .style(theme::quiet_button)
                    .on_press(Message::ZoomReset),
                button(text("+"))
                    .style(theme::quiet_button)
                    .on_press(Message::ZoomIn),
            ]
            .align_y(Alignment::Center),
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::raised),
        ]
        .spacing(theme::SECTION_SPACING),
    )
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
            .padding([0, 10]),
    )
    .height(24)
    .width(Length::Fill)
    .style(theme::flat)
    .into()
}

fn rule_table<'a>(
    rows: &'a [RuleRow],
    sort: impl Fn(RuleColumn) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    let header = row![
        table_header("State", 2, RuleColumn::State, sort),
        table_header("Rule", 4, RuleColumn::Rule, sort),
        table_header("Weight", 1, RuleColumn::Weight, sort),
        table_header("Interpretations", 5, RuleColumn::Interpretations, sort),
    ]
    .spacing(8)
    .padding([0, 8])
    .align_y(Alignment::Center);
    let mut body = Column::new().spacing(0);
    for (index, item) in rows.iter().enumerate() {
        body = body.push(
            container(
                row![
                    table_cell(&item.parent, 2),
                    table_cell(&item.rhs, 4),
                    table_cell_owned(format!("[{}]", item.weight), 1),
                    table_cell(&item.terms, 5),
                ]
                .spacing(8)
                .padding([0, 8])
                .align_y(Alignment::Center),
            )
            .height(theme::TABLE_ROW_HEIGHT)
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
    container(column![
        container(header)
            .height(32)
            .width(Length::Fill)
            .style(theme::raised),
        scrollable(body).height(Length::Fill),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::panel)
    .into()
}

fn table_header<'a>(
    label: &'a str,
    width: u16,
    column: RuleColumn,
    sort: impl Fn(RuleColumn) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    button(text(format!("{label}  ↕")).size(11).color(theme::MUTED))
        .width(Length::FillPortion(width))
        .padding(0)
        .style(theme::quiet_button)
        .on_press(sort(column))
        .into()
}

fn table_cell<'a>(value: &'a str, width: u16) -> Element<'a, Message> {
    container(text(value).size(12))
        .width(Length::FillPortion(width))
        .into()
}

fn table_cell_owned(value: String, width: u16) -> Element<'static, Message> {
    container(text(value).size(12))
        .width(Length::FillPortion(width))
        .into()
}

fn page<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(theme::PAGE_PADDING)
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
        RuleColumn::State => a.parent.cmp(&b.parent),
        RuleColumn::Rule => a.rhs.cmp(&b.rhs),
        RuleColumn::Weight => a.weight.total_cmp(&b.weight),
        RuleColumn::Interpretations => a.terms.cmp(&b.terms),
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
