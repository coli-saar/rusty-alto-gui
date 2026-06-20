use crate::model::{
    ChartDocument, DerivationPresentation, GrammarDocument, RuleRow, TreeEdge, TreeLayout,
    TreeNode, ViewContent,
};
use packed_term_arena::tree::{Tree, TreeArena};
use rusty_alto::{
    Explicit, Irtg, LanguageCardinality, ParseStrategy, RenderedValue, Symbol, TreeValue,
    parse_irtg,
};
use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Instant,
};

#[derive(Debug, Clone)]
pub enum LanguageEvent {
    Ready(LanguageCardinality),
    Derivation(Arc<DerivationPresentation>),
    EndOfLanguage(usize),
    Error(String),
}

#[derive(Debug)]
pub struct LanguageWorker {
    sender: Sender<usize>,
    cancelled: Arc<AtomicBool>,
}

impl LanguageWorker {
    pub fn request(&self, index: usize) {
        let _ = self.sender.send(index);
    }
}

impl Drop for LanguageWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub fn load_grammar(path: PathBuf) -> Result<GrammarDocument, String> {
    let file = File::open(&path).map_err(|error| error.to_string())?;
    let grammar = Arc::new(parse_irtg(file).map_err(|error| error.to_string())?);
    let summary = grammar.grammar().application_summary();
    let interpretations = grammar.interpretation_info();
    let interpretation_names = interpretations.iter().map(|info| info.name.clone()).collect();
    let rules = grammar
        .resolved_grammar_rules()
        .iter()
        .map(RuleRow::from_resolved)
        .collect();
    Ok(GrammarDocument {
        path,
        grammar,
        summary,
        interpretations,
        interpretation_names,
        rules,
    })
}

pub fn parse(
    grammar: Arc<Irtg>,
    inputs: Vec<(String, String)>,
    strategy: ParseStrategy,
) -> Result<ChartDocument, String> {
    let start = Instant::now();
    let mut parsed = Vec::with_capacity(inputs.len());
    for (name, text) in inputs {
        let interpretation = grammar
            .interpretation_ref(&name)
            .ok_or_else(|| format!("Unknown interpretation {name:?}"))?;
        let value = interpretation
            .parse_object_erased(&text)
            .map_err(|error| error.to_string())?;
        parsed.push(interpretation.input_erased(value));
    }
    let materialization = strategy.materialization_strategy();
    let result = grammar
        .parse_with(parsed, &materialization)
        .map_err(|error| error.to_string())?;
    let state_names = result.state_names;
    let automaton = Arc::new(result.automaton);
    let summary = automaton.application_summary();
    let rules = automaton
        .resolve_rules(
            |state| {
                state_names
                    .get(state.index())
                    .cloned()
                    .unwrap_or_else(|| format!("q{}", state.0))
            },
            |symbol| grammar.grammar_signature().resolve(symbol).to_owned(),
        )
        .iter()
        .map(RuleRow::from_resolved)
        .collect();
    Ok(ChartDocument {
        automaton,
        summary,
        elapsed: start.elapsed(),
        rules,
    })
}

pub fn start_chart_language_worker(
    grammar: Arc<Irtg>,
    automaton: Arc<Explicit>,
    sender: Sender<LanguageEvent>,
) -> LanguageWorker {
    let (request_tx, request_rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    std::thread::spawn(move || {
        prepare_and_run_language(&grammar, &automaton, request_rx, sender, &worker_cancelled)
    });
    LanguageWorker {
        sender: request_tx,
        cancelled,
    }
}

pub fn start_grammar_language_worker(
    grammar: Arc<Irtg>,
    sender: Sender<LanguageEvent>,
) -> LanguageWorker {
    let (request_tx, request_rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    std::thread::spawn(move || {
        prepare_and_run_language(
            &grammar,
            grammar.grammar(),
            request_rx,
            sender,
            &worker_cancelled,
        )
    });
    LanguageWorker {
        sender: request_tx,
        cancelled,
    }
}

fn prepare_and_run_language(
    grammar: &Irtg,
    automaton: &Explicit,
    requests: Receiver<usize>,
    events: Sender<LanguageEvent>,
    cancelled: &AtomicBool,
) {
    let cardinality = automaton.language_cardinality();
    let mut iterator = automaton.sorted_language();
    if cancelled.load(Ordering::Relaxed)
        || events.send(LanguageEvent::Ready(cardinality)).is_err()
        || cardinality == LanguageCardinality::Finite(0)
    {
        return;
    }
    let mut cache = Vec::<Arc<DerivationPresentation>>::new();
    while let Ok(requested) = requests.recv() {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        while cache.len() <= requested {
            let Some(weighted) = iterator.next() else {
                let _ = events.send(LanguageEvent::EndOfLanguage(cache.len()));
                break;
            };
            let (arena, root) = iterator.clone_tree(weighted.tree());
            let derivation = grammar.resolve_derivation(&arena, root);
            let rendered = match grammar.render_derivation(&arena, root) {
                Ok(rendered) => rendered,
                Err(error) => {
                    let _ = events.send(LanguageEvent::Error(error.to_string()));
                    return;
                }
            };
            let terms = grammar
                .interpretations()
                .map(|interpretation| {
                    let mut term_arena = TreeArena::<Symbol>::new();
                    interpretation
                        .homomorphism()
                        .apply(&arena, root, &mut term_arena)
                        .map(|term_root| {
                            let signature = interpretation.algebra_signature();
                            let layout = layout_tree_nodes(term_root, &|node| {
                                signature.resolve(*term_arena.get_label(node)).to_owned()
                            }, &|node| {
                                term_arena.get_children(node).to_vec()
                            });
                            (
                                interpretation.name().to_owned(),
                                Arc::new(layout),
                            )
                        })
                })
                .collect::<Result<HashMap<_, _>, _>>();
            let terms = match terms {
                Ok(terms) => terms,
                Err(error) => {
                    let _ = events.send(LanguageEvent::Error(error.to_string()));
                    return;
                }
            };
            let mut views = vec![view_from_tree("Derivation tree", &derivation)];
            views.extend(rendered.into_iter().map(|item| match item.value {
                RenderedValue::Text(text) => ViewContent {
                    term: terms.get(&item.name).cloned(),
                    name: item.name,
                    value: text,
                    tree: None,
                },
                RenderedValue::Tree(tree) => {
                    let mut view = view_from_tree(item.name.clone(), &tree);
                    view.term = terms.get(&item.name).cloned();
                    view
                }
            }));
            cache.push(Arc::new(DerivationPresentation {
                index: cache.len(),
                weight: weighted.weight(),
                views,
            }));
        }
        if let Some(item) = cache.get(requested) {
            if events
                .send(LanguageEvent::Derivation(item.clone()))
                .is_err()
            {
                return;
            }
        }
    }
}

fn view_from_tree(name: impl Into<String>, tree: &TreeValue) -> ViewContent {
    ViewContent {
        name: name.into(),
        value: tree.to_string(),
        term: None,
        tree: Some(Arc::new(layout_tree(tree))),
    }
}

const TREE_H_GAP: f32 = 28.0;
const TREE_V_GAP: f32 = 74.0;
const TREE_NODE_HEIGHT: f32 = 30.0;
const TREE_MARGIN: f32 = 28.0;

/// Lay out any tree given accessors for a node's label and children. Used for
/// derivation trees, tree-valued interpretations, and interpretation terms.
fn layout_tree_nodes<L, C>(root: Tree, label_of: &L, children_of: &C) -> TreeLayout
where
    L: Fn(Tree) -> String,
    C: Fn(Tree) -> Vec<Tree>,
{
    fn visit<L, C>(
        node: Tree,
        depth: usize,
        left: f32,
        layout: &mut TreeLayout,
        label_of: &L,
        children_of: &C,
    ) -> (usize, f32)
    where
        L: Fn(Tree) -> String,
        C: Fn(Tree) -> Vec<Tree>,
    {
        let label = label_of(node);
        let node_width = (label.chars().count() as f32 * 7.5 + 22.0).clamp(58.0, 220.0);
        let children = children_of(node);
        let mut child_centers = Vec::new();
        let subtree_width = if children.is_empty() {
            node_width
        } else {
            let mut cursor = left;
            let mut total = 0.0;
            for (index, child) in children.iter().copied().enumerate() {
                let (child_index, child_width) =
                    visit(child, depth + 1, cursor, layout, label_of, children_of);
                child_centers.push((child_index, layout.nodes[child_index].x));
                cursor += child_width + TREE_H_GAP;
                total += child_width + if index > 0 { TREE_H_GAP } else { 0.0 };
            }
            total.max(node_width)
        };
        let center = child_centers
            .first()
            .zip(child_centers.last())
            .map(|(first, last)| (first.1 + last.1) / 2.0)
            .unwrap_or(left + subtree_width / 2.0);
        let index = layout.nodes.len();
        layout.nodes.push(TreeNode {
            label,
            x: center,
            y: depth as f32 * TREE_V_GAP,
            width: node_width,
        });
        for (child_index, child_x) in child_centers {
            layout.edges.push(TreeEdge {
                parent_x: center,
                parent_y: depth as f32 * TREE_V_GAP + TREE_NODE_HEIGHT,
                child_x,
                child_y: layout.nodes[child_index].y,
            });
        }
        (index, subtree_width)
    }

    let mut layout = TreeLayout::default();
    let (_, width) = visit(root, 0, 0.0, &mut layout, label_of, children_of);
    layout.width = width + TREE_MARGIN * 2.0;
    layout.height = layout.nodes.iter().map(|node| node.y).fold(0.0, f32::max)
        + TREE_NODE_HEIGHT
        + TREE_MARGIN * 2.0;
    for node in &mut layout.nodes {
        node.x += TREE_MARGIN;
        node.y += TREE_MARGIN;
    }
    for edge in &mut layout.edges {
        edge.parent_x += TREE_MARGIN;
        edge.parent_y += TREE_MARGIN;
        edge.child_x += TREE_MARGIN;
        edge.child_y += TREE_MARGIN;
    }
    layout
}

fn layout_tree(tree: &TreeValue) -> TreeLayout {
    let arena = tree.arena();
    layout_tree_nodes(
        tree.root(),
        &|node| arena.get_label(node).clone(),
        &|node| arena.get_children(node).to_vec(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_alto::ParseStrategy;
    use std::time::Duration;

    const SCFG: &str = r#"
interpretation english: de.up.ling.irtg.algebra.StringAlgebra
interpretation german: de.up.ling.irtg.algebra.StringAlgebra

S! -> r1(NP, VP)
  [english] *(?1, ?2)
  [german] *(?1, ?2)
NP -> r2(Det, N)
  [english] *(?1, ?2)
  [german] *(?1, ?2)
N -> r3(N, PP)
  [english] *(?1, ?2)
  [german] *(?1, ?2)
VP -> r4(V, NP)
  [english] *(?1, ?2)
  [german] *(?1, ?2)
VP -> r5(VP, PP)
  [english] *(?1, ?2)
  [german] *(?1, ?2)
PP -> r6(P, NP)
  [english] *(?1, ?2)
  [german] *(?1, ?2)
NP -> r7
  [english] john
  [german] hans
V -> r8
  [english] watches
  [german] betrachtet
Det -> r9
  [english] the
  [german] die
Det -> r9b
  [english] the
  [german] dem
N -> r10
  [english] woman
  [german] frau
N -> r11
  [english] telescope
  [german] fernrohr
P -> r12
  [english] with
  [german] mit
"#;

    #[test]
    fn loads_parses_and_pages_derivations() {
        let grammar_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/tiny.irtg");
        let document = load_grammar(grammar_path).expect("load example grammar");
        assert_eq!(document.summary.rule_count, 3);

        let chart = parse(
            document.grammar.clone(),
            vec![("string".into(), "john sleeps".into())],
            ParseStrategy::TopDownCondensed,
        )
        .expect("parse example input");
        assert!(!chart.summary.is_empty);
        assert!(chart.rules.iter().any(|rule| rule.parent == "NP[0,1]"));
        assert!(chart.rules.iter().any(|rule| rule.parent == "VP[1,2]"));

        let (tx, rx) = mpsc::channel();
        let worker = start_chart_language_worker(document.grammar, chart.automaton, tx);
        let size = match rx
            .recv_timeout(Duration::from_secs(5))
            .expect("language ready")
        {
            LanguageEvent::Ready(size) => size,
            other => panic!("expected ready event, got {other:?}"),
        };
        assert_eq!(size, LanguageCardinality::Finite(1));
        worker.request(0);
        let item = match rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first derivation")
        {
            LanguageEvent::Derivation(item) => item,
            other => panic!("expected derivation, got {other:?}"),
        };
        assert_eq!(item.index, 0);
        assert_eq!(item.views.len(), 3);
        assert!(item.views[0].tree.is_some());
        for view in item.views.iter().skip(1) {
            assert!(view.term.is_some());
        }
    }

    #[test]
    fn nonempty_ambiguous_chart_delivers_first_language_item() {
        let grammar = Arc::new(parse_irtg(SCFG.as_bytes()).expect("parse SCFG"));
        let chart = parse(
            grammar.clone(),
            vec![(
                "english".into(),
                "john watches the woman with the telescope".into(),
            )],
            ParseStrategy::TopDownCondensed,
        )
        .expect("parse ambiguous sentence");
        let (tx, rx) = mpsc::channel();
        let worker = start_chart_language_worker(grammar, chart.automaton, tx);
        let size = match rx
            .recv_timeout(Duration::from_secs(5))
            .expect("language ready")
        {
            LanguageEvent::Ready(size) => size,
            other => panic!("expected ready event, got {other:?}"),
        };
        assert_eq!(size, LanguageCardinality::Finite(8));
        worker.request(0);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5))
                .expect("first language item"),
            LanguageEvent::Derivation(_)
        ));
    }

    #[test]
    fn recursive_grammar_language_becomes_ready_as_infinite() {
        let grammar = Arc::new(parse_irtg(SCFG.as_bytes()).expect("parse SCFG"));
        let (tx, rx) = mpsc::channel();
        let _worker = start_grammar_language_worker(grammar, tx);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5))
                .expect("grammar language ready"),
            LanguageEvent::Ready(LanguageCardinality::Infinite)
        ));
    }
}
