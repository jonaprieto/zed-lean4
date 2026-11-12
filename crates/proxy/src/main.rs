use anyhow::{Context, Result};
use std::process::Stdio;
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // No tracing for the TUI: log lines would corrupt the drawn screen.
    if std::env::args().nth(1).as_deref() == Some("tui") {
        std::process::exit(lean4_ide_proxy::tui::main().await)
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_env("LEAN4_IDE_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let code = match serve().await {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("{e:#}");
            1
        }
    };
    // Exit explicitly: dropping the runtime would block on the blocking stdin read.
    std::process::exit(code)
}

/// Runs the proxy and returns the exit code of the Lean server.
async fn serve() -> Result<i32> {
    let cmd = lean4_ide_proxy::server_command(std::env::args())?;
    let mut child = tokio::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting {cmd:?}"))?;
    let server_in = child.stdout.take().context("server stdout")?;
    let server_out = child.stdin.take().context("server stdin")?;
    let bridge = match lean4_ide_proxy::server::Bridge::bind().await {
        Ok(bridge) => Some(bridge),
        Err(error) => {
            tracing::warn!("infoview unavailable: {error:#}");
            None
        }
    };
    let discovery = bridge.as_ref().and_then(|b| publish_discovery(&b.page_url));
    let result = lean4_ide_proxy::run(
        tokio::io::stdin(),
        tokio::io::stdout(),
        server_in,
        server_out,
        bridge,
    )
    .await;
    if let Some(path) = discovery {
        let _ = std::fs::remove_file(path);
    }
    result?;
    // Dropping our end of the server's stdin (inside run) makes `lake serve` exit.
    let status = child.wait().await?;
    let code = status
        .code()
        .and_then(|c| u8::try_from(c).ok())
        .unwrap_or(1);
    Ok(code.into())
}

fn publish_discovery(page_url: &str) -> Option<std::path::PathBuf> {
    let linked = lean4_ide_proxy::discover::launcher()
        .ok_or_else(|| std::io::Error::other("no home directory"))
        .and_then(|link| {
            let exe = std::env::current_exe()?.canonicalize()?;
            lean4_ide_proxy::discover::link_launcher(&link, &exe)
        });
    if let Err(e) = linked {
        tracing::warn!("terminal infoview launcher unavailable: {e}");
    }
    let dir = lean4_ide_proxy::discover::dir()?;
    let published = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .and_then(|root| lean4_ide_proxy::discover::publish(&dir, &root, page_url));
    match published {
        Ok(path) => {
            tracing::info!("infoview discovery file {path:?}");
            Some(path)
        }
        Err(e) => {
            tracing::warn!("infoview discovery unavailable: {e}");
            None
        }
    }
}
