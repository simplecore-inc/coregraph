use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use coregraph_core::edge::AnalysisOrigin;
use coregraph_graph::SymbolGraph;
use rayon::prelude::*;

#[derive(Debug, Clone)]
pub struct ResolvedRef {
    pub from_file: PathBuf,
    pub from_span: (u32, u32),
    pub to_file: PathBuf,
    pub to_span: (u32, u32),
    /// Origin of this individual reference. Stack-graphs-stitched refs are
    /// `NameResolved` (0.95); syntactic-fallback refs are `SyntaxMatched`
    /// (0.85). Carried per-ref so a mixed result (some stitched, some
    /// fallback) labels each edge correctly rather than promoting the whole
    /// batch off a single global `success` flag.
    pub origin: AnalysisOrigin,
}

#[derive(Debug)]
pub struct ResolutionResult {
    pub refs: Vec<ResolvedRef>,
    /// `true` when produced by a full name-resolution backend (e.g. stack-graphs);
    /// `false` when produced by the syntactic fallback in this module.
    pub success: bool,
}

/// Language-aware minimum-identifier length heuristic.
/// Helps filter out short, ambiguous identifiers (e.g. `fn`, `if`, `Ok`).
fn min_identifier_length(language: &str) -> usize {
    match language {
        "go" => 3, // Go has short names like `ctx`, `err`
        _ => 4,
    }
}

/// Well-known short identifiers that should never be treated as a symbol reference.
fn is_common_keyword(word: &str) -> bool {
    matches!(
        word,
        "self" | "this" | "type" | "fn" | "let" | "const" | "class" | "struct"
        | "enum" | "trait" | "impl" | "pub" | "mod" | "use" | "for" | "while"
        | "loop" | "if" | "else" | "match" | "return" | "async" | "await"
        | "true" | "false" | "null" | "None" | "Some" | "Ok" | "Err"
        | "package" | "import" | "export" | "public" | "private"
        | "protected" | "static" | "final" | "abstract" | "override"
        | "void" | "int" | "String" | "string" | "bool" | "float" | "double"
        // Very common method / field names that would blow up in-degree.
        // These are better modeled with typed Calls edges; skip them in the
        // syntactic-fallback resolver to reduce noise.
        | "name" | "kind" | "id" | "file" | "run" | "new" | "get" | "set"
        | "add" | "from" | "into" | "clone" | "default" | "size" | "len"
        | "format" | "path" | "value" | "index" | "data" | "result"
        | "error" | "test" | "tests" | "main" | "args" | "self_"
    )
}

/// Split a source file into identifier tokens — alphanumeric/underscore words.
///
/// Returns `BTreeSet` (not `HashSet`) so downstream iteration order is
/// deterministic. The function's caller feeds tokens into an edge
/// `refs` vector that is deduplicated with `dedup_by` (adjacent-only),
/// so iteration order actually decides which entries survive dedup
/// and, ultimately, the `Resolves` edge count. HashSet iteration
/// drifts run-to-run and was the last remaining source of
/// non-determinism in `edge_count()`.
fn identifiers_in(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut cur = String::new();
    for ch in source.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.insert(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.insert(cur);
    }
    out
}

/// Syntactic cross-file name-resolution fallback.
///
/// Strategy: for each file, tokenise into identifiers and look up each token in
/// the symbol graph. A token that matches a symbol defined in **another** file
/// produces a ResolvedRef from the current file to the defining file.
///
/// This is intentionally weaker than full stack-graphs resolution (no scoping,
/// no type inference) — hence `success = false`. The resulting edges are still
/// useful: they connect cross-file usages of named symbols, which is better
/// than nothing and drives orphan / impact analysis.
pub fn resolve_files_with_graph(
    files: &[(&Path, &str)],
    language: &str,
    graph: &SymbolGraph,
) -> ResolutionResult {
    // Build name → (defining_file, node_span) map from graph definitions.
    // One name may be defined in multiple files — track all.
    //
    // `ExternalPackage` nodes are deliberately excluded: those are
    // placeholder nodes created by the extractor for cross-crate
    // dependencies (`PathBuf`, `serde`, …). They carry no file path, so
    // including them as "definitions" makes every identifier match
    // across every file produce a spurious cross-file `Resolves` edge
    // pointing at the empty path. Observed impact: edge count 92k →
    // 330k before this filter.
    type DefSites = Vec<(PathBuf, (u32, u32))>;
    let mut defs: HashMap<String, DefSites> = HashMap::new();
    for node in graph.nodes() {
        if node.kind == coregraph_core::SymbolKind::ExternalPackage {
            continue;
        }
        defs.entry(node.name.clone())
            .or_default()
            .push((node.file.to_path_buf(), (node.span_start, node.span_end)));
    }

    let min_len = min_identifier_length(language);

    // Per-file ref extraction is embarrassingly parallel: `defs` is read-only
    // after construction and each file's tokenisation is independent. rayon's
    // par_iter on a slice preserves element order in the collected output, so
    // the subsequent adjacent-duplicate dedup behaves identically to the old
    // sequential loop (same file ordering → same adjacent pairs).
    let per_file_refs: Vec<Vec<ResolvedRef>> = files
        .par_iter()
        .map(|(path, source)| {
            let ids = identifiers_in(source);
            let here = path.to_path_buf();
            let mut file_refs = Vec::new();
            for token in ids {
                if token.len() < min_len {
                    continue;
                }
                if is_common_keyword(&token) {
                    continue;
                }
                if let Some(def_sites) = defs.get(&token) {
                    for (def_file, def_span) in def_sites {
                        if def_file == &here {
                            continue; // skip same-file
                        }
                        file_refs.push(ResolvedRef {
                            from_file: here.clone(),
                            from_span: (0, 0),
                            to_file: def_file.clone(),
                            to_span: *def_span,
                            origin: AnalysisOrigin::SyntaxMatched,
                        });
                    }
                }
            }
            file_refs
        })
        .collect();

    let mut refs: Vec<ResolvedRef> = per_file_refs.into_iter().flatten().collect();

    refs.dedup_by(|a, b| a.from_file == b.from_file && a.to_file == b.to_file);

    ResolutionResult {
        refs,
        success: false,
    }
}

/// Legacy entry point kept for backwards compatibility.
/// Uses a naive whitespace-split fallback when no graph is available.
/// Prefer `resolve_files_with_graph` which uses extracted symbol names.
pub fn resolve_files(files: &[(&Path, &str)], _language: &str) -> ResolutionResult {
    let mut definitions: Vec<(String, PathBuf)> = Vec::new();
    for (path, source) in files {
        for word in source.split_whitespace() {
            let clean: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !clean.is_empty() && clean.len() > 3 {
                definitions.push((clean, path.to_path_buf()));
            }
        }
    }

    let mut refs = Vec::new();
    for (path, source) in files {
        for word in source.split_whitespace() {
            let clean: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if let Some((_, def_file)) = definitions
                .iter()
                .find(|(name, def_path)| *name == clean && *def_path != path.to_path_buf())
            {
                refs.push(ResolvedRef {
                    from_file: path.to_path_buf(),
                    from_span: (0, 0),
                    to_file: def_file.clone(),
                    to_span: (0, 0),
                    origin: AnalysisOrigin::SyntaxMatched,
                });
            }
        }
    }
    refs.dedup_by(|a, b| a.from_file == b.from_file && a.to_file == b.to_file);

    ResolutionResult {
        refs,
        success: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn insert(g: &mut SymbolGraph, name: &str, file: &str, span: (u32, u32)) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            name,
            PathBuf::from(file),
            span.0,
            span.1,
        ))
    }

    #[test]
    fn legacy_fallback_finds_cross_file_names() {
        let a = PathBuf::from("A.java");
        let b = PathBuf::from("B.java");
        let result = resolve_files(
            &[
                (&a, "class UserController { UserService service; }"),
                (&b, "class UserService {}"),
            ],
            "java",
        );
        assert!(!result.success);
        assert!(!result.refs.is_empty());
    }

    #[test]
    fn graph_backed_resolver_matches_defined_names_only() {
        let mut g = SymbolGraph::new();
        insert(&mut g, "UserService", "B.java", (0, 80));
        // "WidgetWidget" is not in the graph — should not match
        let a = PathBuf::from("A.java");
        let b = PathBuf::from("B.java");
        let result = resolve_files_with_graph(
            &[
                (
                    &a,
                    "class UserController { UserService svc; WidgetWidget w; }",
                ),
                (&b, "class UserService {}"),
            ],
            "java",
            &g,
        );
        assert!(!result.success, "syntactic fallback never claims success");
        assert_eq!(result.refs.len(), 1, "only UserService should resolve");
        assert_eq!(result.refs[0].to_file, b);
    }

    #[test]
    fn graph_backed_resolver_skips_same_file() {
        let mut g = SymbolGraph::new();
        insert(&mut g, "LocalThing", "only.rs", (0, 20));
        let f = PathBuf::from("only.rs");
        let result = resolve_files_with_graph(
            &[(&f, "struct LocalThing {} fn foo() { LocalThing {} }")],
            "rust",
            &g,
        );
        assert!(result.refs.is_empty(), "same-file refs are not cross-file");
    }

    #[test]
    fn graph_backed_resolver_filters_keywords() {
        let mut g = SymbolGraph::new();
        // "let" would match if we didn't filter keywords
        insert(&mut g, "let", "a.rs", (0, 3));
        let b = PathBuf::from("b.rs");
        let result = resolve_files_with_graph(&[(&b, "let x = 42;")], "rust", &g);
        assert!(result.refs.is_empty(), "keywords should not resolve");
    }

    #[test]
    fn graph_backed_resolver_min_length() {
        let mut g = SymbolGraph::new();
        // "fx" is too short (under Rust's 4-char threshold)
        insert(&mut g, "fx", "a.rs", (0, 2));
        let b = PathBuf::from("b.rs");
        let result = resolve_files_with_graph(&[(&b, "let x = fx;")], "rust", &g);
        assert!(result.refs.is_empty());
    }
}
