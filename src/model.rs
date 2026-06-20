use rusty_alto::{
    AutomatonSummary, Explicit, InterpretationInfo, Irtg, ParseStrategy, ResolvedRule,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentTab {
    Primary,
    Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleColumn {
    Rule,
    Weight,
}

#[derive(Debug, Clone)]
pub struct RuleRow {
    pub parent: String,
    pub rhs: String,
    pub weight: f64,
    /// Homomorphism term per interpretation, in grammar order.
    pub interpretations: Vec<String>,
}

impl RuleRow {
    pub fn from_resolved(rule: &ResolvedRule) -> Self {
        Self {
            parent: if rule.parent_is_final {
                format!("{}!", rule.parent)
            } else {
                rule.parent.clone()
            },
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
}

#[derive(Clone)]
pub struct GrammarDocument {
    pub path: PathBuf,
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

    pub fn to_strategy(self, stop_at_first_goal: bool, beam: Option<f64>) -> ParseStrategy {
        match self {
            Self::TopDown => ParseStrategy::TopDownCondensed,
            Self::Indexed => ParseStrategy::IndexedCondensed,
            Self::Astar => ParseStrategy::AstarZero {
                stop_at_first_goal,
                beam,
            },
        }
    }
}

impl std::fmt::Display for StrategyChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TopDown => "Top-down condensed",
            Self::Indexed => "Indexed condensed",
            Self::Astar => "A* (zero heuristic)",
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
    pub x: f32,
    pub y: f32,
    pub width: f32,
}

#[derive(Debug, Clone)]
pub struct TreeEdge {
    pub parent_x: f32,
    pub parent_y: f32,
    pub child_x: f32,
    pub child_y: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ViewContent {
    pub name: String,
    pub value: String,
    pub term: Option<Arc<TreeLayout>>,
    pub tree: Option<Arc<TreeLayout>>,
}

#[derive(Debug, Clone, Default)]
pub struct DerivationPresentation {
    pub index: usize,
    pub weight: f64,
    pub views: Vec<ViewContent>,
}
