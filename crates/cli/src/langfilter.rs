//! Shared language filter used by query, orphans, impact, inconsistencies,
//! and export.

use std::path::Path;

/// Returns true when `file` matches any of the listed language codes.
/// Empty list = no filter (everything matches).
pub fn match_langs(langs: &[String], file: &Path) -> bool {
    if langs.is_empty() {
        return true;
    }
    langs.iter().any(|l| matches_single(l, file))
}

fn matches_single(lang: &str, file: &Path) -> bool {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match lang.to_lowercase().as_str() {
        "rust" => ext == "rs",
        "java" => ext == "java",
        "kotlin" => ext == "kt" || ext == "kts",
        "typescript" => ext == "ts" || ext == "tsx",
        "javascript" => ext == "js" || ext == "jsx",
        "python" => ext == "py",
        "go" => ext == "go",
        "config" => matches!(ext.as_str(), "yaml" | "yml" | "toml" | "json"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_filter_matches_all() {
        assert!(match_langs(&[], Path::new("a.java")));
        assert!(match_langs(&[], Path::new("a.rs")));
    }

    #[test]
    fn rust_matches_rs_only() {
        assert!(match_langs(&["rust".into()], Path::new("a.rs")));
        assert!(!match_langs(&["rust".into()], Path::new("a.java")));
    }

    #[test]
    fn multi_lang_matches_any() {
        let langs = vec!["rust".into(), "java".into()];
        assert!(match_langs(&langs, Path::new("a.rs")));
        assert!(match_langs(&langs, Path::new("a.java")));
        assert!(!match_langs(&langs, Path::new("a.py")));
    }
}
