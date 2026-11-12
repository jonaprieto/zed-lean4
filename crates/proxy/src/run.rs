//! I/O shell: LSP framing, browser transport, and a clock around the sans-IO Router.
use crate::router::{Action, Event, KEEPALIVE, Router};
use crate::{codec, server::Bridge};
use anyhow::Result;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

enum Input {
    Msg(Event),
    Closed(&'static str),
}

pub async fn run<CI, CO, SI, SO>(
    client_in: CI,
    mut client_out: CO,
    server_in: SI,
    mut server_out: SO,
    mut bridge: Option<Bridge>,
) -> Result<()>
where
    CI: AsyncRead + Unpin + Send + 'static,
    CO: AsyncWrite + Unpin + Send + 'static,
    SI: AsyncRead + Unpin + Send + 'static,
    SO: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = unbounded_channel();
    let readers = [
        spawn_reader(client_in, tx.clone(), Event::FromClient, "client"),
        spawn_reader(server_in, tx, Event::FromServer, "server"),
    ];
    let mut router = Router::new(bridge.as_ref().map(|b| b.page_url.clone()));
    let mut clock = tokio::time::interval(KEEPALIVE);
    let result = async {
        loop {
            let input = tokio::select! {
                input = rx.recv() => input,
                page = async {
                    match bridge.as_mut() {
                        Some(bridge) => bridge.incoming.recv().await,
                        None => std::future::pending::<Option<Event>>().await,
                    }
                } => match page {
                    Some(page) => Some(Input::Msg(page)),
                    None => { bridge = None; continue; }
                },
                _ = clock.tick() => Some(Input::Msg(Event::Tick)),
            };
            let event = match input {
                Some(Input::Msg(event)) => event,
                Some(Input::Closed(side)) => {
                    tracing::info!("{side} closed its stream");
                    break;
                }
                None => break,
            };
            for action in router.handle(event) {
                match action {
                    Action::ToClient(m) => codec::write(&mut client_out, &m).await?,
                    Action::ToServer(m) => codec::write(&mut server_out, &m).await?,
                    Action::ToBrowser(m) => {
                        if let Some(bridge) = &bridge {
                            let _ = bridge.outgoing.send(m);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    for reader in readers {
        reader.abort();
    }
    result
}

fn spawn_reader<R: AsyncRead + Unpin + Send + 'static>(
    r: R,
    tx: UnboundedSender<Input>,
    wrap: fn(Value) -> Event,
    side: &'static str,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut r = BufReader::new(r);
        loop {
            match codec::read(&mut r).await {
                Ok(Some(m)) => {
                    if tx.send(Input::Msg(wrap(m))).is_err() {
                        return;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::error!("reading from {side}: {e:#}");
                    break;
                }
            }
        }
        let _ = tx.send(Input::Closed(side));
    })
}
