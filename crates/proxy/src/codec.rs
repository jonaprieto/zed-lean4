//! LSP base protocol framing: `Content-Length` header, blank line, JSON body.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Parses a JSON message without serde_json's 128-level nesting limit: Lean's
/// interactive goals nest a few levels per subterm and easily exceed it. The
/// server is trusted, so depth is bounded only by the thread's stack.
pub fn parse(body: &[u8]) -> Result<Value> {
    let mut de = serde_json::Deserializer::from_slice(body);
    de.disable_recursion_limit();
    let mut values = de.into_iter::<Value>();
    let value = values.next().context("empty JSON body")??;
    if values.next().is_some() {
        bail!("trailing data after the JSON body");
    }
    Ok(value)
}

/// Reads one message. `Ok(None)` means the stream ended cleanly between messages.
pub async fn read<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<Option<Value>> {
    let mut len = None;
    let mut line = String::new();
    let mut started = false;
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            if started {
                bail!("stream ended inside an LSP header");
            }
            return Ok(None);
        }
        started = true;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = Some(v.trim().parse::<usize>().context("bad Content-Length")?);
        }
    }
    let Some(len) = len else {
        bail!("LSP header without Content-Length")
    };
    let mut body = vec![0; len];
    r.read_exact(&mut body).await?;
    Ok(Some(parse(&body).context("LSP body is not JSON")?))
}

/// Writes one message and flushes.
pub async fn write<W: AsyncWrite + Unpin>(w: &mut W, m: &Value) -> Result<()> {
    let body = serde_json::to_vec(m)?;
    w.write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn framed(m: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        write(&mut out, m).await.unwrap();
        out
    }

    #[test]
    fn parses_nesting_deeper_than_serde_json_allows() {
        let depth = 1000;
        let deep = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(serde_json::from_str::<Value>(&deep).is_err());
        assert!(parse(deep.as_bytes()).is_ok());
        assert!(parse(b"{} trailing").is_err());
    }

    #[tokio::test]
    async fn round_trips_two_messages() {
        let a = json!({"jsonrpc": "2.0", "id": 1, "result": null});
        let b = json!({"jsonrpc": "2.0", "method": "x", "params": {"s": "⊢ n + 0 = n"}});
        let mut bytes = framed(&a).await;
        bytes.extend(framed(&b).await);
        let mut r = bytes.as_slice();
        assert_eq!(read(&mut r).await.unwrap(), Some(a));
        assert_eq!(read(&mut r).await.unwrap(), Some(b));
        assert_eq!(read(&mut r).await.unwrap(), None);
    }

    #[tokio::test]
    async fn content_length_counts_bytes_not_chars() {
        let m = json!({"s": "⊢"});
        let body = serde_json::to_vec(&m).unwrap();
        let bytes = framed(&m).await;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        assert!(bytes.starts_with(header.as_bytes()));
    }

    #[tokio::test]
    async fn header_is_case_insensitive_and_ignores_others() {
        let raw = b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}";
        let mut r = &raw[..];
        assert_eq!(read(&mut r).await.unwrap(), Some(json!({})));
    }

    #[tokio::test]
    async fn eof_inside_header_is_an_error() {
        let mut r = &b"Content-Length: 2\r\n"[..];
        assert!(read(&mut r).await.is_err());
    }

    #[tokio::test]
    async fn missing_content_length_is_an_error() {
        let mut r = &b"X: 1\r\n\r\n{}"[..];
        assert!(read(&mut r).await.is_err());
    }
}
