pub mod error;
pub mod incremental;
pub mod queries;
pub mod scanner;

// Language extractor modules
pub mod config_extractor;
pub mod doc_comment;
pub mod drift;
pub mod go_extractor;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod markdown;
pub mod python_extractor;
pub mod rust_extractor;
pub mod string_literal_extractor;
pub mod typescript;

pub use drift::{find_doc_param_drift, DocDriftKind, DocDriftReport};

pub use error::ExtractError;

use coregraph_core::edge::AnalysisOrigin;
use coregraph_core::{
    DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode, Visibility, DEFAULT_EXCLUDE_PATTERNS,
};
use coregraph_graph::{
    apply_mediator, DockerComposeMediator, EdgeEvaluator, GoDiMediator, GraphEpoch,
    GraphInvalidator, HookRegistry, Mediator, ReactRouterMediator, SpringConfigMediator,
    SpringDiMediator, SymbolGraph, ValueMatcher,
};
use coregraph_manifest::parse_project;
use coregraph_stack::apply_resolutions;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Kind of syntactic reference that turns into a typed graph edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    Call,
    Import,
    Extends,
    Implements,
    TypeUse,
    /// A value-position use of a symbol that is neither a call nor a type
    /// reference — reading a module constant through a subscript (`OBJ[key]`)
    /// or member access (`obj.prop`), or passing one into a JSX prop
    /// (`durationInFrames={FRAMES}`). Modeled as a `References` edge (like
    /// `TypeUse`) so the used symbol is connected without polluting the call
    /// graph with non-call edges.
    ValueRef,
    /// `export * from './mod'` — re-exports every public symbol of the target
    /// module. The reference's `name` carries the module specifier (not a symbol
    /// name); resolution expands it to edges into the target file's public API.
    ReexportAll,
}

impl ReferenceKind {
    pub fn to_edge_kind(self) -> EdgeKind {
        match self {
            ReferenceKind::Call => EdgeKind::Calls,
            ReferenceKind::Import => EdgeKind::Imports,
            ReferenceKind::Extends => EdgeKind::Extends,
            ReferenceKind::Implements => EdgeKind::Implements,
            ReferenceKind::TypeUse => EdgeKind::References,
            // A value-position read is a use of the symbol, modeled as a
            // References edge (same as TypeUse) — it connects the symbol for
            // dead-code/impact purposes without claiming it is *called*.
            ReferenceKind::ValueRef => EdgeKind::References,
            // A barrel re-export makes the target symbol available through this
            // module — modeled as an Imports edge into each re-exported symbol.
            ReferenceKind::ReexportAll => EdgeKind::Imports,
        }
    }
}

/// A raw reference extracted from source, pending resolution against the graph.
#[derive(Debug, Clone)]
pub struct RawReference {
    pub name: String,
    pub kind: ReferenceKind,
    /// Byte offset where the reference occurs (used to find the enclosing
    /// defining symbol to attribute the source of the edge).
    pub byte_offset: u32,
}

/// A doc comment attached to a definition, located by an extractor. The
/// documentation pass maps `def_span` to its already-extracted symbol node and
/// emits a `DocComment` node (spanning `doc_span`) plus a `Documents` edge.
/// Both spans are byte offsets into the same file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocCommentRef {
    /// Byte span of the documented definition (matches the symbol node's span).
    pub def_span: (u32, u32),
    /// Byte span of the doc-comment block.
    pub doc_span: (u32, u32),
    /// How the attachment was established. `SyntaxMatched` (0.85) when a
    /// dedicated doc marker (`///`, `/** */`, Python docstring) is attached by
    /// the language's own rule; `PatternMatched` (0.60) when it rests on a
    /// convention without a distinct marker (e.g. a Go `//` line comment).
    pub origin: AnalysisOrigin,
}

/// Trait implemented by every language-specific symbol extractor.
pub trait SymbolExtractor: Send + Sync {
    /// Human-readable language name, e.g. "Java", "TypeScript".
    fn language_name(&self) -> &'static str;

    /// File extensions this extractor handles (without dots), e.g. `["java"]`.
    fn file_extensions(&self) -> &[&'static str];

    /// Parse `source` (contents of `path`) and insert discovered symbols/edges
    /// into `graph`.
    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError>;

    /// Extract unresolved references (call sites, imports, extends, etc.) from
    /// the source. Default: none. Languages that provide accurate reference
    /// extraction return typed edges.
    fn extract_references(&self, _path: &Path, _source: &str) -> Vec<RawReference> {
        Vec::new()
    }

    /// Locate doc comments attached to definitions, as `(def_span, doc_span)`
    /// pairs. Default: none (language has no doc extraction yet). The
    /// documentation pass turns each into a `DocComment` node + `Documents`
    /// edge. Returning a pair asserts the doc is attached by the language's own
    /// doc-comment rule (dedicated marker adjacent to the definition), which is
    /// why the resulting edge is `SyntaxMatched` rather than a heuristic guess.
    fn extract_doc_comments(&self, _path: &Path, _source: &str) -> Vec<DocCommentRef> {
        Vec::new()
    }
}

/// Convenience function: read a file and run the given extractor on it.
pub fn extract_file(
    extractor: &dyn SymbolExtractor,
    path: &Path,
    graph: &mut SymbolGraph,
) -> Result<(), ExtractError> {
    let source = std::fs::read_to_string(path).map_err(|e| ExtractError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    extractor.extract(path, &source, graph)
}

/// Returns a list of all built-in extractors.
pub fn all_extractors() -> Vec<Box<dyn SymbolExtractor>> {
    vec![
        Box::<rust_extractor::RustExtractor>::default(),
        Box::<java::JavaExtractor>::default(),
        Box::<kotlin::KotlinExtractor>::default(),
        Box::<typescript::TypeScriptExtractor>::default(),
        Box::<javascript::JavaScriptExtractor>::default(),
        Box::<python_extractor::PythonExtractor>::default(),
        Box::<go_extractor::GoExtractor>::default(),
        Box::<config_extractor::ConfigExtractor>::default(),
    ]
}

/// Detect the primary language of a project by counting files per extractor.
/// Returns `None` for empty projects (no recognised source files) so the
/// caller can skip the stack-graphs backend entirely rather than spinning
/// up an indexer that has nothing to index. The `"rust"` fallback that
/// used to be unconditional leaked into tests as a misleading
/// `StackGraphsBackend` label even when the project contained zero
/// sources.
fn detect_primary_language(sources: &[(PathBuf, String)]) -> Option<&'static str> {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let extractors = all_extractors();
    for (path, _) in sources {
        for ex in &extractors {
            if scanner::extension_matches(path, ex.file_extensions()) {
                *counts.entry(ex.language_name()).or_default() += 1;
                break;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(lang, _)| lang)
}

/// Enumerate every language (by the extractor's reported name, lowercased)
/// present under `root` by counting at least one matching file per extractor.
/// A general-purpose project language census.
///
/// Walks the same ignore-respecting tree as `collect_sources` so
/// `.coregraph/ignore` filters apply.
pub fn detect_languages(root: &Path) -> Vec<String> {
    let extractors = all_extractors();
    // Same gitignore-aware, default-excluding walk the indexer honours, so
    // build outputs / vendored samples don't inflate the language census.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in index_walk_builder(root).build() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        for ex in &extractors {
            if scanner::extension_matches(path, ex.file_extensions()) {
                seen.insert(ex.language_name().to_ascii_lowercase());
                break;
            }
        }
    }
    seen.into_iter().collect()
}

/// Collect source files that match any built-in extractor.
/// Returns a list of `(path, source)` pairs.
///
/// Results are sorted by path so the downstream graph is built in a
/// deterministic order. Previously the `ignore::WalkBuilder` yielded
/// filesystem-order entries, which varies by OS and inode layout and
/// caused `edge_count()` to drift 5–10% between runs on the same
/// source tree. Stable ordering makes snapshots comparable.
fn collect_sources(root: &Path) -> Vec<(PathBuf, String)> {
    let extractors = all_extractors();

    // Collect matching paths first (single-threaded walk — just stat calls).
    // `index_walk_builder` prunes excluded directories (build outputs, deps,
    // .gitignore) at the walk level, so we never descend into them; splitting
    // path discovery from I/O lets us read file contents in parallel below.
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in index_walk_builder(root).build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let matches = extractors
            .iter()
            .any(|ex| scanner::extension_matches(path, ex.file_extensions()));
        if !matches {
            continue;
        }
        paths.push(path.to_path_buf());
    }

    // Read file contents in parallel — I/O is often the bottleneck on cold
    // caches. rayon distributes across available cores, and the OS page
    // cache typically absorbs repeated warm runs.
    use rayon::prelude::*;
    let read: Vec<(PathBuf, String)> = paths
        .into_par_iter()
        .filter_map(|path| {
            std::fs::read_to_string(&path)
                .ok()
                .map(|source| (path, source))
        })
        .collect();

    // Partition out minified/generated bundles so the skip is visible (a code
    // graph silently ignoring files is worse than reporting it). stderr keeps
    // it off the stdout protocol stream used by LSP/MCP.
    let (minified, mut out): (Vec<_>, Vec<_>) = read
        .into_iter()
        .partition(|(_, source)| looks_minified(source));
    if !minified.is_empty() {
        eprintln!(
            "coregraph: skipped {} minified/generated file(s) (e.g. {})",
            minified.len(),
            minified[0].0.display()
        );
    }

    out.sort_by(|(a, _), (b, _)| a.cmp(b));
    out
}

/// True for files that are almost certainly minified or generated bundles, not
/// human-authored source. Indexing them floods orphans/impact with thousands of
/// single-letter symbols nothing meaningfully references (e.g. a committed Dokka
/// `docs/api/scripts/main.js`). Language-agnostic: formatted source essentially
/// never has a line this long, while minified JS/CSS pack whole files into lines
/// tens of thousands of characters wide. Conservative threshold to avoid
/// dropping real source that merely contains a long string or data literal.
fn looks_minified(source: &str) -> bool {
    // Robust signal: a minified/generated bundle packs the whole file into a few
    // enormous lines, so its AVERAGE line length is huge. Formatted source
    // averages tens of chars per line. Gating on the average (not "any one long
    // line") means a real source file that merely contains one long generated /
    // base64 line among normal code is NOT dropped — only files dominated by
    // packed content are. The size floor avoids touching small files.
    const MIN_BYTES: usize = 4096;
    const MAX_AVG_LINE_BYTES: usize = 1000;
    let len = source.len();
    if len < MIN_BYTES {
        return false;
    }
    let lines = source.lines().count().max(1);
    len / lines > MAX_AVG_LINE_BYTES
}

/// Gitignore-style matcher used to skip directories at index time. Always
/// includes the universal `coregraph_core::DEFAULT_EXCLUDE_PATTERNS` (build
/// outputs, dependency caches, VCS and IDE folders) and layers the project's
/// `[index].exclude` array on top. Sharing the defaults with
/// `coregraph_query::PathExcluder` (both read them from `coregraph-core`)
/// guarantees the index-time and analysis-time exclusion sets cannot drift —
/// previously this matcher omitted the defaults entirely, so a project with no
/// `[index].exclude` (or no config at all) indexed `build/`, `node_modules/`,
/// `.gradle/` and the like, parsing thousands of generated/vendored files.
///
/// The extractor crate can't depend on `coregraph-query` (cycle through
/// `coregraph-cli`), so it builds its own matcher from the shared constant.
fn load_index_excluder(root: &Path) -> IndexExcluder {
    let user_patterns = read_index_exclude_patterns(root);
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    // Universal defaults always apply. Compile-time constants, so a failure is
    // a bug — still skip-and-warn rather than disabling every other pattern.
    for p in DEFAULT_EXCLUDE_PATTERNS {
        if builder.add_line(None, p).is_err() {
            eprintln!("[coregraph] WARNING: invalid built-in exclude pattern '{p}' skipped");
        }
    }
    // User `[index].exclude` patterns layer on top so they can add new
    // excludes or negate a default (`!build/keep/`). Skip-and-warn on a bad
    // glob; mirrors PathExcluder::build.
    for p in &user_patterns {
        if builder.add_line(None, p).is_err() {
            eprintln!(
                "[coregraph] WARNING: invalid exclude pattern '{p}' in .coregraph/config.toml skipped"
            );
        }
    }
    match builder.build() {
        Ok(matcher) => IndexExcluder {
            matcher: Some(matcher),
            root: root.to_path_buf(),
        },
        Err(e) => {
            eprintln!(
                "[coregraph] WARNING: exclude matcher failed to build ({e}); no excludes applied"
            );
            IndexExcluder::empty()
        }
    }
}

/// Read the `[index].exclude` array from `<root>/.coregraph/config.toml`.
/// Returns an empty vec when the file or key is absent, or the file is
/// malformed (the parse warning is emitted here once per index run).
fn read_index_exclude_patterns(root: &Path) -> Vec<String> {
    let cfg_path = root.join(".coregraph").join("config.toml");
    let Ok(text) = std::fs::read_to_string(&cfg_path) else {
        return Vec::new();
    };
    let parsed = match toml::from_str::<toml::Value>(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "[coregraph] WARNING: failed to parse {}: {e} — [index].exclude ignored",
                cfg_path.display()
            );
            return Vec::new();
        }
    };
    parsed
        .as_table()
        .and_then(|t| t.get("index").and_then(|v| v.as_table()))
        .and_then(|t| t.get("exclude").and_then(|v| v.as_array()))
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// A gitignore-aware recursive walker for source indexing, shared by every
/// index-time tree walk (`collect_sources`, `detect_languages`,
/// `collect_markdown`, and the daemon's freshness check).
///
/// Two behaviours matter for performance and correctness:
/// 1. `require_git(false)` — honour `.gitignore` even when the tree is not
///    (yet) a git repository. A vendored copy or freshly-extracted tarball
///    still carries the project's ignore intent; the `ignore` crate default
///    (`require_git(true)`) would silently ignore `.gitignore` without a
///    `.git` directory.
/// 2. `filter_entry` with [`load_index_excluder`] — prune the universal
///    build-output / dependency directories (and the project's
///    `[index].exclude`) at the directory level, so the walk never descends
///    into a 3,000-file `build/` or a giant `node_modules/`.
pub fn index_walk_builder(root: &Path) -> ignore::WalkBuilder {
    let excluder = Arc::new(load_index_excluder(root));
    let mut builder = ignore::WalkBuilder::new(root);
    builder.require_git(false);
    builder.filter_entry(move |entry| !excluder.is_excluded(entry.path()));
    builder
}

struct IndexExcluder {
    matcher: Option<ignore::gitignore::Gitignore>,
    root: PathBuf,
}

impl IndexExcluder {
    fn empty() -> Self {
        Self {
            matcher: None,
            root: PathBuf::new(),
        }
    }

    fn is_excluded(&self, path: &Path) -> bool {
        let Some(m) = &self.matcher else {
            return false;
        };
        // NOTE: clean_path (CurDir stripping, see PathExcluder) is deliberately
        // not applied here — the index walker always supplies absolute paths.
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        m.matched_path_or_any_parents(&absolute, absolute.is_dir())
            .is_ignore()
    }
}

/// Default cap on how many distinct files may share one string value before
/// StringMatch pairing skips that value as a convention string. Overridable
/// via `[index] string_match_max_files` (0 = unlimited).
///
/// Also used by the recommendation engine to establish the baseline when no
/// project config overrides the value.
pub const DEFAULT_STRING_MATCH_MAX_FILES: usize = 8;

/// Read `[index] string_match_max_files` from `<root>/.coregraph/config.toml`.
///
/// Returns the configured value, or [`DEFAULT_STRING_MATCH_MAX_FILES`] when
/// the file is absent, unreadable, or the key is not present. Also used by
/// the recommendation engine to read the project's effective cap.
pub fn string_match_max_files(root: &Path) -> usize {
    let cfg_path = root.join(".coregraph").join("config.toml");
    let Ok(text) = std::fs::read_to_string(&cfg_path) else {
        return DEFAULT_STRING_MATCH_MAX_FILES;
    };
    let Ok(parsed) = toml::from_str::<toml::Value>(&text) else {
        // The parse warning is emitted by load_index_excluder on the same run.
        return DEFAULT_STRING_MATCH_MAX_FILES;
    };
    parsed
        .as_table()
        .and_then(|t| t.get("index").and_then(|v| v.as_table()))
        .and_then(|t| t.get("string_match_max_files").and_then(|v| v.as_integer()))
        .map(|n| n.max(0) as usize)
        .unwrap_or(DEFAULT_STRING_MATCH_MAX_FILES)
}

/// Walk `root` recursively (respecting .gitignore), extract symbols from all
/// recognised source files, build cross-file edges, and return the populated
/// graph plus file count.
///
/// Equivalent to `build_graph_with_hooks(root, &HookRegistry::new())`.
pub fn build_graph(root: &Path) -> anyhow::Result<(SymbolGraph, usize)> {
    build_graph_with_hooks(root, &HookRegistry::new())
}

/// Same as `build_graph`, but fires pre/post analysis hooks from the registry.
/// Pre hooks run before any extraction; post hooks run after the graph is
/// fully built (nodes + all edge stages).
pub fn build_graph_with_hooks(
    root: &Path,
    hooks: &HookRegistry,
) -> anyhow::Result<(SymbolGraph, usize)> {
    // Stage 0: pre-analysis hooks (may abort the build on error).
    hooks.fire_pre(root)?;

    // Stage 1: load all source files once.
    let sources = collect_sources(root);
    let file_count = sources.len();

    // Stage 2: run every language extractor to populate nodes.
    //
    // Tree-sitter parsing + symbol extraction is embarrassingly parallel per
    // file — each extractor mutates only the per-file scratch graph it is
    // handed, and the main graph is stitched together once under a single
    // lock. On excalidraw (424 files) this cut the cold-build wall clock
    // from ~240s → ~70s on an 8-core M-series.
    use rayon::prelude::*;
    let extractors = all_extractors();
    let string_literal = string_literal_extractor::StringLiteralExtractor::new();
    let per_file_graphs: Vec<SymbolGraph> = sources
        .par_iter()
        .map(|(path, source)| {
            let mut local = SymbolGraph::new();
            for extractor in &extractors {
                if scanner::extension_matches(path, extractor.file_extensions()) {
                    let _ = extractor.extract(path, source, &mut local);
                    break;
                }
            }
            // Stage 2.5 runs in the same parallel pass: the string-literal
            // scanner is independent of the primary extractor so it can run
            // on every file that matches its extension list.
            if scanner::extension_matches(path, string_literal.file_extensions()) {
                let _ = string_literal.extract(path, source, &mut local);
            }
            local
        })
        .collect();

    let mut graph = SymbolGraph::new();
    // Files produce SymbolNodes. Extractors only add nodes in `extract()`;
    // edges come from later stages that inspect the merged graph. The merge
    // is O(total_nodes) with no cross-file dependencies inside a local graph.
    for local in &per_file_graphs {
        for node in local.nodes() {
            graph.insert_node(node.clone());
        }
    }

    // Stage 3: cross-file StringMatch edges (e.g. API path literals).
    ValueMatcher::match_strings(&mut graph, string_match_max_files(root));

    // Stage 4: mediator edges (Configures).
    let spring_edges = SpringDiMediator.detect(&graph);
    apply_mediator(&mut graph, spring_edges);
    let react_edges = ReactRouterMediator.detect(&graph);
    apply_mediator(&mut graph, react_edges);
    let spring_config_edges = SpringConfigMediator.detect(&graph);
    apply_mediator(&mut graph, spring_config_edges);
    let docker_compose_edges = DockerComposeMediator.detect(&graph);
    apply_mediator(&mut graph, docker_compose_edges);
    let go_di_edges = GoDiMediator.detect(&graph);
    apply_mediator(&mut graph, go_di_edges);

    // Stage 4.5: structural scaffolding — File/Module nodes plus
    // Contains/BelongsTo edges per docs §4.2. Runs before reference
    // resolution so that top-of-file refs (import specifiers, type
    // annotations above any enclosing class/function) can attach to
    // the File node as their enclosing symbol instead of being dropped.
    structural_pass(&mut graph);

    // Stage 4.6: documentation layer — DocComment nodes + Documents edges
    // (docs/graph-model.md §7). Runs as a post-stage because the Stage 2
    // merge copies only NODES from per-file graphs; an edge created inside
    // `extract()` would be discarded. Symbol node ids are stable here.
    documents_pass(&mut graph, &sources, &extractors);

    // Stage 4.7: doc-text mentions — Mentions edges from a DocComment to the
    // code symbols its text links to (`{@link X}`, `` [`X`] ``). Runs after
    // documents_pass so DocComment nodes exist; resolution is name-based and
    // may cross files. See docs/graph-model.md §7.5.
    mentions_pass(&mut graph, &sources);

    // Stage 4.8: external docs — DocSection nodes + DescribedIn edges from
    // Markdown files that reference code symbols by a backticked `` `Name` ``.
    // See docs/graph-model.md §7.6.
    markdown_pass(&mut graph, &collect_markdown(root));

    // Stage 5: typed references from extractors (Calls/Imports/Extends/Implements).
    resolve_references(&mut graph, &sources, &extractors, root);

    // Stage 5.5: derive Inherits from the now-complete Extends set (Java/Kotlin).
    // Runs after resolution so it covers the resolver's cross-file Extends too.
    derive_inherits(&mut graph);

    // Stage 6: cross-file name resolution.
    //
    // `StackGraphsBackend::resolve` runs the stack-graphs pipeline under
    // a per-language wall-clock budget — upstream rules for Java/TS/JS/
    // Python plus CoreGraph's own hand-authored rules for Go, Rust and
    // Kotlin — and merges in the syntactic fallback's result for anything
    // unsupported. The merged `ResolutionResult` drives
    // `apply_resolutions`, which promotes stack-graphs-stitched hits to
    // `NameResolved` (0.95) and leaves fallback hits at `SyntaxMatched`
    // (0.85) per docs/graph-model.md §6.3.
    if let Some(language) = detect_primary_language(&sources) {
        // Pass &sources directly — no clone needed; the backend signature
        // accepts &[(PathBuf, String)] which &Vec<(PathBuf, String)> satisfies.
        let backend = coregraph_stack::StackGraphsBackend::new(
            std::time::Duration::from_secs(5),
            language.to_string(),
        );
        let resolution =
            <coregraph_stack::StackGraphsBackend as coregraph_stack::ResolutionBackend>::resolve(
                &backend, &sources, &graph,
            );
        apply_resolutions(&mut graph, &resolution);
    }

    // Stage 6.6: reclassify StringMatch edges into the more specific
    // EnumValueMatch / ApiPathMatch variants where the endpoint kinds permit.
    reclassify_string_match(&mut graph);

    // Stage 6.7: simple TypeOf / GenericParam extraction from source text.
    extract_type_relationships(&mut graph, &sources);

    // Stage 7: post-analysis hooks (observe-only; errors propagate).
    hooks.fire_post(&graph)?;

    Ok((graph, file_count))
}

/// Incremental rebuild: invalidate the portion of the graph sourced from
/// `changed_files`, then re-run extraction on the full tree.
///
/// This is a pragmatic incremental path. A true incremental rebuild would
/// re-extract only the changed files, but cross-file resolution (imports,
/// stack-graphs, string-match mediators) means a changed definition can
/// invalidate edges in untouched files. We keep the structural and
/// reference edges correct by invalidating the changed-file evidence and
/// re-running a SUBSET of `build_graph`'s downstream stages: string-match,
/// mediators, `structural_pass`, `resolve_references`,
/// `reclassify_string_match` and `extract_type_relationships`. This path
/// does NOT re-run the documentation layer (`documents_pass`,
/// `mentions_pass`, `markdown_pass`) or the Stage 6 stack-graphs
/// resolution. Because `GraphInvalidator::invalidate` removes the changed
/// files' `DocComment` nodes, `Documents`/`Mentions` edges and stitched
/// `Resolves` edges, an incremental rebuild drops the documentation layer
/// and stack-graphs `NameResolved` edges for changed files until the next
/// full `build_graph`. The win comes from skipping extraction of unchanged
/// files — stage 2 is embarrassingly parallel and file-local, so re-running
/// it only on changed files cuts the wall clock proportionally to
/// `changed / total`.
pub fn build_graph_incremental(
    root: &Path,
    graph: &mut SymbolGraph,
    changed_files: &[PathBuf],
) -> anyhow::Result<usize> {
    use rayon::prelude::*;

    if changed_files.is_empty() {
        return Ok(0);
    }

    // Invalidate existing graph content from changed files.
    let epoch = GraphEpoch::zero();
    let _ = GraphInvalidator::invalidate(graph, changed_files, epoch);

    // Load sources for the changed files only; re-extract their nodes.
    let extractors = all_extractors();
    let string_literal = string_literal_extractor::StringLiteralExtractor::new();
    let changed_sources: Vec<(PathBuf, String)> = changed_files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok().map(|s| (p.clone(), s)))
        .collect();
    let per_file_graphs: Vec<SymbolGraph> = changed_sources
        .par_iter()
        .map(|(path, source)| {
            let mut local = SymbolGraph::new();
            for extractor in &extractors {
                if scanner::extension_matches(path, extractor.file_extensions()) {
                    let _ = extractor.extract(path, source, &mut local);
                    break;
                }
            }
            if scanner::extension_matches(path, string_literal.file_extensions()) {
                let _ = string_literal.extract(path, source, &mut local);
            }
            local
        })
        .collect();
    for local in &per_file_graphs {
        for node in local.nodes() {
            graph.insert_node(node.clone());
        }
    }
    drop(per_file_graphs);

    // Re-run the cross-cutting stages against the full graph. Their cost
    // scales with node/edge counts, not file count, and the resolver needs
    // full visibility to rewire dangling references from the invalidated
    // files' consumers.
    let all_sources = collect_sources(root);
    let file_count = all_sources.len();
    ValueMatcher::match_strings(graph, string_match_max_files(root));
    let spring_edges = SpringDiMediator.detect(graph);
    apply_mediator(graph, spring_edges);
    let react_edges = ReactRouterMediator.detect(graph);
    apply_mediator(graph, react_edges);
    let spring_config_edges = SpringConfigMediator.detect(graph);
    apply_mediator(graph, spring_config_edges);
    let docker_compose_edges = DockerComposeMediator.detect(graph);
    apply_mediator(graph, docker_compose_edges);
    let go_di_edges = GoDiMediator.detect(graph);
    apply_mediator(graph, go_di_edges);
    structural_pass(graph);
    resolve_references(graph, &all_sources, &extractors, root);
    derive_inherits(graph);
    reclassify_string_match(graph);
    extract_type_relationships(graph, &all_sources);

    Ok(file_count)
}

/// Canonical name of the synthetic module we attach symbols from `path` to
/// when the language extractor didn't emit its own `Module` node. Mirrors
/// the crate-layout convention: `crates/<name>/...` → `<name>`, else the
/// immediate parent directory, else `"root"`.
fn synthetic_module_name(path: &Path) -> String {
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    parts
        .iter()
        .position(|&p| p == "crates")
        .and_then(|i| parts.get(i + 1).copied())
        .or_else(|| path.parent().and_then(|p| p.file_name()?.to_str()))
        .unwrap_or("root")
        .to_string()
}

/// Score candidate files so the synthetic module node's `file` field lands
/// on a representative source (`src/lib.rs`, `src/mod.rs`, `src/main.rs`),
/// not a test/fixture file that happens to match the same synthetic module
/// name. Higher score wins.
fn module_file_representative_score(path: &Path) -> i32 {
    let mut score = 0;
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    // Test/fixture directories should almost never win over normal source.
    let deprioritise = parts.iter().any(|p| {
        matches!(
            *p,
            "tests" | "test" | "__tests__" | "fixtures" | "testdata" | "benches" | "examples"
        )
    });
    if deprioritise {
        score -= 20;
    }

    // Source directories boost the score so `src/lib.rs` beats
    // `../some-dir/lib.rs` when both carry the same synthetic module name.
    if parts.contains(&"src") {
        score += 10;
    }

    // Canonical entry points are the most representative.
    if let Some("lib.rs" | "main.rs" | "mod.rs" | "index.ts" | "index.js" | "__init__.py") =
        path.file_name().and_then(|n| n.to_str())
    {
        score += 5;
    }

    score
}

/// Emit a `SymbolKind::File` node for every distinct source file and a
/// `Contains` edge from that file node to every symbol defined there.
/// Also creates `BelongsTo` edges from symbols to their enclosing Module
/// (when an extractor produced one).
fn structural_pass(graph: &mut SymbolGraph) {
    // 1. Collect files + modules + children per file.
    //
    // BTreeMap (path-sorted) instead of HashMap so the subsequent
    // edge-emission loops run in a stable order. Previously the same
    // source tree produced runs with 5–10% edge-count drift because
    // the HashMap iteration order affected which candidates won name
    // collisions in `pick_resolve_targets`.
    let mut files_to_children: BTreeMap<PathBuf, Vec<SymbolId>> = BTreeMap::new();
    let mut modules_in_file: BTreeMap<PathBuf, SymbolId> = BTreeMap::new();
    let mut existing_file_nodes: BTreeMap<PathBuf, SymbolId> = BTreeMap::new();

    for n in graph.nodes() {
        if n.kind == SymbolKind::File {
            existing_file_nodes.insert(n.file.to_path_buf(), n.id);
            continue;
        }
        // Skip placeholder nodes with no file anchor. `ExternalPackage`
        // is the primary case: it's a lazily-created stub for
        // unresolved `use`/impl targets and by design has an empty
        // `PathBuf`. Feeding it into `files_to_children` would cause
        // the synthetic File/Module pass below to create phantom
        // `File` and `Module` nodes keyed on the empty path, polluting
        // the node listing and the nodes-per-file stats.
        if n.file.as_os_str().is_empty() {
            continue;
        }
        // Module/Namespace nodes feed the module lookup but are containers, not
        // contained children — they must not receive File->Contains. A clean
        // build never has them as children here (the synthetic Module is minted
        // in step 4, AFTER the Contains pass), so on an incremental re-run a
        // surviving Module would otherwise be wrapped in a fresh Contains and
        // diverge from a clean build.
        if n.kind == SymbolKind::Module || n.kind == SymbolKind::Namespace {
            modules_in_file.insert(n.file.to_path_buf(), n.id);
            continue;
        }
        // Doc-layer nodes are created by LATER passes (documents_pass /
        // markdown_pass) and carry Documents/DescribedIn edges, not containment.
        // A clean build runs structural_pass before they exist, so including
        // them here would emit Contains/BelongsTo only on an incremental re-run.
        if n.kind == SymbolKind::DocComment || n.kind == SymbolKind::DocSection {
            continue;
        }
        files_to_children
            .entry(n.file.to_path_buf())
            .or_default()
            .push(n.id);
    }

    // 2. For every file without an existing File node, insert one.
    let mut file_ids: BTreeMap<PathBuf, SymbolId> = existing_file_nodes.clone();
    for path in files_to_children.keys() {
        if file_ids.contains_key(path) {
            continue;
        }
        let stem = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let id = graph.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::File,
            stem,
            path.clone(),
            0,
            0,
        ));
        file_ids.insert(path.clone(), id);
    }

    // 3. Emit Contains edges: File → every symbol defined in that file.
    let contains_conf =
        EdgeEvaluator::evaluate(EdgeKind::Contains, AnalysisOrigin::CompilerDerived);
    for (path, children) in &files_to_children {
        let Some(&file_id) = file_ids.get(path) else {
            continue;
        };
        for child in children {
            if *child == file_id {
                continue;
            }
            graph.insert_edge(DirectEdge::new(
                file_id,
                *child,
                EdgeKind::Contains,
                AnalysisOrigin::CompilerDerived,
                contains_conf,
                path.clone(),
            ));
        }
    }

    // 4. Emit BelongsTo edges: Symbol → enclosing Module. If no Module node
    //    exists in the file, synthesise one from the first directory segment
    //    of the path so BelongsTo is always populated.
    //
    // The synthetic module's `file` field should point at a representative
    // source file (e.g. `crates/extractor/src/lib.rs`), not whichever file
    // happened to be visited first — previously the first iteration of
    // `files_to_children` could land on a test or fixture file, making
    // `coregraph query` surface the module as "@ tests/.../fixtures.rs".
    //
    // We precompute the best representative per synthetic module name by
    // scoring each contributing file, then build the module node with the
    // winning path.
    let belongs_conf =
        EdgeEvaluator::evaluate(EdgeKind::BelongsTo, AnalysisOrigin::CompilerDerived);
    let mut best_file_for_module: HashMap<String, (PathBuf, i32)> = HashMap::new();
    for path in files_to_children.keys() {
        if modules_in_file.contains_key(path) {
            // File already has a real Module node — no synthetic needed.
            continue;
        }
        let module_name = synthetic_module_name(path);
        let score = module_file_representative_score(path);
        best_file_for_module
            .entry(module_name)
            .and_modify(|existing| {
                if score > existing.1 {
                    *existing = (path.clone(), score);
                }
            })
            .or_insert((path.clone(), score));
    }

    let mut synth_module_ids: HashMap<String, SymbolId> = HashMap::new();
    // Idempotency: reuse synthetic Module nodes left by a prior structural_pass.
    // build_graph runs this once on a freshly-extracted graph, but
    // build_graph_incremental (the daemon's watcher rebuild path) re-runs it on
    // an already-structured graph. Without reusing existing Module nodes here,
    // every re-run allocated a fresh synthetic Module per group and re-emitted a
    // BelongsTo edge for every symbol — the old edges survive (their symbols are
    // not invalidated), so the containment layer duplicated on each rebuild
    // (BelongsTo grew ~one-per-symbol per pass, ratcheting the persisted graph).
    // File nodes (step 2) are already reused by path for the same reason.
    for n in graph.nodes() {
        if n.kind == SymbolKind::Module {
            synth_module_ids.entry(n.name.clone()).or_insert(n.id);
        }
    }
    for (path, children) in &files_to_children {
        let module_id = if let Some(&id) = modules_in_file.get(path) {
            id
        } else {
            let module_name = synthetic_module_name(path);
            let representative = best_file_for_module
                .get(&module_name)
                .map(|(p, _)| p.clone())
                .unwrap_or_else(|| path.clone());
            *synth_module_ids
                .entry(module_name.clone())
                .or_insert_with(|| {
                    graph.insert_node(SymbolNode::new(
                        SymbolId(0),
                        SymbolKind::Module,
                        module_name,
                        representative,
                        0,
                        0,
                    ))
                })
        };
        for child in children {
            if *child == module_id {
                continue;
            }
            graph.insert_edge(DirectEdge::new(
                *child,
                module_id,
                EdgeKind::BelongsTo,
                AnalysisOrigin::CompilerDerived,
                belongs_conf,
                path.clone(),
            ));
        }
    }
}

/// Duplicate `Extends` edges as `Inherits` when the evidence file is Java or
/// Kotlin (languages where `class A extends B` is true inheritance; Rust / TS
/// trait bounds keep the Extends-only form).
///
/// Runs as its OWN stage AFTER `resolve_references` so it covers every `Extends`
/// edge — including the cross-file ones the resolver adds. Folding it into
/// `structural_pass` (which runs before resolution) left the resolver's Extends
/// without an Inherits in a clean build, yet an incremental re-run re-applied it
/// over the now-complete Extends set, so the two diverged. `insert_edge` dedups,
/// so re-running this is idempotent.
fn derive_inherits(graph: &mut SymbolGraph) {
    let inherits_conf = EdgeEvaluator::evaluate(EdgeKind::Inherits, AnalysisOrigin::NameResolved);
    let existing_extends: Vec<DirectEdge> = graph
        .edges()
        .filter(|e| e.kind == EdgeKind::Extends)
        .filter(|e| {
            let ext = e
                .evidence_file
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            ext == "java" || ext == "kt"
        })
        .cloned()
        .collect();
    for e in existing_extends {
        graph.insert_edge(DirectEdge::new(
            e.from,
            e.to,
            EdgeKind::Inherits,
            AnalysisOrigin::NameResolved,
            inherits_conf,
            e.evidence_file.to_path_buf(),
        ));
    }
}

/// Upgrade StringMatch edges to the narrower EnumValueMatch or ApiPathMatch
/// variants when the endpoint kinds allow. The original StringMatch edge is
/// replaced in place.
fn reclassify_string_match(graph: &mut SymbolGraph) {
    let mut replacements: Vec<(DirectEdge, EdgeKind)> = Vec::new();
    for edge in graph.edges() {
        if edge.kind != EdgeKind::StringMatch {
            continue;
        }
        let (Some(a), Some(b)) = (graph.get_node(edge.from), graph.get_node(edge.to)) else {
            continue;
        };
        let new_kind = if a.kind == SymbolKind::EnumVariant && b.kind == SymbolKind::EnumVariant {
            EdgeKind::EnumValueMatch
        } else if a.kind == SymbolKind::StringLiteral
            && b.kind == SymbolKind::StringLiteral
            && a.name.starts_with("api_path::")
            && b.name.starts_with("api_path::")
        {
            EdgeKind::ApiPathMatch
        } else {
            continue;
        };
        replacements.push((edge.clone(), new_kind));
    }

    for (old_edge, new_kind) in replacements {
        graph.retain_edges(|e| {
            !(e.from == old_edge.from && e.to == old_edge.to && e.kind == EdgeKind::StringMatch)
        });
        let confidence = EdgeEvaluator::evaluate(new_kind.clone(), old_edge.origin);
        let new_edge = DirectEdge::new(
            old_edge.from,
            old_edge.to,
            new_kind,
            old_edge.origin,
            confidence,
            old_edge.evidence_file.to_path_buf(),
        );
        graph.insert_edge(new_edge);
    }
}

/// Simple TypeOf / GenericParam extraction from source text. Uses regex over
/// Rust/Java/TypeScript/Python to catch the common cases the language
/// extractors don't currently emit. Coarse — the mediator surfaces
/// candidates, not a full type resolver.
fn extract_type_relationships(graph: &mut SymbolGraph, sources: &[(PathBuf, String)]) {
    use regex::Regex;
    use std::sync::OnceLock;

    // Static regex instances compiled once per process lifetime (OnceLock is
    // guaranteed to initialise exactly once even under concurrent calls).
    static RUST_LET_TYPED: OnceLock<Regex> = OnceLock::new();
    static TS_LET_TYPED: OnceLock<Regex> = OnceLock::new();
    static TS_ANNOTATION: OnceLock<Regex> = OnceLock::new();
    static RUST_ANNOTATION: OnceLock<Regex> = OnceLock::new();
    static JAVA_FIELD: OnceLock<Regex> = OnceLock::new();
    static JAVA_PARAM: OnceLock<Regex> = OnceLock::new();
    static GENERIC_USAGE: OnceLock<Regex> = OnceLock::new();

    let rust_let_typed = RUST_LET_TYPED.get_or_init(|| {
        Regex::new(r#"let\s+(?:mut\s+)?[A-Za-z_][A-Za-z0-9_]*\s*:\s*([A-Z][A-Za-z0-9_]*)"#).unwrap()
    });
    let ts_let_typed = TS_LET_TYPED.get_or_init(|| {
        Regex::new(r#"(?:let|const|var)\s+[A-Za-z_][A-Za-z0-9_]*\s*:\s*([A-Z][A-Za-z0-9_]*)"#)
            .unwrap()
    });
    // Parameter / field / return-type annotations that the `let` form misses.
    // The TS pattern catches `foo(name: Type)`, `private x: Type`, `: Promise<T>`
    // at function boundaries, etc. Matches any identifier followed by `: Type`
    // where Type starts with an uppercase letter — still PatternMatched trust
    // so noise is tolerable.
    let ts_annotation = TS_ANNOTATION.get_or_init(|| {
        Regex::new(r#"[A-Za-z_][A-Za-z0-9_]*\s*:\s*([A-Z][A-Za-z0-9_]*)"#).unwrap()
    });
    // Rust function params / return types: `fn foo(x: T)` and `-> T`.
    let rust_annotation = RUST_ANNOTATION
        .get_or_init(|| Regex::new(r#"(?:->|:)\s*(?:&(?:mut\s+)?)?([A-Z][A-Za-z0-9_]*)"#).unwrap());
    let java_field = JAVA_FIELD.get_or_init(|| {
        Regex::new(r#"(?m)^\s*(?:private|protected|public)?\s*(?:final\s+)?([A-Z][A-Za-z0-9_]+)\s+[a-z][A-Za-z0-9_]*\s*[=;]"#).unwrap()
    });
    // Java method params: `fn(TypeName name, TypeName other)`.
    let java_param = JAVA_PARAM.get_or_init(|| {
        Regex::new(r#"\(([A-Z][A-Za-z0-9_]+)\s+[a-z][A-Za-z0-9_]*(?:\s*,\s*([A-Z][A-Za-z0-9_]+)\s+[a-z][A-Za-z0-9_]*)*"#).unwrap()
    });
    let generic_usage = GENERIC_USAGE.get_or_init(|| {
        Regex::new(
            r#"[A-Z][A-Za-z0-9_]*\s*<\s*([A-Z][A-Za-z0-9_]+)(?:\s*,\s*[A-Z][A-Za-z0-9_]+)*\s*>"#,
        )
        .unwrap()
    });

    let typeof_conf = EdgeEvaluator::evaluate(EdgeKind::TypeOf, AnalysisOrigin::PatternMatched);
    let generic_conf =
        EdgeEvaluator::evaluate(EdgeKind::GenericParam, AnalysisOrigin::PatternMatched);

    // Build a cheap "name → first candidate id" index, preferring Class /
    // Struct / Interface / Enum / TypeAlias nodes so we emit to the type
    // declaration rather than an arbitrary value.
    let mut name_index: HashMap<String, SymbolId> = HashMap::new();
    for n in graph.nodes() {
        let is_type_like = matches!(
            n.kind,
            SymbolKind::Class
                | SymbolKind::Struct
                | SymbolKind::Trait
                | SymbolKind::Interface
                | SymbolKind::Enum
                | SymbolKind::TypeAlias
        );
        if !is_type_like {
            continue;
        }
        name_index.entry(n.name.clone()).or_insert(n.id);
    }

    let mut pending: Vec<DirectEdge> = Vec::new();
    for (path, source) in sources {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let type_regex: Vec<&Regex> = match ext.as_str() {
            "rs" => vec![&rust_let_typed, &rust_annotation],
            "ts" | "tsx" => vec![&ts_let_typed, &ts_annotation],
            "java" | "kt" => vec![&java_field, &java_param],
            _ => Vec::new(),
        };

        // Resolve this file's File node once — it is the stable source for the
        // PatternMatched TypeOf / GenericParam edges below. Previously this O(V)
        // lookup ran inside the per-capture loops (O(captures·V) per file).
        let file_src = graph
            .nodes_in_file(path)
            .find(|n| n.kind == SymbolKind::File)
            .map(|n| n.id);

        // Find a nearby defining symbol to attribute the TypeOf edge source.
        for regex in &type_regex {
            for cap in regex.captures_iter(source) {
                let Some(m) = cap.get(1) else { continue };
                let Some(&tgt) = name_index.get(m.as_str()) else {
                    continue;
                };
                // Use the File node (resolved once above) as a stable source.
                let Some(src) = file_src else { continue };
                if src == tgt {
                    continue;
                }
                pending.push(DirectEdge::new(
                    src,
                    tgt,
                    EdgeKind::TypeOf,
                    AnalysisOrigin::PatternMatched,
                    typeof_conf,
                    path.clone(),
                ));
            }
        }

        // Generic parameter bindings: match `List<T>` / `Map<K,V>` patterns.
        for cap in generic_usage.captures_iter(source) {
            let Some(m) = cap.get(1) else { continue };
            let Some(&tgt) = name_index.get(m.as_str()) else {
                continue;
            };
            let Some(src) = file_src else { continue };
            pending.push(DirectEdge::new(
                src,
                tgt,
                EdgeKind::GenericParam,
                AnalysisOrigin::PatternMatched,
                generic_conf,
                path.clone(),
            ));
        }
    }

    for e in pending {
        graph.insert_edge(e);
    }
}

/// Shared helper for the TS/JS extractors: descends through value wrappers —
/// parentheses, `as`/`satisfies` type assertions, and non-null `!` — to decide
/// whether a declarator's value is really a function. This is what lets
/// `export const x = ((a) => …) as X` and `… as unknown as X` register as
/// functions even though their immediate value node is a wrapper, not a bare
/// arrow. Each wrapper carries its inner expression as the first named child,
/// so walking `named_child(0)` reaches the function (or bottoms out on an
/// identifier/literal, which is not a function definition). The `as`/`satisfies`
/// kinds never appear in the JavaScript grammar, so listing them is harmless
/// there and keeps a single shared implementation.
pub(crate) fn unwraps_to_fn(mut node: tree_sitter::Node) -> bool {
    loop {
        match node.kind() {
            "arrow_function" | "function_expression" => return true,
            "parenthesized_expression"
            | "as_expression"
            | "satisfies_expression"
            | "non_null_expression" => match node.named_child(0) {
                Some(inner) => node = inner,
                None => return false,
            },
            _ => return false,
        }
    }
}

/// Shared helper: run a tree-sitter references query and pack results into
/// RawReference. `map_index` turns a query pattern_index into a ReferenceKind.
pub fn tree_sitter_extract_refs<F>(
    language: &tree_sitter::Language,
    query_src: &str,
    source: &str,
    map_index: F,
) -> Vec<RawReference>
where
    F: Fn(usize) -> Option<ReferenceKind>,
{
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let Ok(query) = tree_sitter::Query::new(language, query_src) else {
        return Vec::new();
    };

    let mut cursor = tree_sitter::QueryCursor::new();
    let source_bytes = source.as_bytes();
    let mut out = Vec::new();
    use streaming_iterator::StreamingIterator;
    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
    while let Some(m) = matches.next() {
        let Some(kind) = map_index(m.pattern_index) else {
            continue;
        };
        for cap in m.captures {
            let Ok(name) = cap.node.utf8_text(source_bytes) else {
                continue;
            };
            let trimmed: &str = name.trim_matches(|c: char| c == '"' || c == '\'' || c == '`');
            if trimmed.is_empty() || trimmed.len() < 3 {
                continue;
            }
            if matches!(trimmed, "self" | "super" | "this" | "None") {
                continue;
            }
            out.push(RawReference {
                name: trimmed.to_string(),
                kind,
                byte_offset: cap.node.start_byte() as u32,
            });
        }
    }
    out
}

/// Documentation pass: for every extractor that reports doc comments, map each
/// documented definition's byte span to its symbol node and emit a `DocComment`
/// node + `Documents` edge (docs/graph-model.md §7).
///
/// Runs after the Stage 2 per-file merge, which copies only NODES — an edge an
/// extractor inserts during `extract()` is discarded, so the doc node + edge
/// must be created here against the merged graph where symbol node ids are
/// stable. Linking is by exact `(file, span)` match (not a nearest-comment
/// heuristic): the extractor already paired each doc with its definition using
/// the language's own doc-comment rule.
fn documents_pass(
    graph: &mut SymbolGraph,
    sources: &[(PathBuf, String)],
    extractors: &[Box<dyn SymbolExtractor>],
) {
    // (file, span_start, span_end) -> SymbolId, built once over real
    // definitions (synthetic File/Module and any existing DocComment excluded).
    let mut span_index: HashMap<(PathBuf, u32, u32), SymbolId> = HashMap::new();
    for node in graph.nodes() {
        if matches!(
            node.kind,
            SymbolKind::File | SymbolKind::Module | SymbolKind::DocComment
        ) {
            continue;
        }
        span_index.insert(
            (node.file.to_path_buf(), node.span_start, node.span_end),
            node.id,
        );
    }

    // Resolve doc refs to (symbol_id, name, doc_span) first; the immutable
    // lookups must finish before the mutable `insert_documentation` calls.
    type PendingDocAttach = (PathBuf, SymbolId, String, (u32, u32), AnalysisOrigin);
    let mut to_attach: Vec<PendingDocAttach> = Vec::new();
    for (path, source) in sources {
        let Some(extractor) = extractors
            .iter()
            .find(|e| scanner::extension_matches(path, e.file_extensions()))
        else {
            continue;
        };
        for dref in extractor.extract_doc_comments(path, source) {
            let key = (path.clone(), dref.def_span.0, dref.def_span.1);
            let Some(&sym) = span_index.get(&key) else {
                continue;
            };
            let Some(name) = graph.get_node(sym).map(|n| n.name.clone()) else {
                continue;
            };
            to_attach.push((path.clone(), sym, name, dref.doc_span, dref.origin));
        }
    }

    for (path, sym, name, doc_span, origin) in to_attach {
        doc_comment::insert_documentation(graph, &path, sym, &name, doc_span, origin);
    }
}

/// Collect Markdown documentation files (`.md` / `.markdown`) under `root`,
/// honouring the same ignore rules as `collect_sources`. Markdown is not a code
/// extractor language, so these are gathered separately for the external-docs
/// layer.
fn collect_markdown(root: &Path) -> Vec<(PathBuf, String)> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in index_walk_builder(root).build() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
            .unwrap_or(false);
        if !is_md {
            continue;
        }
        paths.push(path.to_path_buf());
    }
    use rayon::prelude::*;
    let mut out: Vec<(PathBuf, String)> = paths
        .into_par_iter()
        .filter_map(|path| std::fs::read_to_string(&path).ok().map(|s| (path, s)))
        .collect();
    out.sort_by(|(a, _), (b, _)| a.cmp(b));
    out
}

/// External-docs pass: for each Markdown file, split it into heading sections
/// and, for any section that references code symbols by a backticked
/// `` `Name` ``, create a `DocSection` node and a `DescribedIn` edge from each
/// uniquely-resolved symbol to that section.
///
/// A `DocSection` node is created only when the section resolves at least one
/// symbol — sections that describe no code are not added (no noise). Resolution
/// is unique-name only (precision over recall); origin `PatternMatched` (0.60).
fn markdown_pass(graph: &mut SymbolGraph, md_files: &[(PathBuf, String)]) {
    if md_files.is_empty() {
        return;
    }

    // name -> code symbol ids (synthetic / doc nodes excluded).
    let mut by_name: HashMap<String, Vec<SymbolId>> = HashMap::new();
    for n in graph.nodes() {
        if matches!(
            n.kind,
            SymbolKind::DocComment | SymbolKind::DocSection | SymbolKind::File | SymbolKind::Module
        ) {
            continue;
        }
        by_name.entry(n.name.clone()).or_default().push(n.id);
    }

    // Resolve (section, [described symbol ids]) first; mutate the graph after.
    struct Pending {
        path: PathBuf,
        heading: String,
        span: (u32, u32),
        symbols: Vec<SymbolId>,
    }
    let mut pending: Vec<Pending> = Vec::new();
    for (path, source) in md_files {
        for section in markdown::split_sections(source) {
            let (s, e) = (section.start as usize, section.end as usize);
            if s > e || e > source.len() {
                continue;
            }
            let mut symbols: Vec<SymbolId> = Vec::new();
            let mut seen: HashSet<SymbolId> = HashSet::new();
            for name in markdown::code_span_identifiers(&source[s..e]) {
                if let Some(ids) = by_name.get(&name) {
                    if ids.len() == 1 && seen.insert(ids[0]) {
                        symbols.push(ids[0]);
                    }
                }
            }
            if symbols.is_empty() {
                continue;
            }
            pending.push(Pending {
                path: path.clone(),
                heading: section.heading,
                span: (section.start, section.end),
                symbols,
            });
        }
    }

    let confidence = EdgeEvaluator::evaluate(EdgeKind::DescribedIn, AnalysisOrigin::PatternMatched);
    for p in pending {
        let label = if p.heading.is_empty() {
            "docsection::".to_string()
        } else {
            format!("docsection::{}", p.heading)
        };
        let section_node = SymbolNode::new(
            SymbolId(0),
            SymbolKind::DocSection,
            label,
            p.path.clone(),
            p.span.0,
            p.span.1,
        );
        let section_id = graph.insert_node(section_node);
        for sym in p.symbols {
            // Symbol → DocSection: "X is described in this section".
            let edge = DirectEdge::new(
                sym,
                section_id,
                EdgeKind::DescribedIn,
                AnalysisOrigin::PatternMatched,
                confidence,
                p.path.clone(),
            );
            graph.insert_edge(edge);
        }
    }
}

/// Mentions pass: for every `DocComment` node, scan its text for intra-doc link
/// targets (`{@link X}`, `` [`X`] ``) and emit a `Mentions` edge to the code
/// symbol each names. Resolution is name-based and may cross files; only an
/// UNAMBIGUOUS link marker resolving to a UNIQUELY-named symbol produces an
/// edge (precision over recall). Self-mentions (a doc linking the very symbol it
/// documents) are skipped. Origin `PatternMatched` (0.60).
fn mentions_pass(graph: &mut SymbolGraph, sources: &[(PathBuf, String)]) {
    let scanner = doc_comment::MentionLinkScanner::new();
    let src_by_file: HashMap<&Path, &str> = sources
        .iter()
        .map(|(p, s)| (p.as_path(), s.as_str()))
        .collect();

    // The symbol each DocComment documents (to skip self-mentions).
    let documented: HashMap<SymbolId, SymbolId> = graph
        .edges()
        .filter(|e| e.kind == EdgeKind::Documents)
        .map(|e| (e.from, e.to))
        .collect();

    // name -> code symbol ids (synthetic / doc nodes excluded).
    let mut by_name: HashMap<String, Vec<SymbolId>> = HashMap::new();
    for n in graph.nodes() {
        if matches!(
            n.kind,
            SymbolKind::DocComment | SymbolKind::File | SymbolKind::Module
        ) {
            continue;
        }
        by_name.entry(n.name.clone()).or_default().push(n.id);
    }

    let doc_nodes: Vec<(SymbolId, PathBuf, u32, u32)> = graph
        .nodes()
        .filter(|n| n.kind == SymbolKind::DocComment)
        .map(|n| (n.id, n.file.to_path_buf(), n.span_start, n.span_end))
        .collect();

    let mut to_add: Vec<(SymbolId, SymbolId, PathBuf)> = Vec::new();
    let mut seen: HashSet<(SymbolId, SymbolId)> = HashSet::new();
    for (doc_id, file, start, end) in &doc_nodes {
        let Some(src) = src_by_file.get(file.as_path()) else {
            continue;
        };
        let (s, e) = (*start as usize, *end as usize);
        if s > e || e > src.len() {
            continue;
        }
        for name in scanner.targets(&src[s..e]) {
            let Some(ids) = by_name.get(&name) else {
                continue;
            };
            // Unique-name match only: ambiguity has no scope to disambiguate it.
            if ids.len() != 1 {
                continue;
            }
            let target = ids[0];
            if target == *doc_id || documented.get(doc_id) == Some(&target) {
                continue; // skip self-mention
            }
            if seen.insert((*doc_id, target)) {
                to_add.push((*doc_id, target, file.clone()));
            }
        }
    }

    let confidence = EdgeEvaluator::evaluate(EdgeKind::Mentions, AnalysisOrigin::PatternMatched);
    for (doc_id, target, evidence_file) in to_add {
        let edge = DirectEdge::new(
            doc_id,
            target,
            EdgeKind::Mentions,
            AnalysisOrigin::PatternMatched,
            confidence,
            evidence_file,
        );
        graph.insert_edge(edge);
    }
}

/// Collapse `.`/`..` components in a path without touching the filesystem
/// (the file may be virtual in tests, and we only compare against known keys).
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Select the TS/JS tree-sitter grammar for a path, or None for non-TS/JS
/// files. Used by the import-binding pass below (TSX needs the JSX-aware
/// grammar).
fn ts_js_language_for(path: &Path) -> Option<tree_sitter::Language> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") | Some("mts") | Some("cts") => {
            Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        }
        Some("tsx") => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => {
            Some(tree_sitter_javascript::LANGUAGE.into())
        }
        _ => None,
    }
}

/// A named import specifier carrying everything needed to both disambiguate
/// references (by the local binding) and resolve the import directly to its
/// source symbol (by the imported name + module path).
#[derive(Debug)]
struct NamedImport {
    /// Name as exported by the source module — `Route` in `{ Route as X }`.
    imported: String,
    /// Local binding — the `as` alias if present, else the imported name.
    local: String,
    /// Module specifier with surrounding quotes stripped — `./routes/a`.
    spec: String,
    /// Byte offset of the imported-name token within the importing file, used to
    /// locate the enclosing symbol (the File node) that sources the import edge.
    offset: u32,
}

/// Extracts named-import specifiers from a TS/JS file — e.g.
/// `import { exportToSvg as svg } from "../scene/export"` yields a `NamedImport`
/// with `imported = "exportToSvg"`, `local = "svg"`, `spec = "../scene/export"`.
/// Captured structurally (name, alias and source in one query match) so the
/// imported name, the local binding and the module specifier are never confused.
/// Returns empty for non-TS/JS files or on any parse/query failure (the caller
/// then simply has no disambiguation hint / no direct import edge).
fn extract_import_bindings(path: &Path, source: &str) -> Vec<NamedImport> {
    let Some(language) = ts_js_language_for(path) else {
        return Vec::new();
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    // One match per import_specifier; @source is the shared module string.
    let query_src = "(import_statement \
        (import_clause (named_imports (import_specifier name: (identifier) @name \
        (\"as\" (identifier) @alias)?))) \
        source: (string) @source)";
    let Ok(query) = tree_sitter::Query::new(&language, query_src) else {
        return Vec::new();
    };
    let (Some(name_idx), Some(source_idx)) = (
        query.capture_index_for_name("name"),
        query.capture_index_for_name("source"),
    ) else {
        return Vec::new();
    };
    let alias_idx = query.capture_index_for_name("alias");
    // Capture lookups are inlined rather than wrapped in a closure: a closure
    // returning `Node` ties the node's lifetime to the closure argument instead
    // of the parse tree, and a closure returning `&str` ties it to the argument
    // instead of `source` — both fail to compile. Inlining keeps the borrows of
    // the tree and `source` unambiguous.
    let mut cursor = tree_sitter::QueryCursor::new();
    use streaming_iterator::StreamingIterator;
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut out = Vec::new();
    while let Some(m) = matches.next() {
        let Some(name_node) = m
            .captures
            .iter()
            .find(|c| c.index == name_idx)
            .map(|c| c.node)
        else {
            continue;
        };
        let Some(imported) = source.get(name_node.start_byte()..name_node.end_byte()) else {
            continue;
        };
        // Local binding = alias if `import { A as B }`, else the imported name.
        let local = alias_idx
            .and_then(|ai| m.captures.iter().find(|c| c.index == ai).map(|c| c.node))
            .and_then(|n| source.get(n.start_byte()..n.end_byte()))
            .unwrap_or(imported);
        let Some(spec) = m
            .captures
            .iter()
            .find(|c| c.index == source_idx)
            .map(|c| c.node)
            .and_then(|n| source.get(n.start_byte()..n.end_byte()))
            .map(|s| s.trim_matches(|c| c == '"' || c == '\'' || c == '`'))
        else {
            continue;
        };
        out.push(NamedImport {
            imported: imported.to_string(),
            local: local.to_string(),
            spec: spec.to_string(),
            offset: name_node.start_byte() as u32,
        });
    }
    out
}

/// Try `joined` as a module location: the exact file, sibling files with a
/// TS/JS extension, then a directory `index.*` — TS/ESM resolution order.
fn resolve_with_extensions(joined: &Path, known_files: &HashSet<PathBuf>) -> Option<PathBuf> {
    if known_files.contains(joined) {
        return Some(joined.to_path_buf());
    }
    for ext in ["ts", "tsx", "js", "jsx", "mts", "cts"] {
        let cand = joined.with_extension(ext);
        if known_files.contains(&cand) {
            return Some(cand);
        }
    }
    for idx in ["index.ts", "index.tsx", "index.js", "index.jsx"] {
        let cand = joined.join(idx);
        if known_files.contains(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Strip a written TS/JS extension from a specifier before re-resolving
/// (ESM TypeScript writes `./x.js` for a sibling `x.ts`).
fn strip_ts_extension(spec: &str) -> &str {
    [".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs"]
        .iter()
        .find_map(|ext| spec.strip_suffix(ext))
        .unwrap_or(spec)
}

/// Resolve a TypeScript/JavaScript relative module specifier (`./widget`,
/// `../shared/x`, `./x.js`) to a known project file, using TS/ESM resolution
/// order: a sibling file with a TS/JS extension, then a directory `index.*`.
/// `known_files` is the set of ALL indexed source files (not only those with
/// public symbols), so a barrel that re-exports another barrel still resolves —
/// the target may have no direct public symbols, and that is handled by the
/// caller. Bare/package specifiers (not starting with `.`) return `None`;
/// they are workspace-package territory — see `resolve_ts_bare_specifier`.
fn resolve_ts_module_path(
    ref_file: &Path,
    specifier: &str,
    known_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    if !specifier.starts_with('.') {
        return None;
    }
    let base = ref_file.parent()?;
    let joined = normalize_path(&base.join(strip_ts_extension(specifier)));
    resolve_with_extensions(&joined, known_files)
}

/// Workspace member packages as (name, absolute package root), longest name
/// first so `@x/ui-icons` wins over `@x/ui` for a `@x/ui-icons/...` specifier.
/// Sourced from the project's own manifests (npm/pnpm workspaces, Cargo, …)
/// via coregraph-manifest — nothing about the layout is assumed. The root
/// package (path ".") is excluded: a bare specifier never names the root.
/// The manifest walk runs once per build invocation (including incremental
/// rebuilds); if it ever shows up in profiles on very large workspaces, the
/// right cache lives in the daemon layer keyed by root + manifest mtimes,
/// not here.
fn workspace_package_roots(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(manifest) = parse_project(root) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = manifest
        .packages
        .into_iter()
        .filter(|p| !p.path.as_os_str().is_empty() && p.path != Path::new("."))
        .map(|p| (p.name, root.join(&p.path)))
        .collect();
    out.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
    out
}

/// Entry-point candidates advertised by a package's own package.json, in
/// resolution-preference order (`exports["."]`, `module`, `main`, `types`).
/// Best-effort: unreadable/invalid JSON yields no candidates and the caller
/// falls back to conventional index locations.
fn npm_entry_candidates(pkg_root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(pkg_root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(e) = v.get("exports") {
        if let Some(s) = e.as_str() {
            out.push(s.to_string());
        } else if let Some(dot) = e.get(".") {
            if let Some(s) = dot.as_str() {
                out.push(s.to_string());
            } else if let Some(obj) = dot.as_object() {
                for k in ["import", "default", "require", "types"] {
                    if let Some(s) = obj.get(k).and_then(|x| x.as_str()) {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    for k in ["module", "main", "types"] {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
            out.push(s.to_string());
        }
    }
    out
}

/// Resolve a bare (package) specifier against the workspace's own member
/// packages: `@x/ui` → that package's entry module, `@x/ui/sub/path` → the
/// subpath inside the package root. Non-member specifiers (true external
/// dependencies like `react`) return None. Entry files declared with a
/// built `.js` extension resolve to their source sibling via the usual
/// extension swap. `entry_candidates` is the per-package entry cache built
/// once per resolution pass from `npm_entry_candidates`; a package missing
/// from the cache simply uses the conventional fallbacks.
fn resolve_ts_bare_specifier(
    specifier: &str,
    packages: &[(String, PathBuf)],
    entry_candidates: &HashMap<PathBuf, Vec<String>>,
    known_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let (name, pkg_root) = packages.iter().find_map(|(n, p)| {
        if specifier == n {
            Some((n.as_str(), p))
        } else {
            specifier
                .strip_prefix(n.as_str())
                .and_then(|rest| rest.starts_with('/').then_some((n.as_str(), p)))
        }
    })?;
    let subpath = specifier[name.len()..].trim_start_matches('/');
    if !subpath.is_empty() {
        let joined = normalize_path(&pkg_root.join(strip_ts_extension(subpath)));
        return resolve_with_extensions(&joined, known_files);
    }
    let entries = entry_candidates
        .get(pkg_root)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for entry in entries {
        let entry = entry.trim_start_matches("./");
        let joined = normalize_path(&pkg_root.join(strip_ts_extension(entry)));
        if let Some(hit) = resolve_with_extensions(&joined, known_files) {
            return Some(hit);
        }
    }
    // Conventional fallbacks when the manifest declares no resolvable entry.
    for fallback in ["src/index", "index"] {
        if let Some(hit) = resolve_with_extensions(&pkg_root.join(fallback), known_files) {
            return Some(hit);
        }
    }
    None
}

/// Unified TS/JS specifier resolution: relative specifiers resolve against
/// the importing file, bare specifiers against workspace member packages.
fn resolve_ts_specifier(
    ref_file: &Path,
    specifier: &str,
    known_files: &HashSet<PathBuf>,
    packages: &[(String, PathBuf)],
    entry_candidates: &HashMap<PathBuf, Vec<String>>,
) -> Option<PathBuf> {
    if specifier.starts_with('.') {
        resolve_ts_module_path(ref_file, specifier, known_files)
    } else {
        resolve_ts_bare_specifier(specifier, packages, entry_candidates, known_files)
    }
}

/// Walk every source, call each language extractor's extract_references(),
/// then match names against the symbol graph to emit typed edges
/// (Calls, Imports, Extends, Implements).
///
/// `root` is the project root; it feeds manifest-based workspace-package
/// discovery so bare specifiers (`@scope/pkg`) resolve to member packages.
///
/// The edge's confidence comes from EdgeEvaluator so mediators, the resolver
/// and reference resolution use the same confidence scale.
fn resolve_references(
    graph: &mut SymbolGraph,
    sources: &[(PathBuf, String)],
    extractors: &[Box<dyn SymbolExtractor>],
    root: &Path,
) {
    use rayon::prelude::*;

    // Collect all references in parallel — each file's ref extraction is
    // a pure function of (path, source) that doesn't touch the graph.
    // Serial collection was the second-largest single-threaded stage after
    // stack-graphs resolution.
    let refs: Vec<(PathBuf, RawReference)> = sources
        .par_iter()
        .flat_map(|(path, source)| {
            let mut out: Vec<(PathBuf, RawReference)> = Vec::new();
            for ex in extractors {
                if scanner::extension_matches(path, ex.file_extensions()) {
                    for r in ex.extract_references(path, source) {
                        out.push((path.clone(), r));
                    }
                    break;
                }
            }
            out
        })
        .collect();
    if refs.is_empty() {
        return;
    }

    // Build name → candidate defining nodes map and a side-table of
    // (id → defining file) for proximity ranking during resolution.
    let mut by_name: HashMap<String, Vec<SymbolId>> = HashMap::new();
    let mut def_file: HashMap<SymbolId, PathBuf> = HashMap::new();
    let mut def_qualified: HashMap<SymbolId, String> = HashMap::new();
    // Public (exported) symbols per file — the surface a `export * from './x'`
    // re-exports. Keyed by the defining file so a wildcard re-export can expand
    // to edges into exactly that file's exported symbols (not its private ones).
    let mut public_by_file: HashMap<PathBuf, Vec<SymbolId>> = HashMap::new();
    for node in graph.nodes() {
        // ExternalPackage nodes are synthetic stubs THIS pass mints for
        // unresolved imports. They must not be resolution candidates: a clean
        // build has none when by_name is built (they're created during
        // resolution), so letting a re-run resolve references to a pre-existing
        // ExternalPackage would manufacture edges the first build never made —
        // notably Calls/References, which the external fallback below drops but
        // which a `by_name` hit would turn into edges (the incremental-rebuild
        // idempotency bug).
        if node.kind != SymbolKind::ExternalPackage {
            by_name.entry(node.name.clone()).or_default().push(node.id);
        }
        def_file.insert(node.id, node.file.to_path_buf());
        if !node.qualified_name.is_empty() {
            def_qualified.insert(node.id, node.qualified_name.clone());
        }
        if node.visibility == Visibility::Public
            && !node.file.as_os_str().is_empty()
            && !matches!(
                node.kind,
                SymbolKind::File
                    | SymbolKind::Module
                    | SymbolKind::Namespace
                    | SymbolKind::ExternalPackage
                    | SymbolKind::DocComment
                    | SymbolKind::DocSection
            )
        {
            public_by_file
                .entry(node.file.to_path_buf())
                .or_default()
                .push(node.id);
        }
    }

    // Per-file sorted list of (span_start, span_end, id) for O(log n) enclosing lookup.
    let mut per_file: HashMap<PathBuf, Vec<(u32, u32, SymbolId)>> = HashMap::new();
    for node in graph.nodes() {
        per_file.entry(node.file.to_path_buf()).or_default().push((
            node.span_start,
            node.span_end,
            node.id,
        ));
    }
    for v in per_file.values_mut() {
        v.sort_by_key(|(s, _, _)| *s);
    }

    // Cache of synthetic `ExternalPackage` nodes we've already created
    // so every reference to `std::path::Path` across the project
    // connects to the same `std` node (not N duplicates).
    let mut external_nodes: HashMap<String, SymbolId> = HashMap::new();
    // Seed the cache with ExternalPackage stubs left by a prior run (incremental
    // rebuild) so the external fallback dedups against them instead of minting a
    // second stub per name — keeping the pass idempotent on an already-resolved
    // graph. A clean build starts with none, so this is a no-op there.
    for node in graph.nodes() {
        if node.kind == SymbolKind::ExternalPackage {
            external_nodes.entry(node.name.clone()).or_insert(node.id);
        }
    }

    // Project-internal name set — file stems and crate-directory names.
    // We refuse to mint an `ExternalPackage` for any of these because
    // they're almost always Rust file/module references (`use crate::impact`,
    // `use crate::query::ownership`) that the extractor failed to resolve
    // to a Module node. Tagging them as "external" pollutes the graph
    // and confuses downstream tools.
    let internal_names: HashSet<String> = sources
        .iter()
        .flat_map(|(path, _)| {
            // The file stem (`impact.rs` → `impact`) plus every
            // directory component along the way (`crates`, `query`,
            // `src`). The directory components are needed for
            // `use crate::query::…` style imports.
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
            let parents: Vec<String> = path
                .components()
                .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
                .collect();
            stem.into_iter().chain(parents)
        })
        .collect();

    // Resolve each reference. Proximity ranking cuts the fan-out on
    // common names (`is_empty`, `new`, `insert_node`, …) that are
    // defined in many modules: we prefer a same-file match, then a
    // same-directory match, and only fall back to cross-cutting
    // fanout with a lower origin when neither is available.
    // All indexed source files (for module-specifier resolution) and their
    // content lengths (to span an on-demand barrel File node over the file).
    let all_files: HashSet<PathBuf> = sources.iter().map(|(p, _)| p.clone()).collect();
    let file_lens: HashMap<&PathBuf, usize> = sources.iter().map(|(p, s)| (p, s.len())).collect();

    // Workspace member packages for bare-specifier resolution (npm/pnpm
    // monorepos import siblings as `@scope/pkg`, not relative paths).
    let packages = workspace_package_roots(root);
    // Entry-point candidates per member package, read once — bare-specifier
    // resolution would otherwise re-read the same package.json once per
    // import site (twice per import: the disambiguation map and the
    // per-specifier pass).
    let entry_candidates: HashMap<PathBuf, Vec<String>> = packages
        .iter()
        .map(|(_, pkg_root)| (pkg_root.clone(), npm_entry_candidates(pkg_root)))
        .collect();

    // Per-(file, imported-name) → defining file, derived from each TS/JS file's
    // named imports resolved through TS/ESM module resolution. Lets
    // `pick_resolve_targets` disambiguate a call to a name exported from several
    // files by the import the calling file actually wrote. Built in parallel —
    // each file's bindings are a pure function of (path, source).
    // Named-import specifiers for every TS/JS file — computed once and reused for
    // both the `(file, local-name)` disambiguation map below and the
    // per-specifier import-edge pass at the end of this function.
    let file_imports: Vec<(PathBuf, NamedImport)> = sources
        .par_iter()
        .flat_map(|(path, src)| {
            extract_import_bindings(path, src)
                .into_iter()
                .map(|ni| (path.clone(), ni))
                .collect::<Vec<_>>()
        })
        .collect();
    let import_target: HashMap<(PathBuf, String), PathBuf> = file_imports
        .iter()
        .filter_map(|(path, ni)| {
            resolve_ts_specifier(path, &ni.spec, &all_files, &packages, &entry_candidates)
                .map(|target| ((path.clone(), ni.local.clone()), target))
        })
        .collect();

    // On-demand File nodes for pure re-export barrels that define no symbols of
    // their own (so `structural_pass` created no File node for them).
    let mut barrel_nodes: HashMap<PathBuf, SymbolId> = HashMap::new();

    for (ref_file, r) in &refs {
        // Wildcard re-export (`export * from './x'`): handled before the
        // enclosing-symbol guard, because a pure-barrel file defines no symbols
        // and therefore has no enclosing node. `r.name` is the module specifier,
        // not a symbol name; resolve it to the target file and link the barrel
        // module to each of that file's public symbols, so a symbol surfaced
        // only through the barrel is not a false orphan. A barrel that re-exports
        // another barrel resolves to a file with no direct public symbols — that
        // chain link is skipped (its leaves are de-orphaned by the inner barrel).
        if r.kind == ReferenceKind::ReexportAll {
            let symbols =
                resolve_ts_specifier(ref_file, &r.name, &all_files, &packages, &entry_candidates)
                    .and_then(|target| public_by_file.get(&target))
                    .filter(|s| !s.is_empty());
            let Some(symbols) = symbols else {
                continue;
            };
            // Edge source: the barrel's enclosing symbol (its File node when it
            // has one), else an on-demand File node spanning the barrel file.
            let src_id =
                enclosing_symbol(&per_file, ref_file, r.byte_offset).unwrap_or_else(|| {
                    *barrel_nodes.entry(ref_file.clone()).or_insert_with(|| {
                        let name = ref_file
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("module")
                            .to_string();
                        let len = file_lens.get(ref_file).copied().unwrap_or(0) as u32;
                        graph.insert_node(SymbolNode::new(
                            SymbolId(0),
                            SymbolKind::File,
                            name,
                            ref_file.clone(),
                            0,
                            len,
                        ))
                    })
                });
            let origin = AnalysisOrigin::NameResolved;
            let confidence = EdgeEvaluator::evaluate(EdgeKind::Imports, origin);
            for &tgt in symbols {
                if tgt == src_id {
                    continue;
                }
                graph.insert_edge(DirectEdge::new(
                    src_id,
                    tgt,
                    EdgeKind::Imports,
                    origin,
                    confidence,
                    ref_file.clone(),
                ));
            }
            continue;
        }

        let Some(src_id) = enclosing_symbol(&per_file, ref_file, r.byte_offset) else {
            continue;
        };

        let edge_kind = r.kind.to_edge_kind();

        if let Some(candidates) = by_name.get(&r.name) {
            let (chosen, origin) = pick_resolve_targets(
                candidates,
                src_id,
                ref_file,
                &def_file,
                &def_qualified,
                &r.name,
                &import_target,
            );
            let confidence = EdgeEvaluator::evaluate(edge_kind.clone(), origin);
            for tgt in chosen {
                if tgt == src_id {
                    continue;
                }
                let edge = DirectEdge::new(
                    src_id,
                    tgt,
                    edge_kind.clone(),
                    origin,
                    confidence,
                    ref_file.clone(),
                );
                graph.insert_edge(edge);
            }
            continue;
        }

        // External fallback — the reference's name has no matching node
        // in the project. For `Imports` and `Implements` this reliably
        // indicates a dependency on an external crate/module (serde,
        // tokio, std::Default, …) that was previously invisible to the
        // graph. Create a synthetic `ExternalPackage` node so the
        // dependency edge survives. We deliberately *do not* do this
        // for `Calls` / `References` etc., since those names can
        // legitimately be unresolved identifiers (locals, parameters,
        // macros) and would explode the graph.
        if !matches!(r.kind, ReferenceKind::Import | ReferenceKind::Implements) {
            continue;
        }
        // Refuse to mint an ExternalPackage for a name that matches a
        // project-internal file/module. Such refs are `use crate::X::…`
        // that the extractor failed to resolve; labelling them "external"
        // is worse than dropping the edge (observed: `impact`, `inconsistencies`,
        // `ownership`, `budget`, `edge`, `http` all landed under
        // ExternalPackage despite being in-tree modules).
        if internal_names.contains(&r.name) {
            continue;
        }
        let external_id = *external_nodes.entry(r.name.clone()).or_insert_with(|| {
            graph.insert_node(SymbolNode::new(
                SymbolId(0),
                SymbolKind::ExternalPackage,
                r.name.clone(),
                PathBuf::new(),
                0,
                0,
            ))
        });
        let origin = AnalysisOrigin::SyntaxMatched;
        let confidence = EdgeEvaluator::evaluate(edge_kind.clone(), origin);
        let edge = DirectEdge::new(
            src_id,
            external_id,
            edge_kind,
            origin,
            confidence,
            ref_file.clone(),
        );
        graph.insert_edge(edge);
    }

    // Per-specifier named-import resolution. A named import
    // `import { X } from './m'` (or aliased `import { X as Y } from './m'`) is a
    // genuine use of `./m`'s `X`. The generic reference path above resolves the
    // bare `X` reference only when `X` is globally unique; when several files
    // export the same name — the TanStack `routeTree` case, where one generated
    // file imports `Route` from every route module, each under a distinct alias
    // — the `(file, name)` disambiguation map cannot represent N same-named
    // imports in one file, so those targets stay falsely orphaned. Resolving
    // each specifier through ITS OWN module path is immune to that ambiguity.
    // The edge targets the symbol of the IMPORTED name in the resolved file and
    // is sourced from the import site's enclosing symbol (the File node). It is
    // an `Imports` edge identical in shape to what the reference path emits for
    // unambiguous imports, so `insert_edge`'s `(from, to, kind)` dedup collapses
    // the overlap — this pass only ADDS the precise edges the ambiguous case
    // dropped. A pure-import file with no symbols of its own has no File node to
    // source from; such files are skipped (they are de-orphaned through other
    // paths and are rare).
    for (path, ni) in &file_imports {
        let Some(target_file) =
            resolve_ts_specifier(path, &ni.spec, &all_files, &packages, &entry_candidates)
        else {
            continue;
        };
        let Some(candidates) = by_name.get(&ni.imported) else {
            continue;
        };
        let targets: Vec<SymbolId> = candidates
            .iter()
            .copied()
            .filter(|id| def_file.get(id).map(|f| f == &target_file).unwrap_or(false))
            .collect();
        if targets.is_empty() {
            continue;
        }
        let Some(src_id) = enclosing_symbol(&per_file, path, ni.offset) else {
            continue;
        };
        let origin = AnalysisOrigin::NameResolved;
        let confidence = EdgeEvaluator::evaluate(EdgeKind::Imports, origin);
        for tgt in targets {
            if tgt == src_id {
                continue;
            }
            graph.insert_edge(DirectEdge::new(
                src_id,
                tgt,
                EdgeKind::Imports,
                origin,
                confidence,
                path.clone(),
            ));
        }
    }
}

/// Rank `candidates` against the reference's context and return the
/// subset to link plus the origin confidence tier:
///
/// 1. **Same file**: unique and closest — `NameResolved` (0.95).
/// 2. **Exactly one same-directory candidate** (module-level heuristic) —
///    `NameResolved` (0.95). Typical in Rust `mod x;` and Go packages.
///    Multiple same-dir candidates are inherently ambiguous (distinct
///    types' methods sharing a name) and fall through to the rules below.
/// 3. **Exactly one candidate advertises a qualified_name**: that single
///    hit — `NameResolved`. (The qualified_name is not textually compared
///    against the reference; the rule only requires that exactly one
///    candidate carries one.)
/// 4. **Cross-cutting fallback** (everything else): link only when globally
///    unambiguous (one candidate) with `SyntaxMatched` (0.85); otherwise
///    emit nothing rather than a noisy fanout.
fn pick_resolve_targets(
    candidates: &[SymbolId],
    src_id: SymbolId,
    ref_file: &Path,
    def_file: &HashMap<SymbolId, PathBuf>,
    def_qualified: &HashMap<SymbolId, String>,
    ref_name: &str,
    import_target: &HashMap<(PathBuf, String), PathBuf>,
) -> (Vec<SymbolId>, AnalysisOrigin) {
    // 1. Same-file match (strongest signal).
    let same_file: Vec<SymbolId> = candidates
        .iter()
        .copied()
        .filter(|id| *id != src_id)
        .filter(|id| def_file.get(id).map(|f| f == ref_file).unwrap_or(false))
        .collect();
    if !same_file.is_empty() {
        return (same_file, AnalysisOrigin::NameResolved);
    }

    // 2. Same-directory match (module-level co-location) — only when it
    //    uniquely identifies the definition. Returning every same-dir
    //    sibling turned common method names (`new`, `is_empty`, …) into an
    //    unbounded fan-out at the top confidence tier: in a flat crate
    //    layout (`src/*.rs`) "same directory" is the whole crate, so one
    //    `X::new()` call linked every sibling type's `new`. That is the
    //    exact poison rule 4 below refuses to emit — multiple same-dir
    //    candidates fall through so the qualified/import rules can still
    //    disambiguate, and unresolved ambiguity is dropped, not guessed.
    let ref_dir = ref_file.parent();
    let same_dir: Vec<SymbolId> = candidates
        .iter()
        .copied()
        .filter(|id| *id != src_id)
        .filter(|id| def_file.get(id).and_then(|f| f.parent()) == ref_dir)
        .collect();
    if same_dir.len() == 1 {
        return (same_dir, AnalysisOrigin::NameResolved);
    }

    // 3. qualified_name disambiguation when defs advertise one.
    let qualified_hits: Vec<SymbolId> = candidates
        .iter()
        .copied()
        .filter(|id| *id != src_id)
        .filter(|id| def_qualified.contains_key(id))
        .collect();
    if qualified_hits.len() == 1 {
        return (qualified_hits, AnalysisOrigin::NameResolved);
    }

    // 3.5. Import-scoped disambiguation. When `ref_name` is imported into
    //      `ref_file` from a relative module that resolves to a known file, keep
    //      only the candidate(s) defined in that file. This recovers the common
    //      "same-named function exported from two files, the caller imports one
    //      of them" case that step 4 would otherwise drop. Purely additive: it
    //      runs only after the same-file/dir/qualified checks already failed to
    //      return, so it never changes an edge those steps resolved — it only
    //      converts the ambiguous drop below into a precise edge.
    if let Some(target_file) = import_target.get(&(ref_file.to_path_buf(), ref_name.to_string())) {
        let import_hits: Vec<SymbolId> = candidates
            .iter()
            .copied()
            .filter(|id| *id != src_id)
            .filter(|id| def_file.get(id).map(|f| f == target_file).unwrap_or(false))
            .collect();
        if !import_hits.is_empty() {
            return (import_hits, AnalysisOrigin::NameResolved);
        }
    }

    // 4. Cross-cutting fallback: only link when the name is globally
    //    unambiguous (exactly one candidate after filtering self).
    //    Historically every candidate received an edge which made common
    //    method names like `is_empty` / `new` / `insert_node` dominate the
    //    in-degree ranking with mostly-spurious connections. For truly
    //    ambiguous names, emit nothing rather than poison the graph —
    //    stack-graphs resolution (build_graph Stage 6) is the proper answer
    //    for these; here silence is more honest than a noisy fanout.
    let others: Vec<SymbolId> = candidates
        .iter()
        .copied()
        .filter(|id| *id != src_id)
        .collect();
    if others.len() == 1 {
        return (others, AnalysisOrigin::SyntaxMatched);
    }
    (Vec::new(), AnalysisOrigin::SyntaxMatched)
}

/// Find the defining symbol that most likely encloses a reference at `offset`.
///
/// Extractors typically record span_start/span_end as just the *identifier*
/// token of the definition, not the full body range. So strict containment
/// rarely matches. We instead pick the nearest definition whose span_start
/// precedes `offset` in the same file — treating the definition as "active"
/// until the next one.
fn enclosing_symbol(
    per_file: &HashMap<PathBuf, Vec<(u32, u32, SymbolId)>>,
    file: &Path,
    offset: u32,
) -> Option<SymbolId> {
    let spans = per_file.get(file)?; // already sorted by span_start
                                     // First try strict containment (most precise).
    let strict: Option<(u32, SymbolId)> = spans
        .iter()
        .filter(|(s, e, _)| (*s..=*e).contains(&offset))
        .map(|(s, e, id)| (e - s, *id))
        .min_by_key(|(len, _)| *len);
    if let Some((_, id)) = strict {
        return Some(id);
    }
    // Fallback: last definition that starts at or before `offset`.
    spans
        .iter()
        .rev()
        .find(|(s, _, _)| *s <= offset)
        .map(|(_, _, id)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_graph::HookEntry;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Regression: `structural_pass` MUST be idempotent. `build_graph` runs it
    /// once on a freshly-extracted graph, but `build_graph_incremental` (the
    /// daemon's watcher rebuild path) re-runs it on an already-structured graph.
    /// Before the fix, each re-run allocated fresh synthetic `Module` nodes and
    /// re-emitted a `BelongsTo` per symbol — the old edges survived (their
    /// symbols are not invalidated), so the containment layer duplicated on
    /// every rebuild and the daemon's persisted graph ratcheted upward
    /// (BelongsTo grew ~one-per-symbol per pass; the live graph reached ~3×
    /// its clean edge count on a large Java project).
    #[test]
    fn structural_pass_is_idempotent_on_rerun() {
        let mut g = SymbolGraph::new();
        // Two symbols in distinct files that share a synthetic module name
        // ("foo" via the crates/<name>/ convention) but have no real Module
        // node — exactly the case that synthesises a Module + BelongsTo edges.
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a",
            PathBuf::from("crates/foo/src/a.rs"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b",
            PathBuf::from("crates/foo/src/b.rs"),
            0,
            10,
        ));

        structural_pass(&mut g);
        let modules1 = g.nodes().filter(|n| n.kind == SymbolKind::Module).count();
        let belongs1 = g.edges().filter(|e| e.kind == EdgeKind::BelongsTo).count();
        let nodes1 = g.node_count();
        let edges1 = g.edge_count();
        assert!(modules1 >= 1, "expected a synthetic Module node");
        assert!(belongs1 >= 2, "expected a BelongsTo edge per symbol");

        // Re-run on the already-structured graph. The ratchet invariant: the
        // synthetic Module layer must NOT grow — no new Module nodes and no
        // duplicate BelongsTo edges, the two quantities that scaled with every
        // rebuild before the fix (BelongsTo by one-per-symbol, Module by one
        // per group). Re-running an unbounded number of times stays flat.
        for _ in 0..3 {
            structural_pass(&mut g);
        }
        assert_eq!(
            g.nodes().filter(|n| n.kind == SymbolKind::Module).count(),
            modules1,
            "structural_pass duplicated Module nodes on re-run"
        );
        assert_eq!(
            g.edges().filter(|e| e.kind == EdgeKind::BelongsTo).count(),
            belongs1,
            "structural_pass duplicated BelongsTo edges on re-run"
        );
        assert_eq!(g.node_count(), nodes1, "re-run added nodes");
        // NOTE: edge_count is NOT asserted equal. A synthetic Module created in
        // the first pass becomes a child of its representative file on the next
        // pass, so step 3 emits ONE File->Module `Contains` edge that the first
        // pass could not (the module did not exist yet). That is a bounded,
        // one-time delta — it dedups on every subsequent pass and never scales
        // with symbol count — so it does not ratchet like the bug above.
        let _ = edges1;
    }

    /// Regression: the watcher's `build_graph_incremental` re-runs the derive
    /// stages (structural_pass, resolve_references, derive_inherits) on an
    /// already-built graph. Those stages must be idempotent so a rebuild does
    /// not inflate the graph above a clean build — the daemon edge-ratchet bug,
    /// where each rebuild duplicated the containment layer (BelongsTo per
    /// symbol) and re-resolved Calls/References onto stale ExternalPackage
    /// stubs, ballooning a long-lived daemon to ~3x its true edge count.
    #[test]
    fn build_graph_incremental_does_not_inflate() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir =
            std::env::temp_dir().join(format!("cg-inc-idem-{}-{}", std::process::id(), nonce));
        let src = dir.join("src/main/java/com/app");
        std::fs::create_dir_all(&src).unwrap();
        // Java files under a synthetic module: a cross-file call (A -> B) plus an
        // unresolved external import (`Helper`) that is ALSO referenced — the
        // shape that duplicated on re-run before the fix. The first build mints
        // an ExternalPackage stub for `Helper` and drops the unresolved
        // `new Helper()` constructor call; a non-idempotent re-run would then
        // resolve that call onto the stub and grow the edge set.
        std::fs::write(
            src.join("A.java"),
            "package com.app;\nimport com.external.Helper;\npublic class A {\n  public void run() { new B().go(); Helper h = new Helper(); }\n}\n",
        )
        .unwrap();
        std::fs::write(
            src.join("B.java"),
            "package com.app;\npublic class B {\n  public void go() {}\n}\n",
        )
        .unwrap();

        let (mut g, _) = build_graph(&dir).unwrap();
        let clean_edges = g.edge_count();
        let belongs = |g: &SymbolGraph| g.edges().filter(|e| e.kind == EdgeKind::BelongsTo).count();
        let clean_belongs = belongs(&g);

        let a = src.join("A.java");
        let _ = build_graph_incremental(&dir, &mut g, std::slice::from_ref(&a));
        let after_first = g.edge_count();
        let first_belongs = belongs(&g);
        let _ = build_graph_incremental(&dir, &mut g, std::slice::from_ref(&a));
        let after_second = g.edge_count();

        let _ = std::fs::remove_dir_all(&dir);

        // Idempotent: a second identical rebuild changes nothing.
        assert_eq!(
            after_first, after_second,
            "incremental rebuild must be idempotent (no edge growth on re-run)"
        );
        // Never inflates above a clean build. (It may sit at or slightly below:
        // a changed file's doc / stack-graphs edges are intentionally dropped
        // until the next full build — but the derive layer must not grow.)
        assert!(
            after_first <= clean_edges,
            "incremental ({}) must not inflate above clean ({})",
            after_first,
            clean_edges
        );
        // The containment layer in particular must stay put — its per-symbol
        // duplication was the runaway term in the ratchet.
        assert_eq!(
            first_belongs, clean_belongs,
            "BelongsTo must stay at the clean count across an incremental rebuild"
        );
    }

    #[test]
    fn import_bindings_map_each_name_to_its_specifier() {
        // Each named import yields (imported_name, local_name, specifier), incl.
        // every specifier in a multi-import and — critically for per-specifier
        // resolution — the `as` alias kept as the LOCAL name distinct from the
        // IMPORTED name.
        let src = "import { exportToSvg, exportToCanvas } from \"../scene/export\";\n\
                   import { restore as restoreScene } from \"./restore\";\n\
                   import Foo from \"react\";\n";
        let imports = extract_import_bindings(std::path::Path::new("data/index.ts"), src);
        let has = |imported: &str, local: &str, spec: &str| {
            imports
                .iter()
                .any(|ni| ni.imported == imported && ni.local == local && ni.spec == spec)
        };
        assert!(
            has("exportToSvg", "exportToSvg", "../scene/export"),
            "exportToSvg binding missing: {imports:?}"
        );
        assert!(
            has("exportToCanvas", "exportToCanvas", "../scene/export"),
            "second specifier of a multi-import missing: {imports:?}"
        );
        assert!(
            has("restore", "restoreScene", "./restore"),
            "alias must keep imported='restore' distinct from local='restoreScene': {imports:?}"
        );
    }

    #[test]
    fn import_bindings_empty_for_non_ts_files() {
        assert!(extract_import_bindings(std::path::Path::new("a.go"), "import x").is_empty());
    }

    #[test]
    fn pick_resolve_targets_uses_import_binding_to_disambiguate() {
        use std::collections::HashMap;
        // Two same-name defs in different files/dirs (neither same-file/dir as
        // the caller, no qualified_name) — the classic ambiguous cross-file case.
        let a = SymbolId(1); // def in scene/export.ts
        let b = SymbolId(2); // def in utils/export.ts
        let caller = SymbolId(3);
        let ref_file = PathBuf::from("pkg/excalidraw/data/index.ts");
        let mut def_file = HashMap::new();
        def_file.insert(a, PathBuf::from("pkg/excalidraw/scene/export.ts"));
        def_file.insert(b, PathBuf::from("pkg/utils/src/export.ts"));
        def_file.insert(caller, ref_file.clone());
        let def_qualified: HashMap<SymbolId, String> = HashMap::new();

        // Without an import hint: step 4 drops the ambiguous (2-candidate) ref.
        let (none, _) = pick_resolve_targets(
            &[a, b],
            caller,
            &ref_file,
            &def_file,
            &def_qualified,
            "exportToSvg",
            &HashMap::new(),
        );
        assert!(
            none.is_empty(),
            "ambiguous cross-file ref is dropped without an import hint"
        );

        // With an import hint resolving to scene/export.ts: pick only `a`.
        let mut import_target = HashMap::new();
        import_target.insert(
            (ref_file.clone(), "exportToSvg".to_string()),
            PathBuf::from("pkg/excalidraw/scene/export.ts"),
        );
        let (chosen, origin) = pick_resolve_targets(
            &[a, b],
            caller,
            &ref_file,
            &def_file,
            &def_qualified,
            "exportToSvg",
            &import_target,
        );
        assert_eq!(
            chosen,
            vec![a],
            "import binding selects the imported def, not the other"
        );
        assert_eq!(origin, AnalysisOrigin::NameResolved);
    }

    #[test]
    fn pick_resolve_targets_drops_ambiguous_same_dir_names() {
        use std::collections::HashMap;
        // Flat crate layout: caller and many same-name defs (`new`) all live in
        // `src/`. Linking every sibling `new` produced a 12-way NameResolved
        // fan-out per call site; ambiguity must fall through and be dropped.
        let caller = SymbolId(10);
        let ref_file = PathBuf::from("crates/x/src/lib.rs");
        let defs: Vec<SymbolId> = (1..=3).map(SymbolId).collect();
        let mut def_file = HashMap::new();
        def_file.insert(defs[0], PathBuf::from("crates/x/src/a.rs"));
        def_file.insert(defs[1], PathBuf::from("crates/x/src/b.rs"));
        def_file.insert(defs[2], PathBuf::from("crates/x/src/c.rs"));
        def_file.insert(caller, ref_file.clone());
        let def_qualified: HashMap<SymbolId, String> = HashMap::new();

        let (chosen, _) = pick_resolve_targets(
            &defs,
            caller,
            &ref_file,
            &def_file,
            &def_qualified,
            "new",
            &HashMap::new(),
        );
        assert!(
            chosen.is_empty(),
            "multiple same-dir candidates are ambiguous and must be dropped, got {chosen:?}"
        );

        // A unique same-dir candidate still resolves at NameResolved.
        let (one, origin) = pick_resolve_targets(
            &defs[..1],
            caller,
            &ref_file,
            &def_file,
            &def_qualified,
            "new",
            &HashMap::new(),
        );
        assert_eq!(one, vec![defs[0]]);
        assert_eq!(origin, AnalysisOrigin::NameResolved);
    }

    #[test]
    fn looks_minified_uses_average_line_length() {
        // Formatted source — many short lines — is not minified.
        let formatted =
            "function a() {\n  return 1;\n}\nfunction b() {\n  return 2;\n}\n".repeat(50);
        assert!(!looks_minified(&formatted));
        // A real source file with one long generated/base64 line among normal
        // code must NOT be dropped (the average stays low).
        let mut with_long_line = "const x = 1;\n".repeat(300);
        with_long_line.push_str(&format!("const data = \"{}\";\n", "A".repeat(6000)));
        assert!(
            !looks_minified(&with_long_line),
            "one long line must not drop a real file"
        );
        // A bundle packed into a few enormous lines IS minified (high avg).
        let bundle = format!("function t(e){{return e}};{}", "a;".repeat(4000));
        assert!(bundle.len() >= 4096);
        assert!(looks_minified(&bundle));
    }

    #[test]
    fn export_star_reexport_de_orphans_target_symbols() {
        // `export * from './widget'` re-exports every PUBLIC symbol of widget.ts,
        // so they are consumed via the barrel. Without resolving the star, those
        // symbols (Widget, WidgetProps) look like dead code. A non-exported
        // helper must NOT gain a re-export edge (only the public surface flows
        // through `export *`).
        use coregraph_core::EdgeKind;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("widget.ts"),
            "export function Widget() {}\nexport interface WidgetProps { id: string; }\nfunction internalHelper() { return 1; }\n",
        )
        .unwrap();
        std::fs::write(root.join("barrel.ts"), "export * from './widget';\n").unwrap();
        let (graph, _) = build_graph(root).expect("build");

        let id_of = |name: &str| graph.nodes().find(|n| n.name == name).map(|n| n.id);
        let widget = id_of("Widget").expect("Widget node missing");
        let props = id_of("WidgetProps").expect("WidgetProps node missing");
        let helper = id_of("internalHelper").expect("internalHelper node missing");

        let reexported = |tgt| {
            graph
                .edges()
                .any(|e| e.to == tgt && e.kind == EdgeKind::Imports)
        };
        assert!(
            reexported(widget),
            "`export *` must give Widget an incoming re-export edge"
        );
        assert!(
            reexported(props),
            "`export *` must give WidgetProps an incoming re-export edge"
        );
        assert!(
            !reexported(helper),
            "non-exported internalHelper must NOT receive a re-export edge"
        );
    }

    #[test]
    fn export_star_chains_through_nested_barrels() {
        // Real barrels nest: `base/index.ts` re-exports `charts/` (itself a
        // barrel) which re-exports the leaf `area-chart.tsx`. Each `export *`
        // resolves its DIRECT target (a real file, even one with no public
        // symbols of its own), so the leaf component is de-orphaned by its inner
        // barrel. Uses `export const` (arrow component) — the dominant style.
        use coregraph_core::EdgeKind;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("base/charts")).unwrap();
        std::fs::write(root.join("base/index.ts"), "export * from './charts';\n").unwrap();
        std::fs::write(
            root.join("base/charts/index.ts"),
            "export * from './area-chart';\n",
        )
        .unwrap();
        std::fs::write(
            root.join("base/charts/area-chart.tsx"),
            "export const AreaChart = () => null;\n",
        )
        .unwrap();
        let (graph, _) = build_graph(root).expect("build");

        let area = graph
            .nodes()
            .find(|n| n.name == "AreaChart")
            .expect("AreaChart node missing")
            .id;
        assert!(
            graph
                .edges()
                .any(|e| e.to == area && e.kind == EdgeKind::Imports),
            "leaf AreaChart must be de-orphaned through the nested `export *` barrel chain"
        );
    }

    #[test]
    fn build_graph_produces_edges_not_just_nodes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (graph, files) = build_graph(root).expect("build");
        assert!(files > 0, "extractor crate should have source files");
        // Node count should exceed zero; edges may exist from mediators/matchers
        // on real source, but here we only assert the pipeline runs.
        assert!(graph.node_count() > 0);
    }

    #[test]
    fn aliased_named_import_resolves_per_specifier_to_ambiguous_target() {
        // `import { Route as X } from './a'` is a genuine use of a.ts's `Route`
        // even when several files export `Route` (the TanStack `routeTree`
        // pattern: one generated file imports `Route` from every route module,
        // each under a distinct alias). The generic `(file, name)` disambiguation
        // map cannot represent N same-named imports in one file, so each
        // specifier must resolve through ITS OWN module path. Without this, the
        // aliased `Route` consts are falsely orphaned.
        use coregraph_core::EdgeKind;
        let base = std::env::temp_dir().join(format!("cg_alias_import_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // Route files live in DISTINCT subdirectories (and a different dir from
        // the importer), so the same-file / same-directory heuristics cannot
        // connect them — the edge must come from per-specifier module-path
        // resolution, exactly as in a real `routes/<x>/` tree.
        std::fs::create_dir_all(base.join("routes/a")).unwrap();
        std::fs::create_dir_all(base.join("routes/b")).unwrap();
        std::fs::write(base.join("routes/a/index.ts"), "export const Route = 1;\n").unwrap();
        std::fs::write(base.join("routes/b/index.ts"), "export const Route = 2;\n").unwrap();
        std::fs::write(
            base.join("tree.ts"),
            "import { Route as ARoute } from './routes/a';\n\
             import { Route as BRoute } from './routes/b';\n\
             export const tree = [ARoute, BRoute];\n",
        )
        .unwrap();
        let (graph, _) = build_graph(&base).expect("build");
        let _ = std::fs::remove_dir_all(&base);

        let route_a = graph
            .nodes()
            .find(|n| n.name == "Route" && n.file.ends_with("routes/a/index.ts"))
            .expect("routes/a Route node")
            .id;
        let route_b = graph
            .nodes()
            .find(|n| n.name == "Route" && n.file.ends_with("routes/b/index.ts"))
            .expect("routes/b Route node")
            .id;
        let has_incoming_import = |id| {
            graph
                .edges()
                .any(|e| e.to == id && e.kind == EdgeKind::Imports)
        };
        assert!(
            has_incoming_import(route_a),
            "a.ts `Route` must gain an incoming Imports edge from `import {{ Route as ARoute }} from './a'`"
        );
        assert!(
            has_incoming_import(route_b),
            "b.ts `Route` must gain an incoming Imports edge from `import {{ Route as BRoute }} from './b'`"
        );
    }

    #[test]
    fn bare_specifier_resolves_via_exports_main_and_src_index() {
        let mut known = HashSet::new();
        known.insert(PathBuf::from("/r/packages/ui/src/index.tsx"));
        known.insert(PathBuf::from("/r/packages/api/src/index.ts"));
        known.insert(PathBuf::from("/r/packages/api/src/util.ts"));
        known.insert(PathBuf::from("/r/packages/api/lib/main.ts"));
        let packages = vec![
            ("@x/ui".to_string(), PathBuf::from("/r/packages/ui")),
            ("@x/api".to_string(), PathBuf::from("/r/packages/api")),
        ];
        let no_entries: HashMap<PathBuf, Vec<String>> = HashMap::new();
        // No cached entry candidates → falls back to src/index resolution.
        assert_eq!(
            resolve_ts_bare_specifier("@x/ui", &packages, &no_entries, &known),
            Some(PathBuf::from("/r/packages/ui/src/index.tsx"))
        );
        // Subpath import resolves inside the package root.
        assert_eq!(
            resolve_ts_bare_specifier("@x/api/src/util", &packages, &no_entries, &known),
            Some(PathBuf::from("/r/packages/api/src/util.ts"))
        );
        // Unknown package → None (external dependency).
        assert_eq!(
            resolve_ts_bare_specifier("react", &packages, &no_entries, &known),
            None
        );
        // A cached manifest entry wins over the conventional src/index
        // fallback — `lib/main.ts` is reachable only through the cache.
        let mut entries = HashMap::new();
        entries.insert(
            PathBuf::from("/r/packages/api"),
            vec!["./lib/main.ts".to_string()],
        );
        assert_eq!(
            resolve_ts_bare_specifier("@x/api", &packages, &entries, &known),
            Some(PathBuf::from("/r/packages/api/lib/main.ts"))
        );
    }

    #[test]
    fn jsx_use_of_workspace_package_component_yields_symbol_level_calls_edge() {
        // In an npm/pnpm workspace, `import { Card } from '@x/ui'` is a bare
        // specifier; without manifest-aware resolution the JSX `<Card/>` Call
        // reference cannot be disambiguated when another package exports the
        // same name, so the only surviving link is the file-level syntactic
        // fallback — which impact deliberately ignores. The Calls edge must
        // land on @x/ui's Card from a symbol-level source.
        use coregraph_core::EdgeKind;
        let base = std::env::temp_dir().join(format!("cg_ws_jsx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("packages/ui/src")).unwrap();
        std::fs::create_dir_all(base.join("packages/other/src")).unwrap();
        std::fs::create_dir_all(base.join("apps/web/src")).unwrap();
        std::fs::write(
            base.join("package.json"),
            r#"{"name":"mono","private":true,"workspaces":["packages/*","apps/*"]}"#,
        )
        .unwrap();
        std::fs::write(
            base.join("packages/ui/package.json"),
            r#"{"name":"@x/ui","version":"1.0.0","exports":{".":"./src/index.tsx"}}"#,
        )
        .unwrap();
        std::fs::write(
            base.join("packages/ui/src/index.tsx"),
            "export function Card() { return null; }\n",
        )
        .unwrap();
        std::fs::write(
            base.join("packages/other/package.json"),
            r#"{"name":"@x/other","version":"1.0.0","main":"src/index.ts"}"#,
        )
        .unwrap();
        std::fs::write(
            base.join("packages/other/src/index.ts"),
            "export function Card() { return 2; }\n",
        )
        .unwrap();
        std::fs::write(
            base.join("apps/web/package.json"),
            r#"{"name":"@x/web","private":true,"main":"src/page.tsx"}"#,
        )
        .unwrap();
        std::fs::write(
            base.join("apps/web/src/page.tsx"),
            "import { Card } from '@x/ui';\nexport function Page() { return <Card/>; }\n",
        )
        .unwrap();
        let (graph, _) = build_graph(&base).expect("build");
        let _ = std::fs::remove_dir_all(&base);

        let ui_card = graph
            .nodes()
            .find(|n| n.name == "Card" && n.file.ends_with("packages/ui/src/index.tsx"))
            .expect("@x/ui Card node")
            .id;
        let other_card = graph
            .nodes()
            .find(|n| n.name == "Card" && n.file.ends_with("packages/other/src/index.ts"))
            .expect("@x/other Card node")
            .id;
        let calls_to = |id| {
            graph
                .edges()
                .filter(|e| e.to == id && e.kind == EdgeKind::Calls)
                .collect::<Vec<_>>()
        };
        let ui_calls = calls_to(ui_card);
        assert!(
            !ui_calls.is_empty(),
            "JSX <Card/> must produce a Calls edge to the imported package's Card"
        );
        // The edge source must be a symbol (the Page function), not a File
        // container — that is what makes it visible to impact.
        let src_kinds: Vec<_> = ui_calls
            .iter()
            .filter_map(|e| graph.get_node(e.from).map(|n| n.kind.clone()))
            .collect();
        assert!(
            src_kinds.contains(&SymbolKind::Function),
            "Calls edge source should be the enclosing function, got {src_kinds:?}"
        );
        assert!(
            calls_to(other_card).is_empty(),
            "the non-imported same-named Card must NOT receive the call"
        );
    }

    #[test]
    fn build_graph_emits_documentation_layer() {
        // The extractor crate's own Rust sources carry many `///` doc comments,
        // so a full build must surface the documentation layer end-to-end —
        // this guards the Stage-2 merge survival of the doc pass (an edge made
        // inside `extract()` would be dropped; the post-stage must run instead).
        use coregraph_core::{EdgeKind, SymbolKind};
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (graph, _) = build_graph(root).expect("build");

        let doc_nodes = graph
            .nodes()
            .filter(|n| n.kind == SymbolKind::DocComment)
            .count();
        assert!(
            doc_nodes > 0,
            "expected DocComment nodes from `///` comments"
        );

        let doc_edges: Vec<_> = graph
            .edges()
            .filter(|e| e.kind == EdgeKind::Documents)
            .cloned()
            .collect();
        assert!(!doc_edges.is_empty(), "expected Documents edges");

        // Every Documents edge must go DocComment → (real symbol) within one file.
        for e in &doc_edges {
            let from = graph.get_node(e.from).expect("edge from-node exists");
            let to = graph.get_node(e.to).expect("edge to-node exists");
            assert_eq!(
                from.kind,
                SymbolKind::DocComment,
                "Documents edge must originate at a DocComment node"
            );
            assert_ne!(
                to.kind,
                SymbolKind::DocComment,
                "Documents edge must target a code symbol, not another doc"
            );
            assert_eq!(
                from.file, to.file,
                "a doc and the symbol it documents live in the same file"
            );
        }
    }

    #[test]
    fn build_graph_documents_go_and_python() {
        use coregraph_core::{EdgeKind, SymbolKind};
        // Build a temp project with a Go doc-comment and a Python docstring,
        // then assert the documentation layer attaches both through the full
        // pipeline (not just in isolation).
        let base = std::env::temp_dir().join(format!("cg_doc_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("g")).unwrap();
        std::fs::write(
            base.join("g/greet.go"),
            "package g\n\n// Greet greets.\nfunc Greet() {}\n",
        )
        .unwrap();
        std::fs::write(
            base.join("mod.py"),
            "def documented():\n    \"\"\"A doc.\"\"\"\n    return 1\n",
        )
        .unwrap();

        let (graph, _) = build_graph(&base).expect("build");
        let doc_targets: Vec<String> = graph
            .edges()
            .filter(|e| e.kind == EdgeKind::Documents)
            .filter_map(|e| graph.get_node(e.to).map(|n| n.name.clone()))
            .collect();
        let _ = std::fs::remove_dir_all(&base);

        assert!(
            doc_targets.contains(&"Greet".to_string()),
            "Go func Greet must get a Documents edge, got {doc_targets:?}"
        );
        assert!(
            doc_targets.contains(&"documented".to_string()),
            "Python documented() must get a Documents edge, got {doc_targets:?}"
        );
        // Sanity: the doc nodes exist too.
        assert!(graph.nodes().any(|n| n.kind == SymbolKind::DocComment));
    }

    #[test]
    fn build_graph_emits_mentions_from_doc_links() {
        use coregraph_core::{EdgeKind, SymbolKind};
        // foo's doc links [`bar`]; bar is defined in a sibling file. The mention
        // pass must emit a cross-file Mentions edge from foo's DocComment to bar.
        // (Verified in-process: the 0.60 edge is below the default export filter.)
        let base = std::env::temp_dir().join(format!("cg_mention_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(base.join("src/bar.rs"), "pub fn bar() {}\n").unwrap();
        std::fs::write(
            base.join("src/main.rs"),
            "mod bar;\n/// Calls [`bar`] to do the work.\npub fn foo() {}\n",
        )
        .unwrap();

        let (graph, _) = build_graph(&base).expect("build");
        let mention = graph.edges().find(|e| e.kind == EdgeKind::Mentions);
        let _ = std::fs::remove_dir_all(&base);

        let mention = mention.expect("expected a Mentions edge from foo's doc to bar");
        let from = graph.get_node(mention.from).unwrap();
        let to = graph.get_node(mention.to).unwrap();
        assert_eq!(
            from.kind,
            SymbolKind::DocComment,
            "mention must originate at a doc"
        );
        assert_eq!(to.name, "bar", "the [`bar`] link must resolve to bar");
        assert_ne!(from.file, to.file, "this mention is cross-file");
    }

    #[test]
    fn build_graph_emits_described_in_from_markdown() {
        use coregraph_core::{EdgeKind, SymbolKind};
        // A README references `Server` (a code symbol) in a section. The markdown
        // pass must create a DocSection node and a DescribedIn edge from the
        // Server symbol to it.
        let base = std::env::temp_dir().join(format!("cg_md_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(
            base.join("src/server.rs"),
            "pub struct Server { port: u16 }\n",
        )
        .unwrap();
        std::fs::write(
            base.join("README.md"),
            "# Overview\n\nThe `Server` listens on a port.\n\n## Unrelated\n\nNothing here.\n",
        )
        .unwrap();

        let (graph, _) = build_graph(&base).expect("build");
        let described = graph.edges().find(|e| e.kind == EdgeKind::DescribedIn);
        let has_section = graph.nodes().any(|n| n.kind == SymbolKind::DocSection);
        let _ = std::fs::remove_dir_all(&base);

        let described = described.expect("expected a DescribedIn edge from Server to a DocSection");
        let from = graph.get_node(described.from).unwrap();
        let to = graph.get_node(described.to).unwrap();
        assert_eq!(
            from.name, "Server",
            "DescribedIn must originate at the Server symbol"
        );
        assert_eq!(
            to.kind,
            SymbolKind::DocSection,
            "DescribedIn must target a DocSection"
        );
        assert!(
            to.name.contains("Overview"),
            "the section should be the Overview heading"
        );
        assert!(has_section);
    }

    #[test]
    fn doc_param_drift_flags_stale_param_only() {
        // greet documents `name` (real) and `oldArg` (removed). Only `oldArg`
        // must be flagged as drift; `name` must not.
        let base = std::env::temp_dir().join(format!("cg_drift_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(
            base.join("src/greet.ts"),
            "/**\n * Greets the caller.\n * @param name the name\n * @param oldArg removed\n */\nexport function greet(name: string): string { return name; }\n",
        )
        .unwrap();

        let (graph, _) = build_graph(&base).expect("build");
        let drift = find_doc_param_drift(&graph);
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(drift.len(), 1, "exactly one drift (oldArg), got {drift:?}");
        assert!(
            drift[0].detail.contains("oldArg"),
            "the stale param must be oldArg, got {:?}",
            drift[0].detail
        );
        assert!(
            !drift.iter().any(|d| d.detail.contains("`name`")),
            "the real param `name` must not be flagged"
        );
    }

    #[test]
    fn doc_param_drift_ignores_correct_docs() {
        // A doc whose @param matches the signature produces no drift.
        let base = std::env::temp_dir().join(format!("cg_drift_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(
            base.join("src/greet.ts"),
            "/**\n * @param name the name\n */\nexport function greet(name: string): string { return name; }\n",
        )
        .unwrap();
        let (graph, _) = build_graph(&base).expect("build");
        let drift = find_doc_param_drift(&graph);
        let _ = std::fs::remove_dir_all(&base);
        assert!(
            drift.is_empty(),
            "matching docs must not drift, got {drift:?}"
        );
    }

    #[test]
    fn hooks_fire_around_build() {
        let pre = Arc::new(AtomicUsize::new(0));
        let post = Arc::new(AtomicUsize::new(0));
        let p = pre.clone();
        let q = post.clone();

        let mut hooks = HookRegistry::new();
        hooks.register(HookEntry::pre("count-pre", "", move |_root| {
            p.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        hooks.register(HookEntry::post("count-post", "", move |_g| {
            q.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let _ = build_graph_with_hooks(root, &hooks).expect("build");
        assert_eq!(pre.load(Ordering::SeqCst), 1);
        assert_eq!(post.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn pre_hook_error_aborts_build() {
        let mut hooks = HookRegistry::new();
        hooks.register(HookEntry::pre("abort", "", |_root| {
            Err(anyhow::anyhow!("nope"))
        }));
        let root = Path::new(".");
        let result = build_graph_with_hooks(root, &hooks);
        assert!(result.is_err());
    }

    #[test]
    fn index_excluder_survives_malformed_pattern() {
        // Mirrors the query-side guarantee: one bad glob in [index].exclude must
        // not drop the user's other patterns NOR the built-in defaults at index
        // time. "a{b" errors on add_line (unclosed alternate group); "a[" does
        // not.
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg_dir = dir.path().join(".coregraph");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.toml"),
            "[index]\nexclude = [\"a{b\", \"generated/\"]\n",
        )
        .unwrap();
        let ex = load_index_excluder(dir.path());
        assert!(
            ex.is_excluded(&dir.path().join("generated/file.ts")),
            "valid pattern must survive a malformed sibling"
        );
        assert!(
            ex.is_excluded(&dir.path().join("build/Gen.java")),
            "built-in defaults must still apply alongside a malformed user pattern"
        );
        assert!(!ex.is_excluded(&dir.path().join("src/main.ts")));
    }

    #[test]
    fn index_excludes_default_build_dirs_without_config_or_git() {
        // No .coregraph/config.toml and no .git/.gitignore: the universal
        // default exclude patterns must still keep build outputs out of the
        // index. Before the fix the indexer applied NO defaults, so
        // build/Gen.java was parsed (the root cause of "indexing a Java project
        // is extremely slow" — thousands of generated files under build/).
        let dir = tempfile::tempdir().expect("tmpdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("build")).unwrap();
        std::fs::write(
            dir.path().join("src/Main.java"),
            "package a;\npublic class Main { void run(){ new Gen().go(); } }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("build/Gen.java"),
            "package a;\npublic class Gen { void go(){} }\n",
        )
        .unwrap();
        let (_g, files) = build_graph(dir.path()).expect("build_graph");
        assert_eq!(
            files, 1,
            "only src/Main.java should be indexed; build/ must be excluded by default"
        );
    }

    #[test]
    fn index_honors_gitignore_without_git_dir() {
        // A .gitignore but NO .git directory: the indexer must still honour it
        // (require_git(false)). Uses a non-default pattern so this exercises
        // gitignore specifically, not the built-in defaults. tempdir lives under
        // the system temp, which is not a git repo, so no ancestor .git applies.
        let dir = tempfile::tempdir().expect("tmpdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("ignored_dir")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored_dir/\n").unwrap();
        std::fs::write(
            dir.path().join("src/Main.java"),
            "package a;\npublic class Main {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ignored_dir/Skip.java"),
            "package a;\npublic class Skip {}\n",
        )
        .unwrap();
        let (_g, files) = build_graph(dir.path()).expect("build_graph");
        assert_eq!(
            files, 1,
            ".gitignore must be honoured even without a .git directory"
        );
    }

    #[test]
    fn string_match_max_files_reads_config_with_default() {
        let dir = tempfile::tempdir().expect("tmpdir");
        assert_eq!(
            string_match_max_files(dir.path()),
            8,
            "default when no config"
        );
        let cfg_dir = dir.path().join(".coregraph");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.toml"),
            "[index]\nstring_match_max_files = 0\n",
        )
        .unwrap();
        assert_eq!(
            string_match_max_files(dir.path()),
            0,
            "explicit 0 = unlimited"
        );
        std::fs::write(
            cfg_dir.join("config.toml"),
            "[index]\nstring_match_max_files = 25\n",
        )
        .unwrap();
        assert_eq!(string_match_max_files(dir.path()), 25);
    }
}
