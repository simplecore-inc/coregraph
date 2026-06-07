use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::types::{DependsOn, ExternalPackage, Language, Package, PackageKind, ProjectManifest};
use crate::ManifestParser;

pub struct NpmParser;

/// Library vs application from npm-standard `package.json` fields.
///
/// A consumer-facing API surface (`exports`/`module`/`types`/`typings`) marks a
/// library even when `private: true` — many library repos set `private` because
/// they publish a built `dist/` rather than the repo root (e.g. zustand). With
/// no such surface, `private: true` (or a `bin`-only CLI) is an application; a
/// public package that only declares `main` is a (classic) library.
fn npm_kind(pkg: &serde_json::Value) -> PackageKind {
    let has = |k: &str| pkg.get(k).is_some_and(|v| !v.is_null());
    if has("exports") || has("module") || has("types") || has("typings") {
        return PackageKind::Library;
    }
    if pkg.get("private").and_then(|v| v.as_bool()) == Some(true) {
        return PackageKind::Application;
    }
    if has("bin") {
        return PackageKind::Application;
    }
    if has("main") {
        return PackageKind::Library;
    }
    PackageKind::Unknown
}

impl ManifestParser for NpmParser {
    fn name(&self) -> &'static str {
        "npm/pnpm/yarn"
    }

    fn can_parse(&self, root: &Path) -> bool {
        root.join("package.json").exists()
    }

    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError> {
        let pkg_path = root.join("package.json");
        let content =
            std::fs::read_to_string(&pkg_path).map_err(|e| ManifestError::io(&pkg_path, e))?;

        let pkg: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| ManifestError::json(&pkg_path, e))?;

        let mut manifest = ProjectManifest::new(root);

        let pkg_name = pkg["name"].as_str().unwrap_or("unnamed").to_string();
        let pkg_version = pkg["version"].as_str().map(String::from);

        manifest.packages.push(Package {
            name: pkg_name.clone(),
            version: pkg_version,
            path: PathBuf::from("."),
            language: Language::JavaScript,
            kind: npm_kind(&pkg),
        });

        // Parse dependencies
        Self::extract_deps(&pkg_name, &pkg["dependencies"], false, &mut manifest);
        Self::extract_deps(&pkg_name, &pkg["devDependencies"], true, &mut manifest);
        Self::extract_deps(&pkg_name, &pkg["peerDependencies"], false, &mut manifest);
        Self::extract_deps(
            &pkg_name,
            &pkg["optionalDependencies"],
            false,
            &mut manifest,
        );

        // Check for workspaces (monorepo). npm/yarn declare members in the
        // `workspaces` field of package.json; pnpm declares them in a separate
        // `pnpm-workspace.yaml`. Both are honored so member packages (and their
        // library-vs-application kind) are discovered either way.
        if let Some(workspaces) = pkg["workspaces"].as_array() {
            for ws in workspaces {
                if let Some(pattern) = ws.as_str() {
                    self.parse_workspace_packages(root, pattern, &mut manifest);
                }
            }
        } else if let Some(workspaces) = pkg["workspaces"]["packages"].as_array() {
            for ws in workspaces {
                if let Some(pattern) = ws.as_str() {
                    self.parse_workspace_packages(root, pattern, &mut manifest);
                }
            }
        }
        self.parse_pnpm_workspace(root, &mut manifest);

        Ok(manifest)
    }
}

impl NpmParser {
    fn extract_deps(
        from: &str,
        deps_node: &serde_json::Value,
        dev_only: bool,
        manifest: &mut ProjectManifest,
    ) {
        let map: HashMap<String, serde_json::Value> =
            match serde_json::from_value(deps_node.clone()) {
                Ok(m) => m,
                Err(_) => return,
            };

        for (name, version_val) in &map {
            let version_req = version_val.as_str().unwrap_or("*").to_string();
            if !manifest.external_deps.iter().any(|e| &e.name == name) {
                manifest.external_deps.push(ExternalPackage {
                    name: name.clone(),
                    version_req,
                    registry: Some("npmjs".to_string()),
                    resolved_version: None,
                });
            }
            manifest.edges.push(DependsOn {
                from: from.to_string(),
                to: name.clone(),
                dev_only,
                optional: false,
            });
        }
    }

    /// Discover pnpm-workspace members listed under `packages:` in
    /// `pnpm-workspace.yaml`. Each glob is resolved by the same lister used for
    /// npm/yarn `workspaces`. Negation globs (`!pattern`) are exclusions, not
    /// member locations, so they are skipped. A missing or malformed file is a
    /// no-op (the project is simply treated as having no pnpm workspace).
    fn parse_pnpm_workspace(&self, root: &Path, manifest: &mut ProjectManifest) {
        let ws_path = root.join("pnpm-workspace.yaml");
        let Ok(content) = std::fs::read_to_string(&ws_path) else {
            return;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
            return;
        };
        let Some(patterns) = doc.get("packages").and_then(|p| p.as_sequence()) else {
            return;
        };
        for pattern in patterns {
            if let Some(glob) = pattern.as_str() {
                if glob.starts_with('!') {
                    continue;
                }
                self.parse_workspace_packages(root, glob, manifest);
            }
        }
    }

    fn parse_workspace_packages(&self, root: &Path, pattern: &str, manifest: &mut ProjectManifest) {
        // Simple glob: replace trailing * with directory listing
        let pattern_path = if pattern.ends_with("/*") {
            let dir = pattern.trim_end_matches("/*");
            let dir_path = root.join(dir);
            if dir_path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&dir_path) {
                    for entry in entries.flatten() {
                        let sub_path = entry.path();
                        if sub_path.is_dir() {
                            let pkg_json = sub_path.join("package.json");
                            if pkg_json.exists() {
                                let _ = self.parse_sub_package(root, &sub_path, manifest);
                            }
                        }
                    }
                }
            }
            return;
        } else {
            root.join(pattern)
        };

        if pattern_path.join("package.json").exists() {
            let _ = self.parse_sub_package(root, &pattern_path, manifest);
        }
    }

    fn parse_sub_package(
        &self,
        root: &Path,
        sub_path: &Path,
        manifest: &mut ProjectManifest,
    ) -> Result<(), ManifestError> {
        let pkg_json = sub_path.join("package.json");
        let content =
            std::fs::read_to_string(&pkg_json).map_err(|e| ManifestError::io(&pkg_json, e))?;
        let pkg: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| ManifestError::json(&pkg_json, e))?;

        let pkg_name = pkg["name"].as_str().unwrap_or("unnamed").to_string();
        let pkg_version = pkg["version"].as_str().map(String::from);
        let rel_path = sub_path
            .strip_prefix(root)
            .unwrap_or(sub_path)
            .to_path_buf();

        if !manifest.packages.iter().any(|p| p.name == pkg_name) {
            manifest.packages.push(Package {
                name: pkg_name.clone(),
                version: pkg_version,
                path: rel_path,
                language: Language::JavaScript,
                kind: npm_kind(&pkg),
            });
        }

        Self::extract_deps(&pkg_name, &pkg["dependencies"], false, manifest);
        Self::extract_deps(&pkg_name, &pkg["devDependencies"], true, manifest);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pnpm_workspace_members_are_discovered_with_kinds() {
        // A pnpm monorepo declares its members in `pnpm-workspace.yaml`, not in a
        // `workspaces` field in package.json. Without reading that file, the
        // library-vs-application classifier never sees the member packages, so
        // every published export looks like dead code. Each member's kind must
        // come from its own package.json (a published library with `exports`, a
        // private app), and a nested glob (`extensions/.../packages/*`) must work.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"monorepo-root","private":true}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - packages/*\n  - apps/*\n  - extensions/boot/packages/*\n",
        )
        .unwrap();
        // Published library member (declares an `exports` surface).
        std::fs::create_dir_all(root.join("packages/contract")).unwrap();
        std::fs::write(
            root.join("packages/contract/package.json"),
            r#"{"name":"@x/contract","version":"1.0.0","exports":{".":"./src/index.ts"}}"#,
        )
        .unwrap();
        // Private application member.
        std::fs::create_dir_all(root.join("apps/web")).unwrap();
        std::fs::write(
            root.join("apps/web/package.json"),
            r#"{"name":"@x/web","private":true,"main":"src/main.tsx"}"#,
        )
        .unwrap();
        // Nested-glob member (extension subpackage).
        std::fs::create_dir_all(root.join("extensions/boot/packages/auth")).unwrap();
        std::fs::write(
            root.join("extensions/boot/packages/auth/package.json"),
            r#"{"name":"@x/boot-auth","version":"1.0.0","module":"./dist/index.js"}"#,
        )
        .unwrap();

        let m = NpmParser.parse(root).expect("parse should succeed");
        let kind_of = |name: &str| m.packages.iter().find(|p| p.name == name).map(|p| p.kind);
        assert_eq!(
            kind_of("@x/contract"),
            Some(PackageKind::Library),
            "library member declared in pnpm-workspace.yaml must be discovered as a library"
        );
        assert_eq!(
            kind_of("@x/web"),
            Some(PackageKind::Application),
            "private app member must be discovered as an application"
        );
        assert_eq!(
            kind_of("@x/boot-auth"),
            Some(PackageKind::Library),
            "nested-glob member must be discovered"
        );
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/npm")
    }

    #[test]
    fn npm_kind_from_standard_signals() {
        let lib = serde_json::json!({"name": "zustand", "exports": {".": "./index.js"}});
        assert_eq!(npm_kind(&lib), PackageKind::Library, "exports → library");
        let lib2 = serde_json::json!({"name": "x", "main": "dist/x.js", "module": "dist/x.mjs"});
        assert_eq!(
            npm_kind(&lib2),
            PackageKind::Library,
            "main/module → library"
        );
        let app =
            serde_json::json!({"name": "excalidraw-app", "private": true, "main": "src/index.tsx"});
        assert_eq!(
            npm_kind(&app),
            PackageKind::Application,
            "private + only main → app"
        );
        // zustand pattern: private repo root that still declares a consumer API
        // surface (publishes a built dist) → library.
        let dist_lib = serde_json::json!({"name": "zustand", "private": true, "main": "./index.js", "exports": {".": "./index.js"}, "types": "./index.d.ts"});
        assert_eq!(
            npm_kind(&dist_lib),
            PackageKind::Library,
            "private + exports/types → library"
        );
        let cli = serde_json::json!({"name": "tool", "bin": {"tool": "cli.js"}});
        assert_eq!(
            npm_kind(&cli),
            PackageKind::Application,
            "bin-only → application"
        );
        let unknown = serde_json::json!({"name": "bare"});
        assert_eq!(
            npm_kind(&unknown),
            PackageKind::Unknown,
            "no signal → unknown"
        );
    }

    #[test]
    fn npm_can_parse() {
        let path = fixture_path();
        if path.exists() {
            assert!(NpmParser.can_parse(&path));
        }
    }

    #[test]
    fn npm_parse_basic() {
        let path = fixture_path();
        if !path.exists() {
            return;
        }
        let result = NpmParser.parse(&path).expect("parse should succeed");
        assert!(
            !result.packages.is_empty(),
            "should have at least one package"
        );
        assert_eq!(result.packages[0].language, Language::JavaScript);
    }
}
