# Agent Cleanup

Windows desktop app (Rust + GPUI via `gpui-kit`, pinned with `=`) that finds clutter left by AI coding agents.

- Core logic is UI-free in `src/lib.rs` modules (config, git, model, scan, cache, actions, update, library, activity, trail) and must stay unit-tested. UI is `src/ui/` only; keep list logic (filters, sorting, stats) in `library.rs`, not in the UI.
- Safety rules are product requirements, not suggestions: never permanently delete (use `trash`), always confirm, treat unknown git state as "has unsaved work", log every action via `actions::log_action`.
- Git access is read-only, through the `git` CLI with `GIT_OPTIONAL_LOCKS=0` and `CREATE_NO_WINDOW` on Windows.
- Keep it lean: check binary size impact before adding dependencies (CI prints the .exe size). Icons are Lucide via gpui-kit-assets: add new ones to `ExtraIcons` in `src/ui/assets.rs`, never register `AllAssets` (+1 MB). Colors come from `src/ui/theme.rs`.
- GPUI is pre-1.0: upgrade `gpui-kit` deliberately, one version at a time, and re-check every page. Linux builds need `libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev`.
- Before pushing: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. CI runs the same on `windows-latest` and uploads the `.exe`.
- Releases: bump `version` in Cargo.toml and merge; `.github/workflows/release.yml` publishes `agent-cleanup-vX.Y.Z-windows-x64.exe`, which the in-app updater (`src/update.rs`) looks for. Keep that asset name.
