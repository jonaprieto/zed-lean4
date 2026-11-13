//! The terminal infoview's state, and how it answers page messages, keys and the mouse.
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde_json::{Value, json};

use crate::settings;

use super::{
    detail::Picked,
    goals, hover, lean, markdown,
    rpc::{Held, Rpc, Session},
    screen::{Cache, Screen},
    theme::{self, Theme},
    view::{self, Text},
};

mod draw;
#[cfg(test)]
mod tests;

/// Lines the wheel scrolls per notch.
const WHEEL: usize = 3;

#[derive(Debug, PartialEq)]
pub enum Effect {
    Send(Value),
    OpenBrowser,
    Quit,
}

/// The id of the latest request of each kind still waiting for a reply; a
/// newer request of the same kind supersedes it.
#[derive(Default)]
struct Pending {
    hello: Option<String>,
    subscribe: Option<String>,
    subscribe_progress: Option<String>,
    /// The goals, expected type and interactive messages at the shown location.
    goal: Option<String>,
    term: Option<String>,
    diags: Option<String>,
    /// The hover at the shown location.
    hover: Option<String>,
    insert: Option<String>,
    popup: Option<String>,
    goto: Option<String>,
    open: Option<String>,
}

impl Pending {
    fn slots(&mut self) -> [&mut Option<String>; 11] {
        [
            &mut self.hover,
            &mut self.hello,
            &mut self.subscribe,
            &mut self.subscribe_progress,
            &mut self.goal,
            &mut self.term,
            &mut self.diags,
            &mut self.insert,
            &mut self.popup,
            &mut self.goto,
            &mut self.open,
        ]
    }

    /// Whether some request waits for the reply `id`.
    fn awaits(&mut self, id: &str) -> bool {
        self.slots().iter().any(|slot| slot.as_deref() == Some(id))
    }

    /// Whether `id` asks for the goals, expected type or messages.
    fn is_lookup(&self, id: &str) -> bool {
        [&self.goal, &self.term, &self.diags, &self.hover]
            .iter()
            .any(|slot| slot.as_deref() == Some(id))
    }

    /// Stops waiting for the reply `id`.
    fn forget(&mut self, id: &str) {
        for slot in self.slots() {
            clear_if_awaiting(slot, id);
        }
    }

    fn forget_lookups(&mut self) {
        (self.goal, self.term, self.diags, self.hover) = (None, None, None, None);
    }
}

/// Clears `slot` when it waits for the reply `id`, and says whether it did.
fn clear_if_awaiting(slot: &mut Option<String>, id: &str) -> bool {
    let hit = slot.as_deref() == Some(id);
    if hit {
        *slot = None;
    }
    hit
}

pub struct App {
    rpc: Rpc,
    page_url: Option<String>,
    /// The location the view shows; it stays put while pinned.
    location: Option<Value>,
    /// The latest cursor location, pinned or not.
    latest: Option<Value>,
    pinned: bool,
    goals: Option<Held>,
    term_goal: Option<Held>,
    /// The goals and expected type, typeset once when they arrive.
    goal_texts: Vec<Vec<Text>>,
    term_texts: Vec<Text>,
    /// Lean's hover at the shown location: signature and docstring.
    hover: Option<Vec<markdown::Hanging>>,
    cache: Cache,
    /// Interactive diagnostics on the shown line.
    messages: Option<Held>,
    /// Trace children fetched so far, by reference, and the requests fetching them.
    trace_children: HashMap<String, Value>,
    fetching: HashMap<String, String>,
    /// Published diagnostics per file URI.
    diagnostics: HashMap<String, Vec<Value>>,
    /// The ranges Lean is still elaborating, per file URI.
    progress: HashMap<String, Vec<Value>>,
    /// Whether the view lists every message of the file, not only the shown line's.
    all_messages: bool,
    /// The selected row, which `enter` folds or unfolds.
    selected: usize,
    folded: HashSet<String>,
    /// First visible screen line; render keeps the selection in view while `follow`.
    scroll: Cell<usize>,
    follow: bool,
    screen: RefCell<Screen>,
    picked: Option<Picked>,
    status: Option<String>,
    pending: Pending,
}

impl App {
    pub fn new(id_prefix: String) -> Self {
        Self {
            rpc: Rpc::new(id_prefix),
            page_url: None,
            location: None,
            latest: None,
            pinned: false,
            goals: None,
            term_goal: None,
            goal_texts: Vec::new(),
            term_texts: Vec::new(),
            hover: None,
            cache: Cache::default(),
            messages: None,
            trace_children: HashMap::new(),
            fetching: HashMap::new(),
            diagnostics: HashMap::new(),
            progress: HashMap::new(),
            all_messages: false,
            selected: 0,
            folded: HashSet::new(),
            scroll: Cell::new(0),
            follow: true,
            screen: RefCell::default(),
            picked: None,
            status: None,
            pending: Pending::default(),
        }
    }

    pub fn connected(&mut self, page_url: String) -> Vec<Effect> {
        self.dirty();
        self.page_url = Some(page_url);
        self.status = None;
        let (hello_id, hello) = self.rpc.call(lean::HELLO, json!([]));
        let (subscribe_id, subscribe) = self.rpc.call(
            lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
            json!([lean::PUBLISH_DIAGNOSTICS]),
        );
        let (progress_id, subscribe_progress) = self.rpc.call(
            lean::SUBSCRIBE_SERVER_NOTIFICATIONS,
            json!([lean::FILE_PROGRESS]),
        );
        self.pending.hello = Some(hello_id);
        self.pending.subscribe = Some(subscribe_id);
        self.pending.subscribe_progress = Some(progress_id);
        vec![hello, subscribe, subscribe_progress]
    }

    pub fn disconnected(&mut self, status: String) {
        self.dirty();
        self.page_url = None;
        self.status = Some(status);
        self.rpc.reset();
        self.fetching.clear();
        self.pending = Pending::default();
    }

    pub fn on_message(&mut self, m: &Value) -> Vec<Effect> {
        let event = m["event"].as_str();
        let notification = m["args"][0].as_str();
        if event == Some(lean::GOT_SERVER_NOTIFICATION) {
            match notification {
                Some(lean::PUBLISH_DIAGNOSTICS) => {}
                Some(lean::FILE_PROGRESS) => {
                    let params = &m["args"][1];
                    self.record_progress(params);
                    // Only the shown file's progress is on screen.
                    if params["textDocument"]["uri"]
                        .as_str()
                        .is_some_and(|uri| self.is_shown(uri))
                    {
                        self.dirty();
                    }
                    return vec![];
                }
                // Other notifications change nothing on screen.
                _ => return vec![],
            }
        }
        self.dirty();
        match event {
            Some(lean::CHANGED_CURSOR_LOCATION) => return self.cursor_moved(&m["args"][0]),
            Some(lean::GOT_SERVER_NOTIFICATION) => {
                self.record_diagnostics(&m["args"][1]);
                // Lean re-elaborated the shown file: its goals may have changed.
                if m["args"][1]["uri"]
                    .as_str()
                    .is_some_and(|uri| self.is_shown(uri))
                {
                    return self.show(self.location.clone());
                }
                return vec![];
            }
            Some(lean::SENT_CLIENT_NOTIFICATION)
                if m["args"][0] == lean::DID_CHANGE_CONFIGURATION =>
            {
                return self.apply_settings(&m["args"][1]["settings"][settings::INFOVIEW]);
            }
            Some(_) => return vec![],
            None => {}
        }
        let Some(id) = m["id"].as_str() else {
            return vec![];
        };
        if let Some(uri) = self.rpc.connected(id, m) {
            if let Some(error) = m.get("error") {
                self.status = error["message"].as_str().map(String::from);
                return vec![];
            }
            return if self.is_shown(&uri) {
                self.show(self.location.clone())
            } else {
                vec![]
            };
        }
        // The session an RPC reply came from: its references live there.
        let origin = self.rpc.answered(id, m);
        if let Some(key) = self.fetching.remove(id) {
            if let Some(error) = m.get("error") {
                self.status = error["message"].as_str().map(String::from);
                return vec![];
            }
            // Children for the messages on screen; anything older only holds references.
            let current = self.messages.as_ref().map(|h| &h.session);
            if origin.is_some() && origin.as_ref() == current {
                self.trace_children.insert(key, m["result"].clone());
                return vec![];
            }
            return self
                .rpc
                .release(Held::new(origin, m.get("result").cloned()));
        }
        if !self.pending.awaits(id) {
            // A reply nobody waits for any more still holds references.
            return self
                .rpc
                .release(Held::new(origin, m.get("result").cloned()));
        }
        if let Some(error) = m.get("error") {
            let was_lookup = self.pending.is_lookup(id);
            self.pending.forget(id);
            let code = error["code"].as_i64();
            if was_lookup && code == Some(lean::RPC_NEEDS_RECONNECT) {
                self.pending.forget_lookups();
                return self.show(self.location.clone());
            }
            // An edit overtook the request or it was cancelled: not worth a status.
            let overtaken = matches!(code, Some(lean::CONTENT_MODIFIED | lean::REQUEST_CANCELLED));
            if !(was_lookup && overtaken) {
                self.status = Some(error["message"].as_str().unwrap_or("request failed").into());
            }
            return vec![];
        }
        let result = &m["result"];
        let pending = &mut self.pending;
        if clear_if_awaiting(&mut pending.hello, id) {
            // The run button started this view, but the browser page is wanted.
            if settings::opens_browser(&result["infoview"]) {
                return vec![Effect::OpenBrowser, Effect::Quit];
            }
            self.apply_settings(&result["infoview"]);
            return self.cursor_moved(&result["location"]);
        }
        if clear_if_awaiting(&mut pending.subscribe, id) {
            self.diagnostics.clear();
            for params in result.as_array().into_iter().flatten() {
                self.record_diagnostics(params);
            }
        } else if clear_if_awaiting(&mut pending.subscribe_progress, id) {
            self.progress.clear();
            for params in result.as_array().into_iter().flatten() {
                self.record_progress(params);
            }
        } else if clear_if_awaiting(&mut pending.popup, id) {
            if let Some(picked) = &mut self.picked {
                picked.popup = Some(result.clone());
            }
        } else if clear_if_awaiting(&mut pending.goto, id) {
            return self.open_location(result);
        } else if clear_if_awaiting(&mut pending.open, id) {
            if result["success"] == false {
                self.status = Some("Zed could not open the location".into());
            }
        } else if clear_if_awaiting(&mut pending.goal, id) {
            let held = Held::new(
                origin,
                result.get("goals").is_some().then(|| result.clone()),
            );
            let old = std::mem::replace(&mut self.goals, held);
            self.goal_texts = self.goal_list().iter().map(goals::goal_lines).collect();
            return self.release_replaced(old);
        } else if clear_if_awaiting(&mut pending.diags, id) {
            let old =
                self.replace_messages(Held::new(origin, result.is_array().then(|| result.clone())));
            let mut effects = self.release_replaced(old);
            let open: Vec<Value> = self
                .rows()
                .iter()
                .filter(|r| r.open)
                .filter_map(|r| r.load.clone())
                .collect();
            for children in open {
                effects.extend(self.fetch(children));
            }
            return effects;
        } else if clear_if_awaiting(&mut pending.term, id) {
            let held = Held::new(origin, (!result.is_null()).then(|| result.clone()));
            let old = std::mem::replace(&mut self.term_goal, held);
            self.term_texts = self
                .term_goal
                .as_ref()
                .map_or(vec![], |t| goals::goal_lines(&t.value));
            return self.release_replaced(old);
        } else if clear_if_awaiting(&mut pending.hover, id) {
            self.hover = hover::lines(result);
        } else if clear_if_awaiting(&mut pending.insert, id) && result["applied"] == false {
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
                KeyCode::Char('d') => return self.go_to(lean::GO_TO_DEFINITION),
                KeyCode::Char('t') => return self.go_to(lean::GO_TO_TYPE),
                _ => {}
            }
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return vec![Effect::Quit],
            KeyCode::Char('p') if self.location.is_some() => {
                self.dirty();
                self.pinned = !self.pinned;
                if !self.pinned && self.latest != self.location {
                    return self.show(self.latest.clone());
                }
            }
            KeyCode::Char('a') => {
                self.dirty();
                self.all_messages = !self.all_messages;
            }
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

    fn click(&mut self, x: u16, y: u16) -> Vec<Effect> {
        let Some((segment, col)) = self.screen.borrow().hit(x, y) else {
            return vec![];
        };
        self.selected = segment.row;
        self.follow = true;
        let rows = self.rows();
        let Some(row) = rows.get(segment.row) else {
            return vec![];
        };
        let char = segment.char_at(&row.line, col);
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
        let Some(session) = self.owner(&info) else {
            return effects;
        };
        let (id, call) = self.request(&session, lean::INFO_TO_INTERACTIVE, info.clone());
        self.pending.popup = Some(id);
        self.picked = Some(Picked {
            label,
            info,
            session,
            popup: None,
        });
        effects.push(call);
        effects
    }

    /// Closes the detail panel and releases the references its popup held.
    fn unpick(&mut self) -> Vec<Effect> {
        self.pending.popup = None;
        self.pending.goto = None;
        let Some(picked) = self.picked.take() else {
            return vec![];
        };
        self.rpc.release(picked.into_popup())
    }

    /// The session of the shown result that holds `info`; fetched trace
    /// children belong to the messages' session.
    fn owner(&self, info: &Value) -> Option<Session> {
        let shown = [&self.goals, &self.term_goal, &self.messages]
            .into_iter()
            .flatten();
        if let Some(held) = shown.into_iter().find(|h| goals::holds(&h.value, info)) {
            return Some(held.session.clone());
        }
        let messages = self.messages.as_ref()?;
        let fetched = self.trace_children.values().any(|v| goals::holds(v, info));
        fetched.then(|| messages.session.clone())
    }

    /// Shows `new` messages and hands back the old ones together with the
    /// trace children fetched for them, which hold references of the same session.
    fn replace_messages(&mut self, new: Option<Held>) -> Option<Held> {
        let mut old = std::mem::replace(&mut self.messages, new);
        // Fetches for the old messages now answer as stale replies and are released.
        self.fetching.clear();
        let children: Vec<Value> = std::mem::take(&mut self.trace_children)
            .into_values()
            .collect();
        if let Some(old) = &mut old {
            old.value = json!([old.value.take(), children]);
        }
        old
    }

    /// After a shown result was replaced by a newer one: drops the picked
    /// subterm if no shown result holds it any more, then releases `old`.
    fn release_replaced(&mut self, old: Option<Held>) -> Vec<Effect> {
        let mut effects = match &self.picked {
            Some(picked) if self.owner(&picked.info).is_none() => self.unpick(),
            _ => vec![],
        };
        effects.extend(self.rpc.release(old));
        effects
    }

    fn go_to(&mut self, kind: &str) -> Vec<Effect> {
        let Some(picked) = &self.picked else {
            return vec![];
        };
        let session = picked.session.clone();
        let params = json!({"kind":kind,"info":picked.info});
        let (id, call) = self.request(&session, lean::GET_GO_TO_LOCATION, params);
        self.pending.goto = Some(id);
        vec![call]
    }

    /// Opens the first location Lean found in Zed.
    fn open_location(&mut self, result: &Value) -> Vec<Effect> {
        let Some(target) = result.as_array().and_then(|l| l.first()) else {
            self.status = Some("No location found".into());
            return vec![];
        };
        let show = json!({"uri":target["targetUri"],"selection":target["targetSelectionRange"],"takeFocus":true});
        let (id, call) = self.rpc.call(lean::SHOW_DOCUMENT, json!([show]));
        self.pending.open = Some(id);
        vec![call]
    }

    /// A widget RPC call into `session`, at the shown location when it is in
    /// the session's file and at the file's start otherwise.
    fn request(&mut self, session: &Session, method: &str, params: Value) -> (String, Effect) {
        let shown = self
            .location
            .as_ref()
            .filter(|l| l["uri"] == session.uri.as_str());
        let position = shown.map_or(json!({"line":0,"character":0}), |l| {
            l["range"]["start"].clone()
        });
        self.rpc.request(session, position, method, params)
    }

    /// Takes up the `infoview` settings: draws with their theme from now on and
    /// says what in them is wrong. Goals and the hover keep the colors they
    /// were typeset with, so a new theme asks for them again.
    fn apply_settings(&mut self, infoview: &Value) -> Vec<Effect> {
        let (new, errors) = Theme::from_settings(infoview);
        if !errors.is_empty() {
            self.status = Some(errors.join("; "));
        }
        if new == theme::current() {
            return vec![];
        }
        theme::set(new);
        self.dirty();
        self.show(self.location.clone())
    }

    fn cursor_moved(&mut self, location: &Value) -> Vec<Effect> {
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
            self.pending.forget_lookups();
            (self.goal_texts, self.term_texts, self.hover) = (vec![], vec![], None);
            let held = [
                self.goals.take(),
                self.term_goal.take(),
                self.replace_messages(None),
            ];
            return held.into_iter().flat_map(|h| self.rpc.release(h)).collect();
        };
        let uri = location["uri"].as_str().unwrap_or_default().to_owned();
        let Some(session) = self.rpc.session(&uri) else {
            self.pending.forget_lookups();
            return self.rpc.connect(uri);
        };
        let start = &location["range"]["start"];
        let params = json!({"textDocument":{"uri":uri},"position":start});
        let (goal_id, goal) = self.request(&session, lean::GET_INTERACTIVE_GOALS, params.clone());
        let (term_id, term) =
            self.request(&session, lean::GET_INTERACTIVE_TERM_GOAL, params.clone());
        let line = start["line"].as_u64().unwrap_or_default();
        let range = json!({"lineRange":{"start":line,"end":line + 1}});
        let (diags_id, diags) = self.request(&session, lean::GET_INTERACTIVE_DIAGNOSTICS, range);
        let hover_args = json!([uri, lean::HOVER, params]);
        let (hover_id, hover) = self.rpc.call(lean::SEND_CLIENT_REQUEST, hover_args);
        self.pending.diags = Some(diags_id);
        self.pending.goal = Some(goal_id);
        self.pending.term = Some(term_id);
        self.pending.hover = Some(hover_id);
        vec![goal, term, diags, hover]
    }

    /// Asks Lean for the children of a lazily sent trace node.
    fn fetch(&mut self, children: Value) -> Vec<Effect> {
        let Some(key) = goals::ref_key(&children).map(String::from) else {
            return vec![];
        };
        let Some(session) = self.messages.as_ref().map(|h| h.session.clone()) else {
            return vec![];
        };
        if self.fetching.values().any(|k| *k == key) {
            return vec![];
        }
        let method = lean::LAZY_TRACE_CHILDREN_TO_INTERACTIVE;
        let (id, call) = self.request(&session, method, children);
        self.fetching.insert(id, key);
        vec![call]
    }

    fn goal_list(&self) -> &[Value] {
        self.goals
            .as_ref()
            .and_then(|h| h.value["goals"].as_array())
            .map_or(&[], Vec::as_slice)
    }

    /// Keeps the diagnostics of one `publishDiagnostics` notification.
    fn record_diagnostics(&mut self, params: &Value) {
        if let (Some(uri), Some(diagnostics)) =
            (params["uri"].as_str(), params["diagnostics"].as_array())
        {
            self.diagnostics.insert(uri.into(), diagnostics.clone());
        }
    }

    /// Keeps the ranges of one `$/lean/fileProgress` notification.
    fn record_progress(&mut self, params: &Value) {
        if let (Some(uri), Some(processing)) = (
            params["textDocument"]["uri"].as_str(),
            params["processing"].as_array(),
        ) {
            self.progress.insert(uri.into(), processing.clone());
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
        let (id, insert) = self
            .rpc
            .call(lean::INSERT_TEXT, json!([text, "above", pos]));
        self.pending.insert = Some(id);
        vec![insert]
    }

    fn select(&mut self, row: usize) {
        self.selected = row.min(self.rows().len().saturating_sub(1));
        self.follow = true;
    }

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
        self.dirty();
        let Some((i, header)) = rows
            .iter()
            .enumerate()
            .find(|(_, r)| r.header && r.fold.as_ref() == Some(&key))
        else {
            return vec![];
        };
        self.selected = i;
        match &header.load {
            Some(children) if !header.open => self.fetch(children.clone()),
            _ => vec![],
        }
    }

    /// Forgets the cached rows after a change to what they show.
    /// Whether the view shows the file at `uri`.
    fn is_shown(&self, uri: &str) -> bool {
        self.location.as_ref().is_some_and(|l| l["uri"] == uri)
    }

    fn dirty(&self) {
        self.cache.clear();
    }
}
