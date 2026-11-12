#![cfg(unix)]
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn exits_with_server_status_while_stdin_open() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lean4-ide-proxy"))
        .args(["--", "sh", "-c", "exit 3"])
        // Keep the launcher link and discovery file out of the real home.
        .env("HOME", std::env::temp_dir())
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let _stdin = child.stdin.take();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(3));
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().ok();
            panic!("proxy did not exit within 5s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn exits_nonzero_when_server_cannot_spawn_while_stdin_open() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lean4-ide-proxy"))
        .args(["--", "/nonexistent/binary"])
        // Keep the launcher link and discovery file out of the real home.
        .env("HOME", std::env::temp_dir())
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _stdin = child.stdin.take();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().ok();
            panic!("proxy did not exit within 5s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
