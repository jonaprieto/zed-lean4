//! Lean's interactive messages (`TaggedText<MsgEmbed>`) as foldable rows.
use std::collections::HashMap;

use ratatui::text::{Line, Span};
use serde_json::Value;

use super::{
    goals, theme,
    view::{Rows, Text},
};

/// Writes one diagnostic: `head`, then its message. A message longer than a
/// line folds under its first line, keyed by `key`. `lazy` holds the trace
/// children already fetched, by reference.
pub fn message(
    rows: &mut Rows,
    lazy: &HashMap<String, Value>,
    key: &str,
    head: Line<'static>,
    msg: &Value,
) {
    let multiline = spans_lines(msg);
    let mut w = Writer {
        rows,
        lazy,
        line: head.into(),
        header: multiline.then(|| (key.to_owned(), false)),
    };
    w.embed(msg, key, &mut 0);
    w.finish();
    if multiline {
        w.rows.close();
    }
}

struct Writer<'r, 'a> {
    rows: &'r mut Rows<'a>,
    lazy: &'r HashMap<String, Value>,
    line: Text,
    /// A block whose header is the next flushed line, and whether it starts folded.
    header: Option<(String, bool)>,
}

impl Writer<'_, '_> {
    fn flush(&mut self) {
        let line = std::mem::take(&mut self.line);
        match self.header.take() {
            Some((key, collapsed)) => self.rows.open_as(key, line, collapsed),
            None => self.rows.push(line),
        }
    }

    /// Flushes the current line unless nothing is on it.
    fn finish(&mut self) {
        if !self.line.line.spans.is_empty() || self.header.is_some() {
            self.flush();
        }
    }

    /// `prefix` and `n` give each trace a key that stays put when others load.
    fn embed(&mut self, t: &Value, prefix: &str, n: &mut usize) {
        if let Some(s) = t["text"].as_str() {
            for (i, part) in s.split('\n').enumerate() {
                if i > 0 {
                    self.flush();
                }
                if !part.is_empty() {
                    self.line.line.spans.push(Span::raw(part.to_owned()));
                }
            }
        } else if let Some(parts) = t["append"].as_array() {
            parts.iter().for_each(|p| self.embed(p, prefix, n));
        } else if let Some([tag, inner]) = t["tag"].as_array().map(Vec::as_slice) {
            if let Some(expr) = tag.get("expr") {
                let lines = goals::typeset(std::mem::take(&mut self.line), expr, &[]);
                let last = lines.len() - 1;
                for (i, line) in lines.into_iter().enumerate() {
                    self.line = line;
                    if i < last {
                        self.flush();
                    }
                }
            } else if let Some(goal) = tag.get("goal") {
                self.finish();
                for line in goals::goal_lines(goal) {
                    self.line = line;
                    self.flush();
                }
            } else if let Some(widget) = tag.get("widget") {
                self.embed(&widget["alt"], prefix, n);
            } else if let Some(trace) = tag.get("trace") {
                *n += 1;
                self.trace(trace, &format!("{prefix}.{n}"));
            } else {
                // `highlighted` marks a search match; the text is what matters.
                self.embed(inner, prefix, n);
            }
        }
    }

    fn trace(&mut self, trace: &Value, key: &str) {
        self.finish();
        let children = &trace["children"];
        let strict = children["strict"].as_array();
        let leaf = strict.is_some_and(Vec::is_empty);
        if !leaf {
            self.header = Some((key.to_owned(), trace["collapsed"] == true));
        }
        let indent = trace["indent"].as_u64().unwrap_or(0);
        let class = trace["cls"].as_str().unwrap_or_default();
        self.line.line.spans.extend([
            Span::raw(" ".repeat(usize::try_from(indent).unwrap_or(0))),
            Span::styled(format!("[{class}]"), theme::current().trace_class),
            Span::raw(" "),
        ]);
        self.embed(&trace["msg"], key, &mut 0);
        self.finish();
        if leaf {
            return;
        }
        let mut n = 0;
        let loaded = goals::ref_key(&children["lazy"])
            .and_then(|p| self.lazy.get(p))
            .and_then(Value::as_array);
        match strict.or(loaded) {
            Some(children) => {
                for child in children {
                    self.embed(child, key, &mut n);
                    self.finish();
                }
            }
            None => {
                self.rows.load(children["lazy"].clone());
                self.rows.push(Line::raw("  ⋯"));
            }
        }
        self.rows.close();
    }
}

/// Whether the message needs more than one line.
fn spans_lines(t: &Value) -> bool {
    if let Some(s) = t["text"].as_str() {
        return s.contains('\n');
    }
    if let Some(parts) = t["append"].as_array() {
        return parts.iter().any(spans_lines);
    }
    match t["tag"].as_array().map(Vec::as_slice) {
        Some([tag, inner]) => {
            tag.get("goal").is_some()
                || tag.get("trace").is_some()
                || tag
                    .get("expr")
                    .is_some_and(|e| goals::text(e).contains('\n'))
                || spans_lines(inner)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn render(msg: &Value, folded: &HashSet<String>, lazy: &HashMap<String, Value>) -> Vec<String> {
        let mut rows = Rows::new(folded);
        message(&mut rows, lazy, "m", Line::raw("1:0 error: "), msg);
        rows.rows
            .iter()
            .map(|r| r.line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn trace(cls: &str, msg: &str, children: Value, collapsed: bool) -> Value {
        json!({"tag":[{"trace":{"indent":0,"cls":cls,"msg":{"text":msg},"collapsed":collapsed,"children":children}},{"text":""}]})
    }

    #[test]
    fn single_line_messages_with_exprs_stay_on_one_row() {
        let msg = json!({"append":[{"text":"type mismatch "},{"tag":[{"expr":{"tag":[{"info":{"p":"1"}},{"text":"h"}]}},{"text":""}]}]});
        let none = HashSet::new();
        let mut rows = Rows::new(&none);
        message(
            &mut rows,
            &HashMap::new(),
            "m",
            Line::raw("1:0 error: "),
            &msg,
        );
        let row = &rows.rows[0];
        assert_eq!(rows.rows.len(), 1);
        assert!(!row.header);
        let text: String = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "1:0 error: type mismatch h");
        assert_eq!(row.links[0].start, 25);
        assert_eq!(row.links[0].info, json!({"p":"1"}));
    }

    #[test]
    fn goals_and_lines_fold_under_the_first_line() {
        let goal = json!({"hyps":[{"names":["n"],"type":{"text":"Nat"}}],"type":{"text":"n = n"}});
        let msg =
            json!({"append":[{"text":"unsolved goals\n"},{"tag":[{"goal":goal},{"text":""}]}]});
        assert_eq!(
            render(&msg, &HashSet::new(), &HashMap::new()),
            ["▾ 1:0 error: unsolved goals", "n : Nat", "⊢ n = n"]
        );
        let folded = HashSet::from(["m".to_owned()]);
        assert_eq!(
            render(&msg, &folded, &HashMap::new()),
            ["▸ 1:0 error: unsolved goals"]
        );
    }

    #[test]
    fn traces_nest_fold_and_load_lazily() {
        let leaf = trace("Meta.isDefEq", "✅ a =?= a", json!({"strict":[]}), false);
        let lazy = trace(
            "Elab.step",
            "lazy node",
            json!({"lazy":{"__rpcref":"7"}}),
            true,
        );
        let root = trace(
            "Elab.command",
            "root",
            json!({"strict":[leaf, lazy]}),
            false,
        );
        let none = HashSet::new();
        assert_eq!(
            render(&root, &none, &HashMap::new()),
            [
                "▾ 1:0 error: ",
                "▾ [Elab.command] root",
                "[Meta.isDefEq] ✅ a =?= a",
                "▸ [Elab.step] lazy node",
            ]
        );
        // Toggling a collapsed trace opens it; unloaded children show a placeholder.
        let open = HashSet::from(["m.1.2".to_owned()]);
        let rows = render(&root, &open, &HashMap::new());
        assert_eq!(rows[3..], ["▾ [Elab.step] lazy node", "  ⋯"]);
        let mut built = Rows::new(&open);
        message(&mut built, &HashMap::new(), "m", Line::raw(""), &root);
        assert_eq!(built.rows[3].load, Some(json!({"__rpcref":"7"})));
        let loaded = HashMap::from([("7".to_owned(), json!([{"text":"child"}]))]);
        assert_eq!(
            render(&root, &open, &loaded)[3..],
            ["▾ [Elab.step] lazy node", "child"]
        );
    }

    #[test]
    fn widgets_show_their_alternative_text() {
        let msg = json!({"tag":[{"widget":{"wi":{"id":"x"},"alt":{"text":"Try this: simp"}}},{"text":""}]});
        assert_eq!(
            render(&msg, &HashSet::new(), &HashMap::new()),
            ["1:0 error: Try this: simp"]
        );
    }
}
