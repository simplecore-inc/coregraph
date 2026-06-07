use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::types::{Language, Package, PackageKind, ProjectManifest};
use crate::ManifestParser;

/// Detects Vite or Webpack projects via config file presence and package.json devDeps heuristics.
/// Note: we do NOT eval JavaScript — we only parse heuristic markers.
pub struct ViteParser;

impl ManifestParser for ViteParser {
    fn name(&self) -> &'static str {
        "vite/webpack"
    }

    fn can_parse(&self, root: &Path) -> bool {
        root.join("vite.config.ts").exists()
            || root.join("vite.config.js").exists()
            || root.join("vite.config.mjs").exists()
            || root.join("webpack.config.js").exists()
            || root.join("webpack.config.ts").exists()
            || has_vite_or_webpack_in_package_json(root)
    }

    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError> {
        let mut manifest = ProjectManifest::new(root);

        // Determine bundler kind
        let bundler = if root.join("vite.config.ts").exists()
            || root.join("vite.config.js").exists()
            || root.join("vite.config.mjs").exists()
        {
            "vite"
        } else {
            "webpack"
        };

        let pkg_name = read_package_json_name(root).unwrap_or_else(|| {
            root.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unnamed")
                .to_string()
        });

        manifest.packages.push(Package {
            name: format!("{} ({})", pkg_name, bundler),
            version: None,
            path: PathBuf::from("."),
            language: Language::TypeScript,
            // A bundler config alone does not decide library vs app (both modes
            // exist) and the npm parser already classifies the real package.json
            // package, so the synthetic bundler package stays Unknown.
            kind: PackageKind::Unknown,
        });

        Ok(manifest)
    }
}

fn has_vite_or_webpack_in_package_json(root: &Path) -> bool {
    let pkg_path = root.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_path) {
        if let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&content) {
            let dev_deps = &pkg["devDependencies"];
            if let Some(map) = dev_deps.as_object() {
                return map.contains_key("vite") || map.contains_key("webpack");
            }
        }
    }
    false
}

fn read_package_json_name(root: &Path) -> Option<String> {
    let pkg_path = root.join("package.json");
    let content = std::fs::read_to_string(&pkg_path).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&content).ok()?;
    pkg["name"].as_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/vite")
    }

    #[test]
    fn vite_can_parse() {
        let path = fixture_path();
        if path.exists() {
            assert!(ViteParser.can_parse(&path));
        }
    }
}
