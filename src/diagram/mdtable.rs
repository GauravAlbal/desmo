//! Padded markdown tables. Every table in the product renders through [`md_table`].
//!
//! Columns are padded to the widest cell by display width. A table never exceeds
//! [`MAX_LINE`] columns. When it would, columns that hold prose shrink first, one
//! column at a time starting from the widest, without splitting words. Only if that is
//! not enough do words split. Long cells continue on following rows with the other
//! columns left blank. Header cells never wrap, so a column is never narrower than its
//! header.

use super::{char_width, display_width};

/// Hard ceiling on a rendered table line, in display columns.
pub const MAX_LINE: usize = 120;
const MIN_COLUMN: usize = 12;
const MIN_RULE: usize = 3;
const WORD_CAP: usize = 40;

/// Escape a cell for a markdown table.
pub fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// Render `headers` and `rows` as an aligned markdown table ending in a newline.
/// Cells are raw text; escaping is applied here.
pub fn md_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            (0..headers.len())
                .map(|index| escape_cell(row.get(index).map(String::as_str).unwrap_or("")))
                .collect()
        })
        .collect();
    let widths = column_widths(headers, &cells);
    let mut out = String::new();
    let head: Vec<String> = headers.iter().map(|header| header.to_string()).collect();
    push_row(&mut out, &head, &widths);
    push_rule(&mut out, &widths);
    for row in &cells {
        push_row(&mut out, row, &widths);
    }
    out
}

struct Column {
    width: usize,
    header: usize,
    word: usize,
    prose: bool,
}

fn column_widths(headers: &[&str], cells: &[Vec<String>]) -> Vec<usize> {
    let mut columns: Vec<Column> = headers
        .iter()
        .map(|header| {
            let header = display_width(header).max(MIN_RULE);
            Column {
                width: header,
                header,
                word: 0,
                prose: false,
            }
        })
        .collect();
    for row in cells {
        for (column, text) in columns.iter_mut().zip(row) {
            column.width = column.width.max(display_width(text));
            column.prose |= text.contains(' ');
            let longest = text.split(' ').map(display_width).max().unwrap_or(0);
            column.word = column.word.max(longest.min(WORD_CAP));
        }
    }
    shrink(&mut columns, true);
    shrink(&mut columns, false);
    columns.iter().map(|column| column.width).collect()
}

fn line_width(columns: &[Column]) -> usize {
    columns.iter().map(|column| column.width).sum::<usize>() + 3 * columns.len() + 1
}

/// Shrink the widest eligible column one display column at a time until the line fits
/// or nothing can shrink. Prose columns go first.
fn shrink(columns: &mut [Column], keep_words: bool) {
    while line_width(columns) > MAX_LINE {
        let Some(index) = pick(columns, keep_words) else {
            return;
        };
        columns[index].width -= 1;
    }
}

fn pick(columns: &[Column], keep_words: bool) -> Option<usize> {
    let floor = |column: &Column| {
        let base = column.header.max(MIN_COLUMN);
        if keep_words {
            base.max(column.word)
        } else {
            base
        }
    };
    let eligible = |prose: bool| {
        columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.prose == prose && column.width > floor(column))
            .max_by_key(|(_, column)| column.width)
            .map(|(index, _)| index)
    };
    eligible(true).or_else(|| eligible(false))
}

fn push_rule(out: &mut String, widths: &[usize]) {
    out.push('|');
    for width in widths {
        out.push(' ');
        for _ in 0..*width {
            out.push('-');
        }
        out.push_str(" |");
    }
    out.push('\n');
}

fn push_row(out: &mut String, cells: &[String], widths: &[usize]) {
    let wrapped: Vec<Vec<String>> = cells
        .iter()
        .zip(widths)
        .map(|(text, width)| wrap(text, *width))
        .collect();
    let lines = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    for line in 0..lines {
        out.push('|');
        for (column, width) in widths.iter().enumerate() {
            let text = wrapped[column].get(line).map(String::as_str).unwrap_or("");
            out.push(' ');
            out.push_str(text);
            for _ in display_width(text)..*width {
                out.push(' ');
            }
            out.push_str(" |");
        }
        out.push('\n');
    }
}

/// Split `text` into lines of at most `width` display columns, preferring spaces.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    wrap(text, width)
}

/// Split `text` into lines of at most `width` display columns, preferring spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if display_width(text) <= width {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        place_word(&mut lines, &mut current, word, width);
    }
    lines.push(current);
    lines
}

fn place_word(lines: &mut Vec<String>, current: &mut String, word: &str, width: usize) {
    let used = display_width(current);
    let joined = usize::from(!current.is_empty());
    if used + joined + display_width(word) <= width {
        if joined == 1 {
            current.push(' ');
        }
        current.push_str(word);
        return;
    }
    if !current.is_empty() {
        lines.push(std::mem::take(current));
    }
    if display_width(word) <= width {
        current.push_str(word);
        return;
    }
    split_long(lines, current, word, width);
}

/// Hard-split a word wider than the column. An escaped pipe is never split.
fn split_long(lines: &mut Vec<String>, current: &mut String, word: &str, width: usize) {
    let mut chars = word.chars().peekable();
    while let Some(ch) = chars.next() {
        let mut unit = String::new();
        unit.push(ch);
        if ch == '\\' && chars.peek() == Some(&'|') {
            unit.push('|');
            chars.next();
        }
        let unit_width: usize = unit.chars().map(char_width).sum();
        if display_width(current) + unit_width > width {
            lines.push(std::mem::take(current));
        }
        current.push_str(&unit);
    }
}
