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
