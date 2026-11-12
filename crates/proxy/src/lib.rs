pub mod bridge;
pub mod codec;
pub mod cursor;
pub mod hover;
pub mod init;
pub mod msg;
pub mod router;
mod run;

pub use run::run;

/// The server command is everything after the first `--`.
pub fn server_command(args: impl Iterator<Item = String>) -> anyhow::Result<Vec<String>> {
    let cmd: Vec<String> = args.skip_while(|a| a != "--").skip(1).collect();
    anyhow::ensure!(
        !cmd.is_empty(),
        "usage: lean4-ide-proxy -- <server command>..."
    );
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    #[test]
    fn server_command_keeps_later_double_dashes() {
        let args = ["proxy", "--", "/bin/lake", "serve", "--"]
            .map(String::from)
            .into_iter();
        assert_eq!(
            super::server_command(args).unwrap(),
            ["/bin/lake", "serve", "--"]
        );
    }

    #[test]
    fn server_command_requires_a_command() {
        assert!(super::server_command(["proxy".to_string()].into_iter()).is_err());
    }
}
