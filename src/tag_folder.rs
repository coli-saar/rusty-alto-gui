use packed_term_arena::tree::{Tree, TreeArena};
use rusty_alto::{FeatureStructure, HomLabel, Homomorphism, Irtg, Symbol};
use std::{collections::BTreeMap, error::Error, fmt};

#[derive(Debug, Clone)]
pub struct AnnotatedTree {
    pub label: String,
    pub top: Option<FeatureStructure>,
    pub bottom: Option<FeatureStructure>,
    pub children: Vec<AnnotatedTree>,
    pub provenance: NodeProvenance,
    pub provenance_aliases: Vec<NodeProvenance>,
    pub top_provenance: Option<NodeProvenance>,
    pub bottom_provenance: Option<NodeProvenance>,
    pub foot: bool,
    pub technical: bool,
    pub conflict: ConflictSide,
    pub top_source: ConflictSide,
    pub bottom_source: ConflictSide,
    pub top_conflict: bool,
    pub bottom_conflict: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConflictSide {
    #[default]
    None,
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeProvenance {
    pub derivation_path: Vec<usize>,
    pub local_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FeatureOrigin {
    pub derivation_path: Vec<usize>,
    pub grammar_symbol: String,
    pub local_key: String,
}

#[derive(Debug, Clone)]
pub enum FeatureFailureKind {
    Unification {
        path: Vec<String>,
        left: FeatureStructure,
        right: FeatureStructure,
        left_origins: Vec<FeatureOrigin>,
        right_origins: Vec<FeatureOrigin>,
    },
    Projection {
        attribute: String,
        origin: FeatureOrigin,
    },
    Remapping {
        specification: String,
        origin: FeatureOrigin,
    },
    InvalidOperation {
        operation: String,
        origin: FeatureOrigin,
    },
}

#[derive(Debug, Clone)]
pub struct FeatureFailure {
    pub operation: String,
    pub at: FeatureOrigin,
    pub kind: Box<FeatureFailureKind>,
}

#[derive(Debug, Clone)]
pub struct TagDiagnostic {
    pub tree: AnnotatedTree,
    pub failure: FeatureFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagFoldError {
    MissingInterpretation(&'static str),
    MissingHomomorphism {
        interpretation: &'static str,
        symbol: String,
    },
    FeatureEvaluation(String),
    UnsupportedTreeTerm(String),
    ChildOutOfRange {
        variable: usize,
        child_count: usize,
    },
    InvalidAdjunction(String),
}

impl fmt::Display for TagFoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInterpretation(name) => {
                write!(f, "TAG presentation needs an interpretation named {name:?}")
            }
            Self::MissingHomomorphism {
                interpretation,
                symbol,
            } => write!(
                f,
                "symbol {symbol:?} has no {interpretation:?} homomorphic image"
            ),
            Self::FeatureEvaluation(error) => {
                write!(f, "feature interpretation did not evaluate: {error}")
            }
            Self::UnsupportedTreeTerm(detail) => {
                write!(f, "unsupported TAG tree homomorphism: {detail}")
            }
            Self::ChildOutOfRange {
                variable,
                child_count,
            } => write!(
                f,
                "tree homomorphism variable ?{} has no derivation child ({child_count} available)",
                variable + 1
            ),
            Self::InvalidAdjunction(detail) => write!(f, "invalid TAG adjunction: {detail}"),
        }
    }
}

impl Error for TagFoldError {}

enum Template {
    Ordinary {
        label: String,
        children: Vec<Template>,
        adjunction_child: Option<usize>,
        key: String,
    },
    Substitution {
        child: usize,
        key: String,
    },
    Terminal {
        label: String,
    },
    Foot,
}

pub fn fold_tag_derivation(
    grammar: &Irtg,
    derivation: &TreeArena<Symbol>,
    root: Tree,
) -> Result<AnnotatedTree, TagFoldError> {
    if grammar.interpretation_ref("ft").is_none() {
        return Err(TagFoldError::MissingInterpretation("ft"));
    }
    let mut path = Vec::new();
    let tree = fold_node(grammar, derivation, root, &mut path)?;
    match evaluate_feature_derivation(grammar, derivation, root, &[]) {
        Ok(_) => Ok(tree),
        Err(failure) => Err(TagFoldError::FeatureEvaluation(format_failure(&failure))),
    }
}

pub fn diagnose_tag_derivation(
    grammar: &Irtg,
    derivation: &TreeArena<Symbol>,
    root: Tree,
) -> Result<TagDiagnostic, TagFoldError> {
    if grammar.interpretation_ref("ft").is_none() {
        return Err(TagFoldError::MissingInterpretation("ft"));
    }
    let mut path = Vec::new();
    let mut tree = fold_node(grammar, derivation, root, &mut path)?;
    let failure = evaluate_feature_derivation(grammar, derivation, root, &[])
        .err()
        .ok_or_else(|| {
            TagFoldError::FeatureEvaluation("feature interpretation evaluated successfully".into())
        })?;
    mark_conflicts(&mut tree, &failure);
    Ok(TagDiagnostic { tree, failure })
}

fn fold_node(
    grammar: &Irtg,
    derivation: &TreeArena<Symbol>,
    node: Tree,
    path: &mut Vec<usize>,
) -> Result<AnnotatedTree, TagFoldError> {
    let source_symbol = *derivation.get_label(node);
    let symbol_name = grammar
        .grammar_signature()
        .resolve(source_symbol)
        .to_owned();
    let technical_symbol = symbol_name.starts_with("*NOP*");
    let feature_value = evaluate_feature_derivation(grammar, derivation, node, path)
        .ok()
        .map(|value| value.value)
        .or_else(|| evaluate_local_feature_literals(grammar, source_symbol));

    let tree_interpretation = grammar
        .interpretation_ref("tree")
        .ok_or(TagFoldError::MissingInterpretation("tree"))?;
    let homomorphism = tree_interpretation.homomorphism();
    let term = homomorphism
        .get(source_symbol)
        .ok_or(TagFoldError::MissingHomomorphism {
            interpretation: "tree",
            symbol: symbol_name,
        })?;
    let mut template =
        decode_template(homomorphism, tree_interpretation.algebra_signature(), term)?;
    let mut next_key = 1;
    assign_keys(&mut template, &mut next_key);

    let mut folded_children = Vec::new();
    for (index, &child) in derivation.get_children(node).iter().enumerate() {
        path.push(index);
        folded_children.push(fold_node(grammar, derivation, child, path)?);
        path.pop();
    }
    let feature_owner_path = if technical_symbol && !path.is_empty() {
        &path[..path.len() - 1]
    } else {
        path.as_slice()
    };
    instantiate(
        &template,
        feature_value.as_ref(),
        &folded_children,
        path,
        feature_owner_path,
        technical_symbol,
        true,
    )
}

fn decode_template(
    homomorphism: &Homomorphism,
    signature: &rusty_alto::Signature,
    node: Tree,
) -> Result<Template, TagFoldError> {
    let arena = homomorphism.arena();
    match *arena.get_label(node) {
        HomLabel::Var(child) => Ok(Template::Substitution {
            child,
            key: String::new(),
        }),
        HomLabel::Symbol(symbol) => {
            let label = signature.resolve(symbol);
            let children = arena.get_children(node);
            if label == "@" {
                let [adjunction, ordinary] = children else {
                    return Err(TagFoldError::UnsupportedTreeTerm(
                        "@/2 must have exactly two children".into(),
                    ));
                };
                let HomLabel::Var(adjunction_child) = *arena.get_label(*adjunction) else {
                    return Err(TagFoldError::UnsupportedTreeTerm(
                        "the first child of @ must be an adjunction variable".into(),
                    ));
                };
                let mut template = decode_template(homomorphism, signature, *ordinary)?;
                match &mut template {
                    Template::Ordinary {
                        adjunction_child: slot,
                        ..
                    } => {
                        *slot = Some(adjunction_child);
                        Ok(template)
                    }
                    _ => Err(TagFoldError::UnsupportedTreeTerm(
                        "the second child of @ must be an ordinary TAG node".into(),
                    )),
                }
            } else if label == "*" {
                if children.is_empty() {
                    Ok(Template::Foot)
                } else {
                    Err(TagFoldError::UnsupportedTreeTerm(
                        "TAG foot marker must be nullary".into(),
                    ))
                }
            } else {
                let is_elementary_node = label.rsplit_once('_').is_some_and(|(_, suffix)| {
                    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
                });
                let children = children
                    .iter()
                    .map(|&child| decode_template(homomorphism, signature, child))
                    .collect::<Result<Vec<_>, _>>()?;
                if is_elementary_node {
                    Ok(Template::Ordinary {
                        label: strip_arity(label),
                        children,
                        adjunction_child: None,
                        key: String::new(),
                    })
                } else if children.is_empty() {
                    Ok(Template::Terminal {
                        label: label.to_owned(),
                    })
                } else {
                    Err(TagFoldError::UnsupportedTreeTerm(format!(
                        "non-TAG symbol {label:?} has children"
                    )))
                }
            }
        }
    }
}

fn strip_arity(label: &str) -> String {
    label
        .rsplit_once('_')
        .filter(|(_, suffix)| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
        .map_or_else(|| label.to_owned(), |(base, _)| base.to_owned())
}

fn assign_keys(template: &mut Template, next: &mut usize) {
    match template {
        Template::Ordinary { children, key, .. } => {
            for child in children {
                assign_keys(child, next);
            }
            *key = format!("n{next}");
            *next += 1;
        }
        Template::Substitution { key, .. } => {
            *key = format!("n{next}");
            *next += 1;
        }
        Template::Terminal { .. } | Template::Foot => {}
    }
}

fn instantiate(
    template: &Template,
    features: Option<&FeatureStructure>,
    derivation_children: &[AnnotatedTree],
    path: &[usize],
    feature_owner_path: &[usize],
    technical: bool,
    elementary_root: bool,
) -> Result<AnnotatedTree, TagFoldError> {
    match template {
        Template::Terminal { label } => Ok(AnnotatedTree {
            label: label.clone(),
            top: None,
            bottom: None,
            children: Vec::new(),
            provenance: NodeProvenance {
                derivation_path: path.to_vec(),
                local_key: String::new(),
            },
            provenance_aliases: Vec::new(),
            top_provenance: None,
            bottom_provenance: None,
            foot: false,
            technical,
            conflict: ConflictSide::None,
            top_source: ConflictSide::None,
            bottom_source: ConflictSide::None,
            top_conflict: false,
            bottom_conflict: false,
        }),
        Template::Foot => Ok(AnnotatedTree {
            label: "*".into(),
            top: features.and_then(|value| value.project("foot")),
            bottom: None,
            children: Vec::new(),
            provenance: NodeProvenance {
                derivation_path: path.to_vec(),
                local_key: "foot".into(),
            },
            provenance_aliases: Vec::new(),
            top_provenance: Some(NodeProvenance {
                derivation_path: feature_owner_path.to_vec(),
                local_key: "foot".into(),
            }),
            bottom_provenance: None,
            foot: true,
            technical,
            conflict: ConflictSide::None,
            top_source: ConflictSide::None,
            bottom_source: ConflictSide::None,
            top_conflict: false,
            bottom_conflict: false,
        }),
        Template::Substitution { child, key } => {
            let mut replacement =
                derivation_children
                    .get(*child)
                    .cloned()
                    .ok_or(TagFoldError::ChildOutOfRange {
                        variable: *child,
                        child_count: derivation_children.len(),
                    })?;
            replacement.provenance_aliases.push(NodeProvenance {
                derivation_path: path.to_vec(),
                local_key: key.clone(),
            });
            replacement.top = features.and_then(|value| value.project(key));
            replacement.top_provenance = replacement.top.as_ref().map(|_| NodeProvenance {
                derivation_path: path.to_vec(),
                local_key: key.clone(),
            });
            Ok(replacement)
        }
        Template::Ordinary {
            label,
            children,
            adjunction_child,
            key,
        } => {
            let children = children
                .iter()
                .map(|child| {
                    instantiate(
                        child,
                        features,
                        derivation_children,
                        path,
                        feature_owner_path,
                        technical,
                        false,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let top = (!elementary_root)
                .then(|| features.and_then(|value| value.project(&format!("{key}t"))))
                .flatten();
            let bottom = features.and_then(|value| value.project(&format!("{key}b")));
            let ordinary = AnnotatedTree {
                label: label.clone(),
                top: top.clone(),
                bottom: bottom.clone(),
                children,
                provenance: NodeProvenance {
                    derivation_path: path.to_vec(),
                    local_key: key.clone(),
                },
                provenance_aliases: elementary_root
                    .then(|| NodeProvenance {
                        derivation_path: path.to_vec(),
                        local_key: "root".into(),
                    })
                    .into_iter()
                    .collect(),
                top_provenance: top.as_ref().map(|_| NodeProvenance {
                    derivation_path: feature_owner_path.to_vec(),
                    local_key: format!("{key}t"),
                }),
                bottom_provenance: bottom.as_ref().map(|_| NodeProvenance {
                    derivation_path: feature_owner_path.to_vec(),
                    local_key: format!("{key}b"),
                }),
                foot: false,
                technical,
                conflict: ConflictSide::None,
                top_source: ConflictSide::None,
                bottom_source: ConflictSide::None,
                top_conflict: false,
                bottom_conflict: false,
            };
            if let Some(child) = adjunction_child {
                let auxiliary =
                    derivation_children
                        .get(*child)
                        .ok_or(TagFoldError::ChildOutOfRange {
                            variable: *child,
                            child_count: derivation_children.len(),
                        })?;
                let (tree, replaced) = replace_foot(auxiliary, &ordinary)?;
                match replaced {
                    1 => Ok(tree),
                    0 => Err(TagFoldError::InvalidAdjunction(
                        "auxiliary tree contains no foot node".into(),
                    )),
                    _ => Err(TagFoldError::InvalidAdjunction(
                        "auxiliary tree contains more than one foot node".into(),
                    )),
                }
            } else {
                Ok(ordinary)
            }
        }
    }
}

#[derive(Clone)]
struct TracedValue {
    value: FeatureStructure,
    origins: BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
}

fn evaluate_local_feature_literals(grammar: &Irtg, source: Symbol) -> Option<FeatureStructure> {
    let interpretation = grammar.interpretation_ref("ft")?;
    let homomorphism = interpretation.homomorphism();
    let term = homomorphism.get(source)?;
    let signature = interpretation.algebra_signature();
    let arena = homomorphism.arena();

    fn collect(
        arena: &TreeArena<HomLabel>,
        signature: &rusty_alto::Signature,
        node: Tree,
        values: &mut Vec<FeatureStructure>,
    ) {
        if let HomLabel::Symbol(symbol) = *arena.get_label(node)
            && arena.get_children(node).is_empty()
            && let Ok(value) = FeatureStructure::parse(signature.resolve(symbol))
        {
            values.push(value);
        }
        for &child in arena.get_children(node) {
            collect(arena, signature, child, values);
        }
    }

    let mut values = Vec::new();
    collect(arena, signature, term, &mut values);
    values
        .into_iter()
        .reduce(|left, right| left.unify(&right).unwrap_or(left))
}

fn evaluate_feature_derivation(
    grammar: &Irtg,
    derivation: &TreeArena<Symbol>,
    node: Tree,
    path: &[usize],
) -> Result<TracedValue, FeatureFailure> {
    let interpretation = grammar.interpretation_ref("ft").ok_or_else(|| {
        missing_feature_failure(grammar, derivation, node, path, "missing ft interpretation")
    })?;
    let source = *derivation.get_label(node);
    let symbol_name = grammar.grammar_signature().resolve(source).to_owned();
    let term = interpretation
        .homomorphism()
        .get(source)
        .ok_or_else(|| FeatureFailure {
            operation: "homomorphism".into(),
            at: FeatureOrigin {
                derivation_path: path.to_vec(),
                grammar_symbol: symbol_name.clone(),
                local_key: String::new(),
            },
            kind: Box::new(FeatureFailureKind::InvalidOperation {
                operation: "missing feature homomorphism".into(),
                origin: FeatureOrigin {
                    derivation_path: path.to_vec(),
                    grammar_symbol: symbol_name.clone(),
                    local_key: String::new(),
                },
            }),
        })?;
    let mut children = Vec::new();
    for (index, &child) in derivation.get_children(node).iter().enumerate() {
        let mut child_path = path.to_vec();
        child_path.push(index);
        children.push(evaluate_feature_derivation(
            grammar,
            derivation,
            child,
            &child_path,
        )?);
    }
    evaluate_feature_term(
        interpretation.homomorphism(),
        interpretation.algebra_signature(),
        term,
        &children,
        &FeatureOrigin {
            derivation_path: path.to_vec(),
            grammar_symbol: symbol_name,
            local_key: String::new(),
        },
    )
}

fn evaluate_feature_term(
    homomorphism: &Homomorphism,
    signature: &rusty_alto::Signature,
    node: Tree,
    children: &[TracedValue],
    at: &FeatureOrigin,
) -> Result<TracedValue, FeatureFailure> {
    let arena = homomorphism.arena();
    match *arena.get_label(node) {
        HomLabel::Var(index) => children.get(index).cloned().ok_or_else(|| FeatureFailure {
            operation: format!("?{}", index + 1),
            at: at.clone(),
            kind: Box::new(FeatureFailureKind::InvalidOperation {
                operation: "feature variable is out of range".into(),
                origin: at.clone(),
            }),
        }),
        HomLabel::Symbol(symbol) => {
            let label = signature.resolve(symbol);
            let arguments = arena
                .get_children(node)
                .iter()
                .map(|&child| evaluate_feature_term(homomorphism, signature, child, children, at))
                .collect::<Result<Vec<_>, _>>()?;
            match (label, arguments.as_slice()) {
                ("unify", [left, right]) => {
                    if let Some(value) = left.value.unify(&right.value) {
                        Ok(TracedValue {
                            value,
                            origins: merge_origins(&left.origins, &right.origins),
                        })
                    } else {
                        let path = first_conflict_path(&left.value, &right.value);
                        let local_path = path
                            .first()
                            .filter(|key| key.starts_with('n') || key.as_str() == "foot")
                            .map(std::slice::from_ref)
                            .unwrap_or_default();
                        let left_value = project_path(&left.value, local_path)
                            .unwrap_or_else(|| left.value.clone());
                        let right_value = project_path(&right.value, local_path)
                            .unwrap_or_else(|| right.value.clone());
                        Err(FeatureFailure {
                            operation: "unify".into(),
                            at: at.clone(),
                            kind: Box::new(FeatureFailureKind::Unification {
                                left_origins: origins_at(&left.origins, &path),
                                right_origins: origins_at(&right.origins, &path),
                                path,
                                left: left_value,
                                right: right_value,
                            }),
                        })
                    }
                }
                (_, [value]) if label.starts_with("proj_") => {
                    let attribute = &label["proj_".len()..];
                    let projected =
                        value
                            .value
                            .project(attribute)
                            .ok_or_else(|| FeatureFailure {
                                operation: label.to_owned(),
                                at: at.clone(),
                                kind: Box::new(FeatureFailureKind::Projection {
                                    attribute: attribute.to_owned(),
                                    origin: first_origin(value, at),
                                }),
                            })?;
                    Ok(TracedValue {
                        value: projected,
                        origins: project_origins(&value.origins, attribute),
                    })
                }
                (_, [value]) if label.starts_with("emb_") => {
                    let attribute = &label["emb_".len()..];
                    Ok(TracedValue {
                        value: value.value.embed(attribute),
                        origins: prefix_origins(&value.origins, attribute),
                    })
                }
                (_, [value]) if label.starts_with("remap_") => {
                    let specification = &label["remap_".len()..];
                    let mappings =
                        parse_remappings(specification).ok_or_else(|| FeatureFailure {
                            operation: label.to_owned(),
                            at: at.clone(),
                            kind: Box::new(FeatureFailureKind::Remapping {
                                specification: specification.to_owned(),
                                origin: first_origin(value, at),
                            }),
                        })?;
                    let borrowed = mappings
                        .iter()
                        .map(|(source, target)| (source.as_str(), target.as_str()))
                        .collect::<Vec<_>>();
                    let remapped = value.value.remap(&borrowed).ok_or_else(|| FeatureFailure {
                        operation: label.to_owned(),
                        at: at.clone(),
                        kind: Box::new(FeatureFailureKind::Remapping {
                            specification: specification.to_owned(),
                            origin: first_origin(value, at),
                        }),
                    })?;
                    Ok(TracedValue {
                        value: remapped,
                        origins: remap_origins(&value.origins, &mappings),
                    })
                }
                (_, []) => {
                    let value = FeatureStructure::parse(label).map_err(|_| FeatureFailure {
                        operation: label.to_owned(),
                        at: at.clone(),
                        kind: Box::new(FeatureFailureKind::InvalidOperation {
                            operation: label.to_owned(),
                            origin: at.clone(),
                        }),
                    })?;
                    Ok(TracedValue {
                        origins: literal_origins(&value, at),
                        value,
                    })
                }
                _ => Err(FeatureFailure {
                    operation: label.to_owned(),
                    at: at.clone(),
                    kind: Box::new(FeatureFailureKind::InvalidOperation {
                        operation: label.to_owned(),
                        origin: at.clone(),
                    }),
                }),
            }
        }
    }
}

fn literal_origins(
    value: &FeatureStructure,
    at: &FeatureOrigin,
) -> BTreeMap<Vec<String>, Vec<FeatureOrigin>> {
    fn visit(
        value: &FeatureStructure,
        node: rusty_alto::FeatureStructureNodeId,
        path: &mut Vec<String>,
        at: &FeatureOrigin,
        origins: &mut BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
    ) {
        let mut origin = at.clone();
        origin.local_key = path.first().cloned().unwrap_or_default();
        origins.entry(path.clone()).or_default().push(origin);
        if let Some(attributes) = value.attributes(node) {
            for attribute in attributes {
                path.push(attribute.name.to_owned());
                visit(value, attribute.value, path, at, origins);
                path.pop();
            }
        }
    }
    let mut origins = BTreeMap::new();
    visit(value, value.root(), &mut Vec::new(), at, &mut origins);
    origins
}

fn merge_origins(
    left: &BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
    right: &BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
) -> BTreeMap<Vec<String>, Vec<FeatureOrigin>> {
    let mut merged = left.clone();
    for (path, origins) in right {
        merged
            .entry(path.clone())
            .or_default()
            .extend(origins.iter().cloned());
    }
    for origins in merged.values_mut() {
        origins.sort();
        origins.dedup();
    }
    merged
}

fn project_origins(
    origins: &BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
    attribute: &str,
) -> BTreeMap<Vec<String>, Vec<FeatureOrigin>> {
    origins
        .iter()
        .filter(|(path, _)| path.first().is_some_and(|item| item == attribute))
        .map(|(path, values)| (path[1..].to_vec(), values.clone()))
        .collect()
}

fn prefix_origins(
    origins: &BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
    attribute: &str,
) -> BTreeMap<Vec<String>, Vec<FeatureOrigin>> {
    origins
        .iter()
        .map(|(path, values)| {
            let mut prefixed = vec![attribute.to_owned()];
            prefixed.extend(path.iter().cloned());
            (prefixed, values.clone())
        })
        .collect()
}

fn remap_origins(
    origins: &BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
    mappings: &[(String, String)],
) -> BTreeMap<Vec<String>, Vec<FeatureOrigin>> {
    let mut remapped = BTreeMap::new();
    for (source, target) in mappings {
        for (path, values) in origins {
            if path.first() == Some(source) {
                let mut new_path = vec![target.clone()];
                new_path.extend(path.iter().skip(1).cloned());
                remapped.insert(new_path, values.clone());
            }
        }
    }
    remapped
}

fn parse_remappings(specification: &str) -> Option<Vec<(String, String)>> {
    specification
        .split(',')
        .map(|item| {
            let (source, target) = item.split_once('=')?;
            (!source.is_empty() && !target.is_empty())
                .then(|| (source.to_owned(), target.to_owned()))
        })
        .collect()
}

fn first_conflict_path(left: &FeatureStructure, right: &FeatureStructure) -> Vec<String> {
    use rusty_alto::FeatureStructureNode;
    fn descend(
        left: &FeatureStructure,
        left_node: rusty_alto::FeatureStructureNodeId,
        right: &FeatureStructure,
        right_node: rusty_alto::FeatureStructureNodeId,
        path: &mut Vec<String>,
    ) -> bool {
        match (left.node(left_node), right.node(right_node)) {
            (Some(FeatureStructureNode::Variable), _)
            | (_, Some(FeatureStructureNode::Variable)) => false,
            (Some(FeatureStructureNode::Atom(a)), Some(FeatureStructureNode::Atom(b))) => a != b,
            (Some(FeatureStructureNode::Map), Some(FeatureStructureNode::Map)) => {
                let left_attributes = left
                    .attributes(left_node)
                    .into_iter()
                    .flatten()
                    .map(|attribute| (attribute.name.to_owned(), attribute.value))
                    .collect::<BTreeMap<_, _>>();
                for attribute in right.attributes(right_node).into_iter().flatten() {
                    if let Some(&left_child) = left_attributes.get(attribute.name) {
                        path.push(attribute.name.to_owned());
                        if descend(left, left_child, right, attribute.value, path) {
                            return true;
                        }
                        path.pop();
                    }
                }
                false
            }
            _ => true,
        }
    }
    let mut path = Vec::new();
    let _ = descend(left, left.root(), right, right.root(), &mut path);
    path
}

fn project_path(value: &FeatureStructure, path: &[String]) -> Option<FeatureStructure> {
    path.iter()
        .try_fold(value.clone(), |value, attribute| value.project(attribute))
}

fn origins_at(
    origins: &BTreeMap<Vec<String>, Vec<FeatureOrigin>>,
    path: &[String],
) -> Vec<FeatureOrigin> {
    for length in (0..=path.len()).rev() {
        if let Some(found) = origins.get(&path[..length]) {
            return found.clone();
        }
    }
    Vec::new()
}

fn first_origin(value: &TracedValue, fallback: &FeatureOrigin) -> FeatureOrigin {
    value
        .origins
        .values()
        .flatten()
        .next()
        .cloned()
        .unwrap_or_else(|| fallback.clone())
}

fn missing_feature_failure(
    grammar: &Irtg,
    derivation: &TreeArena<Symbol>,
    node: Tree,
    path: &[usize],
    operation: &str,
) -> FeatureFailure {
    let origin = FeatureOrigin {
        derivation_path: path.to_vec(),
        grammar_symbol: grammar
            .grammar_signature()
            .resolve(*derivation.get_label(node))
            .to_owned(),
        local_key: String::new(),
    };
    FeatureFailure {
        operation: operation.into(),
        at: origin.clone(),
        kind: Box::new(FeatureFailureKind::InvalidOperation {
            operation: operation.into(),
            origin,
        }),
    }
}

fn mark_conflicts(tree: &mut AnnotatedTree, failure: &FeatureFailure) {
    let (left, right, left_value, right_value, conflict_path) = match failure.kind.as_ref() {
        FeatureFailureKind::Unification {
            left_origins,
            right_origins,
            left,
            right,
            path,
            ..
        } => (
            left_origins.as_slice(),
            right_origins.as_slice(),
            Some(left),
            Some(right),
            path.as_slice(),
        ),
        FeatureFailureKind::Projection { origin, .. }
        | FeatureFailureKind::Remapping { origin, .. }
        | FeatureFailureKind::InvalidOperation { origin, .. } => {
            (std::slice::from_ref(origin), &[][..], None, None, &[][..])
        }
    };
    let belongs_left = left
        .iter()
        .any(|origin| tree_matches_derivation(tree, &origin.derivation_path));
    let belongs_right = right
        .iter()
        .any(|origin| tree_matches_derivation(tree, &origin.derivation_path));
    let conflict_site = tree_matches_conflict_site(tree, failure, conflict_path);
    tree.conflict = match (belongs_left, belongs_right) {
        (true, true) => ConflictSide::Both,
        (true, false) => ConflictSide::Left,
        (false, true) => ConflictSide::Right,
        (false, false) => ConflictSide::None,
    };
    tree.top_source = tree
        .top_provenance
        .as_ref()
        .map(|owner| source_for_path(&owner.derivation_path, left, right))
        .unwrap_or(ConflictSide::None);
    tree.bottom_source = tree
        .bottom_provenance
        .as_ref()
        .map(|owner| source_for_path(&owner.derivation_path, left, right))
        .unwrap_or(ConflictSide::None);
    tree.top_conflict = false;
    tree.bottom_conflict = false;
    if conflict_site {
        tree.top = left_value.cloned();
        tree.bottom = right_value.cloned();
        tree.top_conflict = left_value.is_some();
        tree.bottom_conflict = right_value.is_some();
    }
    for child in &mut tree.children {
        mark_conflicts(child, failure);
    }
}

fn source_for_path(
    path: &[usize],
    left: &[FeatureOrigin],
    right: &[FeatureOrigin],
) -> ConflictSide {
    match (
        left.iter().any(|origin| origin.derivation_path == path),
        right.iter().any(|origin| origin.derivation_path == path),
    ) {
        (true, true) => ConflictSide::Both,
        (true, false) => ConflictSide::Left,
        (false, true) => ConflictSide::Right,
        (false, false) => ConflictSide::None,
    }
}

fn tree_matches_conflict_site(
    tree: &AnnotatedTree,
    failure: &FeatureFailure,
    path: &[String],
) -> bool {
    let local_key = path
        .first()
        .filter(|key| key.starts_with('n') || key.as_str() == "foot")
        .map(String::as_str)
        .unwrap_or_else(|| failure.at.local_key.as_str());
    if local_key.is_empty() {
        return tree_matches_derivation(tree, &failure.at.derivation_path);
    }
    std::iter::once(&tree.provenance)
        .chain(&tree.provenance_aliases)
        .any(|provenance| {
            provenance.derivation_path == failure.at.derivation_path
                && normalize_local_key(&provenance.local_key) == normalize_local_key(local_key)
        })
}

fn tree_matches_derivation(tree: &AnnotatedTree, path: &[usize]) -> bool {
    tree.provenance.derivation_path == path
        || tree
            .provenance_aliases
            .iter()
            .any(|provenance| provenance.derivation_path == path)
}

fn normalize_local_key(key: &str) -> &str {
    if key.starts_with('n') && (key.ends_with('t') || key.ends_with('b')) {
        &key[..key.len() - 1]
    } else {
        key
    }
}

fn format_failure(failure: &FeatureFailure) -> String {
    match failure.kind.as_ref() {
        FeatureFailureKind::Unification {
            path, left, right, ..
        } => format!(
            "unification failed at {}: {left} conflicts with {right}",
            if path.is_empty() {
                "<root>".into()
            } else {
                path.join(".")
            }
        ),
        FeatureFailureKind::Projection { attribute, .. } => {
            format!("projection {attribute:?} failed")
        }
        FeatureFailureKind::Remapping { specification, .. } => {
            format!("remapping {specification:?} failed")
        }
        FeatureFailureKind::InvalidOperation { operation, .. } => {
            format!("feature operation {operation:?} failed")
        }
    }
}

fn replace_foot(
    tree: &AnnotatedTree,
    replacement: &AnnotatedTree,
) -> Result<(AnnotatedTree, usize), TagFoldError> {
    if tree.foot {
        let mut replacement = replacement.clone();
        replacement.provenance_aliases.push(tree.provenance.clone());
        if !tree.technical && tree.top.is_some() {
            replacement.top = tree.top.clone();
            replacement.top_provenance = tree.top_provenance.clone();
        }
        return Ok((replacement, 1));
    }
    let mut count = 0;
    let children = tree
        .children
        .iter()
        .map(|child| {
            let (child, replaced) = replace_foot(child, replacement)?;
            count += replaced;
            Ok(child)
        })
        .collect::<Result<Vec<_>, TagFoldError>>()?;
    let mut result = tree.clone();
    result.children = children;
    Ok((result, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_alto::{InputCodec, TulipacInputCodec, parse_irtg};

    const SUBSTITUTION_TAG: &str = r#"
tree sentence:
  S[top=s][bottom=s] {
    NP![case=nom][]
    VP[top=vp][bottom=vp] { V+ }
  }

tree noun:
  NP[top=np][bottom=np] { N+ }

word 'sleeps': sentence
word 'john': noun
"#;

    const ADJUNCTION_TAG: &str = r#"
tree base:
  S[][] { S[][] { V+ [] [] } }

tree adverb:
  S[][] {
    Adv+[][]
    S*[]
  }

word 'sleeps': base
word 'quickly': adverb
"#;

    const NESTED_SUBSTITUTION_TAG: &str = r#"
tree sentence:
  S @NA [][] { NP! [outer=yes][] V+ @NA [][] }

tree phrase:
  NP @NA [][] { N! [inner=yes][] D+ @NA [][] }

tree noun:
  N @NA [][] { W+ @NA [][] }

word 'runs': sentence
word 'john': phrase
word 'name': noun
"#;

    const CONFLICTING_TAG: &str = r#"
tree sentence:
  S @NA [][] { NP! [case=nom][] V+ @NA [][] }

tree noun:
  NP @NA [case=acc][] { N+ @NA [][] }

word 'sleeps': sentence
word 'john': noun
"#;

    fn first_folded(grammar_text: &str) -> AnnotatedTree {
        let grammar = TulipacInputCodec::new().decode(grammar_text).unwrap();
        let mut language = grammar.grammar().sorted_language();
        let weighted = language.next().unwrap();
        let (arena, root) = language.clone_tree(weighted.tree());
        fold_tag_derivation(&grammar, &arena, root).unwrap()
    }

    #[test]
    fn substituted_root_gets_site_top_and_keeps_own_bottom() {
        let tree = first_folded(SUBSTITUTION_TAG);
        assert_eq!(tree.label, "S");
        assert!(
            tree.top.is_none(),
            "final elementary root hides its own top: {:?}",
            tree.top
        );
        assert_eq!(
            tree.bottom.as_ref().unwrap().to_string(),
            "[bottom: s, top: s]"
        );
        let np = &tree.children[0];
        assert_eq!(np.label, "NP");
        assert_eq!(
            np.top.as_ref().unwrap().to_string(),
            "[bottom: np, case: nom, top: np]"
        );
        assert_eq!(
            np.bottom.as_ref().unwrap().to_string(),
            "[bottom: np, top: np]"
        );
    }

    #[test]
    fn numbering_is_local_left_to_right_postorder() {
        let tree = first_folded(SUBSTITUTION_TAG);
        assert_eq!(tree.children[0].children[0].provenance.local_key, "n1");
        assert_eq!(tree.children[0].provenance.local_key, "n2");
        assert_eq!(tree.children[1].children[0].provenance.local_key, "n2");
        assert_eq!(tree.children[1].provenance.local_key, "n3");
        assert_eq!(tree.provenance.local_key, "n4");
        assert_eq!(
            tree.children[0].provenance.derivation_path,
            vec![0],
            "substituted nodes retain their source derivation"
        );
    }

    #[test]
    fn folding_is_deterministic() {
        let left = first_folded(SUBSTITUTION_TAG);
        let right = first_folded(SUBSTITUTION_TAG);
        fn snapshot(tree: &AnnotatedTree, out: &mut Vec<String>) {
            out.push(format!(
                "{}:{}:{:?}:{:?}",
                tree.label,
                tree.provenance.local_key,
                tree.top.as_ref().map(ToString::to_string),
                tree.bottom.as_ref().map(ToString::to_string)
            ));
            for child in &tree.children {
                snapshot(child, out);
            }
        }
        let mut left_snapshot = Vec::new();
        let mut right_snapshot = Vec::new();
        snapshot(&left, &mut left_snapshot);
        snapshot(&right, &mut right_snapshot);
        assert_eq!(left_snapshot, right_snapshot);
    }

    #[test]
    fn adjunction_replaces_the_foot_and_preserves_auxiliary_nodes() {
        let grammar = TulipacInputCodec::new().decode(ADJUNCTION_TAG).unwrap();
        let mut language = grammar.grammar().sorted_language();
        let mut folded = None;
        for _ in 0..32 {
            let weighted = language.next().unwrap();
            let (arena, root) = language.clone_tree(weighted.tree());
            let candidate = fold_tag_derivation(&grammar, &arena, root).unwrap();
            let mut labels = Vec::new();
            fn collect(tree: &AnnotatedTree, labels: &mut Vec<String>) {
                labels.push(tree.label.clone());
                for child in &tree.children {
                    collect(child, labels);
                }
            }
            collect(&candidate, &mut labels);
            if labels.iter().any(|label| label == "Adv") {
                folded = Some((candidate, labels));
                break;
            }
        }
        let (tree, labels) = folded.expect("an adjoined derivation");
        assert_eq!(tree.label, "S");
        assert!(labels.iter().any(|label| label == "Adv"));
        assert!(!labels.iter().any(|label| label == "*"));
        assert!(
            tree.children.iter().any(|child| child.label == "S"),
            "the host tree replaces the auxiliary foot"
        );
        fn mixed_adjunction_boundary(tree: &AnnotatedTree) -> bool {
            let mixed = tree
                .top_provenance
                .as_ref()
                .zip(tree.bottom_provenance.as_ref())
                .is_some_and(|(top, bottom)| {
                    top.local_key == "foot" && top.derivation_path != bottom.derivation_path
                });
            mixed || tree.children.iter().any(mixed_adjunction_boundary)
        }
        assert!(
            mixed_adjunction_boundary(&tree),
            "the inserted host node should retain separate auxiliary-top and host-bottom provenance"
        );
    }

    #[test]
    fn missing_feature_interpretation_is_a_structured_error() {
        let grammar = TulipacInputCodec::new()
            .decode(
                r#"
tree sentence:
  S @NA { V+ }
word 'sleeps': sentence
"#,
            )
            .unwrap();
        let mut language = grammar.grammar().sorted_language();
        let weighted = language.next().unwrap();
        let (arena, root) = language.clone_tree(weighted.tree());
        assert_eq!(
            fold_tag_derivation(&grammar, &arena, root).unwrap_err(),
            TagFoldError::MissingInterpretation("ft")
        );
    }

    #[test]
    fn nested_substitutions_transfer_each_sites_top_features() {
        let tree = first_folded(NESTED_SUBSTITUTION_TAG);
        let np = &tree.children[0];
        let noun = &np.children[0];
        assert_eq!(np.label, "NP");
        assert_eq!(noun.label, "N");
        assert!(np.top.as_ref().unwrap().to_string().contains("outer: yes"));
        assert!(
            noun.top
                .as_ref()
                .unwrap()
                .to_string()
                .contains("inner: yes")
        );
        assert_eq!(np.provenance.derivation_path, vec![0]);
        assert_eq!(noun.provenance.derivation_path, vec![0, 0]);
    }

    #[test]
    fn diagnostic_reports_first_atomic_clash_with_origins() {
        let grammar = TulipacInputCodec::new().decode(CONFLICTING_TAG).unwrap();
        let mut language = grammar.grammar().sorted_language();
        let weighted = language.next().unwrap();
        let (arena, root) = language.clone_tree(weighted.tree());
        let diagnostic = diagnose_tag_derivation(&grammar, &arena, root).unwrap();
        let FeatureFailureKind::Unification {
            path,
            left,
            right,
            left_origins,
            right_origins,
        } = diagnostic.failure.kind.as_ref()
        else {
            panic!("expected a unification conflict");
        };
        assert!(path.ends_with(&["case".to_owned()]), "{path:?}");
        assert_ne!(left.to_string(), right.to_string());
        assert!(left.to_string().starts_with('['));
        assert!(right.to_string().starts_with('['));
        assert!(left.to_string().contains("case:"));
        assert!(right.to_string().contains("case:"));
        assert!(!left_origins.is_empty());
        assert!(!right_origins.is_empty());
        fn conflicts(
            tree: &AnnotatedTree,
            sides: &mut Vec<ConflictSide>,
            incompatible_nodes: &mut usize,
        ) {
            if tree.conflict != ConflictSide::None {
                sides.push(tree.conflict);
            }
            if tree.conflict == ConflictSide::Both && tree.top.is_some() && tree.bottom.is_some() {
                *incompatible_nodes += 1;
            }
            for child in &tree.children {
                conflicts(child, sides, incompatible_nodes);
            }
        }
        let mut sides = Vec::new();
        let mut incompatible_nodes = 0;
        conflicts(&diagnostic.tree, &mut sides, &mut incompatible_nodes);
        assert!(!sides.is_empty());
        assert!(
            incompatible_nodes > 0,
            "the conflict node should show incompatible top and bottom feature structures"
        );
        assert!(
            sides.contains(&ConflictSide::Both)
                || (sides.contains(&ConflictSide::Left) && sides.contains(&ConflictSide::Right)),
            "sides={sides:?}, left={left_origins:?}, right={right_origins:?}"
        );

        fn assert_ordinary_annotations(tree: &AnnotatedTree, is_final_root: bool) {
            if tree.provenance.local_key.starts_with('n') {
                assert!(
                    tree.bottom.is_some(),
                    "{} should retain its bottom FS after a descendant failure",
                    tree.label
                );
                if !is_final_root {
                    assert!(
                        tree.top.is_some(),
                        "{} should retain its top FS after a descendant failure",
                        tree.label
                    );
                }
            }
            for child in &tree.children {
                assert_ordinary_annotations(child, false);
            }
        }
        assert_ordinary_annotations(&diagnostic.tree, true);
    }

    #[test]
    fn diagnostic_reports_nested_conflict_path() {
        let left = FeatureStructure::parse("[agreement: [case: nom, number: sg]]").unwrap();
        let right = FeatureStructure::parse("[agreement: [case: acc, number: sg]]").unwrap();
        assert_eq!(
            first_conflict_path(&left, &right),
            vec!["agreement".to_owned(), "case".to_owned()]
        );
    }

    #[test]
    fn diagnostic_distinguishes_projection_and_remapping_failures() {
        fn diagnose(ft_term: &str) -> FeatureFailureKind {
            let grammar = parse_irtg(
                format!(
                    r#"
interpretation tree: de.up.ling.irtg.algebra.TagTreeAlgebra
interpretation ft: de.up.ling.irtg.algebra.FeatureStructureAlgebra
S! -> bad
  [tree] S_0
  [ft] {ft_term}
"#
                )
                .as_bytes(),
            )
            .unwrap();
            let mut language = grammar.grammar().sorted_language();
            let weighted = language.next().unwrap();
            let (arena, root) = language.clone_tree(weighted.tree());
            *diagnose_tag_derivation(&grammar, &arena, root)
                .unwrap()
                .failure
                .kind
        }

        assert!(matches!(
            diagnose(r#"proj_missing("[present: yes]")"#),
            FeatureFailureKind::Projection { attribute, .. } if attribute == "missing"
        ));
        assert!(matches!(
            diagnose(r#"'remap_missing=target'("[present: yes]")"#),
            FeatureFailureKind::Remapping { specification, .. }
                if specification == "missing=target"
        ));
    }
}
