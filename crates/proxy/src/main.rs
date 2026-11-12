use anyhow::{Context, Result};
use std::process::Stdio;
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "current_thread")]
async fn main() {
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
    lean4_ide_proxy::run(
        tokio::io::stdin(),
        tokio::io::stdout(),
        server_in,
        server_out,
    )
    .await?;
    // Dropping our end of the server's stdin (inside run) makes `lake serve` exit.
    let status = child.wait().await?;
    let code = status
        .code()
        .and_then(|c| u8::try_from(c).ok())
        .unwrap_or(1);
    Ok(code.into())
}
