//! Pluggable cross-file name-resolution backends.
//!
//! Two implementations:
//!
//! - [`SyntacticBackend`]: identifier-matching fallback. Works for
//!   every language but ignores scopes and types. Produces
//!   `ResolutionResult { success: false, … }` so the downstream edge
//!   evaluator marks the result as `SyntaxMatched` (0.85 confidence).
//!
//! - [`StackGraphsBackend`]: real name resolution via
//!   `tree-sitter-stack-graphs` for Java / TypeScript / JavaScript /
//!   Python. Files in an unsupported language pass through to the
//!   syntactic backend so the pipeline remains language-agnostic.
//!   Produces `ResolutionResult { success: true, … }` for files it
//!   fully processed → edges land as `NameResolved` (0.95).
//!
//! Each language pass runs under a wall-clock budget; if stack-graphs
//! exceeds it the backend records nothing for that file and the caller
//! falls back to the syntactic result. Same timeout pattern
//! `QueryHealer` uses in `coregraph-graph`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::resolver::{resolve_files_with_graph, ResolutionResult, ResolvedRef};
use coregraph_core::edge::AnalysisOrigin;
use coregraph_graph::SymbolGraph;

/// Hand-authored stack-graphs rules for Go (no upstream package exists). Built
/// into a `LanguageConfiguration` via `from_sources`. See the file header for
/// the resolution model and coverage notes.
const GO_TSG: &str = include_str!("../rules/go.tsg");

/// Hand-authored stack-graphs rules for Rust (no upstream package exists). See
/// `rules/rust.tsg` for the resolution model and coverage notes.
const RUST_TSG: &str = include_str!("../rules/rust.tsg");

/// Hand-authored stack-graphs rules for Kotlin (no upstream package exists),
/// built on the `tree-sitter-kotlin-ng` grammar. See `rules/kotlin.tsg`.
const KOTLIN_TSG: &str = include_str!("../rules/kotlin.tsg");

/// Backend interface. Implementations return resolved cross-file refs
/// that `apply_resolutions` turns into `Resolves` edges on the
/// `SymbolGraph`.
pub trait ResolutionBackend: Send + Sync {
    fn resolve(&self, files: &[(PathBuf, String)], graph: &SymbolGraph) -> ResolutionResult;

    /// Human-readable label used in logs / telemetry.
    fn label(&self) -> &'static str;
}

/// Identifier-matching fallback. No scopes, no types. Existed before
/// stack-graphs was wired up and stays as the safety net.
pub struct SyntacticBackend {
    pub language: String,
}

impl ResolutionBackend for SyntacticBackend {
    fn resolve(&self, files: &[(PathBuf, String)], graph: &SymbolGraph) -> ResolutionResult {
        let files_ref: Vec<(&Path, &str)> = files
            .iter()
            .map(|(p, s)| (p.as_path(), s.as_str()))
            .collect();
        resolve_files_with_graph(&files_ref, &self.language, graph)
    }

    fn label(&self) -> &'static str {
        "syntactic"
    }
}

/// `tree-sitter-stack-graphs`-backed resolver.
///
/// This initial integration builds per-language stack graphs but
/// does NOT yet enumerate resolved paths into `ResolvedRef`s — that
/// requires the stitching APIs which vary across the stack-graphs
/// versions we can target. For now the backend verifies that every
/// supported file builds a well-formed stack graph under the budget,
/// and delegates reference enumeration to the syntactic fallback so
/// callers always get a usable result. Once the stitching API is
/// wired (follow-up), the backend will start emitting `ResolvedRef`s
/// with `success = true` for resolved files.
pub struct StackGraphsBackend {
    per_language_timeout: Duration,
    fallback: SyntacticBackend,
}

impl StackGraphsBackend {
    /// `per_language_timeout`: wall-clock budget per language pass
    /// before we abort stack-graphs resolution for that language and
    /// fall through to the syntactic backend for its files.
    pub fn new(per_language_timeout: Duration, fallback_language: impl Into<String>) -> Self {
        Self {
            per_language_timeout,
            fallback: SyntacticBackend {
                language: fallback_language.into(),
            },
        }
    }

    fn language_for_file(path: &Path) -> Option<SupportedLanguage> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("java") => Some(SupportedLanguage::Java),
            // `tree-sitter-stack-graphs-typescript` ships two configs;
            // .ts uses the plain TypeScript grammar, .tsx uses TSX.
            // Mixing them in the same StackGraph corrupts parsing, so
            // we route .tsx through its own SupportedLanguage variant.
            Some("ts") => Some(SupportedLanguage::TypeScript),
            Some("tsx") => Some(SupportedLanguage::Tsx),
            Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => {
                Some(SupportedLanguage::JavaScript)
            }
            Some("py") | Some("pyi") => Some(SupportedLanguage::Python),
            // Go, Rust and Kotlin use CoreGraph's own hand-authored rules
            // (rules/go.tsg, rules/rust.tsg, rules/kotlin.tsg).
            Some("go") => Some(SupportedLanguage::Go),
            Some("rs") => Some(SupportedLanguage::Rust),
            Some("kt") | Some("kts") => Some(SupportedLanguage::Kotlin),
            _ => None,
        }
    }

    /// Number of supported files in `files`. Tests and callers use
    /// this to decide whether to prefer the stack-graphs backend.
    pub fn supported_file_count(files: &[(PathBuf, String)]) -> usize {
        files
            .iter()
            .filter(|(p, _)| Self::language_for_file(p).is_some())
            .count()
    }

    /// Resolve references for every supported file, returning cross-
    /// file `(reference, definition)` pairs as `ResolvedRef`s.
    ///
    /// Pipeline per language:
    ///   1. Build a `StackGraph` covering every file in that language.
    ///   2. Generate minimal partial paths per file (`find_minimal_
    ///      partial_path_set_in_file`) and store them in a `Database`.
    ///   3. Enumerate complete paths from every reference node via
    ///      `find_all_complete_partial_paths` against
    ///      `DatabaseCandidates`, which is the only stack-graphs
    ///      candidate source that supports **cross-file** path
    ///      stitching. (`GraphEdgeCandidates` walks only one file's
    ///      graph edges — faster but unable to resolve imports, which
    ///      is exactly what makes stack-graphs a stronger fallback
    ///      than the per-file syntactic resolver.)
    ///   4. For each complete path, map `start_node` (reference) and
    ///      `end_node` (definition) back to `(file, byte_span)` pairs
    ///      so the existing `apply_resolutions` on `SymbolGraph` can
    ///      turn them into `NameResolved` edges.
    ///
    /// `similar_path_detection` is left on (stack-graphs default)
    /// because our graphs are small (per-language, per-project) and
    /// the protection against exponential fanout outweighs the hit on
    /// the fast path. `collect_stats` stays off to keep memory lean.
    fn resolve_supported(&self, files: &[(PathBuf, String)]) -> (Vec<ResolvedRef>, BuildReport) {
        let timeout = self.per_language_timeout;

        // Collect per-language file lists upfront (fast filter pass).
        let active_langs: Vec<SupportedLanguage> = [
            SupportedLanguage::Java,
            SupportedLanguage::TypeScript,
            SupportedLanguage::Tsx,
            SupportedLanguage::JavaScript,
            SupportedLanguage::Python,
            SupportedLanguage::Go,
            SupportedLanguage::Rust,
            SupportedLanguage::Kotlin,
        ]
        .into_iter()
        .filter(|&lang| {
            files
                .iter()
                .any(|(p, _)| Self::language_for_file(p) == Some(lang))
        })
        .collect();

        if active_langs.is_empty() {
            return (Vec::new(), BuildReport::default());
        }

        // Process each language in its own OS thread. Each language's
        // StackGraph, PartialPaths and Database are fully independent —
        // they share no state except the input file list (immutable borrow).
        //
        // Using std::thread::scope so we can borrow `files` and `timeout`
        // from the enclosing stack frame without cloning.
        let results: Vec<(Vec<ResolvedRef>, BuildReport)> = std::thread::scope(|s| {
            let handles: Vec<_> = active_langs
                .iter()
                .map(|&lang| {
                    let files_for_lang: Vec<(&PathBuf, &String)> = files
                        .iter()
                        .filter(|(p, _)| Self::language_for_file(p) == Some(lang))
                        .map(|(p, src)| (p, src))
                        .collect();
                    s.spawn(move || resolve_language(lang, &files_for_lang, timeout))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| match h.join() {
                    Ok(v) => v,
                    Err(payload) => {
                        log::warn!(
                            "stack-graphs per-language thread panicked; \
                                 falling back to default. Payload: {:?}",
                            payload_as_str(&payload)
                        );
                        Default::default()
                    }
                })
                .collect()
        });

        let mut report = BuildReport::default();
        let mut refs: Vec<ResolvedRef> = Vec::new();
        for (r, rpt) in results {
            refs.extend(r);
            report.built += rpt.built;
            report.failed_files += rpt.failed_files;
            report.failed_languages += rpt.failed_languages;
            report.timed_out += rpt.timed_out;
        }
        (refs, report)
    }

    /// Build the stack graph for every supported file under the
    /// configured timeout. Returns the number of files whose graph
    /// construction succeeded; the caller folds this into its
    /// telemetry / confidence decision.
    pub fn build_supported_graphs(&self, files: &[(PathBuf, String)]) -> BuildReport {
        use std::time::Instant;
        use tree_sitter_stack_graphs::NoCancellation as TsNoCancel;

        let mut report = BuildReport::default();

        for lang in [
            SupportedLanguage::Java,
            SupportedLanguage::TypeScript,
            SupportedLanguage::Tsx,
            SupportedLanguage::JavaScript,
            SupportedLanguage::Python,
            SupportedLanguage::Go,
            SupportedLanguage::Rust,
            SupportedLanguage::Kotlin,
        ] {
            let files_for_lang: Vec<_> = files
                .iter()
                .filter(|(p, _)| Self::language_for_file(p) == Some(lang))
                .collect();
            if files_for_lang.is_empty() {
                continue;
            }

            let config = language_configuration(lang);

            let mut sg = stack_graphs::graph::StackGraph::new();
            let start = Instant::now();
            for (path, source) in &files_for_lang {
                if start.elapsed() >= self.per_language_timeout {
                    report.timed_out += files_for_lang.len().saturating_sub(report.built);
                    break;
                }
                let file = match sg.add_file(&path.to_string_lossy()) {
                    Ok(f) => f,
                    Err(_) => {
                        report.failed_files += 1;
                        continue;
                    }
                };
                let globals = build_globals(lang, path);
                match config.sgl.build_stack_graph_into(
                    &mut sg,
                    file,
                    source,
                    &globals,
                    &TsNoCancel,
                ) {
                    Ok(_) => report.built += 1,
                    Err(_) => report.failed_files += 1,
                }
            }
        }

        report
    }
}

fn language_configuration(
    lang: SupportedLanguage,
) -> tree_sitter_stack_graphs::loader::LanguageConfiguration {
    use tree_sitter_stack_graphs::NoCancellation;
    match lang {
        SupportedLanguage::Java => {
            tree_sitter_stack_graphs_java::language_configuration(&NoCancellation)
        }
        // tree-sitter-stack-graphs-typescript ships two configs
        // (`language_configuration_typescript` and `language_configuration_tsx`).
        // We only wire up the TS one for now; TSX files still build but the
        // tree-sitter grammar is strict about JSX syntax. Follow-up can add a
        // separate TSX entry.
        SupportedLanguage::TypeScript => {
            tree_sitter_stack_graphs_typescript::language_configuration_typescript(&NoCancellation)
        }
        SupportedLanguage::Tsx => {
            tree_sitter_stack_graphs_typescript::language_configuration_tsx(&NoCancellation)
        }
        SupportedLanguage::JavaScript => {
            tree_sitter_stack_graphs_javascript::language_configuration(&NoCancellation)
        }
        SupportedLanguage::Python => {
            tree_sitter_stack_graphs_python::language_configuration(&NoCancellation)
        }
        // Go has no upstream package — build the config from CoreGraph's own
        // hand-authored rules. The .tsg is a tested compile-time constant, so a
        // parse failure here is a build-time programming error, not a runtime
        // condition (hence `expect`).
        SupportedLanguage::Go => {
            tree_sitter_stack_graphs::loader::LanguageConfiguration::from_sources(
                tree_sitter_go::LANGUAGE.into(),
                Some("source.go".into()),
                None,
                vec!["go".into()],
                std::path::PathBuf::from("go.tsg"),
                GO_TSG,
                None,
                None,
                &NoCancellation,
            )
            .expect("bundled go.tsg must compile into a LanguageConfiguration")
        }
        // Rust, like Go, has no upstream package — built from CoreGraph's own
        // hand-authored rules. The .tsg is a tested compile-time constant, so a
        // parse failure here is a build-time programming error (hence `expect`).
        SupportedLanguage::Rust => {
            tree_sitter_stack_graphs::loader::LanguageConfiguration::from_sources(
                tree_sitter_rust::LANGUAGE.into(),
                Some("source.rust".into()),
                None,
                vec!["rs".into()],
                std::path::PathBuf::from("rust.tsg"),
                RUST_TSG,
                None,
                None,
                &NoCancellation,
            )
            .expect("bundled rust.tsg must compile into a LanguageConfiguration")
        }
        // Kotlin, like Go/Rust, has no upstream package — built from CoreGraph's
        // own rules on the `tree-sitter-kotlin-ng` grammar.
        SupportedLanguage::Kotlin => {
            tree_sitter_stack_graphs::loader::LanguageConfiguration::from_sources(
                tree_sitter_kotlin_ng::LANGUAGE.into(),
                Some("source.kotlin".into()),
                None,
                vec!["kt".into(), "kts".into()],
                std::path::PathBuf::from("kotlin.tsg"),
                KOTLIN_TSG,
                None,
                None,
                &NoCancellation,
            )
            .expect("bundled kotlin.tsg must compile into a LanguageConfiguration")
        }
    }
}

/// Per-file global variables for stack-graph construction. Most languages need
/// none (the loader injects FILE_PATH / ROOT_NODE / JUMP_TO_SCOPE_NODE itself).
/// Go's rules additionally require `PKG_PATH` — the file's directory — as the
/// package-identity key (the Go package NAME is not unique project-wide, the
/// directory is). Without it `build_stack_graph_into` errors on the undefined
/// global, so this MUST be supplied wherever a Go file is built.
fn build_globals(
    lang: SupportedLanguage,
    path: &Path,
) -> tree_sitter_stack_graphs::Variables<'static> {
    let mut globals = tree_sitter_stack_graphs::Variables::new();
    if lang == SupportedLanguage::Go {
        let dir = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ".".to_string());
        let _ = globals.add("PKG_PATH".into(), dir.into());
    }
    if lang == SupportedLanguage::Rust {
        // rust.tsg declares `global DIR` and `global MOD_NAME` with no defaults,
        // so both MUST be injected wherever a Rust file is built (omitting either
        // makes `build_stack_graph_into` error). `rust_module_globals` is the
        // single shared derivation (also used by the rust_tsg.rs test harness).
        let (dir, mod_name) = rust_module_globals(path);
        let _ = globals.add("DIR".into(), dir.into());
        let _ = globals.add("MOD_NAME".into(), mod_name.into());
    }
    globals
}

/// Derive the `(dir, mod_name)` stack-graph globals for a Rust file under the
/// file-per-module first approximation. Shared by the production Rust arm of
/// `build_globals` and the isolated `rust_tsg.rs` test harness so the two
/// derivations can never drift — a globals mismatch between test and live is
/// exactly what bit the Go transition.
///
/// - `dir`: the resolution anchor — the directory a sibling `mod::item` path is
///   looked up under. Sibling modules share a directory, so this is the file's
///   parent (the GRANDPARENT for `mod.rs`, whose module is the parent directory
///   itself). `"."` when there is no parent.
/// - `mod_name`: the module this file defines. The file stem for a regular file;
///   the parent directory name for `mod.rs`; the sentinel `"crate"` for a crate
///   root (`lib.rs` / `main.rs`).
///
/// Crate roots deliberately get `"crate"`: a single-segment `foo::bar` path
/// never writes `crate` as its first segment (`crate::` is a distinct CST node
/// we do not model), so a root file can never become the false target of a
/// sibling path. When in doubt we prefer a name that simply MISSES (falls
/// through to the 0.85 syntactic layer) over one that could resolve to the
/// wrong file.
pub fn rust_module_globals(path: &Path) -> (String, String) {
    let dir_of = |p: Option<&Path>| {
        p.map(|d| d.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ".".to_string())
    };
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = path.parent();
    match stem {
        "lib" | "main" => (dir_of(parent), "crate".to_string()),
        "mod" => {
            // `dir/mod.rs` defines module `dir`, addressed as `dir::item` from a
            // sibling of `dir/` → anchor on the grandparent, name on the parent.
            let mod_name = parent
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("crate")
                .to_string();
            (dir_of(parent.and_then(|p| p.parent())), mod_name)
        }
        other => (dir_of(parent), other.to_string()),
    }
}

#[derive(Default, Debug, Clone)]
pub struct BuildReport {
    pub built: usize,
    pub failed_files: usize,
    pub failed_languages: usize,
    pub timed_out: usize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SupportedLanguage {
    Java,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Rust,
    Kotlin,
}

impl ResolutionBackend for StackGraphsBackend {
    fn resolve(&self, files: &[(PathBuf, String)], graph: &SymbolGraph) -> ResolutionResult {
        let (stitched_refs, build_report) = self.resolve_supported(files);

        // Always fold in the syntactic fallback so Rust/Go/Kotlin
        // files — which have no upstream stack-graphs rules — still
        // produce cross-file edges. The fallback may also pick up
        // name-level matches stack-graphs missed (e.g. references
        // across supported and unsupported files).
        let fallback = self.fallback.resolve(files, graph);

        // `success` is true when stack-graphs stitched at least one
        // complete path and every supported file either built or was
        // explicitly skipped under the timeout budget. Otherwise fall
        // back to the fallback's own success flag (false).
        let success = !stitched_refs.is_empty()
            && build_report.failed_files == 0
            && build_report.failed_languages == 0;

        let mut refs = stitched_refs;
        refs.extend(fallback.refs);

        ResolutionResult { refs, success }
    }

    fn label(&self) -> &'static str {
        "stack-graphs"
    }
}

/// Translate a stack-graphs `Node` handle into the `(file, byte_span)`
/// pair `ResolvedRef` uses, by looking up the file's source text to
/// convert `lsp_positions::Position` into byte offsets.
fn node_to_ref(
    sg: &stack_graphs::graph::StackGraph,
    node: stack_graphs::arena::Handle<stack_graphs::graph::Node>,
    file_sources: &[(
        stack_graphs::arena::Handle<stack_graphs::graph::File>,
        PathBuf,
        String,
    )],
) -> Option<(PathBuf, (u32, u32))> {
    let file = sg[node].file()?;
    let (_, path, _source) = file_sources.iter().find(|(f, _, _)| *f == file)?;
    let info = sg.source_info(node)?;
    let start = span_to_byte(&info.span.start);
    let end = span_to_byte(&info.span.end);
    Some((path.clone(), (start, end)))
}

fn span_to_byte(pos: &lsp_positions::Position) -> u32 {
    (pos.containing_line.start + pos.column.utf8_offset) as u32
}

/// Process all files for a single language: build a StackGraph, generate
/// partial paths, stitch complete paths, and return resolved cross-file refs.
///
/// This is a standalone free function so it can be dispatched into an OS
/// thread via `std::thread::scope` — each language's StackGraph is
/// entirely independent, so languages can run concurrently without any
/// shared mutable state.
fn resolve_language(
    lang: SupportedLanguage,
    files: &[(&PathBuf, &String)],
    timeout: std::time::Duration,
) -> (Vec<ResolvedRef>, BuildReport) {
    use stack_graphs::partial::PartialPaths;
    use stack_graphs::stitching::{
        Database, DatabaseCandidates, ForwardPartialPathStitcher, StitcherConfig,
    };
    use stack_graphs::CancelAfterDuration;
    use std::time::Instant;
    use tree_sitter_stack_graphs::NoCancellation as TsNoCancel;

    let mut report = BuildReport::default();

    let config = language_configuration(lang);
    let mut sg = stack_graphs::graph::StackGraph::new();
    type FileEntry = (
        stack_graphs::arena::Handle<stack_graphs::graph::File>,
        PathBuf,
        String,
    );
    let mut file_sources: Vec<FileEntry> = Vec::new();

    let start = Instant::now();
    let mut built = 0usize;
    for (path, source) in files {
        if start.elapsed() >= timeout {
            report.timed_out += files.len().saturating_sub(built);
            break;
        }
        let file = match sg.add_file(&path.to_string_lossy()) {
            Ok(f) => f,
            Err(_) => {
                report.failed_files += 1;
                continue;
            }
        };
        let globals = build_globals(lang, path);
        match config
            .sgl
            .build_stack_graph_into(&mut sg, file, source, &globals, &TsNoCancel)
        {
            Ok(_) => {
                report.built += 1;
                built += 1;
                file_sources.push((file, (*path).clone(), (*source).clone()));
            }
            Err(_) => report.failed_files += 1,
        }
    }

    if built == 0 {
        return (Vec::new(), report);
    }

    let remaining = timeout
        .checked_sub(start.elapsed())
        .unwrap_or(std::time::Duration::from_millis(1));

    // Phase 1: generate minimal partial paths per file.
    let phase1_cancel = CancelAfterDuration::new(remaining / 2);
    let mut partials = PartialPaths::new();
    let mut db = Database::new();
    let mut phase1_cancelled = false;
    for (file, _, _) in &file_sources {
        let result = ForwardPartialPathStitcher::find_minimal_partial_path_set_in_file(
            &sg,
            &mut partials,
            *file,
            StitcherConfig::default(),
            &phase1_cancel,
            |g, p, path| {
                db.add_partial_path(g, p, path.clone());
            },
        );
        if result.is_err() {
            phase1_cancelled = true;
            break;
        }
    }

    // Phase 2: stitch complete paths for every reference node.
    let phase2_cancel = CancelAfterDuration::new(remaining / 2);
    let references: Vec<_> = sg.iter_nodes().filter(|&h| sg[h].is_reference()).collect();
    let mut all_paths: Vec<stack_graphs::partial::PartialPath> = Vec::new();
    {
        let mut candidates = DatabaseCandidates::new(&sg, &mut partials, &mut db);
        let _ = ForwardPartialPathStitcher::find_all_complete_partial_paths(
            &mut candidates,
            references,
            StitcherConfig::default(),
            &phase2_cancel,
            |_, _, path| {
                all_paths.push(path.clone());
            },
        );
    }
    if phase1_cancelled {
        report.timed_out += 1;
    }

    // Consumer-side shadowing. The stitcher returns EVERY complete path; it does
    // not suppress a lexical inner definition that hides an outer one (its
    // built-in shadowing only de-duplicates equal paths to the same definition).
    // `PartialPath::shadows` reports when a higher-precedence path (e.g. a
    // function-local binding) hides a lower-precedence one (the package
    // fall-through) for the same reference — so we drop shadowed paths. Paths
    // from different references never shadow each other (their first edges have
    // different source nodes), so this is effectively per-reference.
    let mut collected: Vec<(
        stack_graphs::arena::Handle<stack_graphs::graph::Node>,
        stack_graphs::arena::Handle<stack_graphs::graph::Node>,
    )> = Vec::new();
    for (i, p) in all_paths.iter().enumerate() {
        let shadowed = all_paths
            .iter()
            .enumerate()
            .any(|(j, q)| i != j && q.shadows(&mut partials, p));
        if !shadowed {
            collected.push((p.start_node, p.end_node));
        }
    }

    // Phase 3: translate node handles back into ResolvedRef entries.
    let mut refs = Vec::new();
    for (start_node, end_node) in collected {
        let Some(from_ref) = node_to_ref(&sg, start_node, &file_sources) else {
            continue;
        };
        let Some(to_def) = node_to_ref(&sg, end_node, &file_sources) else {
            continue;
        };
        if from_ref.0 == to_def.0 {
            // Same-file hits handled by per-file extractor; skip.
            continue;
        }
        refs.push(ResolvedRef {
            from_file: from_ref.0,
            from_span: from_ref.1,
            to_file: to_def.0,
            to_span: to_def.1,
            // Stitched by stack-graphs → genuine cross-file name resolution.
            origin: AnalysisOrigin::NameResolved,
        });
    }

    (refs, report)
}

/// Extract a human-readable string from a panic payload.
///
/// Tries `&'static str` first (the common `panic!("literal")` case), then
/// `String` (the `panic!("{}", …)` case). Falls back to a generic sentinel so
/// callers always get a printable value.
fn payload_as_str(payload: &Box<dyn std::any::Any + Send>) -> &str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return s;
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.as_str();
    }
    "<non-string panic payload>"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_trait_is_object_safe() {
        let _syn: Box<dyn ResolutionBackend> = Box::new(SyntacticBackend {
            language: "rust".into(),
        });
        let _sg: Box<dyn ResolutionBackend> =
            Box::new(StackGraphsBackend::new(Duration::from_secs(5), "rust"));
    }

    #[test]
    fn stack_graphs_language_detection() {
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("Foo.java")),
            Some(SupportedLanguage::Java)
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("a.ts")),
            Some(SupportedLanguage::TypeScript)
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("a.tsx")),
            Some(SupportedLanguage::Tsx),
            ".tsx must route to its own Tsx variant"
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("a.js")),
            Some(SupportedLanguage::JavaScript)
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("a.py")),
            Some(SupportedLanguage::Python)
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("a.pyi")),
            Some(SupportedLanguage::Python)
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("main.rs")),
            Some(SupportedLanguage::Rust),
            "Rust uses CoreGraph's own hand-authored rules (rules/rust.tsg)"
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("main.go")),
            Some(SupportedLanguage::Go),
            "Go uses CoreGraph's own hand-authored rules (rules/go.tsg)"
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("Main.kt")),
            Some(SupportedLanguage::Kotlin),
            "Kotlin uses CoreGraph's own rules (rules/kotlin.tsg) on tree-sitter-kotlin-ng"
        );
        assert_eq!(
            StackGraphsBackend::language_for_file(Path::new("main.rb")),
            None,
            "Ruby has no stack-graphs rules in CoreGraph"
        );
    }

    #[test]
    fn supported_file_count_filters_by_extension() {
        let files = vec![
            (PathBuf::from("a.java"), String::new()),
            (PathBuf::from("b.kt"), String::new()),
            (PathBuf::from("c.ts"), String::new()),
            (PathBuf::from("d.go"), String::new()),
            (PathBuf::from("e.rs"), String::new()),
            (PathBuf::from("f.rb"), String::new()),
        ];
        // Java + TS + Go + Rust + Kotlin are supported; only Ruby (f.rb) is not.
        assert_eq!(StackGraphsBackend::supported_file_count(&files), 5);
    }

    #[test]
    fn syntactic_backend_reports_success_false() {
        let graph = SymbolGraph::new();
        let backend = SyntacticBackend {
            language: "rust".into(),
        };
        let files: Vec<(PathBuf, String)> = vec![];
        let result = backend.resolve(&files, &graph);
        assert!(!result.success);
    }

    #[test]
    fn stack_graphs_build_empty_files() {
        let backend = StackGraphsBackend::new(Duration::from_secs(1), "rust");
        let files: Vec<(PathBuf, String)> = vec![];
        let report = backend.build_supported_graphs(&files);
        assert_eq!(report.built, 0);
        assert_eq!(report.failed_files, 0);
    }
}
