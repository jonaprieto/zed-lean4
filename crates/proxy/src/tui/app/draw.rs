//! What the infoview shows: rows built from the app's state, laid out on screen.
use std::rc::Rc;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::Paragraph,
};
use serde_json::Value;

use super::App;
use crate::tui::{
    detail::{self, Picked},
    lean, messages,
    screen::{Screen, Wrapped},
    theme,
    view::{Row, Rows},
};

pub(super) const KEYS: &str = "q quit  p pin  a all  c copy  o browser  j/k move  ⏎ fold";
pub(super) const PICKED_KEYS: &str = "d definition  t type  esc close";

impl App {
    pub fn render(&self, frame: &mut Frame) {
        let detail = self.picked.as_ref().map_or(vec![], Picked::lines);
        let [body, panel, footer] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(detail::height(&detail)),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        let theme = theme::current();
        let wrapped = self.wrapped(body.width);
        let selected = wrapped.lines_of(self.selected);
        let height = usize::from(body.height);
        let mut scroll = self
            .scroll
            .get()
            .min(wrapped.lines.len().saturating_sub(height));
        if self.follow {
            scroll = scroll
                .max(selected.end.saturating_sub(height))
                .min(selected.start);
        }
        self.scroll.set(scroll);
        // Only the visible slice is drawn.
        let end = (scroll + height).min(wrapped.lines.len());
        let mut lines = wrapped.lines[scroll..end].to_vec();
        for (n, line) in lines.iter_mut().enumerate() {
            if selected.contains(&(scroll + n)) {
                line.style = line.style.patch(theme.selection);
            }
        }
        *self.screen.borrow_mut() = Screen {
            area: body,
            lines: wrapped.segments[scroll..end].to_vec(),
        };
        frame.render_widget(Paragraph::new(lines), body);
        detail::render(frame, panel, detail);
        let keys = if self.picked.is_some() {
            PICKED_KEYS
        } else {
            KEYS
        };
        frame.render_widget(Line::raw(self.status.as_deref().unwrap_or(keys)), footer);
    }

    pub(super) fn rows(&self) -> Rc<Vec<Row>> {
        self.cache.rows(|| {
            let mut rows = Rows::new(&self.folded);
            self.build_rows(&mut rows);
            rows.rows
        })
    }

    fn wrapped(&self, width: u16) -> Rc<Wrapped> {
        self.cache.wrapped(width, || self.rows())
    }

    /// The location, then its goals, expected type and messages.
    fn build_rows(&self, rows: &mut Rows) {
        let Some(location) = &self.location else {
            return rows.push(Line::raw("No Lean file at the cursor"));
        };
        let uri = location["uri"].as_str().unwrap_or_default();
        let start = &location["range"]["start"];
        let cursor = start["line"].as_u64().unwrap_or_default();
        let pinned = if self.pinned { " [pinned]" } else { "" };
        rows.push(Line::raw(format!(
            "{}:{}:{}{pinned}",
            uri.rsplit('/').next().unwrap_or(uri),
            cursor + 1,
            start["character"]
        )));
        if self.goals.is_some() {
            let goals = self.goal_list();
            rows.push(Line::raw(""));
            rows.open(
                "tactic".into(),
                Line::styled("Tactic state", theme::current().heading),
            );
            match goals.len() {
                0 => rows.push(Line::raw("No goals")),
                1 => rows.push(Line::raw("1 goal")),
                n => rows.push(Line::raw(format!("{n} goals"))),
            }
            for (i, (goal, texts)) in goals.iter().zip(&self.goal_texts).enumerate() {
                if i > 0 {
                    rows.push(Line::raw(""));
                }
                let mut lines = texts.iter().cloned();
                // A named goal folds under its `case` line; several unnamed ones get a number.
                let case = goal["userName"].as_str();
                let header = match case {
                    Some(_) => lines.next(),
                    None if goals.len() > 1 => Some(Line::raw(format!("goal {}", i + 1)).into()),
                    None => None,
                };
                if let Some(header) = header {
                    rows.open(
                        format!("goal:{}", case.map_or(i.to_string(), String::from)),
                        header,
                    );
                    lines.for_each(|l| rows.push(l));
                    rows.close();
                } else {
                    lines.for_each(|l| rows.push(l));
                }
            }
            rows.close();
        }
        if self.term_goal.is_some() {
            rows.push(Line::raw(""));
            rows.open(
                "term".into(),
                Line::styled("Expected type", theme::current().heading),
            );
            self.term_texts.iter().for_each(|l| rows.push(l.clone()));
            rows.close();
        }
        self.build_messages(rows, uri, cursor);
        if let Some(hover) = &self.hover {
            rows.push(Line::raw(""));
            rows.open(
                "hover".into(),
                Line::styled("At the cursor", theme::current().heading),
            );
            hover
                .iter()
                .for_each(|(l, hang)| rows.push_hanging(l.clone(), *hang));
            rows.close();
        }
    }

    /// Messages on the cursor's line, or all of the file's with `a`.
    fn build_messages(&self, rows: &mut Rows, uri: &str, cursor: u64) {
        let messages: Vec<&Value> = self
            .diagnostics
            .get(uri)
            .into_iter()
            .flatten()
            .filter(|d| {
                let range = &d["range"];
                self.all_messages
                    || (range["start"]["line"].as_u64() <= Some(cursor)
                        && Some(cursor) <= range["end"]["line"].as_u64())
            })
            .collect();
        let title = if self.all_messages {
            "All messages"
        } else {
            "Messages"
        };
        // At the cursor, Lean's interactive messages replace the plain ones.
        let interactive = self.messages.as_ref().filter(|_| !self.all_messages);
        let interactive = interactive.and_then(|h| h.value.as_array());
        let count = interactive.map_or(messages.len(), Vec::len);
        rows.push(Line::raw(""));
        rows.open(
            "messages".into(),
            Line::styled(format!("{title} ({count})"), theme::current().heading),
        );
        if let Some(list) = interactive {
            for (i, d) in list.iter().enumerate() {
                let (position, head) = message_head(d);
                let key = format!("message:{uri}:{position}:{i}");
                messages::message(rows, &self.trace_children, &key, head, &d["message"]);
            }
            return rows.close();
        }
        for d in messages {
            let mut text = d["message"].as_str().unwrap_or_default().lines().peekable();
            let (position, mut first) = message_head(d);
            let summary = text.next().unwrap_or_default();
            first.spans.push(Span::raw(summary.to_owned()));
            if text.peek().is_none() {
                rows.push(first);
                continue;
            }
            let severity = lean::Severity::of(d).name();
            let key = format!("message:{uri}:{position} {severity}: {summary}");
            rows.open(key, first);
            text.for_each(|l| rows.push(Line::raw(format!("  {l}"))));
            rows.close();
        }
        rows.close();
    }
}

/// A diagnostic's `line:character`, and the head of its row: that position
/// and its severity, colored by the theme.
fn message_head(d: &Value) -> (String, Line<'static>) {
    let at = &d["range"]["start"];
    let position = format!(
        "{}:{}",
        at["line"].as_u64().unwrap_or_default() + 1,
        at["character"]
    );
    let severity = lean::Severity::of(d);
    let head = Line::from(vec![
        Span::raw(format!("{position} ")),
        Span::styled(severity.name(), theme::current().severity(severity)),
        Span::raw(": "),
    ]);
    (position, head)
}
