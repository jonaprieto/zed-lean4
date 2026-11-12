//! Lean's interactive goals (`Lean.Widget.InteractiveGoal`, `TaggedText`) as styled terminal lines.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

const HYP: Style = Style::new().fg(Color::Cyan);
const PREFIX: Style = Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD);
const INACCESSIBLE: Style = Style::new().add_modifier(Modifier::DIM);
const INSERTED: Style = Style::new().fg(Color::Black).bg(Color::Green);
const REMOVED: Style = Style::new().fg(Color::Black).bg(Color::Red);

/// The plain text of a `TaggedText`.
pub fn text(t: &Value) -> String {
    let mut out = String::new();
    runs(t, Style::new(), &mut |s, _| out.push_str(s));
    out
}

/// The key of an RPC reference: `{"__rpcref": ..}` in wire format v1, `{"p": ..}` before.
pub fn ref_key(v: &Value) -> Option<&str> {
    let m = v.as_object().filter(|m| m.len() == 1)?;
    m.get("__rpcref").or_else(|| m.get("p"))?.as_str()
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
pub fn goal_lines(goal: &Value) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if let Some(case) = goal["userName"].as_str() {
        out.push(Line::from(vec![
            Span::styled("case ", INACCESSIBLE),
            Span::styled(case.to_owned(), Style::new().add_modifier(Modifier::BOLD)),
        ]));
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
        let mut lines = typeset(head, &hyp["type"]);
        if !hyp["val"].is_null() {
            let last = lines.pop().unwrap_or_default();
            let mut spans = last.spans;
            spans.push(Span::raw(" := "));
            lines.extend(typeset(spans, &hyp["val"]));
        }
        out.extend(lines);
    }
    let target = vec![Span::styled(prefix(goal), PREFIX.patch(marked(goal)))];
    out.extend(typeset(target, &goal["type"]));
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

/// `head` followed by `t`, split into lines at the pretty printer's newlines.
fn typeset(head: Vec<Span<'static>>, t: &Value) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(head)];
    runs(t, Style::new(), &mut |s, style| {
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                lines.push(Line::default());
            }
            if !part.is_empty() {
                let line = lines.last_mut().expect("starts with the head");
                line.spans.push(Span::styled(part.to_owned(), style));
            }
        }
    });
    lines
}

/// Calls `f` with each text run of `t` and the diff style of its innermost tag.
fn runs(t: &Value, style: Style, f: &mut impl FnMut(&str, Style)) {
    if let Some(s) = t["text"].as_str() {
        f(s, style);
    } else if let Some(parts) = t["append"].as_array() {
        parts.iter().for_each(|p| runs(p, style, f));
    } else if let Some([info, inner]) = t["tag"].as_array().map(Vec::as_slice) {
        let style = match info["diffStatus"].as_str() {
            Some("wasChanged" | "wasInserted" | "willInsert") => INSERTED,
            Some("willChange" | "willDelete" | "wasDeleted") => REMOVED,
            _ => style,
        };
        runs(inner, style, f);
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
        let lines = goal_lines(&goal());
        let text: Vec<String> = lines.iter().map(plain).collect();
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
