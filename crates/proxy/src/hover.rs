//! Adds the `$/lean/plainGoal` result to a `textDocument/hover` response.
use serde_json::{Value, json};

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

/// Appends `goal` below the hover text. Errors and non-markdown hovers pass through.
pub fn merge_hover(mut response: Value, goal: Option<String>) -> Value {
    let Some(goal) = goal else { return response };
    if response.get("error").is_some() {
        return response;
    }
    match response.get_mut("result") {
        Some(result) if result.is_null() => {
            *result = json!({"contents": {"kind": "markdown", "value": goal}});
        }
        Some(result) => {
            if let Some(contents) = result.get_mut("contents")
                && contents.get("kind").and_then(Value::as_str) == Some("markdown")
                && let Some(text) = contents.get("value").and_then(Value::as_str)
            {
                let merged = format!("{text}\n\n---\n\n{goal}");
                contents["value"] = json!(merged);
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
