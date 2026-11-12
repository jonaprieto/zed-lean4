use futures_util::{SinkExt, StreamExt};
use lean4_ide_proxy::tui::app::{App, Effect};
use lean4_ide_proxy::{codec, run, server::Bridge, tui::socket_request};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader, duplex, split};
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buf = terminal.backend().buffer();
    (0..20)
        .map(|y| {
            let row: String = (0..60).map(|x| buf[(x, y)].symbol()).collect();
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn tui_shows_the_goal_at_the_zed_cursor() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let bridge = Bridge::bind().await.unwrap();
        let page_url = bridge.page_url.clone();
        let (zed, pc) = duplex(1 << 16);
        let (lean, ps) = duplex(1 << 16);
        let (pc_r, pc_w) = split(pc);
        let (ps_r, ps_w) = split(ps);
        let proxy = tokio::spawn(run(pc_r, pc_w, ps_r, ps_w, Some(bridge)));
        let (zr, mut zw) = split(zed);
        let (lr, mut lw) = split(lean);
        let mut zr = BufReader::new(zr);

        // Fake Lean: answers initialize, the two goal requests, and anything else with null.
        let fake_lean = tokio::spawn(async move {
            let mut lr = BufReader::new(lr);
            while let Some(m) = codec::read(&mut lr).await.unwrap() {
                let Some(id) = m.get("id").cloned() else {
                    continue;
                };
                let result = match m["method"].as_str() {
                    Some("initialize") => json!({"capabilities":{}}),
                    Some("$/lean/plainGoal") => {
                        assert_eq!(m["params"]["position"], json!({"line":1,"character":2}));
                        json!({"rendered":"..","goals":["n : Nat\n⊢ n + 0 = n"]})
                    }
                    _ => Value::Null,
                };
                codec::write(&mut lw, &json!({"id":id,"result":result}))
                    .await
                    .unwrap();
            }
        });

        codec::write(
            &mut zw,
            &json!({"id":1,"method":"initialize","params":{}}),
        )
        .await
        .unwrap();
        let init = codec::read(&mut zr).await.unwrap().unwrap();
        assert_eq!(init["id"], 1);
        // Drain whatever else the proxy sends to Zed.
        let drain = tokio::spawn(async move { while let Ok(Some(_)) = codec::read(&mut zr).await {} });

        let (mut socket, _) = connect_async(socket_request(&page_url).unwrap())
            .await
            .unwrap();
        let mut app = App::new(format!("tui-{}", std::process::id()));
        let hello_id = format!("tui-{}:1", std::process::id());
        let mut effects = app.connected(page_url.clone());
        let mut highlighted = false;
        loop {
            for effect in effects.drain(..) {
                let Effect::Send(m) = effect else {
                    panic!("unexpected effect {effect:?}")
                };
                socket.send(Message::text(m.to_string())).await.unwrap();
            }
            if screen(&app).contains("⊢ n + 0 = n") {
                break;
            }
            let text = socket.next().await.unwrap().unwrap();
            let m: Value = serde_json::from_str(text.to_text().unwrap()).unwrap();
            effects = app.on_message(&m);
            if m["id"] == hello_id.as_str() {
                assert!(m["result"]["initializeResult"].is_object());
                assert!(!highlighted);
                highlighted = true;
                codec::write(
                    &mut zw,
                    &json!({"id":2,"method":"textDocument/documentHighlight","params":{"textDocument":{"uri":"file:///F.lean"},"position":{"line":1,"character":2}}}),
                )
                .await
                .unwrap();
            }
        }
        assert!(screen(&app).contains("F.lean:2:2"));
        zw.shutdown().await.unwrap();
        proxy.await.unwrap().unwrap();
        drain.abort();
        fake_lean.abort();
    })
    .await
    .expect("tui socket test timed out");
}
