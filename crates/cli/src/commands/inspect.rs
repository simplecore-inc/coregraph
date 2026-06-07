use crate::global_opts::{GlobalOpts, OutputFormat};
use clap::Args;
use coregraph_extractor::build_graph;
use std::path::Path;

#[derive(Args)]
pub struct InspectArgs {
    /// File path and line number (e.g. src/main.rs:42)
    pub target: String,

    /// Number of surrounding source lines to include.
    #[arg(long, default_value_t = 5)]
    pub context_lines: usize,
}

pub fn run(args: InspectArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let (file, line) = parse_file_line(&args.target)?;
    if line == 0 {
        return Err(anyhow::anyhow!(
            "line numbers are 1-indexed (got {}:0)",
            file.display()
        ));
    }
    let (graph, _) = build_graph(&globals.project)?;

    // Find symbols whose span contains the requested line — or, failing that,
    // the closest match in the same file.
    let file_path = globals.project.join(&file);
    let source = std::fs::read_to_string(&file_path)
        .or_else(|_| std::fs::read_to_string(&file))
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", file.display(), e))?;

    // Reject out-of-range line numbers instead of silently clamping to EOF,
    // which previously let `inspect foo.rs:1000000` return an arbitrary
    // symbol from the tail of the file.
    let total_lines = source.split_inclusive('\n').count().max(1);
    if line > total_lines {
        return Err(anyhow::anyhow!(
            "line {} is outside {} (file has {} line{})",
            line,
            file.display(),
            total_lines,
            if total_lines == 1 { "" } else { "s" }
        ));
    }

    let line_byte = line_to_byte_offset(&source, line);

    let mut hits: Vec<_> = graph
        .nodes()
        .filter(|n| {
            let nf = n.file.to_string_lossy();
            nf.ends_with(file.to_string_lossy().as_ref()) || *n.file == *file_path
        })
        .filter(|n| (n.span_start..=n.span_end).contains(&line_byte))
        .collect();
    if hits.is_empty() {
        // Fallback: any symbol in the same file, ordered by distance to requested byte.
        // Cap at 5 candidates so a caller inspecting a file with hundreds of
        // symbols still sees a short, focused result.
        hits = graph
            .nodes()
            .filter(|n| {
                let nf = n.file.to_string_lossy();
                nf.ends_with(file.to_string_lossy().as_ref())
            })
            .collect();
        hits.sort_by_key(|n| (n.span_start as i64 - line_byte as i64).abs());
    }

    match globals.output_format {
        OutputFormat::Json => {
            let json: Vec<_> = hits
                .iter()
                .take(5)
                .map(|n| {
                    serde_json::json!({
                        "id": n.id,
                        "name": n.name,
                        "kind": format!("{:?}", n.kind),
                        "file": n.file.display().to_string(),
                        "span_start": n.span_start,
                        "span_end": n.span_end,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        _ => {
            if hits.is_empty() {
                println!("No symbol found at {}:{}", file.display(), line);
            } else {
                println!("── inspect: {}:{} ──", file.display(), line);
                for n in hits.iter().take(5) {
                    println!(
                        "  {} [{:?}] bytes {}..{}",
                        n.name, n.kind, n.span_start, n.span_end
                    );
                }
                println!();
                print_context(&source, line, args.context_lines);
            }
        }
    }
    Ok(())
}

fn parse_file_line(target: &str) -> anyhow::Result<(std::path::PathBuf, usize)> {
    let idx = target
        .rfind(':')
        .ok_or_else(|| anyhow::anyhow!("expected FILE:LINE, got {}", target))?;
    let (file, line) = target.split_at(idx);
    let line: usize = line[1..]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid line number in {}", target))?;
    Ok((Path::new(file).to_path_buf(), line))
}

fn line_to_byte_offset(source: &str, line_1_indexed: usize) -> u32 {
    let mut offset = 0usize;
    for (i, l) in source.split_inclusive('\n').enumerate() {
        if i + 1 >= line_1_indexed {
            return offset as u32;
        }
        offset += l.len();
    }
    offset as u32
}

fn print_context(source: &str, line: usize, context: usize) {
    let lines: Vec<_> = source.split_inclusive('\n').collect();
    if lines.is_empty() {
        return;
    }
    let start = line.saturating_sub(context).max(1);
    let end = (line + context).min(lines.len());
    for n in start..=end {
        let marker = if n == line { "→" } else { " " };
        print!("  {} {:>4} {}", marker, n, lines[n - 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file_line_ok() {
        let (f, l) = parse_file_line("src/main.rs:42").unwrap();
        assert_eq!(f, Path::new("src/main.rs"));
        assert_eq!(l, 42);
    }

    #[test]
    fn parse_file_line_requires_colon() {
        assert!(parse_file_line("nope").is_err());
    }

    #[test]
    fn line_to_byte_offset_first_line_is_zero() {
        assert_eq!(line_to_byte_offset("abc\ndef\n", 1), 0);
    }

    #[test]
    fn line_to_byte_offset_second_line_after_first_newline() {
        assert_eq!(line_to_byte_offset("abc\ndef\n", 2), 4);
    }

    #[test]
    fn parse_file_line_rejects_non_numeric_line() {
        assert!(parse_file_line("src/main.rs:notanumber").is_err());
    }
}
