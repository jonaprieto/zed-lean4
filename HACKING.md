# Hacking on Lean 4 IDE for Zed

This is for working on the extension itself. For using it, see the
[README](README.md). Zed's
[Developing Extensions](https://zed.dev/docs/extensions/developing-extensions)
guide covers the extension API.

## Layout

- `src/`: the extension, compiled to WebAssembly. It finds Lake, finds or
  downloads the proxy, and starts the language server.
- `crates/proxy/`: `lean4-ide-proxy`, a language server proxy between Zed and
  `lake serve`. It also serves the browser infoview and runs the terminal
  infoview (`lean4-ide-proxy tui`).
- `web/`: the browser infoview page.
- `languages/lean4/`: tree-sitter queries, semantic token rules and tasks.
- `snippets/`: the Unicode abbreviations.

## Build and test

```sh
cargo test --workspace
cargo build -p lean4-ide-proxy
```

To try a local proxy, set `lsp.lean4-ide.settings.proxy_path` to the absolute
path of the built binary and restart the language server. To try the
extension, run `zed: install dev extension` on the clone. CI also runs
`cargo fmt`, `cargo clippy -D warnings`, a `wasm32-wasip2` build, Zed's
`zed-extension` check and a lint of the tree-sitter queries.

The extension looks for the proxy in `proxy_path`, then as `lean4-ide-proxy`
on `PATH`, then downloads the release matching its own version. Any failure
falls back to plain `lake serve`. `proxy_path` and `binary.path` run whatever
they point at, so only set them in settings you trust.

## How the infoviews work

Zed extensions cannot draw panels or send custom LSP requests, so the proxy
does both. It forwards Zed's traffic to `lake serve`, adds the goal to hover,
and opens a socket on a random loopback port with a token in its URL. The
browser page and the terminal infoview are both clients of that socket.

On start, the proxy points `~/.cache/lean4-ide/bin/lean4-ide-proxy` at
itself, which is what the "Lean: terminal infoview" task runs, so the view
always matches the running proxy. The terminal infoview finds the proxy
through a discovery file in `$XDG_RUNTIME_DIR/lean4-ide`, or
`~/.cache/lean4-ide/run` when that is unset (`%LOCALAPPDATA%\lean4-ide\run`
on Windows), and picks the server whose project root contains the directory
it ran from. The file holds the page URL with its token, so treat it like the
URL. After a proxy restart the view reconnects once a second.

Goals come from Lean's interactive goal RPC, so hypotheses, case names and
goal diffs are colored as in the VS Code infoview. Goals refresh when the
cursor moves and when Lean publishes new diagnostics. Messages at the cursor
are Lean's interactive messages; trace children are fetched when opened.

## Releases

Bump the version in `Cargo.toml`, `crates/proxy/Cargo.toml` and
`extension.toml` in one commit, then push a signed tag `vX.Y.Z`. The Release
workflow checks the tag matches `extension.toml`, creates the release and
attaches `lean4-ide-proxy-<target>` builds for macOS and Linux (arm64 and
x86_64) and Windows (x86_64). The extension downloads the release whose tag
matches its version, so every version needs its release.

## Roadmap

Compared against [vscode-lean4](https://github.com/leanprover/vscode-lean4).

### Infoview

- [x] Goal state at the cursor in hover, via `$/lean/plainGoal`
- [x] Browser infoview following the editor cursor
- [x] Terminal infoview (goals, expected type, messages) in Zed's terminal
- [x] Tactic goals with hypotheses, case names and goal diffs
- [x] Expected type (term goal)
- [x] Messages at the cursor and an All Messages view
- [ ] Hypothesis filters in the terminal infoview: hide types, instances, inaccessible names, let values; reverse order
- [x] Pin
- [ ] Pause
- [x] Hover inside goals and go to definition from the infoview
- [ ] User widgets (ProofWidgets, `@[widget_module]`, panel widgets)
- [x] Copy goals to a comment, and edits the browser infoview asks for, applied back to the Zed buffer
- [x] Trace explorer with lazy expansion
- [ ] Search in traces
- [x] File progress (elaboration status) and Restart File
- [ ] Clear status when the server stops or restarts

### Editing

- [x] Unicode input by completing the shipped snippets (`\alpha` then Tab); Zed extensions cannot rewrite on space
- [x] Abbreviation hover
- [ ] Symbol picker
- [ ] Unsolved-goals and goals-accomplished markers
- [ ] Fold and unfold all proofs
- [x] Module hierarchy: imports and imported-by (terminal infoview)
- [x] Semantic token rules for Lean's own token types
- [x] Prompt to restart a file when its imports are stale

### Toolchain and projects

- [ ] Update and uninstall elan
- [ ] Select the default and per-project toolchain
- [x] Lake commands: build, clean, update dependency (Zed tasks)
- [x] Fetch the Mathlib cache (`lake exe cache get`)
- [ ] New standalone or Mathlib project
- [ ] Setup diagnostics with actionable errors (missing elan or lake, unsupported Lean version)

### Search and docs

- [ ] Loogle search
- [ ] Abbreviation reference

### Quality

- [x] Rust unit tests for the extension's server resolution
- [x] Integration tests for the proxy against a scripted fake server and a pinned real Lean
- [x] CI: fmt, clippy, tests, `zed-extension` check, tree-sitter query lint
- [x] Release workflow publishing proxy binaries for macOS, Linux and Windows
- [ ] Tree-sitter-lean is experimental and needs improvement, plus highlight fixture tests
- [ ] Narrow the `process:exec` capability from `*` to the commands actually run
