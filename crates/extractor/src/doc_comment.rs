//! Shared helpers for the documentation layer (docs/graph-model.md §7).
//!
//! A `DocComment` node represents a documentation comment attached to a code
//! symbol; a `Documents` edge (DocComment → Symbol) records the attachment.
//! Per-language extractors locate the doc comment (using their own grammar's
//! doc-comment syntax) and call [`insert_documentation`], which owns the node /
//! edge construction so confidence and naming stay consistent across languages.

use std::path::Path;

use coregraph_core::edge::AnalysisOrigin;
use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode};
use coregraph_graph::{EdgeEvaluator, SymbolGraph};
use regex::Regex;
use tree_sitter::Node;

/// Scans doc-comment text for intra-doc link targets that name a code symbol.
/// Deliberately restricted to UNAMBIGUOUS link markers so prose and ordinary
/// markdown reference links are never mistaken for code references:
///   - backticked markdown links `` [`Name`] `` / `` [`mod::Name`] `` (rustdoc)
///   - JSDoc / Javadoc `{@link Name}` / `{@linkcode Name}` / `{@link Foo#bar}`
///
/// Each target is reduced to its FINAL identifier (`Foo#bar` → `bar`,
/// `mod::Name` → `Name`). Bare `[name]` is intentionally NOT matched.
pub struct MentionLinkScanner {
    backtick: Regex,
    jsdoc: Regex,
}

impl MentionLinkScanner {
    pub fn new() -> Self {
        Self {
            // [`Name`] / [`mod::Name`] — backticked text inside a markdown link.
            backtick: Regex::new(r"\[`([^`]+)`\]").unwrap(),
            // {@link X} / {@linkcode X} / {@linkplain X}; X runs to `}` or `|`.
            jsdoc: Regex::new(r"\{@link(?:code|plain)?\s+([^}|]+)").unwrap(),
        }
    }

    /// Link target identifiers found in `text`, in source order (may repeat).
    pub fn targets(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for cap in self.backtick.captures_iter(text) {
            if let Some(id) = final_identifier(&cap[1]) {
                out.push(id);
            }
        }
        for cap in self.jsdoc.captures_iter(text) {
            if let Some(id) = final_identifier(&cap[1]) {
                out.push(id);
            }
        }
        out
    }
}

impl Default for MentionLinkScanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Reduce a link spec to the final identifier it names, or `None` if it is not
/// a plain identifier path. `Foo#bar` → `bar`, `mod::Name` → `Name`,
/// `{@link Name label}` → `Name`, `Foo.bar()` → `bar`.
fn final_identifier(spec: &str) -> Option<String> {
    // Drop a `|display` suffix, then take the first whitespace token (the target).
    let target = spec.split('|').next().unwrap_or(spec).trim();
    let target = target.split_whitespace().next()?;
    let target = target.trim_end_matches("()");
    // Final segment after a path / member separator.
    let token = target.rsplit(['.', '#', ':']).next()?.trim();
    let mut chars = token.chars();
    let first_ok = chars
        .next()
        .map(|c| c.is_alphabetic() || c == '_')
        .unwrap_or(false);
    if first_ok && token.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Some(token.to_string())
    } else {
        None
    }
}

/// Find the doc-comment block immediately preceding `item` for a C-family
/// grammar (one where the doc comment is a *previous sibling* of the
/// definition). Walks previous siblings: nodes whose kind is in `skip_kinds`
/// (attributes / decorators that sit between the doc and the item) are stepped
/// over; nodes whose kind is in `comment_kinds` are accepted as doc comments
/// only when `is_doc` returns true for their source text. Contiguous doc
/// comments are merged. Returns the merged `(start, end)` byte span, or `None`.
///
/// This captures the language's own attachment rule (a dedicated doc marker
/// directly preceding the definition), not a nearest-comment heuristic — which
/// is why callers emit a `SyntaxMatched` edge. Languages whose doc convention is
/// an undistinguished `//` (e.g. Go) must NOT use this with a SyntaxMatched
/// origin.
///
/// `wrapper_kinds` names node kinds that *wrap* a definition without being the
/// doc's sibling — e.g. TS/JS `export_statement` (`export function f(){}`
/// parses the doc as a sibling of the `export_statement`, not of the inner
/// `function_declaration`). The search anchors on the outermost enclosing
/// wrapper before inspecting siblings.
pub fn preceding_doc_span(
    item: Node,
    source: &[u8],
    comment_kinds: &[&str],
    skip_kinds: &[&str],
    wrapper_kinds: &[&str],
    is_doc: &dyn Fn(&str) -> bool,
) -> Option<(u32, u32)> {
    // Climb through wrappers (`export …`) so we inspect the right siblings.
    let mut anchor = item;
    while let Some(parent) = anchor.parent() {
        if wrapper_kinds.contains(&parent.kind()) {
            anchor = parent;
        } else {
            break;
        }
    }

    let mut doc_start: Option<u32> = None;
    let mut doc_end: Option<u32> = None;
    // Start of the node directly *below* the one under inspection. A doc comment
    // must be contiguous with it (no blank line between), else it is not
    // attached — a blank line ends doc attachment in every language here.
    let mut next_start = anchor.start_byte();
    let mut cur = anchor.prev_sibling();
    while let Some(n) = cur {
        let kind = n.kind();
        if skip_kinds.contains(&kind) {
            next_start = n.start_byte();
            cur = n.prev_sibling();
            continue;
        }
        if comment_kinds.contains(&kind) {
            let text = n.utf8_text(source).unwrap_or("");
            if !is_doc(text) {
                break;
            }
            // Blank line between this comment and the node below it → detached.
            let gap = std::str::from_utf8(&source[n.end_byte()..next_start]).unwrap_or("");
            if gap.matches('\n').count() > 1 {
                break;
            }
            // Walking backwards: the first comment seen is closest to the item
            // (largest end); earlier lines only shrink `start`.
            doc_start = Some(n.start_byte() as u32);
            doc_end = Some(doc_end.unwrap_or(n.end_byte() as u32));
            next_start = n.start_byte();
            cur = n.prev_sibling();
            continue;
        }
        break;
    }
    Some((doc_start?, doc_end?))
}

/// True for a JSDoc / Javadoc block comment (`/** … */`). The empty `/**/` is
/// excluded. Shared by the C-family extractors with `/** */` doc syntax.
pub fn is_block_doc_comment(text: &str) -> bool {
    text.starts_with("/**") && text != "/**/"
}

/// Extract `(def_span, doc_span)` pairs for a C-family grammar where the symbol
/// query exposes a `@def` capture per definition and doc comments are previous
/// siblings. Parses `source`, runs `query_src`, and for each unique `@def` span
/// finds the preceding doc comment via [`preceding_doc_span`]. The per-language
/// knobs are the comment node kinds, the kinds to skip (attributes/decorators),
/// and the doc-marker predicate. Returns empty on a parse / query error.
// Each parameter is a distinct per-language knob (grammar, query, source, three
// node-kind lists, origin, doc predicate); bundling them into a struct would add
// indirection without grouping anything cohesive.
#[allow(clippy::too_many_arguments)]
pub fn extract_block_doc_comments(
    language: &tree_sitter::Language,
    query_src: &str,
    source: &str,
    comment_kinds: &[&str],
    skip_kinds: &[&str],
    wrapper_kinds: &[&str],
    origin: AnalysisOrigin,
    is_doc: &dyn Fn(&str) -> bool,
) -> Vec<crate::DocCommentRef> {
    use streaming_iterator::StreamingIterator;
    use tree_sitter::{Parser, Query, QueryCursor};

    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(language, query_src) else {
        return Vec::new();
    };
    let Some(def_idx) = query.capture_index_for_name("def") else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();
    let src = source.as_bytes();
    let mut seen: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src);
    while let Some(m) = matches.next() {
        let Some(def_cap) = m.captures.iter().find(|c| c.index == def_idx) else {
            continue;
        };
        let def_node = def_cap.node;
        let def_span = (def_node.start_byte() as u32, def_node.end_byte() as u32);
        if !seen.insert(def_span) {
            continue;
        }
        if let Some(doc_span) = preceding_doc_span(
            def_node,
            src,
            comment_kinds,
            skip_kinds,
            wrapper_kinds,
            is_doc,
        ) {
            out.push(crate::DocCommentRef {
                def_span,
                doc_span,
                origin,
            });
        }
    }
    out
}

/// Marker prefix in a `DocComment` node's `name`, mirroring the
/// `api_path::` / `config_ref::` convention. Lets a caller find "the doc for X"
/// by looking up `doc::X` in the name index without widening the node schema.
pub const DOC_COMMENT_PREFIX: &str = "doc::";

/// Build the `name` for the `DocComment` node that documents `symbol_name`.
pub fn doc_node_name(symbol_name: &str) -> String {
    format!("{DOC_COMMENT_PREFIX}{symbol_name}")
}

/// Insert a `DocComment` node for the doc block at `doc_span` and a `Documents`
/// edge from it to the symbol it documents.
///
/// `documented_symbol` is the already-inserted symbol's id; `documented_name`
/// is that symbol's local name (used to label the doc node). `origin` records
/// how the attachment was established (`SyntaxMatched` for a dedicated doc
/// marker, `PatternMatched` for a marker-less convention) and drives the edge
/// confidence.
pub fn insert_documentation(
    graph: &mut SymbolGraph,
    path: &Path,
    documented_symbol: SymbolId,
    documented_name: &str,
    doc_span: (u32, u32),
    origin: AnalysisOrigin,
) {
    let doc_node = SymbolNode::new(
        SymbolId(0),
        SymbolKind::DocComment,
        doc_node_name(documented_name),
        path,
        doc_span.0,
        doc_span.1,
    );
    let doc_id = graph.insert_node(doc_node);

    let confidence = EdgeEvaluator::evaluate(EdgeKind::Documents, origin);
    let edge = DirectEdge::new(
        doc_id,
        documented_symbol,
        EdgeKind::Documents,
        origin,
        confidence,
        path,
    );
    graph.insert_edge(edge);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(text: &str) -> Vec<String> {
        MentionLinkScanner::new().targets(text)
    }

    #[test]
    fn backticked_link_is_a_mention() {
        assert_eq!(targets("See [`Bar`] for details."), vec!["Bar"]);
    }

    #[test]
    fn jsdoc_link_is_a_mention() {
        assert_eq!(targets("Uses {@link Bar} internally."), vec!["Bar"]);
        assert_eq!(targets("Uses {@linkcode Bar}."), vec!["Bar"]);
    }

    #[test]
    fn qualified_targets_reduce_to_final_identifier() {
        assert_eq!(targets("[`mod::Widget`]"), vec!["Widget"]);
        assert_eq!(targets("{@link Foo#run}"), vec!["run"]);
        assert_eq!(targets("{@link pkg.Foo}"), vec!["Foo"]);
    }

    #[test]
    fn jsdoc_link_with_label_takes_the_target() {
        assert_eq!(targets("{@link Foo the foo}"), vec!["Foo"]);
        assert_eq!(targets("{@link Foo|the foo}"), vec!["Foo"]);
    }

    #[test]
    fn bare_brackets_and_prose_are_not_mentions() {
        // Ambiguous forms must NOT be treated as code references.
        assert!(targets("a markdown [link](http://x) here").is_empty());
        assert!(targets("see [below] and [the docs]").is_empty());
        assert!(targets("[plain text]").is_empty());
    }
}
