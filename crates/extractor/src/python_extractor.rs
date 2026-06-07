use std::path::Path;

use coregraph_core::edge::AnalysisOrigin;
use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
use coregraph_graph::SymbolGraph;
use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::{
    tree_sitter_extract_refs, DocCommentRef, ExtractError, RawReference, ReferenceKind,
    SymbolExtractor,
};

// Query patterns are ordered: class(0), function(1)
const PY_QUERY: &str = include_str!("queries/python.scm");
const PY_REFS_QUERY: &str = include_str!("queries/python-refs.scm");

/// The docstring span for a Python `def` / `class`: the first statement in its
/// body when that statement is a bare string literal (PEP 257). `None`
/// otherwise. This is the language's own doc rule (a structural first-child
/// string), so the caller emits a `SyntaxMatched` (0.85) edge.
fn python_docstring_span(def_node: Node) -> Option<(u32, u32)> {
    let body = def_node.child_by_field_name("body")?;
    let first = body.named_child(0)?;
    // A bare string statement parses as `expression_statement(string)`.
    let string_node = if first.kind() == "expression_statement" {
        first.named_child(0)?
    } else {
        first
    };
    if string_node.kind() == "string" {
        Some((
            string_node.start_byte() as u32,
            string_node.end_byte() as u32,
        ))
    } else {
        None
    }
}

pub struct PythonExtractor;

impl PythonExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PythonExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor for PythonExtractor {
    fn language_name(&self) -> &'static str {
        "python"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["py"]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| ExtractError::QueryError {
                query_name: "python",
                message: e.to_string(),
            })?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| ExtractError::ParseFailed {
                path: path.to_path_buf(),
            })?;

        let query = Query::new(&language, PY_QUERY).map_err(|e| ExtractError::QueryError {
            query_name: "python",
            message: e.to_string(),
        })?;

        let name_capture_idx =
            query
                .capture_index_for_name("name")
                .ok_or_else(|| ExtractError::QueryError {
                    query_name: "python",
                    message: "capture @name not found".to_string(),
                })?;
        let def_capture_idx = query.capture_index_for_name("def");

        let mut cursor = tree_sitter::QueryCursor::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            let kind = match m.pattern_index {
                0 => SymbolKind::Class,
                1 => SymbolKind::Function,
                2 | 3 => SymbolKind::Constant, // module / class-level assignment
                _ => continue,
            };

            let def_span = def_capture_idx.and_then(|idx| {
                m.captures
                    .iter()
                    .find(|c| c.index == idx)
                    .map(|c| (c.node.start_byte() as u32, c.node.end_byte() as u32))
            });

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
                    );
                    graph.insert_node(symbol);
                }
            }
        }

        Ok(())
    }

    fn extract_references(&self, _path: &Path, source: &str) -> Vec<RawReference> {
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        tree_sitter_extract_refs(&lang, PY_REFS_QUERY, source, |idx| match idx {
            0..=1 => Some(ReferenceKind::Call),
            2..=4 => Some(ReferenceKind::Import),
            5 => Some(ReferenceKind::Extends),
            6 => Some(ReferenceKind::TypeUse),
            _ => None,
        })
    }

    fn extract_doc_comments(&self, _path: &Path, source: &str) -> Vec<DocCommentRef> {
        // Python docstrings are the first statement *inside* the body (PEP 257),
        // not a preceding comment — so this walks each def/class's body rather
        // than using the shared preceding-sibling helper.
        let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        let mut parser = Parser::new();
        if parser.set_language(&language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(source, None) else {
            return Vec::new();
        };
        let Ok(query) = Query::new(&language, PY_QUERY) else {
            return Vec::new();
        };
        let Some(def_idx) = query.capture_index_for_name("def") else {
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        let src = source.as_bytes();
        let mut seen: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        use streaming_iterator::StreamingIterator;
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
            if let Some(doc_span) = python_docstring_span(def_node) {
                out.push(DocCommentRef {
                    def_span,
                    doc_span,
                    origin: AnalysisOrigin::SyntaxMatched,
                });
            }
        }
        out
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
    fn extracts_class() {
        let extractor = PythonExtractor::new();
        let path = fixture_path("python-simple", "example.py");
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract failed");

        let found = graph
            .nodes()
            .any(|n| n.name == "DataProcessor" && n.kind == SymbolKind::Class);
        assert!(found, "DataProcessor class not found");
    }

    #[test]
    fn extracts_top_level_function() {
        let extractor = PythonExtractor::new();
        let path = fixture_path("python-simple", "example.py");
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract failed");

        let found = graph
            .nodes()
            .any(|n| n.name == "create_processor" && n.kind == SymbolKind::Function);
        assert!(found, "create_processor not found");
    }

    #[test]
    fn python_extensions() {
        let extractor = PythonExtractor::new();
        let exts = extractor.file_extensions();
        assert!(exts.contains(&"py"), "py extension missing");
    }

    #[test]
    fn python_docstring_is_paired_with_function() {
        let src = "def greet(name):\n    \"\"\"Greets the caller.\"\"\"\n    return name\n";
        let refs = PythonExtractor::new().extract_doc_comments(std::path::Path::new("g.py"), src);
        assert_eq!(refs.len(), 1, "the docstring must pair with greet");
        let text = &src[refs[0].doc_span.0 as usize..refs[0].doc_span.1 as usize];
        assert!(
            text.contains("Greets the caller"),
            "doc_span must cover the docstring, got {text:?}"
        );
        assert_eq!(refs[0].origin, AnalysisOrigin::SyntaxMatched);
    }

    #[test]
    fn python_function_without_docstring_yields_no_ref() {
        let src = "def greet(name):\n    return name\n";
        let refs = PythonExtractor::new().extract_doc_comments(std::path::Path::new("g.py"), src);
        assert!(refs.is_empty(), "no docstring → no doc ref");
    }

    #[test]
    fn python_class_docstring_is_paired() {
        let src = "class Greeter:\n    \"\"\"A greeter.\"\"\"\n    pass\n";
        let refs = PythonExtractor::new().extract_doc_comments(std::path::Path::new("g.py"), src);
        assert_eq!(refs.len(), 1, "the class docstring must pair with Greeter");
    }

    #[test]
    fn extracts_module_and_class_level_assignments() {
        // Module-level and class-level `name = value` bindings are public API
        // (e.g. `requests.codes`) and must be symbols. Function-local
        // assignments stay out to avoid noise.
        let src = "codes = build_codes()\nMAX_RETRIES = 3\n\
                   class C:\n    attr = 1\n    def m(self):\n        local = 5\n        return local\n";
        let mut g = SymbolGraph::new();
        PythonExtractor::new()
            .extract(std::path::Path::new("m.py"), src, &mut g)
            .unwrap();
        let has = |name: &str| {
            g.nodes()
                .any(|n| n.name == name && n.kind == SymbolKind::Constant)
        };
        assert!(
            has("codes"),
            "module-level assignment `codes` not extracted"
        );
        assert!(has("MAX_RETRIES"), "module-level constant not extracted");
        assert!(has("attr"), "class-level attribute not extracted");
        assert!(
            !g.nodes().any(|n| n.name == "local"),
            "function-local assignment must NOT be extracted"
        );
    }

    #[test]
    fn python_type_hint_refs_captured() {
        // A class used only as a parameter / return type hint produces a TypeUse
        // ref. Also asserts a call ref survives (guards the refs query compiles).
        let src = "def build(c: Widget) -> Result:\n    helper()\n    return c\n";
        let refs = PythonExtractor::new().extract_references(std::path::Path::new("a.py"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "helper" && r.kind == ReferenceKind::Call),
            "call ref lost (refs query broken?): {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Widget" && r.kind == ReferenceKind::TypeUse),
            "Widget type-hint ref not captured: {refs:?}"
        );
    }
}
