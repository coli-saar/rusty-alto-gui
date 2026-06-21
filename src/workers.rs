use crate::model::{
    ChartDocument, DerivationPresentation, FeatureStructureBox, FeatureStructureLayout,
    FeatureStructureLine, FeatureStructureText, GrammarDocument, HeuristicChoice, RuleRow,
    StrategyChoice, TreeEdge, TreeLayout, TreeNode, ValuePresentation, ViewContent,
};
use packed_term_arena::tree::{Tree, TreeArena};
use rusty_alto::{
    AstarHeuristic, AstarOptions, Explicit, FeatureStructure, FeatureStructureNode,
    FeatureStructureNodeId, InputCodecRegistry, Irtg, LanguageCardinality, LogProbabilityScorer,
    MaterializationStrategy, ObligatoryLeafTables, ParseControl, Symbol, TreeValue,
    UniversalSxHeuristic, VisualRepresentation,
};
use std::{
    collections::{HashMap, HashSet},
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
    Failed(String),
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
    let registry = InputCodecRegistry::standard();
    let codec = registry
        .codec_for_path::<Irtg>(&path)
        .map_err(|error| error.to_string())?;
    let grammar = Arc::new(codec.read_path(&path).map_err(|error| error.to_string())?);
    let summary = grammar.grammar().application_summary();
    let interpretations = grammar.interpretation_info();
    let interpretation_names = interpretations
        .iter()
        .map(|info| info.name.clone())
        .collect();
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

#[cfg(test)]
pub fn parse(
    grammar: Arc<Irtg>,
    inputs: Vec<(String, String)>,
    required_valid: Vec<String>,
    strategy: StrategyChoice,
    heuristic: HeuristicChoice,
    stop_at_first_goal: bool,
) -> Result<ChartDocument, String> {
    parse_controlled(
        grammar,
        inputs,
        required_valid,
        strategy,
        heuristic,
        stop_at_first_goal,
        ParseControl::new(),
    )
}

pub fn parse_controlled(
    grammar: Arc<Irtg>,
    inputs: Vec<(String, String)>,
    required_valid: Vec<String>,
    strategy: StrategyChoice,
    heuristic: HeuristicChoice,
    stop_at_first_goal: bool,
    control: ParseControl,
) -> Result<ChartDocument, String> {
    let start = Instant::now();
    let mut parsed = Vec::with_capacity(inputs.len());
    // Remember the first string-algebra input (its homomorphism + length drive
    // the SX heuristic).
    let mut string_input: Option<(String, usize)> = None;
    for (name, text) in &inputs {
        let interpretation = grammar
            .interpretation_ref(name)
            .ok_or_else(|| format!("Unknown interpretation {name:?}"))?;
        if string_input.is_none() && interpretation.algebra_signature().get("*").is_some() {
            string_input = Some((name.clone(), text.split_whitespace().count()));
        }
        let value = interpretation
            .parse_object_erased(text)
            .map_err(|error| error.to_string())?;
        parsed.push(interpretation.input_erased(value));
    }

    // A* heuristic tables must outlive the parse, so build them up front.
    let mut sx_table: Option<UniversalSxHeuristic> = None;
    let mut oblig: Option<ObligatoryLeafTables> = None;
    let mut sx_n = 0usize;
    if strategy == StrategyChoice::Astar && heuristic != HeuristicChoice::Zero {
        let (name, n) = string_input.ok_or_else(|| {
            "The SX heuristic needs a string-algebra interpretation input.".to_string()
        })?;
        let interpretation = grammar
            .interpretation_ref(&name)
            .expect("string interpretation present");
        let concat = interpretation
            .algebra_signature()
            .get("*")
            .unwrap_or(Symbol(0));
        sx_n = n;
        sx_table = Some(UniversalSxHeuristic::new_with(
            grammar.grammar(),
            interpretation.homomorphism(),
            concat,
            n,
            &LogProbabilityScorer,
        ));
        if heuristic == HeuristicChoice::Sxf {
            oblig = Some(ObligatoryLeafTables::from_grammar(
                grammar.grammar(),
                interpretation.homomorphism(),
            ));
        }
    }

    let materialization = match strategy {
        StrategyChoice::TopDown => MaterializationStrategy::TopDownCondensed,
        StrategyChoice::Indexed => MaterializationStrategy::IndexedCondensed,
        StrategyChoice::Astar => MaterializationStrategy::Astar {
            heuristic: match heuristic {
                HeuristicChoice::Zero => AstarHeuristic::Zero,
                HeuristicChoice::Sx => AstarHeuristic::UniversalSx {
                    table: sx_table.as_ref().expect("sx table built"),
                    n: sx_n,
                },
                HeuristicChoice::Sxf => AstarHeuristic::UniversalSxF {
                    table: sx_table.as_ref().expect("sx table built"),
                    oblig: oblig.as_ref().expect("oblig built"),
                    n: sx_n,
                },
            },
            options: AstarOptions {
                stop_at_first_goal,
                beam: None,
            },
        },
    };
    let result = grammar
        .parse_with_control(parsed, &materialization, &control)
        .map_err(|error| error.to_string())?;
    let mut automaton = result.automaton;
    let mut state_names = result.state_names;
    let mut state_parts = result.state_parts;
    for name in required_valid {
        let filtered = grammar
            .filter_non_null_with_state_origins_controlled(&automaton, &name, &control)
            .map_err(|error| error.to_string())?;
        state_names = filtered
            .state_origins
            .iter()
            .map(|(source, filter_state)| {
                let source_name = state_names
                    .get(source.index())
                    .cloned()
                    .unwrap_or_else(|| format!("q{}", source.0));
                format!("{source_name} × q{filter_state}")
            })
            .collect();
        state_parts = filtered
            .state_origins
            .iter()
            .map(|(source, filter_state)| {
                let mut parts = state_parts
                    .get(source.index())
                    .cloned()
                    .unwrap_or_else(|| vec![format!("q{}", source.0)]);
                parts.push(format!("q{filter_state}"));
                parts
            })
            .collect();
        automaton = filtered.automaton;
    }
    let automaton = Arc::new(automaton);
    let summary = automaton.application_summary();
    let resolved = automaton.resolve_rules(
        |state| {
            state_names
                .get(state.index())
                .cloned()
                .unwrap_or_else(|| format!("q{}", state.0))
        },
        |symbol| grammar.grammar_signature().resolve(symbol).to_owned(),
    );
    let rules = resolved
        .iter()
        .zip(automaton.rules())
        .map(|(resolved, rule)| {
            let parts_for = |state: rusty_alto::StateId| {
                state_parts
                    .get(state.index())
                    .cloned()
                    .unwrap_or_else(|| vec![format!("q{}", state.0)])
            };
            RuleRow::from_resolved_with_parts(
                resolved,
                parts_for(rule.result),
                rule.children.iter().copied().map(parts_for).collect(),
            )
        })
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
    spawn_language_thread(grammar, automaton, request_rx, sender, worker_cancelled);
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
    let automaton = Arc::new(grammar.grammar().clone());
    spawn_language_thread(grammar, automaton, request_rx, sender, worker_cancelled);
    LanguageWorker {
        sender: request_tx,
        cancelled,
    }
}

fn spawn_language_thread(
    grammar: Arc<Irtg>,
    automaton: Arc<Explicit>,
    requests: Receiver<usize>,
    events: Sender<LanguageEvent>,
    cancelled: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let failure_events = events.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prepare_and_run_language(&grammar, &automaton, requests, events, &cancelled);
        }));
        if result.is_err() && !cancelled.load(Ordering::Relaxed) {
            let _ = failure_events.send(LanguageEvent::Failed(
                "The background language worker stopped unexpectedly.".into(),
            ));
        }
    });
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
            let mut views = vec![view_from_tree("Derivation tree", &derivation)];
            for interpretation in grammar.interpretations() {
                let name = interpretation.name().to_owned();
                let mut term_arena = TreeArena::<Symbol>::new();
                let term = interpretation
                    .homomorphism()
                    .apply(&arena, root, &mut term_arena)
                    .map(|term_root| {
                        let signature = interpretation.algebra_signature();
                        Arc::new(layout_tree_nodes(
                            term_root,
                            &|node| signature.resolve(*term_arena.get_label(node)).to_owned(),
                            &|node| term_arena.get_children(node).to_vec(),
                        ))
                    });
                let view = match term {
                    Err(error) => interpretation_error_view(
                        name,
                        None,
                        format!("Could not construct the interpretation term: {error}"),
                    ),
                    Ok(term) => match interpretation.evaluate_derivation(&arena, root) {
                        Err(error) => interpretation_error_view(
                            name,
                            Some(term),
                            format!(
                                "The derivation tree did not evaluate in this algebra: {error}"
                            ),
                        ),
                        Ok(evaluated) => {
                            let value = match evaluated.visual() {
                                VisualRepresentation::Text(text) => {
                                    ValuePresentation::Text(text.clone())
                                }
                                VisualRepresentation::Tree(tree) => {
                                    ValuePresentation::Tree(Arc::new(layout_tree(tree)))
                                }
                                VisualRepresentation::FeatureStructure(feature) => {
                                    ValuePresentation::FeatureStructure(Arc::new(
                                        layout_feature_structure(feature),
                                    ))
                                }
                            };
                            let codecs = evaluated.codecs();
                            ViewContent {
                                term: Some(term),
                                name,
                                value,
                                evaluated: Some(Arc::new(evaluated)),
                                codecs,
                            }
                        }
                    },
                };
                views.push(view);
            }
            cache.push(Arc::new(DerivationPresentation {
                index: cache.len(),
                weight: weighted.weight(),
                views,
            }));
        }
        if let Some(item) = cache.get(requested)
            && events
                .send(LanguageEvent::Derivation(item.clone()))
                .is_err()
        {
            return;
        }
    }
}

fn interpretation_error_view(
    name: String,
    term: Option<Arc<TreeLayout>>,
    error: String,
) -> ViewContent {
    ViewContent {
        name,
        term,
        value: ValuePresentation::Error(error),
        evaluated: None,
        codecs: Vec::new(),
    }
}

fn view_from_tree(name: impl Into<String>, tree: &TreeValue) -> ViewContent {
    ViewContent {
        name: name.into(),
        term: None,
        value: ValuePresentation::Tree(Arc::new(layout_tree(tree))),
        evaluated: None,
        codecs: Vec::new(),
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
    struct Subtree {
        nodes: Vec<TreeNode>,
        edges: Vec<TreeEdge>,
        width: f32,
        height: f32,
        root_x: f32,
    }

    fn visit<L, C>(node: Tree, label_of: &L, children_of: &C) -> Subtree
    where
        L: Fn(Tree) -> String,
        C: Fn(Tree) -> Vec<Tree>,
    {
        let label = label_of(node);
        let node_width = (label.chars().count() as f32 * 7.5 + 22.0).clamp(58.0, 220.0);
        let children = children_of(node)
            .into_iter()
            .map(|child| visit(child, label_of, children_of))
            .collect::<Vec<_>>();

        if children.is_empty() {
            return Subtree {
                nodes: vec![TreeNode {
                    label,
                    x: node_width / 2.0,
                    y: 0.0,
                    width: node_width,
                }],
                edges: Vec::new(),
                width: node_width,
                height: TREE_NODE_HEIGHT,
                root_x: node_width / 2.0,
            };
        }

        let children_width = children.iter().map(|child| child.width).sum::<f32>()
            + TREE_H_GAP * children.len().saturating_sub(1) as f32;
        let mut child_roots = Vec::with_capacity(children.len());
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut cursor = 0.0;
        let mut height: f32 = 0.0;

        for child in children {
            let child_root = cursor + child.root_x;
            child_roots.push(child_root);
            height = height.max(TREE_V_GAP + child.height);
            nodes.extend(child.nodes.into_iter().map(|mut child_node| {
                child_node.x += cursor;
                child_node.y += TREE_V_GAP;
                child_node
            }));
            edges.extend(child.edges.into_iter().map(|mut child_edge| {
                child_edge.parent_x += cursor;
                child_edge.child_x += cursor;
                child_edge.parent_y += TREE_V_GAP;
                child_edge.child_y += TREE_V_GAP;
                child_edge
            }));
            cursor += child.width + TREE_H_GAP;
        }

        let root_x = (child_roots[0] + child_roots[child_roots.len() - 1]) / 2.0;
        // A wide parent over narrow or asymmetric children can extend beyond
        // their combined span. Include that overhang in the subtree bounds and
        // shift all descendants so every declared coordinate is non-negative.
        let left = (root_x - node_width / 2.0).min(0.0);
        let right = (root_x + node_width / 2.0).max(children_width);
        let shift_x = -left;
        for child_node in &mut nodes {
            child_node.x += shift_x;
        }
        for child_edge in &mut edges {
            child_edge.parent_x += shift_x;
            child_edge.child_x += shift_x;
        }
        for child_root in &mut child_roots {
            *child_root += shift_x;
        }
        let root_x = root_x + shift_x;

        for child_x in child_roots {
            edges.push(TreeEdge {
                parent_x: root_x,
                parent_y: TREE_NODE_HEIGHT,
                child_x,
                child_y: TREE_V_GAP,
            });
        }
        nodes.push(TreeNode {
            label,
            x: root_x,
            y: 0.0,
            width: node_width,
        });

        Subtree {
            nodes,
            edges,
            width: right - left,
            height: height.max(TREE_NODE_HEIGHT),
            root_x,
        }
    }

    let subtree = visit(root, label_of, children_of);
    let mut layout = TreeLayout {
        nodes: subtree.nodes,
        edges: subtree.edges,
        width: subtree.width + TREE_MARGIN * 2.0,
        height: subtree.height + TREE_MARGIN * 2.0,
    };
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

const FS_CHAR_WIDTH: f32 = 7.6;
const FS_LINE_HEIGHT: f32 = 24.0;
const FS_ROW_GAP: f32 = 6.0;
const FS_BRACKET_WIDTH: f32 = 8.0;
const FS_PADDING: f32 = 8.0;
const FS_COLUMN_GAP: f32 = 14.0;
const FS_MARKER_SIZE: f32 = 20.0;

fn layout_feature_structure(value: &FeatureStructure) -> FeatureStructureLayout {
    fn count_incoming(
        value: &FeatureStructure,
        node: FeatureStructureNodeId,
        incoming: &mut HashMap<FeatureStructureNodeId, usize>,
        visited: &mut HashSet<FeatureStructureNodeId>,
    ) {
        if !visited.insert(node) {
            return;
        }
        if let Some(attributes) = value.attributes(node) {
            for attribute in attributes {
                *incoming.entry(attribute.value).or_default() += 1;
                count_incoming(value, attribute.value, incoming, visited);
            }
        }
    }

    fn assign_markers(
        value: &FeatureStructure,
        node: FeatureStructureNodeId,
        incoming: &HashMap<FeatureStructureNodeId, usize>,
        markers: &mut HashMap<FeatureStructureNodeId, usize>,
        visited: &mut HashSet<FeatureStructureNodeId>,
    ) {
        if incoming.get(&node).copied().unwrap_or_default() > 1 && !markers.contains_key(&node) {
            markers.insert(node, markers.len() + 1);
        }
        if !visited.insert(node) {
            return;
        }
        if let Some(attributes) = value.attributes(node) {
            for attribute in attributes {
                assign_markers(value, attribute.value, incoming, markers, visited);
            }
        }
    }

    fn text_block(text: String) -> FeatureStructureLayout {
        let width = (text.chars().count() as f32 * FS_CHAR_WIDTH).max(12.0);
        FeatureStructureLayout {
            texts: vec![FeatureStructureText {
                text,
                x: 0.0,
                y: FS_LINE_HEIGHT / 2.0,
                centered: false,
            }],
            width,
            height: FS_LINE_HEIGHT,
            ..Default::default()
        }
    }

    fn marker_block(number: usize) -> FeatureStructureLayout {
        FeatureStructureLayout {
            texts: vec![FeatureStructureText {
                text: number.to_string(),
                x: FS_MARKER_SIZE / 2.0,
                y: FS_MARKER_SIZE / 2.0,
                centered: true,
            }],
            boxes: vec![FeatureStructureBox {
                x: 0.0,
                y: 0.0,
                width: FS_MARKER_SIZE,
                height: FS_MARKER_SIZE,
            }],
            width: FS_MARKER_SIZE,
            height: FS_MARKER_SIZE,
            ..Default::default()
        }
    }

    fn append_at(
        target: &mut FeatureStructureLayout,
        mut source: FeatureStructureLayout,
        x: f32,
        y: f32,
    ) {
        for text in &mut source.texts {
            text.x += x;
            text.y += y;
        }
        for line in &mut source.lines {
            line.from_x += x;
            line.to_x += x;
            line.from_y += y;
            line.to_y += y;
        }
        for item in &mut source.boxes {
            item.x += x;
            item.y += y;
        }
        target.texts.extend(source.texts);
        target.lines.extend(source.lines);
        target.boxes.extend(source.boxes);
    }

    fn node_block(
        value: &FeatureStructure,
        node: FeatureStructureNodeId,
        markers: &HashMap<FeatureStructureNodeId, usize>,
        expanded: &mut HashSet<FeatureStructureNodeId>,
    ) -> FeatureStructureLayout {
        let marker = markers.get(&node).copied();
        if let Some(marker) = marker
            && !expanded.insert(node)
        {
            return marker_block(marker);
        }
        if marker.is_none() {
            expanded.insert(node);
        }

        let mut body = match value.node(node) {
            Some(FeatureStructureNode::Variable) => text_block("[]".to_owned()),
            Some(FeatureStructureNode::Atom(atom)) => text_block(atom.to_owned()),
            Some(FeatureStructureNode::Map) => {
                let attributes = value
                    .attributes(node)
                    .map(|attributes| attributes.collect::<Vec<_>>())
                    .unwrap_or_default();
                let attribute_width = attributes
                    .iter()
                    .map(|attribute| attribute.name.chars().count() as f32 * FS_CHAR_WIDTH)
                    .fold(0.0, f32::max);
                let children = attributes
                    .iter()
                    .map(|attribute| node_block(value, attribute.value, markers, expanded))
                    .collect::<Vec<_>>();
                let child_width = children.iter().map(|child| child.width).fold(0.0, f32::max);
                let row_heights = children
                    .iter()
                    .map(|child| child.height.max(FS_LINE_HEIGHT) + FS_ROW_GAP)
                    .collect::<Vec<_>>();
                let content_height = if attributes.is_empty() {
                    FS_LINE_HEIGHT
                } else {
                    row_heights.iter().sum::<f32>() - FS_ROW_GAP
                };
                let width = FS_BRACKET_WIDTH * 2.0
                    + FS_PADDING * 2.0
                    + attribute_width
                    + if attributes.is_empty() {
                        0.0
                    } else {
                        FS_COLUMN_GAP + child_width
                    };
                let height = content_height + FS_PADDING * 2.0;
                let mut layout = FeatureStructureLayout {
                    width,
                    height,
                    ..Default::default()
                };
                let left = FS_BRACKET_WIDTH;
                let right = width - FS_BRACKET_WIDTH;
                for (x, inward) in [(left, 1.0), (right, -1.0)] {
                    layout.lines.push(FeatureStructureLine {
                        from_x: x,
                        from_y: 0.0,
                        to_x: x + inward * FS_BRACKET_WIDTH,
                        to_y: 0.0,
                    });
                    layout.lines.push(FeatureStructureLine {
                        from_x: x,
                        from_y: 0.0,
                        to_x: x,
                        to_y: height,
                    });
                    layout.lines.push(FeatureStructureLine {
                        from_x: x,
                        from_y: height,
                        to_x: x + inward * FS_BRACKET_WIDTH,
                        to_y: height,
                    });
                }
                let mut y = FS_PADDING;
                for ((attribute, child), row_height) in attributes
                    .iter()
                    .zip(children)
                    .zip(row_heights.iter().copied())
                {
                    let child_height = child.height;
                    let content_row_height = row_height - FS_ROW_GAP;
                    layout.texts.push(FeatureStructureText {
                        text: attribute.name.to_owned(),
                        x: FS_BRACKET_WIDTH + FS_PADDING,
                        y: y + content_row_height / 2.0,
                        centered: false,
                    });
                    append_at(
                        &mut layout,
                        child,
                        FS_BRACKET_WIDTH + FS_PADDING + attribute_width + FS_COLUMN_GAP,
                        y + (content_row_height - child_height) / 2.0,
                    );
                    y += row_height;
                }
                layout
            }
            None => text_block("?".to_owned()),
        };

        if let Some(number) = marker {
            let marker = marker_block(number);
            let gap = 6.0;
            let width = marker.width + gap + body.width;
            let height = marker.height.max(body.height);
            let mut combined = FeatureStructureLayout {
                width,
                height,
                ..Default::default()
            };
            append_at(&mut combined, marker, 0.0, (height - FS_MARKER_SIZE) / 2.0);
            let body_y = (height - body.height) / 2.0;
            append_at(&mut combined, body, FS_MARKER_SIZE + gap, body_y);
            body = combined;
        }
        body
    }

    let root = value.root();
    let mut incoming = HashMap::from([(root, 1)]);
    count_incoming(value, root, &mut incoming, &mut HashSet::new());
    let mut markers = HashMap::new();
    assign_markers(value, root, &incoming, &mut markers, &mut HashSet::new());
    let mut layout = node_block(value, root, &markers, &mut HashSet::new());
    let margin = 22.0;
    for text in &mut layout.texts {
        text.x += margin;
        text.y += margin;
    }
    for line in &mut layout.lines {
        line.from_x += margin;
        line.to_x += margin;
        line.from_y += margin;
        line.to_y += margin;
    }
    for item in &mut layout.boxes {
        item.x += margin;
        item.y += margin;
    }
    layout.width += margin * 2.0;
    layout.height += margin * 2.0;
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_alto::parse_irtg;
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

    const SHIEBER_TAG: &str = r#"
family vinf_tv: { vinf_tv, vinf_tv_aux }

tree vinf_tv:
  S @NA {
    np! [case=nom][]
    S { np! [case=?o] [] }
    v+ [objcase=?o] []
  }

tree vinf_tv_aux:
  S @NA {
    S { S @NA { np! [case=?o] [] S* } }
    v+ [objcase=?o][]
  }

family np_n: { np_n }

tree np_n:
  np [] [case=?c] { n+ [case=?c] [] }

tree adj_det:
  np [] [case=?c] {
    det+ [case=?c] []
    np* [case=?c] []
  }

tree np_pron:
  np[][case=?c] { pron+ [case=?c] [] }

word 'mer': np_pron[case=nom]
word 'em': adj_det[case=dat]
word 'es': adj_det[case=acc]
word 'd': adj_det[case=acc]
word 'de': adj_det[case=acc]
word 'hans': np_n
word 'huus': np_n
word 'chind': np_n
word 'aastriiche': <vinf_tv>[objcase=acc]

lemma 'laa': <vinf_tv>[objcase=acc] {
  word "lönd"
  word "laa"
}

lemma 'hälfe': <vinf_tv>[objcase=dat] {
  word 'hälfed'
  word 'hälfe'
}
"#;

    fn assert_tree_layout_is_bounded(layout: &TreeLayout) {
        assert_eq!(layout.edges.len() + 1, layout.nodes.len());
        for node in &layout.nodes {
            assert!(
                node.x - node.width / 2.0 >= 0.0,
                "node {:?} escaped the left layout bound",
                node.label
            );
            assert!(
                node.x + node.width / 2.0 <= layout.width,
                "node {:?} escaped the right layout bound",
                node.label
            );
            assert!(node.y >= 0.0);
            assert!(node.y + TREE_NODE_HEIGHT <= layout.height);
        }
        for edge in &layout.edges {
            for x in [edge.parent_x, edge.child_x] {
                assert!((0.0..=layout.width).contains(&x));
            }
            for y in [edge.parent_y, edge.child_y] {
                assert!((0.0..=layout.height).contains(&y));
            }
        }
    }

    #[test]
    fn loads_parses_and_pages_derivations() {
        let directory = std::env::temp_dir().join(format!("rusty_alto_gui_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let grammar_path = directory.join("tiny.irtg");
        std::fs::write(
            &grammar_path,
            r#"
interpretation string: de.up.ling.irtg.algebra.StringAlgebra
interpretation tree: de.up.ling.irtg.algebra.TreeWithAritiesAlgebra
S! -> r(NP, VP)
  [string] *(?1, ?2)
  [tree] S_2(?1, ?2)
NP -> john
  [string] john
  [tree] john_0
VP -> sleeps
  [string] sleeps
  [tree] sleeps_0
"#,
        )
        .unwrap();
        let document = load_grammar(grammar_path).expect("load example grammar");
        assert_eq!(document.summary.rule_count, 3);

        let chart = parse(
            document.grammar.clone(),
            vec![("string".into(), "john sleeps".into())],
            Vec::new(),
            StrategyChoice::TopDown,
            HeuristicChoice::Zero,
            false,
        )
        .expect("parse example input");
        assert!(!chart.summary.is_empty);
        assert!(chart.rules.iter().any(|rule| rule.parent == "NP[0-1]"));
        assert!(chart.rules.iter().any(|rule| rule.parent == "VP[1-2]"));

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
        assert!(matches!(item.views[0].value, ValuePresentation::Tree(_)));
        for view in item.views.iter().skip(1) {
            assert!(view.term.is_some());
            assert!(view.evaluated.is_some());
            assert_eq!(view.codecs.len(), 1);
        }
        let string = item
            .views
            .iter()
            .find(|view| view.name == "string")
            .unwrap();
        assert!(matches!(
            &string.value,
            ValuePresentation::Text(text) if text == "john sleeps"
        ));
        assert_eq!(
            string.evaluated.as_ref().unwrap().encode("string").unwrap(),
            "john sleeps"
        );
    }

    #[test]
    fn chart_rule_state_names_are_consistent_across_strategies() {
        let directory =
            std::env::temp_dir().join(format!("rusty_alto_gui_state_names_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let grammar_path = directory.join("tiny.irtg");
        std::fs::write(
            &grammar_path,
            r#"
interpretation string: de.up.ling.irtg.algebra.StringAlgebra
S! -> r(NP, VP)
  [string] *(?1, ?2)
NP -> john
  [string] john
VP -> sleeps
  [string] sleeps
"#,
        )
        .unwrap();
        let grammar = load_grammar(grammar_path).unwrap().grammar;

        for strategy in StrategyChoice::ALL {
            let chart = parse(
                grammar.clone(),
                vec![("string".into(), "john sleeps".into())],
                Vec::new(),
                strategy,
                HeuristicChoice::Zero,
                false,
            )
            .unwrap();
            assert!(chart.rules.iter().any(|rule| rule.parent == "NP[0-1]"));
            assert!(chart.rules.iter().any(|rule| rule.parent == "VP[1-2]"));
            assert!(chart.rules.iter().any(|rule| rule.parent == "S[0-2]!"));
            assert!(chart.rules.iter().all(|rule| !rule.parent.starts_with('q')));
        }
    }

    #[test]
    fn grammar_loading_uses_registered_extensions_and_read_path() {
        let directory =
            std::env::temp_dir().join(format!("rusty_alto_gui_codecs_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let trees = directory.join("trees.tag");
        let grammar = directory.join("grammar.tag");
        std::fs::write(&trees, "tree v: S @NA { V+ }").unwrap();
        std::fs::write(&grammar, "#include 'trees.tag'\nword sleeps: v").unwrap();
        let loaded = load_grammar(grammar).expect("load Tulipac grammar with relative include");
        assert!(loaded.grammar.interpretation_ref("string").is_some());

        let unknown = directory.join("grammar.unknown");
        std::fs::write(&unknown, "").unwrap();
        assert!(
            load_grammar(unknown)
                .unwrap_err()
                .contains("no input codec")
        );
        let extensionless = directory.join("grammar");
        std::fs::write(&extensionless, "").unwrap();
        assert!(
            load_grammar(extensionless)
                .unwrap_err()
                .contains("extension")
        );
    }

    #[test]
    fn feature_structure_layout_marks_shared_nodes() {
        let value = FeatureStructure::parse("[left: #x [case: nom], right: #x, open: #y]").unwrap();
        let layout = layout_feature_structure(&value);
        assert!(layout.width > 100.0);
        assert!(layout.height > 40.0);
        assert!(layout.texts.iter().any(|text| text.text == "left"));
        assert!(layout.texts.iter().any(|text| text.text == "right"));
        assert!(layout.texts.iter().filter(|text| text.text == "1").count() >= 2);
        assert!(!layout.boxes.is_empty());
    }

    #[test]
    fn tree_layout_bounds_cover_wide_asymmetric_subtrees() {
        let mut arena = TreeArena::<String>::new();
        let deep_leaf = arena.add_node("deep".into(), vec![]);
        let deep = arena.add_node("right".into(), vec![deep_leaf]);
        let left = arena.add_node("left".into(), vec![]);
        let narrow_parent = arena.add_node("middle".into(), vec![left, deep]);
        let other = arena.add_node("x".into(), vec![]);
        let root = arena.add_node(
            "a-parent-label-much-wider-than-its-children".into(),
            vec![narrow_parent, other],
        );
        let layout = layout_tree_nodes(root, &|node| arena.get_label(node).clone(), &|node| {
            arena.get_children(node).to_vec()
        });

        assert_tree_layout_is_bounded(&layout);
    }

    #[test]
    fn feature_structure_primitives_stay_inside_reported_bounds() {
        let value = FeatureStructure::parse("[left: #x [case: nom], right: #x, open: #y]").unwrap();
        let layout = layout_feature_structure(&value);
        for text in &layout.texts {
            assert!((0.0..=layout.width).contains(&text.x));
            assert!((0.0..=layout.height).contains(&text.y));
        }
        for line in &layout.lines {
            for x in [line.from_x, line.to_x] {
                assert!((0.0..=layout.width).contains(&x));
            }
            for y in [line.from_y, line.to_y] {
                assert!((0.0..=layout.height).contains(&y));
            }
        }
        for item in &layout.boxes {
            assert!(item.x >= 0.0 && item.x + item.width <= layout.width);
            assert!(item.y >= 0.0 && item.y + item.height <= layout.height);
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
            Vec::new(),
            StrategyChoice::TopDown,
            HeuristicChoice::Zero,
            false,
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

    #[test]
    fn tag_language_enumeration_shows_invalid_interpretations_and_continues() {
        let directory =
            std::env::temp_dir().join(format!("rusty_alto_gui_shieber_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("shieber.tag");
        std::fs::write(&path, SHIEBER_TAG).unwrap();
        let grammar = load_grammar(path).unwrap().grammar;
        let (tx, rx) = mpsc::channel();
        let worker = start_grammar_language_worker(grammar.clone(), tx);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            LanguageEvent::Ready(LanguageCardinality::Infinite)
        ));

        let mut failed_index = None;
        for index in 0..64 {
            worker.request(index);
            let item = match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                LanguageEvent::Derivation(item) => item,
                other => panic!("expected derivation {index}, got {other:?}"),
            };
            assert_eq!(item.index, index);
            for view in &item.views {
                if let Some(term) = &view.term {
                    assert_tree_layout_is_bounded(term);
                }
                if let ValuePresentation::Tree(tree) = &view.value {
                    assert_tree_layout_is_bounded(tree);
                }
            }
            let feature_failed = matches!(
                &item.views.iter().find(|view| view.name == "ft").unwrap().value,
                ValuePresentation::Error(error) if !error.trim().is_empty()
            );
            if feature_failed {
                failed_index = Some(index);
                break;
            }
        }
        assert!(
            failed_index.is_some(),
            "fixture should display an invalid ft value"
        );
        let next_index = failed_index.unwrap() + 1;
        worker.request(next_index);
        let next = match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            LanguageEvent::Derivation(item) => item,
            other => panic!("expected derivation after invalid interpretation, got {other:?}"),
        };
        assert_eq!(next.index, next_index);
    }

    #[test]
    fn explicit_non_null_constraint_filters_tag_parse_chart() {
        let directory =
            std::env::temp_dir().join(format!("rusty_alto_gui_filter_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("shieber.tag");
        std::fs::write(&path, SHIEBER_TAG).unwrap();
        let grammar = load_grammar(path).unwrap().grammar;

        let unfiltered = parse(
            grammar.clone(),
            Vec::new(),
            Vec::new(),
            StrategyChoice::TopDown,
            HeuristicChoice::Zero,
            false,
        )
        .unwrap();
        let mut language = unfiltered.automaton.sorted_language();
        assert!((0..12).any(|_| {
            let weighted = language.next().unwrap();
            let (arena, root) = language.clone_tree(weighted.tree());
            grammar
                .interpretation_ref("ft")
                .unwrap()
                .evaluate_derivation(&arena, root)
                .is_err()
        }));

        let filtered = parse(
            grammar.clone(),
            Vec::new(),
            vec!["ft".into()],
            StrategyChoice::TopDown,
            HeuristicChoice::Zero,
            false,
        )
        .unwrap();
        assert!(
            filtered
                .rules
                .iter()
                .any(|rule| rule.parent.contains(" × q")),
            "filtered chart labels should preserve source states and append filter states"
        );
        assert!(
            filtered
                .rules
                .iter()
                .all(|rule| rule.parent_parts.len() == 2),
            "the feature filter should be a separate display component"
        );
        let mut language = filtered.automaton.sorted_language();
        for _ in 0..12 {
            let weighted = language.next().unwrap();
            let (arena, root) = language.clone_tree(weighted.tree());
            assert!(
                grammar
                    .interpretation_ref("ft")
                    .unwrap()
                    .evaluate_derivation(&arena, root)
                    .is_ok()
            );
        }
    }

    #[test]
    fn tag_input_and_non_null_filter_keep_three_display_parts() {
        let directory = std::env::temp_dir().join(format!(
            "rusty_alto_gui_tag_filter_parts_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("shieber.tag");
        std::fs::write(&path, SHIEBER_TAG).unwrap();
        let grammar = load_grammar(path).unwrap().grammar;
        let chart = parse(
            grammar,
            vec![("string".into(), "mer es huus aastriiche".into())],
            vec!["ft".into()],
            StrategyChoice::TopDown,
            HeuristicChoice::Zero,
            false,
        )
        .unwrap();

        assert!(!chart.rules.is_empty());
        assert!(chart.rules.iter().all(|rule| rule.parent_parts.len() == 3));
        assert!(chart.rules.iter().any(|rule| {
            rule.parent_parts[1].starts_with('[')
                && rule.parent_parts[1].ends_with(']')
                && rule.parent_parts[2].starts_with('q')
        }));
    }

    #[test]
    fn tag_input_chart_preserves_grammar_and_decomposition_state_names() {
        let directory =
            std::env::temp_dir().join(format!("rusty_alto_gui_tag_states_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("shieber.tag");
        std::fs::write(&path, SHIEBER_TAG).unwrap();
        let grammar = load_grammar(path).unwrap().grammar;
        let chart = parse(
            grammar,
            vec![("string".into(), "mer es huus aastriiche".into())],
            Vec::new(),
            StrategyChoice::TopDown,
            HeuristicChoice::Zero,
            false,
        )
        .unwrap();
        assert!(chart.rules.iter().any(|rule| rule.parent.contains(" × ")));
        assert!(
            chart
                .rules
                .iter()
                .any(|rule| rule.parent.contains("_S") && rule.parent.contains('['))
        );
        assert!(chart.rules.iter().all(|rule| !rule.parent.starts_with('q')));
    }
}
