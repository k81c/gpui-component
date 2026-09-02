//! Table formatter for djot, markdown (pipe tables) and AsciiDoc (`|===` tables).
//!
//! `format_pipe_table`    – aligns djot/markdown pipe table columns.
//! `format_asciidoc_table` – aligns AsciiDoc `|===` table columns.
//!
//! Both functions rewrite cell content so that every column is padded to the
//! width of its widest cell.  Existing extra spaces are trimmed first so the
//! result is always the minimal-width alignment.

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Per-column alignment derived from a pipe-table separator row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColAlign {
    Default,
    Left,
    Right,
    Center,
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipe table (djot / markdown)
// ─────────────────────────────────────────────────────────────────────────────

/// Split a pipe table row into trimmed cell strings.
///
/// Handles:
/// - Leading/trailing `|` stripped
/// - `\|` escape sequences treated as literal `|`
/// - Backtick verbatim spans (inner `|` not treated as cell separator)
fn split_pipe_row(line: &str) -> Vec<String> {
    let s = line.trim();
    let s = s.strip_prefix('|').unwrap_or(s);
    let s = s.strip_suffix('|').unwrap_or(s);

    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_backtick = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '`' => {
                in_backtick = !in_backtick;
                cur.push(c);
            }
            '\\' if !in_backtick => {
                cur.push(c);
                if let Some(&nc) = chars.peek() {
                    cur.push(nc);
                    chars.next();
                }
            }
            '|' if !in_backtick => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

/// Return `true` when every cell looks like a separator (`-`s + optional `:`).
fn is_separator_row(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|c| {
            let s = c.trim().trim_start_matches(':').trim_end_matches(':');
            !s.is_empty() && s.chars().all(|ch| ch == '-')
        })
}

/// Detect alignment from one separator cell.
fn detect_alignment(cell: &str) -> ColAlign {
    let s = cell.trim();
    match (s.starts_with(':'), s.ends_with(':')) {
        (true, true)  => ColAlign::Center,
        (true, false) => ColAlign::Left,
        (false, true) => ColAlign::Right,
        _             => ColAlign::Default,
    }
}

/// Build a separator cell whose total char count equals `width + 2`
/// (matching the data-cell format ` {cell:<width$} `).
fn make_sep_cell(width: usize, align: ColAlign) -> String {
    match align {
        // `:` + (width+1) dashes  → width+2 chars
        ColAlign::Left    => format!(":{}", "-".repeat(width + 1)),
        // (width+1) dashes + `:`  → width+2 chars
        ColAlign::Right   => format!("{}:", "-".repeat(width + 1)),
        // `:` + width dashes + `:` → width+2 chars
        ColAlign::Center  => format!(":{}:", "-".repeat(width)),
        // (width+2) dashes         → width+2 chars
        ColAlign::Default => "-".repeat(width + 2),
    }
}

/// Format a djot/markdown pipe table so that all columns are padded to a
/// uniform width.  Existing extra spaces are trimmed before measuring so the
/// result is minimal-width aligned.
///
/// Lines that do not start with `|` (e.g. djot captions `^ …`) pass through
/// unchanged in their original position.
///
/// Returns `None` when the source contains no pipe rows.
pub(crate) fn format_pipe_table(src: &str) -> Option<String> {
    let raw_lines: Vec<&str> = src.lines().collect();
    if raw_lines.is_empty() {
        return None;
    }

    struct PipeRow {
        line_idx: usize,
        cells: Vec<String>,
        is_sep: bool,
    }

    let mut pipe_rows: Vec<PipeRow> = Vec::new();
    let mut passthrough: Vec<(usize, &str)> = Vec::new();

    for (i, &line) in raw_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') {
            let cells = split_pipe_row(trimmed);
            let is_sep = is_separator_row(&cells);
            pipe_rows.push(PipeRow { line_idx: i, cells, is_sep });
        } else {
            passthrough.push((i, line));
        }
    }

    if pipe_rows.is_empty() {
        return None;
    }

    let sep_local: Option<usize> = pipe_rows.iter().position(|r| r.is_sep);
    let col_count = pipe_rows.iter().map(|r| r.cells.len()).max()?;

    let alignments: Vec<ColAlign> = if let Some(si) = sep_local {
        (0..col_count)
            .map(|j| {
                pipe_rows[si]
                    .cells
                    .get(j)
                    .map(|c| detect_alignment(c))
                    .unwrap_or(ColAlign::Default)
            })
            .collect()
    } else {
        vec![ColAlign::Default; col_count]
    };

    // Compute per-column width from data rows only (exclude separator row).
    let mut col_widths = vec![1usize; col_count];
    for (li, row) in pipe_rows.iter().enumerate() {
        if sep_local == Some(li) {
            continue;
        }
        for (j, cell) in row.cells.iter().enumerate() {
            if j < col_count {
                col_widths[j] = col_widths[j].max(cell.len());
            }
        }
    }
    // Minimum width = 1 (separator always fits: `:--` / `--:` / `:-:` / `---`).
    for w in col_widths.iter_mut() {
        *w = (*w).max(1);
    }

    let formatted_rows: Vec<String> = pipe_rows
        .iter()
        .enumerate()
        .map(|(li, row)| {
            let is_sep = sep_local == Some(li);
            let mut out = String::from("|");
            for j in 0..col_count {
                let width = col_widths[j];
                let align = alignments.get(j).copied().unwrap_or(ColAlign::Default);
                let cell  = row.cells.get(j).map(|s| s.as_str()).unwrap_or("");

                if is_sep {
                    out.push_str(&make_sep_cell(width, align));
                    out.push('|');
                } else {
                    let padded = match align {
                        ColAlign::Right => format!(" {:>width$} ", cell, width = width),
                        ColAlign::Center => {
                            let pad_total = width.saturating_sub(cell.len().min(width));
                            let pad_left  = pad_total / 2;
                            let pad_right = pad_total - pad_left;
                            format!(
                                " {}{}{} ",
                                " ".repeat(pad_left),
                                cell,
                                " ".repeat(pad_right)
                            )
                        }
                        _ => format!(" {:<width$} ", cell, width = width),
                    };
                    out.push_str(&padded);
                    out.push('|');
                }
            }
            out
        })
        .collect();

    // Reassemble lines in original order.
    let mut result_lines: Vec<String> = Vec::with_capacity(raw_lines.len());
    let mut pipe_iter = pipe_rows.iter().enumerate();
    let mut pass_iter = passthrough.iter();
    let mut next_pipe = pipe_iter.next();
    let mut next_pass = pass_iter.next();

    for i in 0..raw_lines.len() {
        if let Some((li, pr)) = next_pipe {
            if pr.line_idx == i {
                result_lines.push(formatted_rows[li].clone());
                next_pipe = pipe_iter.next();
                continue;
            }
        }
        if let Some(&(pi, pline)) = next_pass {
            if pi == i {
                result_lines.push(pline.to_string());
                next_pass = pass_iter.next();
            }
        }
    }

    let mut result = result_lines.join("\n");
    if src.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
}

// ─────────────────────────────────────────────────────────────────────────────
// AsciiDoc table (`|===` … `|===`)
// ─────────────────────────────────────────────────────────────────────────────

/// Split one AsciiDoc table row into trimmed cell strings.
///
/// AsciiDoc rows look like `| cell1 | cell2`.  The leading `|` opens the
/// first cell; subsequent `|` (possibly preceded by cell-specifiers like
/// `2+|`, `^|`) delimit further cells.
fn split_asciidoc_row(line: &str) -> Vec<String> {
    let s = line.trim().strip_prefix('|').unwrap_or(line.trim());

    let mut cells = Vec::new();
    let mut cur   = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                cur.push(c);
                if let Some(&nc) = chars.peek() {
                    cur.push(nc);
                    chars.next();
                }
            }
            '|' => {
                // Strip any trailing cell-specifier chars before the `|`.
                let cell = cur
                    .trim_end_matches(|c: char| {
                        c.is_ascii_digit() || matches!(c, '+' | '.' | '^' | '<' | '>' | '~')
                    })
                    .trim()
                    .to_string();
                cells.push(cell);
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let last = cur.trim().to_string();
    if !last.is_empty() || !cells.is_empty() {
        cells.push(last);
    }
    cells
}

/// Format an AsciiDoc `|===` … `|===` table so that all columns are padded
/// to a uniform width.
///
/// - Lines before/after the `|===` fences pass through unchanged.
/// - Blank lines inside the body (header/body separator) are preserved.
/// - Non-row lines inside the body (attribute lists etc.) pass through.
/// - Only single-line cells are handled; multi-line cell content is left as-is.
///
/// Returns `None` when no `|===` delimiters are found or the table is empty.
pub(crate) fn format_asciidoc_table(src: &str) -> Option<String> {
    let raw_lines: Vec<&str> = src.lines().collect();
    if raw_lines.is_empty() {
        return None;
    }

    let start_idx = raw_lines.iter().position(|l| l.trim() == "|===")?;
    let end_idx   = raw_lines.iter().rposition(|l| l.trim() == "|===")?;
    if start_idx >= end_idx {
        return None;
    }

    let before     = &raw_lines[..start_idx];
    let body_lines = &raw_lines[start_idx + 1..end_idx];
    let after      = &raw_lines[end_idx + 1..];

    enum BodyLine {
        Row(Vec<String>),
        PassThrough(String),
    }

    let body: Vec<BodyLine> = body_lines
        .iter()
        .map(|&line| {
            let trimmed = line.trim();
            if trimmed.starts_with('|') && trimmed != "|===" {
                BodyLine::Row(split_asciidoc_row(trimmed))
            } else {
                BodyLine::PassThrough(line.to_string())
            }
        })
        .collect();

    let col_count = body
        .iter()
        .filter_map(|bl| {
            if let BodyLine::Row(cells) = bl { Some(cells.len()) } else { None }
        })
        .max()
        .unwrap_or(0);

    if col_count == 0 {
        return None;
    }

    let mut col_widths = vec![1usize; col_count];
    for bl in &body {
        if let BodyLine::Row(cells) = bl {
            for (j, cell) in cells.iter().enumerate() {
                if j < col_count {
                    col_widths[j] = col_widths[j].max(cell.len());
                }
            }
        }
    }

    let mut body_out: Vec<String> = Vec::with_capacity(body.len());
    for bl in &body {
        match bl {
            BodyLine::Row(cells) => {
                let mut row = String::from("|");
                for j in 0..col_count {
                    let width = col_widths[j];
                    let cell  = cells.get(j).map(|s| s.as_str()).unwrap_or("");
                    row.push_str(&format!(" {:<width$} |", cell, width = width));
                }
                body_out.push(row);
            }
            BodyLine::PassThrough(s) => body_out.push(s.clone()),
        }
    }

    let mut result_lines: Vec<String> = Vec::new();
    result_lines.extend(before.iter().map(|s| s.to_string()));
    result_lines.push("|===".to_string());
    result_lines.extend(body_out);
    result_lines.push("|===".to_string());
    result_lines.extend(after.iter().map(|s| s.to_string()));

    let mut result = result_lines.join("\n");
    if src.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public dispatch helper used by `InputState::format_table_at`
// ─────────────────────────────────────────────────────────────────────────────

/// Format the table in `src`, auto-detecting whether it is a pipe table or
/// an AsciiDoc `|===` table.  Returns `None` when formatting is not applicable.
pub(crate) fn format_table(src: &str) -> Option<String> {
    if src.lines().any(|l| l.trim() == "|===") {
        format_asciidoc_table(src)
    } else {
        format_pipe_table(src)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── pipe table ──────────────────────────────────────────────────────────

    #[test]
    fn test_format_simple_table() {
        let src = "| fruit | price |\n|---|---|\n| apple | 4 |\n| banana | 10 |\n";
        let out = format_pipe_table(src).unwrap();
        assert!(out.contains("| fruit  |"), "got: {}", out);
        assert!(out.contains("| banana |"), "got: {}", out);
        // All rows must have the same total length.
        let lens: Vec<usize> = out.trim().lines().map(|l| l.len()).collect();
        assert!(
            lens.windows(2).all(|w| w[0] == w[1]),
            "row lengths differ: {:?}\n{}",
            lens,
            out
        );
    }

    #[test]
    fn test_alignment_preserved() {
        let src = "| a | b |\n|:---|---:|\n| left | right |\n";
        let out = format_pipe_table(src).unwrap();
        let lens: Vec<usize> = out.trim().lines().map(|l| l.len()).collect();
        assert!(
            lens.windows(2).all(|w| w[0] == w[1]),
            "row lengths differ: {:?}\n{}",
            lens,
            out
        );
        assert!(out.contains(":-"), "left-aligned marker missing: {}", out);
        assert!(out.contains("-:"), "right-aligned marker missing: {}", out);
    }

    #[test]
    fn test_no_trailing_newline() {
        let src = "| a | b |\n|---|---|\n| 1 | 2 |";
        let out = format_pipe_table(src).unwrap();
        assert!(!out.ends_with('\n'));
    }

    #[test]
    fn test_passthrough_lines_preserved() {
        let src = "^ Caption line\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let out = format_pipe_table(src).unwrap();
        assert!(out.starts_with("^ Caption line\n"), "got: {}", out);
    }

    #[test]
    fn test_excess_spaces_removed() {
        let src = "| a      | b |\n|---|---|\n| x | y |\n";
        let out = format_pipe_table(src).unwrap();
        // 'a' = 1 char, 'b' = 1 char → both columns width 1
        assert!(out.contains("| a | b |"), "got: {}", out);
    }

    // ── AsciiDoc table ──────────────────────────────────────────────────────

    #[test]
    fn test_format_asciidoc_simple() {
        let src = "|===\n| Name | Age\n\n| Alice | 25\n| Bob | 30\n|===\n";
        let out = format_asciidoc_table(src).unwrap();
        assert!(out.contains("| Alice | 25 |"), "got: {}", out);
        assert!(out.contains("| Bob   | 30 |"), "got: {}", out);
    }

    #[test]
    fn test_format_asciidoc_preserves_blank_separator() {
        let src = "|===\n| H1 | H2\n\n| a | b\n|===\n";
        let out = format_asciidoc_table(src).unwrap();
        assert!(out.contains("\n\n"), "blank separator lost: {}", out);
    }

    #[test]
    fn test_format_asciidoc_before_after_preserved() {
        let src = ".Title\n[cols=\"1,2\"]\n|===\n| a | b\n|===\nParagraph after.\n";
        let out = format_asciidoc_table(src).unwrap();
        assert!(out.starts_with(".Title\n"), "got: {}", out);
        assert!(out.contains("Paragraph after."), "got: {}", out);
    }

    #[test]
    fn test_format_asciidoc_row_lengths_equal() {
        let src =
            "|===\n| Name | Role | Department\n\n| Alice | Engineer | Backend\n| Bob | Designer | Frontend\n|===\n";
        let out = format_asciidoc_table(src).unwrap();
        let rows: Vec<&str> = out
            .lines()
            .filter(|l| l.trim().starts_with('|') && l.trim() != "|===")
            .collect();
        let lens: Vec<usize> = rows.iter().map(|r| r.len()).collect();
        assert!(
            lens.windows(2).all(|w| w[0] == w[1]),
            "row lengths differ: {:?}\n{}",
            lens,
            out
        );
    }

    // ── dispatch ────────────────────────────────────────────────────────────

    #[test]
    fn test_format_table_dispatch_pipe() {
        let src = "| a | b |\n|---|---|\n| 1 | 2 |\n";
        let out = format_table(src).unwrap();
        assert!(out.contains("| a | b |"), "got: {}", out);
    }

    #[test]
    fn test_format_table_dispatch_asciidoc() {
        let src = "|===\n| a | b\n| 1 | 2\n|===\n";
        let out = format_table(src).unwrap();
        assert!(out.contains("| a | b |"), "got: {}", out);
    }
}
