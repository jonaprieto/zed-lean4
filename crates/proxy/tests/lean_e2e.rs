//! Runs the built proxy in front of a real `lake serve` on tests/fixtures/lean.
//! Needs elan on PATH: `cargo test -p lean4-ide-proxy --test lean_e2e -- --ignored`.
use lean4_ide_proxy::codec;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

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
}

impl Lsp {
    async fn start() -> Lsp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lean4-ide-proxy"))
            .args(["--", "lake", "serve", "--"])
            .current_dir(fixture_dir())
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
        let read = async {
            loop {
                let m = codec::read(&mut *rx).await.unwrap().expect("proxy closed");
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
