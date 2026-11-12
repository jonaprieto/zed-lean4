//! Per-process discovery file so `lean4-ide-proxy tui` can find the infoview URL.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::{fs, io};

pub fn dir() -> Option<PathBuf> {
    let var = |k| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|d| d.join("lean4-ide").join("run"));
    }
    var("XDG_RUNTIME_DIR")
        .map(|d| d.join("lean4-ide"))
        .or_else(|| var("HOME").map(|h| h.join(".cache/lean4-ide/run")))
}

pub fn publish(dir: &Path, root: &Path, url: &str) -> io::Result<PathBuf> {
    use io::Write;
    fs::create_dir_all(dir)?;
    if fs::symlink_metadata(dir)?.file_type().is_symlink() {
        return Err(io::Error::other("discovery directory is a symlink"));
    }
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        opts.mode(0o600);
    }
    // ponytail: no ACLs on Windows; %LOCALAPPDATA% is per-user.
    let pid = std::process::id();
    let tmp = dir.join(format!("{pid}.json.tmp"));
    let path = dir.join(format!("{pid}.json"));
    // A stale tmp file would keep its old mode; start fresh.
    let _ = fs::remove_file(&tmp);
    let body = json!({ "root": root, "url": url }).to_string();
    opts.open(&tmp)?.write_all(body.as_bytes())?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// The discovery file and infoview URL of the server that serves `cwd`.
pub fn find(dir: &Path, cwd: &Path) -> Result<(PathBuf, String), String> {
    let mut entries: Vec<(PathBuf, String, SystemTime, PathBuf)> = Vec::new();
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let (Some(root), Some(url)) = (
            v.get("root").and_then(Value::as_str),
            v.get("url").and_then(Value::as_str),
        ) else {
            continue;
        };
        let mtime = e
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        entries.push((PathBuf::from(root), url.to_string(), mtime, p));
    }
    let best = entries
        .iter()
        .filter(|(root, ..)| cwd.starts_with(root))
        .max_by_key(|(root, _, mtime, _)| (root.components().count(), *mtime));
    match (best, entries.as_slice()) {
        (Some((_, url, _, file)), _) | (None, [(_, url, _, file)]) => {
            Ok((file.clone(), url.clone()))
        }
        (None, []) => Err("no Lean server found; open a Lean file in Zed".into()),
        (None, many) => {
            let mut roots: Vec<_> = many.iter().map(|e| e.0.display().to_string()).collect();
            roots.sort();
            Err(format!(
                "several Lean servers are running ({}); run from a project root",
                roots.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lean4-ide-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &Path, file: &str, root: &str, url: &str) {
        fs::write(
            dir.join(file),
            json!({"root": root, "url": url}).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn publish_writes_root_and_url_privately() {
        let d = tmp("publish").join("run");
        let p = publish(&d, Path::new("/p"), "http://127.0.0.1:1/?token=ab").unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"root": "/p", "url": "http://127.0.0.1:1/?token=ab"})
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&d), 0o700);
            assert_eq!(mode(&p), 0o600);
        }
        assert!(
            fs::read_dir(&d)
                .unwrap()
                .all(|e| e.unwrap().path().extension().unwrap() != "tmp")
        );
        fs::remove_dir_all(d.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn publish_refuses_a_symlinked_directory() {
        let base = tmp("symlink");
        fs::create_dir(base.join("real")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
        assert!(publish(&base.join("link"), Path::new("/p"), "u").is_err());
        assert!(fs::read_dir(base.join("real")).unwrap().next().is_none());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn find_prefers_deepest_ancestor_root() {
        let d = tmp("deepest");
        write(&d, "a.json", "/p", "A");
        write(&d, "b.json", "/p/sub", "B");
        write(&d, "c.json", "/q", "C");
        assert_eq!(
            find(&d, Path::new("/p/sub/x")).unwrap(),
            (d.join("b.json"), "B".into())
        );
        assert_eq!(
            find(&d, Path::new("/p/y")).unwrap(),
            (d.join("a.json"), "A".into())
        );
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn find_falls_back_to_single_server() {
        let d = tmp("single");
        write(&d, "c.json", "/q", "C");
        assert_eq!(
            find(&d, Path::new("/p")).unwrap(),
            (d.join("c.json"), "C".into())
        );
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn find_reports_none_and_ambiguity() {
        let d = tmp("errors");
        fs::write(d.join("bad.json"), "{nope").unwrap();
        assert_eq!(
            find(&d, Path::new("/p")).unwrap_err(),
            "no Lean server found; open a Lean file in Zed"
        );
        write(&d, "q.json", "/q", "Q");
        write(&d, "r.json", "/r", "R");
        let e = find(&d, Path::new("/p")).unwrap_err();
        assert!(e.contains("/q") && e.contains("/r") && e.contains("run from a project root"));
        fs::remove_dir_all(d).unwrap();
    }
}
