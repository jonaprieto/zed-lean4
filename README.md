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

### Browser infoview

In a Lean file, choose Open Lean Infoview from Zed's code actions. The official
Lean infoview opens in your browser, follows the Zed cursor and shows the goals
and messages there. Edits and locations the infoview asks for (copy to comment,
suggestions, go to definition) are sent back to Zed; see the TODO list below for
what has been checked so far.

The proxy serves the page on a random loopback port; the socket needs the token
in the URL. Anyone with the URL can read the open Lean files and edit them
through Zed, so keep it private. Lean widgets run inside the page with the same
access, so only open trusted projects.
For now, Restart File is a logged no-op, remote worktrees are unsupported, and a
proxy restart requires reopening the page and resets preferences for the new port.

For development, build with `cargo build -p lean4-ide-proxy`, set
`lsp.lean4-ide.settings.proxy_path` to the absolute binary path, and restart the
language server. Setting `lsp.lean4-ide.settings.proxy: false` or an explicit
`lsp.lean4-ide.binary` bypasses the proxy.

### Terminal infoview

Click the run button in the gutter next to any `theorem`, `lemma` or
`example` (or run the task "Lean: terminal infoview" from `task: spawn`) to get
the goals, the expected type and the messages at the cursor in Zed's terminal.
The editor keeps focus, and the view follows the cursor from then on; you only
open it once per session. The task runs `~/.cache/lean4-ide/bin/lean4-ide-proxy`,
which the proxy points at itself whenever it starts, so nothing needs to be on
your `PATH` and the view always matches the running proxy. To put it
beside the code instead of at the bottom, set `"terminal": {"dock": "right"}` in
your Zed settings.

The terminal infoview is a second client of the same socket the browser page
uses. It finds the running proxy through a discovery file in
`$XDG_RUNTIME_DIR/lean4-ide`, or `~/.cache/lean4-ide/run` when that is unset
(`%LOCALAPPDATA%\lean4-ide\run` on Windows), and picks the server whose
project root contains the directory you ran it from. The file holds the page
URL with its token, so treat it like the URL itself. Goals come from Lean's
interactive goal RPC, so hypothesis names, case names and goal diffs are colored
like in the VS Code infoview, and keywords, literals and local variables are
highlighted in the terminal's theme colors. Goals refresh when the
cursor moves and when Lean publishes new diagnostics for the file. After a
proxy restart it reconnects on its own, once a second.

The first line names the file and cursor, then how far Lean has checked the
file: `⋯ checked through line N` while it elaborates the rest, `✓ checked`
once it is done, and `✗ stopped at line N` when elaboration gave up there.
Zed lets extensions color neither the gutter nor line backgrounds, so this is
where the checked range shows.

Keys: `q`, `Esc` or `Ctrl-C` quit, `p` pins the current goals, `a` shows all
messages, `c` copies the goals into a comment in the buffer, `j`/`k` or the
arrow keys move the selection, `g`/`G` jump to the top or bottom, and `Enter`,
space or `Tab` fold the section, goal or message around the selection. Long
lines wrap with a hanging indent.

The mouse works too: the wheel scrolls, a click on a header folds it, and a
click on a term in a goal shows its type and docstring below the goals. With a
term picked, `d` and `t` jump Zed to its definition or its type, and `Esc`
closes the panel. Hold Shift to select text with the mouse instead.

Messages at the cursor are Lean's interactive messages: expressions in them are
highlighted and clickable, embedded goals render as goals, and traces fold, with
children Lean sends on demand fetched when you open them. `a` (all messages)
lists the plain published diagnostics for the whole file.

The last section, At the cursor, is Lean's hover for the term under the editor
cursor: its signature, highlighted, and its docstring. It follows the cursor,
so you get the documentation of what you are on without the mouse. Lean widgets render
HTML, which a terminal cannot show; press `o` to open the browser infoview for
those.

### Infoview settings

Both infoviews read `lsp.lean4-ide.settings.infoview` from your Zed settings,
and pick up changes without a restart:

```json
"lsp": {
  "lean4-ide": {
    "settings": {
      "infoview": {
        "open": "terminal",
        "theme": {
          "keyword": "light-magenta",
          "inserted": {"fg": "black", "bg": "#87d787"},
          "selection": {"bg": "236", "modifiers": []}
        }
      }
    }
  }
}
```

- `open`: what the gutter run button opens, `"terminal"` (the default) or
  `"browser"`. With `"browser"` the task opens the browser infoview and exits;
  the Open Lean Infoview code action always opens the browser.
- `theme`: colors of the terminal infoview, role by role. By default it uses
  the terminal's named colors, which Zed's terminal takes from your editor
  theme, so most themes need nothing here. A role takes a color (a name such
  as `red` or `light-blue`, `#rrggbb`, or a 256-color index such as `"236"`)
  for its foreground, or an object with `fg`, `bg` and `modifiers` (`bold`,
  `dim`, `italic`, `underlined`, `reversed`, `crossed_out`; a list replaces the
  role's own). Roles: `hypothesis`, `goal_prefix`, `inaccessible`, `case_name`,
  `inserted`, `removed`, `keyword`, `literal`, `trace_class`, `selection`,
  `heading`, `docs`, `link`, `error`, `warning`, `information`, `hint`,
  `checked`, `checking`. Unknown roles and colors show in the view's footer.

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

Setting `binary` bypasses the proxy, including hover goals and the browser infoview; omit it to use the proxy.

## TODO list

Compared against [vscode-lean4](https://github.com/leanprover/vscode-lean4), the reference
editor integration. Zed extensions cannot draw panels or send custom LSP requests, so the
plan is a Rust LSP proxy between Zed and `lake serve` that serves the official
[`@leanprover/infoview`](https://www.npmjs.com/package/@leanprover/infoview) in a browser
window. Items marked (proxy) depend on it.

### Infoview (proxy)

- [x] Goal state at the cursor in hover, via `$/lean/plainGoal`
- [x] Browser infoview following the editor cursor
- [x] Terminal infoview (goals, expected type, messages) in Zed's terminal
- [ ] Tactic goals with hypotheses, case names and goal diffs
- [ ] Expected type (term goal)
- [ ] Messages at the cursor and an All Messages view
- [ ] Hypothesis filters: hide types, instances, inaccessible names, let values; reverse order
- [ ] Pin and pause
- [ ] Hover inside goals and go to definition from the infoview
- [ ] User widgets (ProofWidgets, `@[widget_module]`, panel widgets)
- [ ] "Try this" suggestions and copy-to-comment, applied back to the Zed buffer
- [ ] Trace explorer with lazy expansion and search
- [ ] File progress (elaboration status) and Restart File
- [ ] Clear status when the server stops or restarts

### Editing

- [ ] Unicode input that behaves like VS Code (`\alpha` then space or tab), instead of snippets
- [ ] Abbreviation hover and a symbol picker
- [ ] Unsolved-goals and goals-accomplished markers
- [ ] Fold and unfold all proofs
- [ ] Module hierarchy: imports and imported-by
- [ ] Semantic token highlighting on by default for Lean files
- [ ] Prompt to restart a file when its imports are stale (`$/lean/staleDependency`)

### Toolchain and projects

- [ ] Update and uninstall _elan_
- [ ] Select the default and per-project toolchain
- [ ] Lake commands: build, clean, update dependency
- [ ] Fetch the Mathlib cache (`lake exe cache get`)
- [ ] New standalone or Mathlib project
- [ ] Setup diagnostics with actionable errors (missing elan or lake, unsupported Lean version)

### Search and docs

- [ ] Loogle search
- [ ] Abbreviation reference

### Quality

- [x] Rust unit tests for the extension's server resolution
- [x] Integration tests for the proxy against a scripted fake server and a pinned real Lean
- [x] CI: fmt, clippy, tests, `zed-extension` check, tree-sitter query lint
- [ ] Release workflow publishing proxy binaries for macOS, Linux and Windows
- [ ] Tree-sitter-lean is experimental and needs improvement, plus highlight fixture tests
- [ ] Narrow the `process:exec` capability from `*` to the commands actually run

## Development

To develop this extension, see the [Developing Extensions](https://zed.dev/docs/extensions/developing-extensions) section of the Zed docs.
