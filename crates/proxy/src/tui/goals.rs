//! Lean's interactive goals (`Lean.Widget.InteractiveGoal`, `TaggedText`) as styled terminal lines.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use super::view::{Link, Text};

const HYP: Style = Style::new().fg(Color::Cyan);
const PREFIX: Style = Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD);
const INACCESSIBLE: Style = Style::new().add_modifier(Modifier::DIM);
const INSERTED: Style = Style::new().fg(Color::Black).bg(Color::Green);
const REMOVED: Style = Style::new().fg(Color::Black).bg(Color::Red);
const KEYWORD: Style = Style::new().fg(Color::Magenta);
const LITERAL: Style = Style::new().fg(Color::Green);

const KEYWORDS: &[&str] = &[
    "fun", "λ", "let", "have", "show", "from", "by", "do", "match", "with", "if", "then", "else",
    "at", "in", "Type", "Sort", "Prop", "forall", "exists", "∀", "∃", "∃!", "Π", "Σ",
];

/// The plain text of a `TaggedText`.
pub fn text(t: &Value) -> String {
    let mut out = String::new();
    runs(t, Style::new(), None, &mut |s, _, _| out.push_str(s));
    out
}

/// The key of an RPC reference: `{"__rpcref": ..}` in wire format v1, `{"p": ..}` before.
pub fn ref_key(v: &Value) -> Option<&str> {
    let m = v.as_object().filter(|m| m.len() == 1)?;
    m.get("__rpcref").or_else(|| m.get("p"))?.as_str()
}

/// Whether `v` contains the reference `r`.
pub fn holds(v: &Value, r: &Value) -> bool {
    v == r
        || match v {
            Value::Object(m) => m.values().any(|v| holds(v, r)),
            Value::Array(a) => a.iter().any(|v| holds(v, r)),
            _ => false,
        }
}

/// Every RPC reference inside `v`, so the session can release them.
pub fn refs(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Object(_) if ref_key(v).is_some() => out.push(v.clone()),
        Value::Object(m) => m.values().for_each(|v| refs(v, out)),
        Value::Array(a) => a.iter().for_each(|v| refs(v, out)),
        _ => {}
    }
}

/// The goal as the VS Code infoview copies it: `case`, hypotheses, then `⊢ type`.
pub fn goal_text(goal: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(case) = goal["userName"].as_str() {
        lines.push(format!("case {case}"));
    }
    for hyp in goal["hyps"].as_array().into_iter().flatten() {
        let mut line = format!("{} : {}", names(hyp).join(" "), text(&hyp["type"]));
        if !hyp["val"].is_null() {
            line += &format!(" := {}", text(&hyp["val"]));
        }
        lines.push(line);
    }
    lines.push(format!("{}{}", prefix(goal), text(&goal["type"])));
    lines.join("\n")
}

/// The goal's lines: case name, hypotheses and the target, colored like the infoview.
pub fn goal_lines(goal: &Value) -> Vec<Text> {
    let mut out = Vec::new();
    let locals: Vec<&str> = goal["hyps"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(names)
        .collect();
    if let Some(case) = goal["userName"].as_str() {
        out.push(Text::from(Line::from(vec![
            Span::styled("case ", INACCESSIBLE),
            Span::styled(case.to_owned(), Style::new().add_modifier(Modifier::BOLD)),
        ])));
    }
    for hyp in goal["hyps"].as_array().into_iter().flatten() {
        let mark = marked(hyp);
        let mut head = Vec::new();
        for (i, name) in names(hyp).into_iter().enumerate() {
            if i > 0 {
                head.push(Span::raw(" "));
            }
            let style = if name.contains('✝') {
                INACCESSIBLE
            } else {
                HYP
            };
            head.push(Span::styled(name.to_owned(), style.patch(mark)));
        }
        head.push(Span::raw(" : "));
        let mut lines = typeset(Line::from(head).into(), &hyp["type"], &locals);
        if !hyp["val"].is_null() {
            let mut last = lines.pop().unwrap_or_default();
            last.line.spans.push(Span::raw(" := "));
            lines.extend(typeset(last, &hyp["val"], &locals));
        }
        out.extend(lines);
    }
    let target = Line::from(Span::styled(prefix(goal), PREFIX.patch(marked(goal))));
    out.extend(typeset(target.into(), &goal["type"], &locals));
    out
}

fn names(hyp: &Value) -> Vec<&str> {
    hyp["names"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|n| !n.contains("[anonymous]"))
        .collect()
}

fn prefix(goal: &Value) -> String {
    goal["goalPrefix"].as_str().unwrap_or("⊢ ").to_owned()
}

/// Background for a hypothesis or goal the tactic inserted or removed.
fn marked(v: &Value) -> Style {
    if v["isInserted"] == true {
        INSERTED
    } else if v["isRemoved"] == true {
        REMOVED
    } else {
        Style::new()
    }
}

/// `head` followed by `t`, split into lines at the pretty printer's newlines,
/// with a link for each run inside a tagged subterm.
/// Text outside a diff is highlighted, with the goal's `locals` colored like hypotheses.
pub fn typeset(head: Text, t: &Value, locals: &[&str]) -> Vec<Text> {
    let mut col: usize = head
        .line
        .spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum();
    let mut lines = vec![head];
    runs(t, Style::new(), None, &mut |s, style, info| {
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                lines.push(Text::default());
                col = 0;
            }
            let text = lines.last_mut().expect("starts with the head");
            if style == Style::new() {
                highlight(part, locals, &mut text.line.spans);
            } else if !part.is_empty() {
                text.line.spans.push(Span::styled(part.to_owned(), style));
            }
            let start = col;
            col += part.chars().count();
            if let Some(info) = info.filter(|_| col > start) {
                text.links.push(Link {
                    start,
                    end: col,
                    info: info.clone(),
                });
            }
        }
    });
    lines
}

/// One line of Lean code, highlighted like goal text.
pub fn code_line(s: &str) -> Line<'static> {
    let mut spans = Vec::new();
    highlight(s, &[], &mut spans);
    Line::from(spans)
}

/// Lean tokens of `s` as spans: keywords, number and string literals, and locals.
// ponytail: a lexer, not a parser; bound variables (`fun x =>`) stay uncolored.
fn highlight(s: &str, locals: &[&str], out: &mut Vec<Span<'static>>) {
    let chars: Vec<char> = s.chars().collect();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        let style = if c.is_alphabetic() || c == '_' || c == '«' {
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                let dot = d == '.' && chars.get(i + 1).is_some_and(|n| n.is_alphabetic());
                if d.is_alphanumeric() || "_'!?✝»".contains(d) || dot {
                    i += 1;
                } else {
                    break;
                }
            }
            let word: String = chars[start..i].iter().collect();
            if KEYWORDS.contains(&word.as_str()) {
                Some(KEYWORD)
            } else if locals.contains(&word.as_str()) {
                Some(HYP)
            } else {
                None
            }
        } else if c.is_ascii_digit() {
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
            {
                i += 1;
            }
            Some(LITERAL)
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            Some(LITERAL)
        } else if c == '∀' || c == '∃' {
            i += if chars.get(i + 1) == Some(&'!') { 2 } else { 1 };
            Some(KEYWORD)
        } else {
            i += 1;
            None
        };
        let token: String = chars[start..i].iter().collect();
        match style {
            Some(style) => {
                if !plain.is_empty() {
                    out.push(Span::raw(std::mem::take(&mut plain)));
                }
                out.push(Span::styled(token, style));
            }
            None => plain.push_str(&token),
        }
    }
    if !plain.is_empty() {
        out.push(Span::raw(plain));
    }
}

/// Calls `f` with each text run of `t`, the diff style and the info of its innermost tag.
fn runs<'v>(
    t: &'v Value,
    style: Style,
    info: Option<&'v Value>,
    f: &mut impl FnMut(&str, Style, Option<&'v Value>),
) {
    if let Some(s) = t["text"].as_str() {
        f(s, style, info);
    } else if let Some(parts) = t["append"].as_array() {
        parts.iter().for_each(|p| runs(p, style, info, f));
    } else if let Some([tag, inner]) = t["tag"].as_array().map(Vec::as_slice) {
        let style = match tag["diffStatus"].as_str() {
            Some("wasChanged" | "wasInserted" | "willInsert") => INSERTED,
            Some("willChange" | "willDelete" | "wasDeleted") => REMOVED,
            _ => style,
        };
        let info = tag.get("info").or(info);
        runs(inner, style, info, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tag(info: Value, inner: Value) -> Value {
        json!({"tag":[info, inner]})
    }

    fn goal() -> Value {
        json!({
            "userName": "pos",
            "hyps": [
                {"names":["n","m✝"],"type":{"text":"Nat"}},
                {"names":["h"],"type":tag(json!({"info":{"p":"1"}}), json!({"text":"n = 0"})),
                 "val":{"text":"rfl"},"isInserted":true},
            ],
            "type": {"append":[
                tag(json!({"info":{"p":"2"},"diffStatus":"wasChanged"}), json!({"text":"n"})),
                {"text":" +\n  0 = n"},
            ]},
        })
    }

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn goal_lines_keep_layout_and_mark_names_and_diffs() {
        let texts = goal_lines(&goal());
        let lines: Vec<&Line> = texts.iter().map(|t| &t.line).collect();
        let text: Vec<String> = lines.iter().map(|l| plain(l)).collect();
        assert_eq!(
            text,
            [
                "case pos",
                "n m✝ : Nat",
                "h : n = 0 := rfl",
                "⊢ n +",
                "  0 = n"
            ]
        );
        assert_eq!(lines[1].spans[0].style, HYP);
        assert_eq!(lines[1].spans[2].style, INACCESSIBLE);
        assert_eq!(lines[2].spans[0].style, HYP.patch(INSERTED));
        assert_eq!(lines[3].spans[0].style, PREFIX);
        assert_eq!(lines[3].spans[1].style, INSERTED);
        assert_eq!(lines[3].spans[2].style, Style::new());
        // `h : ` is 4 characters; `n = 0` is the tagged subterm after it.
        assert_eq!(
            texts[2].links,
            [Link {
                start: 4,
                end: 9,
                info: json!({"p":"1"})
            }]
        );
        assert_eq!(
            texts[3].links,
            [Link {
                start: 2,
                end: 3,
                info: json!({"p":"2"})
            }]
        );
    }

    #[test]
    fn highlight_colors_keywords_literals_and_locals() {
        let mut spans = Vec::new();
        highlight(
            "∀ (k : Nat), fun x => n + 10 = Nat.succ k ∧ s = \"a\\\"b\" ∃! y",
            &["n", "s"],
            &mut spans,
        );
        let got: Vec<(&str, Style)> = spans
            .iter()
            .map(|s| (s.content.as_ref(), s.style))
            .collect();
        assert_eq!(
            got,
            [
                ("∀", KEYWORD),
                (" (k : Nat), ", Style::new()),
                ("fun", KEYWORD),
                (" x => ", Style::new()),
                ("n", HYP),
                (" + ", Style::new()),
                ("10", LITERAL),
                (" = Nat.succ k ∧ ", Style::new()),
                ("s", HYP),
                (" = ", Style::new()),
                ("\"a\\\"b\"", LITERAL),
                (" ", Style::new()),
                ("∃!", KEYWORD),
                (" y", Style::new()),
            ]
        );
    }

    #[test]
    fn goal_text_matches_the_infoview_copy() {
        assert_eq!(
            goal_text(&goal()),
            "case pos\nn m✝ : Nat\nh : n = 0 := rfl\n⊢ n +\n  0 = n"
        );
        let anon = json!({"hyps":[{"names":["[anonymous]","x"],"type":{"text":"A"}}],
                          "type":{"text":"B"},"goalPrefix":"| "});
        assert_eq!(goal_text(&anon), "x : A\n| B");
    }

    #[test]
    fn refs_collects_every_pointer() {
        let mut out = Vec::new();
        refs(
            &json!([goal(), {"info":{"__rpcref":"3"}}, {"p":"x","q":1}]),
            &mut out,
        );
        assert_eq!(
            out,
            [json!({"p":"1"}), json!({"p":"2"}), json!({"__rpcref":"3"})]
        );
    }
}
