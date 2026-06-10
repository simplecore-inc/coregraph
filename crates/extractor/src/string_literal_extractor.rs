//! Cross-language scanner that extracts two kinds of literal-valued nodes:
//!
//! 1. **API paths** — string literals that look like HTTP route templates
//!    (starting with `/`, composed of `[A-Za-z0-9/_{}.-]`). Used by the
//!    `api-path` inconsistency detector and by router mediators.
//! 2. **Config key references** — code-side references to config keys via
//!    `@Value("${x.y.z}")`, `process.env.X_Y_Z`, `os.environ["X_Y_Z"]`. Used
//!    by the `config-key` inconsistency detector.
//!
//! This complements the tree-sitter symbol extractors: rather than another
//! full grammar pass, it scans text with regex. That's good enough for the
//! two detectors above; precision is not critical because the inconsistency
//! reports are already ranked + filtered by heuristics.

use std::path::Path;

use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
use coregraph_graph::SymbolGraph;
use regex::Regex;

use crate::{ExtractError, SymbolExtractor};

/// Marker prefix inserted in the `name` of a StringLiteral node that looks
/// like an API path. Lets downstream detectors cheaply filter without
/// widening the SymbolNode schema.
pub const API_PATH_PREFIX: &str = "api_path::";

/// Marker prefix inserted in the `name` of a ConfigKey node that is a
/// *reference* from code (as opposed to a definition from a config file).
pub const CONFIG_REF_PREFIX: &str = "config_ref::";

/// Returns `true` if `name` starts with the API-path marker prefix.
pub fn is_api_path_literal(name: &str) -> bool {
    name.starts_with(API_PATH_PREFIX)
}

/// Returns `true` if `name` starts with the config-ref marker prefix.
pub fn is_config_ref(name: &str) -> bool {
    name.starts_with(CONFIG_REF_PREFIX)
}

/// Strip the marker prefix, returning the bare value. If `name` is not
/// prefixed it is returned unchanged.
pub fn strip_api_path_prefix(name: &str) -> &str {
    name.strip_prefix(API_PATH_PREFIX).unwrap_or(name)
}

pub fn strip_config_ref_prefix(name: &str) -> &str {
    name.strip_prefix(CONFIG_REF_PREFIX).unwrap_or(name)
}

/// Marker prefix on a ConfigKey node that is a RUNTIME environment-variable
/// reference (`process.env.X`, `import.meta.env.X`, `os.environ["X"]`,
/// `std::env::var("X")`). Unlike `config_ref::` — a `@Value`-style reference
/// that expects a matching key in a config file — an env-var reference is
/// satisfied at runtime and has no config-file definition, so it must NOT be
/// reported as a "missing" config key.
pub const ENV_REF_PREFIX: &str = "env_ref::";

/// Returns `true` if `name` starts with the runtime-env-ref marker prefix.
pub fn is_env_ref(name: &str) -> bool {
    name.starts_with(ENV_REF_PREFIX)
}

/// Marker prefix on synthetic Function nodes the extractor inserts when it
/// spots a Go DI wireup call (`wire.Build(...)`, `fx.Provide(...)`). Used by
/// GoDiMediator to recognise which files contain DI plumbing without a full
/// grammar pass.
pub const GO_DI_MARKER_PREFIX: &str = "go_di_marker::";

/// Extractor that scans source text for API paths and config-key references.
/// Runs across every language extension; configs go through the dedicated
/// `ConfigExtractor`.
pub struct StringLiteralExtractor {
    api_path: Regex,
    spring_value: Regex,
    node_env: Regex,
    python_env: Regex,
    rust_env: Regex,
    go_di_wireup: Regex,
}

impl StringLiteralExtractor {
    pub fn new() -> Self {
        Self {
            // API path templates: literal starting with `/` then `[A-Za-z0-9_/-{}.:]`.
            // Requires at least one path segment so we don't match `/`.
            api_path: Regex::new(r#""(/[A-Za-z0-9_\-/{}\.\:]*[A-Za-z0-9_\-}\)])""#).unwrap(),
            // Spring @Value("${x.y.z}") / @Value("${x.y.z:default}")
            spring_value: Regex::new(r#"@Value\(\s*"\$\{([A-Za-z0-9_.\-]+)(?::[^}]*)?\}"\s*\)"#)
                .unwrap(),
            // Node/TypeScript process.env.X_Y_Z or process.env["X_Y_Z"] /
            // import.meta.env.X_Y_Z (Vite). The bracket form also accepts
            // kebab- or dot-cased keys for users who pre-normalise their
            // env var names (e.g. `process.env["app.port"]`).
            node_env: Regex::new(
                r#"(?:process\.env\.|import\.meta\.env\.)([A-Z][A-Z0-9_]*)|(?:process\.env|import\.meta\.env)\[\s*"([A-Za-z][A-Za-z0-9_.-]*)"\s*\]"#,
            )
            .unwrap(),
            // Python os.environ["X"] / os.getenv("X", default). The
            // trailing `(?:\s*,[^)]*)?` tolerates the optional default
            // argument on `os.getenv`.
            python_env: Regex::new(
                r#"os\.environ\[\s*"([A-Z][A-Z0-9_]*)"\s*\]|os\.getenv\(\s*"([A-Z][A-Z0-9_]*)"(?:\s*,[^)]*)?\s*\)"#,
            )
            .unwrap(),
            // Rust `std::env::var("X")` / `env::var("X")` / `env!("X")`.
            rust_env: Regex::new(
                r#"(?:std::env::var|env::var|env!)\(\s*"([A-Z][A-Z0-9_]*)"(?:\s*,[^)]*)?\s*\)"#,
            )
            .unwrap(),
            // Google Wire `wire.Build(...)` and Uber fx `fx.Provide(...)`.
            go_di_wireup: Regex::new(r#"\b(wire\.Build|fx\.Provide)\s*\("#).unwrap(),
        }
    }
}

impl Default for StringLiteralExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor for StringLiteralExtractor {
    fn language_name(&self) -> &'static str {
        "StringLiteral"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &[
            "java", "kt", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "pyi", "go", "rs",
        ]
    }

    fn extract(
        &self,
        path: &Path,
        source: &str,
        graph: &mut SymbolGraph,
    ) -> Result<(), ExtractError> {
        // Each config-ref probe (Spring `@Value`, `process.env`, `os.environ`,
        // `wire.Build`) is language-specific. Running them on every file
        // extension in our list causes false positives — e.g. when this
        // extractor's own Rust source text contains the regex literal
        // `@Value(...)`, that literal itself matches the Spring probe and
        // produces a spurious config_ref node. We gate each probe on the
        // file extension that can plausibly use the pattern.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_jvm = matches!(ext.as_str(), "java" | "kt");
        let is_js_like = matches!(ext.as_str(), "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs");
        let is_python = matches!(ext.as_str(), "py" | "pyi");
        let is_go = ext == "go";

        // API paths — emitted as StringLiteral nodes whose name carries the
        // `api_path::` prefix plus the raw path. Only run against the
        // languages that routinely host HTTP route declarations; the
        // regex previously matched test fixtures inside `.rs` sources
        // (e.g. `#[test]` doc strings demonstrating the detector
        // itself), producing cross-file near-miss false positives.
        let api_path_relevant = is_jvm || is_js_like || is_python || is_go;
        if api_path_relevant {
            for cap in self.api_path.captures_iter(source) {
                let Some(m) = cap.get(1) else { continue };
                let raw = m.as_str();
                // Drop tiny matches (single slash, doubled slash from regex anchors).
                if raw.len() < 4 || !raw.starts_with('/') {
                    continue;
                }
                // Exclude obvious false positives (file-system paths with ~, escape sequences).
                if raw.contains("//") && !raw.starts_with("//") {
                    continue;
                }
                // Exclude SVG path data / base64 / version strings that share the
                // leading slash heuristic.
                if raw.chars().filter(|c| c.is_ascii_digit()).count() > raw.len() / 2 {
                    continue;
                }
                let name = format!("{}{}", API_PATH_PREFIX, raw);
                let span_start = m.start() as u32;
                let span_end = m.end() as u32;
                let node = SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::StringLiteral,
                    name,
                    path,
                    span_start,
                    span_end,
                );
                graph.insert_node(node);
            }
        }

        // Spring @Value("${key}") references — JVM languages only.
        if is_jvm {
            for cap in self.spring_value.captures_iter(source) {
                let Some(key) = cap.get(1) else { continue };
                let full = cap.get(0).unwrap();
                let name = format!("{}{}", CONFIG_REF_PREFIX, key.as_str());
                let node = SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::ConfigKey,
                    name,
                    path,
                    full.start() as u32,
                    full.end() as u32,
                );
                graph.insert_node(node);
            }
        }

        // Node/TS process.env.FOO references — JS/TS only.
        if is_js_like {
            for cap in self.node_env.captures_iter(source) {
                let Some(m) = cap.get(1).or(cap.get(2)) else {
                    continue;
                };
                let full = cap.get(0).unwrap();
                let normalized = m.as_str().to_ascii_lowercase().replace('_', ".");
                let name = format!("{}{}", ENV_REF_PREFIX, normalized);
                let node = SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::ConfigKey,
                    name,
                    path,
                    full.start() as u32,
                    full.end() as u32,
                );
                graph.insert_node(node);
            }
        }

        // Python os.environ["FOO"] / os.getenv("FOO") — Python only.
        if is_python {
            for cap in self.python_env.captures_iter(source) {
                let Some(m) = cap.get(1).or(cap.get(2)) else {
                    continue;
                };
                let full = cap.get(0).unwrap();
                let normalized = m.as_str().to_ascii_lowercase().replace('_', ".");
                let name = format!("{}{}", ENV_REF_PREFIX, normalized);
                let node = SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::ConfigKey,
                    name,
                    path,
                    full.start() as u32,
                    full.end() as u32,
                );
                graph.insert_node(node);
            }
        }

        // Rust `std::env::var("X")` / `env!("X")` references.
        let is_rust = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("rs"))
            .unwrap_or(false);
        if is_rust {
            for cap in self.rust_env.captures_iter(source) {
                let Some(m) = cap.get(1) else { continue };
                let full = cap.get(0).unwrap();
                let normalized = m.as_str().to_ascii_lowercase().replace('_', ".");
                let name = format!("{}{}", ENV_REF_PREFIX, normalized);
                let node = SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::ConfigKey,
                    name,
                    path,
                    full.start() as u32,
                    full.end() as u32,
                );
                graph.insert_node(node);
            }
        }

        // Go DI wireup markers (wire.Build, fx.Provide) — Go only.
        if is_go {
            for cap in self.go_di_wireup.captures_iter(source) {
                let Some(m) = cap.get(1) else { continue };
                let full = cap.get(0).unwrap();
                let name = format!("{}{}", GO_DI_MARKER_PREFIX, m.as_str());
                let node = SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::Function,
                    name,
                    path,
                    full.start() as u32,
                    full.end() as u32,
                );
                graph.insert_node(node);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn extract(source: &str, ext: &str) -> SymbolGraph {
        let mut g = SymbolGraph::new();
        let extractor = StringLiteralExtractor::new();
        let path = PathBuf::from(format!("f.{}", ext));
        extractor.extract(&path, source, &mut g).expect("extract");
        g
    }

    #[test]
    fn extracts_spring_request_mapping() {
        let g = extract(
            r#"@GetMapping("/api/v1/cards") public List<Card> listCards() {}"#,
            "java",
        );
        let literals: Vec<_> = g.nodes().filter(|n| is_api_path_literal(&n.name)).collect();
        assert_eq!(literals.len(), 1);
        assert_eq!(strip_api_path_prefix(&literals[0].name), "/api/v1/cards");
    }

    #[test]
    fn extracts_fetch_call() {
        let g = extract(r#"fetch("/api/v1/card");"#, "ts");
        let literals: Vec<_> = g.nodes().filter(|n| is_api_path_literal(&n.name)).collect();
        assert_eq!(literals.len(), 1);
        assert_eq!(strip_api_path_prefix(&literals[0].name), "/api/v1/card");
    }

    #[test]
    fn spring_value_produces_config_ref() {
        let g = extract(r#"@Value("${database.pool.maxsize}") int maxSize;"#, "java");
        let refs: Vec<_> = g.nodes().filter(|n| is_config_ref(&n.name)).collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(
            strip_config_ref_prefix(&refs[0].name),
            "database.pool.maxsize"
        );
    }

    #[test]
    fn spring_value_with_default_parses_key() {
        let g = extract(r#"@Value("${server.port:8080}") int port;"#, "java");
        let refs: Vec<_> = g.nodes().filter(|n| is_config_ref(&n.name)).collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(strip_config_ref_prefix(&refs[0].name), "server.port");
    }

    #[test]
    fn node_process_env_yields_env_ref_not_config_ref() {
        let g = extract(r#"const k = process.env.DATABASE_URL;"#, "ts");
        // A runtime env var is tagged env_ref::, NOT config_ref:: — it has no
        // config-file definition and must not become a "missing-key" report.
        let env_refs: Vec<_> = g.nodes().filter(|n| is_env_ref(&n.name)).collect();
        assert_eq!(env_refs.len(), 1, "expected one env_ref");
        assert_eq!(env_refs[0].name, "env_ref::database.url");
        assert!(
            !g.nodes().any(|n| is_config_ref(&n.name)),
            "must not emit a config_ref"
        );
    }

    #[test]
    fn python_os_environ_yields_env_ref_not_config_ref() {
        let g = extract(r#"k = os.environ["DB_HOST"]"#, "py");
        let env_refs: Vec<_> = g.nodes().filter(|n| is_env_ref(&n.name)).collect();
        assert_eq!(env_refs.len(), 1, "expected one env_ref");
        assert_eq!(env_refs[0].name, "env_ref::db.host");
        assert!(
            !g.nodes().any(|n| is_config_ref(&n.name)),
            "must not emit a config_ref"
        );
    }

    #[test]
    fn non_api_short_strings_ignored() {
        let g = extract(r#"let s = "/a";"#, "ts");
        let literals: Vec<_> = g.nodes().filter(|n| is_api_path_literal(&n.name)).collect();
        assert!(literals.is_empty());
    }
}
