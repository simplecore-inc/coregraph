//! Pure byte-offset → (line, column) conversion for LSP range reporting.
//!
//! Lives in `coregraph-core` because every crate that surfaces
//! positions (cli, graph, query) must agree on the math.
//! **No I/O, no caching** — see `crates/graph/src/file_content_cache.rs`
//! for the daemon-side file-content cache.

/// Convert a 0-based byte offset within `source` to a 0-based `(line, column)`
/// pair suitable for LSP `Position`. Column is measured in UTF-16 code units
/// per the LSP spec default.
///
/// Returns `(line, col)` clamped so that offsets past the end of `source`
/// map to the last valid position rather than panicking.
pub fn resolve_line_col(source: &str, byte_offset: u32) -> (u32, u32) {
    let mut offset = (byte_offset as usize).min(source.len());
    // Snap down to the nearest valid UTF-8 char boundary so we never
    // slice inside a multibyte codepoint (which would panic).
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let mut line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    // Belt-and-suspenders: the byte after '\n' is always a char boundary,
    // but snap defensively in case of future refactors.
    while line_start > 0 && !source.is_char_boundary(line_start) {
        line_start -= 1;
    }
    let line_text = &source[line_start..offset];
    // UTF-16 code units (LSP default `positionEncoding`).
    let col = line_text.encode_utf16().count() as u32;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_offset_is_line_zero_col_zero() {
        assert_eq!(resolve_line_col("foo\nbar", 0), (0, 0));
    }

    #[test]
    fn after_first_newline_is_line_one_col_zero() {
        assert_eq!(resolve_line_col("foo\nbar", 4), (1, 0));
    }

    #[test]
    fn mid_line_column_is_byte_count_ascii() {
        assert_eq!(resolve_line_col("foo\nbar", 6), (1, 2));
    }

    #[test]
    fn utf16_counts_multibyte_as_one_unit() {
        // "한" = 3 UTF-8 bytes, 1 UTF-16 code unit
        assert_eq!(resolve_line_col("한글", 3), (0, 1));
    }

    #[test]
    fn utf16_counts_astral_plane_as_two_units() {
        // 🦀 = 4 UTF-8 bytes, 2 UTF-16 code units (surrogate pair)
        assert_eq!(resolve_line_col("🦀x", 4), (0, 2));
    }

    #[test]
    fn offset_past_end_clamps_to_last_position() {
        let (line, col) = resolve_line_col("foo", 999);
        assert_eq!((line, col), (0, 3));
    }

    #[test]
    fn empty_source_returns_zero_zero() {
        assert_eq!(resolve_line_col("", 0), (0, 0));
        assert_eq!(resolve_line_col("", 100), (0, 0));
    }

    #[test]
    fn offset_mid_multibyte_char_does_not_panic() {
        // "한" is a 3-byte UTF-8 sequence. Offset 1 is mid-codepoint.
        // Must not panic; should snap to boundary (offset 0 → col 0).
        let (line, col) = resolve_line_col("한글", 1);
        assert_eq!((line, col), (0, 0));
    }

    #[test]
    fn offset_mid_multibyte_char_snaps_down() {
        // Offset 5 is mid-second-char of "한글" (6 bytes total).
        // Should snap to offset 3 → col 1.
        let (line, col) = resolve_line_col("한글", 5);
        assert_eq!((line, col), (0, 1));
    }
}
