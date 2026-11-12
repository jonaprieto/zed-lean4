//! I/O shell for the terminal infoview: discovery, the WebSocket client, keys and redraws.
pub mod app;
mod detail;
pub mod goals;
mod hover;
pub mod lean;
pub mod messages;
mod rpc;
mod screen;
pub mod view;

use crate::discover;
use anyhow::Context;
use app::{App, Effect};
use futures_util::{SinkExt, StreamExt};
use ratatui::{
    DefaultTerminal,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
        },
        execute,
    },
};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    net::TcpStream,
    sync::mpsc::{UnboundedReceiver, unbounded_channel},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest, handshake::client::Request},
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const WAITING: &str = "Waiting for the Lean server...";

/// The socket request for a page URL `http://H/?token=T`: `ws://H/ws?token=T` with `Origin: http://H`.
pub fn socket_request(page_url: &str) -> anyhow::Result<Request> {
    let (origin, query) = page_url
        .split_once("/?")
        .context("infoview URL has no query")?;
    let host = origin
        .strip_prefix("http://127.0.0.1:")
        .context("infoview URL is not http://127.0.0.1")?;
    let host = format!("127.0.0.1:{host}");
    let mut request = format!("ws://{host}/ws?{query}").into_client_request()?;
    request.headers_mut().insert("Origin", origin.parse()?);
    Ok(request)
}

pub fn open_browser(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Turns on mouse reports, and makes sure a panic turns them off again before
/// ratatui's own hook restores the terminal.
fn capture_mouse() -> std::io::Result<()> {
    let restore = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        restore(info);
    }));
    execute!(std::io::stdout(), EnableMouseCapture)
}

/// Runs the terminal infoview; returns the process exit code.
pub async fn main() -> i32 {
    let result = match ratatui::try_init() {
        Ok(mut terminal) => match capture_mouse() {
            Ok(()) => shell(&mut terminal).await,
            Err(e) => Err(e.into()),
        },
        Err(e) => Err(e.into()),
    };
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("lean4-ide-proxy tui: {e:#}");
            1
        }
    }
}

enum Outcome {
    Quit,
    Closed,
}

async fn shell(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    let (tx, mut keys) = unbounded_channel();
    std::thread::spawn(move || {
        loop {
            match event::read() {
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Press => {}
                // Capture reports every motion; only clicks and the wheel matter.
                Ok(Event::Mouse(m))
                    if !matches!(
                        m.kind,
                        MouseEventKind::Down(_)
                            | MouseEventKind::ScrollDown
                            | MouseEventKind::ScrollUp
                    ) => {}
                Ok(e @ (Event::Key(_) | Event::Mouse(_) | Event::Resize(..))) => {
                    if tx.send(e).is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });
    let cwd = std::env::current_dir()?.canonicalize()?;
    let mut app = App::new(format!("tui-{}", std::process::id()));
    loop {
        match connect(&cwd).await {
            Ok((url, mut socket)) => {
                let effects = app.connected(url);
                if let Outcome::Quit =
                    session(terminal, &mut app, &mut socket, effects, &mut keys).await?
                {
                    return Ok(());
                }
                app.disconnected(WAITING.into());
            }
            Err(status) => app.disconnected(status),
        }
        terminal.draw(|f| app.render(f))?;
        let retry = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(retry);
        loop {
            tokio::select! {
                _ = &mut retry => break,
                input = keys.recv() => {
                    let effects = key(&mut app, input);
                    if apply(&mut app, effects, None).await.is_some() {
                        return Ok(());
                    }
                    terminal.draw(|f| app.render(f))?;
                }
            }
        }
    }
}

async fn connect(cwd: &Path) -> Result<(String, Socket), String> {
    let dir = discover::dir().ok_or("no Lean server found; open a Lean file in Zed")?;
    let (file, url) = discover::find(&dir, cwd)?;
    let request = socket_request(&url).map_err(|e| format!("{e:#}"))?;
    match connect_async(request).await {
        Ok((socket, _)) => Ok((url, socket)),
        Err(tokio_tungstenite::tungstenite::Error::Io(e))
            if e.kind() == std::io::ErrorKind::ConnectionRefused =>
        {
            // Nothing listens any more: the proxy died without removing its file.
            let _ = std::fs::remove_file(file);
            Err(WAITING.into())
        }
        Err(_) => Err(WAITING.into()),
    }
}

async fn session(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    socket: &mut Socket,
    mut effects: Vec<Effect>,
    keys: &mut UnboundedReceiver<Event>,
) -> anyhow::Result<Outcome> {
    loop {
        if let Some(outcome) = apply(app, effects, Some(socket)).await {
            return Ok(outcome);
        }
        terminal.draw(|f| app.render(f))?;
        effects = tokio::select! {
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => match crate::codec::parse(text.as_bytes()) {
                    Ok(m) => app.on_message(&m),
                    Err(_) => vec![],
                },
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Ok(Outcome::Closed),
                Some(Ok(_)) => vec![],
            },
            input = keys.recv() => key(app, input),
        };
    }
}

/// Keys and the mouse go to the app; a resize only redraws; lost input quits.
fn key(app: &mut App, input: Option<Event>) -> Vec<Effect> {
    match input {
        Some(Event::Key(key)) => app.on_key(key),
        Some(Event::Mouse(mouse)) => app.on_mouse(mouse),
        Some(_) => vec![],
        None => vec![Effect::Quit],
    }
}

/// Applies effects and says whether the session ends; sends are dropped while disconnected.
async fn apply(
    app: &mut App,
    effects: Vec<Effect>,
    mut socket: Option<&mut Socket>,
) -> Option<Outcome> {
    for effect in effects {
        match effect {
            Effect::Send(m) => {
                if let Some(socket) = socket.as_deref_mut()
                    && socket.send(Message::text(m.to_string())).await.is_err()
                {
                    return Some(Outcome::Closed);
                }
            }
            Effect::OpenBrowser => {
                if let Some(Err(e)) = app.page_url().map(open_browser) {
                    app.set_status(format!("Could not open the browser: {e}"));
                }
            }
            Effect::Quit => return Some(Outcome::Quit),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn socket_request_keeps_token_and_sets_origin() {
        let request = super::socket_request("http://127.0.0.1:9/?token=ab").unwrap();
        assert_eq!(request.uri(), "ws://127.0.0.1:9/ws?token=ab");
        assert_eq!(request.headers()["Origin"], "http://127.0.0.1:9");
        assert!(super::socket_request("http://127.0.0.1:9/").is_err());
        assert!(super::socket_request("http://example.com:9/?token=ab").is_err());
    }
}
