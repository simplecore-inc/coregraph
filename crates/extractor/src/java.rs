use std::path::Path;

use coregraph_core::{SymbolKind, SymbolNode, Visibility};
use coregraph_graph::SymbolGraph;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::doc_comment::{extract_block_doc_comments, is_block_doc_comment};
use crate::{DocCommentRef, ExtractError, RawReference, ReferenceKind, SymbolExtractor};

const JAVA_QUERY: &str = include_str!("queries/java.scm");
const JAVA_REFS_QUERY: &str = include_str!("queries/java-refs.scm");

pub struct JavaExtractor;

impl JavaExtractor {
    pub fn new() -> Self {
        JavaExtractor
    }
}

impl Default for JavaExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Java declared visibility from the declaration's `modifiers` child: explicit
/// `public` → Public, `private`/`protected` → Private. No access modifier stays
/// Unknown (package-private at class level, but implicitly public for interface
/// members — ambiguous, so consumers fall back to the name heuristic).
fn java_visibility(def: tree_sitter::Node) -> Visibility {
    let mut cursor = def.walk();
    for child in def.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mc = child.walk();
            for m in child.children(&mut mc) {
                match m.kind() {
                    "public" => return Visibility::Public,
                    "private" | "protected" => return Visibility::Private,
                    _ => {}
                }
            }
        }
    }
    Visibility::Unknown
}

/// Whether the declaration carries a JUnit/TestNG test annotation in its
/// `modifiers` (`@Test`, `@ParameterizedTest`, `@RepeatedTest`, `@TestFactory`,
/// or a `@Before*`/`@After*` lifecycle hook). The annotation's simple name is
/// matched, so a fully-qualified `@org.junit.jupiter.api.Test` also counts.
fn java_is_test(def: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = def.walk();
    for child in def.children(&mut cursor) {
        if child.kind() != "modifiers" {
            continue;
        }
        let mut mc = child.walk();
        for m in child.children(&mut mc) {
            if m.kind() != "marker_annotation" && m.kind() != "annotation" {
                continue;
            }
            let Some(name_node) = m.child_by_field_name("name") else {
                continue;
            };
            let Ok(name) = name_node.utf8_text(source) else {
                continue;
            };
            let simple = name.rsplit('.').next().unwrap_or(name);
            if is_test_annotation(simple) {
                return true;
            }
        }
    }
    false
}

fn is_test_annotation(simple_name: &str) -> bool {
    matches!(
        simple_name,
        "Test"
            | "ParameterizedTest"
            | "RepeatedTest"
            | "TestFactory"
            | "TestTemplate"
            | "BeforeEach"
            | "AfterEach"
            | "BeforeAll"
            | "AfterAll"
            | "Before"
            | "After"
            | "BeforeClass"
            | "AfterClass"
    )
}

impl SymbolExtractor for JavaExtractor {
    fn language_name(&self) -> &'static str {
        "java"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["java"]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();

        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|_| ExtractError::ParseFailed {
                path: path.to_path_buf(),
            })?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| ExtractError::ParseFailed {
                path: path.to_path_buf(),
            })?;

        let query = Query::new(&language, JAVA_QUERY).map_err(|e| ExtractError::QueryError {
            query_name: "java.scm",
            message: e.to_string(),
        })?;

        let name_idx = query
            .capture_index_for_name("name")
            .expect("java.scm must have a @name capture");
        let def_idx = query.capture_index_for_name("def");

        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();

        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
        while let Some(m) = matches.next() {
            // Map pattern index to SymbolKind:
            // 0 = class, 1 = interface, 2 = method, 3 = constructor,
            // 4 = field, 5 = enum, 6 = enum_variant.
            let kind = match m.pattern_index {
                0 => SymbolKind::Class,
                1 => SymbolKind::Interface,
                2 | 3 => SymbolKind::Method,
                4 => SymbolKind::Field,
                5 => SymbolKind::Enum,
                6 => SymbolKind::EnumVariant,
                _ => SymbolKind::Function,
            };

            let def_node =
                def_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx).map(|c| c.node));
            let def_span = def_node.map(|n| (n.start_byte() as u32, n.end_byte() as u32));
            let visibility = def_node.map(java_visibility).unwrap_or(Visibility::Unknown);
            let is_test = def_node
                .map(|n| java_is_test(n, source_bytes))
                .unwrap_or(false);

            for cap in m.captures {
                if cap.index == name_idx {
                    let node = cap.node;
                    let name = node.utf8_text(source_bytes).unwrap_or("").to_string();
                    let (start, end) =
                        def_span.unwrap_or((node.start_byte() as u32, node.end_byte() as u32));
                    let symbol = SymbolNode::new(
                        coregraph_core::SymbolId(0), // assigned by insert_node
                        kind.clone(),
                        name,
                        path,
                        start,
                        end,
                    )
                    .with_visibility(visibility)
                    .with_test(is_test);
                    graph.insert_node(symbol);
                }
            }
        }

        Ok(())
    }

    fn extract_references(&self, _path: &Path, source: &str) -> Vec<RawReference> {
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let mut parser = Parser::new();
        if parser.set_language(&language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(source, None) else {
            return Vec::new();
        };
        let Ok(query) = Query::new(&language, JAVA_REFS_QUERY) else {
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();
        let mut out = Vec::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
        while let Some(m) = matches.next() {
            let kind = match m.pattern_index {
                0..=2 => ReferenceKind::Call,
                3..=4 => ReferenceKind::Import,
                5 => ReferenceKind::Extends,
                6 => ReferenceKind::Implements,
                7 => ReferenceKind::TypeUse,
                8..=9 => ReferenceKind::Import,
                _ => continue,
            };
            for cap in m.captures {
                let Ok(name) = cap.node.utf8_text(source_bytes) else {
                    continue;
                };
                if name.is_empty() || name == "this" || name == "super" {
                    continue;
                }
                out.push(RawReference {
                    name: name.to_string(),
                    kind,
                    byte_offset: cap.node.start_byte() as u32,
                });
            }
        }
        out.retain(|r| r.name.len() >= 3);
        out
    }

    fn extract_doc_comments(&self, _path: &Path, source: &str) -> Vec<DocCommentRef> {
        // Javadoc `/** … */` is a previous sibling of the declaration;
        // annotations live inside the declaration's `modifiers`, so nothing
        // needs skipping.
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        extract_block_doc_comments(
            &language,
            JAVA_QUERY,
            source,
            &["block_comment", "line_comment"],
            &[],
            &[],
            coregraph_core::edge::AnalysisOrigin::SyntaxMatched,
            &is_block_doc_comment,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_graph::SymbolGraph;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/java-simple")
            .join(name)
    }

    #[test]
    fn extracts_class_from_hello_world() {
        let extractor = JavaExtractor;
        let path = fixture_path("HelloWorld.java");
        let source = std::fs::read_to_string(&path).expect("read HelloWorld.java");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract");

        let found = graph
            .nodes()
            .any(|n| n.name == "HelloWorld" && n.kind == SymbolKind::Class);
        assert!(found, "expected HelloWorld class node");
    }

    #[test]
    fn extracts_methods_from_hello_world() {
        let extractor = JavaExtractor;
        let path = fixture_path("HelloWorld.java");
        let source = std::fs::read_to_string(&path).expect("read HelloWorld.java");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract");

        let found = graph
            .nodes()
            .any(|n| n.name == "greet" && n.kind == SymbolKind::Method);
        assert!(found, "expected greet method node");
    }

    #[test]
    fn extracts_interface() {
        let extractor = JavaExtractor;
        let path = fixture_path("UserService.java");
        let source = std::fs::read_to_string(&path).expect("read UserService.java");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract");

        let found = graph
            .nodes()
            .any(|n| n.name == "UserService" && n.kind == SymbolKind::Interface);
        assert!(found, "expected UserService interface node");
    }

    #[test]
    fn java_extensions() {
        let extractor = JavaExtractor;
        assert!(extractor.file_extensions().contains(&"java"));
    }

    #[test]
    fn javadoc_is_paired_with_class() {
        let src = "/** A greeter. */\npublic class Greeter {}\n";
        let refs = JavaExtractor.extract_doc_comments(std::path::Path::new("G.java"), src);
        assert_eq!(refs.len(), 1, "Javadoc must pair with the class");
        assert!(src[refs[0].doc_span.0 as usize..refs[0].doc_span.1 as usize].starts_with("/**"));
    }

    #[test]
    fn plain_block_comment_is_not_javadoc() {
        let refs = JavaExtractor.extract_doc_comments(
            std::path::Path::new("G.java"),
            "/* not a doc */\npublic class Greeter {}\n",
        );
        assert!(refs.is_empty(), "a plain /* */ comment is not Javadoc");
    }

    #[test]
    fn java_visibility_detected_from_modifiers() {
        let src = "public class Svc {\n  private int field;\n  public void run() {}\n  void pkgPrivate() {}\n}\n";
        let mut g = SymbolGraph::new();
        JavaExtractor
            .extract(std::path::Path::new("Svc.java"), src, &mut g)
            .unwrap();
        let vis = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.visibility);
        assert_eq!(vis("Svc"), Some(Visibility::Public));
        assert_eq!(vis("field"), Some(Visibility::Private));
        assert_eq!(vis("run"), Some(Visibility::Public));
        assert_eq!(vis("pkgPrivate"), Some(Visibility::Unknown)); // no access modifier
    }

    #[test]
    fn java_test_methods_flagged() {
        // A `@Test`/`@BeforeEach` method is test code even in a non-`*Test.java`
        // file; a plain method is not.
        let src = "class Svc {\n  @Test void shouldRun() {}\n  @org.junit.jupiter.api.Test void fq() {}\n  @BeforeEach void setup() {}\n  void real() {}\n}\n";
        let mut g = SymbolGraph::new();
        JavaExtractor
            .extract(std::path::Path::new("Svc.java"), src, &mut g)
            .unwrap();
        let t = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.is_test);
        assert_eq!(t("shouldRun"), Some(true), "@Test method is test");
        assert_eq!(t("fq"), Some(true), "fully-qualified @Test is test");
        assert_eq!(t("setup"), Some(true), "@BeforeEach is test");
        assert_eq!(t("real"), Some(false), "un-annotated method is not test");
    }

    #[test]
    fn java_annotation_refs_captured() {
        // A framework-stereotype annotation references its type, connecting the
        // annotated entry-point symbol so it is not a false orphan.
        let src = "@RestController\nclass OwnerController {\n  @GetMapping String list() { return \"x\"; }\n}\n";
        let refs = JavaExtractor.extract_references(std::path::Path::new("C.java"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "RestController" && r.kind == ReferenceKind::Import),
            "@RestController annotation ref not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "GetMapping" && r.kind == ReferenceKind::Import),
            "@GetMapping annotation ref not captured: {refs:?}"
        );
    }

    #[test]
    fn java_type_position_refs_captured() {
        // A class used only as a field/param/return type produces a TypeUse ref.
        // Also asserts a method-call ref survives (guards the refs query compiles).
        let src = "class C {\n  private Widget w;\n  Widget build() { helper(); return w; }\n}\n";
        let refs = JavaExtractor.extract_references(std::path::Path::new("C.java"), src);
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
