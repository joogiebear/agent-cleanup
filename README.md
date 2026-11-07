# Agent Cleanup

A small, fast Windows app that finds the clutter AI coding agents (Claude Code, Codex and friends) leave behind: repos cloned "for testing" into random folders, stale clones in Downloads or `%TEMP%`, and scratch projects that never made it into your real project folders.

It's written in Rust with [egui](https://github.com/emilk/egui), so it's a single native `.exe` with no installer, web engine or runtime.

![Main window](docs/screenshot.png)

## How it works

1. **You add your project folders.** Repos inside them are yours and are left alone. Any repo found elsewhere is a possible leftover.
2. **It scans for repos** under your home folder, skipping dependency caches (`node_modules`, `target`, `.cargo` and so on) and everything in AppData except `AppData\Local\Temp`. Results are saved, so the app opens instantly next time.
3. **It checks what's unsaved** using git, read-only: uncommitted changes, commits that aren't on any remote, stashes, and whether the repo has a remote at all.
4. **It gives each repo a status:**
   - **Safe to remove:** outside your projects, unused for a while (14 days by default), and everything is pushed.
   - **Has unsaved work:** outside your projects, but something in it exists only on this PC.
   - **Stray, recent:** outside your projects but used recently.
   - **In a project:** inside one of your project folders.

Click any row to see what's unsaved and what to do about it, for example "Run `git push` to upload the 2 commits". Buttons there open the folder or a terminal in it.

![Details panel](docs/details.png)

## Cleaning up

- **Recycle** one repo from its row, or tick several (or use **Select all safe**) and choose **Recycle selected**.
- **Undo** appears right after, and puts everything back where it was.
- **Mark as project** tells the app a folder is your real work. **Hide** stops showing a repo.
- Right-click a row for more options.

## Safety

- Nothing is ever permanently deleted. Recycle moves folders to the **Recycle Bin**.
- Every removal asks first. If any repo has unsaved work, the dialog lists exactly what would be lost and you have to tick a box before the button unlocks.
- If git can't read a repo, the app assumes it has unsaved work. It never guesses "safe".
- Every move and restore is written to `actions.log` in `%APPDATA%\agent-cleanup\config\`, next to `config.toml`. Use **Open settings and action log** in the app to find it.

![Confirm dialog](docs/confirm-dialog.png)

## Getting the app

**Download:** get `agent-cleanup-<version>-windows-x64.exe` from the [latest release](../../releases/latest) and run it. Windows SmartScreen may warn you the first time because the file isn't code-signed. Click **More info**, then **Run anyway**.

You also need [Git for Windows](https://git-scm.com/download/win) on your `PATH`.

**Updates:** the app checks for a new release when it starts (you can turn this off in the left panel). When one is available, click **Update now** and it downloads the new version, swaps it in, and offers to restart.

**Build it yourself** with [Rust](https://rustup.rs):

```powershell
cargo run --release
```

## Releasing

Bump `version` in `Cargo.toml` and merge to `main`. The Release workflow builds the .exe on Windows and publishes it as `vX.Y.Z`, and installed copies offer the update.

## Roadmap

1. **Agent trail reader:** read Claude Code (`~/.claude/projects`) and Codex (`~/.codex/sessions`) session logs to find files and clones an agent created outside the project it was working on.
2. **Downloads matched to projects:** link files in Downloads to the project you were working on when they arrived.
3. **Build junk reclaim:** `node_modules`, `target`, `.venv` and `dist` in projects you haven't touched lately.
4. A scheduled background scan and a tray icon.

## Development

```powershell
cargo test                                  # needs git on PATH
cargo test -- --ignored                     # also hits the real GitHub API
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The core logic (`src/config.rs`, `git.rs`, `model.rs`, `scan.rs`, `cache.rs`, `actions.rs`, `update.rs`) has no UI code and is unit tested. The window lives in `src/app.rs`.
