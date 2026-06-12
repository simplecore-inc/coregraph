//! Path exclusion for analysis commands.
//!
//! Exclusion patterns live in `.coregraph/config.toml` under the
//! `index.exclude` array (gitignore pattern syntax). Previously this
//! module also read a standalone `.coregraph/ignore` file; that file
//! is now gone — everything lives in the single config file so users
//! have one place to look for project-specific knobs.
//!
//! Default patterns (`DEFAULT_EXCLUDE_PATTERNS` below) are always
//! applied — they cover universal build outputs, dependency caches,
//! VCS metadata and IDE folders that are never meaningful for code
//! analysis regardless of language or project layout. The user's
//! `index.exclude` list is appended on top, so project-specific
//! exclusions (e.g. `tests/fixtures/`) add to the defaults rather
//! than replacing them.
//!
//! Per-project runtime artifacts (config, snapshots) all live under
//! the project's `.coregraph/` directory.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};

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
const DEFAULT_EXCLUDE_PATTERNS: &[&str] = &[
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

/// Built from `<project>/.coregraph/config.toml` at the project root.
/// The same type backs two flavors: the index excluder
/// (`from_project_root`, reads `index.exclude` and always applies the
/// universal `DEFAULT_EXCLUDE_PATTERNS`) and the analysis excluder
/// (`analysis_from_project_root`, reads `analysis.exclude` and applies
/// no defaults). For the index excluder, even when the config file is
/// missing or the `index.exclude` array is absent / empty the defaults
/// still match (e.g. `target/`, `node_modules/`). The analysis
/// excluder, by contrast, matches nothing when its section is absent.
pub struct PathExcluder {
    matcher: Option<Gitignore>,
    root: PathBuf,
}

impl PathExcluder {
    /// Load `index.exclude` from `<project>/.coregraph/config.toml`.
    /// Falls back to a no-op excluder when the file / key is absent.
    ///
    /// The root is stored as-is (no canonicalization) so that matching is
    /// symmetric with call sites that pass the same `-C` value. Target
    /// paths are lightly cleaned (`./` components dropped) before
    /// gitignore matching.
    pub fn from_project_root(project_root: &Path) -> Self {
        Self::build(
            project_root,
            true,
            &read_exclude_patterns(project_root, "index"),
        )
    }

    /// Build an excluder from `[analysis].exclude` only, WITHOUT the universal
    /// default patterns. This is the *analysis-surface* exclude: a file matched
    /// here is still parsed and indexed (so its outgoing edges keep the symbols
    /// it references connected), but its own symbols are suppressed from
    /// analysis reports (orphans). It exists because the hard `[index].exclude`
    /// drops a file's edges too — so excluding a generated consumer (e.g.
    /// `routeTree.gen.ts`) silently orphans the hand-written symbols only it
    /// referenced. Defaults are not applied here: build-output / dependency
    /// directories are an index-time concern and are never parsed anyway.
    pub fn analysis_from_project_root(project_root: &Path) -> Self {
        Self::build(
            project_root,
            false,
            &read_exclude_patterns(project_root, "analysis"),
        )
    }

    /// Shared constructor. `include_defaults` adds `DEFAULT_EXCLUDE_PATTERNS`
    /// before the user patterns (index excluder); analysis excluders pass
    /// `false`. A malformed pattern is skipped with a warning on stderr; the
    /// remaining defaults and user patterns continue to apply unaffected.
    fn build(project_root: &Path, include_defaults: bool, user_patterns: &[String]) -> Self {
        let root = project_root.to_path_buf();
        let mut builder = GitignoreBuilder::new(&root);
        if include_defaults {
            // Default patterns always apply — see DEFAULT_EXCLUDE_PATTERNS doc.
            // They are compile-time constants, so a failure here is a bug, not
            // user input; still skip-and-warn instead of disabling everything.
            for p in DEFAULT_EXCLUDE_PATTERNS {
                if builder.add_line(None, p).is_err() {
                    eprintln!(
                        "[coregraph] WARNING: invalid built-in exclude pattern '{p}' skipped"
                    );
                }
            }
        }
        // User patterns are appended so they can layer on top of the
        // defaults (adding new excludes or negating them with `!`).
        // A malformed pattern is skipped with a warning; it must not
        // disable the defaults or the user's other patterns.
        for p in user_patterns {
            if builder.add_line(None, p).is_err() {
                eprintln!(
                    "[coregraph] WARNING: invalid exclude pattern '{p}' in .coregraph/config.toml skipped"
                );
            }
        }
        match builder.build() {
            Ok(m) => Self {
                matcher: Some(m),
                root,
            },
            Err(e) => {
                eprintln!("[coregraph] WARNING: exclude matcher failed to build ({e}); no excludes applied");
                Self {
                    matcher: None,
                    root,
                }
            }
        }
    }

    /// `true` when `path` matches any active pattern for this excluder —
    /// the configured user patterns plus, for the index excluder, the
    /// universal `DEFAULT_EXCLUDE_PATTERNS`. So an index excluder can
    /// return `true` even with no user patterns configured (a default
    /// matched). Malformed patterns are silently skipped during construction
    /// (with a stderr warning), so only a complete builder failure or no
    /// match causes this to return `false`.
    pub fn is_excluded(&self, path: &Path) -> bool {
        let Some(m) = &self.matcher else {
            return false;
        };
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let cleaned = clean_path(&absolute);
        // `matched_path_or_any_parents` so that a pattern like
        // `tests/fixtures/` also excludes files nested below that
        // directory. Plain `matched` only checks the leaf component and
        // would miss `tests/fixtures/generics/foo.rs`.
        m.matched_path_or_any_parents(&cleaned, cleaned.is_dir())
            .is_ignore()
    }
}

/// Read `<section>.exclude` (e.g. `index.exclude` or `analysis.exclude`) as a
/// flat list of gitignore-syntax patterns. Returns an empty vector when the
/// config file is missing or when the section / key is absent or not an array
/// (none of which is an error). A TOML parse failure emits a stderr warning
/// and also returns empty.
fn read_exclude_patterns(root: &Path, section: &str) -> Vec<String> {
    let config_path = root.join(".coregraph").join("config.toml");
    let Ok(text) = std::fs::read_to_string(&config_path) else {
        return Vec::new();
    };
    let parsed = match toml::from_str::<toml::Value>(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "[coregraph] WARNING: failed to parse {}: {e} — exclude patterns ignored",
                config_path.display()
            );
            return Vec::new();
        }
    };
    let Some(table) = parsed.as_table() else {
        return Vec::new();
    };
    let Some(section_tbl) = table.get(section).and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    let Some(exclude) = section_tbl.get("exclude").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    exclude
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect()
}

/// Strip `./` components so gitignore matching is symmetric between call
/// sites that pass `./tests/foo` and those that pass `tests/foo`. Does NOT
/// touch the filesystem (no symlink resolution, no `canonicalize`) — the
/// path may not exist at the time of the check (healing, watch events).
fn clean_path(p: &Path) -> PathBuf {
    let cleaned: PathBuf = p
        .components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .collect();
    if cleaned.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Write a minimal `config.toml` with the given `index.exclude`
    /// patterns — `serde_json::to_string` would quote specials wrong,
    /// so we assemble the TOML by hand for readability in tests.
    fn write_config_with_excludes(root: &Path, patterns: &[&str]) {
        fs::create_dir_all(root.join(".coregraph")).unwrap();
        let mut body = String::from("[index]\nexclude = [\n");
        for p in patterns {
            body.push_str(&format!("  {:?},\n", p));
        }
        body.push_str("]\n");
        fs::write(root.join(".coregraph").join("config.toml"), body).unwrap();
    }

    #[test]
    fn missing_config_only_applies_defaults() {
        // No config file at all — regular source passes, but the
        // universal default patterns (see DEFAULT_EXCLUDE_PATTERNS)
        // still catch build/deps directories.
        let tmp = tempfile::tempdir().unwrap();
        let e = PathExcluder::from_project_root(tmp.path());
        assert!(!e.is_excluded(&tmp.path().join("src/main.rs")));
        assert!(!e.is_excluded(&tmp.path().join("anything/goes")));
    }

    #[test]
    fn empty_exclude_array_keeps_defaults() {
        // An explicit empty array is the common post-auto-init state —
        // user pattern list is empty but the baked-in defaults still
        // apply (otherwise a fresh project would re-index target/).
        let tmp = tempfile::tempdir().unwrap();
        write_config_with_excludes(tmp.path(), &[]);
        let e = PathExcluder::from_project_root(tmp.path());
        assert!(!e.is_excluded(&tmp.path().join("src/main.rs")));
        // Default still active even though user's list is empty.
        assert!(e.is_excluded(&tmp.path().join("target/debug/foo.rs")));
    }

    #[test]
    fn exclude_dir_glob() {
        let tmp = tempfile::tempdir().unwrap();
        write_config_with_excludes(tmp.path(), &["tests/fixtures/"]);
        let e = PathExcluder::from_project_root(tmp.path());
        assert!(e.is_excluded(&tmp.path().join("tests/fixtures/sample.rs")));
        assert!(!e.is_excluded(&tmp.path().join("src/main.rs")));
    }

    #[test]
    fn relative_path_resolved_against_root() {
        let tmp = tempfile::tempdir().unwrap();
        write_config_with_excludes(tmp.path(), &["target/"]);
        let e = PathExcluder::from_project_root(tmp.path());
        // Relative path resolved against the project root.
        assert!(e.is_excluded(Path::new("target/debug/build.log")));
        assert!(!e.is_excluded(Path::new("src/lib.rs")));
    }

    #[test]
    fn negation_and_nested_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        write_config_with_excludes(tmp.path(), &["**/generated/", "!**/generated/keep_me.rs"]);
        let e = PathExcluder::from_project_root(tmp.path());
        assert!(e.is_excluded(&tmp.path().join("a/generated/x.rs")));
        assert!(!e.is_excluded(&tmp.path().join("a/generated/keep_me.rs")));
    }

    #[test]
    fn default_patterns_exclude_universal_dirs() {
        // No config at all — defaults should still kick in so common
        // build/deps/VCS paths don't leak into analysis results.
        let tmp = tempfile::tempdir().unwrap();
        let e = PathExcluder::from_project_root(tmp.path());
        assert!(e.is_excluded(&tmp.path().join("target/debug/deps/foo.rs")));
        assert!(e.is_excluded(&tmp.path().join("node_modules/foo/index.js")));
        assert!(e.is_excluded(&tmp.path().join("build/classes/Foo.class")));
        assert!(e.is_excluded(&tmp.path().join("dist/bundle.js")));
        assert!(e.is_excluded(&tmp.path().join(".git/HEAD")));
        assert!(e.is_excluded(&tmp.path().join(".idea/workspace.xml")));
        assert!(e.is_excluded(&tmp.path().join(".gradle/7.5/executionHistory")));
        assert!(e.is_excluded(&tmp.path().join("__pycache__/module.cpython-311.pyc")));
        // Regular source files are not excluded.
        assert!(!e.is_excluded(&tmp.path().join("src/main.rs")));
        assert!(!e.is_excluded(&tmp.path().join("lib/foo.py")));
    }

    #[test]
    fn default_patterns_apply_to_nested_monorepo_paths() {
        // Gradle / pnpm workspaces / Cargo workspaces often have a
        // build or deps directory deep inside the tree.
        let tmp = tempfile::tempdir().unwrap();
        let e = PathExcluder::from_project_root(tmp.path());
        assert!(e.is_excluded(&tmp.path().join("apps/foo/target/debug/x")));
        assert!(e.is_excluded(&tmp.path().join("services/bar/node_modules/y")));
        assert!(e.is_excluded(&tmp.path().join("packages/baz/build/classes/z")));
    }

    #[test]
    fn user_patterns_layer_on_top_of_defaults() {
        // User's config extends the defaults rather than replacing them.
        let tmp = tempfile::tempdir().unwrap();
        write_config_with_excludes(tmp.path(), &["tests/fixtures/"]);
        let e = PathExcluder::from_project_root(tmp.path());
        // Default still applies
        assert!(e.is_excluded(&tmp.path().join("target/debug/foo.rs")));
        // User pattern applies
        assert!(e.is_excluded(&tmp.path().join("tests/fixtures/sample.rs")));
        // Unrelated source still passes
        assert!(!e.is_excluded(&tmp.path().join("src/main.rs")));
    }

    #[test]
    fn user_can_negate_default_pattern() {
        // Escape hatch for projects that genuinely have source inside
        // one of the default-excluded directories (e.g. generator output
        // committed under target/). Gitignore negation syntax restores it.
        let tmp = tempfile::tempdir().unwrap();
        write_config_with_excludes(tmp.path(), &["!target/keep/"]);
        let e = PathExcluder::from_project_root(tmp.path());
        // Default target/ still excludes most of the tree.
        assert!(e.is_excluded(&tmp.path().join("target/debug/x.rs")));
        // Negation restores the specific subtree the user wants to index.
        assert!(!e.is_excluded(&tmp.path().join("target/keep/foo.rs")));
    }

    #[test]
    fn analysis_excluder_reads_analysis_section_without_defaults() {
        // `analysis_from_project_root` reads `[analysis].exclude` (NOT
        // `[index].exclude`) and applies NO universal defaults — it suppresses
        // user-named files from analysis reports while leaving them indexed.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".coregraph")).unwrap();
        fs::write(
            tmp.path().join(".coregraph").join("config.toml"),
            "[index]\nexclude = [\"only_index/\"]\n[analysis]\nexclude = [\"**/*.gen.ts\"]\n",
        )
        .unwrap();
        let e = PathExcluder::analysis_from_project_root(tmp.path());
        // The analysis pattern matches.
        assert!(e.is_excluded(&tmp.path().join("src/routeTree.gen.ts")));
        // The `[index]` pattern is NOT honored by the analysis excluder.
        assert!(!e.is_excluded(&tmp.path().join("only_index/x.ts")));
        // Universal defaults are NOT applied (build dirs are an index concern).
        assert!(!e.is_excluded(&tmp.path().join("target/debug/x.rs")));
        // Regular source passes.
        assert!(!e.is_excluded(&tmp.path().join("src/main.ts")));
    }

    #[test]
    fn analysis_excluder_absent_section_is_noop() {
        // No `[analysis]` section → analysis excluder matches nothing (and does
        // not fall back to defaults), so ordinary analysis is unchanged.
        let tmp = tempfile::tempdir().unwrap();
        write_config_with_excludes(tmp.path(), &["**/*.gen.ts"]); // [index] only
        let e = PathExcluder::analysis_from_project_root(tmp.path());
        assert!(!e.is_excluded(&tmp.path().join("src/routeTree.gen.ts")));
        assert!(!e.is_excluded(&tmp.path().join("target/debug/x.rs")));
    }

    #[test]
    fn legacy_ignore_file_is_no_longer_read() {
        // Regression guard for the migration — a user who still has
        // an old `.coregraph/ignore` file but no config must not see
        // phantom exclusions.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".coregraph")).unwrap();
        fs::write(
            tmp.path().join(".coregraph").join("ignore"),
            "tests/fixtures/\n",
        )
        .unwrap();
        let e = PathExcluder::from_project_root(tmp.path());
        assert!(!e.is_excluded(&tmp.path().join("tests/fixtures/sample.rs")));
    }

    #[test]
    fn malformed_pattern_does_not_disable_other_patterns() {
        // One bad glob must not nuke the matcher: the defaults and the other
        // user patterns must keep matching, and only the bad pattern is dropped.
        // "a{b" is confirmed to error on add_line (unclosed alternate group);
        // "a[" does NOT error in the ignore crate and is therefore not used here.
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg_dir = dir.path().join(".coregraph");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.toml"),
            "[index]\nexclude = [\"a{b\", \"generated/\"]\n",
        )
        .unwrap();
        let ex = PathExcluder::from_project_root(dir.path());
        assert!(
            ex.is_excluded(&dir.path().join("node_modules/x.js")),
            "default patterns must survive a malformed user pattern"
        );
        assert!(
            ex.is_excluded(&dir.path().join("generated/file.ts")),
            "valid user patterns must survive a malformed sibling pattern"
        );
        assert!(
            !ex.is_excluded(&dir.path().join("src/main.ts")),
            "unrelated files must not be excluded"
        );
    }
}
