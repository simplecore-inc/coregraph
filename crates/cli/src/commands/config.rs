use anyhow::{anyhow, Context};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use toml::Value;

/// Per-project configuration file. Read by commands that accept `-C` so
/// project-specific overrides sit alongside the `.coregraph/ignore` file.
pub fn project_config_path(project_root: &Path) -> PathBuf {
    project_root.join(".coregraph").join("config.toml")
}

/// Config keys the runtime actually reads. Every entry here is merged
/// into `GlobalOpts` by `main::apply_config_file` when the clap-default
/// sentinel still matches (i.e. the user did not override on the CLI).
///
/// Keys that had been listed here previously but never plumbed through
/// (`default.*`, `server.*`, `index.max_file_size`) were removed so the
/// auto-generated `config.toml` only advertises knobs that genuinely
/// change behaviour — a dead config entry is worse than a missing one,
/// because it tricks users into believing they've configured something.
/// If a new knob earns a CLI flag with a non-trivial default, extend
/// both this table and `apply_config_file` together.
const KNOWN_KEYS: &[(&str, &str, &str)] = &[
    (
        "limits.token_budget",
        "8000",
        "Default token budget for LLM output",
    ),
    ("limits.hop_limit", "3", "Default graph traversal depth"),
    (
        "limits.min_confidence",
        "0.70",
        "Default minimum edge confidence (matches clap default)",
    ),
    (
        "server.max_loaded_projects",
        "5",
        "Maximum projects held in the daemon cache (LRU eviction above this)",
    ),
    (
        "server.graceful_shutdown_sec",
        "30",
        "Seconds the daemon waits for in-flight queries before hard-exit on SIGTERM",
    ),
    (
        "server.idle_unload_minutes",
        "10",
        "Minutes a project sits idle before its graph is unloaded from the daemon cache",
    ),
    (
        "server.max_loaded_bytes",
        "0",
        "Approx. heap budget (bytes) across all loaded graphs; LRU-evicts over it. 0 = unlimited",
    ),
];

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommand>,

    /// Legacy positional: configuration key to read (e.g. server.port).
    pub key: Option<String>,

    /// Legacy positional: value to write (requires key).
    pub value: Option<String>,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Create a default config file at the configured path.
    Init {
        /// Overwrite an existing file.
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Create the per-project config (`<project>/.coregraph/config.toml`)
        /// instead of the global one. Flag is `--local` rather than
        /// `--project` because the global CLI already owns `--project`
        /// (project-root override) and clap rejects same-named
        /// arguments at different scopes with a panic.
        #[arg(long = "local", default_value_t = false)]
        local: bool,
    },
    /// Print the current (on-disk + defaults) config values.
    Show,
    /// Remove a key.
    Unset { key: String },
    /// Print the path of the config file.
    Path,
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("coregraph")
        .join("config.toml")
}

fn load() -> anyhow::Result<toml::map::Map<String, Value>> {
    let path = config_path();
    if !path.exists() {
        return Ok(toml::map::Map::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    match toml::from_str::<Value>(&text)? {
        Value::Table(t) => Ok(t),
        _ => Err(anyhow!("config is not a table")),
    }
}

fn save(table: &toml::map::Map<String, Value>) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let text = toml::to_string_pretty(&Value::Table(table.clone()))?;
    std::fs::write(&path, text)?;
    Ok(())
}

fn get_key<'a>(t: &'a toml::map::Map<String, Value>, key: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    match parts.as_slice() {
        [section, sub] => t.get(*section)?.as_table()?.get(*sub),
        [flat] => t.get(*flat),
        _ => None,
    }
}

fn set_key(t: &mut toml::map::Map<String, Value>, key: &str, value: Value) {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    match parts.as_slice() {
        [section, sub] => {
            let section_val = t
                .entry(section.to_string())
                .or_insert_with(|| Value::Table(toml::map::Map::new()));
            if let Value::Table(tt) = section_val {
                tt.insert(sub.to_string(), value);
            }
        }
        [flat] => {
            t.insert(flat.to_string(), value);
        }
        _ => {}
    }
}

fn unset_key(t: &mut toml::map::Map<String, Value>, key: &str) -> bool {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    match parts.as_slice() {
        [section, sub] => {
            if let Some(Value::Table(tt)) = t.get_mut(*section) {
                tt.remove(*sub).is_some()
            } else {
                false
            }
        }
        [flat] => t.remove(*flat).is_some(),
        _ => false,
    }
}

fn parse_value(s: &str) -> Value {
    if let Ok(i) = s.parse::<i64>() {
        return Value::Integer(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        return Value::Float(f);
    }
    if let Ok(b) = s.parse::<bool>() {
        return Value::Boolean(b);
    }
    Value::String(s.to_string())
}

fn value_display(v: &Value) -> std::borrow::Cow<'_, str> {
    match v {
        Value::String(s) => std::borrow::Cow::Borrowed(s.as_str()),
        other => std::borrow::Cow::Owned(other.to_string()),
    }
}

pub fn run(args: ConfigArgs, project_root: &Path) -> anyhow::Result<()> {
    match args.command {
        Some(ConfigCommand::Init { force, local }) => init(force, local, project_root),
        Some(ConfigCommand::Show) => show(project_root),
        Some(ConfigCommand::Unset { key }) => unset(&key),
        Some(ConfigCommand::Path) => {
            println!("global:  {}", config_path().display());
            println!("project: {}", project_config_path(project_root).display());
            Ok(())
        }
        None => legacy(args.key, args.value, project_root),
    }
}

fn init(force: bool, local: bool, project_root: &Path) -> anyhow::Result<()> {
    let path = if local {
        project_config_path(project_root)
    } else {
        config_path()
    };
    if path.exists() && !force {
        println!("Config already exists at {}", path.display());
        println!("Re-run with --force to overwrite.");
        return Ok(());
    }
    write_default_config(&path)?;
    println!("Initialized config at {}", path.display());
    Ok(())
}

/// Write the default config (KNOWN_KEYS + `[index] exclude` list) to
/// `path`, creating parent directories as needed. Used by both
/// `config init` (interactive) and `ensure_local_default` (silent auto-init).
fn write_default_config(path: &Path) -> anyhow::Result<()> {
    let mut t = toml::map::Map::new();
    for (key, default, _) in KNOWN_KEYS {
        set_key(&mut t, key, parse_value(default));
    }
    // `[index].exclude` is an array rather than a dotted scalar key,
    // so it sits outside the `(key, default, desc)` table and we
    // insert it directly. Default is empty — the library does not
    // assume any project-specific paths.
    let mut index_table = toml::map::Map::new();
    index_table.insert("exclude".to_string(), Value::Array(Vec::new()));
    t.insert("index".to_string(), Value::Table(index_table));

    // `[analysis].exclude` is the *analysis-surface* counterpart: files matched
    // here are still parsed and indexed (so their edges keep referenced symbols
    // connected) but their own symbols are suppressed from dead-code (orphans)
    // reports. Use it for generated consumers (e.g. `routeTree.gen.ts`) whose
    // hard `[index].exclude` would otherwise orphan the symbols they import.
    // Default empty — no assumptions about project layout.
    let mut analysis_table = toml::map::Map::new();
    analysis_table.insert("exclude".to_string(), Value::Array(Vec::new()));
    t.insert("analysis".to_string(), Value::Table(analysis_table));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Prepend a header with an inline description per key so users can
    // see at a glance what knobs exist. Every entry is plumbed through
    // to `GlobalOpts` or `PathExcluder`.
    let mut text = String::new();
    text.push_str("# CoreGraph configuration\n");
    text.push_str("#\n");
    text.push_str("# [limits] — query defaults applied when the matching CLI flag\n");
    text.push_str("#   is not explicitly passed. Override per-command with\n");
    text.push_str("#   `--token-budget`, `--hop-limit`, or `--min-confidence`.\n");
    text.push_str("#\n");
    text.push_str("# [index]  — indexing-time knobs.\n");
    text.push_str("#   exclude: gitignore-syntax patterns for files NOT parsed at\n");
    text.push_str("#            all (no symbols, no edges). Cuts symbols/memory, but\n");
    text.push_str("#            dropping a file also drops the edges it would have\n");
    text.push_str("#            contributed — so a symbol referenced ONLY by an\n");
    text.push_str("#            excluded file becomes a false orphan. Example:\n");
    text.push_str("#            [\"tests/fixtures/\", \"target/\"]\n");
    text.push_str("#\n");
    text.push_str("# [analysis] — analysis-surface knobs.\n");
    text.push_str("#   exclude: gitignore-syntax patterns for files still PARSED\n");
    text.push_str("#            (their edges keep referenced symbols connected) but\n");
    text.push_str("#            whose own symbols are hidden from dead-code (orphans)\n");
    text.push_str("#            reports. Prefer this over index.exclude for generated\n");
    text.push_str("#            consumers like routeTree.gen.ts. Example: [\"**/*.gen.ts\"]\n");
    text.push_str("#\n");
    text.push_str("# Keys:\n");
    for (key, _, desc) in KNOWN_KEYS {
        text.push_str(&format!("#   {:<26} {}\n", key, desc));
    }
    text.push_str(&format!(
        "#   {:<26} Gitignore patterns for files excluded from indexing (array)\n",
        "index.exclude"
    ));
    text.push_str(&format!(
        "#   {:<26} Gitignore patterns for files kept indexed but hidden from\n#   {:<26} dead-code reports (array)\n",
        "analysis.exclude", ""
    ));
    text.push('\n');
    text.push_str(&toml::to_string_pretty(&Value::Table(t))?);
    std::fs::write(path, text)?;
    Ok(())
}

/// Ensure `<project>/.coregraph/config.toml` exists with default values.
/// No-op when the file is already present — caller-facing tweaks are
/// preserved. Failure is non-fatal: a missing config never blocks a
/// command, the global defaults still apply.
///
/// If a legacy `.coregraph/ignore` file is present alongside the
/// missing config, the patterns are migrated into the new
/// `[index] exclude` array and the original file is renamed with a
/// `.bak` suffix (never deleted). This preserves whatever the user
/// had before and produces a visible breadcrumb if they grep for
/// "ignore".
///
/// Called from any command that creates the `.coregraph/` directory
/// (`index`, `server start`, daemon spawn) so users never have to run
/// `coregraph config init --local` explicitly.
pub fn ensure_local_default(project_root: &Path) {
    let path = project_config_path(project_root);
    if path.exists() {
        return;
    }
    let legacy_patterns = read_and_retire_legacy_ignore(project_root);
    let write_result = if legacy_patterns.is_empty() {
        write_default_config(&path)
    } else {
        write_default_config_with_excludes(&path, &legacy_patterns)
    };
    if let Err(e) = write_result {
        eprintln!(
            "[coregraph] could not auto-create {}: {}",
            path.display(),
            e
        );
    }
}

/// Load patterns from `<project>/.coregraph/ignore`, rename the source
/// to `ignore.bak` so subsequent runs don't re-migrate, and return the
/// lines verbatim (comments and blank lines dropped). Silent on I/O
/// failure — migration is best-effort.
fn read_and_retire_legacy_ignore(project_root: &Path) -> Vec<String> {
    let legacy = project_root.join(".coregraph").join("ignore");
    let Ok(text) = std::fs::read_to_string(&legacy) else {
        return Vec::new();
    };
    let patterns: Vec<String> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    if patterns.is_empty() {
        // Empty (or comment-only) ignore file — just rename so the
        // migration doesn't keep re-firing.
        let _ = std::fs::rename(&legacy, legacy.with_extension("bak"));
        return Vec::new();
    }
    match std::fs::rename(&legacy, legacy.with_extension("bak")) {
        Ok(()) => {
            eprintln!(
                "[coregraph] migrated {} patterns from .coregraph/ignore → config.toml (original saved as ignore.bak)",
                patterns.len()
            );
        }
        Err(e) => {
            eprintln!(
                "[coregraph] could not rename legacy ignore file: {} — patterns still migrated",
                e
            );
        }
    }
    patterns
}

/// Variant of `write_default_config` that seeds `[index] exclude` with
/// pre-existing patterns (migrated from a legacy ignore file).
fn write_default_config_with_excludes(path: &Path, excludes: &[String]) -> anyhow::Result<()> {
    // Reuse the default writer first, then rewrite the `[index]` block
    // with the real patterns. The header comments are preserved.
    write_default_config(path)?;
    let text = std::fs::read_to_string(path)?;
    let mut new_text = String::new();
    let mut skipping_empty_index = false;
    for line in text.lines() {
        if line.starts_with("exclude = []") && skipping_empty_index {
            let arr: Vec<String> = excludes.iter().map(|p| format!("{:?}", p)).collect();
            new_text.push_str(&format!("exclude = [{}]\n", arr.join(", ")));
            skipping_empty_index = false;
            continue;
        }
        if line == "[index]" {
            skipping_empty_index = true;
        } else if !line.starts_with("exclude") {
            skipping_empty_index = false;
        }
        new_text.push_str(line);
        new_text.push('\n');
    }
    std::fs::write(path, new_text)?;
    Ok(())
}

fn show(project_root: &Path) -> anyhow::Result<()> {
    // Global defaults first; project-local overrides second so the
    // displayed "current" value reflects the merged view the runtime sees.
    let global = load_from(&config_path());
    let project_path = project_config_path(project_root);
    let project = load_from(&project_path);

    println!("Global config:  {}", config_path().display());
    println!("Project config: {}", project_path.display());
    if !project_path.exists() {
        println!("                (not present — run `coregraph config init --local`)");
    }
    println!();
    for (key, default, desc) in KNOWN_KEYS {
        let (value, source) = if let Some(v) = get_key(&project, key) {
            (value_display(v).into_owned(), "project")
        } else if let Some(v) = get_key(&global, key) {
            (value_display(v).into_owned(), "global")
        } else {
            (default.to_string(), "default")
        };
        println!("  {:<30} = {:<12}  [{}]", key, value, source);
        println!("    # {}", desc);
    }
    Ok(())
}

fn load_from(path: &Path) -> toml::map::Map<String, Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return toml::map::Map::new();
    };
    match toml::from_str::<Value>(&text) {
        Ok(Value::Table(t)) => t,
        _ => toml::map::Map::new(),
    }
}

fn unset(key: &str) -> anyhow::Result<()> {
    let mut t = load()?;
    if unset_key(&mut t, key) {
        save(&t)?;
        println!("Removed: {}", key);
    } else {
        println!("Key not set: {}", key);
    }
    Ok(())
}

fn legacy(key: Option<String>, value: Option<String>, project_root: &Path) -> anyhow::Result<()> {
    match (key, value) {
        (Some(key), Some(value)) => {
            let valid = KNOWN_KEYS.iter().any(|(k, _, _)| *k == key);
            if !valid {
                let known: Vec<&str> = KNOWN_KEYS.iter().map(|(k, _, _)| *k).collect();
                return Err(anyhow!(
                    "unknown config key '{}'. Known keys: {}",
                    key,
                    known.join(", ")
                ));
            }
            let mut t = load()?;
            set_key(&mut t, &key, parse_value(&value));
            save(&t)?;
            println!("{} = {}", key, value);
            Ok(())
        }
        (Some(key), None) => {
            let t = load()?;
            match get_key(&t, &key) {
                Some(v) => println!("{} = {}", key, value_display(v)),
                None => {
                    let default = KNOWN_KEYS
                        .iter()
                        .find(|(k, _, _)| *k == key)
                        .map(|(_, d, _)| *d);
                    match default {
                        Some(d) => println!("{} = {}  (default — not set in config)", key, d),
                        None => println!("{} = (not set)", key),
                    }
                }
            }
            Ok(())
        }
        (None, _) => show(project_root),
    }
}

/// Daemon lifecycle knobs read from the merged config (project-local
/// overrides global). `None` per field means the key is absent and the
/// daemon falls back to its compiled default. Consumed only by the daemon
/// start path — unlike query `limits.*`, which `main::apply_config_file`
/// merges into `GlobalOpts`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ServerOverrides {
    pub max_loaded_projects: Option<usize>,
    pub idle_unload_minutes: Option<u64>,
    pub max_loaded_bytes: Option<u64>,
    pub graceful_shutdown_sec: Option<u64>,
}

/// Read [`ServerOverrides`] from `<project>/.coregraph/config.toml`
/// (project) layered over the global config file.
pub fn server_overrides(project_root: &Path) -> ServerOverrides {
    let global = load_from(&config_path());
    let project = load_from(&project_config_path(project_root));
    let read_u64 = |key: &str| -> Option<u64> {
        get_key(&project, key)
            .or_else(|| get_key(&global, key))
            .and_then(|v| v.as_integer())
            .map(|n| n as u64)
    };
    ServerOverrides {
        max_loaded_projects: read_u64("server.max_loaded_projects").map(|n| n as usize),
        idle_unload_minutes: read_u64("server.idle_unload_minutes"),
        max_loaded_bytes: read_u64("server.max_loaded_bytes"),
        graceful_shutdown_sec: read_u64("server.graceful_shutdown_sec"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_overrides_reads_project_local_keys() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(
            &cfg,
            "[server]\nmax_loaded_projects = 9\nidle_unload_minutes = 3\ngraceful_shutdown_sec = 12\n",
        )
        .unwrap();

        let ov = server_overrides(dir.path());
        assert_eq!(ov.max_loaded_projects, Some(9));
        assert_eq!(ov.idle_unload_minutes, Some(3));
        assert_eq!(ov.graceful_shutdown_sec, Some(12));
    }

    #[test]
    fn server_overrides_absent_keys_are_none() {
        let dir = tempfile::tempdir().expect("tmpdir");
        // No project config written. `idle_unload_minutes` is a brand-new
        // key absent from any pre-existing global config, so it stays None.
        let ov = server_overrides(dir.path());
        assert_eq!(ov.idle_unload_minutes, None);
    }

    #[test]
    fn parse_value_types() {
        assert_eq!(parse_value("8080"), Value::Integer(8080));
        assert_eq!(parse_value("true"), Value::Boolean(true));
        assert_eq!(parse_value("human"), Value::String("human".to_string()));
    }

    #[test]
    fn set_and_get_dotkey() {
        let mut t = toml::map::Map::new();
        set_key(&mut t, "server.port", Value::Integer(9000));
        assert_eq!(get_key(&t, "server.port"), Some(&Value::Integer(9000)));
    }

    #[test]
    fn ensure_local_default_creates_when_missing() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        assert!(!cfg.exists());
        ensure_local_default(dir.path());
        assert!(cfg.exists(), "config.toml should be auto-created");
        let body = std::fs::read_to_string(&cfg).unwrap();
        // Every section advertised in `write_default_config` must appear.
        assert!(body.contains("[limits]"), "missing [limits]: {}", body);
        assert!(body.contains("[index]"), "missing [index]: {}", body);
        assert!(body.contains("exclude"), "missing exclude key: {}", body);
        // The analysis-surface exclude must be advertised too so the
        // index-vs-analysis distinction is discoverable.
        assert!(body.contains("[analysis]"), "missing [analysis]: {}", body);
    }

    #[test]
    fn ensure_local_default_preserves_existing() {
        // Critical: the auto-init must never clobber user edits. We
        // write a sentinel value, call `ensure_local_default`, and
        // confirm the original content survives unchanged.
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "# user config\nkey = \"value\"\n").unwrap();
        ensure_local_default(dir.path());
        let body = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(body, "# user config\nkey = \"value\"\n");
    }
}
