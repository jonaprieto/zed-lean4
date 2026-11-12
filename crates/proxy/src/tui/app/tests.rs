use super::{
    draw::{KEYS, PICKED_KEYS},
    *,
};
use ratatui::{Terminal, backend::TestBackend};

const U: &str = "file:///p/F.lean";

fn loc(line: u64, character: u64) -> Value {
    json!({"uri":U,"range":{"start":{"line":line,"character":character},"end":{"line":line,"character":character}}})
}

fn sent(effects: Vec<Effect>) -> Vec<Value> {
    effects
        .into_iter()
        .map(|e| match e {
            Effect::Send(m) => m,
            other => panic!("expected a send, got {other:?}"),
        })
        .collect()
}

fn goal_requests(first: u64, line: u64, character: u64) -> Vec<Value> {
    let position = json!({"line":line,"character":character});
    let rpc = |id: u64, method: &str, params: Value| {
        json!({"id":format!("tui-1:{id}"),"call":"sendClientRequest","args":[U,"$/lean/rpc/call",
            {"textDocument":{"uri":U},"position":position,"sessionId":"s1","method":method,"params":params}]})
    };
    let at = json!({"textDocument":{"uri":U},"position":position});
    let line_range = json!({"lineRange":{"start":line,"end":line + 1}});
    vec![
        rpc(first, lean::GET_INTERACTIVE_GOALS, at.clone()),
        rpc(first + 1, lean::GET_INTERACTIVE_TERM_GOAL, at),
        rpc(first + 2, lean::GET_INTERACTIVE_DIAGNOSTICS, line_range),
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

/// Connected, hello answered with `loc(1, 2)`, session `s1` open; goal ids are 4 and 5.
fn app() -> App {
    let mut app = App::new("tui-1".into());
    app.connected("http://127.0.0.1:1/?token=t".into());
    app.on_message(&json!({"id":"tui-1:1","result":{"location":loc(1, 2),"initializeResult":{}}}));
    app.on_message(&json!({"id":"tui-1:3","result":"s1"}));
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
    let mut app = App::new("tui-1".into());
    assert_eq!(
        sent(app.connected("http://h/?token=t".into())),
        [
            json!({"id":"tui-1:1","call":"hello","args":[]}),
            json!({"id":"tui-1:2","call":"subscribeServerNotifications","args":["textDocument/publishDiagnostics"]}),
        ]
    );
    assert_eq!(app.page_url(), Some("http://h/?token=t"));
}

#[test]
fn hello_location_requests_both_goals() {
    let mut app = App::new("tui-1".into());
    app.connected("http://h/".into());
    let effects = app.on_message(
        &json!({"id":"tui-1:1","result":{"location":loc(1, 2),"initializeResult":{"capabilities":{}}}}),
    );
    assert_eq!(
        sent(effects),
        [json!({"id":"tui-1:3","call":"createRpcSession","args":[U]})]
    );
    let effects = app.on_message(&json!({"id":"tui-1:3","result":"s1"}));
    assert_eq!(sent(effects), goal_requests(4, 1, 2));
}

#[test]
fn null_hello_location_sends_nothing_and_says_so() {
    let mut app = App::new("tui-1".into());
    app.connected("http://h/".into());
    let effects =
        app.on_message(&json!({"id":"tui-1:1","result":{"location":null,"initializeResult":{}}}));
    assert!(effects.is_empty());
    shows(&app, &["No Lean file at the cursor"]);
}

#[test]
fn diagnostics_come_from_subscribe_and_notifications() {
    let mut app = app();
    app.on_message(
        &json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[publish(1, 2, 1, 2, "old")]}]}),
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
    assert_eq!(sent(app.on_message(&note(U))), goal_requests(10, 4, 0));
}

#[test]
fn cursor_moves_request_goals_unless_pinned() {
    let mut app = app();
    let moved = app.on_message(&json!({"event":"changedCursorLocation","args":[loc(4, 0)]}));
    assert_eq!(sent(moved), goal_requests(7, 4, 0));
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
    app.on_message(&json!({"id":"tui-1:4","result":goals(&["n : Nat\n⊢ n + 0 = n"])}));
    app.on_message(&json!({"id":"tui-1:5","result":goal("⊢ Nat")}));
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
    app.on_message(&json!({"id":"tui-1:4","result":goals(&[])}));
    shows(&app, &["▾ Tactic state", "No goals"]);
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.on_message(&json!({"id":"tui-1:7","result":null}));
    hides(&app, "▾ Tactic state");
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
    app.on_message(&json!({"id":"tui-1:10","result":goals(&["⊢ A", "⊢ B"])}));
    shows(&app, &["2 goals", "▾ goal 1", "⊢ A", "", "▾ goal 2", "⊢ B"]);
}

#[test]
fn superseded_replies_are_ignored_and_errors_keep_goals() {
    let mut app = app();
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.on_message(&json!({"id":"tui-1:4","result":goals(&["⊢ old"])}));
    app.on_message(&json!({"id":"other:6","result":goals(&["⊢ foreign"])}));
    hides(&app, "⊢ old");
    hides(&app, "⊢ foreign");
    app.on_message(&json!({"id":"tui-1:7","result":goals(&["⊢ new"])}));
    shows(&app, &["⊢ new"]);
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
    app.on_message(&json!({"id":"tui-1:10","error":{"code":-32602,"message":"bad params"}}));
    shows(&app, &["⊢ new"]);
    assert_eq!(footer(&app), "bad params");
}

#[test]
fn overtaken_and_cancelled_lookups_are_dropped_silently() {
    let mut app = app();
    for (id, code) in [(4, -32801), (5, -32800)] {
        app.on_message(
            &json!({"id":format!("tui-1:{id}"),"error":{"code":code,"message":"File changed."}}),
        );
        assert_eq!(footer(&app), KEYS);
    }
    // The slots were cleared: a late success is ignored too.
    app.on_message(&json!({"id":"tui-1:4","result":goals(&["⊢ late"])}));
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
    app.on_message(
        &json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[publish(1, 0, 1, 1, "old")]}]}),
    );
    shows(&app, &["2:0 error: old"]);
    app.connected("http://127.0.0.1:2/?token=t".into());
    app.on_message(&json!({"id":"tui-1:8","result":[]}));
    hides(&app, "2:0 error: old");
}

#[test]
fn pin_does_nothing_without_a_location() {
    let mut app = App::new("tui-1".into());
    app.connected("http://127.0.0.1:1/?token=t".into());
    app.on_key(key('p'));
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(1, 2)]}));
    app.on_message(&json!({"id":"tui-1:3","result":"s1"}));
    shows(&app, &["F.lean:2:2"]);
    hides(&app, "[pinned]");
    assert_eq!(
        sent(app.on_message(&json!({"event":"changedCursorLocation","args":[loc(4, 0)]}))),
        goal_requests(7, 4, 0)
    );
}

#[test]
fn messages_at_the_cursor_line_and_all_toggle() {
    let mut app = app();
    app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[
        publish(1, 2, 1, 2, "declaration uses 'sorry'"),
        publish(0, 0, 3, 3, "spans\nthe cursor"),
        publish(5, 0, 5, 4, "elsewhere"),
    ]},{"uri":"file:///p/G.lean","diagnostics":[publish(1, 0, 1, 1, "other file")]}]}));
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
    assert_eq!(sent(app.on_key(key('p'))), goal_requests(7, 6, 3));
    shows(&app, &["F.lean:7:3"]);
    app.on_key(key('p'));
    assert!(app.on_key(key('p')).is_empty());
}

#[test]
fn copy_inserts_the_goals_above_the_shown_location() {
    let mut app = app();
    app.on_key(key('c'));
    assert_eq!(footer(&app), "No goals to copy");
    app.on_message(&json!({"id":"tui-1:4","result":goals(&["⊢ A", "h : B\n⊢ C"])}));
    app.on_key(key('p'));
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(9, 0)]}));
    assert_eq!(
        sent(app.on_key(key('c'))),
        [
            json!({"id":"tui-1:7","call":"insertText","args":["/-\n⊢ A\n\nh : B\n⊢ C\n-/","above",{"textDocument":{"uri":U},"position":{"line":1,"character":2}}]})
        ]
    );
    app.on_message(&json!({"id":"tui-1:7","result":{"applied":false,"failureReason":"x"}}));
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
    app.on_message(&json!({"id":"tui-1:4","result":goals(&["⊢ late"])}));
    hides(&app, "⊢ late");
}

#[test]
fn keys_move_the_selection_and_the_view_follows() {
    let mut app = app();
    let diagnostics: Vec<Value> = (0..30)
        .map(|i| publish(1, 0, 1, 1, &format!("m{i}")))
        .collect();
    app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":diagnostics}]}));
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
    app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":diagnostics}]}));
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
    app.on_message(&json!({"id":"tui-1:4","result":two}));
    // Rows: file, blank, Tactic state, 2 goals, case pos, h : A, ⊢ B, blank, case neg, ⊢ C.
    for _ in 0..5 {
        app.on_key(key('j'));
    }
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    shows(&app, &["2 goals", "▸ case pos", "", "▾ case neg", "⊢ C"]);
    hides(&app, "h : A");
    // The fold survives new goals for the same case.
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.on_message(&json!({"id":"tui-1:7","result":two}));
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
    let mut app = App::new("tui-1".into());
    app.connected("http://h/".into());
    app.on_message(&json!({"id":"tui-1:1","result":{"location":loc(1, 2),"initializeResult":{}}}));
    // A second move while the session opens waits for it instead of opening another.
    assert!(
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}))
            .is_empty()
    );
    assert_eq!(
        sent(app.on_message(&json!({"id":"tui-1:3","result":"s1"}))),
        goal_requests(4, 2, 0)
    );
    let lost = json!({"id":"tui-1:4","error":{"code":-32900,"message":"Outdated RPC session"}});
    assert_eq!(
        sent(app.on_message(&lost)),
        [json!({"id":"tui-1:7","call":"createRpcSession","args":[U]})]
    );
    assert_eq!(footer(&app), KEYS);
    app.disconnected("gone".into());
    app.connected("http://h/".into());
    app.on_message(&json!({"id":"tui-1:99","result":"s2"}));
    let again = app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
    assert_eq!(
        sent(again),
        [json!({"id":"tui-1:10","call":"createRpcSession","args":[U]})]
    );
}

#[test]
fn stale_replies_release_their_references_and_racing_sessions_are_kept() {
    let mut app = app();
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    // The first request (id 4) was superseded; its reply still holds a reference.
    let stale = json!({"id":"tui-1:4","result":{"goals":[{"hyps":[],"type":{"tag":[{"info":{"__rpcref":"3"}},{"text":"A"}]}}]}});
    assert_eq!(
        sent(app.on_message(&stale)),
        [
            json!({"id":"tui-1:10","call":"sendClientNotification","args":[U,"$/lean/rpc/release",
            {"uri":U,"sessionId":"s1","refs":[{"__rpcref":"3"}]}]})
        ]
    );
    // Moving to another file while its session opens, then back, keeps both sessions.
    let g = "file:///p/G.lean";
    let at = |uri: &str| json!({"event":"changedCursorLocation","args":[{"uri":uri,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}}]});
    assert_eq!(
        sent(app.on_message(&at(g))),
        [json!({"id":"tui-1:11","call":"createRpcSession","args":[g]})]
    );
    app.on_message(&at(U));
    assert!(
        app.on_message(&json!({"id":"tui-1:11","result":"sg"}))
            .is_empty()
    );
    let back = app.on_message(&at(g));
    assert_eq!(sent(back)[0]["args"][2]["sessionId"], "sg");
}

#[test]
fn replaced_goals_release_their_references() {
    let mut app = app();
    let held = json!({"goals":[{"hyps":[],"type":{"tag":[{"info":{"p":"7"}},{"text":"A"}]}}]});
    assert!(
        app.on_message(&json!({"id":"tui-1:4","result":held}))
            .is_empty()
    );
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    assert_eq!(
        sent(app.on_message(&json!({"id":"tui-1:7","result":goals(&["⊢ B"])}))),
        [
            json!({"id":"tui-1:10","call":"sendClientNotification","args":[U,"$/lean/rpc/release",
            {"uri":U,"sessionId":"s1","refs":[{"p":"7"}]}]})
        ]
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
    app.on_message(&json!({"id":"tui-1:4","result":{"goals":[{"hyps":[],"type":target}]}}));
    // Rows: F.lean:2:2, blank, ▾ Tactic state, 1 goal, ⊢ Nat.succ n.
    assert_eq!(screen(&app)[4], "⊢ Nat.succ n");
    assert!(app.on_mouse(click(11, 4)).is_empty(), "` n` is not tagged");
    let rpc = |id: &str, method: &str, params: Value| {
        json!({"id":id,"call":"sendClientRequest","args":[U,"$/lean/rpc/call",
            {"textDocument":{"uri":U},"position":{"line":1,"character":2},"sessionId":"s1","method":method,"params":params}]})
    };
    assert_eq!(
        sent(app.on_mouse(click(3, 4))),
        [rpc("tui-1:7", lean::INFO_TO_INTERACTIVE, json!({"p":"9"}))]
    );
    shows(&app, &["Nat.succ"]);
    assert_eq!(footer(&app), PICKED_KEYS);
    app.on_message(
        &json!({"id":"tui-1:7","result":{"type":{"text":"Nat → Nat"},"doc":"The successor."}}),
    );
    shows(&app, &["Nat.succ : Nat → Nat", "The successor."]);
    assert_eq!(
        sent(app.on_key(key('d'))),
        [rpc(
            "tui-1:8",
            lean::GET_GO_TO_LOCATION,
            json!({"kind":"definition","info":{"p":"9"}})
        )]
    );
    let range = json!({"start":{"line":3,"character":0},"end":{"line":3,"character":4}});
    let found = json!({"id":"tui-1:8","result":[{"targetUri":"file:///lib/Nat.lean","targetRange":range,"targetSelectionRange":range}]});
    assert_eq!(
        sent(app.on_message(&found)),
        [
            json!({"id":"tui-1:9","call":"showDocument","args":[{"uri":"file:///lib/Nat.lean","selection":range,"takeFocus":true}]})
        ]
    );
    app.on_message(&json!({"id":"tui-1:9","result":{"success":false}}));
    assert_eq!(footer(&app), "Zed could not open the location");
    app.on_key(key('t'));
    app.on_message(&json!({"id":"tui-1:10","result":[]}));
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
    app.on_message(&json!({"id":"tui-1:4","result":{"goals":[{"hyps":[],"type":target}]}}));
    screen(&app);
    app.on_mouse(click(2, 4));
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
    app.on_message(&json!({"id":"tui-1:8","result":goals(&["⊢ B"])}));
    assert_eq!(footer(&app), KEYS);
    app.on_message(&json!({"id":"tui-1:7","result":{"type":{"text":"late"}}}));
    hides(&app, "A : late");
}

#[test]
fn closing_the_detail_releases_its_popup_and_wrapped_ends_are_not_terms() {
    let mut app = app();
    // 55 untagged characters, then a tagged term that wraps onto the next screen line.
    let target = json!({"append":[{"text":"x".repeat(55)},{"text":" "},{"tag":[{"info":{"__rpcref":"9"}},{"text":"Tagged"}]}]});
    app.on_message(&json!({"id":"tui-1:4","result":{"goals":[{"hyps":[],"type":target}]}}));
    assert_eq!(screen(&app)[5], "    Tagged");
    // Column 59 of the first segment is blank space after the break, not `Tagged`.
    assert!(app.on_mouse(click(59, 4)).is_empty());
    let picked = sent(app.on_mouse(click(7, 5)));
    assert_eq!(picked[0]["args"][2]["params"], json!({"__rpcref":"9"}));
    app.on_message(&json!({"id":"tui-1:7","result":{"type":{"tag":[{"info":{"__rpcref":"10"}},{"text":"T"}]}}}));
    assert_eq!(
        sent(app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
        [
            json!({"id":"tui-1:8","call":"sendClientNotification","args":[U,"$/lean/rpc/release",
            {"uri":U,"sessionId":"s1","refs":[{"__rpcref":"10"}]}]})
        ]
    );
}

#[test]
fn clicking_a_header_folds_it_and_the_wheel_scrolls() {
    let mut app = app();
    let diagnostics: Vec<Value> = (0..30)
        .map(|i| publish(1, 0, 1, 1, &format!("m{i}")))
        .collect();
    app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":diagnostics}]}));
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
    app.on_message(
        &json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[publish(1, 2, 1, 2, "plain")]}]}),
    );
    shows(&app, &["2:2 warning: plain"]);
    let trace = json!({"tag":[{"trace":{"indent":0,"cls":"Elab.step","msg":{"text":"node"},
        "collapsed":true,"children":{"lazy":{"p":"5"}}}},{"text":""}]});
    let diags = json!([
        {"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":3}},"severity":2,
         "message":{"append":[{"text":"uses "},{"tag":[{"expr":{"tag":[{"info":{"p":"4"}},{"text":"sorryAx"}]}},{"text":""}]}]}},
        {"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":1}},"severity":3,"message":trace},
    ]);
    app.on_message(&json!({"id":"tui-1:6","result":diags}));
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
    let id = fetch[0]["id"].clone();
    app.on_message(&json!({"id":id,"result":[{"text":"child"}]}));
    shows(&app, &["▾ [Elab.step] node", "child"]);
    // A refresh replaces the references; the open trace is fetched again.
    app.on_message(&json!({"event":"changedCursorLocation","args":[loc(1, 2)]}));
    let refreshed = sent(app.on_message(&json!({"id":"tui-1:11","result":diags})));
    assert_eq!(refreshed[0]["args"][1], "$/lean/rpc/release");
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
    let fetch = sent(app.on_message(&json!({"id":"tui-1:6","result":lazy("3")})));
    assert_eq!(fetch[0]["args"][2]["params"], json!({"__rpcref":"3"}));
    // Lean restarts the worker while the children are in flight.
    app.on_message(
        &json!({"id":"tui-1:4","error":{"code":-32900,"message":"Outdated RPC session"}}),
    );
    app.on_message(&json!({"id":"tui-1:8","result":"s2"}));
    // The new session's messages replace the old ones and fetch the open trace again.
    let refetch = sent(app.on_message(&json!({"id":"tui-1:11","result":lazy("3")})));
    assert_eq!(refetch[0]["args"][2]["sessionId"], "s2");
    let late = json!({"id":"tui-1:7","result":[{"tag":[{"expr":{"tag":[{"info":{"__rpcref":"5"}},{"text":"old"}]}},{"text":""}]}]});
    assert!(
        app.on_message(&late).is_empty(),
        "s1 is gone, nothing to release into"
    );
    hides(&app, "old");
    shows(&app, &["▾ [c] long", "header", "  ⋯"]);
}

#[test]
fn file_label_is_the_last_uri_segment() {
    let mut app = App::new("tui-1".into());
    app.connected("http://h/".into());
    app.on_message(&json!({"id":"tui-1:1","result":{"location":{"uri":"file:///a/b/Deep.lean","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}},"initializeResult":{}}}));
    assert_eq!(screen(&app)[0], "Deep.lean:1:0");
}
