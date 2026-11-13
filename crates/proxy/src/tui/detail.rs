//! The detail panel: a clicked subterm, its type and its docstring.
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;

use super::{
    goals, markdown,
    rpc::{Held, Session},
    view::{HANG, Text, wrap},
};

/// Screen lines the panel takes at most, its top border included.
const MAX_HEIGHT: u16 = 8;

/// A clicked subterm and what Lean says about it.
pub(super) struct Picked {
    pub(super) label: String,
    pub(super) info: Value,
    /// The session that issued `info`, which the popup's references also live in.
    pub(super) session: Session,
    pub(super) popup: Option<Value>,
}

impl Picked {
    /// The popup and the session holding its references, once the panel closes.
    pub(super) fn into_popup(self) -> Option<Held> {
        Held::new(Some(self.session), self.popup)
    }

    /// The subterm, its type and its docstring, wrapped to `width` columns.
    pub(super) fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let popup = self.popup.as_ref();
        let expr = popup.map(|p| &p["exprExplicit"]).filter(|e| !e.is_null());
        let mut head = match expr {
            Some(expr) => {
                goals::typeset(Text::default(), expr, &[])
                    .into_iter()
                    .next()
                    .unwrap_or_default()
                    .line
            }
            None => Line::raw(self.label.clone()),
        };
        let lines: Vec<Line> = match popup.map(|p| &p["type"]).filter(|t| !t.is_null()) {
            Some(ty) => {
                head.spans.push(Span::raw(" : "));
                goals::typeset(head.into(), ty, &[])
                    .into_iter()
                    .map(|t| t.line)
                    .collect()
            }
            None => vec![head],
        };
        let doc = popup.and_then(|p| p["doc"].as_str()).unwrap_or_default();
        lines
            .into_iter()
            .map(|line| (line, HANG))
            .chain(markdown::lines(doc))
            .flat_map(|(line, hang)| wrap(line, width.into(), hang))
            .map(|(line, ..)| line)
            .collect()
    }
}

/// The height of a panel showing `lines`: nothing for no lines, else the
/// lines under a top border, up to `MAX_HEIGHT`.
pub(super) fn height(lines: &[Line]) -> u16 {
    if lines.is_empty() {
        return 0;
    }
    u16::try_from(lines.len() + 1)
        .unwrap_or(u16::MAX)
        .min(MAX_HEIGHT)
}

/// Draws `lines` under a top border, unless there are none.
pub(super) fn render(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    if lines.is_empty() {
        return;
    }
    let panel = Paragraph::new(lines).block(Block::new().borders(Borders::TOP));
    frame.render_widget(panel, area);
}
