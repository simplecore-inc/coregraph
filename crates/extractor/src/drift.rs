//! Documentation drift detection (docs/graph-model.md §7.7).
//!
//! A single-build, rule-based proxy for "the signature changed but the doc
//! didn't": a `@param name` (JSDoc / Javadoc) or `:param name:` (Python) that
//! names a parameter the function's actual signature does not have. This is a
//! detector/report — it adds no graph nodes or edges.
//!
//! Precision-first (this codebase is precision-oriented everywhere):
//! - only plain-identifier documented params are checked (dotted `opts.foo` and
//!   varargs `...args` are skipped — they have no single signature parameter);
//! - the actual parameter set is OVER-collected (every identifier inside the
//!   parameter list, including destructuring shorthand bindings), so a binding
//!   the walker misses can never produce a false drift — at the cost of recall,
//!   never precision;
//! - drift is only reported when the function has a non-empty parameter list;
//! - a single leading-underscore difference alone (`name` documented, `_name`
//!   in the signature) is the intentionally-unused-parameter convention — not
//!   drift; double underscores (`__name`) are a real mismatch;
//! - when a documented name is genuinely absent, rename candidates from the
//!   actual signature within edit distance 2 are surfaced in the report.
//!
//! NOT COVERED (no parameter tags in the convention, so nothing to check):
//! Rust rustdoc (`# Arguments` prose) and Go (sentence doc). The temporal
//! variant — doc-text vs signature hashes across reindexes — is a separate
//! incremental-infrastructure step, not this single-build check.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use coregraph_core::{levenshtein, EdgeKind, SymbolId, SymbolKind};
use coregraph_graph::SymbolGraph;
use regex::Regex;
use tree_sitter::{Language, Node, Parser};

/// The kind of documentation drift detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocDriftKind {
    /// A documented parameter (`@param` / `:param`) is not an actual parameter.
    DocumentedParamMissing,
}

/// A single documentation-drift finding.
#[derive(Debug, Clone)]
pub struct DocDriftReport {
    /// The documented symbol (function / method) name.
    pub symbol: String,
    /// The file the symbol and its doc live in.
    pub file: PathBuf,
    pub kind: DocDriftKind,
    /// Human-readable diagnostic.
    pub detail: String,
    /// Signature parameters within rename distance of the missing documented
    /// name (closest first); empty when nothing in the signature is close.
    pub candidates: Vec<String>,
}

/// tree-sitter grammar for a covered extension, or `None` if the language is
/// not covered by the parameter-drift check.
fn language_for_ext(ext: &str) -> Option<Language> {
    match ext {
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "py" | "pyi" => Some(tree_sitter_python::LANGUAGE.into()),
        _ => None,
    }
}

/// Over-collect every identifier inside the function's parameter list, so a
/// `@param` not in this set is genuinely absent. `None` on parse failure or when
/// the node has no `parameters` field.
fn param_tokens(language: &Language, source: &str, span: (u32, u32)) -> Option<HashSet<String>> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    let tree = parser.parse(source, None)?;
    let func = tree
        .root_node()
        .descendant_for_byte_range(span.0 as usize, span.1 as usize)?;

    // Find the parameter-list node by kind anywhere in the subtree — robust to
    // the function being wrapped (`export function f()`) or the span resolving
    // to an ancestor. The param list precedes the body in child order, so the
    // first match is the function's own parameters, not a nested closure's.
    let params = find_param_list(func)?;

    let mut out = HashSet::new();
    collect_identifiers(params, source.as_bytes(), &mut out);
    Some(out)
}

/// First parameter-list node (Java/TS/JS `formal_parameters`, Python
/// `parameters`) in `node`'s subtree, in pre-order.
fn find_param_list(node: Node) -> Option<Node> {
    if matches!(node.kind(), "formal_parameters" | "parameters") {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(p) = find_param_list(child) {
            return Some(p);
        }
    }
    None
}

fn collect_identifiers(node: Node, src: &[u8], out: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // `identifier` covers plain params; the shorthand-pattern kind covers
        // object destructuring bindings (`{ a, b }`) in the TS/JS grammars.
        // Over-collection is deliberate (precision-first): a binding the
        // walker misses becomes a false "missing param" drift.
        if matches!(
            child.kind(),
            "identifier" | "shorthand_property_identifier_pattern"
        ) {
            if let Ok(t) = child.utf8_text(src) {
                out.insert(t.to_string());
            }
        }
        collect_identifiers(child, src, out);
    }
}

/// Compiled doc-param extractors (compiled once, reused per doc).
struct DocParamRegexes {
    jsdoc: Regex,
    python: Regex,
}

impl DocParamRegexes {
    fn new() -> Self {
        Self {
            // @param {type} name | @param name | @param [name=default]
            jsdoc: Regex::new(r"@param\b\s*(?:\{[^}]*\})?\s*(\[?[\w$.]+)").unwrap(),
            // :param name: | :param type name:
            python: Regex::new(r":param\s+([^:]+):").unwrap(),
        }
    }

    fn documented(&self, is_python: bool, doc_text: &str) -> Vec<String> {
        let re = if is_python { &self.python } else { &self.jsdoc };
        let mut out = Vec::new();
        for cap in re.captures_iter(doc_text) {
            let raw = cap[1].trim();
            // Python ":param type name:" → the name is the last token.
            let token = if is_python {
                raw.split_whitespace().last().unwrap_or(raw)
            } else {
                raw
            };
            // Strip JSDoc optional-param brackets and `=default`.
            let token = token.trim_start_matches('[').trim_end_matches(']');
            let token = token.split('=').next().unwrap_or(token).trim();
            // Skip nested (`opts.foo`) and varargs (`...args`) — no single param.
            if token.is_empty() || token.contains('.') || token.starts_with("...") {
                continue;
            }
            let mut chars = token.chars();
            let first_ok = chars
                .next()
                .map(|c| c.is_alphabetic() || c == '_' || c == '$')
                .unwrap_or(false);
            if first_ok
                && token
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            {
                out.push(token.to_string());
            }
        }
        out
    }
}

/// Maximum edit distance for a signature parameter to count as a rename
/// candidate of a documented-but-missing parameter name.
const RENAME_CANDIDATE_MAX_DISTANCE: usize = 2;

/// Compare documented parameter names against the actual parameter set.
/// Returns, per genuinely-missing documented name, the rename candidates
/// found in the signature (edit distance <= RENAME_CANDIDATE_MAX_DISTANCE,
/// closest first). A SINGLE leading-underscore difference alone (`name`
/// documented, `_name` in the signature) is the unused-marker convention,
/// not drift, and is suppressed entirely; `__name` is a real mismatch.
fn missing_documented_params(
    documented: &[String],
    params: &HashSet<String>,
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for p in documented {
        if params.contains(p) {
            continue;
        }
        let logical = p.strip_prefix('_').unwrap_or(p);
        if params
            .iter()
            .any(|actual| actual.strip_prefix('_').unwrap_or(actual) == logical)
        {
            continue; // single-underscore unused-marker rename — still in sync
        }
        let mut candidates: Vec<(usize, String)> = params
            .iter()
            .map(|actual| (levenshtein(actual, p), actual.clone()))
            .filter(|(d, _)| *d <= RENAME_CANDIDATE_MAX_DISTANCE)
            .collect();
        candidates.sort();
        out.push((p.clone(), candidates.into_iter().map(|(_, c)| c).collect()));
    }
    out
}

/// Detect documented-parameter drift across the graph. Reads each documented
/// function's source file (the doc and the function share a file) to compare its
/// `@param` / `:param` tags against the real signature.
pub fn find_doc_param_drift(graph: &SymbolGraph) -> Vec<DocDriftReport> {
    // Documents edges whose target is a function / method.
    let pairs: Vec<(SymbolId, SymbolId)> = graph
        .edges()
        .filter(|e| e.kind == EdgeKind::Documents)
        .filter(|e| {
            graph
                .get_node(e.to)
                .map(|n| matches!(n.kind, SymbolKind::Function | SymbolKind::Method))
                .unwrap_or(false)
        })
        .map(|e| (e.from, e.to))
        .collect();

    let regexes = DocParamRegexes::new();
    let mut file_cache: HashMap<PathBuf, Option<String>> = HashMap::new();
    let mut out = Vec::new();

    for (doc_id, func_id) in pairs {
        let (Some(doc), Some(func)) = (graph.get_node(doc_id), graph.get_node(func_id)) else {
            continue;
        };
        let ext = func
            .file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(language) = language_for_ext(&ext) else {
            continue; // uncovered language
        };
        let is_python = matches!(ext.as_str(), "py" | "pyi");

        let source = file_cache
            .entry(func.file.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(&func.file).ok());
        let Some(source) = source.as_deref() else {
            continue;
        };

        let (ds, de) = (doc.span_start as usize, doc.span_end as usize);
        if ds > de || de > source.len() {
            continue;
        }
        let documented = regexes.documented(is_python, &source[ds..de]);
        if documented.is_empty() {
            continue;
        }

        let Some(params) = param_tokens(&language, source, (func.span_start, func.span_end)) else {
            continue;
        };
        if params.is_empty() {
            continue; // no signature params → nothing can mismatch
        }

        for (p, candidates) in missing_documented_params(&documented, &params) {
            let detail = if candidates.is_empty() {
                format!(
                    "documented parameter `{p}` is not a parameter of `{}`",
                    func.name
                )
            } else {
                format!(
                    "documented parameter `{p}` is not a parameter of `{}`; closest signature parameter: `{}` (likely rename)",
                    func.name, candidates[0]
                )
            };
            out.push(DocDriftReport {
                symbol: func.name.clone(),
                file: func.file.to_path_buf(),
                kind: DocDriftKind::DocumentedParamMissing,
                detail,
                candidates,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_extracts_jsdoc_params() {
        let r = DocParamRegexes::new();
        let got = r.documented(
            false,
            "/**\n * @param {string} name the name\n * @param age\n */",
        );
        assert_eq!(got, vec!["name", "age"]);
    }

    #[test]
    fn documented_skips_nested_and_varargs() {
        let r = DocParamRegexes::new();
        let got = r.documented(
            false,
            "@param opts.foo nested\n@param ...rest varargs\n@param ok plain",
        );
        assert_eq!(got, vec!["ok"], "dotted and varargs params are skipped");
    }

    #[test]
    fn documented_extracts_python_params() {
        let r = DocParamRegexes::new();
        let got = r.documented(true, ":param name: the name\n:param int count: how many");
        assert_eq!(got, vec!["name", "count"]);
    }

    fn ts_params(src: &str) -> HashSet<String> {
        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        param_tokens(&lang, src, (0, src.len() as u32)).expect("param list")
    }

    #[test]
    fn destructured_shorthand_params_are_collected() {
        // `function f({ a, b })` binds `a`/`b` via
        // shorthand_property_identifier_pattern nodes, not plain identifiers;
        // missing them turned every documented destructured prop into a
        // false "missing parameter" drift.
        let params = ts_params("function f({ role, children }: P, ctx: C) {}");
        assert!(params.contains("role"), "got {params:?}");
        assert!(params.contains("children"), "got {params:?}");
        assert!(params.contains("ctx"), "got {params:?}");
    }

    #[test]
    fn drift_check_underscore_rename_is_suppressed() {
        // `_name` marks an intentionally-unused param; a doc tag naming the
        // logical `name` is still in sync — not drift.
        let mut params = HashSet::new();
        params.insert("_responseAdapter".to_string());
        params.insert("config".to_string());
        let drift = missing_documented_params(&["responseAdapter".to_string()], &params);
        assert!(
            drift.is_empty(),
            "underscore rename must not report: {drift:?}"
        );
    }

    #[test]
    fn drift_check_double_underscore_is_not_suppressed() {
        // Only the single-underscore unused-marker convention is suppressed;
        // `__x` vs documented `x` is a real mismatch (and gets a candidate).
        let mut params = HashSet::new();
        params.insert("__config".to_string());
        let drift = missing_documented_params(&["config".to_string()], &params);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].1, vec!["__config".to_string()]);
    }

    #[test]
    fn drift_check_suggests_rename_candidates() {
        let mut params = HashSet::new();
        params.insert("responseAdapter".to_string());
        let drift = missing_documented_params(&["responseAdaptor".to_string()], &params);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].0, "responseAdaptor");
        assert_eq!(
            drift[0].1,
            vec!["responseAdapter".to_string()],
            "edit-distance candidate must be suggested"
        );
    }

    #[test]
    fn drift_check_no_candidates_for_distant_names() {
        let mut params = HashSet::new();
        params.insert("config".to_string());
        let drift = missing_documented_params(&["responseAdapter".to_string()], &params);
        assert_eq!(drift.len(), 1);
        assert!(drift[0].1.is_empty(), "unrelated names are not candidates");
    }
}
