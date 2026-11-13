//! The text of every file Zed has open in Lean, kept in step with Zed's edits
//! so the proxy can close and reopen a file (Lean's "Restart File") on its own.
use std::collections::HashMap;

use serde_json::{Value, json};

use crate::msg;

pub const DID_OPEN: &str = "textDocument/didOpen";
pub const DID_CHANGE: &str = "textDocument/didChange";
pub const DID_CLOSE: &str = "textDocument/didClose";

struct Document {
    language_id: Value,
    version: Value,
    text: String,
}

#[derive(Default)]
pub struct Documents(HashMap<String, Document>);

impl Documents {
    /// Follows one notification from Zed; all but open, change and close are ignored.
    pub fn observe(&mut self, m: &Value) {
        let params = &m["params"];
        let doc = &params["textDocument"];
        let Some(uri) = doc["uri"].as_str() else {
            return;
        };
        match msg::method(m) {
            Some(DID_OPEN) => {
                let document = Document {
                    language_id: doc["languageId"].clone(),
                    version: doc["version"].clone(),
                    text: doc["text"].as_str().unwrap_or_default().to_owned(),
                };
                self.0.insert(uri.to_owned(), document);
            }
            Some(DID_CHANGE) => {
                if let Some(document) = self.0.get_mut(uri) {
                    for change in params["contentChanges"].as_array().into_iter().flatten() {
                        apply(&mut document.text, change);
                    }
                    document.version = doc["version"].clone();
                }
            }
            Some(DID_CLOSE) => {
                self.0.remove(uri);
            }
            _ => {}
        }
    }

    /// The `didClose` and `didOpen` that make Lean start `uri` over from its
    /// current text, or `None` when the file is not open.
    pub fn reopen(&self, uri: &str) -> Option<[Value; 2]> {
        let document = self.0.get(uri)?;
        Some([
            msg::notification(DID_CLOSE, json!({"textDocument":{"uri":uri}})),
            msg::notification(
                DID_OPEN,
                json!({"textDocument":{"uri":uri,"languageId":document.language_id,
                    "version":document.version,"text":document.text}}),
            ),
        ])
    }
}

/// Applies one `TextDocumentContentChangeEvent`: a range edit, or the whole
/// text when it has no range.
fn apply(text: &mut String, change: &Value) {
    let new = change["text"].as_str().unwrap_or_default();
    match change.get("range") {
        Some(range) => {
            let start = offset(text, &range["start"]);
            let end = offset(text, &range["end"]).max(start);
            text.replace_range(start..end, new);
        }
        None => *text = new.to_owned(),
    }
}

/// The byte offset of an LSP position, whose `character` counts UTF-16 code
/// units. Past the end of its line it stops at the line end, past the last
/// line at the end of the text.
fn offset(text: &str, position: &Value) -> usize {
    let line = position["line"].as_u64().unwrap_or_default();
    let mut units = position["character"].as_u64().unwrap_or_default();
    let mut start = 0;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(newline) => start += newline + 1,
            None => return text.len(),
        }
    }
    let line_end = text[start..].find('\n').map_or(text.len(), |n| start + n);
    for (i, c) in text[start..line_end].char_indices() {
        let width = c.len_utf16() as u64;
        if units < width {
            return start + i;
        }
        units -= width;
    }
    line_end
}

#[cfg(test)]
mod tests {
    use super::*;

    const URI: &str = "file:///p/F.lean";

    fn open(text: &str) -> Value {
        msg::notification(
            DID_OPEN,
            json!({"textDocument":{"uri":URI,"languageId":"lean4","version":1,"text":text}}),
        )
    }

    fn change(version: u64, changes: Value) -> Value {
        msg::notification(
            DID_CHANGE,
            json!({"textDocument":{"uri":URI,"version":version},"contentChanges":changes}),
        )
    }

    fn edit(start: (u32, u32), end: (u32, u32), text: &str) -> Value {
        json!({"range":{"start":{"line":start.0,"character":start.1},
            "end":{"line":end.0,"character":end.1}},"text":text})
    }

    fn text(docs: &Documents) -> String {
        let [_, open] = docs.reopen(URI).unwrap();
        open["params"]["textDocument"]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn reopening_closes_then_opens_the_current_text_and_version() {
        let mut docs = Documents::default();
        assert!(docs.reopen(URI).is_none());
        docs.observe(&open("def a := 1\n"));
        docs.observe(&change(2, json!([edit((0, 9), (0, 10), "2")])));
        assert_eq!(
            docs.reopen(URI).unwrap(),
            [
                msg::notification(DID_CLOSE, json!({"textDocument":{"uri":URI}})),
                msg::notification(
                    DID_OPEN,
                    json!({"textDocument":{"uri":URI,"languageId":"lean4","version":2,
                        "text":"def a := 2\n"}})
                ),
            ]
        );
        docs.observe(&msg::notification(
            DID_CLOSE,
            json!({"textDocument":{"uri":URI}}),
        ));
        assert!(docs.reopen(URI).is_none());
    }

    #[test]
    fn edits_apply_in_order_across_lines_and_in_utf16_columns() {
        let mut docs = Documents::default();
        // `α` is one UTF-16 unit and two bytes; `𝔽` is two units and four bytes.
        docs.observe(&open("theorem t : α = α := rfl\n-- 𝔽 x\n"));
        docs.observe(&change(
            2,
            json!([
                edit((0, 12), (0, 13), "β"),
                edit((1, 6), (1, 7), "y"),
                edit((0, 24), (1, 0), " -- joined "),
            ]),
        ));
        assert_eq!(text(&docs), "theorem t : β = α := rfl -- joined -- 𝔽 y\n");
        // A range past the end of a line or the file stops at it.
        docs.observe(&change(3, json!([edit((0, 99), (9, 0), "!")])));
        assert_eq!(text(&docs), "theorem t : β = α := rfl -- joined -- 𝔽 y!");
        // A change without a range replaces the whole text.
        docs.observe(&change(4, json!([{"text":"new"}])));
        assert_eq!(text(&docs), "new");
    }
}
