use std::path::Path;

use crate::{
    cargo_parser::CargoParser, go::GoParser, gradle::GradleParser, maven::MavenParser,
    npm::NpmParser, python::PythonParser, vite::ViteParser, ManifestParser,
};

/// Probes a project root and returns all applicable parsers.
pub struct ProjectDetector;

impl ProjectDetector {
    /// Detect all build systems present in `root` and return matching parsers.
    /// The order is deterministic: higher-specificity detectors first.
    pub fn detect(root: &Path) -> Vec<Box<dyn ManifestParser>> {
        let all: Vec<Box<dyn ManifestParser>> = vec![
            Box::new(CargoParser),
            Box::new(MavenParser),
            Box::new(GradleParser),
            Box::new(GoParser),
            Box::new(PythonParser),
            // ViteParser runs after NpmParser since Vite projects also have package.json
            Box::new(NpmParser),
            Box::new(ViteParser),
        ];

        all.into_iter().filter(|p| p.can_parse(root)).collect()
    }
}

#[cfg(test)]
mod tests {
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
    fn detect_cargo_fixture() {
        let path = fixtures_root().join("cargo");
        if !path.exists() {
            return;
        }
        let parsers = ProjectDetector::detect(&path);
        assert!(!parsers.is_empty());
        assert!(parsers.iter().any(|p| p.name() == "cargo"));
    }

    #[test]
    fn detect_npm_fixture() {
        let path = fixtures_root().join("npm");
        if !path.exists() {
            return;
        }
        let parsers = ProjectDetector::detect(&path);
        assert!(parsers.iter().any(|p| p.name() == "npm/pnpm/yarn"));
    }

    #[test]
    fn detect_maven_fixture() {
        let path = fixtures_root().join("maven");
        if !path.exists() {
            return;
        }
        let parsers = ProjectDetector::detect(&path);
        assert!(parsers.iter().any(|p| p.name() == "maven"));
    }

    #[test]
    fn detect_gradle_fixture() {
        let path = fixtures_root().join("gradle");
        if !path.exists() {
            return;
        }
        let parsers = ProjectDetector::detect(&path);
        assert!(parsers.iter().any(|p| p.name() == "gradle"));
    }

    #[test]
    fn detect_go_fixture() {
        let path = fixtures_root().join("go");
        if !path.exists() {
            return;
        }
        let parsers = ProjectDetector::detect(&path);
        assert!(parsers.iter().any(|p| p.name() == "go"));
    }

    #[test]
    fn detect_python_fixture() {
        let path = fixtures_root().join("python");
        if !path.exists() {
            return;
        }
        let parsers = ProjectDetector::detect(&path);
        assert!(parsers.iter().any(|p| p.name() == "python"));
    }
}
