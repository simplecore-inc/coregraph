//! Node-level context annotations (docs/cli.md §11.4):
//! - package detection from path (crate in Rust, module in Go/Python, Gradle
//!   submodule in Java, npm workspace member in JS/TS)
//! - GENERATED code markers (file path patterns or well-known headers)
//! - mediator file references (for Configures edges)

use coregraph_core::SymbolNode;
use serde_json::{json, Value};
use std::path::Path;

/// Returns a JSON object describing a node's contextual information.
pub fn node_context(n: &SymbolNode) -> Value {
    json!({
        "package": package_of(&n.file),
        "generated": is_generated(&n.file),
        "generator": detect_generator(&n.file),
    })
}

/// Extract the enclosing package name from a file path, using common layout
/// conventions. Best-effort; returns "unknown" when no convention matches.
pub fn package_of(file: &Path) -> String {
    let s = file.to_string_lossy();
    // Rust workspace: .../crates/<name>/src/... → "<name> (cargo)"
    if let Some(idx) = s.find("/crates/") {
        let after = &s[idx + "/crates/".len()..];
        if let Some(slash) = after.find('/') {
            return format!("{} (cargo)", &after[..slash]);
        }
    }
    // npm workspace: .../packages/<name>/... or workspace root package.json
    if let Some(idx) = s.find("/packages/") {
        let after = &s[idx + "/packages/".len()..];
        if let Some(slash) = after.find('/') {
            return format!("{} (npm)", &after[..slash]);
        }
    }
    // Gradle subproject: .../<module>/src/main/java/...
    if let Some(idx) = s.find("/src/main/") {
        let before = &s[..idx];
        if let Some(slash) = before.rfind('/') {
            return format!("{} (gradle)", &before[slash + 1..]);
        }
    }
    // Go: .../<module>/go.mod boundary, approximate via top-level dir
    if s.ends_with(".go") {
        if let Some(slash) = s.rfind('/') {
            let dir = &s[..slash];
            if let Some(ds) = dir.rfind('/') {
                return format!("{} (go)", &dir[ds + 1..]);
            }
        }
    }
    "unknown".to_string()
}

/// Heuristic for §11.4 "⚠ GENERATED" marker.
pub fn is_generated(file: &Path) -> bool {
    let s = file.to_string_lossy().to_lowercase();
    s.contains("/__generated__/")
        || s.contains("/.gen/")
        || s.contains("/generated/")
        || s.contains("/gen/")
        || s.ends_with(".pb.go")
        || s.ends_with(".g.dart")
        || s.ends_with(".generated.ts")
        || s.ends_with(".generated.js")
        || s.ends_with("_pb.py")
        || s.ends_with(".pb.cc")
        || s.ends_with(".pb.h")
}

/// Guess the generator tool when we can infer it from the path.
pub fn detect_generator(file: &Path) -> Option<String> {
    let s = file.to_string_lossy().to_lowercase();
    if s.ends_with(".pb.go")
        || s.ends_with(".pb.cc")
        || s.ends_with(".pb.h")
        || s.ends_with("_pb.py")
    {
        return Some("protoc".into());
    }
    if s.contains("/__generated__/") && s.contains("openapi") {
        return Some("openapi-codegen".into());
    }
    if s.ends_with(".g.dart") {
        return Some("build_runner".into());
    }
    if s.ends_with(".generated.ts") || s.ends_with(".generated.js") {
        return Some("codegen".into());
    }
    if s.contains("/target/") || s.contains("/build/") {
        return Some("build".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_crate_package() {
        let p = Path::new("/proj/crates/graph/src/lib.rs");
        assert_eq!(package_of(p), "graph (cargo)");
    }

    #[test]
    fn npm_package() {
        let p = Path::new("/proj/packages/frontend/src/App.tsx");
        assert_eq!(package_of(p), "frontend (npm)");
    }

    #[test]
    fn gradle_package() {
        let p = Path::new("/proj/backend/src/main/java/Foo.java");
        assert_eq!(package_of(p), "backend (gradle)");
    }

    #[test]
    fn detects_protoc_generated() {
        assert!(is_generated(Path::new("gen/user.pb.go")));
        assert_eq!(
            detect_generator(Path::new("gen/user.pb.go")),
            Some("protoc".into())
        );
    }

    #[test]
    fn detects_openapi_generated() {
        assert!(is_generated(Path::new("src/__generated__/openapi-api.ts")));
        assert_eq!(
            detect_generator(Path::new("src/__generated__/openapi-api.ts")),
            Some("openapi-codegen".into())
        );
    }

    #[test]
    fn plain_source_is_not_generated() {
        assert!(!is_generated(Path::new("src/main.rs")));
    }
}
