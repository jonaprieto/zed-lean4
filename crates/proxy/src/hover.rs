//! Adds the `$/lean/plainGoal` result, and how to type the symbol under the
//! cursor, to a `textDocument/hover` response.
use serde_json::{Value, json};

/// What separates the hover's sections, as markdown.
const RULE: &str = "\n\n---\n\n";

/// `result.rendered` of a plainGoal response; None for null results and errors.
pub fn rendered_goal(plain_goal_response: &Value) -> Option<String> {
    Some(
        plain_goal_response
            .get("result")?
            .get("rendered")?
            .as_str()?
            .to_owned(),
    )
}

/// Appends `sections` below the hover text, each after a rule: the goal at
/// the position, how to type the symbol there. Errors and non-markdown hovers
/// pass through.
pub fn merge_hover(mut response: Value, sections: impl IntoIterator<Item = String>) -> Value {
    let sections: Vec<String> = sections.into_iter().collect();
    if sections.is_empty() || response.get("error").is_some() {
        return response;
    }
    let added = sections.join(RULE);
    match response.get_mut("result") {
        Some(result) if result.is_null() => {
            *result = json!({"contents": {"kind": "markdown", "value": added}});
        }
        Some(result) => {
            if let Some(contents) = result.get_mut("contents")
                && contents.get("kind").and_then(Value::as_str) == Some("markdown")
                && let Some(text) = contents.get("value").and_then(Value::as_str)
            {
                contents["value"] = json!(format!("{text}{RULE}{added}"));
            }
        }
        None => {}
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn golden(name: &str) -> Value {
        let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn reads_rendered_goal() {
        assert_eq!(
            rendered_goal(&golden("plain_goal.json")).as_deref(),
            Some("```lean\nn : Nat\n⊢ n + 0 = n\n```")
        );
        assert_eq!(
            rendered_goal(&json!({"id": "lean4-ide:2", "result": null})),
            None
        );
        assert_eq!(
            rendered_goal(&json!({"id": "lean4-ide:3", "error": {"code": -32801}})),
            None
        );
    }

    #[test]
    fn appends_goal_after_hover_text_and_keeps_range() {
        let goal = rendered_goal(&golden("plain_goal.json"));
        let merged = merge_hover(golden("hover_rfl.json"), goal);
        let value = merged["result"]["contents"]["value"].as_str().unwrap();
        assert!(value.starts_with("This tactic applies"));
        assert!(value.ends_with("\n\n---\n\n```lean\nn : Nat\n⊢ n + 0 = n\n```"));
        assert_eq!(
            merged["result"]["range"],
            golden("hover_rfl.json")["result"]["range"]
        );
        assert_eq!(merged["id"], json!(2));
    }

    #[test]
    fn every_section_follows_the_hover_text_in_order() {
        let hover = json!({"id": 3, "result": {"contents": {"kind": "markdown", "value": "doc"}}});
        let merged = merge_hover(hover, ["G".to_owned(), "Type `α` with `\\a`.".to_owned()]);
        assert_eq!(
            merged["result"]["contents"]["value"],
            "doc\n\n---\n\nG\n\n---\n\nType `α` with `\\a`."
        );
    }

    #[test]
    fn goal_alone_when_server_has_no_hover() {
        let merged = merge_hover(
            json!({"jsonrpc": "2.0", "id": 4, "result": null}),
            Some("g".into()),
        );
        assert_eq!(
            merged["result"],
            json!({"contents": {"kind": "markdown", "value": "g"}})
        );
    }

    #[test]
    fn unchanged_without_goal_on_error_or_non_markdown() {
        let hover = golden("hover_rfl.json");
        assert_eq!(merge_hover(hover.clone(), None), hover);
        let err =
            json!({"jsonrpc": "2.0", "id": 5, "error": {"code": -32800, "message": "cancelled"}});
        assert_eq!(merge_hover(err.clone(), Some("g".into())), err);
        let marked = json!({"jsonrpc": "2.0", "id": 6, "result": {"contents": ["plain"]}});
        assert_eq!(merge_hover(marked.clone(), Some("g".into())), marked);
    }
}
