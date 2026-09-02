//! Lightweight table formatting for Markdown/Djot pipe tables and AsciiDoc.

fn split_pipe_row(line: &str) -> Vec<String> {
    let line = line.trim().trim_start_matches('|').trim_end_matches('|');
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            cell.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
            cell.push(ch);
        } else if ch == '`' {
            quoted = !quoted;
            cell.push(ch);
        } else if ch == '|' && !quoted {
            cells.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(ch);
        }
    }
    cells.push(cell.trim().to_string());
    cells
}

fn split_asciidoc_row(line: &str) -> Vec<String> {
    let mut cells = split_pipe_row(line);
    // AsciiDoc permits cell specifiers such as `2+|` and `^|` before the
    // first cell. The generic pipe splitter sees the specifier as a cell;
    // discard it while retaining the actual value.
    if !line.trim_start().starts_with('|')
        && cells.len() > 1
        && cells[0]
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '+' | '.' | '^' | '<' | '>' | '~'))
    {
        cells.remove(0);
    }
    cells
}

fn separator(cell: &str) -> bool {
    let cell = cell.trim().trim_matches(':');
    !cell.is_empty() && cell.chars().all(|c| c == '-')
}

fn format_pipe_table(src: &str) -> Option<String> {
    let lines: Vec<&str> = src.lines().collect();
    let rows: Vec<(usize, Vec<String>)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| {
            line.trim()
                .starts_with('|')
                .then(|| (i, split_pipe_row(line)))
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    if rows.len() < 2 {
        return None;
    }
    let separator_row = rows
        .iter()
        .position(|(_, cells)| cells.iter().all(|c| separator(c)));
    let columns = rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0);
    let mut widths = vec![1; columns];
    for (row, (_, cells)) in rows.iter().enumerate() {
        if separator_row == Some(row) {
            continue;
        }
        for (column, cell) in cells.iter().enumerate() {
            widths[column] = widths[column].max(cell.chars().count());
        }
    }
    let mut row_iter = rows.iter();
    let mut output = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        if let Some((row_index, (_, cells))) =
            row_iter.clone().enumerate().find(|(_, (i, _))| *i == index)
        {
            let is_separator = separator_row == Some(row_index);
            let mut formatted = String::from("|");
            for column in 0..columns {
                let width = widths[column];
                let cell = cells.get(column).map(String::as_str).unwrap_or("");
                if is_separator {
                    let left = cell.trim().starts_with(':');
                    let right = cell.trim().ends_with(':');
                    let marker_count = left as usize + right as usize;
                    let dashes = "-".repeat((width + 2).saturating_sub(marker_count));
                    formatted.push_str(if left { ":" } else { "" });
                    formatted.push_str(&dashes);
                    formatted.push_str(if right { ":" } else { "" });
                    formatted.push('|');
                } else {
                    formatted.push_str(&format!(" {:<width$} |", cell, width = width));
                }
            }
            output.push(formatted);
            row_iter.next();
        } else {
            output.push((*line).to_string());
        }
    }
    let mut result = output.join("\n");
    if src.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
}

fn format_asciidoc_table(src: &str) -> Option<String> {
    let lines: Vec<&str> = src.lines().collect();
    let start = lines.iter().position(|line| line.trim() == "|===")?;
    let end = lines.iter().rposition(|line| line.trim() == "|===")?;
    if start >= end {
        return None;
    }
    let rows: Vec<(usize, Vec<String>)> = lines[start + 1..end]
        .iter()
        .enumerate()
        .filter_map(|(offset, line)| {
            let trimmed = line.trim();
            trimmed
                .starts_with('|')
                .then(|| (offset, split_asciidoc_row(trimmed)))
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    let columns = rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0);
    let mut widths = vec![1; columns];
    for (_, cells) in &rows {
        for (column, cell) in cells.iter().enumerate() {
            widths[column] = widths[column].max(cell.chars().count());
        }
    }
    let mut output: Vec<String> = lines[..=start]
        .iter()
        .map(|line| (*line).to_string())
        .collect();
    for line in &lines[start + 1..end] {
        if !line.trim().starts_with('|') {
            output.push((*line).to_string());
            continue;
        }
        let cells = split_asciidoc_row(line);
        let mut formatted = String::from("|");
        for column in 0..columns {
            formatted.push_str(&format!(
                " {:<width$} |",
                cells.get(column).map(String::as_str).unwrap_or(""),
                width = widths[column]
            ));
        }
        output.push(formatted);
    }
    output.extend(lines[end..].iter().map(|line| (*line).to_string()));
    let mut result = output.join("\n");
    if src.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
}

pub(crate) fn format_table(src: &str) -> Option<String> {
    if src.lines().any(|line| line.trim() == "|===") {
        format_asciidoc_table(src)
    } else {
        format_pipe_table(src)
    }
}

#[cfg(test)]
mod tests {
    use super::format_table;

    #[test]
    fn aligns_pipe_rows_and_preserves_newline() {
        let source = "| a | price |\n|---|---|\n| pear | 10 |\n";
        let formatted = format_table(source).unwrap();
        assert!(formatted.ends_with('\n'));
        let rows: Vec<_> = formatted.trim().lines().collect();
        assert_eq!(rows[0].len(), rows[1].len());
        assert_eq!(rows[1].len(), rows[2].len());
    }

    #[test]
    fn formats_asciidoc_fenced_table() {
        let source = "|===\n| Name | Age\n| A | 1\n|===";
        let formatted = format_table(source).unwrap();
        assert!(formatted.contains("| A    | 1   |"));
    }
}
