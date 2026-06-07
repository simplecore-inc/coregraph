use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::types::{DependsOn, ExternalPackage, Language, Package, PackageKind, ProjectManifest};
use crate::ManifestParser;

pub struct PythonParser;

/// A Python project that ships packaging metadata (`setup.py`/`setup.cfg`, or a
/// `pyproject.toml` declaring `[project]`/`[tool.poetry]`) is distributable as a
/// library; a project carrying only a bare `requirements.txt`/`Pipfile` (no
/// packaging) is treated as an application.
fn python_kind(root: &Path) -> PackageKind {
    if root.join("setup.py").exists() || root.join("setup.cfg").exists() {
        return PackageKind::Library;
    }
    let pyproject = root.join("pyproject.toml");
    if pyproject.exists() {
        if let Ok(content) = std::fs::read_to_string(&pyproject) {
            if let Ok(v) = toml::from_str::<toml::Value>(&content) {
                let declares_pkg = v.get("project").and_then(|p| p.get("name")).is_some()
                    || v.get("tool").and_then(|t| t.get("poetry")).is_some();
                if declares_pkg {
                    return PackageKind::Library;
                }
            }
        }
    }
    PackageKind::Application
}

impl ManifestParser for PythonParser {
    fn name(&self) -> &'static str {
        "python"
    }

    fn can_parse(&self, root: &Path) -> bool {
        root.join("requirements.txt").exists()
            || root.join("pyproject.toml").exists()
            || root.join("setup.py").exists()
            || root.join("setup.cfg").exists()
            || root.join("Pipfile").exists()
    }

    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError> {
        let mut manifest = ProjectManifest::new(root);

        // Determine project name from pyproject.toml or setup.py
        let project_name = read_python_project_name(root).unwrap_or_else(|| {
            root.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unnamed")
                .to_string()
        });

        manifest.packages.push(Package {
            name: project_name.clone(),
            version: None,
            path: PathBuf::from("."),
            language: Language::Python,
            kind: python_kind(root),
        });

        // Parse pyproject.toml (Poetry and PEP 517 style)
        let pyproject_path = root.join("pyproject.toml");
        if pyproject_path.exists() {
            parse_pyproject(&pyproject_path, &project_name, &mut manifest);
        }

        // Parse requirements.txt
        let req_path = root.join("requirements.txt");
        if req_path.exists() {
            let content =
                std::fs::read_to_string(&req_path).map_err(|e| ManifestError::io(&req_path, e))?;
            parse_requirements(&content, &project_name, false, &mut manifest);
        }

        // Parse requirements-dev.txt / requirements_dev.txt
        for dev_req in &[
            "requirements-dev.txt",
            "requirements_dev.txt",
            "requirements-test.txt",
        ] {
            let dev_path = root.join(dev_req);
            if dev_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&dev_path) {
                    parse_requirements(&content, &project_name, true, &mut manifest);
                }
            }
        }

        Ok(manifest)
    }
}

fn read_python_project_name(root: &Path) -> Option<String> {
    let pyproject_path = root.join("pyproject.toml");
    if pyproject_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&pyproject_path) {
            if let Ok(toml_val) = toml::from_str::<toml::Value>(&content) {
                // PEP 517: [project] name
                if let Some(name) = toml_val
                    .get("project")
                    .and_then(|p| p.get("name"))
                    .and_then(|n| n.as_str())
                {
                    return Some(name.to_string());
                }
                // Poetry: [tool.poetry] name
                if let Some(name) = toml_val
                    .get("tool")
                    .and_then(|t| t.get("poetry"))
                    .and_then(|p| p.get("name"))
                    .and_then(|n| n.as_str())
                {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn parse_pyproject(path: &Path, from: &str, manifest: &mut ProjectManifest) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let toml_val: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    // PEP 517 [project.dependencies]
    if let Some(deps) = toml_val
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for dep in deps {
            if let Some(dep_str) = dep.as_str() {
                add_python_dep(dep_str, from, false, manifest);
            }
        }
    }

    // Poetry [tool.poetry.dependencies]
    if let Some(deps) = toml_val
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for (name, version_val) in deps {
            if name == "python" {
                continue;
            }
            let version = version_val.as_str().unwrap_or("*").to_string();
            if !manifest.external_deps.iter().any(|e| e.name == *name) {
                manifest.external_deps.push(ExternalPackage {
                    name: name.clone(),
                    version_req: version,
                    registry: Some("pypi".to_string()),
                    resolved_version: None,
                });
            }
            manifest.edges.push(DependsOn {
                from: from.to_string(),
                to: name.clone(),
                dev_only: false,
                optional: false,
            });
        }
    }

    // Poetry dev dependencies
    if let Some(deps) = toml_val
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dev-dependencies"))
        .and_then(|d| d.as_table())
    {
        for (name, _) in deps {
            if !manifest.external_deps.iter().any(|e| e.name == *name) {
                manifest.external_deps.push(ExternalPackage {
                    name: name.clone(),
                    version_req: "*".to_string(),
                    registry: Some("pypi".to_string()),
                    resolved_version: None,
                });
            }
            manifest.edges.push(DependsOn {
                from: from.to_string(),
                to: name.clone(),
                dev_only: true,
                optional: false,
            });
        }
    }
}

fn parse_requirements(content: &str, from: &str, dev_only: bool, manifest: &mut ProjectManifest) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        add_python_dep(line, from, dev_only, manifest);
    }
}

fn add_python_dep(dep_str: &str, from: &str, dev_only: bool, manifest: &mut ProjectManifest) {
    // "requests>=2.28.0" or "flask==2.0.0" or "django"
    let name: String = dep_str
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    if name.is_empty() {
        return;
    }
    let version_req: String = dep_str[name.len()..].trim().to_string();
    let version_req = if version_req.is_empty() {
        "*".to_string()
    } else {
        version_req
    };

    if !manifest.external_deps.iter().any(|e| e.name == name) {
        manifest.external_deps.push(ExternalPackage {
            name: name.clone(),
            version_req,
            registry: Some("pypi".to_string()),
            resolved_version: None,
        });
    }
    manifest.edges.push(DependsOn {
        from: from.to_string(),
        to: name,
        dev_only,
        optional: false,
    });
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
            .join("tests/fixtures/python")
    }

    #[test]
    fn python_can_parse() {
        let path = fixture_path();
        if path.exists() {
            assert!(PythonParser.can_parse(&path));
        }
    }

    #[test]
    fn python_parse_fixture() {
        let path = fixture_path();
        if !path.exists() {
            return;
        }
        let result = PythonParser.parse(&path).expect("parse should succeed");
        assert!(!result.packages.is_empty());
        assert_eq!(result.packages[0].language, Language::Python);
    }
}
