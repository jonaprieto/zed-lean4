//! All routing decisions. No I/O: feed events in, get actions out.
use crate::{hover, msg};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug)]
pub enum Event {
    FromClient(Value),
    FromServer(Value),
}

#[derive(Debug, PartialEq)]
pub enum Action {
    ToClient(Value),
    ToServer(Value),
}

#[derive(Default)]
pub struct Router {
    next_id: u64,
    /// Proxy plainGoal id -> client hover id key.
    goal_for: HashMap<String, String>,
    /// Client hover id key -> parts received so far.
    hovers: HashMap<String, HoverJoin>,
}

#[derive(Default)]
struct HoverJoin {
    response: Option<Value>,
    goal: Option<Option<String>>,
}

impl Router {
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        match event {
            Event::FromClient(m) => self.on_client(m),
            Event::FromServer(m) => self.on_server(m),
        }
    }

    fn proxy_id(&mut self) -> String {
        self.next_id += 1;
        format!("{}{}", msg::PROXY_ID_PREFIX, self.next_id)
    }

    fn on_client(&mut self, m: Value) -> Vec<Action> {
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
            self.goal_for.insert(id, key.clone());
            self.hovers.insert(key, HoverJoin::default());
            return vec![Action::ToServer(m), Action::ToServer(goal_req)];
        }
        vec![Action::ToServer(m)]
    }

    fn on_server(&mut self, m: Value) -> Vec<Action> {
        if !msg::is_response(&m) {
            return vec![Action::ToClient(m)];
        }
        if let Some(own) = m["id"]
            .as_str()
            .filter(|id| id.starts_with(msg::PROXY_ID_PREFIX))
        {
            let Some(key) = self.goal_for.remove(own) else {
                return vec![];
            };
            if let Some(join) = self.hovers.get_mut(&key) {
                join.goal = Some(hover::rendered_goal(&m));
            }
            return self.finish_hover(&key);
        }
        let key = msg::id_key(&m["id"]);
        match self.hovers.get_mut(&key) {
            Some(join) => {
                join.response = Some(m);
                self.finish_hover(&key)
            }
            None => vec![Action::ToClient(m)],
        }
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
            vec![Action::ToClient(note)]
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
}
