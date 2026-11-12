use crate::msg;
use serde_json::{Map, Value, json};

fn object(value: &mut Value) -> Option<&mut Map<String, Value>> {
    if value.is_null() {
        *value = json!({});
    }
    value.as_object_mut()
}

pub fn rewrite_request(mut m: Value) -> Value {
    if !msg::is_request(&m) || msg::method(&m) != Some("initialize") {
        return m;
    }
    let mut params = m["params"].clone();
    let Some(p) = object(&mut params) else {
        return m;
    };
    let Some(caps) = object(p.entry("capabilities").or_insert(Value::Null)) else {
        return m;
    };
    let Some(lean) = object(caps.entry("lean").or_insert(Value::Null)) else {
        return m;
    };
    lean.remove("incrementalDiagnosticSupport");
    lean.remove("silentDiagnosticSupport");
    lean.insert("rpcWireFormat".into(), json!("v1"));
    let Some(opts) = object(p.entry("initializationOptions").or_insert(Value::Null)) else {
        return m;
    };
    opts.insert("hasWidgets".into(), json!(true));
    m["params"] = params;
    m
}

pub const OPEN_COMMAND: &str = "lean4-ide.openInfoview";

pub fn advertise_command(result: &mut Value) {
    let Some(result) = object(result) else { return };
    let Some(caps) = object(result.entry("capabilities").or_insert(Value::Null)) else {
        return;
    };
    let Some(provider) = object(caps.entry("executeCommandProvider").or_insert(Value::Null)) else {
        return;
    };
    let commands = provider.entry("commands").or_insert_with(|| json!([]));
    if let Some(commands) = commands.as_array_mut()
        && !commands.iter().any(|v| v == OPEN_COMMAND)
    {
        commands.push(json!(OPEN_COMMAND));
    }
}

pub fn add_open_action(mut response: Value) -> Value {
    if response.get("error").is_some() {
        return response;
    }
    if let Some(result) = response.get_mut("result") {
        if result.is_null() {
            *result = json!([]);
        }
        if let Some(actions) = result.as_array_mut() {
            actions.push(json!({"title":"Open Lean Infoview","command":{
                "title":"Open Lean Infoview","command":OPEN_COMMAND}}));
        }
    }
    response
}

#[cfg(test)]
mod init_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rewrite_preserves_fields_and_does_not_enable_extra_diagnostics() {
        let m = json!({"id": 1, "method": "initialize", "params": {
            "rootUri": "file:///p", "capabilities": {"textDocument": {}, "lean": {
                "incrementalDiagnosticSupport": true, "silentDiagnosticSupport": true, "extra": 7}},
            "initializationOptions": {"extra": 8}}});
        let out = rewrite_request(m);
        assert_eq!(out["params"]["rootUri"], "file:///p");
        assert_eq!(
            out["params"]["capabilities"]["lean"],
            json!({"rpcWireFormat":"v1", "extra":7})
        );
        assert_eq!(
            out["params"]["initializationOptions"],
            json!({"hasWidgets":true,"extra":8})
        );
        assert_eq!(
            rewrite_request(json!({"id":1,"method":"initialize","params":{}}))["params"]["capabilities"]
                ["lean"]["rpcWireFormat"],
            "v1"
        );
    }

    #[test]
    fn malformed_and_unrelated_messages_are_safe() {
        for m in [
            json!({"id":1,"method":"initialize","params":7}),
            json!({"id":2,"method":"unknown","params":{}}),
        ] {
            assert_eq!(rewrite_request(m.clone()), m);
        }
    }
}
