use futures_util::{SinkExt, StreamExt};
use lean4_ide_proxy::{codec, run, server::Bridge};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader, duplex, split};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error, Message, client::IntoClientRequest},
};

#[tokio::test]
async fn socket_routes_both_directions_and_closes_with_lsp() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let bridge = Bridge::bind().await.unwrap();
        let (origin, query) = bridge.page_url.split_once("/?").unwrap();
        let url = format!("{}/ws?{}", origin.replacen("http:", "ws:", 1), query);
        for (url, origin_header) in [
            (url.clone(), None),
            (url.clone(), Some("http://evil.test")),
            (
                format!("{}/ws?token=bad", origin.replacen("http:", "ws:", 1)),
                Some(origin),
            ),
        ] {
            let mut request = url.into_client_request().unwrap();
            if let Some(origin) = origin_header {
                request
                    .headers_mut()
                    .insert("Origin", origin.parse().unwrap());
            }
            let Err(Error::Http(response)) = connect_async(request).await else {
                panic!("upgrade should be forbidden")
            };
            assert_eq!(response.status(), 403);
        }
        let mut request = url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        let (mut socket, _) = connect_async(request).await.unwrap();
        let (zed, pc) = duplex(1 << 16);
        let (lean, ps) = duplex(1 << 16);
        let (pc_r, pc_w) = split(pc);
        let (ps_r, ps_w) = split(ps);
        let proxy = tokio::spawn(run(pc_r, pc_w, ps_r, ps_w, Some(bridge)));
        let (zr, mut zw) = split(zed);
        let (lr, mut lw) = split(lean);
        let mut zr = BufReader::new(zr);
        let mut lr = BufReader::new(lr);
        socket
            .send(Message::text(
                json!({"id":"a:1","call":"createRpcSession","args":["file:///F.lean"]}).to_string(),
            ))
            .await
            .unwrap();
        let req = codec::read(&mut lr).await.unwrap().unwrap();
        assert_eq!(req["method"], "$/lean/rpc/connect");
        codec::write(
            &mut lw,
            &json!({"id":req["id"],"result":{"sessionId":"opaque"}}),
        )
        .await
        .unwrap();
        let reply: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(reply, json!({"id":"a:1","result":"opaque"}));
        socket
            .send(Message::text(
                json!({"id":"a:2","call":"applyEdit","args":[{"changes":{}}]}).to_string(),
            ))
            .await
            .unwrap();
        let req = codec::read(&mut zr).await.unwrap().unwrap();
        assert_eq!(req["method"], "workspace/applyEdit");
        codec::write(&mut zw, &json!({"id":req["id"],"result":{"applied":true}}))
            .await
            .unwrap();
        let reply: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(reply, json!({"id":"a:2","result":{"applied":true}}));
        zw.shutdown().await.unwrap();
        proxy.await.unwrap().unwrap();
        assert!(matches!(
            socket.next().await,
            None | Some(Ok(Message::Close(_))) | Some(Err(_))
        ));
    })
    .await
    .expect("socket test timed out");
}

#[tokio::test]
async fn socket_connections_have_distinct_ids_and_report_closure() {
    use lean4_ide_proxy::router::Event;

    tokio::time::timeout(Duration::from_secs(5), async {
        let mut bridge = Bridge::bind().await.unwrap();
        let (origin, query) = bridge.page_url.split_once("/?").unwrap();
        let url = format!("{}/ws?{query}", origin.replacen("http:", "ws:", 1));
        let mut request = url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        let (mut first, _) = connect_async(request.clone()).await.unwrap();
        let (mut second, _) = connect_async(request).await.unwrap();
        let call = json!({"id":"a:1","call":"hello","args":[]});
        first.send(Message::text(call.to_string())).await.unwrap();
        let Some(Event::FromBrowser(first_id, message)) = bridge.incoming.recv().await else {
            panic!("expected first page call")
        };
        assert_eq!(message, call);
        second.send(Message::text(call.to_string())).await.unwrap();
        let Some(Event::FromBrowser(second_id, message)) = bridge.incoming.recv().await else {
            panic!("expected second page call")
        };
        assert_eq!(message, call);
        assert_ne!(first_id, second_id);
        let reply = json!({"id":"a:1","result":null});
        bridge.outgoing.send(reply.clone()).unwrap();
        for socket in [&mut first, &mut second] {
            let message = socket.next().await.unwrap().unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(message.to_text().unwrap()).unwrap(),
                reply
            );
        }
        first.close(None).await.unwrap();
        let closed = bridge.incoming.recv().await;
        assert!(matches!(closed, Some(Event::PageClosed(id)) if id == first_id));
        second.send(Message::text(call.to_string())).await.unwrap();
        let received = bridge.incoming.recv().await;
        assert!(matches!(received, Some(Event::FromBrowser(id, _)) if id == second_id));
        second.close(None).await.unwrap();
        let closed = bridge.incoming.recv().await;
        assert!(matches!(closed, Some(Event::PageClosed(id)) if id == second_id));
    })
    .await
    .expect("connection test timed out");
}
