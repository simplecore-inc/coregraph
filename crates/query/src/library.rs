//! Library-vs-application awareness for orphan reporting.
//!
//! A symbol that is the public API of a *library* package is consumed by
//! downstream code outside this repository, so its being unreferenced *within*
//! the repo does not make it dead code. This module reads the project manifest
//! (via `coregraph-manifest`) to decide, per package, whether it is a library,
//! and maps each symbol's file to its owning package by longest path prefix.
//!
//! Detection is intentionally coarse: "the package is a library → its public
//! symbols are likely API surface". It does NOT compute the precise re-exported
//! surface (a library can have genuinely-dead `pub` internals that are never
//! re-exported) — that needs the type-aware resolution tracked separately. The
//! coarse signal is the right trade for false-positive reduction; consumers
//! label rather than drop so the over-suppression is never silent.

use std::path::{Path, PathBuf};

use coregraph_manifest::{parse_project, PackageKind};

/// Classifies whether the package owning a source file is a library, using
/// manifest signals with a `[project] kind` config override escape hatch.
pub struct LibraryClassifier {
    root: PathBuf,
    /// `(package path relative to root, kind)`, longest paths first so the most
    /// specific (deepest) package wins the longest-prefix match.
    packages: Vec<(PathBuf, PackageKind)>,
    /// `[project] kind = "library"|"application"` in `.coregraph/config.toml`,
    /// applied to the whole project, taking precedence over manifest detection.
    override_kind: Option<PackageKind>,
}

impl LibraryClassifier {
    /// Build a classifier for `root` by parsing its manifest and reading the
    /// optional config override. Parsing failure yields a classifier that
    /// classifies nothing as a library (no adjustment) rather than erroring.
    pub fn from_project_root(root: &Path) -> Self {
        let override_kind = read_project_kind_override(root);
        let packages = parse_project(root)
            .map(|m| m.packages.into_iter().map(|p| (p.path, p.kind)).collect())
            .unwrap_or_default();
        Self::from_parts(root.to_path_buf(), packages, override_kind)
    }

    /// Assemble from explicit parts (kept public for deterministic tests and for
    /// callers that already hold a parsed manifest).
    pub fn from_parts(
        root: PathBuf,
        mut packages: Vec<(PathBuf, PackageKind)>,
        override_kind: Option<PackageKind>,
    ) -> Self {
        // Longest path first so a nested package (`packages/x`) is matched
        // before the root package (`.`).
        packages.sort_by_key(|p| std::cmp::Reverse(p.0.as_os_str().len()));
        Self {
            root,
            packages,
            override_kind,
        }
    }

    /// Whether the package owning `file` is a library. Returns `false` when the
    /// owning package is an application or undecidable — i.e. no adjustment is
    /// the default, so this never *hides* dead code unless a library is proven.
    pub fn is_library_file(&self, file: &Path) -> bool {
        if let Some(k) = self.override_kind {
            return k == PackageKind::Library;
        }
        let rel = file.strip_prefix(&self.root).unwrap_or(file);
        self.packages
            .iter()
            .find(|(pkg_path, _)| package_contains(pkg_path, rel))
            .is_some_and(|(_, kind)| *kind == PackageKind::Library)
    }

    /// Whether manifest detection (or the override) found any package at all —
    /// lets callers phrase the note differently when nothing was detected.
    pub fn has_signal(&self) -> bool {
        self.override_kind.is_some() || !self.packages.is_empty()
    }
}

/// A package whose relative path is `pkg_path` owns `rel_file` when the file
/// sits under it. The root package (`.` or empty) owns everything not claimed
/// by a deeper package (the caller sorts longest-first so deeper wins).
fn package_contains(pkg_path: &Path, rel_file: &Path) -> bool {
    if pkg_path.as_os_str().is_empty() || pkg_path == Path::new(".") {
        return true;
    }
    rel_file.starts_with(pkg_path)
}

/// Read `[project] kind = "library"|"application"` from `.coregraph/config.toml`.
/// Absent file/key (or an unrecognised value) yields `None` (detection decides).
fn read_project_kind_override(root: &Path) -> Option<PackageKind> {
    let cfg = root.join(".coregraph").join("config.toml");
    let text = std::fs::read_to_string(&cfg).ok()?;
    let parsed = toml::from_str::<toml::Value>(&text).ok()?;
    let kind = parsed
        .as_table()?
        .get("project")?
        .as_table()?
        .get("kind")?
        .as_str()?;
    match kind.to_ascii_lowercase().as_str() {
        "library" | "lib" => Some(PackageKind::Library),
        "application" | "app" | "binary" | "bin" => Some(PackageKind::Application),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cls(pkgs: &[(&str, PackageKind)]) -> LibraryClassifier {
        LibraryClassifier::from_parts(
            PathBuf::from("/proj"),
            pkgs.iter().map(|(p, k)| (PathBuf::from(p), *k)).collect(),
            None,
        )
    }

    #[test]
    fn longest_prefix_package_wins() {
        // A monorepo with an app at the root and a library subpackage: a file in
        // the subpackage is a library file; a file only under the root is not.
        let c = cls(&[
            (".", PackageKind::Application),
            ("packages/excalidraw", PackageKind::Library),
        ]);
        assert!(c.is_library_file(Path::new("/proj/packages/excalidraw/src/index.ts")));
        assert!(!c.is_library_file(Path::new("/proj/excalidraw-app/src/main.ts")));
    }

    #[test]
    fn single_library_package_covers_all_files() {
        let c = cls(&[(".", PackageKind::Library)]);
        assert!(c.is_library_file(Path::new("/proj/src/api.rs")));
    }

    #[test]
    fn application_package_is_not_library() {
        let c = cls(&[(".", PackageKind::Application)]);
        assert!(!c.is_library_file(Path::new("/proj/src/main.rs")));
    }

    #[test]
    fn unknown_or_unmatched_is_not_library() {
        let c = cls(&[("sub", PackageKind::Library)]);
        // File outside any package path → no match → not a library.
        assert!(!c.is_library_file(Path::new("/proj/other/x.rs")));
        let u = cls(&[(".", PackageKind::Unknown)]);
        assert!(!u.is_library_file(Path::new("/proj/x.rs")));
    }

    #[test]
    fn config_override_is_read_from_disk() {
        // `[project] kind = "library"` in .coregraph/config.toml forces the
        // whole project to library, regardless of (here absent) manifest.
        let dir = tempfile::tempdir().unwrap();
        let cfg_dir = dir.path().join(".coregraph");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.toml"),
            "[project]\nkind = \"library\"\n",
        )
        .unwrap();
        let c = LibraryClassifier::from_project_root(dir.path());
        assert!(c.has_signal(), "override is a signal");
        assert!(
            c.is_library_file(&dir.path().join("src/x.rs")),
            "override → library"
        );

        std::fs::write(
            cfg_dir.join("config.toml"),
            "[project]\nkind = \"application\"\n",
        )
        .unwrap();
        let c2 = LibraryClassifier::from_project_root(dir.path());
        assert!(
            !c2.is_library_file(&dir.path().join("src/x.rs")),
            "override → application"
        );
    }

    #[test]
    fn override_takes_precedence_over_packages() {
        let c = LibraryClassifier::from_parts(
            PathBuf::from("/proj"),
            vec![(PathBuf::from("."), PackageKind::Application)],
            Some(PackageKind::Library),
        );
        assert!(
            c.is_library_file(Path::new("/proj/src/main.rs")),
            "override → library"
        );
    }
}
