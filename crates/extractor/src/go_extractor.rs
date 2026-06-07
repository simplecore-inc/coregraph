use std::path::Path;

use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
use coregraph_graph::SymbolGraph;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::doc_comment::extract_block_doc_comments;
use crate::{
    tree_sitter_extract_refs, DocCommentRef, ExtractError, RawReference, ReferenceKind,
    SymbolExtractor,
};

const GO_QUERY: &str = include_str!("queries/go.scm");
const GO_REFS_QUERY: &str = include_str!("queries/go-refs.scm");

pub struct GoExtractor {
    language: Language,
}

impl GoExtractor {
    pub fn new() -> Self {
        let language: Language = tree_sitter_go::LANGUAGE.into();
        Self { language }
    }
}

impl Default for GoExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Go declared visibility follows the language rule (not a heuristic): an
/// identifier is exported (public) iff its first character is an upper-case
/// Unicode letter; otherwise it is package-private. This is exact, so it
/// supersedes the generic name-shape fallback, which would wrongly treat a
/// lower-case Go identifier as public.
fn go_visibility(name: &str) -> Visibility {
    match name.chars().next() {
        Some(c) if c.is_uppercase() => Visibility::Public,
        _ => Visibility::Private,
    }
}

/// Go predeclared (builtin) identifiers — types, constants and builtin
/// functions from the universe block. They are never project symbols, so a
/// reference to one must not be resolved by bare name onto a same-named user
/// declaration (which produces clusters of false edges).
fn is_go_predeclared(name: &str) -> bool {
    matches!(
        name,
        // types
        "any" | "bool" | "byte" | "comparable" | "complex64" | "complex128"
            | "error" | "float32" | "float64" | "int" | "int8" | "int16"
            | "int32" | "int64" | "rune" | "string" | "uint" | "uint8"
            | "uint16" | "uint32" | "uint64" | "uintptr"
            // constants / zero value
            | "true" | "false" | "iota" | "nil"
            // builtin functions
            | "append" | "cap" | "clear" | "close" | "complex" | "copy"
            | "delete" | "imag" | "len" | "make" | "max" | "min" | "new"
            | "panic" | "print" | "println" | "real" | "recover"
    )
}

impl SymbolExtractor for GoExtractor {
    fn language_name(&self) -> &'static str {
        "Go"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["go"]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        let mut parser = Parser::new();
        parser
            .set_language(&self.language)
            .map_err(|e| ExtractError::QueryError {
                query_name: "go",
                message: e.to_string(),
            })?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| ExtractError::ParseFailed {
                path: path.to_path_buf(),
            })?;

        let query = Query::new(&self.language, GO_QUERY).map_err(|e| ExtractError::QueryError {
            query_name: "go.scm",
            message: e.to_string(),
        })?;

        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();
        let name_idx = query.capture_index_for_name("name");
        let def_idx = query.capture_index_for_name("def");
        let typekind_idx = query.capture_index_for_name("go_typekind");

        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
        while let Some(m) = matches.next() {
            let name_cap = name_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                .or_else(|| m.captures.first());
            let Some(name_cap) = name_cap else { continue };

            let kind = match m.pattern_index {
                0 => SymbolKind::Function,
                1 => SymbolKind::Method,
                2 => SymbolKind::Struct,
                3 => SymbolKind::Interface,
                4 => SymbolKind::Constant, // const_spec
                5 => SymbolKind::Variable, // var_spec
                // Catch-all named type: skip struct/interface (already patterns
                // 2/3) so they are not double-emitted; everything else (func,
                // slice, map, named type, `type X = Y`) is a TypeAlias.
                6 => {
                    let is_struct_or_iface = typekind_idx
                        .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                        .map(|c| matches!(c.node.kind(), "struct_type" | "interface_type"))
                        .unwrap_or(false);
                    if is_struct_or_iface {
                        continue;
                    }
                    SymbolKind::TypeAlias
                }
                7 => SymbolKind::TypeAlias, // `type X = Y`
                _ => continue,
            };

            let name = match name_cap.node.utf8_text(source_bytes) {
                Ok(n) => n.to_string(),
                Err(_) => continue,
            };

            // Prefer @def (full declaration) span; fall back to identifier.
            let (span_start, span_end) = def_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                .map(|c| (c.node.start_byte() as u32, c.node.end_byte() as u32))
                .unwrap_or((
                    name_cap.node.start_byte() as u32,
                    name_cap.node.end_byte() as u32,
                ));

            let visibility = go_visibility(&name);
            let node = SymbolNode::new(SymbolId(0), kind, name, path, span_start, span_end)
                .with_visibility(visibility);
            graph.insert_node(node);
        }

        Ok(())
    }

    fn extract_references(&self, _path: &Path, source: &str) -> Vec<RawReference> {
        let mut refs =
            tree_sitter_extract_refs(&self.language, GO_REFS_QUERY, source, |idx| match idx {
                0..=1 => Some(ReferenceKind::Call),
                2 => Some(ReferenceKind::Import),
                3 => Some(ReferenceKind::TypeUse),
                _ => None,
            });
        // Drop predeclared identifiers: a builtin type/func name (`string`,
        // `error`, `len`, …) resolved by bare name onto a same-named user
        // symbol produces large clusters of false edges (a method `string()`
        // gaining a reference from every `string`-typed declaration). Builtins
        // are not project symbols, so these references can never be real.
        refs.retain(|r| !is_go_predeclared(&r.name));
        refs
    }

    fn extract_doc_comments(&self, _path: &Path, source: &str) -> Vec<DocCommentRef> {
        // Go has no dedicated doc marker: the convention (godoc) treats the
        // contiguous comment block directly above a declaration as its doc.
        // That is a convention, not a distinct syntax, so the attachment is
        // PatternMatched (0.60), and ANY immediately-preceding comment qualifies
        // (the contiguity / blank-line rule in `preceding_doc_span` guards it).
        extract_block_doc_comments(
            &self.language,
            GO_QUERY,
            source,
            &["comment"],
            &[],
            &[],
            coregraph_core::edge::AnalysisOrigin::PatternMatched,
            &|_| true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn extract_from_fixture(subdir: &str, filename: &str) -> SymbolGraph {
        let extractor = GoExtractor::new();
        let path = fixture_path(subdir, filename);
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extraction failed");
        graph
    }

    #[test]
    fn extracts_struct() {
        let graph = extract_from_fixture("go-simple", "main.go");
        let found = graph
            .nodes()
            .any(|n| n.name == "User" && n.kind == SymbolKind::Struct);
        assert!(found, "Expected to find User struct");
    }

    #[test]
    fn extracts_interface_go() {
        let graph = extract_from_fixture("go-simple", "main.go");
        let found = graph
            .nodes()
            .any(|n| n.name == "UserRepository" && n.kind == SymbolKind::Interface);
        assert!(found, "Expected to find UserRepository interface");
    }

    #[test]
    fn extracts_function_go() {
        let graph = extract_from_fixture("go-simple", "main.go");
        let found = graph
            .nodes()
            .any(|n| n.name == "NewUser" && n.kind == SymbolKind::Function);
        assert!(found, "Expected to find NewUser function");
    }

    #[test]
    fn go_visibility_follows_export_capitalization() {
        // Go's language rule: an upper-case initial means exported (public);
        // anything else is package-private. Covers funcs, types and methods.
        let extractor = GoExtractor::new();
        let src = "package p\n\nfunc Exported() {}\nfunc internal() {}\ntype Public struct {}\ntype private struct {}\nfunc (s *Public) Handle() {}\nfunc (s *Public) helper() {}\n";
        let mut g = SymbolGraph::new();
        extractor
            .extract(std::path::Path::new("p.go"), src, &mut g)
            .unwrap();
        let vis = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.visibility);
        assert_eq!(
            vis("Exported"),
            Some(Visibility::Public),
            "upper-case func is exported"
        );
        assert_eq!(
            vis("internal"),
            Some(Visibility::Private),
            "lower-case func is package-private"
        );
        assert_eq!(
            vis("Public"),
            Some(Visibility::Public),
            "upper-case type is exported"
        );
        assert_eq!(
            vis("private"),
            Some(Visibility::Private),
            "lower-case type is package-private"
        );
        assert_eq!(
            vis("Handle"),
            Some(Visibility::Public),
            "upper-case method is exported"
        );
        assert_eq!(
            vis("helper"),
            Some(Visibility::Private),
            "lower-case method is package-private"
        );
    }

    #[test]
    fn go_extensions() {
        let extractor = GoExtractor::new();
        assert!(extractor.file_extensions().contains(&"go"));
    }

    #[test]
    fn go_predeclared_identifiers_are_not_references() {
        // Builtin type/func names must NOT become references, or every
        // `string`-typed decl name-merges onto a user method named `string()`
        // (cobra: in-degree 306 of false edges). `error`, `len`, etc. likewise.
        let src = "package p\n\
                   func (x T) string() string { var s string = \"\"; return s }\n\
                   func use(a string) error { _ = len(a); return nil }\n";
        let refs = GoExtractor::new().extract_references(std::path::Path::new("p.go"), src);
        for builtin in ["string", "error", "len"] {
            assert!(
                !refs.iter().any(|r| r.name == builtin),
                "builtin `{builtin}` must be filtered from references: {refs:?}"
            );
        }
        // A genuine user call still survives.
        let src2 = "package p\nfunc caller() { helper() }\n";
        let refs2 = GoExtractor::new().extract_references(std::path::Path::new("p.go"), src2);
        assert!(
            refs2.iter().any(|r| r.name == "helper"),
            "user call lost: {refs2:?}"
        );
    }

    #[test]
    fn extracts_go_const_var_and_type_alias() {
        // Package-level const/var declarations (incl. grouped blocks) and
        // non-struct/non-interface type definitions/aliases must be symbols;
        // struct/interface types keep their existing dedicated kinds.
        let src = "package p\n\
                   const MaxLen = 100\n\
                   const (\n\tA = 1\n\tB = 2\n)\n\
                   var Logger = newLogger()\n\
                   type Handler func(int) error\n\
                   type Celsius = float64\n\
                   type Point struct{ X int }\n\
                   type Reader interface{ Read() }\n";
        let mut g = SymbolGraph::new();
        GoExtractor::new()
            .extract(std::path::Path::new("p.go"), src, &mut g)
            .unwrap();
        let kind_of = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.kind.clone());
        assert_eq!(
            kind_of("MaxLen"),
            Some(SymbolKind::Constant),
            "top-level const"
        );
        assert_eq!(kind_of("A"), Some(SymbolKind::Constant), "grouped const A");
        assert_eq!(kind_of("B"), Some(SymbolKind::Constant), "grouped const B");
        assert_eq!(kind_of("Logger"), Some(SymbolKind::Variable), "package var");
        assert_eq!(
            kind_of("Handler"),
            Some(SymbolKind::TypeAlias),
            "func type def"
        );
        assert_eq!(
            kind_of("Celsius"),
            Some(SymbolKind::TypeAlias),
            "type alias"
        );
        assert_eq!(
            kind_of("Point"),
            Some(SymbolKind::Struct),
            "struct kept as Struct"
        );
        assert_eq!(
            kind_of("Reader"),
            Some(SymbolKind::Interface),
            "interface kept"
        );
    }

    #[test]
    fn go_doc_comment_is_paired_at_convention_confidence() {
        // Go's contiguous `//` block above a declaration is its doc (godoc).
        let src = "package p\n\n// Greet greets.\nfunc Greet() {}\n";
        let refs = GoExtractor::new().extract_doc_comments(std::path::Path::new("p.go"), src);
        assert_eq!(
            refs.len(),
            1,
            "the // block above func must pair as its doc"
        );
        assert_eq!(
            refs[0].origin,
            coregraph_core::edge::AnalysisOrigin::PatternMatched,
            "Go doc is a convention (no marker) → PatternMatched 0.60"
        );
    }

    #[test]
    fn go_comment_with_blank_line_is_not_a_doc() {
        // A blank line between the comment and the declaration detaches it.
        let src = "package p\n\n// unrelated note\n\nfunc Greet() {}\n";
        let refs = GoExtractor::new().extract_doc_comments(std::path::Path::new("p.go"), src);
        assert!(refs.is_empty(), "a blank line must detach the comment");
    }

    #[test]
    fn go_type_position_refs_captured() {
        // A struct used via a composite literal / return type produces a TypeUse
        // ref. Also asserts a call ref survives (guards the refs query compiles).
        let src = "package p\ntype Group struct { n int }\nfunc build() *Group {\n  return &Group{n: 1}\n}\nfunc other() { build() }\n";
        let refs = GoExtractor::new().extract_references(std::path::Path::new("a.go"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "call ref lost (refs query broken?): {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Group" && r.kind == ReferenceKind::TypeUse),
            "Group type-position ref not captured: {refs:?}"
        );
    }
}
