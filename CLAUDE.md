# Agent Cleanup

Windows desktop app (Rust + egui/eframe, glow backend) that finds clutter left by AI coding agents.

- Core logic is UI-free in `src/lib.rs` modules (config, git, model, scan, actions) and must stay unit-tested. UI is `src/app.rs` only.
- Safety rules are product requirements, not suggestions: never permanently delete (use `trash`), always confirm, treat unknown git state as "has unsaved work", log every action via `actions::log_action`.
- Git access is read-only, through the `git` CLI with `GIT_OPTIONAL_LOCKS=0` and `CREATE_NO_WINDOW` on Windows.
- Keep it lean: check binary size impact before adding dependencies. Default egui fonts have no emoji, so use plain-text button labels.
- Before pushing: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. CI runs the same on `windows-latest` and uploads the `.exe`.
