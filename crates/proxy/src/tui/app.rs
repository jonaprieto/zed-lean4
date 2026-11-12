use std::{cell::Cell, collections::HashMap};

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Layout},
    style::Stylize,
    text::Line,
    widgets::{Paragraph, Wrap},
};
use serde_json::{Value, json};

const DIAGNOSTICS: &str = "textDocument/publishDiagnostics";
const KEYS: &str = "q quit  p pin  a all  c copy  o browser  j/k scroll";

#[derive(Debug, PartialEq)]
pub enum Effect {
    Send(Value),
    OpenBrowser,
    Quit,
}

pub struct App {
    prefix: String,
    next: u64,
    page_url: Option<String>,
    /// The location the view shows; it stays put while pinned.
    location: Option<Value>,
    /// The latest cursor location, pinned or not.
    latest: Option<Value>,
    pinned: bool,
    goals: Option<Vec<String>>,
    term_goal: Option<String>,
    diagnostics: HashMap<String, Vec<Value>>,
    all: bool,
    scroll: u16,
    /// Body height of the last render, for `G`.
    height: Cell<u16>,
    status: Option<String>,
    hello: Option<String>,
    subscribe: Option<String>,
    goal: Option<String>,
    term: Option<String>,
    insert: Option<String>,
}

/// Clears `slot` and returns true when it holds `id`.
fn take(slot: &mut Option<String>, id: &str) -> bool {
    let hit = slot.as_deref() == Some(id);
    if hit {
        *slot = None;
    }
    hit
}

fn severity(d: &Value) -> &'static str {
    match d["severity"].as_u64() {
        Some(2) => "warning",
        Some(3) => "information",
        Some(4) => "hint",
        _ => "error",
    }
}

impl App {
    pub fn new(id_prefix: String) -> Self {
        Self {
            prefix: id_prefix,
            next: 0,
            page_url: None,
            location: None,
            latest: None,
            pinned: false,
            goals: None,
            term_goal: None,
            diagnostics: HashMap::new(),
            all: false,
            scroll: 0,
            height: Cell::new(0),
            status: None,
            hello: None,
            subscribe: None,
            goal: None,
            term: None,
            insert: None,
        }
    }

    pub fn connected(&mut self, page_url: String) -> Vec<Effect> {
        self.page_url = Some(page_url);
        self.status = None;
        let (hello_id, hello) = self.call("hello", json!([]));
        let (subscribe_id, subscribe) =
            self.call("subscribeServerNotifications", json!([DIAGNOSTICS]));
        self.hello = Some(hello_id);
        self.subscribe = Some(subscribe_id);
        vec![hello, subscribe]
    }

    pub fn disconnected(&mut self, status: String) {
        self.page_url = None;
        self.status = Some(status);
        for slot in self.pending() {
            *slot = None;
        }
    }

    pub fn on_message(&mut self, m: &Value) -> Vec<Effect> {
        match m["event"].as_str() {
            Some("changedCursorLocation") => return self.cursor(&m["args"][0]),
            Some("gotServerNotification") if m["args"][0] == DIAGNOSTICS => {
                self.publish(&m["args"][1]);
                // Lean re-elaborated the shown file: its goals may have changed.
                let shown = self.location.as_ref().map(|l| &l["uri"]);
                if shown.is_some_and(|uri| *uri == m["args"][1]["uri"]) {
                    return self.show(self.location.clone());
                }
                return vec![];
            }
            Some(_) => return vec![],
            None => {}
        }
        let Some(id) = m["id"].as_str() else {
            return vec![];
        };
        if !self
            .pending()
            .iter()
            .any(|slot| slot.as_deref() == Some(id))
        {
            return vec![];
        }
        if let Some(error) = m.get("error") {
            let was_lookup = take(&mut self.goal, id) || take(&mut self.term, id);
            for slot in self.pending() {
                take(slot, id);
            }
            // An edit overtook the request or it was cancelled: not worth a status.
            let overtaken = matches!(error["code"].as_i64(), Some(-32801 | -32800));
            if !(was_lookup && overtaken) {
                self.status = Some(error["message"].as_str().unwrap_or("request failed").into());
            }
            return vec![];
        }
        let result = &m["result"];
        if take(&mut self.hello, id) {
            return self.cursor(&result["location"]);
        }
        if take(&mut self.subscribe, id) {
            self.diagnostics.clear();
            for params in result.as_array().into_iter().flatten() {
                self.publish(params);
            }
        } else if take(&mut self.goal, id) {
            self.goals = result["goals"].as_array().map(|goals| {
                goals
                    .iter()
                    .filter_map(|g| g.as_str().map(String::from))
                    .collect()
            });
        } else if take(&mut self.term, id) {
            self.term_goal = result["goal"].as_str().map(String::from);
        } else if take(&mut self.insert, id) && result["applied"] == false {
            let reason = result["failureReason"].as_str();
            self.status = Some(reason.unwrap_or("The edit was not applied").into());
        }
        vec![]
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        self.status = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('c') => vec![Effect::Quit],
                _ => vec![],
            };
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return vec![Effect::Quit],
            KeyCode::Char('p') if self.location.is_some() => {
                self.pinned = !self.pinned;
                if !self.pinned && self.latest != self.location {
                    return self.show(self.latest.clone());
                }
            }
            KeyCode::Char('a') => self.all = !self.all,
            KeyCode::Char('c') => return self.copy(),
            KeyCode::Char('o') if self.page_url.is_some() => return vec![Effect::OpenBrowser],
            KeyCode::Char('o') => self.status = Some("Not connected".into()),
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll = (self.scroll + 1).min(self.max_scroll())
            }
            KeyCode::Char('k') | KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Char('g') => self.scroll = 0,
            KeyCode::Char('G') => self.scroll = self.max_scroll(),
            _ => {}
        }
        vec![]
    }

    pub fn set_status(&mut self, status: String) {
        self.status = Some(status);
    }

    pub fn page_url(&self) -> Option<&str> {
        self.page_url.as_deref()
    }

    pub fn render(&self, frame: &mut Frame) {
        let [body, footer] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
        self.height.set(body.height);
        let scroll = self.scroll.min(self.max_scroll());
        let paragraph = Paragraph::new(self.lines())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(paragraph, body);
        frame.render_widget(Line::raw(self.status.as_deref().unwrap_or(KEYS)), footer);
    }

    /// The ids of requests still waiting for a reply.
    fn pending(&mut self) -> [&mut Option<String>; 5] {
        [
            &mut self.hello,
            &mut self.subscribe,
            &mut self.goal,
            &mut self.term,
            &mut self.insert,
        ]
    }

    fn call(&mut self, name: &str, args: Value) -> (String, Effect) {
        self.next += 1;
        let id = format!("{}:{}", self.prefix, self.next);
        let m = json!({"id":id,"call":name,"args":args});
        (id, Effect::Send(m))
    }

    fn cursor(&mut self, location: &Value) -> Vec<Effect> {
        self.latest = (!location.is_null()).then(|| location.clone());
        if self.pinned {
            return vec![];
        }
        self.show(self.latest.clone())
    }

    /// Shows `location` and asks for its goals.
    fn show(&mut self, location: Option<Value>) -> Vec<Effect> {
        self.location = location;
        let Some(location) = &self.location else {
            (self.goals, self.term_goal, self.goal, self.term) = (None, None, None, None);
            return vec![];
        };
        let uri = location["uri"].clone();
        let params = json!({"textDocument":{"uri":uri},"position":location["range"]["start"]});
        let (goal_id, goal) = self.call(
            "sendClientRequest",
            json!([uri, "$/lean/plainGoal", params]),
        );
        let (term_id, term) = self.call(
            "sendClientRequest",
            json!([uri, "$/lean/plainTermGoal", params]),
        );
        self.goal = Some(goal_id);
        self.term = Some(term_id);
        vec![goal, term]
    }

    fn publish(&mut self, params: &Value) {
        if let (Some(uri), Some(diagnostics)) =
            (params["uri"].as_str(), params["diagnostics"].as_array())
        {
            self.diagnostics.insert(uri.into(), diagnostics.clone());
        }
    }

    fn copy(&mut self) -> Vec<Effect> {
        let goals = self.goals.as_ref().filter(|goals| !goals.is_empty());
        let (Some(goals), Some(location)) = (goals, &self.location) else {
            self.status = Some("No goals to copy".into());
            return vec![];
        };
        let text = format!("/-\n{}\n-/", goals.join("\n\n"));
        let pos =
            json!({"textDocument":{"uri":location["uri"]},"position":location["range"]["start"]});
        let (id, insert) = self.call("insertText", json!([text, "above", pos]));
        self.insert = Some(id);
        vec![insert]
    }

    fn max_scroll(&self) -> u16 {
        let lines = u16::try_from(self.lines().len()).unwrap_or(u16::MAX);
        lines.saturating_sub(self.height.get())
    }

    fn lines(&self) -> Vec<Line<'_>> {
        let Some(location) = &self.location else {
            return vec![Line::raw("No Lean file at the cursor")];
        };
        let uri = location["uri"].as_str().unwrap_or_default();
        let start = &location["range"]["start"];
        let cursor = start["line"].as_u64().unwrap_or_default();
        let pinned = if self.pinned { " [pinned]" } else { "" };
        let mut lines = vec![Line::raw(format!(
            "{}:{}:{}{pinned}",
            uri.rsplit('/').next().unwrap_or(uri),
            cursor + 1,
            start["character"]
        ))];
        if let Some(goals) = &self.goals {
            lines.extend([Line::raw(""), Line::from("Tactic state".bold())]);
            match goals.len() {
                0 => lines.push(Line::raw("No goals")),
                1 => lines.push(Line::raw("1 goal")),
                n => lines.push(Line::raw(format!("{n} goals"))),
            }
            for (i, goal) in goals.iter().enumerate() {
                if i > 0 {
                    lines.push(Line::raw(""));
                }
                lines.extend(goal.lines().map(Line::raw));
            }
        }
        if let Some(term) = &self.term_goal {
            lines.extend([Line::raw(""), Line::from("Expected type".bold())]);
            lines.extend(term.lines().map(Line::raw));
        }
        let messages: Vec<&Value> = self
            .diagnostics
            .get(uri)
            .into_iter()
            .flatten()
            .filter(|d| {
                let range = &d["range"];
                self.all
                    || (range["start"]["line"].as_u64() <= Some(cursor)
                        && Some(cursor) <= range["end"]["line"].as_u64())
            })
            .collect();
        let title = if self.all { "All messages" } else { "Messages" };
        lines.extend([
            Line::raw(""),
            Line::from(format!("{title} ({})", messages.len()).bold()),
        ]);
        for d in messages {
            let at = &d["range"]["start"];
            let mut text = d["message"].as_str().unwrap_or_default().lines();
            lines.push(Line::raw(format!(
                "{}:{} {}: {}",
                at["line"].as_u64().unwrap_or_default() + 1,
                at["character"],
                severity(d),
                text.next().unwrap_or_default()
            )));
            lines.extend(text.map(|l| Line::raw(format!("  {l}"))));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

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
        let params =
            json!({"textDocument":{"uri":U},"position":{"line":line,"character":character}});
        vec![
            json!({"id":format!("tui-1:{first}"),"call":"sendClientRequest","args":[U,"$/lean/plainGoal",params]}),
            json!({"id":format!("tui-1:{}", first + 1),"call":"sendClientRequest","args":[U,"$/lean/plainTermGoal",params]}),
        ]
    }

    /// Connected, hello answered with `loc(1, 2)`; goal ids are 3 and 4.
    fn app() -> App {
        let mut app = App::new("tui-1".into());
        app.connected("http://127.0.0.1:1/?token=t".into());
        app.on_message(
            &json!({"id":"tui-1:1","result":{"location":loc(1, 2),"initializeResult":{}}}),
        );
        app
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn screen(app: &App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer();
        (0..20)
            .map(|y| {
                let row: String = (0..60).map(|x| buf[(x, y)].symbol()).collect();
                row.trim_end().to_owned()
            })
            .collect()
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
        assert_eq!(sent(effects), goal_requests(3, 1, 2));
    }

    #[test]
    fn null_hello_location_sends_nothing_and_says_so() {
        let mut app = App::new("tui-1".into());
        app.connected("http://h/".into());
        let effects = app
            .on_message(&json!({"id":"tui-1:1","result":{"location":null,"initializeResult":{}}}));
        assert!(effects.is_empty());
        shows(&app, &["No Lean file at the cursor"]);
    }

    #[test]
    fn diagnostics_come_from_subscribe_and_notifications() {
        let mut app = app();
        app.on_message(
            &json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[publish(1, 2, 1, 2, "old")]}]}),
        );
        shows(&app, &["Messages (1)", "2:2 warning: old"]);
        app.on_message(&json!({"event":"gotServerNotification","args":["textDocument/publishDiagnostics",{"uri":U,"diagnostics":[publish(1, 0, 1, 1, "new")]}]}));
        shows(&app, &["Messages (1)", "2:0 error: new"]);
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
        assert_eq!(sent(app.on_message(&note(U))), goal_requests(7, 4, 0));
    }

    #[test]
    fn cursor_moves_request_goals_unless_pinned() {
        let mut app = app();
        let moved = app.on_message(&json!({"event":"changedCursorLocation","args":[loc(4, 0)]}));
        assert_eq!(sent(moved), goal_requests(5, 4, 0));
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
        app.on_message(
            &json!({"id":"tui-1:3","result":{"rendered":"x","goals":["n : Nat\n⊢ n + 0 = n"]}}),
        );
        app.on_message(&json!({"id":"tui-1:4","result":{"goal":"Nat","range":{}}}));
        shows(
            &app,
            &[
                "F.lean:2:2",
                "",
                "Tactic state",
                "1 goal",
                "n : Nat",
                "⊢ n + 0 = n",
            ],
        );
        shows(&app, &["Expected type", "Nat"]);
    }

    #[test]
    fn empty_null_and_several_goals() {
        let mut app = app();
        app.on_message(&json!({"id":"tui-1:3","result":{"rendered":"","goals":[]}}));
        shows(&app, &["Tactic state", "No goals"]);
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
        app.on_message(&json!({"id":"tui-1:5","result":null}));
        hides(&app, "Tactic state");
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
        app.on_message(&json!({"id":"tui-1:7","result":{"goals":["⊢ A","⊢ B"]}}));
        shows(&app, &["2 goals", "⊢ A", "", "⊢ B"]);
    }

    #[test]
    fn superseded_replies_are_ignored_and_errors_keep_goals() {
        let mut app = app();
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(2, 0)]}));
        app.on_message(&json!({"id":"tui-1:3","result":{"goals":["⊢ old"]}}));
        app.on_message(&json!({"id":"other:5","result":{"goals":["⊢ foreign"]}}));
        hides(&app, "⊢ old");
        hides(&app, "⊢ foreign");
        app.on_message(&json!({"id":"tui-1:5","result":{"goals":["⊢ new"]}}));
        shows(&app, &["⊢ new"]);
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(3, 0)]}));
        app.on_message(&json!({"id":"tui-1:7","error":{"code":-32602,"message":"bad params"}}));
        shows(&app, &["⊢ new"]);
        assert_eq!(screen(&app)[19], "bad params");
    }

    #[test]
    fn overtaken_and_cancelled_lookups_are_dropped_silently() {
        let mut app = app();
        let keys = "q quit  p pin  a all  c copy  o browser  j/k scroll";
        for (id, code) in [(3, -32801), (4, -32800)] {
            app.on_message(
                &json!({"id":format!("tui-1:{id}"),"error":{"code":code,"message":"File changed."}}),
            );
            assert_eq!(screen(&app)[19], keys);
        }
        // The slots were cleared: a late success is ignored too.
        app.on_message(&json!({"id":"tui-1:3","result":{"goals":["⊢ late"]}}));
        hides(&app, "⊢ late");
    }

    #[test]
    fn reconnect_clears_the_no_server_status() {
        let mut app = app();
        app.disconnected("no Lean server found; open a Lean file in Zed".into());
        assert_eq!(
            screen(&app)[19],
            "no Lean server found; open a Lean file in Zed"
        );
        app.connected("http://127.0.0.1:1/?token=t".into());
        assert_eq!(
            screen(&app)[19],
            "q quit  p pin  a all  c copy  o browser  j/k scroll"
        );
    }

    #[test]
    fn subscribe_reply_replaces_all_diagnostics() {
        let mut app = app();
        app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[publish(1, 0, 1, 1, "old")]}]}));
        shows(&app, &["2:0 error: old"]);
        app.connected("http://127.0.0.1:2/?token=t".into());
        app.on_message(&json!({"id":"tui-1:6","result":[]}));
        hides(&app, "2:0 error: old");
    }

    #[test]
    fn pin_does_nothing_without_a_location() {
        let mut app = App::new("tui-1".into());
        app.connected("http://127.0.0.1:1/?token=t".into());
        app.on_key(key('p'));
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(1, 2)]}));
        shows(&app, &["F.lean:2:2"]);
        hides(&app, "[pinned]");
        assert_eq!(
            sent(app.on_message(&json!({"event":"changedCursorLocation","args":[loc(4, 0)]}))),
            goal_requests(5, 4, 0)
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
                "Messages (2)",
                "2:2 warning: declaration uses 'sorry'",
                "1:0 information: spans",
                "  the cursor",
            ],
        );
        hides(&app, "6:0 hint: elsewhere");
        app.on_key(key('a'));
        shows(&app, &["All messages (3)"]);
        shows(&app, &["6:0 hint: elsewhere"]);
        hides(&app, "2:0 error: other file");
        app.on_key(key('a'));
        shows(&app, &["Messages (2)"]);
    }

    #[test]
    fn unpinning_to_a_newer_location_requests_goals() {
        let mut app = app();
        app.on_key(key('p'));
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(6, 3)]}));
        assert_eq!(sent(app.on_key(key('p'))), goal_requests(5, 6, 3));
        shows(&app, &["F.lean:7:3"]);
        app.on_key(key('p'));
        assert!(app.on_key(key('p')).is_empty());
    }

    #[test]
    fn copy_inserts_the_goals_above_the_shown_location() {
        let mut app = app();
        app.on_key(key('c'));
        assert_eq!(screen(&app)[19], "No goals to copy");
        app.on_message(&json!({"id":"tui-1:3","result":{"goals":["⊢ A","h : B\n⊢ C"]}}));
        app.on_key(key('p'));
        app.on_message(&json!({"event":"changedCursorLocation","args":[loc(9, 0)]}));
        assert_eq!(
            sent(app.on_key(key('c'))),
            [
                json!({"id":"tui-1:5","call":"insertText","args":["/-\n⊢ A\n\nh : B\n⊢ C\n-/","above",{"textDocument":{"uri":U},"position":{"line":1,"character":2}}]})
            ]
        );
        app.on_message(&json!({"id":"tui-1:5","result":{"applied":false,"failureReason":"x"}}));
        assert_eq!(screen(&app)[19], "x");
    }

    #[test]
    fn open_needs_a_connection() {
        let mut app = app();
        assert_eq!(app.on_key(key('o')), [Effect::OpenBrowser]);
        app.disconnected("Waiting for the Lean server...".into());
        assert_eq!(screen(&app)[19], "Waiting for the Lean server...");
        shows(&app, &["F.lean:2:2"]);
        assert!(app.on_key(key('o')).is_empty());
        assert_eq!(screen(&app)[19], "Not connected");
    }

    #[test]
    fn disconnect_forgets_pending_requests() {
        let mut app = app();
        app.disconnected("gone".into());
        app.on_message(&json!({"id":"tui-1:3","result":{"goals":["⊢ late"]}}));
        hides(&app, "⊢ late");
    }

    #[test]
    fn keys_scroll_and_quit() {
        let mut app = app();
        let diagnostics: Vec<Value> = (0..30)
            .map(|i| publish(1, 0, 1, 1, &format!("m{i}")))
            .collect();
        app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":diagnostics}]}));
        assert_eq!(screen(&app)[0], "F.lean:2:2");
        app.on_key(key('j'));
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(screen(&app)[0], "Messages (30)");
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        app.on_key(key('k'));
        assert_eq!(screen(&app)[0], "F.lean:2:2");
        app.on_key(key('G'));
        assert_eq!(screen(&app)[18], "2:0 error: m29");
        app.on_key(key('k'));
        assert_eq!(screen(&app)[18], "2:0 error: m28");
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
    fn footer_shows_keys_until_a_status_and_any_key_clears_it() {
        let mut app = app();
        let keys = "q quit  p pin  a all  c copy  o browser  j/k scroll";
        assert_eq!(screen(&app)[19], keys);
        app.on_key(key('c'));
        assert_eq!(screen(&app)[19], "No goals to copy");
        app.on_key(key('x'));
        assert_eq!(screen(&app)[19], keys);
    }

    #[test]
    fn file_label_is_the_last_uri_segment() {
        let mut app = App::new("tui-1".into());
        app.connected("http://h/".into());
        app.on_message(&json!({"id":"tui-1:1","result":{"location":{"uri":"file:///a/b/Deep.lean","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}},"initializeResult":{}}}));
        assert_eq!(screen(&app)[0], "Deep.lean:1:0");
    }
}
