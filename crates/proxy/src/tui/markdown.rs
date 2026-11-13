//! Markdown, as in Lean docstrings, as terminal lines.
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::{goals, theme, view::HANG};

/// What the view draws for a bullet list item.
const BULLET: &str = "• ";

/// A line and how far its wrapped continuations hang.
pub type Hanging = (Line<'static>, usize);

/// Markdown as terminal lines: paragraphs and list items as single lines the
/// view wraps (not the docstring's own line breaks), code highlighted like
/// goals, emphasis and links styled, a blank line between blocks.
pub fn lines(text: &str) -> Vec<Hanging> {
    let mut writer = Writer::default();
    Parser::new(text).for_each(|event| writer.event(event));
    writer.lines
}

/// An open list: the next item's number (`None` for bullets) and the column
/// its items start at.
struct List {
    next: Option<u64>,
    column: usize,
}

/// Turns parser events into lines.
#[derive(Default)]
struct Writer {
    lines: Vec<Hanging>,
    /// The line being written and how far its continuations hang.
    line: Vec<Span<'static>>,
    hang: usize,
    /// Styles of the emphasis, strong and link elements around the text.
    overlays: Vec<Style>,
    lists: Vec<List>,
    /// Column where the text of the innermost list item starts.
    indent: usize,
    heading: bool,
    /// The text of the code block being read, highlighted when it ends.
    code_block: Option<String>,
    /// A block ended, so the next one starts after a blank line.
    gap: bool,
}

impl Writer {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match &mut self.code_block {
                Some(code) => code.push_str(&text),
                None => self.prose(&text),
            },
            Event::Code(code) => {
                self.begin_text();
                let overlay = self.overlay();
                for span in goals::code_line(&code).spans {
                    self.push(span.patch_style(overlay));
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => self.prose(&html),
            Event::SoftBreak => self.prose(" "),
            Event::HardBreak => self.finish(),
            Event::Rule => self.end_block(),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { .. } => self.heading = true,
            Tag::CodeBlock(_) => self.code_block = Some(String::new()),
            Tag::List(start) => self.lists.push(List {
                next: start,
                column: self.indent,
            }),
            Tag::Item => {
                self.finish();
                let Some(list) = self.lists.last_mut() else {
                    return;
                };
                let marker = match &mut list.next {
                    Some(n) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    None => BULLET.to_owned(),
                };
                let column = list.column;
                let width = marker.chars().count();
                self.begin_line(format!("{}{marker}", " ".repeat(column)), width);
                self.indent = column + width;
            }
            Tag::Emphasis => self
                .overlays
                .push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .overlays
                .push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Link { .. } => self.overlays.push(theme::current().link),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_block(),
            TagEnd::Heading(_) => {
                self.end_block();
                self.heading = false;
            }
            TagEnd::CodeBlock => {
                let code = self.code_block.take().unwrap_or_default();
                for line in code.lines() {
                    self.begin_line(" ".repeat(self.indent), HANG);
                    self.line.extend(goals::code_line(line).spans);
                    self.finish();
                }
                self.gap = true;
            }
            TagEnd::Item => self.finish(),
            TagEnd::List(_) => {
                if let Some(list) = self.lists.pop() {
                    self.indent = list.column;
                }
                self.gap = true;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Link => {
                self.overlays.pop();
            }
            _ => {}
        }
    }

    /// Plain text in the style of where it stands.
    fn prose(&mut self, text: &str) {
        self.begin_text();
        let theme = theme::current();
        let base = if self.heading {
            theme.heading
        } else {
            theme.docs
        };
        let style = base.patch(self.overlay());
        self.push(Span::styled(text.to_owned(), style));
    }

    fn overlay(&self) -> Style {
        self.overlays
            .iter()
            .fold(Style::new(), |all, style| all.patch(*style))
    }

    /// Starts a line for text unless one is open: indented under the list
    /// item it belongs to.
    fn begin_text(&mut self) {
        if self.line.is_empty() {
            self.begin_line(" ".repeat(self.indent), 0);
        }
    }

    /// Starts a line with `lead`, after a blank line if a block just ended.
    fn begin_line(&mut self, lead: String, hang: usize) {
        if std::mem::take(&mut self.gap) && !self.lines.is_empty() {
            self.lines.push((Line::default(), 0));
        }
        self.hang = hang;
        if !lead.is_empty() {
            self.line.push(Span::raw(lead));
        }
    }

    /// Adds `span`, merged into the last one when they look the same.
    fn push(&mut self, span: Span<'static>) {
        match self.line.last_mut() {
            Some(last) if last.style == span.style => {
                last.content.to_mut().push_str(&span.content);
            }
            _ => self.line.push(span),
        }
    }

    fn finish(&mut self) {
        if !self.line.is_empty() {
            let line = Line::from(std::mem::take(&mut self.line));
            self.lines.push((line, self.hang));
        }
    }

    fn end_block(&mut self) {
        self.finish();
        self.gap = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Hanging]) -> Vec<String> {
        lines.iter().map(|(line, _)| line.to_string()).collect()
    }

    #[test]
    fn docstring_line_breaks_reflow_into_paragraphs_and_items() {
        let doc = "`namespace <id>` opens a section that\ninfluences naming:\n* Names are prefixed: `def x`\n  is given the full name.\n  1. Nested\n  2. and numbered\n\nAs with `section`, they nest.";
        let lines = lines(doc);
        assert_eq!(
            plain(&lines),
            [
                "namespace <id> opens a section that influences naming:",
                "",
                "• Names are prefixed: def x is given the full name.",
                "  1. Nested",
                "  2. and numbered",
                "",
                "As with section, they nest.",
            ]
        );
        let hangs: Vec<usize> = lines.iter().map(|(_, h)| *h).collect();
        assert_eq!(hangs, [0, 0, 2, 3, 3, 0, 0]);
    }

    #[test]
    fn inline_markdown_styles_code_links_and_emphasis() {
        let doc = "A list of `α`. See [Inductive\ntypes](https://lean-lang.org/x.html) for **more** *information*.\n# Notes";
        let lines = lines(doc);
        assert_eq!(
            plain(&lines),
            [
                "A list of α. See Inductive types for more information.",
                "",
                "Notes"
            ]
        );
        let theme = theme::current();
        let styles: Vec<(&str, Style)> = lines[0]
            .0
            .spans
            .iter()
            .map(|s| (s.content.as_ref(), s.style))
            .collect();
        assert_eq!(
            styles,
            [
                ("A list of ", theme.docs),
                ("α", Style::new()),
                (". See ", theme.docs),
                ("Inductive types", theme.docs.patch(theme.link)),
                (" for ", theme.docs),
                ("more", theme.docs.add_modifier(Modifier::BOLD)),
                (" ", theme.docs),
                ("information", theme.docs.add_modifier(Modifier::ITALIC)),
                (".", theme.docs),
            ]
        );
        assert_eq!(lines[2].0.spans[0].style, theme.heading);
        let names = super::lines("a * b * c, Nat.add_zero_right");
        assert_eq!(plain(&names), ["a * b * c, Nat.add_zero_right"]);
    }
}
