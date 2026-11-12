//! The tokio shell around `Router`: two reader tasks feed one loop that writes.
use crate::codec;
use crate::router::{Action, Event, Router};
use anyhow::Result;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

enum Input {
    Msg(Event),
    Closed(&'static str),
}

/// Proxies until either side closes its stream.
pub async fn run<CI, CO, SI, SO>(
    client_in: CI,
    mut client_out: CO,
    server_in: SI,
    mut server_out: SO,
) -> Result<()>
where
    CI: AsyncRead + Unpin + Send + 'static,
    CO: AsyncWrite + Unpin + Send + 'static,
    SI: AsyncRead + Unpin + Send + 'static,
    SO: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = unbounded_channel();
    spawn_reader(client_in, tx.clone(), Event::FromClient, "client");
    spawn_reader(server_in, tx, Event::FromServer, "server");
    let mut router = Router::default();
    while let Some(input) = rx.recv().await {
        let event = match input {
            Input::Msg(event) => event,
            Input::Closed(side) => {
                tracing::info!("{side} closed its stream");
                break;
            }
        };
        for action in router.handle(event) {
            match action {
                Action::ToClient(m) => codec::write(&mut client_out, &m).await?,
                Action::ToServer(m) => codec::write(&mut server_out, &m).await?,
            }
        }
    }
    Ok(())
}

fn spawn_reader<R: AsyncRead + Unpin + Send + 'static>(
    r: R,
    tx: UnboundedSender<Input>,
    wrap: fn(Value) -> Event,
    side: &'static str,
) {
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
    });
}
