//! Detects cross-file inconsistencies in the symbol graph.
//!
//! Three categories, described in `docs/use-cases.md §4`:
//! - `EnumMismatch`: same local enum-variant name in different enums.
//! - `ApiPath`: two `api_path::` route-string literals in different files
//!   that are near-misses of each other (edit distance 1–2, length diff
//!   ≤ 20%). The detector has no server/client notion — any two such
//!   literals in different files qualify regardless of origin.
//! - `ConfigKey`: code-side config key reference (`@Value`, `process.env`,
//!   `os.environ`) that doesn't match any ConfigKey defined in the
//!   project's config files, or vice-versa.

use coregraph_core::{SymbolKind, SymbolNode};
use coregraph_graph::SymbolGraph;

/// Discriminator used by the CLI to group the different detection outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InconsistencyCategory {
    EnumMismatch,
    ApiPath,
    ConfigKey,
}

impl InconsistencyCategory {
    pub fn label(&self) -> &'static str {
        match self {
            InconsistencyCategory::EnumMismatch => "enum-mismatch",
            InconsistencyCategory::ApiPath => "api-path",
            InconsistencyCategory::ConfigKey => "config-key",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InconsistencyReport {
    pub category: InconsistencyCategory,
    pub node_a: SymbolNode,
    pub node_b: SymbolNode,
    /// Category-specific diagnostic:
    /// - EnumMismatch: the shared local name
    /// - ApiPath: a human-readable summary like "/api/v1/cards vs /api/v1/card"
    /// - ConfigKey: "missing-key" or "unused-key" + the key
    pub shared_value: String,
}

/// Run all detectors and return the combined report set.
pub fn find_inconsistencies(graph: &SymbolGraph) -> Vec<InconsistencyReport> {
    let mut reports = find_enum_mismatches(graph);
    reports.extend(find_api_path_mismatches(graph));
    reports.extend(find_config_key_mismatches(graph));
    reports
}

/// Enum-mismatch detector.
///
/// Two `EnumVariant` nodes are a mismatch when they share the same *local*
/// variant name (normalized for cross-language case) but belong to different
/// enums. The previous implementation only walked `StringMatch` edges, which
/// required the pairs to already be linked by the value matcher. That worked
/// when variants were stored by bare name (`ACTIVE`) but never fired once
/// extractors started disambiguating them (`Role.ACTIVE` vs `State.ACTIVE`),
/// because the value-index keys no longer collided. We instead group variants
/// by their local name directly.
/// Universal sentinel enum-variant names present in most enums; their
/// cross-enum co-occurrence is deliberate, not an inconsistency. Kept minimal
/// and universal (not project-specific) per the no-hardcoding policy — a future
/// `[inconsistencies] enum_ignore_names` config key can extend it. Compared in
/// `normalize_enum_literal` form (lowercase, alphanumeric only).
const ENUM_SENTINEL_NAMES: &[&str] = &["none", "default", "unknown"];

pub fn find_enum_mismatches(graph: &SymbolGraph) -> Vec<InconsistencyReport> {
    use std::collections::HashMap;
    let mut by_local: HashMap<String, Vec<&SymbolNode>> = HashMap::new();
    for node in graph.nodes() {
        if node.kind != SymbolKind::EnumVariant {
            continue;
        }
        let local = normalize_enum_literal(local_name(&node.name));
        if local.is_empty() || ENUM_SENTINEL_NAMES.contains(&local.as_str()) {
            continue;
        }
        by_local.entry(local).or_default().push(node);
    }

    let mut reports = vec![];
    for (local, variants) in &by_local {
        if variants.len() < 2 {
            continue;
        }
        // Emit a report for every distinct (parent_a, parent_b) pair so
        // downstream tools can see every pairwise clash.
        for i in 0..variants.len() {
            for j in (i + 1)..variants.len() {
                let a = variants[i];
                let b = variants[j];
                let parent_a = parent_name(&a.name);
                let parent_b = parent_name(&b.name);
                if parent_a == parent_b {
                    // Same enum across files (e.g. declared twice) — not a
                    // mismatch in the sense this detector reports.
                    continue;
                }
                reports.push(InconsistencyReport {
                    category: InconsistencyCategory::EnumMismatch,
                    node_a: a.clone(),
                    node_b: b.clone(),
                    shared_value: local.clone(),
                });
            }
        }
    }
    reports
}

/// API-path detector. Pairs StringLiteral nodes whose `name` starts with the
/// `api_path::` marker prefix (set by `string_literal_extractor`), looking
/// for near-miss pairs. The only locality filter is that the two literals
/// live in different files; there is no server/client distinction, so any
/// two such literals across files qualify (two server files or two client
/// files included). A pair is a near-miss when:
///
/// - edit distance = 1 or 2
/// - length difference ≤ 20%
pub fn find_api_path_mismatches(graph: &SymbolGraph) -> Vec<InconsistencyReport> {
    use coregraph_extractor_api_path_helper::{strip_prefix_if_api_path, PATH_PREFIX};
    let _ = PATH_PREFIX; // suppress unused warning on the marker constant
    let mut literals: Vec<(&SymbolNode, &str)> = graph
        .nodes()
        .filter(|n| n.kind == SymbolKind::StringLiteral)
        .filter_map(|n| {
            let raw = strip_prefix_if_api_path(&n.name)?;
            Some((n, raw))
        })
        .collect();
    // Deterministic order helps tests and repeat runs.
    literals.sort_by_key(|(_, s)| s.len());

    let mut reports = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for i in 0..literals.len() {
        let (n_a, path_a) = literals[i];
        for (n_b, path_b) in literals.iter().skip(i + 1).copied() {
            if n_a.file == n_b.file {
                continue;
            }
            if path_a == path_b {
                continue; // identical path isn't a mismatch
            }
            if !length_diff_within(path_a, path_b, 20) {
                continue;
            }
            let dist = levenshtein(path_a, path_b);
            if !(1..=2).contains(&dist) {
                continue;
            }
            // Paths that differ only by an API version segment (`/v1/` vs `/v2/`)
            // are intentionally distinct versions, not a cross-file path typo.
            if differs_only_by_api_version(path_a, path_b) {
                continue;
            }
            let key = {
                let (x, y) = if path_a < path_b {
                    (path_a, path_b)
                } else {
                    (path_b, path_a)
                };
                (x.to_string(), y.to_string())
            };
            if !seen.insert(key) {
                continue;
            }
            let summary = format!("{} vs {}", path_a, path_b);
            reports.push(InconsistencyReport {
                category: InconsistencyCategory::ApiPath,
                node_a: n_a.clone(),
                node_b: n_b.clone(),
                shared_value: summary,
            });
        }
    }
    reports
}

/// Config-key detector. Classifies every ConfigKey node into:
/// - *definitions*: name does NOT start with the `config_ref::` marker
///   (emitted by `config_extractor` from YAML/TOML/JSON files)
/// - *references*: name starts with `config_ref::` (emitted by
///   `string_literal_extractor` from Java/TS/Python/Go code)
///
/// Reports:
/// - "missing-key": a reference whose key has no matching definition
/// - "unused-key": a definition with no matching reference
pub fn find_config_key_mismatches(graph: &SymbolGraph) -> Vec<InconsistencyReport> {
    use coregraph_extractor_api_path_helper::{
        is_env_ref, strip_prefix_if_config_ref, CONFIG_PREFIX,
    };
    let _ = CONFIG_PREFIX;

    let mut definitions: std::collections::HashMap<String, &SymbolNode> =
        std::collections::HashMap::new();
    let mut references: std::collections::HashMap<String, Vec<&SymbolNode>> =
        std::collections::HashMap::new();

    for n in graph.nodes() {
        if n.kind != SymbolKind::ConfigKey {
            continue;
        }
        if let Some(raw) = strip_prefix_if_config_ref(&n.name) {
            references
                .entry(canonicalize_config_key(raw))
                .or_default()
                .push(n);
        } else if is_env_ref(&n.name) {
            // Runtime env-var reference — not a config-file key. It cannot be a
            // "missing" key (no file defines it) nor a definition.
            continue;
        } else {
            definitions.insert(canonicalize_config_key(&n.name), n);
        }
    }

    // File-liveness: a config file is "live application config" only if code
    // references at least one of its keys. Files with zero referenced keys are
    // non-application data (generated docs, i18n bundles, infra manifests) or
    // config consumed by whole-subtree binding we cannot observe per-key —
    // either way, flagging their keys as "unused" is a false positive. We gate
    // the unused-key pass on liveness (precision over recall). KNOWN TRADE-OFF:
    // a config file that is genuinely 100% dead is also unreferenced, so it is
    // not reported; that recall loss is deliberate (the unreferenced-file signal
    // cannot distinguish dead config from non-config data).
    let mut live_files: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for n in graph.nodes() {
        if n.kind != SymbolKind::ConfigKey {
            continue;
        }
        if strip_prefix_if_config_ref(&n.name).is_some() || is_env_ref(&n.name) {
            continue; // reference node (config or runtime-env), not a definition
        }
        let key = canonicalize_config_key(&n.name);
        if references.contains_key(&key) {
            live_files.insert(n.file.to_path_buf());
        }
    }

    let mut reports = Vec::new();

    // Missing: referenced in code but never defined. Keys are matched on their
    // canonical form, but reported with the author's original spelling.
    for (key, ref_nodes) in &references {
        if !definitions.contains_key(key) {
            for ref_node in ref_nodes {
                let original = strip_prefix_if_config_ref(&ref_node.name).unwrap_or(&ref_node.name);
                reports.push(InconsistencyReport {
                    category: InconsistencyCategory::ConfigKey,
                    node_a: (*ref_node).clone(),
                    node_b: (*ref_node).clone(),
                    shared_value: format!("missing-key: {}", original),
                });
            }
        }
    }

    // Unused: defined in config but never referenced.
    for (key, def_node) in &definitions {
        if !references.contains_key(key) {
            // Only report for files the code demonstrably reads from.
            if !live_files.contains(def_node.file.as_ref()) {
                continue;
            }
            reports.push(InconsistencyReport {
                category: InconsistencyCategory::ConfigKey,
                node_a: (*def_node).clone(),
                node_b: (*def_node).clone(),
                shared_value: format!("unused-key: {}", def_node.name),
            });
        }
    }

    reports
}

/// Canonical form of a config key for matching: drop the separators that vary
/// between the config-file spelling and the code spelling (`.`, `_`, `-`) and
/// lowercase, so `maxPoolSize` (YAML) == `max.pool.size` (`@Value`) ==
/// `MAX_POOL_SIZE` (`process.env`) all collapse to `maxpoolsize`. Both the
/// definition and reference sides are keyed by this form, eliminating the
/// missing+unused double-fire for the same setting written different ways.
fn canonicalize_config_key(key: &str) -> String {
    key.chars()
        .filter(|c| *c != '.' && *c != '_' && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// True when two API paths are identical except for a single version segment
/// (`v1` vs `v2`, `v1` vs `v3`). Such pairs are intentionally distinct API
/// versions co-existing, not a cross-file path typo, so the near-miss
/// detector must not flag them. A non-version difference (or more than one
/// differing segment) returns false so genuine typos still surface.
fn differs_only_by_api_version(a: &str, b: &str) -> bool {
    let sa: Vec<&str> = a.split('/').collect();
    let sb: Vec<&str> = b.split('/').collect();
    if sa.len() != sb.len() {
        return false;
    }
    let is_version = |s: &str| {
        let bytes = s.as_bytes();
        bytes.len() >= 2 && bytes[0] == b'v' && bytes[1..].iter().all(u8::is_ascii_digit)
    };
    let mut version_diff_seen = false;
    for (x, y) in sa.iter().zip(sb.iter()) {
        if x == y {
            continue;
        }
        if !version_diff_seen && is_version(x) && is_version(y) {
            version_diff_seen = true;
        } else {
            return false;
        }
    }
    version_diff_seen
}

/// Max 20% length delta between two API paths — suppresses `/api` vs
/// `/api/v1/something-much-longer` noise.
fn length_diff_within(a: &str, b: &str, percent: u32) -> bool {
    let (longer, shorter) = if a.len() >= b.len() {
        (a.len(), b.len())
    } else {
        (b.len(), a.len())
    };
    if longer == 0 {
        return true;
    }
    let diff = (longer - shorter) * 100;
    diff <= percent as usize * longer
}

/// Classic iterative Levenshtein with u16 rows to keep it cache-friendly
/// for short API paths.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let n = a_bytes.len();
    let m = b_bytes.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<u16> = (0..=m as u16).collect();
    let mut curr: Vec<u16> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i as u16;
        for j in 1..=m {
            let cost = if a_bytes[i - 1] == b_bytes[j - 1] {
                0
            } else {
                1
            };
            curr[j] = std::cmp::min(
                std::cmp::min(prev[j] + 1, curr[j - 1] + 1),
                prev[j - 1] + cost,
            );
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m] as usize
}

/// Normalize an enum-like literal so `CARD_READER` (Java),
/// `CardReader` (TypeScript), and `card_reader` (Python) compare equal.
fn normalize_enum_literal(value: &str) -> String {
    let mut s = String::new();
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
        }
    }
    s
}

/// Extract the local name: part after the last '.' separator.
fn local_name(qualified_name: &str) -> &str {
    match qualified_name.rfind('.') {
        Some(pos) => &qualified_name[pos + 1..],
        None => qualified_name,
    }
}

/// Extract the parent name: part before the last '.' separator.
fn parent_name(qualified_name: &str) -> &str {
    match qualified_name.rfind('.') {
        Some(pos) => &qualified_name[..pos],
        None => "",
    }
}

// Internal helper module to avoid a circular dependency with coregraph-extractor.
// The string_literal_extractor owns the canonical marker prefixes; we mirror
// them here so the query crate doesn't need to depend on the extractor crate.
mod coregraph_extractor_api_path_helper {
    pub const PATH_PREFIX: &str = "api_path::";
    pub const CONFIG_PREFIX: &str = "config_ref::";
    /// Runtime env-var reference marker (`process.env`, `os.environ`,
    /// `std::env::var`). Distinct from `config_ref::` so the detector can ignore
    /// it: an env var is runtime-provided and has no config-file definition.
    pub const ENV_PREFIX: &str = "env_ref::";

    pub fn strip_prefix_if_api_path(name: &str) -> Option<&str> {
        name.strip_prefix(PATH_PREFIX)
    }

    pub fn strip_prefix_if_config_ref(name: &str) -> Option<&str> {
        name.strip_prefix(CONFIG_PREFIX)
    }

    pub fn is_env_ref(name: &str) -> bool {
        name.starts_with(ENV_PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolNode};
    use std::path::PathBuf;

    fn insert_variant(
        g: &mut SymbolGraph,
        qualified_name: &str,
        file: &str,
    ) -> coregraph_core::SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::EnumVariant,
            qualified_name,
            PathBuf::from(file),
            0,
            10,
        ))
    }

    fn insert_string_match(
        g: &mut SymbolGraph,
        from: coregraph_core::SymbolId,
        to: coregraph_core::SymbolId,
    ) {
        g.insert_edge(DirectEdge::new(
            from,
            to,
            EdgeKind::StringMatch,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.7),
            PathBuf::from("a.rs"),
        ));
    }

    fn insert_api_path(g: &mut SymbolGraph, raw_path: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            format!("api_path::{}", raw_path),
            PathBuf::from(file),
            0,
            raw_path.len() as u32,
        ))
    }

    fn insert_config_ref(g: &mut SymbolGraph, key: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::ConfigKey,
            format!("config_ref::{}", key),
            PathBuf::from(file),
            0,
            key.len() as u32,
        ))
    }

    fn insert_config_def(g: &mut SymbolGraph, key: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::ConfigKey,
            key.to_string(),
            PathBuf::from(file),
            0,
            key.len() as u32,
        ))
    }

    #[test]
    fn detects_cross_enum_inconsistency() {
        let mut g = SymbolGraph::new();
        let a = insert_variant(&mut g, "EnumA.Active", "a.rs");
        let b = insert_variant(&mut g, "EnumB.Active", "b.rs");
        insert_string_match(&mut g, a, b);
        let reports = find_inconsistencies(&g);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].category, InconsistencyCategory::EnumMismatch);
        assert_eq!(reports[0].shared_value, "active");
    }

    #[test]
    fn common_sentinel_enum_names_not_flagged() {
        // NONE / DEFAULT / UNKNOWN are universal sentinel variants present in
        // most enums; their cross-enum co-occurrence is deliberate, not a
        // mismatch. (A distinctive shared name like `Active` still flags — see
        // detects_cross_enum_inconsistency.)
        let mut g = SymbolGraph::new();
        insert_variant(&mut g, "StatusA.None", "a.rs");
        insert_variant(&mut g, "StatusB.None", "b.rs");
        insert_variant(&mut g, "ModeA.UNKNOWN", "c.rs");
        insert_variant(&mut g, "ModeB.Unknown", "d.rs");
        let reports = find_enum_mismatches(&g);
        assert!(
            reports.is_empty(),
            "universal sentinel enum names must not flag: {reports:?}"
        );
    }

    #[test]
    fn cross_language_enum_case_normalized() {
        // Java CARD_READER → "cardreader"; TypeScript CardReader → "cardreader"
        let mut g = SymbolGraph::new();
        let a = insert_variant(&mut g, "DeviceTypeJ.CARD_READER", "J.java");
        let b = insert_variant(&mut g, "DeviceTypeT.CardReader", "t.ts");
        insert_string_match(&mut g, a, b);
        let reports = find_inconsistencies(&g);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].shared_value, "cardreader");
    }

    #[test]
    fn api_path_near_miss_flagged() {
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/cards", "server.java");
        insert_api_path(&mut g, "/api/v1/card", "client.ts");
        let reports = find_api_path_mismatches(&g);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].category, InconsistencyCategory::ApiPath);
        assert!(reports[0].shared_value.contains("/api/v1/cards"));
        assert!(reports[0].shared_value.contains("/api/v1/card"));
    }

    #[test]
    fn identical_api_paths_not_flagged() {
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/cards", "server.java");
        insert_api_path(&mut g, "/api/v1/cards", "client.ts");
        let reports = find_api_path_mismatches(&g);
        assert!(reports.is_empty());
    }

    #[test]
    fn api_paths_differing_only_by_version_not_flagged() {
        // `/api/v1/cards` and `/api/v2/cards` are intentionally distinct API
        // versions, not a client/server typo — must not be a near-miss report.
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/cards", "server.java");
        insert_api_path(&mut g, "/api/v2/cards", "client.ts");
        let reports = find_api_path_mismatches(&g);
        assert!(
            reports.is_empty(),
            "version-only difference must not flag: {reports:?}"
        );
    }

    #[test]
    fn distant_api_paths_not_flagged() {
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/users", "server.java");
        insert_api_path(&mut g, "/api/v1/settings", "client.ts");
        let reports = find_api_path_mismatches(&g);
        assert!(reports.is_empty());
    }

    #[test]
    fn missing_config_key_flagged() {
        let mut g = SymbolGraph::new();
        insert_config_def(&mut g, "server.port", "app.yml");
        insert_config_ref(&mut g, "server.port", "A.java");
        insert_config_ref(&mut g, "feature.caching.enabled", "B.java");
        let reports = find_config_key_mismatches(&g);
        let missing = reports
            .iter()
            .find(|r| r.shared_value.starts_with("missing-key"))
            .expect("missing-key reported");
        assert!(missing.shared_value.contains("feature.caching.enabled"));
    }

    #[test]
    fn unused_config_key_flagged() {
        let mut g = SymbolGraph::new();
        insert_config_def(&mut g, "server.port", "app.yml");
        insert_config_def(&mut g, "unused.key", "app.yml");
        insert_config_ref(&mut g, "server.port", "A.java");
        let reports = find_config_key_mismatches(&g);
        let unused = reports
            .iter()
            .find(|r| r.shared_value.starts_with("unused-key"))
            .expect("unused-key reported");
        assert!(unused.shared_value.contains("unused.key"));
    }

    #[test]
    fn config_key_matching_is_separator_and_case_insensitive() {
        // A camelCase config definition and a dotted/snake reference for the
        // same key must match — no missing-key, no unused-key — so the same
        // setting written `maxPoolSize` in YAML and `max.pool.size` in a
        // `@Value` does not double-fire.
        let mut g = SymbolGraph::new();
        insert_config_def(&mut g, "maxPoolSize", "app.yml");
        insert_config_ref(&mut g, "max.pool.size", "A.java");
        let reports = find_config_key_mismatches(&g);
        assert!(
            reports.is_empty(),
            "camelCase def must match dotted ref after canonicalization: {reports:?}"
        );
    }

    #[test]
    fn runtime_env_ref_is_neither_missing_nor_unused() {
        // A runtime env-var reference (process.env.X / os.environ / env::var) is
        // tagged env_ref:: and references runtime state, not a config file — it
        // must produce no config-key inconsistency in either direction.
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::ConfigKey,
            "env_ref::node.env",
            "App.tsx",
            0,
            1,
        ));
        assert!(
            find_config_key_mismatches(&g).is_empty(),
            "runtime env-var reference must not be a missing/unused config key"
        );
    }

    #[test]
    fn unused_keys_suppressed_for_unreferenced_file() {
        // A config file none of whose keys are referenced by code is treated as
        // non-application data (generated docs, i18n bundles, infra manifests),
        // and its keys are NOT reported as unused — precision over recall. A
        // separate file with a referenced key proves the gate is per-file.
        let mut g = SymbolGraph::new();
        insert_config_def(&mut g, "pages.title", "docs/api/pages.json");
        insert_config_def(&mut g, "pages.body", "docs/api/pages.json");
        insert_config_def(&mut g, "server.port", "app.yml");
        insert_config_def(&mut g, "server.unused", "app.yml");
        insert_config_ref(&mut g, "server.port", "A.java");

        let unused: Vec<String> = find_config_key_mismatches(&g)
            .into_iter()
            .filter(|r| r.shared_value.starts_with("unused-key"))
            .map(|r| r.shared_value)
            .collect();
        assert!(
            !unused.iter().any(|u| u.contains("pages.")),
            "keys of an unreferenced file must be suppressed: {unused:?}"
        );
        assert!(
            unused.iter().any(|u| u.contains("server.unused")),
            "a live file's genuinely-unused key must still report: {unused:?}"
        );
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("cards", "card"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}
