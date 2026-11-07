# Agent Cleanup

A small, fast Windows app that finds the clutter AI coding agents (Claude Code, Codex and friends) leave behind: repos cloned "for testing" into random folders, stale clones in Downloads or `%TEMP%`, and scratch projects that never made it into your real project folders.

It's written in Rust with [GPUI](https://www.gpui.rs), the GPU-accelerated UI framework behind the Zed editor, plus the [gpui-component](https://github.com/longbridge/gpui-kit) widgets. It's a single native `.exe` (about 30 MB) with no installer, web engine or runtime.

![Main window](docs/screenshot.png)

## How it works

1. **You add your project folders** on the Projects page. Repos inside them are yours and are left alone. Any repo found elsewhere is a possible leftover.
2. **It scans for repos** under your home folder, skipping dependency caches (`node_modules`, `target`, `.cargo` and so on) and everything in AppData except `AppData\Local\Temp`. Results are saved, so the app opens instantly next time.
3. **It checks what's unsaved** using git, read-only: uncommitted changes, commits that aren't on any remote, stashes, and whether the repo has a remote at all.
4. **It gives each repo a status:**
   - **Safe to remove:** outside your projects, unused for a while (14 days by default), and everything is pushed.
   - **Has unsaved work:** outside your projects, but something in it exists only on this PC.
   - **Stray, recent:** outside your projects but used recently.
   - **In a project:** inside one of your project folders.

The left rail switches between **Leftovers**, **Agent trail**, **Build junk**, **Updates**, **Projects**, **Activity** (everything the app moved or put back), **How this works** and **Settings**. On Leftovers, the sidebar filters by status and the cards at the top show how much space you can free. Click any repo to see what's unsaved and what to do about it, for example "Run `git push` to upload the 2 commits". Buttons there open the folder or a terminal in it.

![Details panel](docs/details.png)

## Agent trail

Claude Code and Codex log every session (`~/.claude/projects` and `~/.codex/sessions`). The **Agent trail** page reads the last 90 days of those logs, read-only, and lists what an agent made outside the folder it was working in: files it wrote, things it downloaded with `curl` or `wget`, and repos it cloned. Items are grouped by the project the agent was working on, with which agent did it and when.

Files an agent only *changed* (say, your `.bashrc`), files that are older than the session, and clones with unsaved git work are marked **Check first**. They need a tick before they can be recycled, and **Clean up** leaves them out.

![Agent trail](docs/trail.png)

## Build junk

The **Build junk** page looks inside the repos the scan found for dependency and build folders: `node_modules`, Rust `target`, Python venvs, `dist`/`build`/`.next` and other framework caches, Gradle and .NET output, and Python tool caches. A folder is only listed when git ignores it *and* the file that rebuilds it is next to it (`package.json` beside `node_modules`, `Cargo.toml` beside `target`, `pyvenv.cfg` inside a venv). So a `build` folder that holds real source never shows up.

Each row says what brings it back (`npm install`, `cargo build`, ...). **Clean up** only takes folders from projects you haven't touched for the "stale" days set in Settings. Recycling one from a project you're still working on needs a tick first. Big folders move in the background so the window stays responsive. Windows refuses to recycle anything bigger than the Recycle Bin allows, so those stay where they are and the error says so. They are never permanently deleted.

![Build junk](docs/junk.png)

## Updates

The **Updates** page shows outdated packages in your projects, using each tool's own check: `npm outdated`, `cargo update --dry-run` and `pip list --outdated` (in the project's venv). Only repos inside your project folders are checked, because checking runs their tools. Bigger updates that may need code changes (a new major version) are marked but left for you to do by hand.

**Update** opens a terminal in the project running `npm update`, `cargo update` or `pip install --upgrade ...`, so you can watch it and stop it. It only starts when git shows no uncommitted changes, so `git diff` shows exactly what the update changed and `git checkout .` undoes it.

![Updates](docs/updates.png)

## Cleaning up

- **Clean up** recycles every repo marked safe in one go. You can also recycle one repo from its card (hover for the buttons), or tick several and choose **Recycle selected**.
- **Undo** appears right after, and puts everything back where it was.
- **It's a project** tells the app a folder is your real work. **Hide** stops showing a repo (unhide it on the Projects page).
- Press Escape to close a dialog or the details panel.

## Safety

- Nothing is ever permanently deleted. Recycle moves folders to the **Recycle Bin**.
- Every removal asks first. If any repo has unsaved work, the dialog lists exactly what would be lost and you have to tick a box before the button unlocks.
- If git can't read a repo, the app assumes it has unsaved work. It never guesses "safe".
- Every move and restore is written to `actions.log` in `%APPDATA%\agent-cleanup\config\`, next to `config.toml`, and shown on the Activity page.

![Confirm dialog](docs/confirm-dialog.png)

## Getting the app

**Download:** get `agent-cleanup-<version>-windows-x64.exe` from the [latest release](../../releases/latest) and run it. Windows SmartScreen may warn you the first time because the file isn't code-signed. Click **More info**, then **Run anyway**.

You also need [Git for Windows](https://git-scm.com/download/win) on your `PATH`.

**Updates:** the app checks for a new release when it starts (you can turn this off in Settings). When one is available, click **Update now** and it downloads the new version, swaps it in, and offers to restart.

**Build it yourself** with [Rust](https://rustup.rs):

```powershell
cargo run --release
```

On Linux you also need `libxkbcommon-dev`, `libxkbcommon-x11-dev` and `libwayland-dev`.

## Releasing

Bump `version` in `Cargo.toml` and merge to `main`. The Release workflow builds the .exe on Windows and publishes it as `vX.Y.Z`, and installed copies offer the update.

## Roadmap

1. **Downloads matched to projects:** link files in Downloads to the project you were working on when they arrived.
2. A scheduled background scan and a tray icon.

## Development

```powershell
cargo test                                  # needs git on PATH
cargo test -- --ignored                     # also hits the real GitHub API
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The core logic (`src/config.rs`, `git.rs`, `model.rs`, `scan.rs`, `cache.rs`, `actions.rs`, `update.rs`, `library.rs`, `activity.rs`) has no UI code and is unit tested. The window lives in `src/ui/`: `app.rs` (state and actions), `leftovers.rs`, `details.rs`, `pages.rs`, `widgets.rs`, `theme.rs` (palette) and `assets.rs` (fonts and icons).
