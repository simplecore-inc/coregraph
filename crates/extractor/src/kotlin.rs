// KotlinExtractor uses regex-based pattern matching to produce symbol nodes.
// Cross-file name resolution is handled separately by the stack-graphs backend
// on the `tree-sitter-kotlin-ng` grammar (crates/stack/rules/kotlin.tsg); the
// resolution spans align with these regex-derived symbol spans well enough for
// the resolved edges to attach.

use std::path::Path;
use std::sync::LazyLock;

use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
use coregraph_graph::SymbolGraph;

use crate::{ExtractError, RawReference, ReferenceKind, SymbolExtractor};

static RE_CLASS: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?:^|\s)class\s+(\w+)").expect("valid regex"));
static RE_INTERFACE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?:^|\s)interface\s+(\w+)").expect("valid regex"));
// Matches `fun name(` plus the two forms the old `fun (\w+)\(` regex dropped:
//   - generic: `fun <T> name(`        (optional `<…>` type-parameter list)
//   - extension: `fun Recv.name(`     (optional receiver, incl. generic
//                                       receivers like `Iterable<T>.`)
// The receiver group is greedy up to the last `.`, so capture group 1 is always
// the declared function name (the final identifier before `(`).
static RE_FUN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?:^|\s)fun\s+(?:<[^>]*>\s*)?(?:[\w.<>,?\s]+\.)?(\w+)\s*\(")
        .expect("valid regex")
});
static RE_OBJECT: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?:^|\s)object\s+(\w+)").expect("valid regex"));
// A call site: an identifier immediately followed by `(`. Capture group 1 is
// the callee name; its start offset locates the enclosing caller during
// reference resolution.
static RE_CALL: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(\w+)\s*\(").expect("valid regex"));
// A type-position reference: a PascalCase identifier right after `:` (a val /
// param / return type or a supertype in `class A : B`) or after `<` (a generic
// argument). Kotlin types are PascalCase while values are camelCase, so this
// targets type usages with low noise. Connects a class/interface used only as a
// type so it is not reported as a false orphan. Same regex-based, best-effort
// approach as the symbol/call extraction above.
static RE_TYPE_REF: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"[:<]\s*([A-Z]\w*)").expect("valid regex"));

// Control-flow / soft keywords that can precede `(` but are not call sites
// (`if (...)`, `when (...)`, `super(...)`, `constructor(...)`). They are never
// symbol names, so leaving them in would only be dropped during resolution —
// filtering here keeps the reference list honest.
const NON_CALL_KEYWORDS: &[&str] = &[
    "if",
    "else",
    "while",
    "for",
    "when",
    "catch",
    "do",
    "return",
    "throw",
    "try",
    "is",
    "as",
    "in",
    "by",
    "super",
    "this",
    "init",
    "constructor",
    "synchronized",
];

// Keywords that introduce a *definition* whose name is followed by `(`
// (`fun greet(`, `class Foo(`). The captured identifier is being declared, not
// called, so it must not become a Call reference.
const DECL_KEYWORDS: &[&str] = &["fun", "class", "object", "interface"];

/// Scans a Kotlin `enum class` body and returns its entries as
/// `(name, start_byte, end_byte)`. Entries are identifiers at the top level of
/// the body, before the first top-level `;` (which separates entries from
/// member declarations), each optionally followed by `(…)` constructor args or
/// a `{…}` entry body. The scan is depth-aware so commas/identifiers nested in
/// those are ignored.
fn extract_enum_entries(source: &str, class_line_offset: usize) -> Vec<(String, u32, u32)> {
    let bytes = source.as_bytes();
    // Locate the enum body's opening brace from the class declaration onward.
    let mut i = class_line_offset;
    while i < bytes.len() && bytes[i] != b'{' {
        i += 1;
    }
    if i >= bytes.len() {
        return Vec::new();
    }
    i += 1; // step past the body '{'
    let mut depth: i32 = 1; // inside the body
    let mut expect_entry = true; // an entry name is expected at body start / after a top-level ','
    let mut entries = Vec::new();
    while i < bytes.len() && depth > 0 {
        let c = bytes[i] as char;
        match c {
            '{' | '(' | '[' => {
                depth += 1;
                i += 1;
            }
            '}' | ')' | ']' => {
                depth -= 1;
                i += 1;
            }
            ';' if depth == 1 => break, // entry section ends; members follow
            ',' if depth == 1 => {
                expect_entry = true;
                i += 1;
            }
            c if depth == 1 && expect_entry && (c.is_alphabetic() || c == '_') => {
                let start = i;
                while i < bytes.len() && ((bytes[i] as char).is_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                entries.push((source[start..i].to_string(), start as u32, i as u32));
                expect_entry = false;
            }
            c if c.is_whitespace() => i += 1,
            _ => {
                if depth == 1 {
                    expect_entry = false;
                }
                i += 1;
            }
        }
    }
    entries
}

pub struct KotlinExtractor;

impl KotlinExtractor {
    pub fn new() -> Self {
        KotlinExtractor
    }
}

impl Default for KotlinExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor for KotlinExtractor {
    fn language_name(&self) -> &'static str {
        "kotlin"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["kt", "kts"]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        // Blank comments and string/char literals (offset-preserving) so the
        // declaration regexes never match a keyword that appears in prose or a
        // literal (e.g. `class for` in a KDoc → phantom `for` symbol). Byte
        // offsets are unchanged, so the spans below remain valid for `source`.
        let scrubbed = blank_comments_and_strings(source);
        let source: &str = &scrubbed;

        // Track brace depth to distinguish top-level vs. class-member functions.
        let mut brace_depth: i32 = 0;
        // Byte offset of the current line start.
        let mut line_offset: usize = 0;

        for line in source.lines() {
            // Check for class/interface/object before counting braces on this line.
            if let Some(cap) = RE_CLASS.captures(line) {
                let name = cap[1].to_string();
                let prefix = &line[..cap.get(1).map_or(0, |m| m.start())];
                let visibility = kotlin_visibility(prefix);
                let start = line_offset as u32;
                // Brace-match from this line forward for the full class
                // body span. Without this `enclosing_symbol` can't resolve
                // refs inside class methods to the enclosing class.
                let end = body_end_offset(source, line_offset);
                // `enum class` entries are members; emit each as an
                // `EnumName.ENTRY` EnumVariant (same convention as TS) so the
                // cross-enum-mismatch detector can compare them.
                if prefix.split_whitespace().any(|w| w == "enum") {
                    for (entry, es, ee) in extract_enum_entries(source, line_offset) {
                        graph.insert_node(
                            SymbolNode::new(
                                SymbolId(0),
                                SymbolKind::EnumVariant,
                                format!("{name}.{entry}"),
                                path,
                                es,
                                ee,
                            )
                            .with_visibility(visibility),
                        );
                    }
                }
                graph.insert_node(
                    SymbolNode::new(SymbolId(0), SymbolKind::Class, name, path, start, end)
                        .with_visibility(visibility),
                );
            } else if let Some(cap) = RE_INTERFACE.captures(line) {
                let name = cap[1].to_string();
                let visibility = kotlin_visibility(&line[..cap.get(1).map_or(0, |m| m.start())]);
                let start = line_offset as u32;
                let end = body_end_offset(source, line_offset);
                graph.insert_node(
                    SymbolNode::new(SymbolId(0), SymbolKind::Interface, name, path, start, end)
                        .with_visibility(visibility),
                );
            } else if let Some(cap) = RE_OBJECT.captures(line) {
                let name = cap[1].to_string();
                let visibility = kotlin_visibility(&line[..cap.get(1).map_or(0, |m| m.start())]);
                let start = line_offset as u32;
                let end = body_end_offset(source, line_offset);
                graph.insert_node(
                    SymbolNode::new(SymbolId(0), SymbolKind::Module, name, path, start, end)
                        .with_visibility(visibility),
                );
            } else if let Some(cap) = RE_FUN.captures(line) {
                // fun inside a class body (brace_depth > 0) → Method; top-level → Function
                let kind = if brace_depth > 0 {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                let name = cap[1].to_string();
                let visibility = kotlin_visibility(&line[..cap.get(1).map_or(0, |m| m.start())]);
                let start = line_offset as u32;
                let end = body_end_offset(source, line_offset);
                graph.insert_node(
                    SymbolNode::new(SymbolId(0), kind, name, path, start, end)
                        .with_visibility(visibility),
                );
            }

            // Update brace depth for subsequent lines.
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }

            // +1 for the newline character consumed by lines().
            line_offset += line.len() + 1;
        }

        Ok(())
    }

    // Best-effort regex call-site extraction, matching the regex design of
    // `extract` above. These are SyntaxMatched (≈0.85) edges — the precise
    // cross-file layer is handled by stack-graphs (NameResolved 0.95). Known
    // limitations: generic-call syntax `foo<T>()` is not matched, and an
    // identifier inside a comment or string may be captured. Such captures are
    // harmless: resolution only emits an edge when the name matches a defined
    // project symbol and drops self-edges.
    fn extract_references(&self, _path: &Path, source: &str) -> Vec<RawReference> {
        let mut refs: Vec<RawReference> = RE_CALL
            .captures_iter(source)
            .filter_map(|cap| {
                let m = cap.get(1)?;
                let name = m.as_str();
                if NON_CALL_KEYWORDS.contains(&name) {
                    return None;
                }
                if is_declaration_or_annotation(source, m.start()) {
                    return None;
                }
                Some(RawReference {
                    name: name.to_string(),
                    kind: ReferenceKind::Call,
                    byte_offset: m.start() as u32,
                })
            })
            .collect();
        // Type-position references (PascalCase after `:` or `<`).
        for cap in RE_TYPE_REF.captures_iter(source) {
            if let Some(m) = cap.get(1) {
                refs.push(RawReference {
                    name: m.as_str().to_string(),
                    kind: ReferenceKind::TypeUse,
                    byte_offset: m.start() as u32,
                });
            }
        }
        refs
    }
}

/// Kotlin declared visibility, read from the modifiers preceding the
/// declaration keyword on the same line (`prefix` is the line text up to the
/// declared name). Kotlin's default — no modifier — is public; `private` /
/// `protected` / `internal` lower it (`internal` is module-scoped, not external
/// API, so it maps to Private in the public/private/unknown model). Annotations
/// and non-visibility modifiers (`open`, `data`, `suspend`, …) are ignored.
fn kotlin_visibility(prefix: &str) -> Visibility {
    for tok in prefix.split_whitespace() {
        match tok {
            "private" | "protected" | "internal" => return Visibility::Private,
            "public" => return Visibility::Public,
            _ => {}
        }
    }
    Visibility::Public
}

/// True when the identifier starting at `ident_start` is a definition name
/// (preceded by a `DECL_KEYWORDS` keyword) or an annotation use (immediately
/// preceded by `@`) — neither is a call site. Walks characters (not bytes)
/// backward so it stays correct on Unicode identifiers.
fn is_declaration_or_annotation(source: &str, ident_start: usize) -> bool {
    let before = &source[..ident_start];
    // Annotation use: `@Foo(` — `@` directly adjacent to the identifier.
    if before.ends_with('@') {
        return true;
    }
    // Preceding word: the trailing run of identifier characters after skipping
    // any whitespace between it and the identifier.
    let prev: String = before
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    DECL_KEYWORDS.contains(&prev.as_str())
}

/// Replace the contents of comments, string literals and char literals with
/// spaces — preserving length and newlines so every byte offset stays valid —
/// so the symbol regexes never match a declaration keyword that appears in
/// prose or a literal (e.g. `class for` inside a KDoc, which produced phantom
/// `for` Class symbols). A best-effort Kotlin scanner matching the rest of this
/// extractor's regex design: it handles `//` line comments, nestable `/* */`
/// (incl. KDoc `/** */`) block comments, `"`/`"""` strings and `'` char
/// literals; it does not need to be a full lexer. As a side benefit, brace
/// counting in `body_end_offset` no longer trips on braces inside comments.
fn blank_comments_and_strings(source: &str) -> String {
    #[derive(PartialEq)]
    enum S {
        Code,
        Line,
        Block,
        Str,
        Raw,
        Char,
    }
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut state = S::Code;
    let mut block_depth: u32 = 0;
    let blank = |b: u8| if b == b'\n' { b'\n' } else { b' ' };
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match state {
            S::Code => {
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = S::Line;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = S::Block;
                    block_depth = 1;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if b == b'"'
                    && i + 2 < bytes.len()
                    && bytes[i + 1] == b'"'
                    && bytes[i + 2] == b'"'
                {
                    state = S::Raw;
                    out.extend_from_slice(b"   ");
                    i += 3;
                } else if b == b'"' {
                    state = S::Str;
                    out.push(b' ');
                    i += 1;
                } else if b == b'\'' {
                    state = S::Char;
                    out.push(b' ');
                    i += 1;
                } else {
                    out.push(b);
                    i += 1;
                }
            }
            S::Line => {
                if b == b'\n' {
                    state = S::Code;
                    out.push(b'\n');
                } else {
                    out.push(b' ');
                }
                i += 1;
            }
            S::Block => {
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    block_depth += 1;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    block_depth -= 1;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    if block_depth == 0 {
                        state = S::Code;
                    }
                } else {
                    out.push(blank(b));
                    i += 1;
                }
            }
            S::Str => {
                if b == b'\\' && i + 1 < bytes.len() {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if b == b'"' {
                    state = S::Code;
                    out.push(b' ');
                    i += 1;
                } else {
                    out.push(blank(b));
                    i += 1;
                }
            }
            S::Raw => {
                if b == b'"' && i + 2 < bytes.len() && bytes[i + 1] == b'"' && bytes[i + 2] == b'"'
                {
                    state = S::Code;
                    out.extend_from_slice(b"   ");
                    i += 3;
                } else {
                    out.push(blank(b));
                    i += 1;
                }
            }
            S::Char => {
                if b == b'\\' && i + 1 < bytes.len() {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if b == b'\'' {
                    state = S::Code;
                    out.push(b' ');
                    i += 1;
                } else {
                    out.push(blank(b));
                    i += 1;
                }
            }
        }
    }
    // Only whole ASCII delimiter/content bytes are replaced with ASCII spaces
    // and all other bytes are copied verbatim, so multi-byte UTF-8 sequences in
    // code are preserved and the result is valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| source.to_string())
}

/// Find the byte offset of the closing `}` that terminates the first `{`
/// opened at or after `decl_line_offset`. Returns the line-end offset when
/// the declaration has no body (e.g. `interface Foo` on a line by itself)
/// or when braces don't balance before EOF.
fn body_end_offset(source: &str, decl_line_offset: usize) -> u32 {
    let bytes = source.as_bytes();
    let start = decl_line_offset;
    let mut depth: i32 = 0;
    let mut started = false;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                started = true;
            }
            b'}' => {
                depth -= 1;
                if started && depth == 0 {
                    return (i + 1) as u32;
                }
            }
            _ => {}
        }
        i += 1;
    }
    // No matched brace — fall back to end of the declaration's own line so
    // the span at least covers the header.
    let line_end = source[start..]
        .find('\n')
        .map(|n| start + n)
        .unwrap_or(source.len());
    line_end as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReferenceKind;
    use coregraph_graph::SymbolGraph;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/kotlin-simple")
            .join(name)
    }

    #[test]
    fn extracts_class_from_kotlin() {
        let extractor = KotlinExtractor;
        let path = fixture_path("Example.kt");
        let source = std::fs::read_to_string(&path).expect("read Example.kt");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract");

        let found = graph
            .nodes()
            .any(|n| n.name == "MyService" && n.kind == SymbolKind::Class);
        assert!(found, "expected MyService class node");
    }

    #[test]
    fn extracts_function_from_kotlin() {
        let extractor = KotlinExtractor;
        let path = fixture_path("Example.kt");
        let source = std::fs::read_to_string(&path).expect("read Example.kt");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extract");

        let found = graph
            .nodes()
            .any(|n| n.name == "processData" && n.kind == SymbolKind::Method);
        assert!(found, "expected processData method node");
    }

    #[test]
    fn extracts_extension_and_generic_kotlin_functions() {
        // The function name can be preceded by a generic parameter list and/or a
        // receiver type. These are pervasive in Kotlin DSLs (Exposed defines
        // hundreds of `fun Recv.x(` and `fun <T> x(`). The old `fun (\w+)\(`
        // regex missed all of them.
        let src = "fun FieldSet.selectAll(): Query { return q }\n\
                   fun <T> emptySized(): SizedIterable<T> { return e }\n\
                   fun <T> Iterable<T>.firstOrNull(): T? { return null }\n\
                   fun plain() {}\n";
        let mut g = SymbolGraph::new();
        KotlinExtractor
            .extract(std::path::Path::new("x.kt"), src, &mut g)
            .unwrap();
        let has = |name: &str| {
            g.nodes()
                .any(|n| n.name == name && n.kind == SymbolKind::Function)
        };
        assert!(
            has("selectAll"),
            "extension function `FieldSet.selectAll` missing"
        );
        assert!(
            has("emptySized"),
            "generic function `<T> emptySized` missing"
        );
        assert!(
            has("firstOrNull"),
            "generic extension `<T> Iterable<T>.firstOrNull` missing"
        );
        assert!(has("plain"), "plain function regressed");
    }

    #[test]
    fn extracts_kotlin_enum_entries() {
        // `enum class` entries are members the regex extractor previously
        // ignored. They are stored as `EnumName.ENTRY` EnumVariant nodes (same
        // convention as the TS extractor) so the cross-enum-mismatch detector
        // can compare them. The `;` terminates the entry section — methods
        // after it are NOT entries.
        let src = "enum class ReferenceOption {\n\tCASCADE,\n\tRESTRICT,\n\tSET_NULL;\n\tfun describe() {}\n}\n\
                   enum class Color(val rgb: Int) { RED(0xFF0000), GREEN(0x00FF00) }\n";
        let mut g = SymbolGraph::new();
        KotlinExtractor
            .extract(std::path::Path::new("e.kt"), src, &mut g)
            .unwrap();
        let is_variant = |name: &str| {
            g.nodes()
                .any(|n| n.name == name && n.kind == SymbolKind::EnumVariant)
        };
        assert!(
            is_variant("ReferenceOption.CASCADE"),
            "enum entry CASCADE missing"
        );
        assert!(
            is_variant("ReferenceOption.RESTRICT"),
            "enum entry RESTRICT missing"
        );
        assert!(
            is_variant("ReferenceOption.SET_NULL"),
            "enum entry SET_NULL missing"
        );
        assert!(
            is_variant("Color.RED"),
            "enum entry with ctor args RED missing"
        );
        assert!(is_variant("Color.GREEN"), "enum entry GREEN missing");
        // The method after `;` must not be captured as an enum entry.
        assert!(
            !g.nodes().any(|n| n.name == "ReferenceOption.describe"),
            "method after `;` wrongly captured as enum entry"
        );
    }

    #[test]
    fn kotlin_ignores_keywords_in_comments_and_strings() {
        // A regex extractor must not pick up declaration keywords that appear in
        // prose. `class for` inside a KDoc/line comment and `class Fake` inside a
        // string literal must NOT become phantom symbols (these inflated exposed
        // orphans), while the real declaration is still found.
        let src = "/** Main configuration class for Exposed. */\n// define the main class for the app\nclass RealThing {\n    val s = \"this class Fake is in a string\"\n    fun run() {}\n}\n";
        let mut g = SymbolGraph::new();
        KotlinExtractor
            .extract(std::path::Path::new("a.kt"), src, &mut g)
            .unwrap();
        let names: Vec<_> = g.nodes().map(|n| n.name.clone()).collect();
        assert!(
            names.iter().any(|n| n == "RealThing"),
            "real class must be extracted: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "run"),
            "real fun must be extracted: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "for"),
            "comment `class for` must not yield a symbol: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "Fake"),
            "string `class Fake` must not yield a symbol: {names:?}"
        );
    }

    #[test]
    fn kotlin_visibility_detected() {
        // Kotlin's default visibility is public; private/protected/internal
        // lower it (internal is module-scoped, not external API, so it maps to
        // Private in the public/private/unknown model).
        let src = "class Pub {\n  private fun secret() {}\n  fun open() {}\n}\ninternal class Hidden {}\nprivate fun helper() {}\n";
        let mut g = SymbolGraph::new();
        KotlinExtractor
            .extract(std::path::Path::new("a.kt"), src, &mut g)
            .unwrap();
        let vis = |name: &str| g.nodes().find(|n| n.name == name).map(|n| n.visibility);
        assert_eq!(
            vis("Pub"),
            Some(Visibility::Public),
            "no modifier → public default"
        );
        assert_eq!(vis("secret"), Some(Visibility::Private), "private method");
        assert_eq!(
            vis("open"),
            Some(Visibility::Public),
            "no-modifier method → public"
        );
        assert_eq!(vis("Hidden"), Some(Visibility::Private), "internal class");
        assert_eq!(
            vis("helper"),
            Some(Visibility::Private),
            "private top-level fun"
        );
    }

    #[test]
    fn kotlin_extensions() {
        let extractor = KotlinExtractor;
        assert!(extractor.file_extensions().contains(&"kt"));
    }

    #[test]
    fn extract_references_captures_call() {
        let src = "fun use() {\n    greet()\n}\n\nfun greet() {}\n";
        let refs = KotlinExtractor.extract_references(std::path::Path::new("a.kt"), src);
        let call = refs
            .iter()
            .find(|r| r.name == "greet" && r.kind == ReferenceKind::Call);
        assert!(
            call.is_some(),
            "expected a Call reference to `greet`, got {refs:?}"
        );
    }

    #[test]
    fn extract_references_captures_type_position() {
        // A class used only as a parameter / return / generic type produces a
        // TypeUse ref so it is not a false orphan.
        let src = "fun build(repo: UserRepo): List<Account> {\n    return repo.all()\n}\n";
        let refs = KotlinExtractor.extract_references(std::path::Path::new("a.kt"), src);
        assert!(
            refs.iter()
                .any(|r| r.name == "UserRepo" && r.kind == ReferenceKind::TypeUse),
            "param type UserRepo not captured: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "Account" && r.kind == ReferenceKind::TypeUse),
            "generic type Account not captured: {refs:?}"
        );
    }

    #[test]
    fn extract_references_skips_function_definition() {
        // A bare definition with no call sites must yield zero Call references —
        // the `greet` in `fun greet(` is a declaration, not a call.
        let src = "fun greet() {}\n";
        let refs = KotlinExtractor.extract_references(std::path::Path::new("a.kt"), src);
        assert!(
            refs.is_empty(),
            "a function definition must not be captured as a call, got {refs:?}"
        );
    }
}
