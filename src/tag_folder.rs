use packed_term_arena::tree::{Tree, TreeArena};
use rusty_alto::{FeatureStructure, FeatureStructureAlgebra, HomLabel, Homomorphism, Irtg, Symbol};
use std::{error::Error, fmt};

#[derive(Debug, Clone)]
pub struct AnnotatedTree {
    pub label: String,
    pub top: Option<FeatureStructure>,
    pub bottom: Option<FeatureStructure>,
    pub children: Vec<AnnotatedTree>,
    pub provenance: NodeProvenance,
    pub foot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeProvenance {
    pub derivation_path: Vec<usize>,
    pub local_key: String,
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
    if grammar.interpretation_ref("tree").is_none() {
        return Err(TagFoldError::MissingInterpretation("tree"));
    }
    let features = grammar
        .interpretation::<FeatureStructureAlgebra>("ft")
        .map_err(|_| TagFoldError::MissingInterpretation("ft"))?;
    let mut path = Vec::new();
    fold_node(grammar, &features, derivation, root, &mut path)
}

fn fold_node(
    grammar: &Irtg,
    features: &rusty_alto::TypedInterpretation<'_, FeatureStructureAlgebra>,
    derivation: &TreeArena<Symbol>,
    node: Tree,
    path: &mut Vec<usize>,
) -> Result<AnnotatedTree, TagFoldError> {
    let source_symbol = *derivation.get_label(node);
    let symbol_name = grammar
        .grammar_signature()
        .resolve(source_symbol)
        .to_owned();
    let feature_value = features
        .interpret_derivation(derivation, node)
        .map_err(|error| TagFoldError::FeatureEvaluation(error.to_string()))?;

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
        folded_children.push(fold_node(grammar, features, derivation, child, path)?);
        path.pop();
    }
    instantiate(&template, &feature_value, &folded_children, path, true)
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
    features: &FeatureStructure,
    derivation_children: &[AnnotatedTree],
    path: &[usize],
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
            foot: false,
        }),
        Template::Foot => Ok(AnnotatedTree {
            label: "*".into(),
            top: features.project("foot"),
            bottom: None,
            children: Vec::new(),
            provenance: NodeProvenance {
                derivation_path: path.to_vec(),
                local_key: "foot".into(),
            },
            foot: true,
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
            replacement.top = features.project(key);
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
                .map(|child| instantiate(child, features, derivation_children, path, false))
                .collect::<Result<Vec<_>, _>>()?;
            let ordinary = AnnotatedTree {
                label: label.clone(),
                top: (!elementary_root)
                    .then(|| features.project(&format!("{key}t")))
                    .flatten(),
                bottom: features.project(&format!("{key}b")),
                children,
                provenance: NodeProvenance {
                    derivation_path: path.to_vec(),
                    local_key: key.clone(),
                },
                foot: false,
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

fn replace_foot(
    tree: &AnnotatedTree,
    replacement: &AnnotatedTree,
) -> Result<(AnnotatedTree, usize), TagFoldError> {
    if tree.foot {
        return Ok((replacement.clone(), 1));
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
    use rusty_alto::{InputCodec, TulipacInputCodec};

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
  S[][] { V+ [] [] }

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
            "final elementary root hides its own top"
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
}
