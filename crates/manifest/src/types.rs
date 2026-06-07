use std::path::PathBuf;

/// Programming language associated with a package.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Language {
    Rust,
    JavaScript,
    TypeScript,
    Java,
    Kotlin,
    Go,
    Python,
    Unknown,
}

/// Whether a package exposes a public API consumed by downstream code
/// (`Library`) or is an end product with no importable surface (`Application`).
/// Derived from ecosystem-standard manifest signals (Cargo `[lib]`, npm
/// `exports`/`private`, Maven/Gradle packaging, …). `Unknown` when the manifest
/// gives no decisive signal — consumers then make no library-based adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PackageKind {
    Library,
    Application,
    #[default]
    Unknown,
}

/// An internal package / module within the project.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Package {
    pub name: String,
    pub version: Option<String>,
    /// Path to the package root, relative to the project root.
    pub path: PathBuf,
    pub language: Language,
    /// Library vs application, from manifest signals. `#[serde(default)]` so
    /// snapshots written before this field deserialize as `Unknown`.
    #[serde(default)]
    pub kind: PackageKind,
}

/// An external dependency declared in the manifest.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalPackage {
    pub name: String,
    /// Version requirement string (semver range or exact).
    pub version_req: String,
    /// Resolved exact version from lockfile, if available.
    pub resolved_version: Option<String>,
    /// Registry or source (e.g. "npmjs", "crates.io", "pypi").
    pub registry: Option<String>,
}

/// A dependency edge between a package and an external dependency (or another package).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DependsOn {
    /// `Package.name` of the dependent.
    pub from: String,
    /// `ExternalPackage.name` or `Package.name` of the dependency.
    pub to: String,
    /// True for dev/test-only dependencies.
    pub dev_only: bool,
    /// True if the dependency is optional.
    pub optional: bool,
}

/// Full result of parsing a project manifest.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ProjectManifest {
    /// Project root (absolute path).
    pub root: PathBuf,
    pub packages: Vec<Package>,
    pub external_deps: Vec<ExternalPackage>,
    pub edges: Vec<DependsOn>,
}

impl ProjectManifest {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            ..Default::default()
        }
    }

    pub fn merge(&mut self, other: ProjectManifest) {
        self.packages.extend(other.packages);
        self.external_deps.extend(other.external_deps);
        self.edges.extend(other.edges);
    }
}
