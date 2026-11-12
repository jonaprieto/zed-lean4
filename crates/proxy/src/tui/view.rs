//! Foldable rows and line wrapping for the terminal infoview.
use std::collections::HashSet;

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

pub struct Row {
    pub line: Line<'static>,
    /// The innermost foldable block the row belongs to (its own key on a header).
    pub fold: Option<String>,
    pub header: bool,
}

/// Builds rows, leaving out the bodies of folded blocks.
pub struct Rows<'a> {
    pub rows: Vec<Row>,
    folded: &'a HashSet<String>,
    open: Vec<String>,
    /// Depth of the outermost folded block, while inside one.
    hidden: Option<usize>,
}

impl<'a> Rows<'a> {
    pub fn new(folded: &'a HashSet<String>) -> Self {
        Self {
            rows: Vec::new(),
            folded,
            open: Vec::new(),
            hidden: None,
        }
    }

    pub fn push(&mut self, line: Line<'static>) {
        if self.hidden.is_none() {
            let fold = self.open.last().cloned();
            self.rows.push(Row {
                line,
                fold,
                header: false,
            });
        }
    }

    /// Starts a block whose header is `label` with a fold marker.
    pub fn open(&mut self, key: String, label: Line<'static>) {
        if self.hidden.is_none() {
            let folded = self.folded.contains(&key);
            let mut line = label;
            line.spans
                .insert(0, Span::raw(if folded { "▸ " } else { "▾ " }));
            self.rows.push(Row {
                line,
                fold: Some(key.clone()),
                header: true,
            });
            if folded {
                self.hidden = Some(self.open.len() + 1);
            }
        }
        self.open.push(key);
    }

    pub fn close(&mut self) {
        if self.hidden == Some(self.open.len()) {
            self.hidden = None;
        }
        self.open.pop();
    }
}

pub const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

/// Wraps `line` to `width` columns, at a space when there is one, indenting
/// continuations four columns past the line's own indentation.
pub fn wrap(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    let lead = cells.iter().take_while(|(c, _)| *c == ' ').count();
    let indent = (lead + 4).min(width / 2);
    let mut out = Vec::new();
    let mut start = 0;
    while start < cells.len() {
        let avail = if out.is_empty() {
            width
        } else {
            width - indent
        };
        let (mut used, mut end) = (0, start);
        while end < cells.len() {
            let w = cells[end].0.width().unwrap_or(0);
            if used + w > avail {
                break;
            }
            used += w;
            end += 1;
        }
        if end < cells.len()
            && let Some(space) = (start + 1..end).rev().find(|&k| cells[k].0 == ' ')
        {
            end = space + 1;
        }
        end = end.max(start + 1);
        let mut spans = Vec::new();
        if !out.is_empty() {
            spans.push(Span::raw(" ".repeat(indent)));
        }
        for (c, style) in &cells[start..end] {
            match spans.last_mut() {
                Some(last) if last.style == *style => last.content.to_mut().push(*c),
                _ => spans.push(Span::styled(c.to_string(), *style)),
            }
        }
        out.push(Line::from(spans).style(line.style));
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn folded_blocks_keep_their_header_and_hide_their_body() {
        let folded = HashSet::from(["inner".to_owned()]);
        let mut rows = Rows::new(&folded);
        rows.open("outer".into(), Line::raw("Outer"));
        rows.push(Line::raw("a"));
        rows.open("inner".into(), Line::raw("Inner"));
        rows.push(Line::raw("hidden"));
        rows.open("deeper".into(), Line::raw("Deeper"));
        rows.close();
        rows.close();
        rows.push(Line::raw("b"));
        rows.close();
        rows.push(Line::raw("c"));
        let got: Vec<(String, Option<&str>, bool)> = rows
            .rows
            .iter()
            .map(|r| (plain(&r.line), r.fold.as_deref(), r.header))
            .collect();
        assert_eq!(
            got,
            [
                ("▾ Outer".into(), Some("outer"), true),
                ("a".into(), Some("outer"), false),
                ("▸ Inner".into(), Some("inner"), true),
                ("b".into(), Some("outer"), false),
                ("c".into(), None, false),
            ]
        );
    }

    #[test]
    fn wrap_breaks_at_spaces_with_a_hanging_indent() {
        let red = Style::new().fg(Color::Red);
        let line = Line::from(vec![
            Span::raw("  h : "),
            Span::styled("aaaa bbbb", red),
            Span::raw(" cccccccccccccc"),
        ]);
        let lines = wrap(line, 12);
        let text: Vec<String> = lines.iter().map(plain).collect();
        assert_eq!(
            text,
            [
                "  h : aaaa ",
                "      bbbb ",
                "      cccccc",
                "      cccccc",
                "      cc"
            ]
        );
        assert_eq!(lines[1].spans[1].style, red);
        assert_eq!(wrap(Line::raw("short"), 12).len(), 1);
    }
}
