//! Page call ids and the Lean RPC sessions the terminal infoview opens and calls into.
use std::collections::HashMap;

use serde_json::{Value, json};

use super::{app::Effect, goals, lean};

/// An RPC session and the file it is for: where the references it hands out live.
#[derive(Clone, PartialEq)]
pub(super) struct Session {
    pub(super) uri: String,
    pub(super) id: Value,
}

/// An RPC result and the session that holds its references.
pub(super) struct Held {
    pub(super) session: Session,
    pub(super) value: Value,
}

impl Held {
    /// `value` as held by `session`, when there are both.
    pub(super) fn new(session: Option<Session>, value: Option<Value>) -> Option<Self> {
        Some(Self {
            session: session?,
            value: value?,
        })
    }
}

/// Numbers page calls and keeps track of the RPC sessions and the calls into them.
pub(super) struct Rpc {
    id_prefix: String,
    last_id: u64,
    /// RPC session per file URI.
    sessions: HashMap<String, Value>,
    /// Pending `createRpcSession` calls and the file each is for.
    connects: HashMap<String, String>,
    /// The session of every RPC call still in flight, by id.
    inflight: HashMap<String, Session>,
}

impl Rpc {
    pub(super) fn new(id_prefix: String) -> Self {
        Self {
            id_prefix,
            last_id: 0,
            sessions: HashMap::new(),
            connects: HashMap::new(),
            inflight: HashMap::new(),
        }
    }

    /// Forgets every session and call; ids keep counting.
    pub(super) fn reset(&mut self) {
        self.sessions.clear();
        self.connects.clear();
        self.inflight.clear();
    }

    /// A page call `name` with a fresh id.
    pub(super) fn call(&mut self, name: &str, args: Value) -> (String, Effect) {
        self.last_id += 1;
        let id = format!("{}:{}", self.id_prefix, self.last_id);
        (
            id.clone(),
            Effect::Send(json!({"id":id,"call":name,"args":args})),
        )
    }

    /// The open session for the file `uri`.
    pub(super) fn session(&self, uri: &str) -> Option<Session> {
        self.sessions.get(uri).map(|id| Session {
            uri: uri.to_owned(),
            id: id.clone(),
        })
    }

    /// Opens a session for the file `uri` unless one is opening already.
    pub(super) fn connect(&mut self, uri: String) -> Vec<Effect> {
        if self.connects.values().any(|u| *u == uri) {
            return vec![];
        }
        let (id, connect) = self.call(lean::CREATE_RPC_SESSION, json!([uri]));
        self.connects.insert(id, uri);
        vec![connect]
    }

    /// The file the `createRpcSession` reply `id` is for, if it is one; keeps
    /// the session it opened unless the reply is an error.
    pub(super) fn connected(&mut self, id: &str, reply: &Value) -> Option<String> {
        let uri = self.connects.remove(id)?;
        if reply.get("error").is_none() {
            self.sessions.insert(uri.clone(), reply["result"].clone());
        }
        Some(uri)
    }

    /// A widget RPC call into `session` at `position`.
    pub(super) fn request(
        &mut self,
        session: &Session,
        position: Value,
        method: &str,
        params: Value,
    ) -> (String, Effect) {
        let uri = &session.uri;
        let call = json!({"textDocument":{"uri":uri},"position":position,
            "sessionId":session.id,"method":method,"params":params});
        let (id, call) = self.call(
            lean::SEND_CLIENT_REQUEST,
            json!([uri, lean::RPC_CALL, call]),
        );
        self.inflight.insert(id.clone(), session.clone());
        (id, call)
    }

    /// The session the RPC reply `id` came from, forgetting that session when
    /// the reply says Lean dropped it.
    pub(super) fn answered(&mut self, id: &str, reply: &Value) -> Option<Session> {
        let session = self.inflight.remove(id);
        if reply["error"]["code"].as_i64() == Some(lean::RPC_NEEDS_RECONNECT)
            && let Some(session) = &session
            && self.is_open(session)
        {
            self.sessions.remove(&session.uri);
        }
        session
    }

    /// Lets Lean free the references inside a result the view no longer shows.
    pub(super) fn release(&mut self, old: Option<Held>) -> Vec<Effect> {
        let Some(Held { session, value }) = old else {
            return vec![];
        };
        let mut refs = Vec::new();
        goals::refs(&value, &mut refs);
        if refs.is_empty() || !self.is_open(&session) {
            return vec![];
        }
        let params = json!({"uri":session.uri,"sessionId":session.id,"refs":refs});
        let args = json!([session.uri, lean::RPC_RELEASE, params]);
        let (_, release) = self.call(lean::SEND_CLIENT_NOTIFICATION, args);
        vec![release]
    }

    fn is_open(&self, session: &Session) -> bool {
        self.sessions.get(&session.uri) == Some(&session.id)
    }
}
