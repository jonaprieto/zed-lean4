use lean4_ide_proxy::{codec, run};
use serde_json::json;
use tokio::io::{AsyncWriteExt, BufReader, duplex, split};

#[tokio::test]
async fn forwards_both_ways_and_stops_when_zed_closes() {
    let (zed, proxy_client) = duplex(1 << 16);
    let (lean, proxy_server) = duplex(1 << 16);
    let (pc_r, pc_w) = split(proxy_client);
    let (ps_r, ps_w) = split(proxy_server);
    let proxy = tokio::spawn(run(pc_r, pc_w, ps_r, ps_w, None));

    let (zed_r, mut zed_w) = split(zed);
    let (lean_r, mut lean_w) = split(lean);
    let (mut zed_r, mut lean_r) = (BufReader::new(zed_r), BufReader::new(lean_r));

    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "unknown/request", "params": {}});
    codec::write(&mut zed_w, &req).await.unwrap();
    assert_eq!(codec::read(&mut lean_r).await.unwrap(), Some(req));

    let resp = json!({"jsonrpc": "2.0", "id": 1, "result": null});
    codec::write(&mut lean_w, &resp).await.unwrap();
    assert_eq!(codec::read(&mut zed_r).await.unwrap(), Some(resp));

    zed_w.shutdown().await.unwrap();
    proxy.await.unwrap().unwrap();
}
