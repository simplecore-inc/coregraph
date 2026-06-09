use std::path::Path;

use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
use coregraph_graph::SymbolGraph;
use tree_sitter::Query;

use crate::doc_comment::{extract_block_doc_comments, is_block_doc_comment};
use crate::{
    tree_sitter_extract_refs, unwraps_to_fn, DocCommentRef, ExtractError, RawReference,
    ReferenceKind, SymbolExtractor,
};

// Query patterns are ordered: class(0), interface(1), function(2), method(3),
// type_alias(4), enum(5), enum-variant(6,7), arrow/fn-expr const(8),
// class arrow-property(9). The pattern_index match below mirrors this order.
const TS_QUERY: &str = include_str!("queries/typescript.scm");
// Base reference query — grammar-agnostic patterns valid for both `.ts`
// (typescript grammar) and `.tsx` (tsx grammar). Pattern indices 0..=29.
const TS_REFS_QUERY: &str = include_str!("queries/typescript-refs.scm");
// JSX-only reference patterns, appended to the base query for `.tsx` files (the
// `jsx_*` nodes do not exist in the plain typescript grammar, so compiling them
// against it fails the whole query and drops every `.ts` ref). Becomes the LAST
// patterns, so their indices follow every base pattern (30..=32).
const TS_REFS_JSX_QUERY: &str = include_str!("queries/typescript-refs-jsx.scm");

pub struct TypeScriptExtractor;

impl TypeScriptExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TypeScriptExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// TypeScript declared visibility from the declaration node (`@def`). Class
/// members carry an explicit `accessibility_modifier` (`public`/`private`/
/// `protected`); top-level declarations wrapped in an `export_statement` are the
/// module's public API. Anything else stays Unknown so consumers fall back to
/// the name-shape heuristic (a non-exported top-level symbol is module-private
/// in principle, but inferring that reliably needs scope analysis, so we do not
/// over-claim here).
fn ts_visibility(def: tree_sitter::Node, source: &str) -> Visibility {
    let mut cursor = def.walk();
    for child in def.children(&mut cursor) {
        if child.kind() == "accessibility_modifier" {
            return match &source[child.start_byte()..child.end_byte()] {
                "private" | "protected" => Visibility::Private,
                "public" => Visibility::Public,
                _ => Visibility::Unknown,
            };
        }
    }
    // Arrow-const symbols capture the `variable_declarator` as @def, so the
    // `export` keyword sits two levels up (declarator → lexical_declaration →
    // export_statement). Walk up through the declaration wrapper so exported
    // arrow-consts are recognised as public API, not just `function` decls.
    let mut node = def;
    if node.kind() == "variable_declarator" {
        if let Some(parent) = node.parent() {
            node = parent;
        }
    }
    if node.parent().map(|p| p.kind()) == Some("export_statement") {
        return Visibility::Public;
    }
    Visibility::Unknown
}

/// True when a `variable_declarator` is a module top-level binding (its
/// declaration's parent is the program root or an `export_statement`), as
/// opposed to a function-local `const`/`let`. Top-level value bindings are
/// part of the module's API surface and worth indexing; function-local ones
/// are noise.
fn is_top_level_declarator(declarator: tree_sitter::Node) -> bool {
    declarator
        .parent() // lexical_declaration / variable_declaration
        .and_then(|decl| decl.parent())
        .map(|p| matches!(p.kind(), "program" | "export_statement"))
        .unwrap_or(false)
}

/// Maps a value declarator to Constant (`const`) or Variable (`let`/`var`) by
/// inspecting the declaration keyword.
fn value_binding_kind(declarator: tree_sitter::Node, source: &str) -> SymbolKind {
    let is_const = declarator
        .parent()
        .map(|decl| {
            source[decl.start_byte()..decl.end_byte()]
                .trim_start()
                .starts_with("const")
        })
        .unwrap_or(true);
    if is_const {
        SymbolKind::Constant
    } else {
        SymbolKind::Variable
    }
}

/// Selects the tree-sitter grammar dialect for a TypeScript source file: `.tsx`
/// files need the TSX grammar (JSX support) so JSX bodies parse as real syntax
/// rather than error nodes that swallow the symbols and references inside them;
/// every other TypeScript file uses the plain TypeScript grammar.
fn ts_language_for(path: &Path) -> tree_sitter::Language {
    if path.extension().and_then(|e| e.to_str()) == Some("tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
}

impl SymbolExtractor for TypeScriptExtractor {
    fn language_name(&self) -> &'static str {
        "typescript"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["ts", "tsx"]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        let language = ts_language_for(path);

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| ExtractError::QueryError {
                query_name: "typescript",
                message: e.to_string(),
            })?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| ExtractError::ParseFailed {
                path: path.to_path_buf(),
            })?;

        let query = Query::new(&language, TS_QUERY).map_err(|e| ExtractError::QueryError {
            query_name: "typescript",
            message: e.to_string(),
        })?;

        let name_capture_idx =
            query
                .capture_index_for_name("name")
                .ok_or_else(|| ExtractError::QueryError {
                    query_name: "typescript",
                    message: "capture @name not found".to_string(),
                })?;
        let enum_name_capture_idx = query.capture_index_for_name("enum_name");
        let def_capture_idx = query.capture_index_for_name("def");
        let fnbody_capture_idx = query.capture_index_for_name("fnbody");

        let mut cursor = tree_sitter::QueryCursor::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            // Patterns 8/9 capture the declarator value broadly as @fnbody (the
            // function may be wrapped in parens / `as` / `satisfies` / `!`).
            let fnbody = || {
                fnbody_capture_idx
                    .and_then(|idx| m.captures.iter().find(|c| c.index == idx).map(|c| c.node))
            };
            let def_node_of = || {
                def_capture_idx
                    .and_then(|idx| m.captures.iter().find(|c| c.index == idx).map(|c| c.node))
            };
            let kind = match m.pattern_index {
                0 => SymbolKind::Class,
                1 => SymbolKind::Interface,
                2 => SymbolKind::Function,
                3 => SymbolKind::Method,
                4 => SymbolKind::TypeAlias,
                5 => SymbolKind::Enum,
                6 | 7 => SymbolKind::EnumVariant,
                // Function-valued const -> Function. Otherwise a top-level value
                // binding (`const x = {…}`, `export const persist = impl as P`)
                // -> Constant/Variable; a function-LOCAL value const is skipped
                // to avoid graph noise.
                8 => {
                    if fnbody().map(unwraps_to_fn).unwrap_or(false) {
                        SymbolKind::Function
                    } else {
                        match def_node_of().filter(|d| is_top_level_declarator(*d)) {
                            Some(d) => value_binding_kind(d, source),
                            None => continue,
                        }
                    }
                }
                // Class field: a function-valued property is a Method; a plain
                // data field is left out (not the focus of this extractor).
                9 => {
                    if fnbody().map(unwraps_to_fn).unwrap_or(false) {
                        SymbolKind::Method
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };

            // For EnumVariant patterns (6, 7), the @name capture is the variant
            // identifier and @enum_name captures the parent Enum's identifier.
            // We store the variant as `EnumName.VariantName` so the cross-file
            // enum-mismatch detector (which splits on `.`) can tell
            // `Role.ADMIN` and `Permission.ADMIN` apart.
            let parent_enum = if matches!(kind, SymbolKind::EnumVariant) {
                enum_name_capture_idx.and_then(|idx| {
                    m.captures
                        .iter()
                        .find(|c| c.index == idx)
                        .map(|c| source[c.node.start_byte()..c.node.end_byte()].to_string())
                })
            } else {
                None
            };

            // Span covers the full declaration (@def). Falls back to the
            // identifier span (@name) when the query has no @def capture,
            // keeping backwards compatibility with any reduced query set.
            let def_node = def_capture_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx).map(|c| c.node));
            let def_span = def_node.map(|n| (n.start_byte() as u32, n.end_byte() as u32));
            let visibility = def_node
                .map(|n| ts_visibility(n, source))
                .unwrap_or(Visibility::Unknown);

            for cap in m.captures {
                if cap.index != name_capture_idx {
                    continue;
                }
                let node = cap.node;
                let raw_name = &source[node.start_byte()..node.end_byte()];
                let name = match &parent_enum {
                    Some(p) => format!("{p}.{raw_name}"),
                    None => raw_name.to_string(),
                };
                let (span_start, span_end) =
                    def_span.unwrap_or((node.start_byte() as u32, node.end_byte() as u32));
                let symbol = SymbolNode::new(
                    SymbolId(0), // will be overwritten by insert_node
                    kind.clone(),
                    name,
                    path,
                    span_start,
                    span_end,
                )
                .with_visibility(visibility);
                graph.insert_node(symbol);
            }
        }

        Ok(())
    }

    fn extract_references(&self, path: &Path, source: &str) -> Vec<RawReference> {
        let lang = ts_language_for(path);
        // `.tsx` gets the JSX patterns appended (TSX grammar only); `.ts` uses the
        // base query alone. The JSX patterns are last, so the base indices
        // (0..=29) are identical in both and the JSX patterns take the next
        // indices (30..=32) — present only for `.tsx`.
        let is_tsx = path.extension().and_then(|e| e.to_str()) == Some("tsx");
        let query_src: std::borrow::Cow<'static, str> = if is_tsx {
            std::borrow::Cow::Owned(format!("{TS_REFS_QUERY}\n{TS_REFS_JSX_QUERY}"))
        } else {
            std::borrow::Cow::Borrowed(TS_REFS_QUERY)
        };
        tree_sitter_extract_refs(&lang, &query_src, source, |idx| match idx {
            0..=2 => Some(ReferenceKind::Call),
            3..=5 => Some(ReferenceKind::Import),
            6 => Some(ReferenceKind::Extends),
            7..=10 => Some(ReferenceKind::TypeUse),
            11 => Some(ReferenceKind::Import),
            12..=13 => Some(ReferenceKind::TypeUse),
            14..=18 => Some(ReferenceKind::Call), // function-value refs (default/arg/param)
            19..=24 => Some(ReferenceKind::TypeUse), // composite + generic-name type refs
            25..=26 => Some(ReferenceKind::Call), // value-ternary function refs
            27 => Some(ReferenceKind::ReexportAll), // `export * from` wildcard re-export
            28..=29 => Some(ReferenceKind::ValueRef), // value-position reads: subscript/member object
            30..=31 => Some(ReferenceKind::Call),     // JSX element references (.tsx only)
            32 => Some(ReferenceKind::ValueRef),      // JSX prop value identifier (.tsx only)
            _ => None,
        })
    }

    fn extract_doc_comments(&self, path: &Path, source: &str) -> Vec<DocCommentRef> {
        // JSDoc `/** … */` is a previous sibling of the declaration; TS
        // decorators (`@Component(...)`) sit between the doc and the item.
        let language = ts_language_for(path);
        extract_block_doc_comments(
            &language,
            TS_QUERY,
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
    fn extracts_class() {
        let extractor = TypeScriptExtractor::new();
        let path = fixture_path("ts-simple", "index.ts");
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract failed");

        let found = graph
            .nodes()
            .any(|n| n.name == "UserController" && n.kind == SymbolKind::Class);
        assert!(found, "UserController class not found");
    }

    #[test]
    fn extracts_interface() {
        let extractor = TypeScriptExtractor::new();
        let path = fixture_path("ts-simple", "index.ts");
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract failed");

        let found = graph
            .nodes()
            .any(|n| n.name == "UserService" && n.kind == SymbolKind::Interface);
        assert!(found, "UserService interface not found");
    }

    #[test]
    fn extracts_function() {
        let extractor = TypeScriptExtractor::new();
        let path = fixture_path("ts-simple", "index.ts");
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract failed");

        let found = graph
            .nodes()
            .any(|n| n.name == "createController" && n.kind == SymbolKind::Function);
        assert!(found, "createController function not found");
    }

    #[test]
    fn extracts_arrow_const_function() {
        // `export const x = () => {}` is the dominant modern TS/JS function
        // style (every zustand public API symbol uses it). It must be captured
        // as a Function symbol, otherwise calls never resolve to it and the
        // whole call graph for idiomatic TS is missing.
        let src =
            "export const createStore = (createState) => createState;\nconst helper = () => 1;\n";
        let mut g = SymbolGraph::new();
        TypeScriptExtractor::new()
            .extract(std::path::Path::new("vanilla.ts"), src, &mut g)
            .unwrap();
        assert!(
            g.nodes()
                .any(|n| n.name == "createStore" && n.kind == SymbolKind::Function),
            "exported arrow-const createStore not extracted as Function: {:?}",
            g.nodes()
                .map(|n| (n.name.clone(), n.kind.clone()))
                .collect::<Vec<_>>()
        );
        assert!(
            g.nodes()
                .any(|n| n.name == "helper" && n.kind == SymbolKind::Function),
            "local arrow-const helper not extracted as Function"
        );
    }

    #[test]
    fn extracts_top_level_value_consts_but_not_function_local() {
        // Top-level value bindings are public/module API and must be symbols,
        // including zustand's cast-wrapped impl re-exports
        // (`export const persist = persistImpl as unknown as Persist`) and plain
        // data consts. Function-LOCAL consts stay out to avoid graph noise.
        let src = "export const persist = persistImpl as unknown as Persist;\n\
                   const moduleConst = { ok: 200 };\n\
                   export let counter = 0;\n\
                   function f() { const localOnly = 5; return localOnly; }\n";
        let mut g = SymbolGraph::new();
        TypeScriptExtractor::new()
            .extract(std::path::Path::new("m.ts"), src, &mut g)
            .unwrap();
        let node = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.kind.clone());
        assert_eq!(
            node("persist"),
            Some(SymbolKind::Constant),
            "cast-wrapped re-export `persist` must be a Constant symbol"
        );
        assert_eq!(
            node("moduleConst"),
            Some(SymbolKind::Constant),
            "module-level data const must be a Constant"
        );
        assert_eq!(
            node("counter"),
            Some(SymbolKind::Variable),
            "exported `let` is a Variable"
        );
        assert_eq!(
            node("localOnly"),
            None,
            "function-local const must NOT be extracted"
        );
    }

    #[test]
    fn extracts_cast_wrapped_and_parenthesized_arrow_const() {
        // zustand's public API style: the arrow is parenthesized and/or wrapped
        // in a TS type assertion, so the declarator value is an `as_expression`
        // / `parenthesized_expression`, not a bare arrow. These MUST still be
        // extracted as Function symbols, otherwise the public API (createStore,
        // create, …) is invisible and nothing resolves to it.
        let src = "export const createStore = ((createState) => createState) as CreateStore;\n\
                   export const wrapped = ((x) => x);\n\
                   export const chained = (() => 1) as unknown as Foo;\n\
                   export const satisfied = ((y) => y) satisfies Bar;\n\
                   export const notAFn = 5 as number;\n\
                   export const aliasOnly = impl as unknown as Persist;\n";
        let mut g = SymbolGraph::new();
        TypeScriptExtractor::new()
            .extract(std::path::Path::new("vanilla.ts"), src, &mut g)
            .unwrap();
        let is_fn = |name: &str| {
            g.nodes()
                .any(|n| n.name == name && n.kind == SymbolKind::Function)
        };
        let names: Vec<_> = g
            .nodes()
            .map(|n| (n.name.clone(), n.kind.clone()))
            .collect();
        assert!(
            is_fn("createStore"),
            "cast-wrapped arrow createStore missing: {names:?}"
        );
        assert!(
            is_fn("wrapped"),
            "parenthesized arrow `wrapped` missing: {names:?}"
        );
        assert!(
            is_fn("chained"),
            "`as unknown as` chained arrow missing: {names:?}"
        );
        assert!(
            is_fn("satisfied"),
            "`satisfies`-wrapped arrow missing: {names:?}"
        );
        // A non-function const (`5 as number`) must NOT become a Function.
        assert!(
            !is_fn("notAFn"),
            "non-function const wrongly extracted as Function: {names:?}"
        );
        // `impl as unknown as Persist` is an alias to an identifier, not a
        // function definition — it must NOT be a Function symbol here.
        assert!(
            !is_fn("aliasOnly"),
            "identifier alias wrongly extracted as Function: {names:?}"
        );
    }

    #[test]
    fn extracts_function_expression_const() {
        // `const x = function () {}` (classic function expression) is the same
        // case as an arrow const and must also be a Function symbol.
        let src = "const legacy = function () { return 1; };\n";
        let mut g = SymbolGraph::new();
        TypeScriptExtractor::new()
            .extract(std::path::Path::new("a.ts"), src, &mut g)
            .unwrap();
        assert!(
            g.nodes()
                .any(|n| n.name == "legacy" && n.kind == SymbolKind::Function),
            "function-expression const not extracted as Function"
        );
    }

    #[test]
    fn exported_arrow_const_visibility_is_public() {
        // An exported arrow-const is public API; a non-exported one stays
        // Unknown (name-shape heuristic decides). The declarator sits two
        // levels under the export_statement, so visibility must walk up.
        let src = "export const api = () => {};\nconst internal = () => {};\n";
        let mut g = SymbolGraph::new();
        TypeScriptExtractor::new()
            .extract(std::path::Path::new("a.ts"), src, &mut g)
            .unwrap();
        let vis = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.visibility);
        assert_eq!(
            vis("api"),
            Some(Visibility::Public),
            "exported arrow-const is public"
        );
        assert_eq!(
            vis("internal"),
            Some(Visibility::Unknown),
            "non-exported arrow-const stays Unknown"
        );
    }

    #[test]
    fn typescript_visibility_detected() {
        // Exported top-level declarations are public API; class members carry an
        // explicit accessibility modifier; a non-exported top-level declaration
        // stays Unknown so the name-shape heuristic decides.
        let src = "export class Svc {\n  private secret(): void {}\n  public run(): void {}\n}\nclass Internal {}\n";
        let mut g = SymbolGraph::new();
        TypeScriptExtractor::new()
            .extract(std::path::Path::new("svc.ts"), src, &mut g)
            .unwrap();
        let vis = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.visibility);
        assert_eq!(
            vis("Svc"),
            Some(Visibility::Public),
            "exported class is public"
        );
        assert_eq!(vis("secret"), Some(Visibility::Private), "private method");
        assert_eq!(vis("run"), Some(Visibility::Public), "public method");
        assert_eq!(
            vis("Internal"),
            Some(Visibility::Unknown),
            "non-exported top-level"
        );
    }

    #[test]
    fn typescript_extensions() {
        let extractor = TypeScriptExtractor::new();
        let exts = extractor.file_extensions();
        assert!(exts.contains(&"ts"), "ts extension missing");
    }

    #[test]
    fn tsx_is_a_typescript_extension() {
        // .tsx files must route to this extractor, otherwise every React/TSX
        // symbol (and every edge into a .tsx consumer) is silently absent.
        assert!(
            TypeScriptExtractor::new()
                .file_extensions()
                .contains(&"tsx"),
            "tsx extension missing"
        );
    }

    #[test]
    fn extracts_component_and_following_decl_from_tsx() {
        // A function returning JSX, followed by another declaration. The plain
        // TypeScript grammar mis-parses the JSX; the TSX grammar parses both.
        let src = "export function App() {\n  return <div className=\"box\">{label()}</div>;\n}\nexport function label(): string {\n  return \"hi\";\n}\n";
        let mut graph = SymbolGraph::new();
        TypeScriptExtractor::new()
            .extract(std::path::Path::new("App.tsx"), src, &mut graph)
            .expect("extract");
        let names: Vec<String> = graph.nodes().map(|n| n.name.clone()).collect();
        assert!(
            names.iter().any(|n| n == "App"),
            "App component not found: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "label"),
            "label declaration after JSX not found: {names:?}"
        );
    }

    #[test]
    fn call_refs_still_work_and_type_position_refs_captured() {
        // Also guards that the refs query still COMPILES: a bad node name would
        // make tree_sitter_extract_refs silently return nothing, so asserting
        // the call ref survives catches a broken query.
        let src = "type Config = { n: number };\nfunction use(c: Config) {\n  build();\n}\n";
        let refs = TypeScriptExtractor::new().extract_references(std::path::Path::new("a.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "call ref lost (refs query broken?): {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Config" && r.kind == ReferenceKind::TypeUse),
            "type-position reference to Config not captured: {refs:?}"
        );
    }

    #[test]
    fn interface_extends_and_type_alias_refs_captured() {
        // `interface A extends Base` and `type Alias = Target` reference Base /
        // Target — precise type-position edges (no broad member capture).
        let src = "interface Child extends Parent {}\ntype Alias = Target;\n";
        let refs = TypeScriptExtractor::new().extract_references(std::path::Path::new("a.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "Parent" && r.kind == ReferenceKind::TypeUse),
            "interface extends Parent not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Target" && r.kind == ReferenceKind::TypeUse),
            "type-alias RHS Target not captured: {refs:?}"
        );
    }

    #[test]
    fn barrel_reexport_captured() {
        // `export { Foo } from './foo'` re-exports Foo (a real use). Guards the
        // query still compiles (call ref survives) and the re-export is captured.
        let src = "export { Widget } from './widget';\nexport function helper() { build(); }\n";
        let refs =
            TypeScriptExtractor::new().extract_references(std::path::Path::new("index.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "call ref lost (refs query broken?): {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Widget" && r.kind == ReferenceKind::Import),
            "barrel re-export of Widget not captured: {refs:?}"
        );
    }

    #[test]
    fn as_cast_and_implements_type_refs_captured() {
        // `as Widget` cast and `implements Renderable` both reference a type and
        // should produce TypeUse refs (guards the heritage/cast query patterns
        // compile against the grammar).
        let src =
            "class C implements Renderable {}\nfunction f(x: unknown) { return x as Widget; }\n";
        let refs = TypeScriptExtractor::new().extract_references(std::path::Path::new("a.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "Widget" && r.kind == ReferenceKind::TypeUse),
            "`as Widget` cast type ref not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Renderable" && r.kind == ReferenceKind::TypeUse),
            "`implements Renderable` type ref not captured: {refs:?}"
        );
    }

    #[test]
    fn tsx_resolves_call_inside_jsx_expression() {
        // A call inside a JSX expression container `{label()}`. The plain TS
        // grammar treats the JSX as an error node and loses the inner call; the
        // TSX grammar exposes it as a normal call_expression. This edge is what
        // connects a .tsx consumer to the symbols it uses.
        let src =
            "export function App() {\n  return <div>{label()}</div>;\n}\nfunction label() {}\n";
        let refs =
            TypeScriptExtractor::new().extract_references(std::path::Path::new("App.tsx"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "label" && r.kind == ReferenceKind::Call),
            "call to label() inside JSX not extracted: {refs:?}"
        );
    }

    #[test]
    fn jsx_element_reference_captured_as_call() {
        // A component rendered via JSX (`<Icon/>`, `<Panel>…</Panel>`, nested
        // `<Child/>`) is a real use of that component. Without this, components
        // used only in markup (Storybook stories, page trees) look like dead
        // code. Also asserts an existing call ref survives, proving the refs
        // query still compiles (a broken query silently returns nothing).
        let src = "export function Page() {\n  build();\n  return <Icon />;\n}\nexport function Tree() {\n  return <Wrapper><Child /></Wrapper>;\n}\n";
        let refs =
            TypeScriptExtractor::new().extract_references(std::path::Path::new("Page.tsx"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "Icon" && r.kind == ReferenceKind::Call),
            "self-closing JSX element <Icon/> not captured as Call: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Wrapper" && r.kind == ReferenceKind::Call),
            "opening JSX element <Wrapper> not captured as Call: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Child" && r.kind == ReferenceKind::Call),
            "nested JSX element <Child/> not captured as Call: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "existing call ref lost (refs query broken?): {refs:?}"
        );
    }

    #[test]
    fn function_value_references_captured_as_call() {
        // A function passed by name — as a destructured default
        // (`{ decode = defaultDecode }`), a parameter default (`cb = noop`), or a
        // callback argument (`subscribe(snapshot)`) — is a real use. Without this,
        // helpers only ever passed by reference look like dead code. Also asserts
        // an existing call ref survives, proving the refs query still compiles.
        let src = "function defaultDecode() {}\nfunction noop() {}\nfunction snapshot() {}\nexport function make(config: any) {\n  const { decode = defaultDecode } = config;\n  build();\n  subscribe(snapshot);\n}\nexport function run(cb = noop) {\n  cb();\n}\n";
        let refs = TypeScriptExtractor::new().extract_references(std::path::Path::new("a.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "defaultDecode" && r.kind == ReferenceKind::Call),
            "destructured default `decode = defaultDecode` not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "noop" && r.kind == ReferenceKind::Call),
            "parameter default `cb = noop` not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "snapshot" && r.kind == ReferenceKind::Call),
            "callback argument `subscribe(snapshot)` not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "existing call ref lost (refs query broken?): {refs:?}"
        );
    }

    #[test]
    fn value_position_references_captured() {
        // Reading a module const through a subscript (`OBJ[key]`), a member
        // access (`obj.prop`), or a method receiver (`set.has(x)`) is a real
        // use. Without these, top-level consts used only in value position look
        // like dead code (the dominant TS orphan false-positive class). Each is
        // a ValueRef (a References edge), not a Call. A call ref must survive to
        // prove the refs query still compiles.
        let src = concat!(
            "const k = MODE_VARIANTS[mode];\n",
            "const v = authClient.session;\n",
            "AUTH_ROUTES.has(path);\n",
            "build();\n",
        );
        let refs = TypeScriptExtractor::new().extract_references(std::path::Path::new("a.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "MODE_VARIANTS" && r.kind == ReferenceKind::ValueRef),
            "subscript object `MODE_VARIANTS[..]` not captured as ValueRef: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "authClient" && r.kind == ReferenceKind::ValueRef),
            "member-read object `authClient.session` not captured as ValueRef: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "AUTH_ROUTES" && r.kind == ReferenceKind::ValueRef),
            "method receiver `AUTH_ROUTES.has(..)` not captured as ValueRef: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "existing call ref lost (refs query broken?): {refs:?}"
        );
    }

    #[test]
    fn jsx_prop_value_reference_captured() {
        // A module const passed into a JSX prop (`durationInFrames={FRAMES}`) is
        // a real use — common with framework component props (Remotion, charts).
        // Captured as ValueRef (.tsx only). A JSX element ref must survive to
        // prove the appended JSX query still compiles. A string-valued attribute
        // must NOT produce a reference.
        let src = "export function Hero() {\n  return <Player durationInFrames={DURATION_FRAMES} fps={FPS} label=\"x\" />;\n}\n";
        let refs =
            TypeScriptExtractor::new().extract_references(std::path::Path::new("Hero.tsx"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "DURATION_FRAMES" && r.kind == ReferenceKind::ValueRef),
            "JSX prop value `DURATION_FRAMES` not captured as ValueRef: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "FPS" && r.kind == ReferenceKind::ValueRef),
            "JSX prop value `FPS` not captured as ValueRef: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Player" && r.kind == ReferenceKind::Call),
            "JSX element `<Player>` lost (JSX query broken?): {refs:?}"
        );
        assert!(
            !refs.iter().any(|r| r.name == "label" || r.name == "x"),
            "string-valued JSX attribute must not produce a reference: {refs:?}"
        );
    }

    #[test]
    fn composite_type_references_captured() {
        // Type identifiers referenced inside conditional types
        // (`K extends Constraint ? Then : Else`), intersections (`A & B`) and
        // unions (`A | B`). These are the type-level edges the precise base
        // patterns missed, so the referenced types looked like dead code. All
        // should be TypeUse; a call ref must survive to prove the query compiles.
        let src = concat!(
            "type CrudOperationName = string;\n",
            "interface DerivedListHook {}\n",
            "interface DerivedQueryHook {}\n",
            "interface Base {}\n",
            "interface EntityHasListRole {}\n",
            "interface Active {}\n",
            "interface Archived {}\n",
            "type ResolveHook<K> = K extends CrudOperationName ? DerivedListHook : DerivedQueryHook;\n",
            "type EntityHooks = Base & EntityHasListRole;\n",
            "type Status = Active | Archived;\n",
            "function go() { build(); }\n",
        );
        let refs =
            TypeScriptExtractor::new().extract_references(std::path::Path::new("types.ts"), src);
        let is_type = |n: &str| {
            refs.iter()
                .any(|r| r.name == n && r.kind == ReferenceKind::TypeUse)
        };
        assert!(
            is_type("CrudOperationName"),
            "conditional-type constraint not captured: {refs:?}"
        );
        assert!(
            is_type("DerivedListHook"),
            "conditional-type consequence not captured: {refs:?}"
        );
        assert!(
            is_type("DerivedQueryHook"),
            "conditional-type alternative not captured: {refs:?}"
        );
        assert!(
            is_type("EntityHasListRole"),
            "intersection-type member not captured: {refs:?}"
        );
        assert!(
            is_type("Active"),
            "union-type member not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "existing call ref lost (refs query broken?): {refs:?}"
        );
    }

    #[test]
    fn generic_type_name_captured_as_type_use() {
        // `Box<Item>` references the generic type `Box`, not only its argument
        // `Item`. Generic-heavy type machinery (`EntityHookFor<X>`,
        // `EntityHasListRole<T>`) needs the generic-name edge or the type looks
        // dead. Both name and argument are TypeUse; a call ref guards compilation.
        let src = "interface Box<T> { v: T; }\ninterface Item {}\ntype Wrapped = Box<Item>;\nfunction go() { build(); }\n";
        let refs =
            TypeScriptExtractor::new().extract_references(std::path::Path::new("types.ts"), src);
        let is_type = |n: &str| {
            refs.iter()
                .any(|r| r.name == n && r.kind == ReferenceKind::TypeUse)
        };
        assert!(
            is_type("Box"),
            "generic-type name `Box<…>` not captured: {refs:?}"
        );
        assert!(
            is_type("Item"),
            "generic-type argument `Item` not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "existing call ref lost (refs query broken?): {refs:?}"
        );
    }

    #[test]
    fn ternary_value_operands_captured_as_call() {
        // A function passed by name through a value-level ternary
        // (`cond ? onYes : emptyGet`, e.g. `useSyncExternalStore(p ? p.get :
        // emptyGet, …)`) is a real use of each branch. Without this the helper
        // only ever reached via a ternary branch looks dead.
        let src = "function onYes() {}\nfunction emptyGet() {}\nexport function pick(cond: boolean) {\n  build();\n  return cond ? onYes : emptyGet;\n}\n";
        let refs = TypeScriptExtractor::new().extract_references(std::path::Path::new("a.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "onYes" && r.kind == ReferenceKind::Call),
            "ternary consequence `onYes` not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "emptyGet" && r.kind == ReferenceKind::Call),
            "ternary alternative `emptyGet` not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "existing call ref lost (refs query broken?): {refs:?}"
        );
    }

    #[test]
    fn export_star_captured_as_reexport_all() {
        // `export * from './mod'` yields a ReexportAll ref carrying the module
        // specifier; `export { X } from` and a normal call must be unaffected
        // (guards both the new pattern and that the query still compiles).
        let src = "export * from './widget';\nexport { Named } from './named';\nfunction f() { build(); }\n";
        let refs =
            TypeScriptExtractor::new().extract_references(std::path::Path::new("index.ts"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "./widget" && r.kind == ReferenceKind::ReexportAll),
            "`export *` specifier not captured as ReexportAll: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Named" && r.kind == ReferenceKind::Import),
            "`export {{ Named }} from` must still be an Import: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "build" && r.kind == ReferenceKind::Call),
            "existing call ref lost (refs query broken?): {refs:?}"
        );
    }

    #[test]
    fn jsdoc_is_paired_with_function() {
        let src = "/** Greets. */\nexport function greet() {}\n";
        let refs =
            TypeScriptExtractor::new().extract_doc_comments(std::path::Path::new("g.ts"), src);
        assert_eq!(refs.len(), 1, "JSDoc must pair with the function");
    }

    #[test]
    fn plain_block_comment_is_not_jsdoc() {
        let refs = TypeScriptExtractor::new().extract_doc_comments(
            std::path::Path::new("g.ts"),
            "/* note */\nexport function greet() {}\n",
        );
        assert!(refs.is_empty(), "a plain /* */ comment is not JSDoc");
    }
}
