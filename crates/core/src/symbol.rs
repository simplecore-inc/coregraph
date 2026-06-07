use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use crate::file_state::NodeStatus;

/// Stable identifier for a symbol across graph mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SymbolId(pub u64);

/// Source language of a symbol. Used by qualified-name conversion,
/// trust-weighting heuristics, and extractor dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Language {
    Rust,
    Java,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Unknown,
}

impl Language {
    /// Infer the language from a file extension (without the leading dot).
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Language::Rust,
            "java" => Language::Java,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "py" | "pyi" => Language::Python,
            "go" => Language::Go,
            _ => Language::Unknown,
        }
    }

    /// Infer the language from a file path.
    pub fn from_path(path: &Path) -> Self {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(Language::Unknown)
    }
}

/// Classifies what language construct a symbol represents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SymbolKind {
    // Code constructs
    Function,
    Method,
    Class,
    Struct,
    Interface,
    Trait,
    Enum,
    EnumVariant,
    Constant,
    Variable,
    Field,
    TypeAlias,
    Module,
    Namespace,
    // Structural containers (spec §4.2)
    /// A source file as a symbol. Owns Contains edges to everything defined
    /// inside it. Auto-generated at build time.
    File,
    // Config / document constructs
    ConfigKey,
    StringLiteral,
    /// A documentation comment (`///`, `//!`, `/** */`, JSDoc, docstring)
    /// attached to a code symbol. A first-class node so docs are queryable
    /// (e.g. drift detection, "find the doc for X"). See docs/graph-model.md §7.
    DocComment,
    /// A section of an external documentation file (a Markdown heading and its
    /// body). Linked to the code symbols it describes via `DescribedIn` edges.
    /// See docs/graph-model.md §7.6.
    DocSection,
    // Package / manifest constructs
    Package,
    ExternalPackage,
}

/// Declared visibility of a symbol, when the extractor can determine it from a
/// language keyword (`pub`, `export`, `public`/`private`, Kotlin modifiers).
/// `Unknown` means the extractor did not set it — consumers then fall back to a
/// name-shape heuristic (correct for Go/Python, where the *name* is the
/// visibility convention). The root-cause replacement for guessing visibility
/// purely from the name shape.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Visibility {
    /// Exported / public API surface.
    Public,
    /// Module- or type-private.
    Private,
    /// Not determined by the extractor; consumers use a name heuristic.
    #[default]
    Unknown,
}

/// A node in the symbol graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SymbolNode {
    pub id: SymbolId,
    pub kind: SymbolKind,
    /// Local / display name (e.g. "getUser")
    pub name: String,
    /// Fully-qualified name for cross-file / cross-language identity.
    /// Defaults to `name` when the extractor cannot derive a richer form.
    /// Examples: Java `com.example.UserController.getUser`,
    /// TypeScript `@scope/pkg.Foo.bar`, Python `foo.bar.Baz.qux`.
    #[serde(default)]
    pub qualified_name: String,
    /// Source file path (relative to project root).
    ///
    /// Stored as `Arc<Path>` so the graph can intern it: every symbol defined
    /// in the same file shares one allocation instead of holding a private
    /// copy. The field also shrinks from a 24-byte `PathBuf` to a 16-byte
    /// fat pointer.
    #[serde(with = "crate::arc_path")]
    pub file: Arc<Path>,
    /// Byte offset of the symbol definition start in the file
    pub span_start: u32,
    /// Byte offset of the symbol definition end in the file
    pub span_end: u32,
    /// Life-cycle state. Defaults to `Verified` for backwards-compatible
    /// snapshot deserialization.
    #[serde(default)]
    pub status: NodeStatus,
    /// Declared visibility when the extractor set it from a language keyword;
    /// `Unknown` (the default) means consumers fall back to a name heuristic.
    /// `#[serde(default)]` keeps old snapshots loadable.
    #[serde(default)]
    pub visibility: Visibility,
    /// True when the extractor recognised this as test code from an attribute
    /// or convention (`#[test]`/`#[cfg(test)]`, JUnit `@Test`, pytest `test_*`),
    /// even though the file is not under a test-path directory. Lets dead-code
    /// analysis treat inline test functions as test code rather than orphans.
    /// `#[serde(default)]` (false) keeps old snapshots loadable.
    #[serde(default)]
    pub is_test: bool,
}

impl SymbolNode {
    pub fn new(
        id: SymbolId,
        kind: SymbolKind,
        name: impl Into<String>,
        file: impl Into<PathBuf>,
        span_start: u32,
        span_end: u32,
    ) -> Self {
        let name = name.into();
        Self {
            id,
            kind,
            qualified_name: name.clone(),
            name,
            file: Arc::from(file.into()),
            span_start,
            span_end,
            status: NodeStatus::Verified,
            visibility: Visibility::Unknown,
            is_test: false,
        }
    }

    /// Override the qualified name. Used by extractors that can derive a
    /// richer identity (e.g. `com.example.Foo.bar`) than the local identifier.
    pub fn with_qualified_name(mut self, qualified_name: impl Into<String>) -> Self {
        self.qualified_name = qualified_name.into();
        self
    }

    /// Override the node status (used by invalidation to mark Stale/Gone).
    pub fn with_status(mut self, status: NodeStatus) -> Self {
        self.status = status;
        self
    }

    /// Set the declared visibility (used by extractors that detect a `pub` /
    /// `export` / `public` keyword).
    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }

    /// Mark this symbol as test code (extractor recognised a test attribute /
    /// convention). See the `is_test` field.
    pub fn with_test(mut self, is_test: bool) -> Self {
        self.is_test = is_test;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_equality() {
        let a = SymbolId(1);
        let b = SymbolId(1);
        let c = SymbolId(2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn symbol_node_construction() {
        let node = SymbolNode::new(
            SymbolId(42),
            SymbolKind::Function,
            "my_func",
            PathBuf::from("src/main.rs"),
            0,
            100,
        );
        assert_eq!(node.id, SymbolId(42));
        assert_eq!(node.kind, SymbolKind::Function);
        assert_eq!(node.name, "my_func");
    }

    #[test]
    fn symbol_node_serde_roundtrip() {
        let node = SymbolNode::new(
            SymbolId(1),
            SymbolKind::Class,
            "MyClass",
            PathBuf::from("src/lib.rs"),
            10,
            200,
        );
        let json = serde_json::to_string(&node).expect("serialize");
        let back: SymbolNode = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.name, "MyClass");
    }
}
