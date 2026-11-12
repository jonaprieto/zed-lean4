mod resolve;

use std::{env, fs, path::PathBuf};
use zed_extension_api::{self as zed, Result, serde_json::Value, settings::LspSettings};

struct Lean4Extension {
    cached_proxy: Option<String>,
}

impl Lean4Extension {
    /// Proxy binary to run, or None to run lake directly. Failures fall back to lake.
    fn proxy(
        &mut self,
        id: &zed::LanguageServerId,
        settings: Option<&Value>,
        worktree: &zed::Worktree,
    ) -> Option<String> {
        if !resolve::proxy_enabled(settings) {
            return None;
        }
        if let Some(path) = settings
            .and_then(|s| s.get("proxy_path"))
            .and_then(Value::as_str)
        {
            return Some(path.into());
        }
        if let Some(path) = worktree.which("lean4-ide-proxy") {
            return Some(path);
        }
        if let Some(path) = &self.cached_proxy
            && fs::metadata(path).is_ok_and(|m| m.is_file())
        {
            return Some(path.clone());
        }
        match download_proxy(id) {
            Ok(path) => {
                self.cached_proxy = Some(path.clone());
                Some(path)
            }
            Err(e) => {
                eprintln!("lean4-ide: proxy unavailable, running lake directly: {e}");
                None
            }
        }
    }

    fn launch(
        &mut self,
        id: &zed::LanguageServerId,
        settings: Option<&Value>,
        worktree: &zed::Worktree,
        lake: String,
        lake_args: Vec<String>,
    ) -> zed::Command {
        let proxy = self.proxy(id, settings, worktree);
        let (command, args) = resolve::server_command(lake, lake_args, proxy);
        zed::Command {
            command,
            args,
            env: vec![],
        }
    }
}

/// Proxy release matching this extension's version, from disk or GitHub.
fn download_proxy(id: &zed::LanguageServerId) -> Result<String> {
    let (os, arch) = zed::current_platform();
    let windows = matches!(os, zed::Os::Windows);
    let tag = concat!("v", env!("CARGO_PKG_VERSION"));
    let dir = resolve::proxy_dir(tag);
    let bin = resolve::proxy_bin(&dir, windows);
    // Offline restarts reuse the earlier download and spend no GitHub API call.
    if fs::metadata(&bin).is_ok_and(|m| m.is_file()) {
        zed::make_file_executable(&bin)?;
        return Ok(bin);
    }
    let asset_name = resolve::proxy_asset_name(os, arch)?;
    let release = zed::github_release_by_tag_name(resolve::PROXY_REPO, tag)?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| format!("release {} has no {asset_name}", release.version))?;
    zed::set_language_server_installation_status(
        id,
        &zed::LanguageServerInstallationStatus::Downloading,
    );
    let kind = if windows {
        zed::DownloadedFileType::Zip
    } else {
        zed::DownloadedFileType::GzipTar
    };
    if let Err(e) = zed::download_file(&asset.download_url, &dir, kind)
        .and_then(|()| zed::make_file_executable(&bin))
    {
        // Drop a partial download so the next start retries.
        fs::remove_dir_all(&dir).ok();
        return Err(e);
    }
    for entry in fs::read_dir(".").map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("lean4-ide-proxy-") && name != dir {
            fs::remove_dir_all(entry.path()).ok();
        }
    }
    Ok(bin)
}

impl zed::Extension for Lean4Extension {
    fn new() -> Self {
        Self { cached_proxy: None }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let shell_env = worktree.shell_env();
        let LspSettings {
            binary, settings, ..
        } = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?;

        // Check lsp.lean4-ide.binary
        if let Some(binary) = binary
            && let Some(path) = binary.path
        {
            return Ok(zed::Command {
                command: path,
                args: binary.arguments.unwrap_or_default(),
                env: vec![],
            });
        }

        let lake_args = resolve::lake_server_args(settings.as_ref(), &worktree.root_path());

        // Check $PATH
        if let Some(path) = worktree.which("lake") {
            return Ok(self.launch(
                language_server_id,
                settings.as_ref(),
                worktree,
                path,
                lake_args,
            ));
        }

        // Check $ELAN_HOME or default directory
        let elan_home = shell_env
            .iter()
            .find_map(|(k, v)| (k == "ELAN_HOME").then_some(PathBuf::from(v)))
            .or_else(|| {
                let home = shell_env
                    .iter()
                    .find_map(|(k, v)| (k == "HOME" || k == "USERPROFILE").then_some(v))?;
                Some(PathBuf::from(home).join(".elan"))
            })
            .ok_or("Failed to find ELAN_HOME, HOME, or USERPROFILE")?;

        let (platform, arch) = zed::current_platform();

        let lake_path = elan_home.join("bin").join(match platform {
            zed::Os::Windows => "lake.exe",
            zed::Os::Mac | zed::Os::Linux => "lake",
        });
        let lake_path_str = lake_path.to_string_lossy().to_string();

        if worktree.which(&lake_path_str).is_some() {
            return Ok(self.launch(
                language_server_id,
                settings.as_ref(),
                worktree,
                lake_path_str,
                lake_args,
            ));
        }

        // Check lsp.lean4-ide.settings
        let elan_auto_install = settings
            .as_ref()
            .and_then(|s| s.get("elan_auto_install"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if !elan_auto_install {
            return Err(
                "Failed to find lake in PATH or ELAN_HOME/default path. Enable lsp.lean4-ide.settings.elan_auto_install or configure lsp.lean4-ide.binary.path."
                    .into(),
            );
        }

        let elan_default_toolchain = settings
            .as_ref()
            .and_then(|s| s.get("elan_default_toolchain"))
            .and_then(Value::as_str)
            .unwrap_or("stable");

        // Install elan and lean 4 toolchain
        let release = zed::latest_github_release(
            "leanprover/elan",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset_name: String = format!(
            "elan-{}-{}.{}",
            match arch {
                zed::Architecture::Aarch64 => "aarch64",
                zed::Architecture::X8664 => "x86_64",
                zed::Architecture::X86 =>
                    return Err("32-bit x86 architecture is not supported by elan".into()),
            },
            match platform {
                zed::Os::Windows => "pc-windows-msvc",
                zed::Os::Mac => "apple-darwin",
                zed::Os::Linux => "unknown-linux-gnu",
            },
            match platform {
                zed::Os::Windows => "zip",
                zed::Os::Mac | zed::Os::Linux => "tar.gz",
            },
        );

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("No asset found matching: {:?}", asset_name))?;

        let version_dir = format!("elan-{}", release.version);
        let elan_init_name = match platform {
            zed::Os::Windows => "elan-init.exe",
            zed::Os::Mac | zed::Os::Linux => "elan-init",
        };

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::Downloading,
        );
        zed::download_file(
            &asset.download_url,
            &version_dir,
            match platform {
                zed::Os::Windows => zed::DownloadedFileType::Zip,
                zed::Os::Mac | zed::Os::Linux => zed::DownloadedFileType::GzipTar,
            },
        )
        .map_err(|e| format!("Failed to download file: {e}"))?;

        let cwd =
            env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;
        let elan_init_path = cwd.join(&version_dir).join(elan_init_name);
        let elan_init_path_str = elan_init_path.to_string_lossy().to_string();

        zed::make_file_executable(&elan_init_path_str)?;
        zed::Command::new(&elan_init_path_str)
            .args([
                "-y",
                "--default-toolchain",
                &format!("leanprover/lean4:{elan_default_toolchain}"),
            ])
            .output()?;

        fs::remove_dir_all(&version_dir).ok();

        Ok(self.launch(
            language_server_id,
            settings.as_ref(),
            worktree,
            lake_path_str,
            lake_args,
        ))
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<Value>> {
        LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.initialization_options)
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<Value>> {
        LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.settings)
    }
}

zed::register_extension!(Lean4Extension);
