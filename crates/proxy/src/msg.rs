//! Messages stay `serde_json::Value` so unknown fields and `"result": null` survive.
use serde_json::{Value, json};

pub const PROXY_ID_PREFIX: &str = "lean4-ide:";

pub fn method(m: &Value) -> Option<&str> {
    m.get("method")?.as_str()
}
pub fn id(m: &Value) -> Option<&Value> {
    m.get("id")
}
pub fn is_request(m: &Value) -> bool {
    method(m).is_some() && id(m).is_some()
}
pub fn is_response(m: &Value) -> bool {
    method(m).is_none() && id(m).is_some()
}
/// Stable map key for a JSON-RPC id (`1` and `"1"` stay distinct).
pub fn id_key(id: &Value) -> String {
    id.to_string()
}
pub fn request(id: &str, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}
