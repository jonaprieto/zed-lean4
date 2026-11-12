# Lean 4 IDE for Zed

This is a fork of owlx56/zed-lean4.

A [Lean(4) Theorem Prover](https://lean-lang.org/) extension for [Zed](https://zed.dev).

- Tree-sitter: [tree-sitter-lean](https://github.com/Julian/tree-sitter-lean)
- Language Server: [Lean Language Server](https://github.com/leanprover/lean4/tree/master/src/Lean/Server)

## Lean Toolchain

The Lean Language Server is integrated with the Lean toolchain. It is recommended to manage Lean toolchain versions via [_elan_](https://github.com/leanprover/elan?tab=readme-ov-file#installation). For example, you can install the stable Lean version by running:

```sh
elan default stable
lean --version
```

## Features

### Highlight

- For optimal highlighting, set `"semantic_tokens"` to `"combined"`, which will enable LSP semantic tokens highlight in Zed.

### Goal in hover

Hovering inside a tactic proof shows the goal state below the usual hover text. Disable the proxy with `lsp.lean4-ide.settings.proxy: false`.

### Abbreviation

- Abbreviation (unicode character) insertion is supported by [snippets](https://github.com/jonaprieto/zed-lean4/blob/main/snippets/lean%204.json).

### Nested Lean project

- If the Zed worktree is a parent directory of a Lean project, set `lsp.lean4-ide.settings.lean_project_dir` to the project directory containing `lakefile.toml` (or `lakefile.lean`). Relative paths are resolved from the worktree root; absolute paths are also supported. This starts an automatically detected Lake with `--dir` so project dependencies such as Mathlib are available.
- A configured `lsp.lean4-ide.binary` takes precedence over `lean_project_dir`. Omit `binary` to detect Lake automatically; with an explicit binary, include `"--dir", "/path/to/project"` before `"serve"` in `binary.arguments`.

### Installation

- Firstly, the extension will check `path` and `arguments` in `lsp.lean4-ide.binary` settings. Then it will search `lake` in `$PATH`, try `$ELAN_HOME` and default directory.
- If Lean 4 is not detected, this extension can automatically install elan with default toolchain. To enable this feature, you must set `"elan_auto_install"` to `true`. You can also specify the default toolchain by setting `"elan_default_toolchain"` to `"stable"`, `"nightly"` or `"v4.28.0"`.
- During installation, the extension will automatically add `~/.elan` to your $PATH. The Lean toolchain can be completely removed by running `elan self uninstall`.

### Proxy

The extension runs `lake serve` behind `lean4-ide-proxy`. It looks for the proxy in `lsp.lean4-ide.settings.proxy_path`, then as `lean4-ide-proxy` on `$PATH`, then downloads the matching release from github.com/jonaprieto/zed-lean4. Any failure falls back to plain `lake serve`, and `"proxy": false` disables it. Until the first release exists, install it with `cargo install --path crates/proxy --locked`. `proxy_path` and `binary.path` run whatever they point at, so only set them in settings you trust.

## settings.json Sample

```json
{
  "semantic_tokens": "combined",
  "lsp": {
    "lean4-ide": {
      "binary": {
        "path": "/path/to/your/.elan/bin/lake",
        "arguments": ["--dir", "/path/to/your/worktree/formalization", "serve", "--"]
      },
      "settings": {
        "elan_auto_install": true,
        "elan_default_toolchain": "stable",
        "lean_project_dir": "formalization"
      }
    }
  }
}
```

Setting `binary` bypasses the proxy (and so the goal-in-hover feature); omit it to use the proxy.

## TODO list

- Implement infoview like [VSCode](https://github.com/leanprover/vscode-lean4) and [Neovim](https://github.com/Julian/lean.nvim) with Rust
- Tree-sitter-lean is experimental now and needs improvement
- Update and uninstall _elan_

## Development

To develop this extension, see the [Developing Extensions](https://zed.dev/docs/extensions/developing-extensions) section of the Zed docs.
