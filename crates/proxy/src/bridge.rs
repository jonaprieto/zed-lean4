use serde_json::{Value, json};

pub struct Call {
    pub id: String,
    pub name: String,
    pub args: Vec<Value>,
}

impl Call {
    pub fn parse(m: &Value) -> Option<Self> {
        Some(Self {
            id: m.get("id")?.as_str()?.to_owned(),
            name: m.get("call")?.as_str()?.to_owned(),
            args: m.get("args")?.as_array()?.clone(),
        })
    }
}

pub fn reply(id: &str, result: Value) -> Value {
    json!({"id": id, "result": result})
}

pub fn reply_error(id: &str, code: i64, message: &str) -> Value {
    json!({"id": id, "error": {"code": code, "message": message}})
}

pub fn relay(id: &str, response: &Value) -> Value {
    match response.get("error") {
        Some(error) => json!({"id": id, "error": error}),
        None => reply(id, response["result"].clone()),
    }
}

pub fn event(name: &str, args: Value) -> Value {
    json!({"event": name, "args": args})
}
