use std::ops::{Deref, DerefMut};

use super::{
    draw::{KEYS, PICKED_KEYS},
    *,
};
use ratatui::{Terminal, backend::TestBackend};

const U: &str = "file:///p/F.lean";
const ID_PREFIX: &str = "tui-1";
/// JSON-RPC `InvalidParams`.
const INVALID_PARAMS: i64 = -32602;

/// An `App` that keeps every message it sent, so a test answers a request by
/// what it asks for, not by an id that depends on how many requests came before.
struct Tui {
    app: App,
    log: Vec<Value>,
}

impl Deref for Tui {
    type Target = App;

    fn deref(&self) -> &App {
        &self.app
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.app
    }
}

impl Tui {
    fn new() -> Self {
        Self {
            app: App::new(ID_PREFIX.into()),
            log: Vec::new(),
        }
    }

    fn connected(&mut self, page_url: String) -> Vec<Effect> {
        let effects = self.app.connected(page_url);
        self.record(effects)
    }

    fn on_message(&mut self, m: &Value) -> Vec<Effect> {
        let effects = self.app.on_message(m);
        self.record(effects)
    }

    fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let effects = self.app.on_key(key);
        self.record(effects)
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        let effects = self.app.on_mouse(mouse);
        self.record(effects)
    }

    fn record(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        for effect in &effects {
            if let Effect::Send(m) = effect {
                self.log.push(m.clone());
            }
        }
        effects
    }

    /// The id of the latest request for `method`, as `asks_for` names it.
    #[track_caller]
    fn id(&self, method: &str) -> String {
        let latest = self.log.iter().rev().find(|m| asks_for(m) == method);
        let Some(id) = latest.and_then(|m| m["id"].as_str()) else {
            panic!("no {method} request was sent");
        };
        id.to_owned()
    }

    /// Answers the latest request for `method` with `result`.
    #[track_caller]
    fn reply(&mut self, method: &str, result: Value) -> Vec<Effect> {
        let id = self.id(method);
        self.on_message(&json!({"id":id,"result":result}))
    }

    /// Fails the latest request for `method` with the error `code`.
    #[track_caller]
    fn fail(&mut self, method: &str, code: i64, message: &str) -> Vec<Effect> {
        let id = self.id(method);
        self.on_message(&json!({"id":id,"error":{"code":code,"message":message}}))
    }
}

/// What a page call asks for: the widget method of an RPC call, the LSP method
/// of another client request or notification, or else the call itself.
fn asks_for(m: &Value) -> &str {
    let call = m["call"].as_str().unwrap_or_default();
    let args = &m["args"];
    let method = if args[1] == lean::RPC_CALL {
        &args[2]["method"]
    } else if [lean::SEND_CLIENT_REQUEST, lean::SEND_CLIENT_NOTIFICATION].contains(&call) {
        &args[1]
    } else {
        return call;
    };
    method.as_str().unwrap_or_default()
}

fn loc(line: u64, character: u64) -> Value {
    json!({"uri":U,"range":{"start":{"line":line,"character":character},"end":{"line":line,"character":character}}})
}

/// The reply to `hello` when the cursor is at `location`.
fn hello(location: Value) -> Value {
    json!({"location":location,"initializeResult":{}})
}

/// The messages `effects` send, without their ids: tests answer by method.
fn sent(effects: Vec<Effect>) -> Vec<Value> {
    effects
        .into_iter()
        .map(|e| match e {
            Effect::Send(mut m) => {
                m.as_object_mut().unwrap().remove("id");
                m
            }
            other => panic!("expected a send, got {other:?}"),
        })
        .collect()
}

/// The page call `call`, as `sent` shows it.
fn page_call(call: &str, args: Value) -> Value {
    json!({"call":call,"args":args})
}

/// A widget RPC call into session `s1` at `position`.
fn rpc_call(position: &Value, method: &str, params: Value) -> Value {
    page_call(
        lean::SEND_CLIENT_REQUEST,
        json!([U, lean::RPC_CALL, {"textDocument":{"uri":U},"position":position,
            "sessionId":"s1","method":method,"params":params}]),
    )
}

/// Lets Lean free `refs` in session `s1`.
fn release(refs: Value) -> Value {
    page_call(
        lean::SEND_CLIENT_NOTIFICATION,
        json!([U, lean::RPC_RELEASE, {"uri":U,"sessionId":"s1","refs":refs}]),
    )
}

/// What showing `line:character` asks for: goals, expected type, the line's
/// messages and the hover.
fn goal_requests(line: u64, character: u64) -> Vec<Value> {
    let position = json!({"line":line,"character":character});
    let at = json!({"textDocument":{"uri":U},"position":position});
    let line_range = json!({"lineRange":{"start":line,"end":line + 1}});
    vec![
        rpc_call(&position, lean::GET_INTERACTIVE_GOALS, at.clone()),
        rpc_call(&position, lean::GET_INTERACTIVE_TERM_GOAL, at.clone()),
        rpc_call(&position, lean::GET_INTERACTIVE_DIAGNOSTICS, line_range),
        page_call(lean::SEND_CLIENT_REQUEST, json!([U, lean::HOVER, at])),
    ]
}

/// An interactive goal from `"h : A\n⊢ B"`-style text, without references.
fn goal(text: &str) -> Value {
    let mut lines: Vec<&str> = text.lines().collect();
    let target = lines.pop().unwrap().strip_prefix("⊢ ").unwrap();
    let hyps: Vec<Value> = lines
        .iter()
        .map(|l| {
            let (names, ty) = l.split_once(" : ").unwrap();
            json!({"names":names.split(' ').collect::<Vec<_>>(),"type":{"text":ty}})
        })
        .collect();
    json!({"hyps":hyps,"type":{"text":target}})
}

fn goals(texts: &[&str]) -> Value {
    json!({"goals": texts.iter().map(|t| goal(t)).collect::<Vec<_>>()})
}

/// Lean's hover over `Nat.add_zero`: its signature, a rule, its docstring.
fn hover() -> Value {
    json!({"contents":{"kind":"markdown",
        "value":"```lean\nNat.add_zero : ∀ (n : Nat), n + 0 = n\n```\n***\nAdding zero changes nothing."}})
}

/// Connected, hello answered with `loc(1, 2)`, session `s1` open.
fn app() -> Tui {
    let mut app = Tui::new();
    app.connected("http://127.0.0.1:1/?token=t".into());
    app.reply(lean::HELLO, hello(loc(1, 2)));
    app.reply(lean::CREATE_RPC_SESSION, json!("s1"));
    app
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

const WIDTH: u16 = 60;
const HEIGHT: u16 = 20;

fn screen(app: &App) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buf = terminal.backend().buffer();
    (0..HEIGHT)
        .map(|y| {
            let row: String = (0..WIDTH).map(|x| buf[(x, y)].symbol()).collect();
            row.trim_end().to_owned()
        })
        .collect()
}

/// The footer: the status, or the keys when there is none.
fn footer(app: &App) -> String {
    screen(app).pop().unwrap()
}

#[track_caller]
fn shows(app: &App, lines: &[&str]) {
    let screen = screen(app);
    let text = screen.join("\n");
    assert!(
        text.contains(&lines.join("\n")),
        "missing {lines:?} in\n{text}"
    );
}

#[track_caller]
fn hides(app: &App, line: &str) {
    let screen = screen(app);
    assert!(
        !screen.iter().any(|l| l == line),
        "unexpected {line:?} in\n{}",
        screen.join("\n")
    );
}

fn publish(line: u64, character: u64, end_line: u64, severity: u64, message: &str) -> Value {
    json!({"range":{"start":{"line":line,"character":character},"end":{"line":end_line,"character":0}},"severity":severity,"message":message})
}

#[test]
fn connected_sends_hello_then_subscribe() {
    let mut app = Tui::new();
    assert_eq!(
        sent(app.connected("http://h/?token=t".into())),
        [
            page_call(lean::HELLO, json!([])),
            page_call(
                lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
                json!([lean::PUBLISH_DIAGNOSTICS])
            ),
        ]
    );
    assert_eq!(
        [
            app.id(lean::HELLO),
            app.id(lean::SUBSCRIBE_SERVER_NOTIFICATIONS)
        ],
        ["tui-1:1", "tui-1:2"]
    );
    assert_eq!(app.page_url(), Some("http://h/?token=t"));
}

#[test]
fn hello_location_requests_both_goals() {
    let mut app = Tui::new();
    app.connected("http://h/".into());
    let effects = app.reply(lean::HELLO, hello(loc(1, 2)));
    assert_eq!(
        sent(effects),
        [page_call(lean::CREATE_RPC_SESSION, json!([U]))]
    );
    let effects = app.reply(lean::CREATE_RPC_SESSION, json!("s1"));
    assert_eq!(sent(effects), goal_requests(1, 2));
}

#[test]
fn null_hello_location_sends_nothing_and_says_so() {
    let mut app = Tui::new();
    app.connected("http://h/".into());
    let effects = app.reply(lean::HELLO, hello(Value::Null));
    assert!(effects.is_empty());
    shows(&app, &["No Lean file at the cursor"]);
}

#[test]
fn diagnostics_come_from_subscribe_and_notifications() {
    let mut app = app();
    app.reply(
        lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
        json!([{"uri":U,"diagnostics":[publish(1, 2, 1, 2, "old")]}]),
    );
    shows(&app, &["▾ Messages (1)", "2:2 warning: old"]);
    app.on_message(&json!({"event":"gotServerNotification","args":["textDocument/publishDiagnostics",{"uri":U,"diagnostics":[publish(1, 0, 1, 1, "new")]}]}));
    shows(&app, &["▾ Messages (1)", "2:0 error: new"]);
    hides(&app, "2:2 warning: old");
    app.on_message(&json!({"event":"gotServerNotification","args":["$/lean/fileProgress",{"uri":U,"diagnostics":[]}]}));
    shows(&app, &["2:0 error: new"]);
}

#[test]
fn diagnostics_for_the_shown_file_refresh_its_goals() {
    let mut app = app();
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(4, 0)]}));
    let note = |uri: &str| json!({"event":"gotServerNotification","args":["textDocument/publishDiagnostics",{"uri":uri,"diagnostics":[]}]});
    assert!(app.on_message(&note("file:///p/G.lean")).is_empty());
    assert_eq!(sent(app.on_message(&note(U))), goal_requests(4, 0));
}

#[test]
fn cursor_moves_request_goals_unless_pinned() {
    let mut app = app();
    let moved = app.on_message(&json!({"event":"changedCursorLocation","args":[loc(4, 0)]}));
    assert_eq!(sent(moved), goal_requests(4, 0));
    shows(&app, &["F.lean:5:0"]);
    app.on_key(key('p'));
    let moved = app.on_message(&json!({"event":"changedCursorLocation","args":[loc(7, 1)]}));
    assert!(moved.is_empty());
    shows(&app, &["F.lean:5:0 [pinned]"]);
    app.on_key(key('p'));
    app.on_message(&json!({"event":"changedCursorLocation","args":[null]}));
    shows(&app, &["No Lean file at the cursor"]);
}

#[test]
fn goals_render_with_header_and_expected_type() {
    let mut app = app();
    app.reply(
        lean::GET_INTERACTIVE_GOALS,
        goals(&["n : Nat\n⊢ n + 0 = n"]),
    );
    app.reply(lean::GET_INTERACTIVE_TERM_GOAL, goal("⊢ Nat"));
    shows(
        &app,
        &[
            "F.lean:2:2",
            "",
            "▾ Tactic state",
            "1 goal",
            "n : Nat",
            "⊢ n + 0 = n",
        ],
    );
    shows(&app, &["▾ Expected type", "⊢ Nat"]);
}

#[test]
fn empty_null_and_several_goals() {
    let mut app = app();
    app.reply(lean::GET_INTERACTIVE_GOALS, goals(&[]));
    shows(&app, &["▾ Tactic state", "No goals"]);
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.reply(lean::GET_INTERACTIVE_GOALS, Value::Null);
    hides(&app, "▾ Tactic state");
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
    app.reply(lean::GET_INTERACTIVE_GOALS, goals(&["⊢ A", "⊢ B"]));
    shows(&app, &["2 goals", "▾ goal 1", "⊢ A", "", "▾ goal 2", "⊢ B"]);
}

#[test]
fn superseded_replies_are_ignored_and_errors_keep_goals() {
    let mut app = app();
    let superseded = app.id(lean::GET_INTERACTIVE_GOALS);
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.on_message(&json!({"id":superseded,"result":goals(&["⊢ old"])}));
    let foreign = app
        .id(lean::GET_INTERACTIVE_GOALS)
        .replacen(ID_PREFIX, "other", 1);
    app.on_message(&json!({"id":foreign,"result":goals(&["⊢ foreign"])}));
    hides(&app, "⊢ old");
    hides(&app, "⊢ foreign");
    app.reply(lean::GET_INTERACTIVE_GOALS, goals(&["⊢ new"]));
    shows(&app, &["⊢ new"]);
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
    app.fail(lean::GET_INTERACTIVE_GOALS, INVALID_PARAMS, "bad params");
    shows(&app, &["⊢ new"]);
    assert_eq!(footer(&app), "bad params");
}

#[test]
fn overtaken_and_cancelled_lookups_are_dropped_silently() {
    let mut app = app();
    for (method, code) in [
        (lean::GET_INTERACTIVE_GOALS, lean::CONTENT_MODIFIED),
        (lean::GET_INTERACTIVE_TERM_GOAL, lean::REQUEST_CANCELLED),
        (lean::HOVER, lean::CONTENT_MODIFIED),
    ] {
        app.fail(method, code, "File changed.");
        assert_eq!(footer(&app), KEYS);
    }
    // The slots were cleared: a late success is ignored too.
    app.reply(lean::GET_INTERACTIVE_GOALS, goals(&["⊢ late"]));
    hides(&app, "⊢ late");
}

#[test]
fn reconnect_clears_the_no_server_status() {
    let mut app = app();
    app.disconnected("no Lean server found; open a Lean file in Zed".into());
    assert_eq!(
        footer(&app),
        "no Lean server found; open a Lean file in Zed"
    );
    app.connected("http://127.0.0.1:1/?token=t".into());
    assert_eq!(footer(&app), KEYS);
}

#[test]
fn subscribe_reply_replaces_all_diagnostics() {
    let mut app = app();
    app.reply(
        lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
        json!([{"uri":U,"diagnostics":[publish(1, 0, 1, 1, "old")]}]),
    );
    shows(&app, &["2:0 error: old"]);
    app.connected("http://127.0.0.1:2/?token=t".into());
    app.reply(lean::SUBSCRIBE_SERVER_NOTIFICATIONS, json!([]));
    hides(&app, "2:0 error: old");
}

#[test]
fn pin_does_nothing_without_a_location() {
    let mut app = Tui::new();
    app.connected("http://127.0.0.1:1/?token=t".into());
    app.on_key(key('p'));
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(1, 2)]}));
    app.reply(lean::CREATE_RPC_SESSION, json!("s1"));
    shows(&app, &["F.lean:2:2"]);
    hides(&app, "[pinned]");
    assert_eq!(
        sent(app.on_message(&json!({"event":"changedCursorLocation","args":[loc(4, 0)]}))),
        goal_requests(4, 0)
    );
}

#[test]
fn messages_at_the_cursor_line_and_all_toggle() {
    let mut app = app();
    app.reply(
        lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
        json!([{"uri":U,"diagnostics":[
            publish(1, 2, 1, 2, "declaration uses 'sorry'"),
            publish(0, 0, 3, 3, "spans\nthe cursor"),
            publish(5, 0, 5, 4, "elsewhere"),
        ]},{"uri":"file:///p/G.lean","diagnostics":[publish(1, 0, 1, 1, "other file")]}]),
    );
    shows(
        &app,
        &[
            "▾ Messages (2)",
            "2:2 warning: declaration uses 'sorry'",
            "▾ 1:0 information: spans",
            "  the cursor",
        ],
    );
    hides(&app, "6:0 hint: elsewhere");
    app.on_key(key('a'));
    shows(&app, &["▾ All messages (3)"]);
    shows(&app, &["6:0 hint: elsewhere"]);
    hides(&app, "2:0 error: other file");
    app.on_key(key('a'));
    shows(&app, &["▾ Messages (2)"]);
}

#[test]
fn unpinning_to_a_newer_location_requests_goals() {
    let mut app = app();
    app.on_key(key('p'));
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(6, 3)]}));
    assert_eq!(sent(app.on_key(key('p'))), goal_requests(6, 3));
    shows(&app, &["F.lean:7:3"]);
    app.on_key(key('p'));
    assert!(app.on_key(key('p')).is_empty());
}

#[test]
fn copy_inserts_the_goals_above_the_shown_location() {
    let mut app = app();
    app.on_key(key('c'));
    assert_eq!(footer(&app), "No goals to copy");
    app.reply(lean::GET_INTERACTIVE_GOALS, goals(&["⊢ A", "h : B\n⊢ C"]));
    app.on_key(key('p'));
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(9, 0)]}));
    assert_eq!(
        sent(app.on_key(key('c'))),
        [page_call(
            lean::INSERT_TEXT,
            json!(["/-\n⊢ A\n\nh : B\n⊢ C\n-/","above",{"textDocument":{"uri":U},"position":{"line":1,"character":2}}])
        )]
    );
    app.reply(
        lean::INSERT_TEXT,
        json!({"applied":false,"failureReason":"x"}),
    );
    assert_eq!(footer(&app), "x");
}

#[test]
fn open_needs_a_connection() {
    let mut app = app();
    assert_eq!(app.on_key(key('o')), [Effect::OpenBrowser]);
    app.disconnected("Waiting for the Lean server...".into());
    assert_eq!(footer(&app), "Waiting for the Lean server...");
    shows(&app, &["F.lean:2:2"]);
    assert!(app.on_key(key('o')).is_empty());
    assert_eq!(footer(&app), "Not connected");
}

#[test]
fn disconnect_forgets_pending_requests() {
    let mut app = app();
    app.disconnected("gone".into());
    app.reply(lean::GET_INTERACTIVE_GOALS, goals(&["⊢ late"]));
    hides(&app, "⊢ late");
}

#[test]
fn keys_move_the_selection_and_the_view_follows() {
    let mut app = app();
    let diagnostics: Vec<Value> = (0..30)
        .map(|i| publish(1, 0, 1, 1, &format!("m{i}")))
        .collect();
    app.reply(
        lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
        json!([{"uri":U,"diagnostics":diagnostics}]),
    );
    assert_eq!(screen(&app)[0], "F.lean:2:2");
    app.on_key(key('j'));
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(screen(&app)[0], "F.lean:2:2");
    app.on_key(key('G'));
    assert_eq!(screen(&app)[18], "2:0 error: m29");
    app.on_key(key('k'));
    assert_eq!(screen(&app)[18], "2:0 error: m29");
    app.on_key(key('g'));
    assert_eq!(screen(&app)[0], "F.lean:2:2");
    for quit in [
        key('q'),
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        assert_eq!(app.on_key(quit), [Effect::Quit]);
    }
}

#[test]
fn the_selection_follows_rows_that_shrink() {
    let mut app = app();
    let diagnostics: Vec<Value> = (0..30)
        .map(|i| publish(1, 0, 1, 1, &format!("m{i}")))
        .collect();
    app.reply(
        lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
        json!([{"uri":U,"diagnostics":diagnostics}]),
    );
    app.on_key(key('G'));
    app.on_message(&json!({"event":"gotServerNotification","args":["textDocument/publishDiagnostics",{"uri":U,"diagnostics":[]}]}));
    // Rows: file, blank, Messages (0); the selection sits on the last one.
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    shows(&app, &["▸ Messages (0)"]);
    app.on_key(key('k'));
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    shows(&app, &["▸ Messages (0)"]);
}

#[test]
fn enter_folds_the_block_around_the_selection() {
    let mut app = app();
    let two = json!({"goals":[
        {"userName":"pos","hyps":[{"names":["h"],"type":{"text":"A"}}],"type":{"text":"B"}},
        {"userName":"neg","hyps":[],"type":{"text":"C"}},
    ]});
    app.reply(lean::GET_INTERACTIVE_GOALS, two.clone());
    // Rows: file, blank, Tactic state, 2 goals, case pos, h : A, ⊢ B, blank, case neg, ⊢ C.
    for _ in 0..5 {
        app.on_key(key('j'));
    }
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    shows(&app, &["2 goals", "▸ case pos", "", "▾ case neg", "⊢ C"]);
    hides(&app, "h : A");
    // The fold survives new goals for the same case.
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.reply(lean::GET_INTERACTIVE_GOALS, two);
    hides(&app, "h : A");
    app.on_key(key(' '));
    shows(&app, &["▾ case pos", "h : A"]);
    app.on_key(key('g'));
    app.on_key(key('j'));
    app.on_key(key('j'));
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    shows(
        &app,
        &["F.lean:3:0", "", "▸ Tactic state", "", "▾ Messages (0)"],
    );
}

#[test]
fn footer_shows_keys_until_a_status_and_any_key_clears_it() {
    let mut app = app();
    assert_eq!(footer(&app), KEYS);
    app.on_key(key('c'));
    assert_eq!(footer(&app), "No goals to copy");
    app.on_key(key('x'));
    assert_eq!(footer(&app), KEYS);
}

#[test]
fn one_session_per_file_and_reconnect_when_lean_drops_it() {
    let mut app = Tui::new();
    app.connected("http://h/".into());
    app.reply(lean::HELLO, hello(loc(1, 2)));
    // A second move while the session opens waits for it instead of opening another.
    assert!(
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}))
            .is_empty()
    );
    assert_eq!(
        sent(app.reply(lean::CREATE_RPC_SESSION, json!("s1"))),
        goal_requests(2, 0)
    );
    let lost = app.fail(
        lean::GET_INTERACTIVE_GOALS,
        lean::RPC_NEEDS_RECONNECT,
        "Outdated RPC session",
    );
    assert_eq!(
        sent(lost),
        [page_call(lean::CREATE_RPC_SESSION, json!([U]))]
    );
    assert_eq!(footer(&app), KEYS);
    app.disconnected("gone".into());
    app.connected("http://h/".into());
    // A late reply to the session asked for before the disconnect opens nothing.
    app.reply(lean::CREATE_RPC_SESSION, json!("s2"));
    let again = app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
    assert_eq!(
        sent(again),
        [page_call(lean::CREATE_RPC_SESSION, json!([U]))]
    );
}

#[test]
fn stale_replies_release_their_references_and_racing_sessions_are_kept() {
    let mut app = app();
    let superseded = app.id(lean::GET_INTERACTIVE_GOALS);
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    // The first goals request was superseded; its reply still holds a reference.
    let stale = json!({"id":superseded,"result":{"goals":[{"hyps":[],"type":{"tag":[{"info":{"__rpcref":"3"}},{"text":"A"}]}}]}});
    assert_eq!(
        sent(app.on_message(&stale)),
        [release(json!([{"__rpcref":"3"}]))]
    );
    // Moving to another file while its session opens, then back, keeps both sessions.
    let g = "file:///p/G.lean";
    let at = |uri: &str| json!({"event":"changedCursorLocation","args":[{"uri":uri,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}}]});
    assert_eq!(
        sent(app.on_message(&at(g))),
        [page_call(lean::CREATE_RPC_SESSION, json!([g]))]
    );
    app.on_message(&at(U));
    assert!(app.reply(lean::CREATE_RPC_SESSION, json!("sg")).is_empty());
    let back = app.on_message(&at(g));
    assert_eq!(sent(back)[0]["args"][2]["sessionId"], "sg");
}

#[test]
fn replaced_goals_release_their_references() {
    let mut app = app();
    let held = json!({"goals":[{"hyps":[],"type":{"tag":[{"info":{"p":"7"}},{"text":"A"}]}}]});
    assert!(app.reply(lean::GET_INTERACTIVE_GOALS, held).is_empty());
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    assert_eq!(
        sent(app.reply(lean::GET_INTERACTIVE_GOALS, goals(&["⊢ B"]))),
        [release(json!([{"p":"7"}]))]
    );
    shows(&app, &["⊢ B"]);
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn click(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

#[test]
fn clicking_a_subterm_shows_its_type_and_goes_to_its_definition() {
    let mut app = app();
    let target = json!({"append":[{"tag":[{"info":{"p":"9"}},{"text":"Nat.succ"}]},{"text":" n"}]});
    app.reply(
        lean::GET_INTERACTIVE_GOALS,
        json!({"goals":[{"hyps":[],"type":target}]}),
    );
    // Rows: F.lean:2:2, blank, ▾ Tactic state, 1 goal, ⊢ Nat.succ n.
    assert_eq!(screen(&app)[4], "⊢ Nat.succ n");
    assert!(app.on_mouse(click(11, 4)).is_empty(), "` n` is not tagged");
    let shown = json!({"line":1,"character":2});
    assert_eq!(
        sent(app.on_mouse(click(3, 4))),
        [rpc_call(
            &shown,
            lean::INFO_TO_INTERACTIVE,
            json!({"p":"9"})
        )]
    );
    shows(&app, &["Nat.succ"]);
    assert_eq!(footer(&app), PICKED_KEYS);
    app.reply(
        lean::INFO_TO_INTERACTIVE,
        json!({"type":{"text":"Nat → Nat"},"doc":"The successor."}),
    );
    shows(&app, &["Nat.succ : Nat → Nat", "The successor."]);
    assert_eq!(
        sent(app.on_key(key('d'))),
        [rpc_call(
            &shown,
            lean::GET_GO_TO_LOCATION,
            json!({"kind":"definition","info":{"p":"9"}})
        )]
    );
    let range = json!({"start":{"line":3,"character":0},"end":{"line":3,"character":4}});
    let found = json!([{"targetUri":"file:///lib/Nat.lean","targetRange":range,"targetSelectionRange":range}]);
    assert_eq!(
        sent(app.reply(lean::GET_GO_TO_LOCATION, found)),
        [page_call(
            lean::SHOW_DOCUMENT,
            json!([{"uri":"file:///lib/Nat.lean","selection":range,"takeFocus":true}])
        )]
    );
    app.reply(lean::SHOW_DOCUMENT, json!({"success":false}));
    assert_eq!(footer(&app), "Zed could not open the location");
    app.on_key(key('t'));
    app.reply(lean::GET_GO_TO_LOCATION, json!([]));
    assert_eq!(footer(&app), "No location found");
    // Esc closes the detail first, then quits.
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .is_empty()
    );
    hides(&app, "Nat.succ : Nat → Nat");
    assert_eq!(
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        [Effect::Quit]
    );
}

#[test]
fn new_goals_drop_the_picked_subterm() {
    let mut app = app();
    let target = json!({"tag":[{"info":{"p":"9"}},{"text":"A"}]});
    app.reply(
        lean::GET_INTERACTIVE_GOALS,
        json!({"goals":[{"hyps":[],"type":target}]}),
    );
    screen(&app);
    app.on_mouse(click(2, 4));
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.reply(lean::GET_INTERACTIVE_GOALS, goals(&["⊢ B"]));
    assert_eq!(footer(&app), KEYS);
    app.reply(lean::INFO_TO_INTERACTIVE, json!({"type":{"text":"late"}}));
    hides(&app, "A : late");
}

#[test]
fn closing_the_detail_releases_its_popup_and_wrapped_ends_are_not_terms() {
    let mut app = app();
    // 55 untagged characters, then a tagged term that wraps onto the next screen line.
    let target = json!({"append":[{"text":"x".repeat(55)},{"text":" "},{"tag":[{"info":{"__rpcref":"9"}},{"text":"Tagged"}]}]});
    app.reply(
        lean::GET_INTERACTIVE_GOALS,
        json!({"goals":[{"hyps":[],"type":target}]}),
    );
    assert_eq!(screen(&app)[5], "    Tagged");
    // Column 59 of the first segment is blank space after the break, not `Tagged`.
    assert!(app.on_mouse(click(59, 4)).is_empty());
    let picked = sent(app.on_mouse(click(7, 5)));
    assert_eq!(picked[0]["args"][2]["params"], json!({"__rpcref":"9"}));
    app.reply(
        lean::INFO_TO_INTERACTIVE,
        json!({"type":{"tag":[{"info":{"__rpcref":"10"}},{"text":"T"}]}}),
    );
    assert_eq!(
        sent(app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
        [release(json!([{"__rpcref":"10"}]))]
    );
}

#[test]
fn clicking_a_header_folds_it_and_the_wheel_scrolls() {
    let mut app = app();
    let diagnostics: Vec<Value> = (0..30)
        .map(|i| publish(1, 0, 1, 1, &format!("m{i}")))
        .collect();
    app.reply(
        lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
        json!([{"uri":U,"diagnostics":diagnostics}]),
    );
    assert_eq!(screen(&app)[2], "▾ Messages (30)");
    app.on_mouse(mouse(MouseEventKind::ScrollDown, 0, 0));
    assert_eq!(screen(&app)[0], "2:0 error: m0");
    app.on_mouse(mouse(MouseEventKind::ScrollUp, 0, 0));
    assert_eq!(screen(&app)[0], "F.lean:2:2");
    app.on_mouse(click(5, 2));
    assert_eq!(screen(&app)[2], "▸ Messages (30)");
    hides(&app, "2:0 error: m0");
    // Clicks outside the body do nothing.
    assert!(app.on_mouse(click(5, 19)).is_empty());
}

#[test]
fn interactive_messages_replace_plain_ones_and_load_traces() {
    let mut app = app();
    app.reply(
        lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
        json!([{"uri":U,"diagnostics":[publish(1, 2, 1, 2, "plain")]}]),
    );
    shows(&app, &["2:2 warning: plain"]);
    let trace = json!({"tag":[{"trace":{"indent":0,"cls":"Elab.step","msg":{"text":"node"},
        "collapsed":true,"children":{"lazy":{"p":"5"}}}},{"text":""}]});
    let diags = json!([
        {"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":3}},"severity":2,
         "message":{"append":[{"text":"uses "},{"tag":[{"expr":{"tag":[{"info":{"p":"4"}},{"text":"sorryAx"}]}},{"text":""}]}]}},
        {"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":1}},"severity":3,"message":trace},
    ]);
    app.reply(lean::GET_INTERACTIVE_DIAGNOSTICS, diags.clone());
    shows(
        &app,
        &[
            "▾ Messages (2)",
            "2:2 warning: uses sorryAx",
            "▾ 2:0 information:",
            "▸ [Elab.step] node",
        ],
    );
    hides(&app, "2:2 warning: plain");
    // Expressions in messages are clickable like goals.
    let row = screen(&app)
        .iter()
        .position(|l| l.contains("sorryAx"))
        .unwrap();
    let picked = sent(app.on_mouse(click(22, row.try_into().unwrap())));
    assert_eq!(picked[0]["args"][2]["params"], json!({"p":"4"}));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // Opening the lazy trace asks Lean for its children, once.
    let row = screen(&app)
        .iter()
        .position(|l| l.contains("[Elab.step]"))
        .unwrap();
    let fetch = sent(app.on_mouse(click(0, row.try_into().unwrap())));
    assert_eq!(
        fetch[0]["args"][2]["method"],
        lean::LAZY_TRACE_CHILDREN_TO_INTERACTIVE
    );
    assert_eq!(fetch[0]["args"][2]["params"], json!({"p":"5"}));
    shows(&app, &["▾ [Elab.step] node", "  ⋯"]);
    app.reply(
        lean::LAZY_TRACE_CHILDREN_TO_INTERACTIVE,
        json!([{"text":"child"}]),
    );
    shows(&app, &["▾ [Elab.step] node", "child"]);
    // A refresh replaces the references; the open trace is fetched again.
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(1, 2)]}));
    let refreshed = sent(app.reply(lean::GET_INTERACTIVE_DIAGNOSTICS, diags));
    assert_eq!(refreshed[0]["args"][1], lean::RPC_RELEASE);
    assert_eq!(refreshed[1]["args"][2]["params"], json!({"p":"5"}));
    app.on_key(key('a'));
    shows(&app, &["▾ All messages (1)", "2:2 warning: plain"]);
}

#[test]
fn lazy_children_from_an_older_session_are_released_not_shown() {
    let mut app = app();
    let lazy = |p: &str| {
        json!([{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":0}},"severity":3,
            "message":{"tag":[{"trace":{"indent":0,"cls":"c","msg":{"text":"long\nheader"},
            "collapsed":false,"children":{"lazy":{"__rpcref":p}}}},{"text":""}]}}])
    };
    // An open trace whose header wraps still asks for its children.
    let fetch = sent(app.reply(lean::GET_INTERACTIVE_DIAGNOSTICS, lazy("3")));
    assert_eq!(fetch[0]["args"][2]["params"], json!({"__rpcref":"3"}));
    let first_fetch = app.id(lean::LAZY_TRACE_CHILDREN_TO_INTERACTIVE);
    // Lean restarts the worker while the children are in flight.
    app.fail(
        lean::GET_INTERACTIVE_GOALS,
        lean::RPC_NEEDS_RECONNECT,
        "Outdated RPC session",
    );
    app.reply(lean::CREATE_RPC_SESSION, json!("s2"));
    // The new session's messages replace the old ones and fetch the open trace again.
    let refetch = sent(app.reply(lean::GET_INTERACTIVE_DIAGNOSTICS, lazy("3")));
    assert_eq!(refetch[0]["args"][2]["sessionId"], "s2");
    let late = json!({"id":first_fetch,"result":[{"tag":[{"expr":{"tag":[{"info":{"__rpcref":"5"}},{"text":"old"}]}},{"text":""}]}]});
    assert!(
        app.on_message(&late).is_empty(),
        "s1 is gone, nothing to release into"
    );
    hides(&app, "old");
    shows(&app, &["▾ [c] long", "header", "  ⋯"]);
}

#[test]
fn the_hover_shows_after_the_messages_and_folds() {
    let mut app = app();
    app.reply(lean::HOVER, hover());
    shows(
        &app,
        &[
            "▾ Messages (0)",
            "",
            "▾ At the cursor",
            "Nat.add_zero : ∀ (n : Nat), n + 0 = n",
            "",
            "Adding zero changes nothing.",
        ],
    );
    app.on_key(key('G'));
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    shows(&app, &["▾ Messages (0)", "", "▸ At the cursor"]);
    hides(&app, "Adding zero changes nothing.");
}

#[test]
fn a_null_hover_hides_the_section() {
    let mut app = app();
    app.reply(lean::HOVER, hover());
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.reply(lean::HOVER, Value::Null);
    hides(&app, "▾ At the cursor");
}

#[test]
fn file_label_is_the_last_uri_segment() {
    let mut app = Tui::new();
    app.connected("http://h/".into());
    let deep = json!({"uri":"file:///a/b/Deep.lean","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}});
    app.reply(lean::HELLO, hello(deep));
    assert_eq!(screen(&app)[0], "Deep.lean:1:0");
}
