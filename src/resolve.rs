use std::path::PathBuf;
use zed_extension_api::serde_json::Value;

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
}
