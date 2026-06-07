use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::types::{DependsOn, ExternalPackage, Language, Package, PackageKind, ProjectManifest};
use crate::ManifestParser;

pub struct GoParser;

impl ManifestParser for GoParser {
    fn name(&self) -> &'static str {
        "go"
    }

    fn can_parse(&self, root: &Path) -> bool {
        root.join("go.mod").exists()
    }

    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError> {
        let go_mod_path = root.join("go.mod");
        let content = std::fs::read_to_string(&go_mod_path)
            .map_err(|e| ManifestError::io(&go_mod_path, e))?;

        let mut manifest = ProjectManifest::new(root);
        let mut module_name = String::from("unnamed");
        let mut in_require_block = false;

        for line in content.lines() {
            let line = line.trim();

            if let Some(rest) = line.strip_prefix("module ") {
                module_name = rest.trim().to_string();
                continue;
            }

            if line == "require (" {
                in_require_block = true;
                continue;
            }
            if line == ")" && in_require_block {
                in_require_block = false;
                continue;
            }

            // Single-line require
            if line.starts_with("require ") && !line.ends_with('(') {
                let dep_line = line["require ".len()..].trim();
                parse_go_dep_line(dep_line, &module_name, &mut manifest);
                continue;
            }

            // Inside require block
            if in_require_block && !line.is_empty() && !line.starts_with("//") {
                parse_go_dep_line(line, &module_name, &mut manifest);
            }
        }

        manifest.packages.push(Package {
            name: module_name,
            version: None,
            path: PathBuf::from("."),
            language: Language::Go,
            // A Go module declares an import path and is importable by downstream
            // code, so the module surface is treated as a library. Whether it
            // *also* builds a command (a `package main`) is a per-file property
            // that needs source scanning to distinguish — not done at the
            // manifest level, so a command-only module is not separated here.
            kind: PackageKind::Library,
        });

        Ok(manifest)
    }
}

fn parse_go_dep_line(line: &str, from: &str, manifest: &mut ProjectManifest) {
    // Format: "github.com/foo/bar v1.2.3" or "github.com/foo/bar v1.2.3 // indirect"
    let line = line.split("//").next().unwrap_or(line).trim();
    let parts: Vec<&str> = line.splitn(2, ' ').collect();
    if parts.len() == 2 {
        let dep_name = parts[0].to_string();
        let version = parts[1].trim().to_string();

        if !manifest.external_deps.iter().any(|e| e.name == dep_name) {
            manifest.external_deps.push(ExternalPackage {
                name: dep_name.clone(),
                version_req: version,
                registry: Some("pkg.go.dev".to_string()),
                resolved_version: None,
            });
        }
        manifest.edges.push(DependsOn {
            from: from.to_string(),
            to: dep_name,
            dev_only: false,
            optional: false,
        });
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
            .join("tests/fixtures/go")
    }

    #[test]
    fn go_can_parse() {
        let path = fixture_path();
        if path.exists() {
            assert!(GoParser.can_parse(&path));
        }
    }

    #[test]
    fn go_parse_fixture() {
        let path = fixture_path();
        if !path.exists() {
            return;
        }
        let result = GoParser.parse(&path).expect("parse should succeed");
        assert!(!result.packages.is_empty());
        assert_eq!(result.packages[0].language, Language::Go);
    }
}
