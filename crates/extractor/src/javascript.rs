use std::path::Path;

use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
use coregraph_graph::SymbolGraph;
use tree_sitter::Query;

use crate::doc_comment::{extract_block_doc_comments, is_block_doc_comment};
use crate::{
    tree_sitter_extract_refs, DocCommentRef, ExtractError, RawReference, ReferenceKind,
    SymbolExtractor,
};

// Query patterns are ordered: function(0), class(1), method(2), arrow_function(3)
const JS_QUERY: &str = include_str!("queries/javascript.scm");
const JS_REFS_QUERY: &str = include_str!("queries/javascript-refs.scm");

pub struct JavaScriptExtractor;

impl JavaScriptExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JavaScriptExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor for JavaScriptExtractor {
    fn language_name(&self) -> &'static str {
        "javascript"
    }

    fn file_extensions(&self) -> &[&'static str] {
        // The tree-sitter JavaScript grammar parses JSX natively, so .jsx needs
        // no separate dialect — only routing here so the files get indexed.
        &["js", "mjs", "cjs", "jsx"]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| ExtractError::QueryError {
                query_name: "javascript",
                message: e.to_string(),
            })?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| ExtractError::ParseFailed {
                path: path.to_path_buf(),
            })?;

        let query = Query::new(&language, JS_QUERY).map_err(|e| ExtractError::QueryError {
            query_name: "javascript",
            message: e.to_string(),
        })?;

        let name_capture_idx =
            query
                .capture_index_for_name("name")
                .ok_or_else(|| ExtractError::QueryError {
                    query_name: "javascript",
                    message: "capture @name not found".to_string(),
                })?;
        let def_capture_idx = query.capture_index_for_name("def");
        let fnbody_capture_idx = query.capture_index_for_name("fnbody");

        let mut cursor = tree_sitter::QueryCursor::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            let kind = match m.pattern_index {
                0 => SymbolKind::Function,
                1 => SymbolKind::Class,
                2 => SymbolKind::Method,
                3 => SymbolKind::Function, // arrow / function-expression const
                _ => continue,
            };

            // Pattern 3 matches every value-bearing declarator (value captured
            // broadly as @fnbody); keep it only when the value really resolves
            // to a function (unwrapping any parentheses), so `const n = 5` and
            // aliases stay out of the symbol set.
            if m.pattern_index == 3 {
                let is_fn = fnbody_capture_idx
                    .and_then(|idx| m.captures.iter().find(|c| c.index == idx).map(|c| c.node))
                    .map(crate::unwraps_to_fn)
                    .unwrap_or(false);
                if !is_fn {
                    continue;
                }
            }

            let def_node = def_capture_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx).map(|c| c.node));
            let def_span = def_node.map(|n| (n.start_byte() as u32, n.end_byte() as u32));
            // JS has no access keywords; the only declared-visibility signal is
            // a top-level `export`. Non-exported and `#private` members fall back
            // to the name-shape heuristic (Unknown).
            let visibility = match def_node.and_then(|n| n.parent()).map(|p| p.kind()) {
                Some("export_statement") => Visibility::Public,
                _ => Visibility::Unknown,
            };

            for cap in m.captures {
                if cap.index == name_capture_idx {
                    let node = cap.node;
                    let name = &source[node.start_byte()..node.end_byte()];
                    let (start, end) =
                        def_span.unwrap_or((node.start_byte() as u32, node.end_byte() as u32));
                    let symbol = SymbolNode::new(
                        SymbolId(0), // will be overwritten by insert_node
                        kind.clone(),
                        name,
                        path,
                        start,
                        end,
                    )
                    .with_visibility(visibility);
                    graph.insert_node(symbol);
                }
            }
        }

        Ok(())
    }

    fn extract_references(&self, _path: &Path, source: &str) -> Vec<RawReference> {
        let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
        tree_sitter_extract_refs(&lang, JS_REFS_QUERY, source, |idx| match idx {
            0..=2 => Some(ReferenceKind::Call),
            3..=4 => Some(ReferenceKind::Import),
            5 => Some(ReferenceKind::Extends),
            _ => None,
        })
    }

    fn extract_doc_comments(&self, _path: &Path, source: &str) -> Vec<DocCommentRef> {
        // JSDoc `/** … */` is a previous sibling of the declaration; decorators
        // sit between the doc and the item.
        let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
        extract_block_doc_comments(
            &language,
            JS_QUERY,
            source,
            &["comment"],
            &["decorator"],
            &["export_statement"],
            coregraph_core::edge::AnalysisOrigin::SyntaxMatched,
            &is_block_doc_comment,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_graph::SymbolGraph;

    fn fixture_path(subdir: &str, name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures")
            .join(subdir)
            .join(name)
    }

    #[test]
    fn extracts_function() {
        let extractor = JavaScriptExtractor::new();
        let path = fixture_path("ts-simple", "helper.js");
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract failed");

        let found = graph
            .nodes()
            .any(|n| n.name == "add" && n.kind == SymbolKind::Function);
        assert!(found, "add function not found");
    }

    #[test]
    fn extracts_arrow_and_function_expression_consts() {
        // Both `const x = () => {}` and `const y = function () {}` are function
        // definitions and must be Function symbols, otherwise calls to them in
        // idiomatic JS never resolve.
        let src = "export const handler = (e) => e;\nconst legacy = function () { return 1; };\n";
        let mut g = SymbolGraph::new();
        JavaScriptExtractor::new()
            .extract(std::path::Path::new("a.js"), src, &mut g)
            .unwrap();
        assert!(
            g.nodes()
                .any(|n| n.name == "handler" && n.kind == SymbolKind::Function),
            "arrow-const handler not extracted as Function"
        );
        assert!(
            g.nodes()
                .any(|n| n.name == "legacy" && n.kind == SymbolKind::Function),
            "function-expression const legacy not extracted as Function"
        );
    }

    #[test]
    fn extracts_parenthesized_arrow_const() {
        // `const x = (() => {})` — a parenthesized arrow. Valid JS; the
        // declarator value is a parenthesized_expression wrapping the arrow, so
        // it must still register as a Function (parity with the TS extractor).
        // `const n = 5` must NOT become a Function.
        let src = "export const wrapped = (() => 1);\nconst n = 5;\n";
        let mut g = SymbolGraph::new();
        JavaScriptExtractor::new()
            .extract(std::path::Path::new("a.js"), src, &mut g)
            .unwrap();
        assert!(
            g.nodes()
                .any(|n| n.name == "wrapped" && n.kind == SymbolKind::Function),
            "parenthesized arrow const not extracted as Function"
        );
        assert!(
            !g.nodes().any(|n| n.name == "n"),
            "non-function const `n = 5` wrongly extracted"
        );
    }

    #[test]
    fn extracts_class() {
        let extractor = JavaScriptExtractor::new();
        let path = fixture_path("ts-simple", "helper.js");
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract failed");

        let found = graph
            .nodes()
            .any(|n| n.name == "Calculator" && n.kind == SymbolKind::Class);
        assert!(found, "Calculator class not found");
    }

    #[test]
    fn javascript_visibility_from_export() {
        // Exported declarations are the module's public API; everything else
        // stays Unknown so the name-shape heuristic decides (JS has no access
        // keywords; `#private` members are caught by name shape).
        let src = "export class Svc {}\nfunction helper() {}\n";
        let mut g = SymbolGraph::new();
        JavaScriptExtractor::new()
            .extract(std::path::Path::new("a.js"), src, &mut g)
            .unwrap();
        let vis = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.visibility);
        assert_eq!(
            vis("Svc"),
            Some(Visibility::Public),
            "exported class is public"
        );
        assert_eq!(
            vis("helper"),
            Some(Visibility::Unknown),
            "non-exported stays unknown"
        );
    }

    #[test]
    fn javascript_extensions() {
        let extractor = JavaScriptExtractor::new();
        let exts = extractor.file_extensions();
        assert!(exts.contains(&"js"), "js extension missing");
    }

    #[test]
    fn jsx_is_a_javascript_extension() {
        // .jsx files must route to this extractor (the JS grammar already
        // parses JSX natively); otherwise React component files are unindexed.
        assert!(
            JavaScriptExtractor::new()
                .file_extensions()
                .contains(&"jsx"),
            "jsx extension missing"
        );
    }

    #[test]
    fn extracts_component_from_jsx() {
        let src = "export function Button() {\n  return <button onClick={handle}>{text()}</button>;\n}\nexport function text() { return \"x\"; }\n";
        let mut graph = SymbolGraph::new();
        JavaScriptExtractor::new()
            .extract(std::path::Path::new("Button.jsx"), src, &mut graph)
            .expect("extract");
        let names: Vec<String> = graph.nodes().map(|n| n.name.clone()).collect();
        assert!(
            names.iter().any(|n| n == "Button"),
            "Button not found: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "text"),
            "text not found: {names:?}"
        );
    }

    #[test]
    fn jsdoc_is_paired_with_function() {
        let src = "/** Greets. */\nfunction greet() {}\n";
        let refs =
            JavaScriptExtractor::new().extract_doc_comments(std::path::Path::new("g.js"), src);
        assert_eq!(refs.len(), 1, "JSDoc must pair with the function");
    }
}
