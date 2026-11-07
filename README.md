# Agent Cleanup

A small, fast Windows app that finds the clutter AI coding agents (Claude Code, Codex and friends) leave behind: repos cloned "for testing" into random folders, stale clones in Downloads or `%TEMP%`, and scratch projects that never made it into your real project folders.

It's written in Rust with [egui](https://github.com/emilk/egui), so it's a single native `.exe` with no web engine or runtime to install.

![Main window](docs/screenshot.png)

## What it does today

- **Finds every git repo** under the folders you choose (your home folder by default), skipping dependency and toolchain caches like `node_modules`, `target` and `.cargo`.
- **Tells your work from leftovers.** You list your project folders. Any repo found outside them is flagged as a possible leftover.
- **Reads each repo's git state:** uncommitted changes, commits that aren't on any remote, stashes, and whether it has a remote at all.
- **Gives each repo a verdict:**
  - **Safe to remove:** outside your projects, unused for a while (14 days by default), and everything is pushed.
  - **Has unsaved work:** outside your projects, but something in it exists only on this PC.
  - **Stray, recent:** outside your projects but used recently.
  - **In a project:** inside one of your project folders.
- **Shows size and last activity**, so you can see where the space went.
- **Actions:** open in File Explorer, move to the Recycle Bin, mark as a project folder, hide from the list, copy path.

## Safety

- Nothing is ever permanently deleted. Removing a repo moves it to the **Recycle Bin**.
- Every removal asks first. If the repo has unsaved work, the dialog lists exactly what would be lost and you have to tick a box before the button unlocks.
- If git can't read a repo, the app assumes it has unsaved work. It never guesses "safe".
- Every action is written to `actions.log` in `%APPDATA%\agent-cleanup\config\`, next to `config.toml`.
- Git is only used read-only (`status`, `log`, `rev-list`, `remote`, `stash list`).

![Confirm dialog](docs/confirm-dialog.png)

## Requirements

- Windows 10 or 11
- [Git for Windows](https://git-scm.com/download/win) on your `PATH`

## Getting the app

**Download:** grab `agent-cleanup-<version>-windows-x64.exe` from the [latest release](../../releases/latest). It's a single file with no installer, so just run it.

To ship a new version, bump `version` in `Cargo.toml` and merge to `main`. The Release workflow builds the .exe on Windows and publishes it as a release.

**Or build it yourself** with [Rust](https://rustup.rs):

```powershell
cargo run --release
```

## Roadmap

1. **Agent trail reader:** read Claude Code (`~/.claude/projects`) and Codex (`~/.codex/sessions`) session logs to find files and clones an agent created outside the project it was working on.
2. **Downloads matched to projects:** link files in Downloads to the project you were working on when they arrived.
3. **Build junk reclaim:** `node_modules`, `target`, `.venv` and `dist` in projects you haven't touched lately.
4. Bulk actions, a scheduled background scan, and a tray icon.

## Development

```powershell
cargo test                                  # needs git on PATH
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The core logic (`src/config.rs`, `git.rs`, `model.rs`, `scan.rs`, `actions.rs`) has no UI code and is unit tested. The window lives in `src/app.rs`.
