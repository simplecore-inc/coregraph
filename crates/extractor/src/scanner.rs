use std::path::Path;

/// Check if a file path ends with one of the given extensions.
///
/// Extensions should be specified without the leading dot, e.g. `["java", "kt"]`.
pub fn extension_matches(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| exts.contains(&ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_java_extensions() {
        let path = Path::new("src/Main.java");
        assert!(extension_matches(path, &["java", "kt"]));
    }

    #[test]
    fn no_extension_returns_false() {
        let path = Path::new("Makefile");
        assert!(!extension_matches(path, &["java", "kt"]));
    }

    #[test]
    fn non_matching_extension() {
        let path = Path::new("script.py");
        assert!(!extension_matches(path, &["java", "kt"]));
    }

    #[test]
    fn match_systems_extensions() {
        assert!(extension_matches(Path::new("main.go"), &["go"]));
        assert!(extension_matches(Path::new("lib.rs"), &["rs"]));
        assert!(extension_matches(
            Path::new("config.yaml"),
            &["yaml", "yml"]
        ));
        assert!(extension_matches(Path::new("config.yml"), &["yaml", "yml"]));
        assert!(extension_matches(Path::new("app.toml"), &["toml"]));
        assert!(extension_matches(Path::new("data.json"), &["json"]));
    }
}
