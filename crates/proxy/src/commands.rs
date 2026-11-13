//! The commands the proxy adds to Lean's, and the code actions that run them:
//! open the browser infoview, and restart a file whose imports changed.
use serde_json::{Value, json};

use crate::msg;

pub const OPEN_INFOVIEW: &str = "lean4-ide.openInfoview";
pub const RESTART_FILE: &str = "lean4-ide.restartFile";

/// How Lean's diagnostic starts when a file's imports changed after it was
/// elaborated; Lean offers no code action for it, only this text.
const STALE_IMPORTS: &str = "Imports are out of date";

/// Adds `commands` to the server's `executeCommandProvider` so Zed runs them.
pub fn advertise(result: &mut Value, commands: &[&str]) {
    let Some(result) = msg::object(result) else {
        return;
    };
    let Some(caps) = msg::object(result.entry("capabilities").or_insert(Value::Null)) else {
        return;
    };
    let provider = caps.entry("executeCommandProvider").or_insert(Value::Null);
    let Some(provider) = msg::object(provider) else {
        return;
    };
    let Some(known) = provider
        .entry("commands")
        .or_insert_with(|| json!([]))
        .as_array_mut()
    else {
        return;
    };
    for command in commands {
        if !known.iter().any(|c| c == command) {
            known.push(json!(command));
        }
    }
}

/// Whether `diagnostic` is Lean asking for the file to be restarted.
pub fn imports_out_of_date(diagnostic: &Value) -> bool {
    diagnostic["message"]
        .as_str()
        .is_some_and(|m| m.starts_with(STALE_IMPORTS))
}

pub fn open_infoview() -> Value {
    let title = "Open Lean Infoview";
    json!({"title":title,"command":{"title":title,"command":OPEN_INFOVIEW}})
}

/// The quick fix for `stale`, Lean's out-of-date-imports diagnostics of `uri`.
pub fn restart_file(uri: &str, stale: Vec<Value>) -> Value {
    let title = "Restart File";
    json!({"title":title,"kind":"quickfix","isPreferred":true,"diagnostics":stale,
        "command":{"title":title,"command":RESTART_FILE,"arguments":[uri]}})
}

/// Appends `actions` to a code action response; an error, or a response
/// with nothing to add, stays as it is.
pub fn add_actions(mut response: Value, actions: Vec<Value>) -> Value {
    if actions.is_empty() || response.get("error").is_some() {
        return response;
    }
    let result = &mut response["result"];
    if result.is_null() {
        *result = json!([]);
    }
    if let Some(list) = result.as_array_mut() {
        list.extend(actions);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertising_keeps_lean_commands_and_adds_each_once() {
        let mut result = json!({"capabilities":{"executeCommandProvider":{"commands":["lean"]}}});
        advertise(&mut result, &[OPEN_INFOVIEW, RESTART_FILE]);
        advertise(&mut result, &[RESTART_FILE]);
        assert_eq!(
            result["capabilities"]["executeCommandProvider"]["commands"],
            json!(["lean", OPEN_INFOVIEW, RESTART_FILE])
        );
        let mut bare = json!({});
        advertise(&mut bare, &[RESTART_FILE]);
        assert_eq!(
            bare,
            json!({"capabilities":{"executeCommandProvider":{"commands":[RESTART_FILE]}}})
        );
    }

    #[test]
    fn only_the_stale_imports_message_asks_for_a_restart() {
        let stale = json!({"message":"Imports are out of date and should be rebuilt; use the \"Restart File\" command in your editor."});
        assert!(imports_out_of_date(&stale));
        assert!(!imports_out_of_date(
            &json!({"message":"unknown identifier"})
        ));
        assert!(!imports_out_of_date(&json!({})));
    }

    #[test]
    fn actions_append_to_lean_ones_and_errors_pass_through() {
        let stale = json!({"message":"Imports are out of date"});
        let restart = restart_file("file:///F.lean", vec![stale.clone()]);
        assert_eq!(
            restart,
            json!({"title":"Restart File","kind":"quickfix","isPreferred":true,
                "diagnostics":[stale],
                "command":{"title":"Restart File","command":RESTART_FILE,
                    "arguments":["file:///F.lean"]}})
        );
        let lean = json!({"title":"Try this"});
        assert_eq!(
            add_actions(json!({"id":1,"result":[lean]}), vec![open_infoview()]),
            json!({"id":1,"result":[lean, open_infoview()]})
        );
        assert_eq!(
            add_actions(json!({"id":1,"result":null}), vec![restart.clone()]),
            json!({"id":1,"result":[restart]})
        );
        let none = json!({"id":1,"result":null});
        assert_eq!(
            add_actions(none.clone(), vec![]),
            none,
            "nothing to add leaves null"
        );
        let error = json!({"id":1,"error":{"code":-32801,"message":"modified"}});
        assert_eq!(add_actions(error.clone(), vec![open_infoview()]), error);
    }
}
