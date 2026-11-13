use crate::msg::{self, object};
use serde_json::{Value, json};

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
