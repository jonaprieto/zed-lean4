//! Runs the built proxy in front of a real `lake serve` on tests/fixtures/lean.
//! Needs elan on PATH: `cargo test -p lean4-ide-proxy --test lean_e2e -- --ignored`.
mod common;

use common::{screen, screen_of};
use futures_util::{SinkExt, StreamExt};
use lean4_ide_proxy::codec;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/lean")
        .canonicalize()
        .unwrap()
}

fn fixture_uri() -> String {
    format!("file://{}", fixture_dir().join("Fixture.lean").display())
}

struct Lsp {
    _child: Child,
    tx: ChildStdin,
    rx: BufReader<ChildStdout>,
    next: i64,
    show_url: Option<String>,
}

impl Lsp {
    async fn start() -> Lsp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lean4-ide-proxy"))
            .args(["--", "lake", "serve", "--"])
            .current_dir(fixture_dir())
            // Keep the launcher link and discovery file out of the real home;
            // elan still needs to find its toolchains there.
            .env(
                "ELAN_HOME",
                std::env::var_os("ELAN_HOME")
                    .unwrap_or_else(|| format!("{}/.elan", std::env::var("HOME").unwrap()).into()),
            )
            .env("HOME", std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let (tx, rx) = (
            child.stdin.take().unwrap(),
            BufReader::new(child.stdout.take().unwrap()),
        );
        let mut lsp = Lsp {
            _child: child,
            tx,
            rx,
            next: 0,
            show_url: None,
        };
        let root = format!("file://{}", fixture_dir().display());
        lsp.request(
            "initialize",
            json!({"processId": null, "rootUri": root, "capabilities": {}}),
        )
        .await;
        lsp.notify("initialized", json!({})).await;
        let text = std::fs::read_to_string(fixture_dir().join("Fixture.lean")).unwrap();
        lsp.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": fixture_uri(), "languageId": "lean4", "version": 1, "text": text}}),
        )
        .await;
        lsp
    }

    async fn notify(&mut self, method: &str, params: Value) {
        codec::write(
            &mut self.tx,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
        .await
        .unwrap();
    }

    /// Sends a request and returns its response, skipping notifications and server requests.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        codec::write(
            &mut self.tx,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .await
        .unwrap();
        let rx = &mut self.rx;
        let tx = &mut self.tx;
        let show_url = &mut self.show_url;
        let read = async {
            loop {
                let m = codec::read(&mut *rx).await.unwrap().expect("proxy closed");
                if m["method"] == "window/showDocument" {
                    *show_url = m["params"]["uri"].as_str().map(str::to_owned);
                    codec::write(
                        &mut *tx,
                        &json!({"jsonrpc":"2.0","id":m["id"],"result":{"success":true}}),
                    )
                    .await
                    .unwrap();
                }
                if m.get("id") == Some(&json!(id)) && m.get("method").is_none() {
                    return m;
                }
            }
        };
        match tokio::time::timeout(std::time::Duration::from_secs(120), read).await {
            Ok(m) => m,
            Err(_) => panic!("no response to {method} within 120s"),
        }
    }
}

type InfoviewSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn page_call(socket: &mut InfoviewSocket, id: &str, name: &str, args: Value) -> Value {
    socket
        .send(Message::text(
            json!({"id":id,"call":name,"args":args}).to_string(),
        ))
        .await
        .unwrap();
    loop {
        let message = socket.next().await.unwrap().unwrap();
        let Message::Text(text) = message else {
            continue;
        };
        let response: Value = serde_json::from_str(text.as_str()).unwrap();
        if response["id"] == id {
            assert!(response.get("error").is_none(), "{response}");
            return response["result"].clone();
        }
    }
}

fn tagged_text(value: &Value) -> String {
    if let Some(text) = value["text"].as_str() {
        return text.into();
    }
    if let Some(parts) = value["append"].as_array() {
        return parts.iter().map(tagged_text).collect();
    }
    if let Some(tag) = value["tag"].as_array() {
        return tagged_text(&tag[1]);
    }
    panic!("unexpected TaggedText: {value}");
}

#[tokio::test]
#[ignore = "needs elan and Lean v4.33.1"]
async fn plain_goal_passes_through() {
    let mut lsp = Lsp::start().await;
    let pos =
        json!({"textDocument": {"uri": fixture_uri()}, "position": {"line": 1, "character": 2}});
    let resp = lsp.request("$/lean/plainGoal", pos).await;
    assert_eq!(resp["result"]["goals"], json!(["n : Nat\n⊢ n + 0 = n"]));
}

#[tokio::test]
#[ignore = "needs elan and Lean v4.33.1"]
async fn restart_file_reopens_the_edited_text_in_lean() {
    let mut lsp = Lsp::start().await;
    let doc = json!({"uri": fixture_uri(), "version": 2});
    // A comment above the theorem moves its proof from line 1 to line 2.
    let top = json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}});
    lsp.notify(
        "textDocument/didChange",
        json!({"textDocument": doc, "contentChanges": [{"range": top, "text": "-- edited\n"}]}),
    )
    .await;
    let restart = lsp
        .request(
            "workspace/executeCommand",
            json!({"command": "lean4-ide.restartFile", "arguments": [fixture_uri()]}),
        )
        .await;
    assert_eq!(restart.get("error"), None, "{restart}");
    let pos =
        json!({"textDocument": {"uri": fixture_uri()}, "position": {"line": 2, "character": 2}});
    let resp = lsp.request("$/lean/plainGoal", pos).await;
    assert_eq!(
        resp["result"]["goals"],
        json!(["n : Nat\n⊢ n + 0 = n"]),
        "{resp}"
    );
}

#[tokio::test]
#[ignore = "needs elan and Lean v4.33.1"]
async fn hover_shows_goal_on_tactic() {
    let mut lsp = Lsp::start().await;
    let pos =
        json!({"textDocument": {"uri": fixture_uri()}, "position": {"line": 1, "character": 2}});
    let resp = lsp.request("textDocument/hover", pos).await;
    let value = resp["result"]["contents"]["value"].as_str().unwrap();
    assert!(value.starts_with("This tactic applies"), "{value}");
    assert!(
        value.ends_with("```lean\nn : Nat\n⊢ n + 0 = n\n```"),
        "{value}"
    );
}

#[tokio::test]
#[ignore = "needs elan and Lean v4.33.1"]
async fn infoview_gets_interactive_goals() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let mut lsp = Lsp::start().await;
        let position = json!({
            "textDocument": {"uri": fixture_uri()},
            "position": {"line": 1, "character": 2}
        });
        lsp.request("textDocument/documentHighlight", position.clone())
            .await;
        let opened = lsp
            .request(
                "workspace/executeCommand",
                json!({"command": "lean4-ide.openInfoview"}),
            )
            .await;
        assert!(opened.get("error").is_none(), "{opened}");
        let url = lsp.show_url.as_deref().expect("window/showDocument URL");
        let (origin, query) = url.split_once("/?").unwrap();
        let ws_url = format!("{}/ws?{query}", origin.replacen("http:", "ws:", 1));
        let mut request = ws_url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        let (mut socket, _) = connect_async(request).await.unwrap();
        let hello = page_call(&mut socket, "e2e:1", "hello", json!([])).await;
        assert!(
            hello["initializeResult"]["capabilities"].is_object(),
            "{hello}"
        );
        let session = page_call(
            &mut socket,
            "e2e:2",
            "createRpcSession",
            json!([fixture_uri()]),
        )
        .await;
        assert!(session.is_string(), "{session}");
        let rpc = json!({
            "textDocument": position["textDocument"],
            "position": position["position"],
            "sessionId": session,
            "method": "Lean.Widget.getInteractiveGoals",
            "params": position
        });
        let goals = page_call(
            &mut socket,
            "e2e:3",
            "sendClientRequest",
            json!([fixture_uri(), "$/lean/rpc/call", rpc]),
        )
        .await;
        let goal = &goals["goals"][0];
        let has_v1_ref = goal["ctx"].get("__rpcref").is_some();
        assert!(
            has_v1_ref,
            "Lean must encode RPC refs in wire format v1: {goals}"
        );
        let hyps = goal["hyps"].as_array().unwrap();
        let has_n = hyps.iter().any(|hyp| {
            let names = hyp["names"].as_array().unwrap();
            names.contains(&json!("n"))
        });
        assert!(has_n, "{goals}");
        assert_eq!(tagged_text(&goal["type"]), "n + 0 = n");
        page_call(&mut socket, "e2e:4", "closeRpcSession", json!([session])).await;
        socket.close(None).await.unwrap();
    })
    .await
    .expect("infoview e2e timed out");
}

#[tokio::test]
#[ignore = "needs elan and Lean v4.33.1"]
async fn tui_shows_goal_from_real_lean() {
    use lean4_ide_proxy::tui::{
        app::{App, Effect},
        lean, socket_request,
    };
    use ratatui::{
        crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        },
        layout::Size,
    };

    /// Tall enough to show the hover below the goal and the detail panel.
    const TALL: Size = Size::new(80, 60);
    /// Whether the screen shows the hover section with something in it.
    fn hovered(text: &str) -> bool {
        let mut section = text.lines().skip_while(|l| *l != "▾ At the cursor");
        section.next().is_some() && section.next().is_some_and(|l| !l.is_empty())
    }

    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let mut lsp = Lsp::start().await;
        let opened = lsp
            .request(
                "workspace/executeCommand",
                json!({"command": "lean4-ide.openInfoview"}),
            )
            .await;
        assert!(opened.get("error").is_none(), "{opened}");
        let url = lsp.show_url.clone().expect("window/showDocument URL");
        let (mut socket, _) = connect_async(socket_request(&url).unwrap()).await.unwrap();
        let pid = std::process::id();
        let mut app = App::new(format!("tui-{pid}"));
        let hello_id = format!("tui-{pid}:1");
        let mut effects = app.connected(url);
        let mut moved = false;
        // Goals appear, a click on `Nat` shows its type, `d` opens its definition;
        // Lean's hover shows at the cursor meanwhile.
        let mut clicked = false;
        let mut asked = false;
        let mut opened = false;
        loop {
            for effect in effects.drain(..) {
                let Effect::Send(m) = effect else {
                    panic!("unexpected effect {effect:?}")
                };
                if m["call"] == lean::SHOW_DOCUMENT {
                    let uri = m["args"][0]["uri"].as_str().unwrap();
                    assert!(uri.ends_with("Init/Prelude.lean"), "{uri}");
                    opened = true;
                    continue;
                }
                socket.send(Message::text(m.to_string())).await.unwrap();
            }
            let text = screen_of(&app, TALL);
            if opened && hovered(&text) {
                return;
            }
            if !clicked && text.contains("⊢ n + 0 = n") {
                clicked = true;
                let row = text.lines().position(|l| l.starts_with("n : Nat")).unwrap();
                effects = app.on_mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 4,
                    row: row.try_into().unwrap(),
                    modifiers: KeyModifiers::NONE,
                });
                continue;
            }
            if !asked && text.contains("Nat : Type") {
                asked = true;
                effects = app.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
                continue;
            }
            let msg = socket.next().await.unwrap().unwrap();
            let m: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            effects = app.on_message(&m);
            if m["id"] == hello_id.as_str() && !moved {
                moved = true;
                lsp.request(
                    "textDocument/documentHighlight",
                    json!({
                        "textDocument": {"uri": fixture_uri()},
                        "position": {"line": 1, "character": 2}
                    }),
                )
                .await;
            }
        }
    })
    .await
    .expect("tui e2e timed out");
}

#[tokio::test]
#[ignore = "needs elan and Lean v4.33.1"]
async fn tui_shows_interactive_messages_from_real_lean() {
    use lean4_ide_proxy::tui::{
        app::{App, Effect},
        socket_request,
    };
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let mut lsp = Lsp::start().await;
        let uri = format!("file://{}/Check.lean", fixture_dir().display());
        lsp.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "lean4", "version": 1, "text": "#check Nat.succ\n"}}),
        )
        .await;
        let opened = lsp
            .request(
                "workspace/executeCommand",
                json!({"command": "lean4-ide.openInfoview"}),
            )
            .await;
        assert!(opened.get("error").is_none(), "{opened}");
        let url = lsp.show_url.clone().expect("window/showDocument URL");
        let (mut socket, _) = connect_async(socket_request(&url).unwrap()).await.unwrap();
        let pid = std::process::id();
        let mut app = App::new(format!("tui-{pid}"));
        let hello_id = format!("tui-{pid}:1");
        let mut effects = app.connected(url);
        let mut clicked = false;
        loop {
            for effect in effects.drain(..) {
                let Effect::Send(m) = effect else {
                    panic!("unexpected effect {effect:?}")
                };
                socket.send(Message::text(m.to_string())).await.unwrap();
            }
            let text = screen(&app);
            // The detail panel shows the clicked binder with its type, and
            // the header says Lean checked the whole file.
            if clicked
                && text.lines().any(|l| l.trim_end() == "n : Nat")
                && text.lines().next().is_some_and(|l| l.contains("✓ checked"))
            {
                return;
            }
            // Published diagnostics show first; the click works once the
            // interactive ones replace them.
            if !clicked
                && let Some(row) = text.lines().position(|l| l.contains("information: Nat.succ"))
            {
                let line = text.lines().nth(row).unwrap();
                // `#check` tags the binder `n`, not the constant's name.
                let column = line[..line.find("(n").unwrap() + 1].chars().count();
                effects = app.on_mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: column.try_into().unwrap(),
                    row: row.try_into().unwrap(),
                    modifiers: KeyModifiers::NONE,
                });
                if !effects.is_empty() {
                    clicked = true;
                    continue;
                }
            }
            let msg = socket.next().await.unwrap().unwrap();
            let m: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            effects = app.on_message(&m);
            if m["id"] == hello_id.as_str() {
                lsp.request(
                    "textDocument/documentHighlight",
                    json!({"textDocument": {"uri": uri}, "position": {"line": 0, "character": 1}}),
                )
                .await;
            }
        }
    })
    .await
    .expect("tui message e2e timed out");
}
