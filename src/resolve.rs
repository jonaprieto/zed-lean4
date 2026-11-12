use std::path::PathBuf;
use zed_extension_api::serde_json::Value;
use zed_extension_api::{Architecture, Os};

pub const PROXY_REPO: &str = "jonaprieto/zed-lean4";

/// `lake` arguments for `serve`, honouring `settings.lean_project_dir`.
pub fn lake_server_args(settings: Option<&Value>, worktree_root: &str) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(dir) = settings
        .and_then(|s| s.get("lean_project_dir"))
        .and_then(Value::as_str)
    {
        let dir = PathBuf::from(dir);
        let dir = if dir.is_absolute() {
            dir
        } else {
            PathBuf::from(worktree_root).join(dir)
        };
        args.extend(["--dir".into(), dir.to_string_lossy().into_owned()]);
    }
    args.extend(["serve".into(), "--".into()]);
    args
}

/// Release asset for this platform; names match `.github/workflows/release.yml`.
pub fn proxy_asset_name(os: Os, arch: Architecture) -> Result<String, String> {
    let arch = match arch {
        Architecture::Aarch64 => "aarch64",
        Architecture::X8664 => "x86_64",
        Architecture::X86 => return Err("32-bit x86 is not supported".into()),
    };
    let (triple, ext) = match os {
        Os::Mac => ("apple-darwin", "tar.gz"),
        Os::Linux => ("unknown-linux-gnu", "tar.gz"),
        Os::Windows => ("pc-windows-msvc", "zip"),
    };
    Ok(format!("lean4-ide-proxy-{arch}-{triple}.{ext}"))
}

/// Directory holding the proxy downloaded for release `tag`.
pub fn proxy_dir(tag: &str) -> String {
    format!("lean4-ide-proxy-{tag}")
}

/// Proxy binary inside `dir`.
pub fn proxy_bin(dir: &str, windows: bool) -> String {
    format!("{dir}/lean4-ide-proxy{}", if windows { ".exe" } else { "" })
}

/// Runs lake behind the proxy when one is available.
pub fn server_command(
    lake: String,
    lake_args: Vec<String>,
    proxy: Option<String>,
) -> (String, Vec<String>) {
    match proxy {
        Some(proxy) => (
            proxy,
            ["--".to_string(), lake]
                .into_iter()
                .chain(lake_args)
                .collect(),
        ),
        None => (lake, lake_args),
    }
}

/// `lsp.lean4-ide.settings.proxy = false` runs `lake serve` directly.
pub fn proxy_enabled(settings: Option<&Value>) -> bool {
    settings
        .and_then(|s| s.get("proxy"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zed_extension_api::serde_json::json;

    #[test]
    fn serve_without_project_dir() {
        assert_eq!(lake_server_args(None, "/w"), ["serve", "--"]);
    }

    #[test]
    fn relative_project_dir_joins_worktree_root() {
        let s = json!({"lean_project_dir": "formal"});
        assert_eq!(
            lake_server_args(Some(&s), "/w"),
            ["--dir", "/w/formal", "serve", "--"]
        );
    }

    #[test]
    fn absolute_project_dir_kept() {
        let s = json!({"lean_project_dir": "/abs/p"});
        assert_eq!(
            lake_server_args(Some(&s), "/w"),
            ["--dir", "/abs/p", "serve", "--"]
        );
    }

    #[test]
    fn asset_names_match_release_workflow() {
        assert_eq!(
            proxy_asset_name(Os::Mac, Architecture::Aarch64).unwrap(),
            "lean4-ide-proxy-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            proxy_asset_name(Os::Linux, Architecture::X8664).unwrap(),
            "lean4-ide-proxy-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            proxy_asset_name(Os::Windows, Architecture::X8664).unwrap(),
            "lean4-ide-proxy-x86_64-pc-windows-msvc.zip"
        );
        assert!(proxy_asset_name(Os::Linux, Architecture::X86).is_err());
    }

    #[test]
    fn proxy_wraps_lake_command() {
        let (cmd, args) = server_command(
            "/e/lake".into(),
            vec!["serve".into(), "--".into()],
            Some("/p/lean4-ide-proxy".into()),
        );
        assert_eq!(cmd, "/p/lean4-ide-proxy");
        assert_eq!(args, ["--", "/e/lake", "serve", "--"]);
    }

    #[test]
    fn without_proxy_lake_runs_directly() {
        let (cmd, args) = server_command("/e/lake".into(), vec!["serve".into(), "--".into()], None);
        assert_eq!(
            (cmd.as_str(), args),
            ("/e/lake", vec!["serve".to_string(), "--".to_string()])
        );
    }

    #[test]
    fn proxy_paths_follow_tag() {
        assert_eq!(proxy_dir("v0.4.0"), "lean4-ide-proxy-v0.4.0");
        assert_eq!(
            proxy_bin("lean4-ide-proxy-v0.4.0", false),
            "lean4-ide-proxy-v0.4.0/lean4-ide-proxy"
        );
        assert_eq!(
            proxy_bin("lean4-ide-proxy-v0.4.0", true),
            "lean4-ide-proxy-v0.4.0/lean4-ide-proxy.exe"
        );
    }

    #[test]
    fn proxy_on_by_default() {
        assert!(proxy_enabled(None));
        assert!(!proxy_enabled(Some(&json!({"proxy": false}))));
    }
}
