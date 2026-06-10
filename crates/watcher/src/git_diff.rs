use std::path::{Path, PathBuf};
use std::process::Command;

/// Uses `git` CLI to detect files changed relative to HEAD.
pub struct GitDiffStrategy;

impl GitDiffStrategy {
    /// Returns absolute paths of files that differ between HEAD and the working tree.
    /// Returns an empty vec when git exits non-zero (e.g. the directory is not a
    /// repo). A failure to spawn the `git` binary (not installed / not on PATH) is
    /// propagated as an `Err` rather than swallowed into an empty vec.
    pub fn changed_files_since_head(repo_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(repo_root)
            .output()?;

        if !output.status.success() {
            return Ok(vec![]);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let paths: Vec<PathBuf> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| repo_root.join(l.trim()))
            .collect();

        Ok(paths)
    }

    /// Returns true if a multi-file git operation (merge, rebase, cherry-pick) is in progress.
    pub fn detect_git_operation(repo_root: &Path) -> bool {
        let git_dir = repo_root.join(".git");
        git_dir.join("MERGE_HEAD").exists()
            || git_dir.join("REBASE_HEAD").exists()
            || git_dir.join("CHERRY_PICK_HEAD").exists()
    }

    /// Files changed between two arbitrary git revisions, including
    /// the working tree when `to == "HEAD"`. Used by `coregraph diff`
    /// to compute the seed file set for impact analysis.
    ///
    /// Falls back to an empty vec when git exits non-zero (not in a repo,
    /// bad rev) so callers can degrade to "no impact". A failure to spawn
    /// the `git` binary itself is propagated as an `Err`.
    pub fn changed_files_between(
        repo_root: &Path,
        from: &str,
        to: &str,
    ) -> anyhow::Result<Vec<PathBuf>> {
        // `from..to` is the "what's in `to` that wasn't in `from`"
        // form; for an "everything that differs" view we use the
        // ranged diff with `--name-only`. When the user passes
        // `HEAD` as `to` we also pick up uncommitted changes by
        // appending an explicit working-tree diff.
        let mut paths: Vec<PathBuf> = Vec::new();
        let output = Command::new("git")
            .args(["diff", "--name-only", &format!("{}..{}", from, to)])
            .current_dir(repo_root)
            .output()?;
        if !output.status.success() {
            return Ok(paths);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().filter(|l| !l.is_empty()) {
            paths.push(repo_root.join(line.trim()));
        }
        if to == "HEAD" {
            // Also pick up working-tree edits the caller hasn't
            // committed yet — most "what does my PR touch" workflows
            // assume those are part of the diff.
            if let Ok(extra) = Self::changed_files_since_head(repo_root) {
                paths.extend(extra);
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn no_git_operation_in_clean_state() {
        let repo_root = env::current_dir()
            .unwrap()
            .ancestors()
            .find(|p| p.join(".git").exists())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| env::current_dir().unwrap());

        let op = GitDiffStrategy::detect_git_operation(&repo_root);
        let _ = op; // just verify no panic
    }

    #[test]
    fn changed_files_returns_vec() {
        let repo_root = env::current_dir()
            .unwrap()
            .ancestors()
            .find(|p| p.join(".git").exists())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| env::current_dir().unwrap());

        let files = GitDiffStrategy::changed_files_since_head(&repo_root);
        assert!(files.is_ok());
    }
}
