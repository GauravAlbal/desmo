//! Return rails: back-edges drawn as a vertical rail in a left gutter.
//!
//! Operates on already rendered river lines, so it only adds gutter columns
//! and rewrites the leading indent of the two rows a rail touches. The
//! right-column annotation that names the command and its target stays in
//! place, so every command label is still drawn exactly once.

use std::collections::BTreeMap;

/// Most rails drawn in one diagram; further back-edges keep their annotation.
const MAX_RAILS: usize = 2;

/// Longest horizontal run drawn from a rail into the river area; wider
/// padding before a right-column label stays blank.
const RUN_MAX: usize = 6;

/// A back-edge as rows of the rendered lines: the annotation row it leaves
/// from and the state row it returns to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    top: usize,
    bottom: usize,
}

impl Span {
    fn len(self) -> usize {
        self.bottom - self.top
    }

    fn strictly_inside(self, outer: Span) -> bool {
        outer.top < self.top && self.bottom < outer.bottom
    }

    fn disjoint(self, other: Span) -> bool {
        self.bottom < other.top || other.bottom < self.top
    }
}

/// Add return rails to `lines`. Returns the new lines and the gutter width
/// (0 when no rail is drawn and the lines are unchanged).
pub(crate) fn add_return_rails(lines: Vec<String>) -> (Vec<String>, usize) {
    let spans = accepted_spans(candidate_spans(&lines));
    if spans.is_empty() {
        return (lines, 0);
    }
    let columns = rail_columns(&spans);
    let width = 2 * (columns.iter().copied().max().unwrap_or(0) + 1) + 1;
    let drawn = lines
        .iter()
        .enumerate()
        .map(|(row, line)| draw_row(row, line, &spans, &columns, width))
        .collect();
    (drawn, width)
}

fn candidate_spans(lines: &[String]) -> Vec<Span> {
    let rows = state_rows(lines);
    lines
        .iter()
        .enumerate()
        .filter_map(|(row, line)| {
            let target = annotation_target(line)?;
            let top = *rows.get(target)?;
            (top < row).then_some(Span { top, bottom: row })
        })
        .collect()
}

/// First row on which each state is drawn.
fn state_rows(lines: &[String]) -> BTreeMap<&str, usize> {
    let mut rows = BTreeMap::new();
    for (row, line) in lines.iter().enumerate() {
        if let Some(name) = state_name(line) {
            rows.entry(name).or_insert(row);
        }
    }
    rows
}

fn state_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("  ")?;
    let mut chars = rest.chars();
    let glyph = chars.next()?;
    if !matches!(glyph, '●' | '○' | '✔') {
        return None;
    }
    let after = chars.as_str().strip_prefix(" [ ")?;
    let end = after.find(" ]")?;
    Some(&after[..end])
}

/// Target state of a back-edge annotation `↗ cmd → [ state ]` or `↺ ...`.
fn annotation_target(line: &str) -> Option<&str> {
    let body = line.trim_start();
    if !(body.starts_with('↗') || body.starts_with('↺')) {
        return None;
    }
    let after = body.split_once("→ [ ")?.1;
    let end = after.find(" ]")?;
    Some(&after[..end])
}

/// Keep the shortest spans first; skip any that share an endpoint row with,
/// partially overlap, or would exceed the rail budget of an accepted span.
fn accepted_spans(mut candidates: Vec<Span>) -> Vec<Span> {
    candidates.sort_by_key(|span| (span.len(), span.top, span.bottom));
    let mut accepted: Vec<Span> = Vec::new();
    for span in candidates {
        if accepted.len() == MAX_RAILS {
            break;
        }
        if fits(span, &accepted) {
            accepted.push(span);
        }
    }
    accepted.sort_by_key(|span| (span.top, span.bottom));
    accepted
}

fn fits(span: Span, accepted: &[Span]) -> bool {
    accepted.iter().all(|other| compatible(span, *other))
}

fn compatible(a: Span, b: Span) -> bool {
    a.disjoint(b) || nested(a, b)
}

fn nested(a: Span, b: Span) -> bool {
    a.strictly_inside(b) || b.strictly_inside(a)
}

/// Gutter column of each span: 0 is next to the river; a span that encloses
/// another sits one column further out.
fn rail_columns(spans: &[Span]) -> Vec<usize> {
    spans
        .iter()
        .map(|span| {
            spans
                .iter()
                .filter(|inner| inner.strictly_inside(*span))
                .count()
                .min(MAX_RAILS - 1)
        })
        .collect()
}

fn draw_row(row: usize, line: &str, spans: &[Span], columns: &[usize], width: usize) -> String {
    let mut gutter = vec![' '; width];
    let mut line = line.to_string();
    for (span, column) in spans.iter().zip(columns) {
        let x = width - 2 - 2 * column;
        if row == span.top {
            mark_end(&mut gutter, x, '┌');
            line = replace_indent(&line, 2, "─▶");
        } else if row == span.bottom {
            mark_end(&mut gutter, x, '└');
            let lead = line.chars().take_while(|c| *c == ' ').count().min(RUN_MAX);
            line = replace_indent(&line, lead, &"─".repeat(lead));
        } else if span.top < row && row < span.bottom && gutter[x] == ' ' {
            gutter[x] = '│';
        }
    }
    format!("{}{line}", gutter.iter().collect::<String>())
}

fn mark_end(gutter: &mut [char], x: usize, corner: char) {
    gutter[x] = corner;
    for cell in gutter.iter_mut().skip(x + 1) {
        *cell = '─';
    }
}

/// Replace the first `count` characters of `line` with `with`.
fn replace_indent(line: &str, count: usize, with: &str) -> String {
    let rest: String = line.chars().skip(count).collect();
    format!("{with}{rest}")
}
