use crate::{
    model::{
        ChartDocument, DerivationPresentation, DocumentTab, GrammarDocument, HeuristicChoice,
        InputField, PresentationMode, StrategyChoice,
    },
    workers::{LanguageEvent, LanguageWorker},
};
use iced::Point;
use rusty_alto::{CodecMetadata, EvaluatedAlgebraValue, LanguageCardinality, ParseControl};
use std::{
    sync::{Arc, mpsc},
    time::Instant,
};

pub(crate) struct Workbench {
    pub(crate) grammar: Option<GrammarDocument>,
    pub(crate) grammar_language: Option<LanguageSession>,
    pub(crate) parses: Vec<ParseSession>,
    pub(crate) next_parse_id: u64,
    pub(crate) selection: Selection,
    pub(crate) active_tab: DocumentTab,
    pub(crate) inputs: Vec<InputField>,
    pub(crate) strategy: StrategyChoice,
    pub(crate) heuristic: HeuristicChoice,
    pub(crate) stop_at_first_goal: bool,
    pub(crate) presentation_mode: PresentationMode,
    pub(crate) show_technical_nodes: bool,
    pub(crate) pending_label: Option<String>,
    pub(crate) active_parse: Option<ParseJob>,
    pub(crate) next_parse_job_id: u64,
    pub(crate) busy: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) cursor_position: Point,
    pub(crate) viewport_size: iced::Size,
    pub(crate) copy_menu: Option<CopyMenu>,
    pub(crate) show_shortcuts: bool,
}

pub(crate) struct CopyMenu {
    pub(crate) position: Point,
    pub(crate) value: Arc<EvaluatedAlgebraValue>,
    pub(crate) codecs: Vec<CodecMetadata>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParseJob {
    pub(crate) id: u64,
    pub(crate) started: Instant,
    pub(crate) control: ParseControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    Grammar,
    Parse(u64),
    NewParse,
}

pub(crate) struct ParseSession {
    pub(crate) id: u64,
    pub(crate) label: String,
    pub(crate) chart: ChartDocument,
    pub(crate) language: LanguageSession,
    pub(crate) rejected_by_features: bool,
}

pub(crate) struct LanguageSession {
    pub(crate) status: LanguageStatus,
    pub(crate) receiver: Option<mpsc::Receiver<LanguageEvent>>,
    pub(crate) worker: Option<LanguageWorker>,
    pub(crate) derivations: Vec<Arc<DerivationPresentation>>,
    pub(crate) derivation_index: usize,
    pub(crate) output_index: usize,
    pub(crate) zoom: f32,
    pub(crate) request_pending: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum LanguageStatus {
    Preparing,
    Ready(LanguageCardinality),
    Failed(String),
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
            heuristic: HeuristicChoice::Zero,
            stop_at_first_goal: false,
            presentation_mode: PresentationMode::RawIrtg,
            show_technical_nodes: false,
            pending_label: None,
            active_parse: None,
            next_parse_job_id: 1,
            busy: None,
            error: None,
            cursor_position: Point::ORIGIN,
            viewport_size: iced::Size::new(1440.0, 900.0),
            copy_menu: None,
            show_shortcuts: false,
        }
    }
}

impl LanguageSession {
    pub(crate) fn preparing(
        worker: LanguageWorker,
        receiver: mpsc::Receiver<LanguageEvent>,
    ) -> Self {
        Self {
            status: LanguageStatus::Preparing,
            receiver: Some(receiver),
            worker: Some(worker),
            derivations: Vec::new(),
            derivation_index: 0,
            output_index: 0,
            zoom: 1.0,
            request_pending: false,
        }
    }

    pub(crate) fn ready(&self) -> bool {
        matches!(self.status, LanguageStatus::Ready(_))
    }

    pub(crate) fn polling(&self) -> bool {
        matches!(self.status, LanguageStatus::Preparing) || self.request_pending
    }

    pub(crate) fn has_next(&self) -> bool {
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

    pub(crate) fn size_label(&self) -> String {
        match self.status {
            LanguageStatus::Ready(LanguageCardinality::Finite(size)) => size.to_string(),
            LanguageStatus::Ready(LanguageCardinality::Infinite) => "∞".into(),
            LanguageStatus::Ready(LanguageCardinality::TooLarge) => "many".into(),
            _ => "…".into(),
        }
    }

    pub(crate) fn sidebar_status(&self) -> String {
        match &self.status {
            LanguageStatus::Preparing => "Preparing language…".into(),
            LanguageStatus::Failed(_) => "Language failed".into(),
            LanguageStatus::Ready(LanguageCardinality::Finite(0)) => "Empty language".into(),
            LanguageStatus::Ready(LanguageCardinality::Finite(1)) => "1 derivation".into(),
            LanguageStatus::Ready(LanguageCardinality::Finite(size)) => {
                format!("{size} derivations")
            }
            LanguageStatus::Ready(LanguageCardinality::Infinite) => "∞ derivations".into(),
            LanguageStatus::Ready(LanguageCardinality::TooLarge) => "Many derivations".into(),
        }
    }
}
