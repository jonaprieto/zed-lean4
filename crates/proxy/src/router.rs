//! All routing decisions. No I/O: feed events in, get actions out.
use serde_json::Value;

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
pub struct Router {}

impl Router {
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        match event {
            Event::FromClient(m) => vec![Action::ToServer(m)],
            Event::FromServer(m) => vec![Action::ToClient(m)],
        }
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
}
