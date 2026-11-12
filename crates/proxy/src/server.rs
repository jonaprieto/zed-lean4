use crate::router::Event;
use axum::{
    Router,
    extract::{
        RawQuery, State,
        ws::{Message, WebSocket, WebSocketUpgrade, rejection::WebSocketUpgradeRejection},
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rust_embed::Embed;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

#[derive(Embed)]
#[folder = "../../web/public/"]
struct Assets;

#[derive(Clone)]
struct Shared {
    token: String,
    origin: String,
    incoming: mpsc::UnboundedSender<Event>,
    next_connection: Arc<AtomicU64>,
    outgoing: broadcast::Sender<Value>,
    shutdown: watch::Receiver<bool>,
}

pub struct Bridge {
    pub page_url: String,
    pub incoming: mpsc::UnboundedReceiver<Event>,
    pub outgoing: broadcast::Sender<Value>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Bridge {
    pub async fn bind() -> anyhow::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let origin = format!("http://{addr}");
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("infoview token: {e}"))?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let page_url = format!("{origin}/?token={token}");
        let (incoming_tx, incoming) = mpsc::unbounded_channel();
        let (outgoing, _) = broadcast::channel(4096);
        let (shutdown, stopped) = watch::channel(false);
        let shared = Shared {
            token,
            origin,
            incoming: incoming_tx,
            next_connection: Arc::new(AtomicU64::new(1)),
            outgoing: outgoing.clone(),
            shutdown: stopped,
        };
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app(shared)).await {
                tracing::warn!("infoview HTTP server stopped: {error}");
            }
        });
        Ok(Self {
            page_url,
            incoming,
            outgoing,
            shutdown,
            task,
        })
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}

fn app(shared: Shared) -> Router {
    Router::new()
        .route("/ws", get(upgrade))
        .fallback(get(asset))
        .with_state(shared)
}

fn token_ok(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in actual.bytes().zip(expected.bytes()) {
        diff |= std::hint::black_box(a ^ b);
    }
    diff == 0
}

fn query_token(query: Option<&str>) -> Option<&str> {
    let mut tokens = query?
        .split('&')
        .filter_map(|pair| pair.strip_prefix("token="));
    let first = tokens.next()?;
    tokens.next().is_none().then_some(first)
}

fn host_ok(headers: &HeaderMap, shared: &Shared) -> bool {
    headers.get(header::HOST).and_then(|v| v.to_str().ok()) == shared.origin.strip_prefix("http://")
}

async fn upgrade(
    State(shared): State<Shared>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    if !host_ok(&headers, &shared)
        || headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(shared.origin.as_str())
        || !query_token(query.as_deref()).is_some_and(|token| token_ok(token, &shared.token))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    match ws {
        Ok(ws) => ws.on_upgrade(move |socket| socket_loop(socket, shared)),
        Err(error) => error.into_response(),
    }
}

async fn asset(State(shared): State<Shared>, headers: HeaderMap, uri: Uri) -> Response {
    if !host_ok(&headers, &shared) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = if uri.path() == "/" {
        "index.html"
    } else {
        uri.path().trim_start_matches('/')
    };
    if path.split('/').any(|part| part == "..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(file) = Assets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        file.data,
    )
        .into_response()
}

async fn socket_loop(mut socket: WebSocket, mut shared: Shared) {
    let connection = shared.next_connection.fetch_add(1, Ordering::Relaxed);
    let mut outgoing = shared.outgoing.subscribe();
    loop {
        tokio::select! {
            _ = shared.shutdown.changed() => break,
            received = socket.recv() => match received {
                Some(Ok(Message::Text(text))) => {
                    let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else { break };
                    if shared.incoming.send(Event::FromBrowser(connection, message)).is_err() { break; }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break,
            },
            message = outgoing.recv() => match message {
                Ok(message) => {
                    if socket.send(Message::text(message.to_string())).await.is_err() { break; }
                }
                Err(_) => break,
            },
        }
    }
    let _ = shared.incoming.send(Event::PageClosed(connection));
    let _ = socket.send(Message::Close(None)).await;
}

#[cfg(test)]
mod server_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn shared() -> Shared {
        let (incoming, _) = mpsc::unbounded_channel();
        let (outgoing, _) = broadcast::channel(8);
        let (_, shutdown) = watch::channel(false);
        Shared {
            token: "0123456789abcdef0123456789abcdef".into(),
            origin: "http://127.0.0.1:1234".into(),
            incoming,
            next_connection: Arc::new(AtomicU64::new(1)),
            outgoing,
            shutdown,
        }
    }

    #[test]
    fn token_comparison_checks_all_bytes_and_length() {
        let token = "0123456789abcdef0123456789abcdef";
        assert!(token_ok(token, token));
        assert!(!token_ok("", token));
        for i in 0..token.len() {
            let mut wrong = token.as_bytes().to_vec();
            wrong[i] = b'x';
            assert!(!token_ok(std::str::from_utf8(&wrong).unwrap(), token));
        }
    }

    #[tokio::test]
    async fn assets_check_host_and_socket_checks_origin() {
        let state = shared();
        let path = "/vendor/infoview/index.css";
        for (host, status) in [
            ("127.0.0.1:1234", StatusCode::OK),
            ("evil.test:1234", StatusCode::FORBIDDEN),
        ] {
            let req = Request::builder()
                .uri(path)
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap();
            let response = app(state.clone()).oneshot(req).await.unwrap();
            assert_eq!(response.status(), status);
            if status == StatusCode::OK {
                assert_eq!(
                    response.headers()[header::CONTENT_TYPE],
                    "text/css; charset=utf-8"
                );
            }
        }
        let req = Request::builder()
            .uri("/ws?token=0123456789abcdef0123456789abcdef")
            .header(header::HOST, "127.0.0.1:1234")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app(state).oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}
