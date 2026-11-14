# AGENTS.md

Notes for coding agents working on Grids. [README.md](README.md) explains what
the app is and what it does for users; this file is about how to change it
without breaking the rules the project is built on.

## Three layers, one rule

| Where | What belongs there |
| --- | --- |
| `crates/core` | Pure logic: layout model, pane state, sorting and filtering, listings, formatting, sessions, settings, journal and undo, frecency, fuzzy matching. No GPUI, no Win32, so it builds and tests anywhere. |
| `crates/shell` | Everything that talks to Windows: file operations, the Recycle Bin, the Restart Manager, clipboard, thumbnails, zip, context menus, the shell's preview handlers. |
| `src/` | The GPUI app: window, panes, file lists, menus, theme, key bindings. Drawing and input wiring only. |

If behaviour can be tested without a window, it belongs in `crates/core`. If it
touches the OS, it belongs in `crates/shell`. `src/` composes the two and should
not grow logic of its own that a test could cover.

## Build and check

```
build.cmd                                                 # release build, copies grids.exe next to the script
cargo build --release --locked
cargo test --release --locked --workspace
cargo clippy --release --locked -p grids_shell --all-targets
```

`rust-toolchain.toml` pins the compiler and CI installs exactly that version.
Keep `--locked` on every command: `Cargo.lock` is committed and CI rejects a
lock file it would have to change. The build needs the Windows SDK's `fxc.exe`
(GPUI compiles HLSL shaders with it); `build.cmd` sets `GPUI_FXC_PATH` to the
newest installed SDK, as CI does.

## Branches

Two branches, and only two.

- `main` is what GitHub shows and what users get. It moves by pull request.
- `gpui-rewrite` is where work happens: commit there, then open the PR into
  `main`. Do not push to `main` directly, and do not add other long-lived
  branches.

The fork's upstream (`efww/grids`) is only where the project came from. Its
0.7.x build, whose panes were embedded Explorer views, has nothing in common
with this app, so there is nothing to pull, merge or sync from it, and no
upstream remote is configured. Leave it that way: an upstream commit would
conflict everywhere and fix nothing here.

Two things from upstream stay regardless: the MIT copyright lines in
[LICENSE](LICENSE) and the credit paragraph at the end of README.md. That is a
licence obligation, not tracking upstream.

## Hard constraints

- **No networking code beyond the version check.** `grids.exe` links no HTTP
  stack: CI runs a deny-list over `cargo tree` (reqwest, hyper, tokio, rustls,
  tungstenite, curl and friends) and checks the exe's imports with `dumpbin`
  for winhttp, wininet, ws2_32 and similar. The one outbound request is the
  update check in `src/workspace.rs` (`fetch_latest`): a single GET to the
  GitHub releases API, run through the system's `curl.exe` so no library is
  linked, sending only that URL, and switchable off in the version popover.
  Anything else that talks to the network is out of scope, and the README says
  exactly this: keep that promise true.
- **No panics in the platform layer.** `crates/shell` returns `Result` or
  `Option` and degrades gracefully; a failing shell call must not take the app
  down. `unwrap`/`expect` belong in tests and build scripts, not in code that
  runs against a user's disk.
- **Size budget.** The release profile is size-first (`opt-level = "z"`, fat
  LTO, one codegen unit; only `gpui` and `taffy` get `opt-level = 3`). CI fails
  when `grids.exe` passes 8,000,000 bytes, and the `size` job reports per-crate
  growth. Prefer std over a new crate, and delete code before adding a feature
  flag for it.
- **Static C runtime.** `.cargo/config.toml` links the CRT statically so a clean
  Windows can start the exe; CI fails if `vcruntime*`, `msvcp*` or
  `api-ms-win-crt-*` imports come back.
- **The vendored GPUI and taffy are patched copies**, not upstream. Any change
  there must be minimal, keep the `quiet.md:` marker comment on the patched
  line, and add or update a row in [vendor/PATCHES.md](vendor/PATCHES.md) (that
  file is in Chinese, like the rest of its history).

## Conventions

- Every module starts with a `//!` line saying what it owns. Comments explain
  why the code is the way it is; they never restate what the next line does.
  The codebase favours short functions, early returns and one-line bodies.
- User-visible text goes through `tr(cx, "中文", "English")` (`src/state.rs`).
  Simplified Chinese is the default, so both strings are required; `GRIDS_LANG=en`
  switches to English at startup. Column names that come from the Windows Shell
  stay in the system language on purpose.
- Identifiers and code comments are English. Commit messages are one short
  imperative line with a layer prefix: `core:`, `shell:`, `CI:`, `U<n>:` for a
  numbered feature step, or `Version X.Y.Z` for a version commit. Add a body
  only when it explains a non-obvious cause.
- Runtime state – `grids-gpui.json`, `grids-journal.jsonl`, `sessions/` – is
  written next to the exe and is gitignored. Never commit it, and never commit a
  path or fixture that only exists on your machine.
- Tests are `#[test]` functions inside the module they cover, in `crates/core`
  or `crates/shell` (some shell tests call the real Windows Shell: known
  folders, system icons, a file-operation round trip). `src/` has no tests by
  design; move testable behaviour down a layer instead.
- README.md is the user-facing feature list, in Simplified Chinese. When
  behaviour, a shortcut or a setting changes, update README.md and keep
  README.en.md in step; the language link at the top of each file switches
  between them.

## Running it while you work

Run `grids.exe` from the project folder so its settings stay in
`grids-gpui.json` beside it. `GRIDS_LANG=en` switches the interface to English,
`GRIDS_CONFIG=<path>` moves the settings file, `GRIDS_LOG=info` logs warnings to
stderr. Panics are appended to `grids-crash.log` as `grids+0x…` offsets, which
resolve against the `grids.pdb` that CI uploads with the build.

Don't run Grids as administrator: Windows' UIPI then blocks mouse input from
normal programs, and tools such as PowerToys freeze over the window.

## Before saying it is done

Build it, run the tests, and run clippy on the layer you touched. If the change
is large, check `cargo tree` for new crates and look at the CI `size` job's
per-crate table. If a user-visible string, shortcut or setting changed, README
changed with it.
