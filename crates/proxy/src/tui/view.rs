//! Foldable rows and line wrapping for the terminal infoview.
use std::collections::HashSet;

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

/// Characters `start..end` of a line show the subterm Lean knows as `info`.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub info: Value,
}

/// A line and the subterms in it.
#[derive(Clone, Default)]
pub struct Text {
    pub line: Line<'static>,
    pub links: Vec<Link>,
}

impl From<Line<'static>> for Text {
    fn from(line: Line<'static>) -> Self {
        Self {
            line,
            links: Vec::new(),
        }
    }
}

pub struct Row {
    pub line: Line<'static>,
    pub links: Vec<Link>,
    /// The innermost foldable block the row belongs to (its own key on a header).
    pub fold: Option<String>,
    pub header: bool,
    /// On a header: whether its body shows.
    pub open: bool,
    /// On a header whose children Lean sends on demand: the reference to ask for them.
    pub load: Option<Value>,
    /// Columns that wrapped continuations sit past the row's own indentation.
    pub hang: usize,
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

    pub fn push(&mut self, text: impl Into<Text>) {
        self.push_hanging(text, HANG);
    }

    /// Pushes a row whose wrapped continuations hang `hang` columns in.
    pub fn push_hanging(&mut self, text: impl Into<Text>, hang: usize) {
        if self.hidden.is_none() {
            let Text { line, links } = text.into();
            let fold = self.open.last().cloned();
            self.rows.push(Row {
                line,
                links,
                fold,
                header: false,
                open: false,
                load: None,
                hang,
            });
        }
    }

    /// Starts a block whose header is `label` with a fold marker.
    pub fn open(&mut self, key: String, label: impl Into<Text>) {
        self.open_as(key, label, false);
    }

    /// Starts a block that is folded by default when `collapsed`; toggling
    /// its key flips that default.
    pub fn open_as(&mut self, key: String, label: impl Into<Text>, collapsed: bool) {
        if self.hidden.is_none() {
            let folded = collapsed != self.folded.contains(&key);
            let Text {
                mut line,
                mut links,
            } = label.into();
            line.spans
                .insert(0, Span::raw(if folded { "▸ " } else { "▾ " }));
            for link in &mut links {
                (link.start, link.end) = (link.start + MARKER_WIDTH, link.end + MARKER_WIDTH);
            }
            self.rows.push(Row {
                line,
                links,
                fold: Some(key.clone()),
                header: true,
                open: !folded,
                load: None,
                hang: HANG,
            });
            if folded {
                self.hidden = Some(self.open.len() + 1);
            }
        }
        self.open.push(key);
    }

    /// Marks the current block's header as needing `children` from Lean. The
    /// header may be followed by its own continuation lines.
    pub fn load(&mut self, children: Value) {
        let key = self.open.last();
        if let Some(row) = self
            .rows
            .iter_mut()
            .rev()
            .find(|r| r.header && r.fold.as_ref() == key)
        {
            row.load = Some(children);
        }
    }

    pub fn close(&mut self) {
        if self.hidden == Some(self.open.len()) {
            self.hidden = None;
        }
        self.open.pop();
    }
}

/// How far wrapped continuations of goals and messages hang past their line.
pub const HANG: usize = 4;

/// Characters the `▾ `/`▸ ` fold marker adds before a header.
pub const MARKER_WIDTH: usize = 2;

/// The character of `line` under column `col`, counting from character `start`.
pub fn char_at(line: &Line, start: usize, col: usize) -> Option<usize> {
    let mut used = 0;
    for (i, c) in line
        .spans
        .iter()
        .flat_map(|s| s.content.chars())
        .enumerate()
        .skip(start)
    {
        used += c.width().unwrap_or(0);
        if used > col {
            return Some(i);
        }
    }
    None
}

/// A screen line: its text, the index of its first character in the row's
/// line, and how many columns of indentation `wrap` put before it.
pub type Wrapped = (Line<'static>, usize, usize);

/// Wraps `line` to `width` columns, at a space when there is one, indenting
/// continuations `hang` columns past the line's own indentation.
pub fn wrap(line: Line<'static>, width: usize, hang: usize) -> Vec<Wrapped> {
    if width == 0 || line.width() <= width {
        return vec![(line, 0, 0)];
    }
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    let lead = cells.iter().take_while(|(c, _)| *c == ' ').count();
    let indent = (lead + hang).min(width / 2);
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
        let pre = if out.is_empty() { 0 } else { indent };
        if pre > 0 {
            spans.push(Span::raw(" ".repeat(pre)));
        }
        for (c, style) in &cells[start..end] {
            match spans.last_mut() {
                Some(last) if last.style == *style => last.content.to_mut().push(*c),
                _ => spans.push(Span::styled(c.to_string(), *style)),
            }
        }
        out.push((Line::from(spans).style(line.style), start, pre));
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
        let wrapped = wrap(line.clone(), 12, HANG);
        let starts: Vec<(usize, usize)> = wrapped.iter().map(|w| (w.1, w.2)).collect();
        assert_eq!(starts, [(0, 0), (11, 6), (16, 6), (22, 6), (28, 6)]);
        assert_eq!(char_at(&line, 11, 0), Some(11));
        assert_eq!(char_at(&line, 0, 7), Some(7));
        assert_eq!(char_at(&line, 28, 5), None);
        let lines: Vec<Line> = wrapped.into_iter().map(|w| w.0).collect();
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
        assert_eq!(wrap(Line::raw("short"), 12, HANG).len(), 1);
    }
}
