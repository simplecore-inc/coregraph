use std::collections::BTreeMap;
use std::path::Path;

use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
use coregraph_graph::SymbolGraph;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::doc_comment::extract_block_doc_comments;
use crate::{DocCommentRef, ExtractError, RawReference, ReferenceKind, SymbolExtractor};

/// Returns `true` for a Rust doc comment's source text — the dedicated doc
/// syntax (`///` outer, `//!` inner line comments; `/** */`, `/*! */` block
/// comments). Plain `//` / `/* */` comments and the `////` / `/**/` edge cases
/// are NOT doc comments.
fn is_rust_doc_comment(text: &str) -> bool {
    (text.starts_with("///") && !text.starts_with("////"))
        || text.starts_with("//!")
        || (text.starts_with("/**") && text != "/**/" && !text.starts_with("/***"))
        || text.starts_with("/*!")
}

const RUST_QUERY: &str = include_str!("queries/rust.scm");
const RUST_REFS_QUERY: &str = include_str!("queries/rust-refs.scm");

pub struct RustExtractor {
    language: Language,
}

impl RustExtractor {
    pub fn new() -> Self {
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        Self { language }
    }
}

impl Default for RustExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor for RustExtractor {
    fn language_name(&self) -> &'static str {
        "Rust"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["rs"]
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
                query_name: "rust",
                message: e.to_string(),
            })?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| ExtractError::ParseFailed {
                path: path.to_path_buf(),
            })?;

        let query =
            Query::new(&self.language, RUST_QUERY).map_err(|e| ExtractError::QueryError {
                query_name: "rust.scm",
                message: e.to_string(),
            })?;

        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();

        // Dedup by span. The query has two patterns that both match impl-block
        // `fn` nodes: pattern 3 (any `function_item`) and pattern 4 (methods
        // scoped under `impl_item > declaration_list`). Without dedup the same
        // identifier's span is inserted twice — once as `Function`, once as
        // `Method` — doubling every method's in-degree and crowding the name
        // index. We collect first, then commit, preferring `Method` when both
        // kinds name the same span.
        //
        // BTreeMap keyed on the span so the subsequent `insert_node` pass
        // runs in a fixed order. HashMap iteration would feed `SymbolGraph`
        // nodes in random order, which in turn shuffled `graph.nodes()` and
        // caused `apply_resolutions_report` to emit a non-deterministic set
        // of `Resolves` edges (observed: ~400 drift per 48k edges run-to-run).
        let mut collected: BTreeMap<(u32, u32), (SymbolKind, String, Visibility, bool)> =
            BTreeMap::new();
        let name_idx = query.capture_index_for_name("name");
        let def_idx = query.capture_index_for_name("def");

        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
        while let Some(m) = matches.next() {
            let name_cap = name_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                .or_else(|| m.captures.first());
            let Some(name_cap) = name_cap else { continue };

            let kind = match m.pattern_index {
                0 => SymbolKind::Struct,
                1 => SymbolKind::Enum,
                2 => SymbolKind::Trait,
                3 => SymbolKind::Function,
                4 => SymbolKind::Method,
                5 => SymbolKind::TypeAlias,
                _ => continue,
            };

            let name = match name_cap.node.utf8_text(source_bytes) {
                Ok(n) => n.to_string(),
                Err(_) => continue,
            };

            // Use @def span (full declaration) when available, falling back
            // to the identifier span. Full-body spans let `enclosing_symbol`
            // resolve refs inside method bodies to the right method.
            let def_cap = def_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
            let (span_start, span_end) = match def_cap {
                Some(c) => (c.node.start_byte() as u32, c.node.end_byte() as u32),
                None => (
                    name_cap.node.start_byte() as u32,
                    name_cap.node.end_byte() as u32,
                ),
            };
            let key = (span_start, span_end);

            // A `pub` (`visibility_modifier`) child on the declaration → Public;
            // its absence in Rust means module-private. (Falls back to Unknown
            // when there is no @def node to inspect.)
            let visibility = match def_cap {
                Some(c) => {
                    let has_pub = (0..c.node.child_count())
                        .filter_map(|i| c.node.child(i))
                        .any(|child| child.kind() == "visibility_modifier");
                    if has_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    }
                }
                None => Visibility::Unknown,
            };

            let is_test = def_cap
                .map(|c| rust_is_test(c.node, source_bytes))
                .unwrap_or(false);

            collected
                .entry(key)
                .and_modify(|(existing_kind, _, _, _)| {
                    // `Method` is more specific than `Function` for the same
                    // span, so prefer it when both patterns hit. Any other
                    // conflict is left as-is (shouldn't happen for a single
                    // tree-sitter node).
                    if matches!(existing_kind, SymbolKind::Function)
                        && matches!(kind, SymbolKind::Method)
                    {
                        *existing_kind = SymbolKind::Method;
                    }
                })
                .or_insert((kind, name, visibility, is_test));
        }

        for ((span_start, span_end), (kind, name, visibility, is_test)) in collected {
            let node = SymbolNode::new(SymbolId(0), kind, name, path, span_start, span_end)
                .with_visibility(visibility)
                .with_test(is_test);
            graph.insert_node(node);
        }

        Ok(())
    }

    fn extract_references(&self, path: &Path, source: &str) -> Vec<RawReference> {
        let mut parser = Parser::new();
        if parser.set_language(&self.language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(source, None) else {
            return Vec::new();
        };
        let Ok(query) = Query::new(&self.language, RUST_REFS_QUERY) else {
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();
        let mut out = Vec::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
        while let Some(m) = matches.next() {
            // Patterns 0-2 = Call, 3-7 = Import, 8-9 = Implements, 10 = TypeUse
            let kind = match m.pattern_index {
                0..=2 => ReferenceKind::Call,
                3..=7 => ReferenceKind::Import,
                8..=9 => ReferenceKind::Implements,
                10 => ReferenceKind::TypeUse,
                _ => continue,
            };
            for cap in m.captures {
                let Ok(name) = cap.node.utf8_text(source_bytes) else {
                    continue;
                };
                if should_skip_reference(name) {
                    continue;
                }
                out.push(RawReference {
                    name: name.to_string(),
                    kind,
                    byte_offset: cap.node.start_byte() as u32,
                });
            }
        }
        // Drop suspiciously short names (1-2 chars) to avoid `i`/`x` noise.
        out.retain(|r| r.name.len() >= 3);
        let _ = path;
        out
    }

    fn extract_doc_comments(&self, _path: &Path, source: &str) -> Vec<DocCommentRef> {
        // Rust attaches doc comments as previous siblings; attributes
        // (`#[derive(...)]`) may sit between the doc and the item.
        extract_block_doc_comments(
            &self.language,
            RUST_QUERY,
            source,
            &["line_comment", "block_comment"],
            &["attribute_item"],
            &[],
            coregraph_core::edge::AnalysisOrigin::SyntaxMatched,
            &is_rust_doc_comment,
        )
    }
}

/// Whether the declaration at `def` is Rust test code: it (or any enclosing
/// module) carries a `#[test]`/`#[tokio::test]`-family attribute or a
/// `#[cfg(test)]` gate. Walks up the ancestor chain so a function inside a
/// `#[cfg(test)] mod tests` block is recognised even though its file is not a
/// test-path file.
fn rust_is_test(def: tree_sitter::Node, source: &[u8]) -> bool {
    let mut node = Some(def);
    while let Some(n) = node {
        if node_has_test_attribute(n, source) {
            return true;
        }
        node = n.parent();
    }
    false
}

/// True when an `attribute_item` immediately preceding `n` (skipping comments)
/// is a test marker. Attributes in Rust are previous siblings of the item.
fn node_has_test_attribute(n: tree_sitter::Node, source: &[u8]) -> bool {
    let mut sib = n.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "attribute_item" => {
                if let Ok(text) = s.utf8_text(source) {
                    if is_test_attribute(text) {
                        return true;
                    }
                }
                sib = s.prev_sibling();
            }
            // Doc/comment lines may sit between an attribute and its item.
            "line_comment" | "block_comment" => sib = s.prev_sibling(),
            _ => break,
        }
    }
    false
}

/// Classify an attribute's source text as a test marker: a `#[test]`-family
/// attribute (path ending in `test`, e.g. `test`, `tokio::test`, `rstest`) or a
/// `#[cfg(test)]` gate (a `cfg` predicate mentioning the bare `test` token).
fn is_test_attribute(attr_text: &str) -> bool {
    let s = attr_text.trim();
    let s = s
        .strip_prefix("#![")
        .or_else(|| s.strip_prefix("#["))
        .unwrap_or(s);
    let s = s.strip_suffix(']').unwrap_or(s).trim();
    let name_part = s.split('(').next().unwrap_or(s).trim();
    let last_seg = name_part.rsplit("::").next().unwrap_or(name_part).trim();
    if last_seg == "test" {
        return true;
    }
    if name_part == "cfg" {
        // Drop string-literal values so `cfg(feature = "test")` (a production
        // gate that merely names a "test" feature) does NOT count — only a bare
        // `test` cfg option, as in `cfg(test)` / `cfg(all(test, …))`, matches.
        let pred = &s[name_part.len()..];
        let mut bare = String::with_capacity(pred.len());
        let mut in_str = false;
        for c in pred.chars() {
            match c {
                '"' => in_str = !in_str,
                _ if !in_str => bare.push(c),
                _ => {}
            }
        }
        return bare
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|tok| tok == "test");
    }
    false
}

/// Rust identifiers that look like references to the extractor but are
/// actually language keywords, built-in variants, or universal types.
/// Forwarding them to `resolve_references` wastes a lookup per occurrence
/// and risks spurious edges when a project accidentally defines a symbol
/// with the same name (e.g. a type alias `type Ok = ...;`).
fn should_skip_reference(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    matches!(
        name,
        // Rust keyword-like path segments the parser exposes as identifiers.
        "self" | "Self" | "super" | "crate"
        // Option / Result variants — matched by `enum_variant` capture
        // everywhere, never refer to user-defined symbols.
        | "None" | "Some" | "Ok" | "Err"
        // Boolean literals.
        | "true" | "false"
        // Primitive numeric types — `u32`, `i64`, etc. — plus other
        // built-ins that show up in references but never as user symbols.
        | "bool" | "char" | "str" | "usize" | "isize"
        | "u8" | "u16" | "u32" | "u64" | "u128"
        | "i8" | "i16" | "i32" | "i64" | "i128"
        | "f32" | "f64"
    )
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
        let extractor = RustExtractor::new();
        let path = fixture_path(subdir, filename);
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extraction failed");
        graph
    }

    #[test]
    fn extracts_struct_rust() {
        let graph = extract_from_fixture("rust-simple", "lib.rs");
        let found = graph
            .nodes()
            .any(|n| n.name == "Config" && n.kind == SymbolKind::Struct);
        assert!(found, "Expected to find Config struct");
    }

    #[test]
    fn extracts_trait() {
        let graph = extract_from_fixture("rust-simple", "lib.rs");
        let found = graph
            .nodes()
            .any(|n| n.name == "Processor" && n.kind == SymbolKind::Trait);
        assert!(found, "Expected to find Processor trait");
    }

    #[test]
    fn extracts_enum_rust() {
        let graph = extract_from_fixture("rust-simple", "lib.rs");
        let found = graph
            .nodes()
            .any(|n| n.name == "ProcessError" && n.kind == SymbolKind::Enum);
        assert!(found, "Expected to find ProcessError enum");
    }

    #[test]
    fn extracts_function_rust() {
        let graph = extract_from_fixture("rust-simple", "lib.rs");
        let found = graph
            .nodes()
            .any(|n| n.name == "default_config" && n.kind == SymbolKind::Function);
        assert!(found, "Expected to find default_config function");
    }

    #[test]
    fn rust_extensions() {
        let extractor = RustExtractor::new();
        assert!(extractor.file_extensions().contains(&"rs"));
    }

    fn doc_refs(source: &str) -> Vec<DocCommentRef> {
        RustExtractor::new().extract_doc_comments(std::path::Path::new("src/lib.rs"), source)
    }

    /// The source text covered by a `DocCommentRef`'s `doc_span`.
    fn doc_text<'a>(source: &'a str, dref: &DocCommentRef) -> &'a str {
        &source[dref.doc_span.0 as usize..dref.doc_span.1 as usize]
    }

    #[test]
    fn outer_doc_comment_is_paired_with_its_definition() {
        let src = "/// Greets the caller.\npub fn greet() {}\n";
        let refs = doc_refs(src);
        assert_eq!(refs.len(), 1, "expected one documented definition");
        // doc_span must cover the `///` line; def_span must start at the fn.
        assert!(doc_text(src, &refs[0]).starts_with("///"));
        assert_eq!(
            &src[refs[0].def_span.0 as usize..refs[0].def_span.0 as usize + 3],
            "pub"
        );
    }

    #[test]
    fn function_without_doc_yields_no_ref() {
        assert!(doc_refs("pub fn greet() {}\n").is_empty());
    }

    #[test]
    fn plain_line_comment_is_not_a_doc_comment() {
        // A regular `//` comment is not documentation.
        assert!(doc_refs("// just a note\npub fn greet() {}\n").is_empty());
    }

    #[test]
    fn multiline_doc_comment_merges_into_one_span() {
        let src = "/// Line one.\n/// Line two.\n/// Line three.\npub fn greet() {}\n";
        let refs = doc_refs(src);
        assert_eq!(refs.len(), 1, "three /// lines describe one definition");
        let text = doc_text(src, &refs[0]);
        assert!(
            text.contains("Line one.") && text.contains("Line three."),
            "merged doc span must cover all three lines, got {text:?}"
        );
    }

    #[test]
    fn doc_comment_on_struct_is_paired() {
        let refs = doc_refs("/// A configuration holder.\npub struct Config { x: i32 }\n");
        assert_eq!(refs.len(), 1, "the struct's doc comment must be paired");
    }

    #[test]
    fn doc_comment_through_attribute_is_paired() {
        // A doc comment, then an attribute, then the item: the attribute sits
        // between the doc and the definition, but the pairing must still hold.
        let refs = doc_refs("/// Derives debug.\n#[derive(Debug)]\npub struct Config { x: i32 }\n");
        assert_eq!(refs.len(), 1, "a doc before an attribute must still pair");
    }

    #[test]
    fn is_rust_doc_comment_classifies_markers() {
        assert!(is_rust_doc_comment("/// outer"));
        assert!(is_rust_doc_comment("//! inner"));
        assert!(is_rust_doc_comment("/** block */"));
        assert!(is_rust_doc_comment("/*! inner block */"));
        // Non-doc forms:
        assert!(!is_rust_doc_comment("// plain"));
        assert!(!is_rust_doc_comment("//// not a doc"));
        assert!(!is_rust_doc_comment("/* block */"));
        assert!(!is_rust_doc_comment("/**/"));
    }

    #[test]
    fn visibility_detected_from_pub_keyword() {
        let src = "pub fn exported() {}\nfn internal() {}\npub struct Cfg;\nstruct Hidden;\n";
        let mut g = SymbolGraph::new();
        RustExtractor::new()
            .extract(std::path::Path::new("a.rs"), src, &mut g)
            .unwrap();
        let vis = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.visibility);
        assert_eq!(vis("exported"), Some(Visibility::Public));
        assert_eq!(vis("internal"), Some(Visibility::Private));
        assert_eq!(vis("Cfg"), Some(Visibility::Public));
        assert_eq!(vis("Hidden"), Some(Visibility::Private));
    }

    #[test]
    fn detects_inline_test_functions() {
        // `#[test]`/`#[tokio::test]` on a fn, and any fn inside a `#[cfg(test)]`
        // module, are test code even though the file is not a test-path file.
        let src = "pub fn real() {}\n\
                   #[cfg(feature = \"test\")]\nfn feature_gated() {}\n\
                   #[test]\nfn direct_test() {}\n\
                   #[tokio::test]\nasync fn async_test() {}\n\
                   #[cfg(test)]\nmod tests {\n  fn helper_in_test_mod() {}\n  #[test]\n  fn nested() {}\n}\n";
        let mut g = SymbolGraph::new();
        RustExtractor::new()
            .extract(std::path::Path::new("a.rs"), src, &mut g)
            .unwrap();
        let t = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.is_test);
        assert_eq!(t("real"), Some(false), "production fn is not test");
        assert_eq!(
            t("feature_gated"),
            Some(false),
            "cfg(feature=\"test\") is NOT a test gate"
        );
        assert_eq!(t("direct_test"), Some(true), "#[test] fn is test");
        assert_eq!(t("async_test"), Some(true), "#[tokio::test] fn is test");
        assert_eq!(
            t("helper_in_test_mod"),
            Some(true),
            "fn in #[cfg(test)] mod is test"
        );
        assert_eq!(t("nested"), Some(true), "#[test] in test mod is test");
    }

    #[test]
    fn type_position_refs_captured() {
        // A struct used only as a field / param / return type produces a TypeUse
        // ref. Also asserts a call ref survives (guards the refs query compiles).
        let src = "struct Widget;\nfn build(w: Widget) -> Widget {\n    helper();\n    w\n}\n";
        let refs = RustExtractor::new().extract_references(std::path::Path::new("a.rs"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "helper" && r.kind == ReferenceKind::Call),
            "call ref lost (refs query broken?): {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Widget" && r.kind == ReferenceKind::TypeUse),
            "Widget type-position ref not captured: {refs:?}"
        );
    }
}
