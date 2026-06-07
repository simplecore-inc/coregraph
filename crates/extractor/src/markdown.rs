//! Minimal Markdown structure parsing for the external-docs layer
//! (docs/graph-model.md §7.6). Splits a document into heading-delimited
//! sections and finds backticked identifiers that may name a code symbol.
//!
//! This is deliberately a small line scanner, not a full CommonMark parser:
//! the docs layer only needs section boundaries (ATX headings) and inline code
//! spans, and avoiding a Markdown grammar dependency keeps ingestion light.

use regex::Regex;

/// A heading-delimited section of a Markdown document. `start`/`end` are byte
/// offsets into the source (`start` at the heading line, `end` at the next
/// heading or EOF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdSection {
    pub heading: String,
    pub start: u32,
    pub end: u32,
}

/// Split `source` into ATX-heading-delimited sections. Headings inside fenced
/// code blocks (``` ``` ```) are ignored. Content before the first heading is
/// not a section (nothing to name it).
pub fn split_sections(source: &str) -> Vec<MdSection> {
    let mut heads: Vec<(u32, String)> = Vec::new();
    let mut offset: usize = 0;
    let mut in_fence = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence && trimmed.starts_with('#') {
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) {
                let rest = &trimmed[hashes..];
                // An ATX heading needs a space (or be empty) after the hashes.
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    heads.push((offset as u32, rest.trim().to_string()));
                }
            }
        }
        offset += line.len();
    }

    let total = source.len() as u32;
    heads
        .iter()
        .enumerate()
        .map(|(i, (start, heading))| {
            let end = heads.get(i + 1).map(|(s, _)| *s).unwrap_or(total);
            MdSection {
                heading: heading.clone(),
                start: *start,
                end,
            }
        })
        .collect()
}

/// Inline backticked identifiers in `text` — `` `Name` `` where the content is
/// a single plain identifier (a possible code-symbol reference). Multi-word or
/// non-identifier code spans (`` `git status` ``, `` `a.b` ``) are skipped, as
/// is anything inside a triple-backtick fence is naturally excluded because a
/// fenced block has no single-backtick spans around bare identifiers.
pub fn code_span_identifiers(text: &str) -> Vec<String> {
    // A single backtick, a plain identifier, a single backtick — not preceded
    // or followed by another backtick (so fence markers ``` don't match).
    let re = Regex::new(r"(?:^|[^`])`([A-Za-z_][A-Za-z0-9_]*)`(?:[^`]|$)").unwrap();
    re.captures_iter(text).map(|c| c[1].to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_atx_headings() {
        let src = "# Title\nintro\n## Config\nuses `port`\n## Other\n";
        let secs = split_sections(src);
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[0].heading, "Title");
        assert_eq!(secs[1].heading, "Config");
        // The Config section text must include its body line.
        let body = &src[secs[1].start as usize..secs[1].end as usize];
        assert!(body.contains("uses `port`"));
    }

    #[test]
    fn ignores_headings_inside_fences() {
        let src = "# Real\n```\n# not a heading\n```\n## Also Real\n";
        let secs = split_sections(src);
        assert_eq!(
            secs.len(),
            2,
            "the fenced `# not a heading` must be ignored"
        );
        assert_eq!(secs[1].heading, "Also Real");
    }

    #[test]
    fn extracts_backticked_identifiers() {
        assert_eq!(
            code_span_identifiers("see `Server` and `port`"),
            vec!["Server", "port"]
        );
    }

    #[test]
    fn skips_non_identifier_code_spans() {
        assert!(code_span_identifiers("run `git status` now").is_empty());
        assert!(code_span_identifiers("the `a.b.c` path").is_empty());
    }
}
