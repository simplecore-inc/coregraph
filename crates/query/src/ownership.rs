use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Aggregated ownership information for a file, derived from git blame.
#[derive(Debug, Clone)]
pub struct OwnershipInfo {
    pub file: std::path::PathBuf,
    pub owners: HashMap<String, f32>,
    pub available: bool,
}

impl OwnershipInfo {
    pub fn unknown(file: impl Into<std::path::PathBuf>) -> Self {
        Self {
            file: file.into(),
            owners: HashMap::new(),
            available: false,
        }
    }

    pub fn top_owner(&self) -> Option<(&str, f32)> {
        self.owners
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(k, v)| (k.as_str(), *v))
    }
}

/// Run `git blame --porcelain <file>` and derive per-author ownership fractions
/// from the number of distinct commits attributed to each author.
///
/// Note: `--porcelain` emits the `author-mail` header only the first time each
/// commit appears, so this counts commit appearances per author, not blamed
/// lines. The resulting fractions reflect commit-appearance share, not true
/// line ownership; `--line-porcelain` would be required for per-line semantics.
/// Returns `OwnershipInfo::unknown()` if git is unavailable or the file is untracked.
pub fn blame_file(file: &Path, repo_root: &Path) -> OwnershipInfo {
    let output = Command::new("git")
        .args(["blame", "--porcelain"])
        .arg(file)
        .current_dir(repo_root)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return OwnershipInfo::unknown(file),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut author_lines: HashMap<String, usize> = HashMap::new();
    let mut total_lines = 0usize;

    for line in stdout.lines() {
        if line.starts_with("author-mail ") {
            let email = line.trim_start_matches("author-mail ").trim().to_string();
            *author_lines.entry(email).or_insert(0) += 1;
            total_lines += 1;
        }
    }

    if total_lines == 0 {
        return OwnershipInfo::unknown(file);
    }

    let owners: HashMap<String, f32> = author_lines
        .into_iter()
        .map(|(k, v)| (k, v as f32 / total_lines as f32))
        .collect();

    OwnershipInfo {
        file: file.to_path_buf(),
        owners,
        available: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn unknown_ownership_not_available() {
        let info = OwnershipInfo::unknown("nonexistent.rs");
        assert!(!info.available);
        assert!(info.owners.is_empty());
        assert!(info.top_owner().is_none());
    }

    #[test]
    fn blame_returns_ownership_info() {
        let repo_root = env::current_dir()
            .unwrap()
            .ancestors()
            .find(|p| p.join(".git").exists())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| env::current_dir().unwrap());

        let cargo_toml = repo_root.join("Cargo.toml");
        let info = blame_file(&cargo_toml, &repo_root);
        let _ = info.available; // just verify no panic
    }

    #[test]
    fn blame_nonexistent_file_returns_unknown() {
        let repo_root = env::current_dir()
            .unwrap()
            .ancestors()
            .find(|p| p.join(".git").exists())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| env::current_dir().unwrap());

        let info = blame_file(&repo_root.join("nonexistent_phantom_file.rs"), &repo_root);
        assert!(!info.available);
    }
}
