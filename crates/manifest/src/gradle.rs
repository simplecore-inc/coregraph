use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::types::{DependsOn, ExternalPackage, Language, Package, PackageKind, ProjectManifest};
use crate::ManifestParser;

pub struct GradleParser;

/// Gradle plugin signals: the `application`, Spring Boot, or Android-application
/// plugins build a runnable app; a JVM project applying `java`/`java-library`/
/// `kotlin("jvm")`/`maven-publish` (and none of the app plugins) is a consumable
/// library. No decisive plugin → Unknown.
fn gradle_kind(content: &str) -> PackageKind {
    const APP: &[&str] = &[
        "id(\"application\")",
        "id('application')",
        "apply plugin: \"application\"",
        "apply plugin: 'application'",
        "org.springframework.boot",
        "com.android.application",
    ];
    if APP.iter().any(|s| content.contains(s)) {
        return PackageKind::Application;
    }
    const LIB: &[&str] = &[
        "java-library",
        "maven-publish",
        "kotlin(\"jvm\")",
        "kotlin('jvm')",
        "org.jetbrains.kotlin.jvm",
        "id(\"java\")",
        "id('java')",
        "apply plugin: \"java\"",
        "apply plugin: 'java'",
    ];
    if LIB.iter().any(|s| content.contains(s)) {
        return PackageKind::Library;
    }
    PackageKind::Unknown
}

/// Read a directory's Gradle build script (kts preferred) and classify it.
fn gradle_kind_at(dir: &Path) -> PackageKind {
    for f in ["build.gradle.kts", "build.gradle"] {
        if let Ok(content) = std::fs::read_to_string(dir.join(f)) {
            return gradle_kind(&content);
        }
    }
    PackageKind::Unknown
}

// Dependency configuration keywords to detect
const DEP_CONFIGS: &[&str] = &[
    "implementation",
    "api",
    "compile",
    "testImplementation",
    "testCompile",
    "runtimeOnly",
    "compileOnly",
    "annotationProcessor",
    "kapt",
];

impl ManifestParser for GradleParser {
    fn name(&self) -> &'static str {
        "gradle"
    }

    fn can_parse(&self, root: &Path) -> bool {
        root.join("build.gradle").exists()
            || root.join("build.gradle.kts").exists()
            || root.join("settings.gradle").exists()
            || root.join("settings.gradle.kts").exists()
    }

    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError> {
        let mut manifest = ProjectManifest::new(root);

        // Determine project name from settings.gradle
        let project_name = self.read_project_name(root);

        let build_file = if root.join("build.gradle").exists() {
            root.join("build.gradle")
        } else {
            root.join("build.gradle.kts")
        };

        let language = if build_file.extension().and_then(|e| e.to_str()) == Some("kts") {
            Language::Kotlin
        } else {
            Language::Java
        };

        manifest.packages.push(Package {
            name: project_name.clone(),
            version: None,
            path: PathBuf::from("."),
            language,
            kind: gradle_kind_at(root),
        });

        if build_file.exists() {
            let content = std::fs::read_to_string(&build_file)
                .map_err(|e| ManifestError::io(&build_file, e))?;
            self.extract_deps(&project_name, &content, &mut manifest);
        }

        // Parse subprojects from settings.gradle
        self.parse_subprojects(root, &project_name, &mut manifest);

        Ok(manifest)
    }
}

impl GradleParser {
    fn read_project_name(&self, root: &Path) -> String {
        for settings_file in &["settings.gradle", "settings.gradle.kts"] {
            let path = root.join(settings_file);
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    let line = line.trim();
                    // rootProject.name = 'myapp' or rootProject.name = "myapp"
                    if line.starts_with("rootProject.name") {
                        if let Some(name) = extract_quoted_value(line) {
                            return name;
                        }
                    }
                }
            }
        }
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_string()
    }

    fn extract_deps(&self, project_name: &str, content: &str, manifest: &mut ProjectManifest) {
        for line in content.lines() {
            let line = line.trim();

            for &config in DEP_CONFIGS {
                if !line.starts_with(config) {
                    continue;
                }

                let dev_only = config.starts_with("test");
                let rest = &line[config.len()..].trim_start_matches(['(', ' ']);

                // Try "group:artifact:version" notation
                if let Some(dep) = parse_gradle_dep(rest) {
                    let name = format!("{}:{}", dep.group, dep.artifact);
                    if !manifest.external_deps.iter().any(|e| e.name == name) {
                        manifest.external_deps.push(ExternalPackage {
                            name: name.clone(),
                            version_req: dep.version,
                            registry: Some("maven_central".to_string()),
                            resolved_version: None,
                        });
                    }
                    manifest.edges.push(DependsOn {
                        from: project_name.to_string(),
                        to: name,
                        dev_only,
                        optional: false,
                    });
                }
            }
        }
    }

    fn parse_subprojects(&self, root: &Path, _parent_name: &str, manifest: &mut ProjectManifest) {
        for settings_file in &["settings.gradle", "settings.gradle.kts"] {
            let path = root.join(settings_file);
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with("include") {
                        if let Some(name) = extract_quoted_value(line) {
                            // ':subproject' -> 'subproject'
                            let clean = name.trim_start_matches(':').replace(':', "/");
                            let sub_path = root.join(&clean);
                            if sub_path.is_dir()
                                && !manifest.packages.iter().any(|p| p.name == clean)
                            {
                                let kind = gradle_kind_at(&sub_path);
                                manifest.packages.push(Package {
                                    name: clean,
                                    version: None,
                                    path: sub_path
                                        .strip_prefix(root)
                                        .unwrap_or(&sub_path)
                                        .to_path_buf(),
                                    language: Language::Java,
                                    kind,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GradleDep {
    group: String,
    artifact: String,
    version: String,
}

fn parse_gradle_dep(rest: &str) -> Option<GradleDep> {
    // Match "group:artifact:version" inside quotes
    let content = rest
        .trim_start_matches(['\'', '"'])
        .split(['\'', '"'])
        .next()
        .unwrap_or("")
        .trim();

    let parts: Vec<&str> = content.split(':').collect();
    if parts.len() >= 3 {
        return Some(GradleDep {
            group: parts[0].to_string(),
            artifact: parts[1].to_string(),
            version: parts[2].trim_end_matches([' ', ')', '\'']).to_string(),
        });
    }

    // Try group: "...", name: "...", version: "..."
    None
}

fn extract_quoted_value(s: &str) -> Option<String> {
    let start = s.find(['\'', '"'])?;
    let quote_char = s.chars().nth(start)?;
    let rest = &s[start + 1..];
    let end = rest.find(quote_char)?;
    Some(rest[..end].to_string())
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
            .join("tests/fixtures/gradle")
    }

    #[test]
    fn gradle_can_parse() {
        let path = fixture_path();
        if path.exists() {
            assert!(GradleParser.can_parse(&path));
        }
    }

    #[test]
    fn gradle_parse_fixture() {
        let path = fixture_path();
        if !path.exists() {
            return;
        }
        let result = GradleParser.parse(&path).expect("parse should succeed");
        assert!(!result.packages.is_empty());
    }
}
