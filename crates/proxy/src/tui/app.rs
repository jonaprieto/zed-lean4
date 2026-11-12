use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::{Constraint, Layout, Position, Rect},
    style::Stylize,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde_json::{Value, json};

use super::{
    goals, messages,
    view::{self, Rows, Text},
};

const DIAGNOSTICS: &str = "textDocument/publishDiagnostics";
/// Lean's `RpcNeedsReconnect`: the file's worker restarted and dropped the session.
const NEEDS_RECONNECT: i64 = -32900;
const KEYS: &str = "q quit  p pin  a all  c copy  o browser  j/k move  ⏎ fold";
const PICKED_KEYS: &str = "d definition  t type  esc close";
/// Lines the wheel scrolls per notch.
const WHEEL: usize = 3;

#[derive(Debug, PartialEq)]
pub enum Effect {
    Send(Value),
    OpenBrowser,
    Quit,
}

/// A clicked subterm and what Lean says about it.
struct Picked {
    label: String,
    info: Value,
    uri: String,
    session: Value,
    popup: Option<Value>,
}

/// Where the last render put each visible screen line: the body area and, per
/// line, its row, the row characters it shows, and its wrap indentation.
#[derive(Default)]
struct Screen {
    area: Rect,
    lines: Vec<Segment>,
}

#[derive(Clone, Copy)]
struct Segment {
    row: usize,
    start: usize,
    end: usize,
    indent: usize,
}

/// An RPC result and the session that holds its references.
struct Held {
    uri: String,
    session: Value,
    value: Value,
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
    goals: Option<Held>,
    term_goal: Option<Held>,
    /// Interactive diagnostics on the shown line.
    messages: Option<Held>,
    /// Trace children fetched so far, by reference, and the requests fetching them.
    lazy: HashMap<String, Value>,
    loading: HashMap<String, String>,
    /// RPC session per file URI.
    sessions: HashMap<String, Value>,
    /// Pending `createRpcSession` calls and the file each is for.
    connects: HashMap<String, String>,
    /// The file and session of every RPC call still in flight, by id.
    inflight: HashMap<String, (String, Value)>,
    /// The file and session of the RPC reply being handled.
    asked: Option<(String, Value)>,
    diagnostics: HashMap<String, Vec<Value>>,
    all: bool,
    /// The selected row, which `enter` folds or unfolds.
    selected: usize,
    folded: HashSet<String>,
    /// First visible screen line; render keeps the selection in view while `follow`.
    scroll: Cell<usize>,
    follow: bool,
    screen: RefCell<Screen>,
    picked: Option<Picked>,
    status: Option<String>,
    hello: Option<String>,
    subscribe: Option<String>,
    goal: Option<String>,
    term: Option<String>,
    diags: Option<String>,
    insert: Option<String>,
    popup: Option<String>,
    goto: Option<String>,
    open: Option<String>,
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
            messages: None,
            lazy: HashMap::new(),
            loading: HashMap::new(),
            sessions: HashMap::new(),
            connects: HashMap::new(),
            inflight: HashMap::new(),
            asked: None,
            diagnostics: HashMap::new(),
            all: false,
            selected: 0,
            folded: HashSet::new(),
            scroll: Cell::new(0),
            follow: true,
            screen: RefCell::default(),
            picked: None,
            status: None,
            hello: None,
            subscribe: None,
            goal: None,
            term: None,
            diags: None,
            insert: None,
            popup: None,
            goto: None,
            open: None,
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
        self.sessions.clear();
        self.connects.clear();
        self.inflight.clear();
        self.loading.clear();
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
        if let Some(uri) = self.connects.remove(id) {
            match m.get("error") {
                Some(error) => self.status = error["message"].as_str().map(String::from),
                None => _ = self.sessions.insert(uri.clone(), m["result"].clone()),
            }
            let shown = self
                .location
                .as_ref()
                .is_some_and(|l| l["uri"] == uri.as_str());
            return if shown && m.get("error").is_none() {
                self.show(self.location.clone())
            } else {
                vec![]
            };
        }
        // The session an RPC reply came from: its references live there.
        self.asked = self.inflight.remove(id);
        if m["error"]["code"].as_i64() == Some(NEEDS_RECONNECT)
            && let Some((uri, session)) = &self.asked
            && self.sessions.get(uri) == Some(session)
        {
            self.sessions.remove(uri);
        }
        if let Some(p) = self.loading.remove(id) {
            if let Some(error) = m.get("error") {
                self.status = error["message"].as_str().map(String::from);
                return vec![];
            }
            // Children for the messages on screen; anything older only holds references.
            let current = self
                .messages
                .as_ref()
                .map(|h| (h.uri.clone(), h.session.clone()));
            if self.asked.is_some() && self.asked == current {
                self.lazy.insert(p, m["result"].clone());
                return vec![];
            }
            let stale = self.held(m.get("result").cloned());
            return self.release(stale);
        }
        if !self
            .pending()
            .iter()
            .any(|slot| slot.as_deref() == Some(id))
        {
            // A reply nobody waits for any more still holds references.
            let stale = self.held(m.get("result").cloned());
            return self.release(stale);
        }
        if let Some(error) = m.get("error") {
            let was_lookup =
                take(&mut self.goal, id) || take(&mut self.term, id) || take(&mut self.diags, id);
            for slot in self.pending() {
                take(slot, id);
            }
            if was_lookup && error["code"].as_i64() == Some(NEEDS_RECONNECT) {
                (self.goal, self.term, self.diags) = (None, None, None);
                return self.show(self.location.clone());
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
        } else if take(&mut self.popup, id) {
            if let Some(picked) = &mut self.picked {
                picked.popup = Some(result.clone());
            }
        } else if take(&mut self.goto, id) {
            return self.open_location(result);
        } else if take(&mut self.open, id) {
            if result["success"] == false {
                self.status = Some("Zed could not open the location".into());
            }
        } else if take(&mut self.goal, id) {
            let held = self.held(result.get("goals").is_some().then(|| result.clone()));
            let old = std::mem::replace(&mut self.goals, held);
            let mut effects = self.unpick_if_gone();
            effects.extend(self.release(old));
            return effects;
        } else if take(&mut self.diags, id) {
            let held = self.held(result.is_array().then(|| result.clone()));
            let old = self.replace_messages(held);
            let mut effects = self.unpick_if_gone();
            effects.extend(self.release(old));
            let open: Vec<Value> = self
                .rows()
                .into_iter()
                .filter(|r| r.open)
                .filter_map(|r| r.load)
                .collect();
            for children in open {
                effects.extend(self.fetch(children));
            }
            return effects;
        } else if take(&mut self.term, id) {
            let held = self.held((!result.is_null()).then(|| result.clone()));
            let old = std::mem::replace(&mut self.term_goal, held);
            let mut effects = self.unpick_if_gone();
            effects.extend(self.release(old));
            return effects;
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
        if self.picked.is_some() {
            match key.code {
                KeyCode::Esc => return self.unpick(),
                KeyCode::Char('d') => return self.go_to("definition"),
                KeyCode::Char('t') => return self.go_to("type"),
                _ => {}
            }
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
            KeyCode::Char('j') | KeyCode::Down => self.select(self.selected.saturating_add(1)),
            KeyCode::Char('k') | KeyCode::Up => {
                let last = self.rows().len().saturating_sub(1);
                self.select(self.selected.min(last).saturating_sub(1))
            }
            KeyCode::Char('g') => self.select(0),
            KeyCode::Char('G') => self.select(usize::MAX),
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Tab => return self.toggle_fold(),
            _ => {}
        }
        vec![]
    }

    /// The wheel scrolls; a left click selects a row, folds a header, or picks a subterm.
    pub fn on_mouse(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.follow = false;
                self.scroll.set(self.scroll.get() + WHEEL);
            }
            MouseEventKind::ScrollUp => {
                self.follow = false;
                self.scroll.set(self.scroll.get().saturating_sub(WHEEL));
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.status = None;
                return self.click(mouse.column, mouse.row);
            }
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
        let detail = self.detail();
        let panel = if detail.is_empty() {
            0
        } else {
            u16::try_from(detail.len() + 1).unwrap_or(u16::MAX).min(8)
        };
        let [body, panel, footer] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(panel),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        let rows = self.rows();
        let selected = self.selected.min(rows.len().saturating_sub(1));
        let mut screen = Screen {
            area: body,
            lines: Vec::new(),
        };
        let (mut lines, mut top, mut bottom) = (Vec::new(), 0, 0);
        for (i, row) in rows.into_iter().enumerate() {
            if i == selected {
                top = lines.len();
            }
            let chars = row
                .line
                .spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum();
            let wrapped = view::wrap(row.line, body.width.into());
            let ends: Vec<usize> = wrapped.iter().skip(1).map(|w| w.1).chain([chars]).collect();
            for ((mut line, start, indent), end) in wrapped.into_iter().zip(ends) {
                if i == selected {
                    line.style = line.style.patch(view::SELECTED);
                }
                screen.lines.push(Segment {
                    row: i,
                    start,
                    end,
                    indent,
                });
                lines.push(line);
            }
            if i == selected {
                bottom = lines.len();
            }
        }
        let height = usize::from(body.height);
        let mut scroll = self.scroll.get().min(lines.len().saturating_sub(height));
        if self.follow {
            scroll = scroll.max(bottom.saturating_sub(height)).min(top);
        }
        self.scroll.set(scroll);
        screen.lines.drain(..scroll.min(screen.lines.len()));
        *self.screen.borrow_mut() = screen;
        let paragraph =
            Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0));
        frame.render_widget(paragraph, body);
        if !detail.is_empty() {
            let detail = Paragraph::new(detail)
                .block(Block::new().borders(Borders::TOP))
                .wrap(Wrap { trim: false });
            frame.render_widget(detail, panel);
        }
        let keys = if self.picked.is_some() {
            PICKED_KEYS
        } else {
            KEYS
        };
        frame.render_widget(Line::raw(self.status.as_deref().unwrap_or(keys)), footer);
    }

    /// The picked subterm, its type and its docstring.
    fn detail(&self) -> Vec<Line<'static>> {
        let Some(picked) = &self.picked else {
            return vec![];
        };
        let popup = picked.popup.as_ref();
        let expr = popup.map(|p| &p["exprExplicit"]).filter(|e| !e.is_null());
        let mut head = match expr {
            Some(expr) => {
                goals::typeset(Text::default(), expr, &[])
                    .into_iter()
                    .next()
                    .unwrap_or_default()
                    .line
            }
            None => Line::raw(picked.label.clone()),
        };
        let mut lines: Vec<Line> = match popup.map(|p| &p["type"]).filter(|t| !t.is_null()) {
            Some(ty) => {
                head.spans.push(Span::raw(" : "));
                goals::typeset(head.into(), ty, &[])
                    .into_iter()
                    .map(|t| t.line)
                    .collect()
            }
            None => vec![head],
        };
        if let Some(doc) = popup.and_then(|p| p["doc"].as_str()) {
            lines.extend(doc.lines().map(|l| Line::raw(l.to_owned()).dim()));
        }
        lines
    }

    fn click(&mut self, x: u16, y: u16) -> Vec<Effect> {
        let hit = {
            let screen = self.screen.borrow();
            if !screen.area.contains(Position { x, y }) {
                return vec![];
            }
            let line = usize::from(y - screen.area.y);
            let col = usize::from(x - screen.area.x);
            screen.lines.get(line).map(|&segment| (segment, col))
        };
        let Some((segment, col)) = hit else {
            return vec![];
        };
        self.selected = segment.row;
        self.follow = true;
        let rows = self.rows();
        let Some(row) = rows.get(segment.row) else {
            return vec![];
        };
        let char = col
            .checked_sub(segment.indent)
            .and_then(|col| view::char_at(&row.line, segment.start, col))
            .filter(|&c| c < segment.end);
        let link = char.and_then(|c| row.links.iter().find(|l| l.start <= c && c < l.end));
        // A header folds unless the click lands on a term in it.
        let on_marker = char.is_some_and(|c| c < view::MARKER_WIDTH);
        if row.header && (link.is_none() || on_marker) {
            return self.toggle_fold();
        }
        let Some(link) = link.cloned() else {
            return self.unpick();
        };
        let label: String = row
            .line
            .spans
            .iter()
            .flat_map(|s| s.content.chars())
            .skip(link.start)
            .take(link.end - link.start)
            .collect();
        self.pick(label, link.info)
    }

    /// Asks Lean for the type and docstring of the subterm `info`, in the
    /// session that issued the reference.
    fn pick(&mut self, label: String, info: Value) -> Vec<Effect> {
        let mut effects = self.unpick();
        let Some((uri, session)) = self.owner(&info) else {
            return effects;
        };
        let (id, call) = self.rpc(
            &uri,
            &session,
            "Lean.Widget.InteractiveDiagnostics.infoToInteractive",
            info.clone(),
        );
        self.popup = Some(id);
        self.picked = Some(Picked {
            label,
            info,
            uri,
            session,
            popup: None,
        });
        effects.push(call);
        effects
    }

    /// Closes the detail panel and releases the references its popup held.
    fn unpick(&mut self) -> Vec<Effect> {
        self.popup = None;
        self.goto = None;
        let Some(picked) = self.picked.take() else {
            return vec![];
        };
        let popup = picked.popup.map(|value| Held {
            uri: picked.uri,
            session: picked.session,
            value,
        });
        self.release(popup)
    }

    /// The file and session of the shown result that holds `info`; fetched
    /// trace children belong to the messages' session.
    fn owner(&self, info: &Value) -> Option<(String, Value)> {
        let origin = |h: &Held| (h.uri.clone(), h.session.clone());
        let shown = [&self.goals, &self.term_goal, &self.messages]
            .into_iter()
            .flatten();
        if let Some(held) = shown.into_iter().find(|h| goals::holds(&h.value, info)) {
            return Some(origin(held));
        }
        let messages = self.messages.as_ref()?;
        let fetched = self.lazy.values().any(|v| goals::holds(v, info));
        fetched.then(|| origin(messages))
    }

    /// Shows `new` messages and hands back the old ones together with the
    /// trace children fetched for them, which hold references of the same session.
    fn replace_messages(&mut self, new: Option<Held>) -> Option<Held> {
        let mut old = std::mem::replace(&mut self.messages, new);
        // Fetches for the old messages now answer as stale replies and are released.
        self.loading.clear();
        let lazy: Vec<Value> = std::mem::take(&mut self.lazy).into_values().collect();
        if let Some(old) = &mut old {
            old.value = json!([old.value.take(), lazy]);
        }
        old
    }

    /// Drops the picked subterm once no shown result holds its reference.
    fn unpick_if_gone(&mut self) -> Vec<Effect> {
        match &self.picked {
            Some(picked) if self.owner(&picked.info).is_none() => self.unpick(),
            _ => vec![],
        }
    }

    fn go_to(&mut self, kind: &str) -> Vec<Effect> {
        let Some(picked) = &self.picked else {
            return vec![];
        };
        let (uri, session) = (picked.uri.clone(), picked.session.clone());
        let params = json!({"kind":kind,"info":picked.info});
        let (id, call) = self.rpc(&uri, &session, "Lean.Widget.getGoToLocation", params);
        self.goto = Some(id);
        vec![call]
    }

    /// Opens the first location Lean found in Zed.
    fn open_location(&mut self, result: &Value) -> Vec<Effect> {
        let Some(target) = result.as_array().and_then(|l| l.first()) else {
            self.status = Some("No location found".into());
            return vec![];
        };
        let show = json!({"uri":target["targetUri"],"selection":target["targetSelectionRange"],"takeFocus":true});
        let (id, call) = self.call("showDocument", json!([show]));
        self.open = Some(id);
        vec![call]
    }

    /// A widget RPC call into `session` for the file `uri`.
    fn rpc(&mut self, uri: &str, session: &Value, method: &str, params: Value) -> (String, Effect) {
        let shown = self.location.as_ref().filter(|l| l["uri"] == uri);
        let position = shown.map_or(json!({"line":0,"character":0}), |l| {
            l["range"]["start"].clone()
        });
        let call = json!({"textDocument":{"uri":uri},"position":position,
            "sessionId":session,"method":method,"params":params});
        let (id, call) = self.call("sendClientRequest", json!([uri, "$/lean/rpc/call", call]));
        self.inflight
            .insert(id.clone(), (uri.to_owned(), session.clone()));
        (id, call)
    }

    /// The ids of requests still waiting for a reply.
    fn pending(&mut self) -> [&mut Option<String>; 9] {
        [
            &mut self.diags,
            &mut self.popup,
            &mut self.goto,
            &mut self.open,
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

    /// Shows `location` and asks for its goals, opening an RPC session for the file first.
    fn show(&mut self, location: Option<Value>) -> Vec<Effect> {
        self.location = location;
        let Some(location) = self.location.clone() else {
            (self.goal, self.term, self.diags) = (None, None, None);
            let held = [
                self.goals.take(),
                self.term_goal.take(),
                self.replace_messages(None),
            ];
            return held.into_iter().flat_map(|h| self.release(h)).collect();
        };
        let uri = location["uri"].as_str().unwrap_or_default().to_owned();
        let Some(session) = self.sessions.get(&uri).cloned() else {
            (self.goal, self.term, self.diags) = (None, None, None);
            if self.connects.values().any(|u| *u == uri) {
                return vec![];
            }
            let (id, connect) = self.call("createRpcSession", json!([uri]));
            self.connects.insert(id, uri);
            return vec![connect];
        };
        let params = json!({"textDocument":{"uri":uri},"position":location["range"]["start"]});
        let (goal_id, goal) = self.rpc(
            &uri,
            &session,
            "Lean.Widget.getInteractiveGoals",
            params.clone(),
        );
        let (term_id, term) =
            self.rpc(&uri, &session, "Lean.Widget.getInteractiveTermGoal", params);
        let line = location["range"]["start"]["line"]
            .as_u64()
            .unwrap_or_default();
        let range = json!({"lineRange":{"start":line,"end":line + 1}});
        let (diags_id, diags) = self.rpc(
            &uri,
            &session,
            "Lean.Widget.getInteractiveDiagnostics",
            range,
        );
        self.diags = Some(diags_id);
        self.goal = Some(goal_id);
        self.term = Some(term_id);
        vec![goal, term, diags]
    }

    /// Asks Lean for the children of a lazily sent trace node.
    fn fetch(&mut self, children: Value) -> Vec<Effect> {
        let Some(p) = goals::ref_key(&children).map(String::from) else {
            return vec![];
        };
        let held = self.messages.as_ref();
        let Some((uri, session)) = held.map(|h| (h.uri.clone(), h.session.clone())) else {
            return vec![];
        };
        if self.loading.values().any(|q| *q == p) {
            return vec![];
        }
        let method = "Lean.Widget.lazyTraceChildrenToInteractive";
        let (id, call) = self.rpc(&uri, &session, method, children);
        self.loading.insert(id, p);
        vec![call]
    }

    fn held(&self, value: Option<Value>) -> Option<Held> {
        let (uri, session) = self.asked.clone()?;
        value.map(|value| Held {
            uri,
            session,
            value,
        })
    }

    /// Lets Lean free the references inside a result the view no longer shows.
    fn release(&mut self, old: Option<Held>) -> Vec<Effect> {
        let Some(old) = old else { return vec![] };
        let mut refs = Vec::new();
        goals::refs(&old.value, &mut refs);
        if refs.is_empty() || self.sessions.get(&old.uri) != Some(&old.session) {
            return vec![];
        }
        let params = json!({"uri":old.uri,"sessionId":old.session,"refs":refs});
        let (_, release) = self.call(
            "sendClientNotification",
            json!([old.uri, "$/lean/rpc/release", params]),
        );
        vec![release]
    }

    fn goal_list(&self) -> &[Value] {
        self.goals
            .as_ref()
            .and_then(|h| h.value["goals"].as_array())
            .map_or(&[], Vec::as_slice)
    }

    fn publish(&mut self, params: &Value) {
        if let (Some(uri), Some(diagnostics)) =
            (params["uri"].as_str(), params["diagnostics"].as_array())
        {
            self.diagnostics.insert(uri.into(), diagnostics.clone());
        }
    }

    fn copy(&mut self) -> Vec<Effect> {
        let goals: Vec<String> = self.goal_list().iter().map(goals::goal_text).collect();
        let (false, Some(location)) = (goals.is_empty(), &self.location) else {
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

    fn select(&mut self, row: usize) {
        self.selected = row.min(self.rows().len().saturating_sub(1));
        self.follow = true;
    }

    /// Folds or unfolds the innermost block around the selected row.
    /// Folds or unfolds the innermost block around the selected row, fetching
    /// trace children Lean has not sent yet.
    fn toggle_fold(&mut self) -> Vec<Effect> {
        let rows = self.rows();
        // New goals or messages may have left fewer rows than the selection.
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        let Some(key) = rows.get(self.selected).and_then(|r| r.fold.clone()) else {
            return vec![];
        };
        if !self.folded.remove(&key) {
            self.folded.insert(key.clone());
        }
        let Some(header) = rows
            .into_iter()
            .enumerate()
            .find(|(_, r)| r.header && r.fold.as_ref() == Some(&key))
        else {
            return vec![];
        };
        self.selected = header.0;
        match header.1.load {
            Some(children) if !header.1.open => self.fetch(children),
            _ => vec![],
        }
    }

    fn rows(&self) -> Vec<view::Row> {
        let mut rows = Rows::new(&self.folded);
        self.build(&mut rows);
        rows.rows
    }

    fn build(&self, rows: &mut Rows) {
        let Some(location) = &self.location else {
            return rows.push(Line::raw("No Lean file at the cursor"));
        };
        let uri = location["uri"].as_str().unwrap_or_default();
        let start = &location["range"]["start"];
        let cursor = start["line"].as_u64().unwrap_or_default();
        let pinned = if self.pinned { " [pinned]" } else { "" };
        rows.push(Line::raw(format!(
            "{}:{}:{}{pinned}",
            uri.rsplit('/').next().unwrap_or(uri),
            cursor + 1,
            start["character"]
        )));
        if self.goals.is_some() {
            let goals = self.goal_list();
            rows.push(Line::raw(""));
            rows.open("tactic".into(), Line::from("Tactic state".bold()));
            match goals.len() {
                0 => rows.push(Line::raw("No goals")),
                1 => rows.push(Line::raw("1 goal")),
                n => rows.push(Line::raw(format!("{n} goals"))),
            }
            for (i, goal) in goals.iter().enumerate() {
                if i > 0 {
                    rows.push(Line::raw(""));
                }
                let mut lines = goals::goal_lines(goal).into_iter();
                // A named goal folds under its `case` line; several unnamed ones get a number.
                let case = goal["userName"].as_str();
                let header = match case {
                    Some(_) => lines.next(),
                    None if goals.len() > 1 => Some(Line::raw(format!("goal {}", i + 1)).into()),
                    None => None,
                };
                if let Some(header) = header {
                    rows.open(
                        format!("goal:{}", case.map_or(i.to_string(), String::from)),
                        header,
                    );
                    lines.for_each(|l| rows.push(l));
                    rows.close();
                } else {
                    lines.for_each(|l| rows.push(l));
                }
            }
            rows.close();
        }
        if let Some(term) = &self.term_goal {
            rows.push(Line::raw(""));
            rows.open("term".into(), Line::from("Expected type".bold()));
            goals::goal_lines(&term.value)
                .into_iter()
                .for_each(|l| rows.push(l));
            rows.close();
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
        // At the cursor, Lean's interactive messages replace the plain ones.
        let interactive = self.messages.as_ref().filter(|_| !self.all);
        let interactive = interactive.and_then(|h| h.value.as_array());
        let count = interactive.map_or(messages.len(), Vec::len);
        rows.push(Line::raw(""));
        rows.open(
            "messages".into(),
            Line::from(format!("{title} ({count})").bold()),
        );
        if let Some(list) = interactive {
            for (i, d) in list.iter().enumerate() {
                let at = &d["range"]["start"];
                let (line, character) = (
                    at["line"].as_u64().unwrap_or_default() + 1,
                    &at["character"],
                );
                let head = Line::raw(format!("{line}:{character} {}: ", severity(d)));
                let key = format!("message:{uri}:{line}:{character}:{i}");
                messages::message(rows, &self.lazy, &key, head, &d["message"]);
            }
            return rows.close();
        }
        for d in messages {
            let at = &d["range"]["start"];
            let mut text = d["message"].as_str().unwrap_or_default().lines().peekable();
            let first = format!(
                "{}:{} {}: {}",
                at["line"].as_u64().unwrap_or_default() + 1,
                at["character"],
                severity(d),
                text.next().unwrap_or_default()
            );
            if text.peek().is_none() {
                rows.push(Line::raw(first));
                continue;
            }
            rows.open(format!("message:{uri}:{first}"), Line::raw(first));
            text.for_each(|l| rows.push(Line::raw(format!("  {l}"))));
            rows.close();
        }
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
        let rpc = |id: u64, method: &str| {
            json!({"id":format!("tui-1:{id}"),"call":"sendClientRequest","args":[U,"$/lean/rpc/call",
                {"textDocument":{"uri":U},"position":params["position"],"sessionId":"s1","method":method,"params":params}]})
        };
        vec![
            rpc(first, "Lean.Widget.getInteractiveGoals"),
            rpc(first + 1, "Lean.Widget.getInteractiveTermGoal"),
            json!({"id":format!("tui-1:{}", first + 2),"call":"sendClientRequest","args":[U,"$/lean/rpc/call",
                {"textDocument":{"uri":U},"position":params["position"],"sessionId":"s1",
                 "method":"Lean.Widget.getInteractiveDiagnostics","params":{"lineRange":{"start":line,"end":line + 1}}}]}),
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
        app.on_message(
            &json!({"id":"tui-1:1","result":{"location":loc(1, 2),"initializeResult":{}}}),
        );
        app.on_message(&json!({"id":"tui-1:3","result":"s1"}));
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
        assert_eq!(screen(&app)[19], "bad params");
    }

    #[test]
    fn overtaken_and_cancelled_lookups_are_dropped_silently() {
        let mut app = app();
        let keys = KEYS;
        for (id, code) in [(4, -32801), (5, -32800)] {
            app.on_message(
                &json!({"id":format!("tui-1:{id}"),"error":{"code":code,"message":"File changed."}}),
            );
            assert_eq!(screen(&app)[19], keys);
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
            screen(&app)[19],
            "no Lean server found; open a Lean file in Zed"
        );
        app.connected("http://127.0.0.1:1/?token=t".into());
        assert_eq!(screen(&app)[19], KEYS);
    }

    #[test]
    fn subscribe_reply_replaces_all_diagnostics() {
        let mut app = app();
        app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[publish(1, 0, 1, 1, "old")]}]}));
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
        assert_eq!(screen(&app)[19], "No goals to copy");
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
        let keys = KEYS;
        assert_eq!(screen(&app)[19], keys);
        app.on_key(key('c'));
        assert_eq!(screen(&app)[19], "No goals to copy");
        app.on_key(key('x'));
        assert_eq!(screen(&app)[19], keys);
    }

    #[test]
    fn one_session_per_file_and_reconnect_when_lean_drops_it() {
        let mut app = App::new("tui-1".into());
        app.connected("http://h/".into());
        app.on_message(
            &json!({"id":"tui-1:1","result":{"location":loc(1, 2),"initializeResult":{}}}),
        );
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
        assert_eq!(screen(&app)[19], KEYS);
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
        let target =
            json!({"append":[{"tag":[{"info":{"p":"9"}},{"text":"Nat.succ"}]},{"text":" n"}]});
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
            [rpc(
                "tui-1:7",
                "Lean.Widget.InteractiveDiagnostics.infoToInteractive",
                json!({"p":"9"})
            )]
        );
        shows(&app, &["Nat.succ"]);
        assert_eq!(screen(&app)[19], PICKED_KEYS);
        app.on_message(
            &json!({"id":"tui-1:7","result":{"type":{"text":"Nat → Nat"},"doc":"The successor."}}),
        );
        shows(&app, &["Nat.succ : Nat → Nat", "The successor."]);
        assert_eq!(
            sent(app.on_key(key('d'))),
            [rpc(
                "tui-1:8",
                "Lean.Widget.getGoToLocation",
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
        assert_eq!(screen(&app)[19], "Zed could not open the location");
        app.on_key(key('t'));
        app.on_message(&json!({"id":"tui-1:10","result":[]}));
        assert_eq!(screen(&app)[19], "No location found");
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
        assert_eq!(screen(&app)[19], KEYS);
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
        app.on_message(&json!({"id":"tui-1:2","result":[{"uri":U,"diagnostics":[publish(1, 2, 1, 2, "plain")]}]}));
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
            "Lean.Widget.lazyTraceChildrenToInteractive"
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
}
