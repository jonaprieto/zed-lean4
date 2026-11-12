//! Lean's hover at the cursor (`textDocument/hover`) as terminal lines.
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::{goals, theme, view::HANG};

/// Markdown lines that separate the signature from the docstring.
const RULES: &[&str] = &["***", "---", "___"];

/// Bullets that start a list item.
const BULLETS: &[&str] = &["* ", "- ", "+ "];

/// A hover line and how far its wrapped continuations hang.
pub type HoverLine = (Line<'static>, usize);

/// The hover's markdown as lines: fenced code highlighted like goals, rules as
/// blank lines, prose reflowed into paragraphs and list items so the terminal
/// wraps them instead of the docstring's own line breaks. `None` when Lean has
/// nothing to say.
pub fn lines(result: &Value) -> Option<Vec<HoverLine>> {
    let markdown = match &result["contents"] {
        Value::String(s) => s.as_str(),
        contents => contents["value"].as_str()?,
    };
    let mut out = Prose::default();
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            out.flush();
            in_code = !in_code;
        } else if in_code {
            out.lines.push((goals::code_line(line), HANG));
        } else if trimmed.is_empty() || RULES.contains(&trimmed) {
            out.flush();
            out.lines.push((Line::default(), 0));
        } else if let Some(marker) = list_marker(trimmed) {
            out.flush();
            out.paragraph = Some((line.trim_end().to_owned(), marker));
        } else if trimmed.starts_with('#') {
            out.flush();
            out.paragraph = Some((trimmed.to_owned(), 0));
            out.flush();
        } else if let Some((text, _)) = &mut out.paragraph {
            text.push(' ');
            text.push_str(trimmed);
        } else {
            out.paragraph = Some((trimmed.to_owned(), 0));
        }
    }
    out.flush();
    (!out.lines.is_empty()).then_some(out.lines)
}

/// Lines so far and the paragraph being joined, with its hanging indent.
#[derive(Default)]
struct Prose {
    lines: Vec<HoverLine>,
    paragraph: Option<(String, usize)>,
}

impl Prose {
    fn flush(&mut self) {
        if let Some((text, hang)) = self.paragraph.take() {
            self.lines
                .push((Line::from(Span::styled(text, theme::current().docs)), hang));
        }
    }
}

/// Width of the bullet or number that starts a list item, if `s` starts one.
fn list_marker(s: &str) -> Option<usize> {
    if BULLETS.iter().any(|b| s.starts_with(b)) {
        return Some(2);
    }
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    (digits > 0 && s[digits..].starts_with(". ")).then_some(digits + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plain(lines: &[HoverLine]) -> Vec<String> {
        lines
            .iter()
            .map(|(l, _)| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn signature_then_docstring() {
        let hover = json!({"contents":{"kind":"markdown",
            "value":"```lean\nNat.add_zero : ∀ (n : Nat), n + 0 = n\n```\n***\nAdding zero changes nothing."}});
        let lines = lines(&hover).unwrap();
        assert_eq!(
            plain(&lines),
            [
                "Nat.add_zero : ∀ (n : Nat), n + 0 = n",
                "",
                "Adding zero changes nothing."
            ]
        );
        assert!(lines[0].0.spans.len() > 1, "code is highlighted");
    }

    #[test]
    fn docstring_line_breaks_reflow_into_paragraphs_and_items() {
        let doc = "`namespace <id>` opens a section that\ninfluences naming:\n* Names are prefixed: `def x`\n  is given the full name.\n12. Numbered\n\nAs with `section`, they nest.";
        let lines = lines(&json!({"contents":{"kind":"markdown","value":doc}})).unwrap();
        assert_eq!(
            plain(&lines),
            [
                "`namespace <id>` opens a section that influences naming:",
                "* Names are prefixed: `def x` is given the full name.",
                "12. Numbered",
                "",
                "As with `section`, they nest.",
            ]
        );
        let hangs: Vec<usize> = lines.iter().map(|(_, h)| *h).collect();
        assert_eq!(hangs, [0, 2, 4, 0, 0]);
    }

    #[test]
    fn nothing_to_show() {
        assert!(lines(&Value::Null).is_none());
        assert!(lines(&json!({"contents":{"kind":"markdown","value":""}})).is_none());
        assert_eq!(
            plain(&lines(&json!({"contents":"plain"})).unwrap()),
            ["plain"]
        );
    }
}
