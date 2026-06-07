use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::types::{DependsOn, ExternalPackage, Language, Package, PackageKind, ProjectManifest};
use crate::ManifestParser;

pub struct MavenParser;

/// Maven packaging signals: the `spring-boot-maven-plugin` repackages into an
/// executable app; `<packaging>war</packaging>` is a deployable web app;
/// `<packaging>pom</packaging>` is an aggregator/BOM (no API of its own); the
/// default `jar` is a consumable library.
fn maven_kind(content: &str) -> PackageKind {
    if content.contains("spring-boot-maven-plugin") {
        return PackageKind::Application;
    }
    match extract_xml_tag(content, "packaging").as_deref() {
        Some("war") => PackageKind::Application,
        Some("pom") => PackageKind::Unknown,
        Some("jar") | None => PackageKind::Library,
        Some(_) => PackageKind::Unknown,
    }
}

impl ManifestParser for MavenParser {
    fn name(&self) -> &'static str {
        "maven"
    }

    fn can_parse(&self, root: &Path) -> bool {
        root.join("pom.xml").exists()
    }

    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError> {
        let pom_path = root.join("pom.xml");
        let content =
            std::fs::read_to_string(&pom_path).map_err(|e| ManifestError::io(&pom_path, e))?;

        let mut manifest = ProjectManifest::new(root);

        let artifact_id =
            extract_xml_tag(&content, "artifactId").unwrap_or_else(|| "unnamed".to_string());
        let version = extract_xml_tag(&content, "version");

        manifest.packages.push(Package {
            name: artifact_id.clone(),
            version,
            path: PathBuf::from("."),
            language: Language::Java,
            kind: maven_kind(&content),
        });

        // Parse <dependencies> block
        if let Some(deps_block) = extract_block(&content, "dependencies") {
            parse_maven_dependencies(&artifact_id, &deps_block, &mut manifest);
        }

        // Parse submodules from <modules>
        if let Some(modules_block) = extract_block(&content, "modules") {
            for module_name in extract_all_tag_values(&modules_block, "module") {
                let sub_path = root.join(&module_name);
                if sub_path.join("pom.xml").exists() {
                    if let Ok(sub_manifest) = MavenParser.parse(&sub_path) {
                        manifest.merge(sub_manifest);
                    }
                }
            }
        }

        Ok(manifest)
    }
}

fn extract_xml_tag(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)?;
    Some(content[start..start + end].trim().to_string())
}

fn extract_all_tag_values(content: &str, tag: &str) -> Vec<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let mut result = Vec::new();
    let mut rest = content;
    while let Some(start_pos) = rest.find(&open) {
        let after_open = &rest[start_pos + open.len()..];
        if let Some(end_pos) = after_open.find(&close) {
            result.push(after_open[..end_pos].trim().to_string());
            rest = &after_open[end_pos + close.len()..];
        } else {
            break;
        }
    }
    result
}

fn extract_block(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)?;
    Some(content[start..start + end].to_string())
}

fn parse_maven_dependencies(from: &str, deps_block: &str, manifest: &mut ProjectManifest) {
    // Split on <dependency> boundaries
    let mut rest = deps_block;
    while let Some(dep_start) = rest.find("<dependency>") {
        let after = &rest[dep_start + "<dependency>".len()..];
        let dep_end = after.find("</dependency>").unwrap_or(after.len());
        let dep_content = &after[..dep_end];

        let group_id = extract_xml_tag(dep_content, "groupId").unwrap_or_default();
        let artifact_id = extract_xml_tag(dep_content, "artifactId").unwrap_or_default();
        let version = extract_xml_tag(dep_content, "version").unwrap_or_else(|| "*".to_string());
        let scope = extract_xml_tag(dep_content, "scope").unwrap_or_default();
        let optional = extract_xml_tag(dep_content, "optional")
            .map(|v| v == "true")
            .unwrap_or(false);

        if !group_id.is_empty() && !artifact_id.is_empty() {
            let dep_name = format!("{}:{}", group_id, artifact_id);
            let dev_only = scope == "test";

            if !manifest.external_deps.iter().any(|e| e.name == dep_name) {
                manifest.external_deps.push(ExternalPackage {
                    name: dep_name.clone(),
                    version_req: version,
                    registry: Some("maven_central".to_string()),
                    resolved_version: None,
                });
            }
            manifest.edges.push(DependsOn {
                from: from.to_string(),
                to: dep_name,
                dev_only,
                optional,
            });
        }

        rest = &after[dep_end..];
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
            .join("tests/fixtures/maven")
    }

    #[test]
    fn maven_can_parse() {
        let path = fixture_path();
        if path.exists() {
            assert!(MavenParser.can_parse(&path));
        }
    }

    #[test]
    fn maven_parse_fixture() {
        let path = fixture_path();
        if !path.exists() {
            return;
        }
        let result = MavenParser.parse(&path).expect("parse should succeed");
        assert!(!result.packages.is_empty());
        assert_eq!(result.packages[0].language, Language::Java);
    }
}
