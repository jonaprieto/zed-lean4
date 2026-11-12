//! All routing decisions. No I/O: feed events in, get actions out.
use crate::{bridge, cursor, hover, init, msg};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

pub const KEEPALIVE: Duration = Duration::from_secs(10);

/// The file a notification is about: `uri` in diagnostics, `textDocument.uri`
/// in Lean's file progress.
fn notification_uri(params: &Value) -> Option<&str> {
    params["uri"]
        .as_str()
        .or_else(|| params["textDocument"]["uri"].as_str())
}

#[derive(Debug)]
pub enum Event {
    Tick,
    PageClosed(u64),
    FromBrowser(u64, Value),
    FromClient(Value),
    FromServer(Value),
}

#[derive(Debug, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub enum Action {
    ToBrowser(Value),
    ToClient(Value),
    ToServer(Value),
}

#[derive(Default)]
pub struct Router {
    next_id: u64,
    sessions: HashMap<String, Session>,
    location: Option<Value>,
    initialize_id: Option<String>,
    initialize_result: Option<Value>,
    latest: HashMap<String, HashMap<String, Value>>,
    /// Proxy request id -> response destination.
    pending: HashMap<String, Pending>,
    /// Client hover id key -> parts received so far.
    hovers: HashMap<String, HoverJoin>,
}

enum Pending {
    Client {
        id: String,
    },
    Page {
        id: String,
    },
    Connect {
        id: String,
        uri: String,
        connection: u64,
    },
    Goal(String),
}

struct Session {
    uri: String,
    id: Value,
    connection: u64,
}

struct HoverJoin {
    goal_id: String,
    response: Option<Value>,
    goal: Option<Option<String>>,
}

impl Router {
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        match event {
            Event::Tick => self.tick(),
            Event::PageClosed(connection) => {
                self.sessions
                    .retain(|_, session| session.connection != connection);
                self.pending.retain(|_, pending| {
                    !matches!(pending, Pending::Connect { connection: owner, .. } if *owner == connection)
                });
                vec![]
            }
            Event::FromBrowser(connection, m) => self.on_browser(connection, m),
            Event::FromClient(m) => self.on_client(init::rewrite_request(m)),
            Event::FromServer(m) => self.on_server(m),
        }
    }

    fn proxy_id(&mut self) -> String {
        self.next_id += 1;
        format!("{}{}", msg::PROXY_ID_PREFIX, self.next_id)
    }

    fn on_client(&mut self, m: Value) -> Vec<Action> {
        if msg::is_request(&m) && msg::method(&m) == Some("initialize") {
            self.initialize_id = Some(msg::id_key(&m["id"]));
        }
        if let Some(loc) = cursor::location(&m)
            && self.location.as_ref() != Some(&loc)
        {
            self.location = Some(loc.clone());
            let mut out = self.client_message(m);
            out.push(Action::ToBrowser(bridge::event(
                "changedCursorLocation",
                json!([loc]),
            )));
            return out;
        }
        if !msg::is_request(&m)
            && let Some(method) = msg::method(&m)
        {
            if method == "textDocument/didClose"
                && let Some(uri) = m["params"]["textDocument"]["uri"].as_str()
            {
                self.sessions.retain(|_, session| session.uri != uri);
                for values in self.latest.values_mut() {
                    values.remove(uri);
                }
            }
            let copy = bridge::event("sentClientNotification", json!([method, m["params"]]));
            let closed = method == "textDocument/didClose"
                && self
                    .location
                    .as_ref()
                    .is_some_and(|loc| loc["uri"] == m["params"]["textDocument"]["uri"]);
            let mut out = self.client_message(m);
            out.push(Action::ToBrowser(copy));
            if closed {
                self.location = None;
                out.push(Action::ToBrowser(bridge::event(
                    "changedCursorLocation",
                    json!([null]),
                )));
            }
            return out;
        }
        self.client_message(m)
    }

    fn client_message(&mut self, m: Value) -> Vec<Action> {
        if let Some(own) = msg::proxy_response_id(&m) {
            if matches!(self.pending.get(own), Some(Pending::Client { .. }))
                && let Some(Pending::Client { id }) = self.pending.remove(own)
            {
                return vec![Action::ToBrowser(bridge::relay(&id, &m))];
            }
            return vec![];
        }
        if msg::is_request(&m) && msg::method(&m) == Some("textDocument/hover") {
            let (Some(doc), Some(pos)) =
                (m["params"].get("textDocument"), m["params"].get("position"))
            else {
                return vec![Action::ToServer(m)];
            };
            let params = json!({"textDocument": doc, "position": pos});
            let key = msg::id_key(&m["id"]);
            let id = self.proxy_id();
            let goal_req = msg::request(&id, "$/lean/plainGoal", params);
            let join = HoverJoin {
                goal_id: id.clone(),
                response: None,
                goal: None,
            };
            self.pending.insert(id, Pending::Goal(key.clone()));
            self.hovers.insert(key, join);
            return vec![Action::ToServer(m), Action::ToServer(goal_req)];
        }
        if msg::method(&m) == Some("$/cancelRequest")
            && let Some(join) = self.hovers.get(&msg::id_key(&m["params"]["id"]))
        {
            let goal_cancel = msg::notification("$/cancelRequest", json!({"id": join.goal_id}));
            return vec![Action::ToServer(m), Action::ToServer(goal_cancel)];
        }
        vec![Action::ToServer(m)]
    }

    fn on_server(&mut self, m: Value) -> Vec<Action> {
        if msg::is_response(&m) && self.initialize_id.as_ref() == Some(&msg::id_key(&m["id"])) {
            self.initialize_id = None;
            self.initialize_result = m.get("result").filter(|v| v.is_object()).cloned();
        }
        if !msg::is_request(&m)
            && let Some(method) = msg::method(&m)
        {
            if matches!(
                method,
                "textDocument/publishDiagnostics" | "$/lean/fileProgress"
            ) && let Some(uri) = notification_uri(&m["params"])
            {
                self.latest
                    .entry(method.into())
                    .or_default()
                    .insert(uri.into(), m["params"].clone());
            }
            let copy = bridge::event("gotServerNotification", json!([method, m["params"]]));
            return vec![Action::ToClient(m), Action::ToBrowser(copy)];
        }
        if !msg::is_response(&m) {
            return vec![Action::ToClient(m)];
        }
        if let Some(own) = msg::proxy_response_id(&m) {
            if matches!(self.pending.get(own), Some(Pending::Client { .. })) {
                return vec![];
            }
            return match self.pending.remove(own) {
                Some(Pending::Goal(key)) => {
                    if let Some(join) = self.hovers.get_mut(&key) {
                        join.goal = Some(hover::rendered_goal(&m));
                    }
                    self.finish_hover(&key)
                }
                Some(Pending::Page { id }) => vec![Action::ToBrowser(bridge::relay(&id, &m))],
                Some(Pending::Connect {
                    id,
                    uri,
                    connection,
                }) => {
                    if m.get("error").is_some() {
                        return vec![Action::ToBrowser(bridge::relay(&id, &m))];
                    }
                    let sid = &m["result"]["sessionId"];
                    if !sid.is_string() && !sid.is_number() {
                        return vec![Action::ToBrowser(bridge::reply_error(
                            &id,
                            -32603,
                            "missing RPC session id",
                        ))];
                    }
                    self.sessions.insert(
                        msg::id_key(sid),
                        Session {
                            uri,
                            id: sid.clone(),
                            connection,
                        },
                    );
                    vec![Action::ToBrowser(bridge::reply(&id, sid.clone()))]
                }
                Some(Pending::Client { .. }) | None => vec![],
            };
        }
        let key = msg::id_key(&m["id"]);
        match self.hovers.get_mut(&key) {
            Some(_) if m.get("error").is_some() => {
                if let Some(join) = self.hovers.remove(&key) {
                    self.pending.remove(&join.goal_id);
                }
                vec![Action::ToClient(m)]
            }
            Some(join) => {
                join.response = Some(m);
                self.finish_hover(&key)
            }
            None => vec![Action::ToClient(m)],
        }
    }

    fn on_browser(&mut self, connection: u64, m: Value) -> Vec<Action> {
        let Some(call) = bridge::Call::parse(&m) else {
            return m["id"]
                .as_str()
                .map(|id| {
                    vec![Action::ToBrowser(bridge::reply_error(
                        id,
                        -32600,
                        "invalid page call",
                    ))]
                })
                .unwrap_or_default();
        };
        match (call.name.as_str(), call.args.as_slice()) {
            ("insertText", [text, kind, pos]) if text.is_string() && kind.is_string() => {
                let edit = cursor::insert_edit(
                    text.as_str().unwrap(),
                    kind.as_str().unwrap(),
                    (!pos.is_null()).then_some(pos),
                    self.location.as_ref(),
                );
                match edit {
                    Some(edit) => {
                        self.request_client(call.id, "workspace/applyEdit", json!({"edit":edit}))
                    }
                    None => vec![Action::ToBrowser(bridge::reply_error(
                        &call.id,
                        -32602,
                        "no valid insertion position",
                    ))],
                }
            }
            ("applyEdit", [edit]) if edit.is_object() => {
                self.request_client(call.id, "workspace/applyEdit", json!({"edit":edit}))
            }
            ("showDocument", [params]) if params["uri"].is_string() => {
                let mut params = params.clone();
                params["external"] = json!(false);
                self.request_client(call.id, "window/showDocument", params)
            }
            ("restartFile", [uri]) if uri.is_string() => {
                tracing::info!("restartFile is not implemented yet");
                vec![Action::ToBrowser(bridge::reply(&call.id, Value::Null))]
            }
            ("createRpcSession", [uri]) if uri.is_string() => {
                let id = self.proxy_id();
                self.pending.insert(
                    id.clone(),
                    Pending::Connect {
                        id: call.id,
                        uri: uri.as_str().unwrap().into(),
                        connection,
                    },
                );
                vec![Action::ToServer(msg::request(
                    &id,
                    "$/lean/rpc/connect",
                    json!({"uri":uri}),
                ))]
            }
            ("closeRpcSession", [sid]) => {
                self.sessions.remove(&msg::id_key(sid));
                vec![Action::ToBrowser(bridge::reply(&call.id, Value::Null))]
            }
            ("hello", []) => vec![Action::ToBrowser(bridge::reply(
                &call.id,
                json!({"location":self.location,"initializeResult":self.initialize_result}),
            ))],
            ("subscribeServerNotifications", [method]) if method.is_string() => {
                let values: Vec<Value> = self
                    .latest
                    .get(method.as_str().unwrap())
                    .into_iter()
                    .flat_map(|values| values.values().cloned())
                    .collect();
                vec![Action::ToBrowser(bridge::reply(&call.id, json!(values)))]
            }
            ("sendClientRequest", [uri, method, params])
                if uri.is_string()
                    && method.is_string()
                    && method.as_str().is_some_and(|m| m.starts_with("$/lean/")) =>
            {
                let id = self.proxy_id();
                self.pending
                    .insert(id.clone(), Pending::Page { id: call.id });
                vec![Action::ToServer(msg::request(
                    &id,
                    method.as_str().unwrap(),
                    params.clone(),
                ))]
            }
            ("sendClientNotification", [uri, method, params])
                if uri.is_string()
                    && method.is_string()
                    && method.as_str().is_some_and(|m| m.starts_with("$/lean/")) =>
            {
                vec![
                    Action::ToServer(msg::notification(method.as_str().unwrap(), params.clone())),
                    Action::ToBrowser(bridge::reply(&call.id, Value::Null)),
                ]
            }
            _ => vec![Action::ToBrowser(bridge::reply_error(
                &call.id,
                -32601,
                "unsupported or malformed page call",
            ))],
        }
    }

    fn request_client(&mut self, page_id: String, method: &str, params: Value) -> Vec<Action> {
        let id = self.proxy_id();
        self.pending
            .insert(id.clone(), Pending::Client { id: page_id });
        vec![Action::ToClient(msg::request(&id, method, params))]
    }

    fn tick(&self) -> Vec<Action> {
        self.sessions
            .values()
            .map(|session| {
                Action::ToServer(msg::notification(
                    "$/lean/rpc/keepAlive",
                    json!({"uri":session.uri,"sessionId":session.id}),
                ))
            })
            .collect()
    }

    fn finish_hover(&mut self, key: &str) -> Vec<Action> {
        let ready = self
            .hovers
            .get(key)
            .is_some_and(|j| j.response.is_some() && j.goal.is_some());
        if !ready {
            return vec![];
        }
        let join = self.hovers.remove(key).expect("checked above");
        vec![Action::ToClient(hover::merge_hover(
            join.response.unwrap(),
            join.goal.unwrap(),
        ))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn passes_unknown_traffic_through_untouched() {
        let mut r = Router::default();
        let req = json!({"jsonrpc": "2.0", "id": 3, "method": "textDocument/definition", "params": {"extra": [1]}});
        let resp = json!({"jsonrpc": "2.0", "id": 3, "result": null});
        let note = json!({"jsonrpc": "2.0", "method": "$/lean/fileProgress", "params": {}});
        assert_eq!(
            r.handle(Event::FromClient(req.clone())),
            vec![Action::ToServer(req)]
        );
        assert_eq!(
            r.handle(Event::FromServer(resp.clone())),
            vec![Action::ToClient(resp)]
        );
        assert_eq!(
            r.handle(Event::FromServer(note.clone())),
            vec![
                Action::ToClient(note),
                Action::ToBrowser(bridge::event(
                    "gotServerNotification",
                    json!(["$/lean/fileProgress", {}])
                ))
            ]
        );
    }

    fn hover_req(id: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "textDocument/hover",
               "params": {"textDocument": {"uri": "file:///F.lean"}, "position": {"line": 1, "character": 2}}})
    }
    fn hover_resp(id: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "result": {"contents": {"kind": "markdown", "value": "doc"}}})
    }
    fn goal_resp(id: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "result": {"goals": ["⊢ True"], "rendered": "G"}})
    }

    #[test]
    fn hover_also_asks_for_plain_goal() {
        let mut r = Router::default();
        let out = r.handle(Event::FromClient(hover_req(json!(7))));
        assert_eq!(
            out,
            vec![
                Action::ToServer(hover_req(json!(7))),
                Action::ToServer(
                    json!({"jsonrpc": "2.0", "id": "lean4-ide:1", "method": "$/lean/plainGoal",
                "params": {"textDocument": {"uri": "file:///F.lean"}, "position": {"line": 1, "character": 2}}})
                ),
            ]
        );
    }

    #[test]
    fn merged_hover_sent_once_both_arrive_in_either_order() {
        for goal_first in [false, true] {
            let mut r = Router::default();
            r.handle(Event::FromClient(hover_req(json!(7))));
            let (a, b) = (
                Event::FromServer(hover_resp(json!(7))),
                Event::FromServer(goal_resp("lean4-ide:1")),
            );
            let (first, second) = if goal_first { (b, a) } else { (a, b) };
            assert_eq!(r.handle(first), vec![]);
            assert_eq!(
                r.handle(second),
                vec![Action::ToClient(
                    json!({"jsonrpc": "2.0", "id": 7, "result": {"contents": {"kind": "markdown", "value": "doc\n\n---\n\nG"}}})
                )]
            );
        }
    }

    #[test]
    fn string_client_ids_do_not_collide_with_numbers() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!("7"))));
        r.handle(Event::FromServer(goal_resp("lean4-ide:1")));
        let unrelated = json!({"jsonrpc": "2.0", "id": 7, "result": null});
        assert_eq!(
            r.handle(Event::FromServer(unrelated.clone())),
            vec![Action::ToClient(unrelated)]
        );
    }

    #[test]
    fn proxy_responses_are_never_forwarded() {
        let mut r = Router::default();
        assert_eq!(
            r.handle(Event::FromServer(goal_resp("lean4-ide:99"))),
            vec![]
        );
    }

    #[test]
    fn hover_error_passes_through_unchanged() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!(7))));
        r.handle(Event::FromServer(goal_resp("lean4-ide:1")));
        let err =
            json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -32800, "message": "cancelled"}});
        assert_eq!(
            r.handle(Event::FromServer(err.clone())),
            vec![Action::ToClient(err)]
        );
    }

    #[test]
    fn null_hover_result_shows_goal_alone() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!(7))));
        r.handle(Event::FromServer(goal_resp("lean4-ide:1")));
        let out = r.handle(Event::FromServer(
            json!({"jsonrpc": "2.0", "id": 7, "result": null}),
        ));
        assert_eq!(
            out,
            vec![Action::ToClient(
                json!({"jsonrpc": "2.0", "id": 7, "result": {"contents": {"kind": "markdown", "value": "G"}}})
            )]
        );
    }

    #[test]
    fn plain_goal_error_keeps_hover() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!(7))));
        r.handle(Event::FromServer(json!({"jsonrpc": "2.0", "id": "lean4-ide:1", "error": {"code": -32801, "message": "modified"}})));
        assert_eq!(
            r.handle(Event::FromServer(hover_resp(json!(7)))),
            vec![Action::ToClient(hover_resp(json!(7)))]
        );
    }

    #[test]
    fn hover_without_position_skips_plain_goal() {
        let mut r = Router::default();
        let bare = json!({"jsonrpc": "2.0", "id": 8, "method": "textDocument/hover", "params": {}});
        assert_eq!(
            r.handle(Event::FromClient(bare.clone())),
            vec![Action::ToServer(bare)]
        );
        let resp = json!({"jsonrpc": "2.0", "id": 8, "result": null});
        assert_eq!(
            r.handle(Event::FromServer(resp.clone())),
            vec![Action::ToClient(resp)]
        );
    }

    #[test]
    fn string_client_id_completes_its_own_join() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!("7"))));
        r.handle(Event::FromServer(goal_resp("lean4-ide:1")));
        let out = r.handle(Event::FromServer(hover_resp(json!("7"))));
        assert_eq!(
            out,
            vec![Action::ToClient(
                json!({"jsonrpc": "2.0", "id": "7", "result": {"contents": {"kind": "markdown", "value": "doc\n\n---\n\nG"}}})
            )]
        );
    }

    fn cancel(id: Value) -> Value {
        json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": id}})
    }

    #[test]
    fn cancelling_a_hover_also_cancels_its_plain_goal() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!(7))));
        assert_eq!(
            r.handle(Event::FromClient(cancel(json!(7)))),
            vec![
                Action::ToServer(cancel(json!(7))),
                Action::ToServer(cancel(json!("lean4-ide:1"))),
                Action::ToBrowser(bridge::event(
                    "sentClientNotification",
                    json!(["$/cancelRequest", {"id":7}])
                )),
            ]
        );
    }

    #[test]
    fn other_cancels_pass_through_alone() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!(7))));
        assert_eq!(
            r.handle(Event::FromClient(cancel(json!(9)))),
            vec![
                Action::ToServer(cancel(json!(9))),
                Action::ToBrowser(bridge::event(
                    "sentClientNotification",
                    json!(["$/cancelRequest", {"id":9}])
                ))
            ]
        );
    }

    #[test]
    fn hover_error_is_sent_without_waiting_for_the_goal() {
        let mut r = Router::default();
        r.handle(Event::FromClient(hover_req(json!(7))));
        let err =
            json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -32800, "message": "cancelled"}});
        assert_eq!(
            r.handle(Event::FromServer(err.clone())),
            vec![Action::ToClient(err)]
        );
        assert_eq!(
            r.handle(Event::FromServer(goal_resp("lean4-ide:1"))),
            vec![]
        );
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;

    #[test]
    fn goal_pending_is_removed_on_early_hover_error() {
        let mut r = Router::default();
        r.handle(Event::FromClient(
            json!({"id": 7, "method": "textDocument/hover",
            "params": {"textDocument": {"uri": "file:///F.lean"},
            "position": {"line": 1, "character": 2}}}),
        ));
        assert!(matches!(
            r.pending.get("lean4-ide:1"),
            Some(Pending::Goal(_))
        ));
        r.handle(Event::FromServer(
            json!({"id": 7, "error": {"code": -32800}}),
        ));
        assert!(r.pending.is_empty());
        assert!(
            r.handle(Event::FromServer(
                json!({"id": "lean4-ide:1", "result": null})
            ))
            .is_empty()
        );
    }
}

#[cfg(test)]
mod reverse_tests {
    use super::*;

    #[test]
    fn consumes_only_proxy_responses_from_zed() {
        let mut r = Router::default();
        for m in [
            json!({"id": "lean4-ide:9", "result": {"applied": true}}),
            json!({"id": "lean4-ide:9", "error": {"code": -1}}),
        ] {
            assert!(r.handle(Event::FromClient(m)).is_empty());
        }
        for m in [
            json!({"id": 9, "result": null}),
            json!({"id": "lean4-ide:9", "method": "unknown"}),
        ] {
            assert_eq!(
                r.handle(Event::FromClient(m.clone())),
                vec![Action::ToServer(m)]
            );
        }
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::*;

    #[test]
    fn routes_two_tabs_out_of_order_and_preserves_errors() {
        let mut r = Router::default();
        for (id, proxy) in [("a:1", "lean4-ide:1"), ("b:1", "lean4-ide:2")] {
            assert_eq!(
                r.handle(Event::FromBrowser(
                    1,
                    json!({"id":id,"call":"sendClientRequest",
                "args":["file:///F.lean","$/lean/rpc/call",{"x":1}]})
                )),
                vec![Action::ToServer(msg::request(
                    proxy,
                    "$/lean/rpc/call",
                    json!({"x":1})
                ))]
            );
        }
        let error = json!({"code":-32900,"message":"reconnect","data":{"x":2}});
        assert_eq!(
            r.handle(Event::FromServer(json!({"id":"lean4-ide:2","error":error}))),
            vec![Action::ToBrowser(json!({"id":"b:1","error":error}))]
        );
        assert_eq!(
            r.handle(Event::FromServer(json!({"id":"lean4-ide:1","result":null}))),
            vec![Action::ToBrowser(json!({"id":"a:1","result":null}))]
        );
        assert!(r.pending.is_empty());
    }

    #[test]
    fn notifications_acknowledge_and_bad_calls_fail_locally() {
        let mut r = Router::default();
        assert_eq!(
            r.handle(Event::FromBrowser(
                1,
                json!({"id":"a:2","call":"sendClientNotification",
            "args":["file:///F.lean","$/lean/rpc/release",{}]})
            )),
            vec![
                Action::ToServer(msg::notification("$/lean/rpc/release", json!({}))),
                Action::ToBrowser(json!({"id":"a:2","result":null}))
            ]
        );
        for m in [
            json!({"id":"a:3","call":"sendClientRequest","args":[]}),
            json!({"id":"a:4","call":"runTestScript","args":["bad"]}),
            json!({"id":"a:5","call":7,"args":[]}),
            json!({"id":"a:6","call":"sendClientNotification","args":["file:///F.lean","exit",null]}),
            json!({"id":"a:7","call":"sendClientRequest","args":["file:///F.lean","shutdown",null]}),
        ] {
            let out = r.handle(Event::FromBrowser(1, m));
            assert!(matches!(out.as_slice(), [Action::ToBrowser(v)] if v.get("error").is_some()));
        }
        assert!(r.handle(Event::FromBrowser(1, json!([]))).is_empty());
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[test]
    fn latest_notifications_replay_and_close_forgets() {
        let mut r = Router::default();
        for method in ["textDocument/publishDiagnostics", "$/lean/fileProgress"] {
            let params = |version: u64| match method {
                "$/lean/fileProgress" => {
                    json!({"textDocument":{"uri":"file:///F.lean","version":version},"processing":[]})
                }
                _ => json!({"uri":"file:///F.lean","version":version}),
            };
            for version in [1, 2] {
                let params = params(version);
                let m = msg::notification(method, params.clone());
                assert_eq!(
                    r.handle(Event::FromServer(m.clone())),
                    vec![
                        Action::ToClient(m),
                        Action::ToBrowser(bridge::event(
                            "gotServerNotification",
                            json!([method, params])
                        ))
                    ]
                );
            }
            assert_eq!(
                r.handle(Event::FromBrowser(
                    1,
                    json!({"id":"a:1","call":"subscribeServerNotifications","args":[method]})
                )),
                vec![Action::ToBrowser(bridge::reply("a:1", json!([params(2)])))]
            );
        }
        let m = msg::notification(
            "textDocument/didClose",
            json!({"textDocument":{"uri":"file:///F.lean"}}),
        );
        let out = r.handle(Event::FromClient(m.clone()));
        assert_eq!(out[0], Action::ToServer(m));
        assert!(matches!(&out[1],Action::ToBrowser(v) if v["event"]=="sentClientNotification"));
        assert!(r.latest.values().all(HashMap::is_empty));
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    #[test]
    fn hello_replays_initialize_and_cursor_without_consuming_lsp() {
        let mut r = Router::default();
        r.handle(Event::FromClient(
            json!({"id":1,"method":"initialize","params":{}}),
        ));
        let result = json!({"capabilities":{"experimental":{"rpcProvider":{"rpcWireFormat":"v1"}}},"serverInfo":{"version":"4.33.1"}});
        let response = json!({"id":1,"result":result});
        assert_eq!(
            r.handle(Event::FromServer(response.clone())),
            vec![Action::ToClient(response)]
        );
        let m = json!({"id":2,"method":"textDocument/documentHighlight","params":{"textDocument":{"uri":"file:///F.lean"},"position":{"line":1,"character":2}}});
        let loc = json!({"uri":"file:///F.lean","range":{"start":{"line":1,"character":2},"end":{"line":1,"character":2}}});
        assert_eq!(
            r.handle(Event::FromClient(m.clone())),
            vec![
                Action::ToServer(m.clone()),
                Action::ToBrowser(bridge::event("changedCursorLocation", json!([loc])))
            ]
        );
        assert_eq!(
            r.handle(Event::FromClient(m.clone())),
            vec![Action::ToServer(m)]
        );
        assert_eq!(
            r.handle(Event::FromBrowser(
                1,
                json!({"id":"a:1","call":"hello","args":[]})
            )),
            vec![Action::ToBrowser(bridge::reply(
                "a:1",
                json!({"location":loc,"initializeResult":result})
            ))]
        );
        r.handle(Event::FromClient(msg::notification(
            "textDocument/didClose",
            json!({"textDocument":{"uri":"file:///F.lean"}}),
        )));
        assert!(r.location.is_none());
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn opaque_sessions_keep_alive_close_and_reconnect() {
        let mut r = Router::default();
        let create = json!({"id":"a:1","call":"createRpcSession","args":["file:///F.lean"]});
        assert_eq!(
            r.handle(Event::FromBrowser(1, create.clone())),
            vec![Action::ToServer(msg::request(
                "lean4-ide:1",
                "$/lean/rpc/connect",
                json!({"uri":"file:///F.lean"})
            ))]
        );
        let sid = json!("18446744073709551615");
        assert_eq!(
            r.handle(Event::FromServer(
                json!({"id":"lean4-ide:1","result":{"sessionId":sid}})
            )),
            vec![Action::ToBrowser(bridge::reply("a:1", sid.clone()))]
        );
        let keep = vec![Action::ToServer(msg::notification(
            "$/lean/rpc/keepAlive",
            json!({"uri":"file:///F.lean","sessionId":sid}),
        ))];
        assert_eq!(r.handle(Event::Tick), keep);
        assert_eq!(r.handle(Event::Tick), keep);
        r.handle(Event::FromBrowser(
            1,
            json!({"id":"a:2","call":"closeRpcSession","args":[sid]}),
        ));
        assert!(r.handle(Event::Tick).is_empty());
        r.handle(Event::FromBrowser(1, create));
        r.handle(Event::FromServer(
            json!({"id":"lean4-ide:2","result":{"sessionId":"new"}}),
        ));
        assert_eq!(r.sessions.len(), 1);
        r.handle(Event::FromBrowser(
            1,
            json!({"id":"a:3","call":"closeRpcSession","args":["new"]}),
        ));
        assert!(r.handle(Event::Tick).is_empty());
    }

    #[test]
    fn page_close_removes_only_owned_sessions_and_pending_connects() {
        let mut r = Router::default();
        for (connection, page, proxy, session) in [
            (1, "a:1", "lean4-ide:1", "first"),
            (2, "b:1", "lean4-ide:2", "second"),
        ] {
            r.handle(Event::FromBrowser(
                connection,
                json!({"id":page,"call":"createRpcSession","args":["file:///F.lean"]}),
            ));
            r.handle(Event::FromServer(
                json!({"id":proxy,"result":{"sessionId":session}}),
            ));
        }
        r.handle(Event::FromBrowser(
            1,
            json!({"id":"a:2","call":"createRpcSession","args":["file:///F.lean"]}),
        ));
        assert_eq!(r.handle(Event::Tick).len(), 2);
        assert!(r.handle(Event::PageClosed(1)).is_empty());
        assert!(r.pending.is_empty());
        assert!(
            r.handle(Event::FromServer(
                json!({"id":"lean4-ide:3","result":{"sessionId":"late"}})
            ))
            .is_empty()
        );
        let keep = vec![Action::ToServer(msg::notification(
            "$/lean/rpc/keepAlive",
            json!({"uri":"file:///F.lean","sessionId":"second"}),
        ))];
        assert_eq!(r.handle(Event::Tick), keep);
        assert_eq!(r.handle(Event::Tick), keep);
        assert_eq!(r.sessions.len(), 1);
        assert!(r.handle(Event::PageClosed(2)).is_empty());
        assert!(r.handle(Event::Tick).is_empty());
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn zed_answers_go_to_pages_and_never_to_lean() {
        let mut r = Router::default();
        let edit = json!({"changes":{}});
        assert_eq!(
            r.handle(Event::FromBrowser(
                1,
                json!({"id":"a:1","call":"applyEdit","args":[edit]})
            )),
            vec![Action::ToClient(msg::request(
                "lean4-ide:1",
                "workspace/applyEdit",
                json!({"edit":edit})
            ))]
        );
        let response =
            json!({"id":"lean4-ide:1","result":{"applied":false,"failureReason":"read only"}});
        assert_eq!(
            r.handle(Event::FromClient(response)),
            vec![Action::ToBrowser(bridge::reply(
                "a:1",
                json!({"applied":false,"failureReason":"read only"})
            ))]
        );
        let show = json!({"uri":"file:///F.lean","external":true,"selection":{"start":{"line":1,"character":2},"end":{"line":1,"character":3}}});
        assert_eq!(
            r.handle(Event::FromBrowser(
                1,
                json!({"id":"a:2","call":"showDocument","args":[show]})
            )),
            vec![Action::ToClient(msg::request(
                "lean4-ide:2",
                "window/showDocument",
                json!({"uri":"file:///F.lean","selection":show["selection"],"external":false})
            ))]
        );
        let error = json!({"code":-1,"message":"denied"});
        assert_eq!(
            r.handle(Event::FromClient(json!({"id":"lean4-ide:2","error":error}))),
            vec![Action::ToBrowser(json!({"id":"a:2","error":error}))]
        );
        assert!(r.pending.is_empty());
        assert_eq!(
            r.handle(Event::FromBrowser(
                1,
                json!({"id":"a:3","call":"restartFile","args":["file:///F.lean"]})
            )),
            vec![Action::ToBrowser(bridge::reply("a:3", Value::Null))]
        );
    }
}
