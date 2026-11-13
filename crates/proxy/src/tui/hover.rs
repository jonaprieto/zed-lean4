//! Lean's hover at the cursor (`textDocument/hover`) as terminal lines.
use serde_json::Value;

use super::markdown::{self, Hanging};

/// The hover's markdown as lines. `None` when Lean has nothing to say.
pub fn lines(result: &Value) -> Option<Vec<Hanging>> {
    let text = match &result["contents"] {
        Value::String(s) => s.as_str(),
        contents => contents["value"].as_str()?,
    };
    let lines = markdown::lines(text);
    (!lines.is_empty()).then_some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plain(lines: &[Hanging]) -> Vec<String> {
        lines.iter().map(|(line, _)| line.to_string()).collect()
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
    fn nothing_to_show() {
        assert!(lines(&Value::Null).is_none());
        assert!(lines(&json!({"contents":{"kind":"markdown","value":""}})).is_none());
        assert_eq!(
            plain(&lines(&json!({"contents":"plain"})).unwrap()),
            ["plain"]
        );
    }
}
