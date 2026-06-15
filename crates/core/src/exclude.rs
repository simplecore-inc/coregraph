//! Shared default exclusion patterns.
//!
//! Both the indexer (`coregraph-extractor`) and the analysis-surface
//! excluder (`coregraph-query`) build their gitignore matchers from this
//! single source of truth, so the set of directories skipped at index time
//! can never drift from the set skipped during analysis.

/// Universal exclude patterns applied to every project unconditionally.
/// Covers only the categories that are truly language/tool-agnostic:
/// 1. VCS metadata (`.git/`)
/// 2. Build outputs that every major ecosystem writes outside the
///    source tree (`target/`, `build/`, `dist/`, `out/`)
/// 3. Dependency caches (`node_modules/`, `.gradle/`, `vendor/`,
///    `__pycache__/`, `.venv/`, `venv/`)
/// 4. IDE / editor workspace folders (`.idea/`, `.vscode/`)
///
/// Each top-level entry is duplicated with a `**/` prefix so the
/// pattern matches nested occurrences too (e.g. monorepos that have
/// `apps/foo/target/` in addition to a root-level `target/`).
///
/// Callers who need a project whose source lives inside one of these
/// directories can add an un-ignore pattern (`!target/my-subtree/`)
/// to their own config; the builder honors negations.
pub const DEFAULT_EXCLUDE_PATTERNS: &[&str] = &[
    // VCS
    ".git/",
    // IDE / editor
    ".idea/",
    ".vscode/",
    // Build outputs — language-generic names common to many ecosystems
    "target/",
    "**/target/",
    "build/",
    "**/build/",
    "dist/",
    "**/dist/",
    "out/",
    "**/out/",
    // Dependency directories
    "node_modules/",
    "**/node_modules/",
    ".gradle/",
    "**/.gradle/",
    "vendor/",
    "**/vendor/",
    "__pycache__/",
    "**/__pycache__/",
    ".venv/",
    "**/.venv/",
    "venv/",
    "**/venv/",
];
