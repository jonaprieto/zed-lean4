//! What the infoview shows: rows built from the app's state, laid out on screen.
use std::rc::Rc;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::Stylize,
    text::Line,
    widgets::Paragraph,
};
use serde_json::Value;

use super::App;
use crate::tui::{
    detail::{self, Picked},
    lean, messages,
    screen::{Screen, Wrapped},
    view::{self, Row, Rows},
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
                line.style = line.style.patch(view::SELECTED);
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
            rows.open("tactic".into(), Line::from("Tactic state".bold()));
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
            rows.open("term".into(), Line::from("Expected type".bold()));
            self.term_texts.iter().for_each(|l| rows.push(l.clone()));
            rows.close();
        }
        self.build_messages(rows, uri, cursor);
        if let Some(hover) = &self.hover {
            rows.push(Line::raw(""));
            rows.open("hover".into(), Line::from("At the cursor".bold()));
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
            Line::from(format!("{title} ({count})").bold()),
        );
        if let Some(list) = interactive {
            for (i, d) in list.iter().enumerate() {
                let at = &d["range"]["start"];
                let (line, character) = (
                    at["line"].as_u64().unwrap_or_default() + 1,
                    &at["character"],
                );
                let head = Line::raw(format!("{line}:{character} {}: ", lean::severity(d)));
                let key = format!("message:{uri}:{line}:{character}:{i}");
                messages::message(rows, &self.trace_children, &key, head, &d["message"]);
            }
            return rows.close();
        }
        for d in messages {
            let at = &d["range"]["start"];
            let mut text = d["message"].as_str().unwrap_or_default().lines().peekable();
            let first = format!(
                "{}:{} {}: {}",
                at["line"].as_u64().unwrap_or_default() + 1,
                at["character"],
                lean::severity(d),
                text.next().unwrap_or_default()
            );
            if text.peek().is_none() {
                rows.push(Line::raw(first));
                continue;
            }
            rows.open(format!("message:{uri}:{first}"), Line::raw(first));
            text.for_each(|l| rows.push(Line::raw(format!("  {l}"))));
            rows.close();
        }
        rows.close();
    }
}
