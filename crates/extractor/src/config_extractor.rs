use std::path::Path;

use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
use coregraph_graph::SymbolGraph;

use crate::{ExtractError, SymbolExtractor};

pub struct ConfigExtractor;

impl ConfigExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConfigExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor for ConfigExtractor {
    fn language_name(&self) -> &'static str {
        "Config"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["yaml", "yml", "toml", "json", "properties"]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        // Build-system manifest files (Cargo.toml, package.json, pyproject.toml,
        // etc.) share file extensions with application configuration, but their
        // keys (`workspace.dependencies.tokio.features`, `scripts.build`, ...)
        // are NOT config references the inconsistency detector should track.
        // Treating them as such poisons `coregraph inconsistencies` with
        // thousands of `unused-key` reports for every package declaration.
        //
        // We skip them by filename. The list is universal ecosystem
        // convention — not project-specific — so it belongs in library code.
        if is_build_manifest(path) {
            return Ok(());
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let keys: Vec<String> = match ext.as_str() {
            "yaml" | "yml" => extract_yaml_keys(source, path)?,
            "toml" => extract_toml_keys(source, path)?,
            "json" => extract_json_keys(source, path)?,
            "properties" => extract_properties_keys(source),
            _ => return Ok(()),
        };

        for key in keys {
            // Find the deepest path segment (e.g. "server.port" → "port") in the
            // source — its first occurrence anywhere in the file. If the segment
            // is not found, the span falls back to byte offset 0.
            let last = key.rsplit('.').next().unwrap_or(&key);
            let span_start = source.find(last).unwrap_or(0) as u32;
            let span_end = span_start + last.len() as u32;
            let node = SymbolNode::new(
                SymbolId(0),
                SymbolKind::ConfigKey,
                key,
                path,
                span_start,
                span_end,
            );
            graph.insert_node(node);
        }

        Ok(())
    }
}

/// Parses a Java `.properties` file: each non-comment line is `key=value` or
/// `key:value` (the key is everything before the first `=`/`:`), or a bare key
/// with no value. Lines starting with `#` or `!` are comments. Keys are already
/// dotted (`spring.datasource.url`) so each is emitted verbatim as a ConfigKey.
fn extract_properties_keys(source: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            continue;
        }
        let key = match trimmed.find(['=', ':']) {
            Some(i) => trimmed[..i].trim(),
            None => trimmed.trim(),
        };
        if !key.is_empty() {
            keys.push(key.to_string());
        }
    }
    keys
}

fn extract_yaml_keys(source: &str, path: &Path) -> Result<Vec<String>, ExtractError> {
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(source).map_err(|_| ExtractError::ParseFailed {
            path: path.to_path_buf(),
        })?;

    let mut keys = Vec::new();
    walk_yaml(&parsed, "", &mut keys);
    Ok(keys)
}

fn walk_yaml(value: &serde_yaml::Value, prefix: &str, out: &mut Vec<String>) {
    // Only emit referenceable leaf keys: a key whose value is a scalar or a
    // list. Mapping (container) keys are not emitted — they recurse to reach
    // their leaves. Array elements are never emitted: a synthetic `[i]` key can
    // never match a code reference and is structurally guaranteed to be unused.
    if let serde_yaml::Value::Mapping(map) = value {
        for (k, v) in map {
            let key_str = match k {
                serde_yaml::Value::String(s) => s.clone(),
                other => format!("{:?}", other),
            };
            let path = join_path(prefix, &key_str);
            if matches!(v, serde_yaml::Value::Mapping(_)) {
                walk_yaml(v, &path, out);
            } else {
                out.push(path);
            }
        }
    }
}

fn extract_toml_keys(source: &str, path: &Path) -> Result<Vec<String>, ExtractError> {
    let parsed: toml::Value = toml::from_str(source).map_err(|_| ExtractError::ParseFailed {
        path: path.to_path_buf(),
    })?;

    let mut keys = Vec::new();
    walk_toml(&parsed, "", &mut keys);
    Ok(keys)
}

fn walk_toml(value: &toml::Value, prefix: &str, out: &mut Vec<String>) {
    // Leaf-only (see `walk_yaml`): emit scalar/array-valued keys, recurse into
    // tables, never emit `[i]` array-index keys.
    if let toml::Value::Table(table) = value {
        for (k, v) in table {
            let path = join_path(prefix, k);
            if matches!(v, toml::Value::Table(_)) {
                walk_toml(v, &path, out);
            } else {
                out.push(path);
            }
        }
    }
}

fn extract_json_keys(source: &str, path: &Path) -> Result<Vec<String>, ExtractError> {
    let parsed: serde_json::Value =
        serde_json::from_str(source).map_err(|_| ExtractError::ParseFailed {
            path: path.to_path_buf(),
        })?;

    let mut keys = Vec::new();
    walk_json(&parsed, "", &mut keys);
    Ok(keys)
}

fn walk_json(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    // Leaf-only (see `walk_yaml`): emit scalar/array-valued keys, recurse into
    // objects, never emit `[i]` array-index keys.
    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            let path = join_path(prefix, k);
            if matches!(v, serde_json::Value::Object(_)) {
                walk_json(v, &path, out);
            } else {
                out.push(path);
            }
        }
    }
}

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{}.{}", prefix, key)
    }
}

/// True if `path`'s basename matches a known build-system manifest. These
/// files live in TOML/JSON/YAML just like application configuration, but
/// their contents describe packages and the build graph — not runtime
/// config keys that code references. The inconsistency detector must not
/// treat `workspace.dependencies.tokio.features` as an application setting.
///
/// The list tracks universal packaging conventions (Cargo, npm, pip,
/// Gradle, Maven, Go modules, Composer, Deno, Bun, JVM build tools,
/// JS toolchain configs). It is NOT a project-specific heuristic —
/// every Rust crate uses Cargo.toml, every Node app uses package.json,
/// every TS project ships tsconfig.json, etc. Adding a new ecosystem
/// here is the right place to declare the skip; embedding paths in
/// callers would be.
///
/// Patterns:
/// - Exact name match against `MANIFEST_NAMES`.
/// - Prefix-and-suffix match for variants like `tsconfig.build.json`,
///   `tsconfig.test.json` (anything starting with `tsconfig.` and
///   ending with `.json`).
fn is_build_manifest(path: &Path) -> bool {
    const MANIFEST_NAMES: &[&str] = &[
        // Rust
        "Cargo.toml",
        "Cargo.lock",
        // Node / npm / yarn / pnpm / bun
        "package.json",
        "package-lock.json",
        "pnpm-lock.yaml",
        "pnpm-workspace.yaml",
        "yarn.lock",
        "lerna.json",
        "bun.lockb",
        "bun.lock",
        // TypeScript / JavaScript toolchain configs
        "tsconfig.json",
        "jsconfig.json",
        "babel.config.js",
        "babel.config.json",
        ".babelrc",
        ".babelrc.json",
        ".eslintrc",
        ".eslintrc.json",
        ".eslintrc.yaml",
        ".eslintrc.yml",
        ".prettierrc",
        ".prettierrc.json",
        ".prettierrc.yaml",
        ".prettierrc.yml",
        "tslint.json",
        // Deno
        "deno.json",
        "deno.jsonc",
        "deno.lock",
        // Python
        "pyproject.toml",
        "Pipfile",
        "Pipfile.lock",
        "poetry.lock",
        "setup.cfg",
        // JVM (Gradle / Maven / sbt / mill)
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        "pom.xml",
        // Go
        "go.mod",
        "go.sum",
        "go.work",
        "go.work.sum",
        // PHP / Ruby
        "composer.json",
        "composer.lock",
        "Gemfile",
        "Gemfile.lock",
    ];
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if MANIFEST_NAMES.contains(&name) {
        return true;
    }
    // `tsconfig.<flavour>.json` — TS projects routinely ship multiple
    // build configs (tsconfig.build.json, tsconfig.test.json, …). All
    // describe the build graph, none describe runtime app config.
    if name.starts_with("tsconfig.") && name.ends_with(".json") {
        return true;
    }
    false
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
        let extractor = ConfigExtractor::new();
        let path = fixture_path(subdir, filename);
        let source = std::fs::read_to_string(&path).expect("fixture not found");
        let mut graph = SymbolGraph::new();
        extractor
            .extract(&path, &source, &mut graph)
            .expect("extraction failed");
        graph
    }

    #[test]
    fn extracts_properties_keys() {
        // Java `.properties` (Spring's primary config format): `key=value` or
        // `key:value`, with `#`/`!` comments. Each dotted key is a ConfigKey.
        let src = "# a comment\n\
                   spring.datasource.url=jdbc:h2:mem:test\n\
                   server.port: 8080\n\
                   ! bang comment\n\
                   logging.level.root\n";
        let mut graph = SymbolGraph::new();
        ConfigExtractor::new()
            .extract(
                std::path::Path::new("application.properties"),
                src,
                &mut graph,
            )
            .expect("extraction failed");
        let names: Vec<String> = graph.nodes().map(|n| n.name.clone()).collect();
        assert!(
            names.contains(&"spring.datasource.url".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"server.port".to_string()),
            "':' separator key: {names:?}"
        );
        assert!(
            names.contains(&"logging.level.root".to_string()),
            "no-value key: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("comment")),
            "comments skipped: {names:?}"
        );
        assert!(graph.nodes().all(|n| n.kind == SymbolKind::ConfigKey));
    }

    #[test]
    fn extracts_yaml_keys() {
        let graph = extract_from_fixture("config-simple", "app.yaml");
        let names: Vec<&str> = graph.nodes().map(|n| n.name.as_str()).collect();
        // Leaf keys are emitted; the `database`/`server` container mappings are not.
        assert!(
            names.contains(&"database.host"),
            "Expected leaf 'database.host'"
        );
        assert!(
            names.contains(&"server.port"),
            "Expected leaf 'server.port'"
        );
        assert!(
            !names.contains(&"server"),
            "container 'server' should not be emitted"
        );
        assert!(graph.nodes().all(|n| n.kind == SymbolKind::ConfigKey));
    }

    #[test]
    fn extracts_toml_keys() {
        let graph = extract_from_fixture("config-simple", "app.toml");
        let names: Vec<&str> = graph.nodes().map(|n| n.name.as_str()).collect();
        assert!(
            names.contains(&"database.host"),
            "Expected leaf 'database.host'"
        );
        assert!(
            names.contains(&"server.port"),
            "Expected leaf 'server.port'"
        );
        assert!(
            !names.contains(&"database"),
            "container 'database' should not be emitted"
        );
        assert!(graph.nodes().all(|n| n.kind == SymbolKind::ConfigKey));
    }

    #[test]
    fn config_extensions() {
        let extractor = ConfigExtractor::new();
        let exts = extractor.file_extensions();
        assert!(exts.contains(&"yaml"));
        assert!(exts.contains(&"yml"));
        assert!(exts.contains(&"toml"));
        assert!(exts.contains(&"json"));
    }

    #[test]
    fn recurses_nested_yaml() {
        let extractor = ConfigExtractor::new();
        let path = std::path::Path::new("in-memory.yaml");
        let source =
            "server:\n  port: 8080\n  host: localhost\ndatabase:\n  primary:\n    url: jdbc:x\n";
        let mut graph = SymbolGraph::new();
        extractor
            .extract(path, source, &mut graph)
            .expect("extract");

        let names: Vec<String> = graph.nodes().map(|n| n.name.clone()).collect();
        // Leaf scalars are emitted; container mappings are not.
        assert!(names.contains(&"server.port".to_string()));
        assert!(names.contains(&"server.host".to_string()));
        assert!(names.contains(&"database.primary.url".to_string()));
        assert!(
            !names.contains(&"server".to_string()),
            "container emitted: {names:?}"
        );
        assert!(
            !names.contains(&"database".to_string()),
            "container emitted: {names:?}"
        );
        assert!(
            !names.contains(&"database.primary".to_string()),
            "container emitted: {names:?}"
        );
    }

    #[test]
    fn recurses_nested_toml() {
        let extractor = ConfigExtractor::new();
        let path = std::path::Path::new("in-memory.toml");
        let source = "[server]\nport = 8080\n\n[database.primary]\nurl = \"x\"\n";
        let mut graph = SymbolGraph::new();
        extractor
            .extract(path, source, &mut graph)
            .expect("extract");

        let names: Vec<String> = graph.nodes().map(|n| n.name.clone()).collect();
        // Leaf scalars are emitted; container tables are not.
        assert!(names.contains(&"server.port".to_string()));
        assert!(names.contains(&"database.primary.url".to_string()));
        assert!(
            !names.contains(&"server".to_string()),
            "container table emitted: {names:?}"
        );
        assert!(
            !names.contains(&"database.primary".to_string()),
            "container table emitted: {names:?}"
        );
    }

    #[test]
    fn emits_leaf_keys_only_drops_containers_and_array_indices() {
        // Only scalar leaves and list keys are referenceable from code. Container
        // (mapping) keys and synthetic `[i]` array-index keys are structurally
        // guaranteed to be "unused" and must not be emitted.
        let extractor = ConfigExtractor::new();
        let source = "server:\n  port: 8080\nservers:\n  - host: h1\n  - host: h2\n";
        let mut graph = SymbolGraph::new();
        extractor
            .extract(std::path::Path::new("m.yaml"), source, &mut graph)
            .expect("extract");
        let names: Vec<String> = graph.nodes().map(|n| n.name.clone()).collect();
        assert!(
            names.contains(&"server.port".to_string()),
            "leaf key missing: {names:?}"
        );
        assert!(
            names.contains(&"servers".to_string()),
            "list key missing: {names:?}"
        );
        assert!(
            !names.contains(&"server".to_string()),
            "container key emitted: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains('[')),
            "array-index key emitted: {names:?}"
        );
    }

    #[test]
    fn skips_build_manifest_cargo_toml() {
        // `Cargo.toml` declares the crate graph, not application config.
        // Emitting its keys as ConfigKey nodes poisons the inconsistency
        // detector — every declared dependency shows up as an "unused-key".
        let extractor = ConfigExtractor::new();
        let path = std::path::Path::new("some/dir/Cargo.toml");
        let source = "[package]\nname = \"demo\"\n\n[dependencies]\ntokio = \"1\"\n";
        let mut graph = SymbolGraph::new();
        extractor
            .extract(path, source, &mut graph)
            .expect("extract");
        assert_eq!(graph.node_count(), 0, "Cargo.toml must be skipped");
    }

    #[test]
    fn skips_package_json_manifest() {
        let extractor = ConfigExtractor::new();
        let path = std::path::Path::new("apps/web/package.json");
        let source = r#"{"name":"x","scripts":{"build":"vite build"}}"#;
        let mut graph = SymbolGraph::new();
        extractor
            .extract(path, source, &mut graph)
            .expect("extract");
        assert_eq!(graph.node_count(), 0, "package.json must be skipped");
    }

    #[test]
    fn skips_tsconfig_and_variants() {
        // Regression: a `vscode-extension/tsconfig.json` shipping in
        // the repo used to populate `compilerOptions.*` keys as
        // ConfigKey nodes, producing 9 spurious orphans + 10 false
        // `unused-key` inconsistencies. tsconfig + variants must be
        // treated like Cargo.toml — build description, not app config.
        for name in [
            "tsconfig.json",
            "tsconfig.build.json",
            "tsconfig.test.json",
            "jsconfig.json",
            "babel.config.json",
            ".eslintrc.json",
            "deno.jsonc",
        ] {
            let extractor = ConfigExtractor::new();
            let path = std::path::Path::new("subdir").join(name);
            let source = r#"{"compilerOptions":{"target":"ES2022"}}"#;
            let mut graph = SymbolGraph::new();
            extractor
                .extract(&path, source, &mut graph)
                .expect("extract");
            assert_eq!(
                graph.node_count(),
                0,
                "{} must be treated as a build manifest",
                name
            );
        }
    }

    #[test]
    fn still_extracts_app_yaml_even_when_named_config() {
        // Plain `app.yaml` is not a build manifest — it must still flow
        // through the extractor. Only the specific manifest filenames
        // are denied.
        let extractor = ConfigExtractor::new();
        let path = std::path::Path::new("config/app.yaml");
        let source = "server:\n  port: 8080\n";
        let mut graph = SymbolGraph::new();
        extractor
            .extract(path, source, &mut graph)
            .expect("extract");
        assert!(graph.node_count() > 0, "normal yaml must still extract");
    }

    #[test]
    fn json_array_emits_list_key_but_no_array_index_keys() {
        let extractor = ConfigExtractor::new();
        let path = std::path::Path::new("in-memory.json");
        let source = r#"{"servers":[{"host":"a"},{"host":"b"}]}"#;
        let mut graph = SymbolGraph::new();
        extractor
            .extract(path, source, &mut graph)
            .expect("extract");

        let names: Vec<String> = graph.nodes().map(|n| n.name.clone()).collect();
        // The list key itself is referenceable; the synthetic `[i].host` keys
        // can never match a code reference and must not be emitted.
        assert!(
            names.contains(&"servers".to_string()),
            "list key missing: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains('[')),
            "array-index key emitted: {names:?}"
        );
    }
}
