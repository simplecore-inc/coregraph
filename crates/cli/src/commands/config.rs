use anyhow::{anyhow, Context};
use clap::{Args, Subcommand};
use coregraph_extractor::string_match_max_files;
use coregraph_query::{
    recommend as compute_recommendations, InconsistencyOptions, Recommendations,
};
use std::path::{Path, PathBuf};
use toml::Value;
use toml_edit::DocumentMut;

use crate::global_opts::GlobalOpts;

/// Per-project configuration file. Read by commands that accept `-C` so
/// project-specific overrides sit alongside the `.coregraph/ignore` file.
pub fn project_config_path(project_root: &Path) -> PathBuf {
    project_root.join(".coregraph").join("config.toml")
}

/// Config keys the runtime actually reads. The three `limits.*` entries
/// are merged into `GlobalOpts` by `main::apply_config_file` when the
/// clap-default sentinel still matches (i.e. the user did not override on
/// the CLI). The four `server.*` entries take a different path: they are
/// never touched by `apply_config_file` and are read only by
/// `server_overrides()` when a foreground daemon starts.
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

    /// Legacy positional: configuration key to read (e.g. limits.hop_limit).
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
    /// Print recommended config.toml tuning derived from the indexed graph.
    Recommend {
        /// Apply the recommendations to <project>/.coregraph/config.toml
        /// (comment-preserving merge) instead of only printing them.
        #[arg(long, default_value_t = false)]
        write: bool,
    },
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

pub fn run(args: ConfigArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let project_root = &globals.project;
    match args.command {
        Some(ConfigCommand::Init { force, local }) => init(force, local, project_root),
        Some(ConfigCommand::Show) => show(project_root),
        Some(ConfigCommand::Unset { key }) => unset(&key),
        Some(ConfigCommand::Path) => {
            println!("global:  {}", config_path().display());
            println!("project: {}", project_config_path(project_root).display());
            Ok(())
        }
        Some(ConfigCommand::Recommend { write }) => recommend_cmd(globals, write),
        None => legacy(args.key, args.value, project_root),
    }
}

fn recommend_cmd(globals: &GlobalOpts, write: bool) -> anyhow::Result<()> {
    let root = globals.project_root();
    let (graph, _) = crate::graph_loader::load_project_graph(&root)?;
    let current_cap = string_match_max_files(&root);
    let opts = InconsistencyOptions::from_project_root(&root);
    let recs = compute_recommendations(&graph, &root, current_cap, &opts);
    println!(
        "{}",
        crate::dispatch::render_recommendations(&recs, globals.output_format)
    );
    if write {
        println!("{}", apply_recommendations(&root, &recs)?);
    }
    Ok(())
}

/// Apply `recs` to `<project>/.coregraph/config.toml` with a comment-preserving
/// toml_edit merge: array keys append (deduplicated), scalar keys are set.
/// Returns a human summary of what changed. The file must exist (it is
/// auto-created on first index); a parse failure or an existing value of an
/// incompatible shape (e.g. `exclude` written as a string instead of an
/// array) aborts before anything is written.
fn apply_recommendations(root: &Path, recs: &Recommendations) -> anyhow::Result<String> {
    if recs.is_empty() {
        return Ok("nothing to apply".to_string());
    }

    let config_path = project_config_path(root);
    let text = std::fs::read_to_string(&config_path).with_context(|| {
        format!(
            "could not read {}; run `coregraph index` first to create it",
            config_path.display()
        )
    })?;

    let mut doc = text
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse TOML in {}", config_path.display()))?;

    let mut summary_lines: Vec<String> = Vec::new();

    // --- [index].exclude ---
    if !recs.index_exclude.is_empty() {
        let arr = ensure_array(&mut doc, "index", "exclude")?;
        for noise in &recs.index_exclude {
            let rel = noise
                .file
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| noise.file.clone());
            let path_str = rel.to_string_lossy().into_owned();
            if !array_contains(arr, &path_str) {
                arr.push(path_str.as_str());
                summary_lines.push(format!("index.exclude += {:?}", path_str));
            }
        }
    }

    // --- [index].string_match_max_files ---
    if let Some(ref cap) = recs.string_match {
        ensure_table_section(&mut doc, "index")?;
        let target = cap.recommended_cap as i64;
        // Safe get: `get` returns None when the key is absent, unlike
        // the indexing operator which panics.
        let already = doc["index"]
            .get("string_match_max_files")
            .and_then(|v| v.as_value())
            .and_then(|v| v.as_integer())
            .map(|n| n == target)
            .unwrap_or(false);
        if !already {
            // `target` is the single source for both the written value and
            // the summary line, so the two can never drift apart.
            doc["index"]["string_match_max_files"] = toml_edit::value(target);
            summary_lines.push(format!("index.string_match_max_files = {target}"));
        }
    }

    // --- [inconsistencies].disable ---
    if recs.disable_api_path.is_some() {
        let arr = ensure_array(&mut doc, "inconsistencies", "disable")?;
        let api_path = "api-path";
        if !array_contains(arr, api_path) {
            arr.push(api_path);
            summary_lines.push(r#"inconsistencies.disable += "api-path""#.to_string());
        }
    }

    // --- [analysis].exclude ---
    if !recs.analysis_exclude.is_empty() {
        let arr = ensure_array(&mut doc, "analysis", "exclude")?;
        for gen in &recs.analysis_exclude {
            let rel = gen
                .file
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| gen.file.clone());
            let path_str = rel.to_string_lossy().into_owned();
            if !array_contains(arr, &path_str) {
                arr.push(path_str.as_str());
                summary_lines.push(format!("analysis.exclude += {:?}", path_str));
            }
        }
    }

    if summary_lines.is_empty() {
        return Ok("config already contains all recommended values".to_string());
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(summary_lines.join("\n"))
}

/// Ensure `doc[section]` exists as an explicit table, creating it when
/// absent. Errors when the name already holds a non-table value: silently
/// replacing user data would be destructive, so the user is asked to fix
/// the file manually.
fn ensure_table_section(doc: &mut DocumentMut, section: &str) -> anyhow::Result<()> {
    let item = doc.entry(section).or_insert(toml_edit::table());
    if item.as_table_like().is_none() {
        return Err(anyhow!(
            "[{section}] in config.toml is not a table; fix it manually before using --write"
        ));
    }
    Ok(())
}

/// Get or create the array at `[section].<key>` and return a mutable
/// reference to it. The section table and the key are created when absent.
/// Errors when the section or the key already holds an incompatible value
/// (non-table section, non-array key): silently replacing user data would
/// be destructive, so the user is asked to fix the file manually. Callers
/// persist the document only on full success, so an error leaves the file
/// untouched.
fn ensure_array<'a>(
    doc: &'a mut DocumentMut,
    section: &str,
    key: &str,
) -> anyhow::Result<&'a mut toml_edit::Array> {
    ensure_table_section(doc, section)?;
    let item = &mut doc[section];
    // `get` returns None for an absent key, unlike the immutable indexing
    // operator which panics.
    if item.get(key).is_none() {
        item[key] = toml_edit::value(toml_edit::Array::new());
    }
    item[key]
        .as_value_mut()
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| {
            anyhow!(
                "[{section}].{key} in config.toml is not an array; fix it manually before using --write"
            )
        })
}

/// Check whether `arr` already contains `s` as a string element.
// NOTE: exact byte equality only — semantically equivalent variants like
// "./foo" and "foo" are treated as distinct. Intentional for simplicity:
// the recommendation engine always emits normalized paths (no leading ./
// or trailing /), so duplicates only arise from hand-entered variants.
fn array_contains(arr: &toml_edit::Array, s: &str) -> bool {
    arr.iter()
        .any(|v| v.as_str().map(|x| x == s).unwrap_or(false))
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
    index_table.insert("string_match_max_files".to_string(), Value::Integer(8));
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

    // `[inconsistencies]` — detector tuning. `disable` suppresses categories
    // in the run-everything mode (explicit --category still runs them);
    // api_path_min_segments floors the near-miss segment count.
    let mut inconsistencies_table = toml::map::Map::new();
    inconsistencies_table.insert("disable".to_string(), Value::Array(Vec::new()));
    inconsistencies_table.insert("api_path_min_segments".to_string(), Value::Integer(2));
    t.insert(
        "inconsistencies".to_string(),
        Value::Table(inconsistencies_table),
    );

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Prepend a header with an inline description per key so users can
    // see at a glance what knobs exist. The `limits.*` keys feed
    // `GlobalOpts` (via `apply_config_file`) and the `[index]`/`[analysis]`
    // exclude lists feed `PathExcluder`; the `server.*` keys are consumed
    // separately by `server_overrides()` when a foreground daemon starts.
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
    text.push_str("#   string_match_max_files: skip cross-file StringMatch pairing for a\n");
    text.push_str("#            string value found in more than N distinct files (convention\n");
    text.push_str("#            strings would otherwise emit O(k²) edges). 0 = unlimited.\n");
    text.push_str("#\n");
    text.push_str("# [analysis] — analysis-surface knobs.\n");
    text.push_str("#   exclude: gitignore-syntax patterns for files still PARSED\n");
    text.push_str("#            (their edges keep referenced symbols connected) but\n");
    text.push_str("#            whose own symbols are hidden from dead-code (orphans)\n");
    text.push_str("#            reports. Prefer this over index.exclude for generated\n");
    text.push_str("#            consumers like routeTree.gen.ts. Example: [\"**/*.gen.ts\"]\n");
    text.push_str("#\n");
    text.push_str("# [inconsistencies] — cross-file detector tuning.\n");
    text.push_str("#   disable: categories to skip when running without --category\n");
    text.push_str("#            (e.g. [\"api-path\"] for a frontend-only repo where\n");
    text.push_str("#            route literals have no server counterpart).\n");
    text.push_str("#   api_path_min_segments: minimum non-empty path segments for an\n");
    text.push_str("#            api-path near-miss report (default 2).\n");
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
    for w in validate_project_config(project_root) {
        println!("  ⚠ {w}");
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

/// Known config sections and the keys each may contain. Used only for
/// diagnostics — unknown entries are warned about, never rejected, so a
/// newer config keeps working with an older binary. Must remain a superset
/// of the dotted `KNOWN_KEYS` entries — enforced by the
/// `known_sections_subsumes_known_keys` test.
const KNOWN_SECTIONS: &[(&str, &[&str])] = &[
    ("limits", &["token_budget", "hop_limit", "min_confidence"]),
    ("index", &["exclude", "string_match_max_files"]),
    ("analysis", &["exclude"]),
    (
        "server",
        &[
            "max_loaded_projects",
            "graceful_shutdown_sec",
            "idle_unload_minutes",
            "max_loaded_bytes",
        ],
    ),
    ("inconsistencies", &["disable", "api_path_min_segments"]),
];

/// Lint `<project>/.coregraph/config.toml`: report a TOML parse failure,
/// unknown top-level sections, and unknown keys inside known sections.
/// Returns human-readable warnings; an absent file returns none (it is
/// auto-created elsewhere). Read-only — never mutates the file.
pub fn validate_project_config(project_root: &Path) -> Vec<String> {
    let path = project_config_path(project_root);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let table = match toml::from_str::<Value>(&text) {
        Ok(Value::Table(t)) => t,
        Ok(_) => {
            return vec![format!(
                "config {} is not a TOML table — all settings ignored",
                path.display()
            )]
        }
        Err(e) => {
            return vec![format!(
                "failed to parse {}: {e} — all settings in it are ignored",
                path.display()
            )]
        }
    };
    let mut warnings = Vec::new();
    for (section, value) in &table {
        let Some((_, known_keys)) = KNOWN_SECTIONS.iter().find(|(s, _)| s == section) else {
            warnings.push(format!(
                "unknown config section [{section}] in {} (known: {})",
                path.display(),
                KNOWN_SECTIONS
                    .iter()
                    .map(|(s, _)| *s)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            continue;
        };
        if let Value::Table(entries) = value {
            for key in entries.keys() {
                if !known_keys.contains(&key.as_str()) {
                    warnings.push(format!(
                        "unknown config key {section}.{key} in {} (known {section} keys: {})",
                        path.display(),
                        known_keys.join(", ")
                    ));
                }
            }
        }
    }
    warnings
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
        assert!(body.contains("string_match_max_files"));
        assert!(
            body.contains("[inconsistencies]"),
            "missing [inconsistencies]: {}",
            body
        );
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

    #[test]
    fn validate_flags_unknown_sections_and_keys() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(
            &cfg,
            "[limits]\nhop_limt = 3\n[indxe]\nexclude = []\n[index]\nexclude = []\n",
        )
        .unwrap();
        let warnings = validate_project_config(dir.path());
        assert!(
            warnings.iter().any(|w| w.contains("indxe")),
            "unknown section must be flagged: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("hop_limt")),
            "unknown key in a known section must be flagged: {warnings:?}"
        );
    }

    #[test]
    fn validate_flags_toml_parse_error() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        // `[index]/` is the field-observed typo: invalid TOML, previously silent.
        std::fs::write(&cfg, "[index]/\nexclude = []\n").unwrap();
        let warnings = validate_project_config(dir.path());
        assert!(
            warnings.iter().any(|w| w.contains("parse")),
            "TOML syntax error must produce a warning: {warnings:?}"
        );
    }

    #[test]
    fn validate_clean_config_is_quiet() {
        let dir = tempfile::tempdir().expect("tmpdir");
        ensure_local_default(dir.path());
        assert!(validate_project_config(dir.path()).is_empty());
    }

    #[test]
    fn known_sections_subsumes_known_keys() {
        // KNOWN_KEYS (runtime-read scalar knobs) and KNOWN_SECTIONS (the
        // validate_project_config whitelist) must not drift: every dotted
        // KNOWN_KEYS entry must be whitelisted, or the validator would warn
        // about a key the runtime genuinely reads.
        for (dotted, _, _) in KNOWN_KEYS {
            let (section, key) = dotted
                .split_once('.')
                .expect("KNOWN_KEYS entries are dotted section.key names");
            let entry = KNOWN_SECTIONS.iter().find(|(s, _)| *s == section);
            assert!(
                entry.is_some(),
                "KNOWN_KEYS section '{section}' missing from KNOWN_SECTIONS"
            );
            assert!(
                entry.unwrap().1.contains(&key),
                "KNOWN_KEYS key '{section}.{key}' missing from KNOWN_SECTIONS"
            );
        }
    }

    // Build a Recommendations value used across the apply_ tests.
    fn make_test_recs(root: &std::path::Path) -> Recommendations {
        use coregraph_query::{
            ApiPathRecommendation, CapRecommendation, CapStep, GeneratedFile, NoiseFile,
        };
        Recommendations {
            index_exclude: vec![NoiseFile {
                // Absolute path under the root — expect relativized in config.
                file: root.join("docs/api/overview.md"),
                data_symbols: 250,
                share_pct: 38,
            }],
            string_match: Some(CapRecommendation {
                current_cap: 8,
                current_edges: 6875,
                recommended_cap: 4,
                projected_edges: 2210,
                total_edges: 61471,
                steps: vec![
                    CapStep {
                        cap: 2,
                        projected_edges: 800,
                    },
                    CapStep {
                        cap: 4,
                        projected_edges: 2210,
                    },
                ],
            }),
            disable_api_path: Some(ApiPathRecommendation { report_count: 25 }),
            analysis_exclude: vec![GeneratedFile {
                file: std::path::PathBuf::from("src/routeTree.gen.ts"),
                marker: "Code generated by".to_string(),
                symbols: 41,
            }],
        }
    }

    #[test]
    fn apply_recommendations_preserves_comments_and_merges() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();

        // Config with a precious comment and a pre-existing exclude entry.
        std::fs::write(
            &cfg,
            "# my precious note\n[index]\nexclude = [\"keep/\"]\n[analysis]\nexclude = []\n[inconsistencies]\ndisable = []\n",
        )
        .unwrap();

        let recs = make_test_recs(dir.path());
        let summary = apply_recommendations(dir.path(), &recs).unwrap();

        // Comment must survive.
        let body = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            body.contains("# my precious note"),
            "user comment was destroyed: {body}"
        );

        // Parse result with plain toml for structural assertions.
        let parsed: toml::Value = toml::from_str(&body).expect("should parse valid TOML");
        let index = parsed["index"].as_table().unwrap();
        let excl: Vec<&str> = index["exclude"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            excl.contains(&"keep/"),
            "pre-existing entry must be kept: {excl:?}"
        );
        assert!(
            excl.contains(&"docs/api/overview.md"),
            "noise file must be appended (relative): {excl:?}"
        );

        assert_eq!(
            index["string_match_max_files"].as_integer(),
            Some(4),
            "cap must be set"
        );

        let inconsistencies = parsed["inconsistencies"].as_table().unwrap();
        let disable: Vec<&str> = inconsistencies["disable"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            disable.contains(&"api-path"),
            "api-path must be in disable: {disable:?}"
        );

        let analysis = parsed["analysis"].as_table().unwrap();
        let analysis_excl: Vec<&str> = analysis["exclude"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            analysis_excl.contains(&"src/routeTree.gen.ts"),
            "generated file must be in analysis.exclude: {analysis_excl:?}"
        );

        // Summary must list applied changes.
        assert!(
            summary.contains("docs/api/overview.md"),
            "summary must mention the noise file: {summary}"
        );
        assert!(
            summary.contains("string_match_max_files = 4"),
            "summary must mention the cap: {summary}"
        );

        // Every key --write produces must be whitelisted in KNOWN_SECTIONS —
        // an applied config must never trip the config lint.
        assert!(
            validate_project_config(dir.path()).is_empty(),
            "applied config must stay lint-clean: {:?}",
            validate_project_config(dir.path())
        );
    }

    #[test]
    fn apply_recommendations_is_idempotent() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(
            &cfg,
            "[index]\nexclude = []\n[analysis]\nexclude = []\n[inconsistencies]\ndisable = []\n",
        )
        .unwrap();

        let recs = make_test_recs(dir.path());

        // Apply once.
        apply_recommendations(dir.path(), &recs).unwrap();
        let body_after_first = std::fs::read_to_string(&cfg).unwrap();

        // Apply a second time — should produce "already contains" and NOT duplicate entries.
        let summary2 = apply_recommendations(dir.path(), &recs).unwrap();
        let body_after_second = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(
            body_after_first, body_after_second,
            "second apply must not modify the file"
        );
        assert!(
            summary2.contains("already contains"),
            "idempotent summary expected: {summary2}"
        );

        // Parse and verify no duplicate entries.
        let parsed: toml::Value = toml::from_str(&body_after_second).expect("valid TOML");
        let excl = parsed["index"]["exclude"].as_array().unwrap();
        let noise_count = excl
            .iter()
            .filter(|v| v.as_str() == Some("docs/api/overview.md"))
            .count();
        assert_eq!(
            noise_count, 1,
            "noise file must appear exactly once: {excl:?}"
        );

        let disable = parsed["inconsistencies"]["disable"].as_array().unwrap();
        let api_count = disable
            .iter()
            .filter(|v| v.as_str() == Some("api-path"))
            .count();
        assert_eq!(
            api_count, 1,
            "api-path must appear exactly once: {disable:?}"
        );
    }

    #[test]
    fn apply_recommendations_empty_is_noop() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        let original = "# sentinel\n[index]\nexclude = []\n";
        std::fs::write(&cfg, original).unwrap();

        let empty_recs = Recommendations::default();
        let summary = apply_recommendations(dir.path(), &empty_recs).unwrap();

        // File must be byte-identical.
        let body = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(body, original, "empty recs must not touch the file");
        assert!(
            summary.contains("nothing to apply"),
            "empty recs summary expected: {summary}"
        );
    }

    #[test]
    fn apply_recommendations_rejects_non_array_key_without_writing() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        // User mistake: `exclude` holds a string scalar instead of an array.
        // --write must refuse instead of silently replacing user data.
        let original = "[index]\nexclude = \"keep/\"\n";
        std::fs::write(&cfg, original).unwrap();

        let recs = make_test_recs(dir.path());
        let err = apply_recommendations(dir.path(), &recs)
            .expect_err("a non-array exclude key must abort the apply");
        assert!(
            err.to_string().contains("[index].exclude"),
            "error must name the offending key: {err}"
        );
        let body = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(body, original, "no partial write on error");
    }

    #[test]
    fn apply_recommendations_rejects_non_table_section_without_writing() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        // `index` is a scalar, so no table can be created under that name.
        let original = "index = 5\n";
        std::fs::write(&cfg, original).unwrap();

        let recs = make_test_recs(dir.path());
        let err = apply_recommendations(dir.path(), &recs)
            .expect_err("a non-table [index] must abort the apply");
        assert!(
            err.to_string().contains("[index]"),
            "error must name the offending section: {err}"
        );
        let body = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(body, original, "no partial write on error");
    }

    #[test]
    fn apply_recommendations_creates_missing_sections_and_keys() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        // None of the target sections or keys exist yet — each must be
        // created rather than panicking on the absent-key lookup.
        std::fs::write(&cfg, "# bare config\n").unwrap();

        let recs = make_test_recs(dir.path());
        apply_recommendations(dir.path(), &recs).unwrap();

        let body = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            body.contains("# bare config"),
            "comment must survive: {body}"
        );
        let parsed: toml::Value = toml::from_str(&body).expect("valid TOML");
        let excl = parsed["index"]["exclude"].as_array().unwrap();
        assert!(
            excl.iter()
                .any(|v| v.as_str() == Some("docs/api/overview.md")),
            "noise file must be present: {excl:?}"
        );
        assert_eq!(
            parsed["index"]["string_match_max_files"].as_integer(),
            Some(4)
        );
        let disable = parsed["inconsistencies"]["disable"].as_array().unwrap();
        assert!(
            disable.iter().any(|v| v.as_str() == Some("api-path")),
            "api-path must be present: {disable:?}"
        );
        let analysis_excl = parsed["analysis"]["exclude"].as_array().unwrap();
        assert!(
            analysis_excl
                .iter()
                .any(|v| v.as_str() == Some("src/routeTree.gen.ts")),
            "generated file must be present: {analysis_excl:?}"
        );
    }

    #[test]
    fn apply_recommendations_outside_root_path_falls_back_to_absolute() {
        use coregraph_query::NoiseFile;
        let dir = tempfile::tempdir().expect("tmpdir");
        let other = tempfile::tempdir().expect("other tmpdir");
        let cfg = project_config_path(dir.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "[index]\nexclude = []\n").unwrap();

        // Degenerate case the engine never produces — graph node paths always
        // live under the indexed root — pinned here only to document the
        // strip_prefix fallback: a path outside the root is written verbatim
        // as an absolute gitignore pattern.
        let outside = other.path().join("vendor/bundle.json");
        let recs = Recommendations {
            index_exclude: vec![NoiseFile {
                file: outside.clone(),
                data_symbols: 300,
                share_pct: 12,
            }],
            ..Default::default()
        };

        let summary = apply_recommendations(dir.path(), &recs).unwrap();
        let expected = outside.to_string_lossy().into_owned();
        assert!(
            summary.contains(&expected),
            "summary must carry the absolute fallback: {summary}"
        );

        let body = std::fs::read_to_string(&cfg).unwrap();
        let parsed: toml::Value = toml::from_str(&body).expect("valid TOML");
        let excl = parsed["index"]["exclude"].as_array().unwrap();
        assert!(
            excl.iter().any(|v| v.as_str() == Some(expected.as_str())),
            "absolute path fallback must be written verbatim: {excl:?}"
        );
    }
}
