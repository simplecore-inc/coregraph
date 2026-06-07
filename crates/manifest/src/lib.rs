// `ManifestError` carries a `PathBuf` plus a parser error in several variants.
// On Windows `OsString`/`PathBuf` is wider, pushing the enum just over clippy's
// `result_large_err` size threshold (it stays under on Unix). The type is
// returned by value throughout the parsers; boxing every result to satisfy a
// platform-dependent size threshold would add indirection without real benefit.
#![allow(clippy::result_large_err)]

pub mod cargo_parser;
pub mod detector;
pub mod error;
pub mod filter;
pub mod go;
pub mod gradle;
pub mod maven;
pub mod npm;
pub mod python;
pub mod types;
pub mod vite;

pub use detector::ProjectDetector;
pub use error::ManifestError;
pub use filter::FileFilter;
pub use types::{DependsOn, ExternalPackage, Language, Package, PackageKind, ProjectManifest};

use std::path::Path;

/// Trait implemented by every build-system parser.
pub trait ManifestParser: Send + Sync {
    /// Human-readable name of this parser (e.g. "cargo", "npm/pnpm/yarn").
    fn name(&self) -> &'static str;

    /// Returns `true` if the given `root` directory looks like a project this parser can handle.
    fn can_parse(&self, root: &Path) -> bool;

    /// Parse the project rooted at `root` and return a `ProjectManifest`.
    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError>;
}

/// Parse a project using all detected parsers and merge the results.
pub fn parse_project(root: &Path) -> Result<ProjectManifest, ManifestError> {
    let parsers = ProjectDetector::detect(root);
    let mut merged = ProjectManifest::new(root);

    for parser in &parsers {
        match parser.parse(root) {
            Ok(m) => merged.merge(m),
            Err(e) => {
                // Log but continue — other parsers may succeed
                eprintln!(
                    "[coregraph-manifest] parser '{}' failed: {}",
                    parser.name(),
                    e
                );
            }
        }
    }

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures")
    }

    #[test]
    fn parse_project_cargo_fixture() {
        let path = fixtures_root().join("cargo");
        if !path.exists() {
            return;
        }
        let manifest = parse_project(&path).expect("should not return hard error");
        assert!(
            !manifest.packages.is_empty(),
            "cargo fixture should produce packages"
        );
    }

    #[test]
    fn parse_project_npm_fixture() {
        let path = fixtures_root().join("npm");
        if !path.exists() {
            return;
        }
        let manifest = parse_project(&path).expect("should not return hard error");
        assert!(
            !manifest.packages.is_empty(),
            "npm fixture should produce packages"
        );
    }

    #[test]
    fn parse_project_maven_fixture() {
        let path = fixtures_root().join("maven");
        if !path.exists() {
            return;
        }
        let manifest = parse_project(&path).expect("should not return hard error");
        assert!(!manifest.packages.is_empty());
    }

    #[test]
    fn parse_project_gradle_fixture() {
        let path = fixtures_root().join("gradle");
        if !path.exists() {
            return;
        }
        let manifest = parse_project(&path).expect("should not return hard error");
        assert!(!manifest.packages.is_empty());
    }

    #[test]
    fn parse_project_go_fixture() {
        let path = fixtures_root().join("go");
        if !path.exists() {
            return;
        }
        let manifest = parse_project(&path).expect("should not return hard error");
        assert!(!manifest.packages.is_empty());
    }

    #[test]
    fn parse_project_python_fixture() {
        let path = fixtures_root().join("python");
        if !path.exists() {
            return;
        }
        let manifest = parse_project(&path).expect("should not return hard error");
        assert!(!manifest.packages.is_empty());
    }
}
