use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::types::{DependsOn, ExternalPackage, Language, Package, PackageKind, ProjectManifest};
use crate::ManifestParser;

pub struct CargoParser;

/// Cargo target signals: a `[lib]` table or `src/lib.rs` means the crate exposes
/// an API (a library — even if it also ships a binary); a crate with only
/// `[[bin]]`/`src/main.rs` is an application; neither → Unknown.
fn cargo_kind(toml_val: &toml::Value, crate_dir: &Path) -> PackageKind {
    let has_lib = toml_val.get("lib").is_some() || crate_dir.join("src/lib.rs").exists();
    let has_bin = toml_val.get("bin").is_some() || crate_dir.join("src/main.rs").exists();
    match (has_lib, has_bin) {
        (true, _) => PackageKind::Library,
        (false, true) => PackageKind::Application,
        (false, false) => PackageKind::Unknown,
    }
}

impl ManifestParser for CargoParser {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn can_parse(&self, root: &Path) -> bool {
        let cargo_toml = root.join("Cargo.toml");
        if !cargo_toml.exists() {
            return false;
        }
        // Check if it's a workspace or standalone crate
        std::fs::read_to_string(&cargo_toml)
            .map(|c| c.contains("[workspace]") || c.contains("[package]"))
            .unwrap_or(false)
    }

    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError> {
        let cargo_toml_path = root.join("Cargo.toml");
        let content = std::fs::read_to_string(&cargo_toml_path)
            .map_err(|e| ManifestError::io(&cargo_toml_path, e))?;

        let toml_val: toml::Value =
            toml::from_str(&content).map_err(|e| ManifestError::toml(&cargo_toml_path, e))?;

        let mut manifest = ProjectManifest::new(root);

        // Handle workspace
        if let Some(workspace) = toml_val.get("workspace") {
            if let Some(members) = workspace.get("members").and_then(|m| m.as_array()) {
                for member_val in members {
                    if let Some(member_pattern) = member_val.as_str() {
                        self.parse_member_glob(root, member_pattern, &mut manifest)?;
                    }
                }
            }
        }

        // Handle standalone package
        if let Some(package) = toml_val.get("package") {
            let pkg_name = package
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unnamed")
                .to_string();
            let pkg_version = package
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);

            if !manifest.packages.iter().any(|p| p.name == pkg_name) {
                manifest.packages.push(Package {
                    name: pkg_name.clone(),
                    version: pkg_version,
                    path: PathBuf::from("."),
                    language: Language::Rust,
                    kind: cargo_kind(&toml_val, root),
                });
            }

            self.extract_deps(root, &pkg_name, &toml_val, false, &mut manifest);
        }

        Ok(manifest)
    }
}

impl CargoParser {
    fn parse_member_glob(
        &self,
        root: &Path,
        pattern: &str,
        manifest: &mut ProjectManifest,
    ) -> Result<(), ManifestError> {
        if pattern.contains('*') {
            let prefix = pattern.trim_end_matches("/*").trim_end_matches('*');
            let dir = root.join(prefix);
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let sub = entry.path();
                        if sub.is_dir() && sub.join("Cargo.toml").exists() {
                            self.parse_member_crate(root, &sub, manifest)?;
                        }
                    }
                }
            }
        } else {
            let member_path = root.join(pattern);
            if member_path.join("Cargo.toml").exists() {
                self.parse_member_crate(root, &member_path, manifest)?;
            }
        }
        Ok(())
    }

    fn parse_member_crate(
        &self,
        root: &Path,
        crate_path: &Path,
        manifest: &mut ProjectManifest,
    ) -> Result<(), ManifestError> {
        let cargo_toml_path = crate_path.join("Cargo.toml");
        let content = std::fs::read_to_string(&cargo_toml_path)
            .map_err(|e| ManifestError::io(&cargo_toml_path, e))?;

        let toml_val: toml::Value =
            toml::from_str(&content).map_err(|e| ManifestError::toml(&cargo_toml_path, e))?;

        let pkg_name = toml_val
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("unnamed")
            .to_string();

        let pkg_version = toml_val
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let rel_path = crate_path
            .strip_prefix(root)
            .unwrap_or(crate_path)
            .to_path_buf();

        if !manifest.packages.iter().any(|p| p.name == pkg_name) {
            manifest.packages.push(Package {
                name: pkg_name.clone(),
                version: pkg_version,
                path: rel_path,
                language: Language::Rust,
                kind: cargo_kind(&toml_val, crate_path),
            });
        }

        self.extract_deps(root, &pkg_name, &toml_val, false, manifest);
        Ok(())
    }

    fn extract_deps(
        &self,
        _root: &Path,
        pkg_name: &str,
        toml_val: &toml::Value,
        dev_only: bool,
        manifest: &mut ProjectManifest,
    ) {
        let sections = if dev_only {
            vec!["dev-dependencies"]
        } else {
            vec!["dependencies", "build-dependencies"]
        };

        for section in &sections {
            if let Some(deps) = toml_val.get(section).and_then(|d| d.as_table()) {
                let is_dev = *section == "dev-dependencies";
                for (dep_name, dep_val) in deps {
                    let version_req = match dep_val {
                        toml::Value::String(s) => s.clone(),
                        toml::Value::Table(t) => t
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("*")
                            .to_string(),
                        _ => "*".to_string(),
                    };

                    let optional = dep_val
                        .as_table()
                        .and_then(|t| t.get("optional"))
                        .and_then(|o| o.as_bool())
                        .unwrap_or(false);

                    if !manifest.external_deps.iter().any(|e| &e.name == dep_name) {
                        manifest.external_deps.push(ExternalPackage {
                            name: dep_name.clone(),
                            version_req,
                            registry: Some("crates.io".to_string()),
                            resolved_version: None,
                        });
                    }

                    manifest.edges.push(DependsOn {
                        from: pkg_name.to_string(),
                        to: dep_name.clone(),
                        dev_only: is_dev,
                        optional,
                    });
                }
            }
        }

        // Also check dev-dependencies
        if !dev_only {
            self.extract_deps(_root, pkg_name, toml_val, true, manifest);
        }
    }
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
            .join("tests/fixtures/cargo")
    }

    #[test]
    fn cargo_can_parse() {
        let path = fixture_path();
        if path.exists() {
            assert!(CargoParser.can_parse(&path));
        }
    }

    #[test]
    fn cargo_parse_fixture() {
        let path = fixture_path();
        if !path.exists() {
            return;
        }
        let result = CargoParser.parse(&path).expect("parse should succeed");
        assert!(
            !result.packages.is_empty(),
            "should have at least one package"
        );
        assert_eq!(result.packages[0].language, Language::Rust);
    }
}
