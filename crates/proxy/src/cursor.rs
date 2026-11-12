use crate::msg;
use serde_json::{Value, json};

pub fn position(p: &Value) -> bool {
    p["line"].as_u64().is_some() && p["character"].as_u64().is_some()
}

pub fn location(m: &Value) -> Option<Value> {
    if !msg::is_request(m) {
        return None;
    }
    let p = &m["params"];
    let uri = p["textDocument"]["uri"].as_str()?;
    let range = match msg::method(m)? {
        "textDocument/documentHighlight" if position(&p["position"]) => {
            json!({"start":p["position"],"end":p["position"]})
        }
        "textDocument/codeAction"
            if p["context"].get("only").is_none()
                && position(&p["range"]["start"])
                && position(&p["range"]["end"]) =>
        {
            p["range"].clone()
        }
        _ => return None,
    };
    Some(json!({"uri":uri,"range":range}))
}

pub fn insert_edit(
    text: &str,
    kind: &str,
    pos: Option<&Value>,
    location: Option<&Value>,
) -> Option<Value> {
    let (uri, mut at) = match pos {
        Some(pos) => (
            pos["textDocument"]["uri"].as_str()?,
            pos["position"].clone(),
        ),
        None => {
            let loc = location?;
            (loc["uri"].as_str()?, loc["range"]["start"].clone())
        }
    };
    if !position(&at) {
        return None;
    }
    let text = match kind {
        "here" => text.to_owned(),
        "above" => {
            at["character"] = json!(0);
            format!("{text}\n")
        }
        _ => return None,
    };
    Some(json!({"changes":{uri:[{"range":{"start":at,"end":at},"newText":text}]}}))
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selection_uses_its_range_but_save_and_hover_do_not_move_cursor() {
        let mut m = json!({"id":1,"method":"textDocument/codeAction","params":{"textDocument":{"uri":"file:///F.lean"},"range":{"start":{"line":0,"character":0},"end":{"line":2,"character":3}},"context":{}}});
        assert_eq!(location(&m).unwrap()["range"], m["params"]["range"]);
        m["params"]["context"]["only"] = json!([]);
        assert!(location(&m).is_none());
        m["method"] = json!("textDocument/hover");
        assert!(location(&m).is_none());
        assert!(!position(&json!({"line":-1,"character":0})));
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn inserts_above_line_zero_or_here_without_replacing_text() {
        let pos =
            json!({"textDocument":{"uri":"file:///F.lean"},"position":{"line":0,"character":4}});
        let edit = insert_edit("/- goal -/", "above", Some(&pos), None).unwrap();
        assert_eq!(
            edit["changes"]["file:///F.lean"][0],
            json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"/- goal -/\n"})
        );
        let edit = insert_edit("exact h", "here", Some(&pos), None).unwrap();
        assert_eq!(
            edit["changes"]["file:///F.lean"][0]["range"]["start"],
            pos["position"]
        );
        assert!(insert_edit("x", "here", None, None).is_none());
        assert!(insert_edit("x", "bad", Some(&pos), None).is_none());
    }
}
