//! `coregraph review --pr <N>` — auto-comment a GitHub pull request
//! with the diff impact summary.
//!
//! Pipeline:
//!   1. Resolve the PR's base/head SHAs through `gh pr view --json …`.
//!   2. Run the `diff` analysis between those two refs.
//!   3. Format a Markdown summary and either print it (`--dry-run`)
//!      or post it as an issue comment via `gh pr comment`.
//!
//! Depends on the `gh` CLI being installed and authenticated. We
//! shell out rather than touching the GitHub REST API directly so
//! the user's existing auth (token, OAuth) just works.

use clap::Args;
use coregraph_core::SymbolId;
use coregraph_query::{compute_impact, is_test_path};
use coregraph_watcher::GitDiffStrategy;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use crate::global_opts::GlobalOpts;
use crate::graph_loader::load_project_graph_only;

#[derive(Args)]
pub struct ReviewArgs {
    /// PR number in the current repo. Inferred via `gh pr view`.
    #[arg(long)]
    pub pr: u32,

    /// Print the comment body to stdout instead of posting.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Impact propagation depth from each touched symbol.
    #[arg(long, default_value_t = 3)]
    pub max_depth: usize,

    /// Skip symbols defined under test directories.
    #[arg(long, default_value_t = false)]
    pub exclude_tests: bool,
}

pub fn run(args: ReviewArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let project_root = globals.project_root();

    // 1. Resolve PR refs via gh.
    let (base_sha, head_sha, title) = pr_metadata(&project_root, args.pr)?;

    // 2. Run the diff analysis.
    let changed = GitDiffStrategy::changed_files_between(&project_root, &base_sha, &head_sha)?;
    let graph = load_project_graph_only(&project_root)?;
    let canonical_changed: HashSet<PathBuf> = changed
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect();
    let mut touched: Vec<&coregraph_core::SymbolNode> = graph
        .nodes()
        .filter(|n| {
            let nf = std::fs::canonicalize(&n.file).unwrap_or_else(|_| n.file.to_path_buf());
            canonical_changed.contains(&nf)
        })
        .collect();
    if args.exclude_tests {
        touched.retain(|n| !is_test_path(&n.file));
    }
    touched.sort_by(|a, b| (a.file.as_ref(), a.span_start).cmp(&(b.file.as_ref(), b.span_start)));

    let mut reached: HashSet<SymbolId> = HashSet::new();
    for seed in &touched {
        for n in compute_impact(&graph, seed.id, args.max_depth).reachable {
            reached.insert(n.id);
        }
    }
    for seed in &touched {
        reached.remove(&seed.id);
    }

    // 3. Render the Markdown comment.
    let body = render_comment(
        &title,
        &base_sha,
        &head_sha,
        changed.len(),
        &touched,
        reached.len(),
        args.max_depth,
    );

    if args.dry_run {
        println!("{}", body);
        return Ok(());
    }

    // Post via gh.
    let status = Command::new("gh")
        .args(["pr", "comment", &args.pr.to_string(), "--body-file", "-"])
        .current_dir(&project_root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(body.as_bytes())?;
            }
            child.wait()
        })?;
    if !status.success() {
        anyhow::bail!("gh pr comment exited with status {}", status);
    }
    println!("Posted impact comment on PR #{}", args.pr);
    Ok(())
}

/// Run `gh pr view <N> --json baseRefOid,headRefOid,title` and
/// return the parsed values. We require all three; missing fields
/// signal a stale/closed PR or an authentication problem and we'd
/// rather fail loudly than post a misleading comment.
fn pr_metadata(repo_root: &std::path::Path, pr: u32) -> anyhow::Result<(String, String, String)> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr.to_string(),
            "--json",
            "baseRefOid,headRefOid,title",
        ])
        .current_dir(repo_root)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "`gh pr view {}` failed (exit {}). Is the GitHub CLI installed and authenticated?",
            pr,
            output.status
        );
    }
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let base = parsed["baseRefOid"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("gh response missing baseRefOid"))?
        .to_string();
    let head = parsed["headRefOid"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("gh response missing headRefOid"))?
        .to_string();
    let title = parsed["title"].as_str().unwrap_or("").to_string();
    Ok((base, head, title))
}

/// Escape a contributor-controlled string for safe inline embedding in a
/// Markdown comment: strip line breaks (which would escape a blockquote) and
/// backslash-escape the characters that open emphasis, code spans, links, or
/// raw HTML. Prevents Markdown-content injection via the PR title.
fn escape_md_inline(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\r' | '\n' => out.push(' '),
            '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '<' | '>' | '|' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn render_comment(
    title: &str,
    base: &str,
    head: &str,
    changed_files: usize,
    touched: &[&coregraph_core::SymbolNode],
    reachable: usize,
    depth: usize,
) -> String {
    let mut s = String::new();
    s.push_str("## CoreGraph impact analysis\n\n");
    if !title.is_empty() {
        // The PR title is contributor-controlled; neutralize Markdown-active
        // characters so it cannot break out of the blockquote/emphasis context.
        s.push_str(&format!("> _{}_\n\n", escape_md_inline(title)));
    }
    s.push_str(&format!(
        "Diffing `{base}..{head}` (depth {depth}):\n\n\
         | Metric | Count |\n\
         |--------|-------|\n\
         | Changed files | **{}** |\n\
         | Touched symbols | **{}** |\n\
         | Reachable symbols | **{}** |\n\n",
        changed_files,
        touched.len(),
        reachable,
        base = &base[..base.len().min(7)],
        head = &head[..head.len().min(7)],
    ));
    if touched.is_empty() {
        s.push_str("_No symbols touched — refactor or doc-only change._\n");
    } else {
        s.push_str("### Touched symbols\n\n");
        for n in touched.iter().take(25) {
            s.push_str(&format!(
                "- `{}` _{:?}_ — `{}`\n",
                n.name,
                n.kind,
                n.file.display()
            ));
        }
        if touched.len() > 25 {
            s.push_str(&format!("\n…and {} more.\n", touched.len() - 25));
        }
    }
    s.push_str("\n<sub>Generated by `coregraph review`.</sub>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::escape_md_inline;

    #[test]
    fn escapes_markdown_breakout_chars_in_pr_title() {
        // Emphasis / code / link / HTML / table openers are backslash-escaped.
        assert_eq!(
            escape_md_inline("pwn_ed `code` [x](y) <img> *b* |t| ~s~"),
            "pwn\\_ed \\`code\\` \\[x\\]\\(y\\) \\<img\\> \\*b\\* \\|t\\| \\~s\\~"
        );
        // Line breaks (which would escape the blockquote) collapse to spaces.
        assert_eq!(escape_md_inline("a\r\nb"), "a  b");
        // A benign title is left untouched.
        assert_eq!(escape_md_inline("Fix bug in parser"), "Fix bug in parser");
    }
}
