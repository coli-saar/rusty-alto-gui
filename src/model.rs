use rusty_alto::{
    AutomatonSummary, CodecMetadata, EvaluatedAlgebraValue, Explicit, InterpretationInfo, Irtg,
    ResolvedRule,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentTab {
    Primary,
    Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationMode {
    RawIrtg,
    Tag,
}

impl std::fmt::Display for PresentationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RawIrtg => "IRTG",
            Self::Tag => "TAG",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleColumn {
    Rule,
    Weight,
}

#[derive(Debug, Clone)]
pub struct RuleRow {
    pub parent: String,
    pub parent_parts: Vec<String>,
    pub symbol: String,
    pub children: Vec<String>,
    pub child_parts: Vec<Vec<String>>,
    pub rhs: String,
    pub weight: f64,
    /// Homomorphism term per interpretation, in grammar order.
    pub interpretations: Vec<String>,
}

impl RuleRow {
    pub fn from_resolved(rule: &ResolvedRule) -> Self {
        Self::from_resolved_with_parts(
            rule,
            vec![rule.parent.clone()],
            rule.children
                .iter()
                .cloned()
                .map(|child| vec![child])
                .collect(),
        )
    }

    pub fn from_resolved_with_parts(
        rule: &ResolvedRule,
        parent_parts: Vec<String>,
        child_parts: Vec<Vec<String>>,
    ) -> Self {
        Self {
            parent: if rule.parent_is_final {
                format!("{}!", rule.parent)
            } else {
                rule.parent.clone()
            },
            parent_parts,
            symbol: rule.symbol.clone(),
            children: rule.children.clone(),
            child_parts,
            rhs: if rule.children.is_empty() {
                rule.symbol.clone()
            } else {
                format!("{}({})", rule.symbol, rule.children.join(", "))
            },
            weight: rule.weight,
            interpretations: rule
                .interpretation_terms
                .iter()
                .map(|(_, term)| term.clone())
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InputField {
    pub name: String,
    pub value: String,
    pub id: iced::widget::Id,
    pub input_capable: bool,
    pub non_null_filter_capable: bool,
    pub require_valid: bool,
}

#[derive(Clone)]
pub struct GrammarDocument {
    pub path: PathBuf,
    pub detected_mode: PresentationMode,
    pub grammar: Arc<Irtg>,
    pub summary: AutomatonSummary,
    pub interpretations: Vec<InterpretationInfo>,
    /// Interpretation names in grammar order (column headers for the rule table).
    pub interpretation_names: Vec<String>,
    pub rules: Vec<RuleRow>,
}

impl std::fmt::Debug for GrammarDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrammarDocument")
            .field("path", &self.path)
            .field("summary", &self.summary)
            .field("rules", &self.rules.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct ChartDocument {
    pub automaton: Arc<Explicit>,
    pub summary: AutomatonSummary,
    pub elapsed: Duration,
    pub rules: Vec<RuleRow>,
}

impl std::fmt::Debug for ChartDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartDocument")
            .field("summary", &self.summary)
            .field("elapsed", &self.elapsed)
            .field("rules", &self.rules.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrategyChoice {
    TopDown,
    Indexed,
    Astar,
}

impl StrategyChoice {
    pub const ALL: [Self; 3] = [Self::TopDown, Self::Indexed, Self::Astar];
}

impl std::fmt::Display for StrategyChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TopDown => "Top-down condensed",
            Self::Indexed => "Indexed condensed",
            Self::Astar => "A*",
        })
    }
}

/// A* heuristic; only meaningful when [`StrategyChoice::Astar`] is selected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeuristicChoice {
    Zero,
    Sx,
    Sxf,
}

impl HeuristicChoice {
    pub const ALL: [Self; 3] = [Self::Zero, Self::Sx, Self::Sxf];
}

impl std::fmt::Display for HeuristicChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Zero => "Zero",
            Self::Sx => "SX",
            Self::Sxf => "SX + F",
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct TreeLayout {
    pub nodes: Vec<TreeNode>,
    pub edges: Vec<TreeEdge>,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub label: String,
    pub top: Option<String>,
    pub bottom: Option<String>,
    pub muted: bool,
    pub conflict: ConflictHighlight,
    pub top_source: ConflictHighlight,
    pub bottom_source: ConflictHighlight,
    pub top_conflict: bool,
    pub bottom_conflict: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConflictHighlight {
    #[default]
    None,
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone)]
pub struct TreeEdge {
    pub parent_x: f32,
    pub parent_y: f32,
    pub child_x: f32,
    pub child_y: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ViewContent {
    pub name: String,
    pub warning: Option<String>,
    pub term: Option<Arc<TreeLayout>>,
    pub value: ValuePresentation,
    pub evaluated: Option<Arc<EvaluatedAlgebraValue>>,
    pub codecs: Vec<CodecMetadata>,
}

#[derive(Debug, Clone, Default)]
pub enum ValuePresentation {
    #[default]
    Empty,
    Error(String),
    Text(String),
    Tree(Arc<TreeLayout>),
    FeatureStructure(Arc<FeatureStructureLayout>),
}

#[derive(Debug, Clone, Default)]
pub struct FeatureStructureLayout {
    pub texts: Vec<FeatureStructureText>,
    pub lines: Vec<FeatureStructureLine>,
    pub boxes: Vec<FeatureStructureBox>,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct FeatureStructureText {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub centered: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FeatureStructureLine {
    pub from_x: f32,
    pub from_y: f32,
    pub to_x: f32,
    pub to_y: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct FeatureStructureBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Default)]
pub struct DerivationPresentation {
    pub index: usize,
    pub weight: f64,
    pub views: Vec<ViewContent>,
    pub tag: Option<TagPresentation>,
}

#[derive(Debug, Clone)]
pub struct TagPresentation {
    pub derived_tree: ViewContent,
    pub derivation: ViewContent,
    pub derivation_with_technical: ViewContent,
    pub failure: Option<FailurePresentation>,
}

#[derive(Debug, Clone)]
pub struct FailurePresentation {
    pub title: String,
    pub path: String,
    pub left: String,
    pub right: String,
    pub left_origin: String,
    pub right_origin: String,
    pub operation: String,
}

#[derive(Debug, Clone)]
pub struct ParseOutcome {
    pub chart: ChartDocument,
    pub rejected_by_features: bool,
}
