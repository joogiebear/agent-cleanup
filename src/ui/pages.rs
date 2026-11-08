//! The other pages (welcome, projects, activity, settings, help) and the
//! overlays (update banner, toast, confirm dialog).

use super::app::{AgentCleanup, ConfirmFor, Page, UpdateState};
use super::downloads::download_icon;
use super::theme::*;
use super::trail::trail_icon;
use super::widgets::*;
use agent_cleanup::config::{Config, path_within};
use agent_cleanup::model::{Verdict, human_size, relative_time};
use agent_cleanup::update;
use gpui_kit::assets::IconName;
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::path::PathBuf;
use std::time::SystemTime;

/// Scrollable page body with a centered, readable column.
fn page_body(id: &'static str) -> Stateful<Div> {
    div().id(id).size_full().overflow_y_scroll()
}

fn column() -> Div {
    v_flex()
        .w_full()
        .max_w(px(760.))
        .mx_auto()
        .px_8()
        .py_8()
        .gap_6()
}

fn setting_row(title: &'static str, help: &'static str, control: impl IntoElement) -> Div {
    h_flex()
        .gap_4()
        .px_5()
        .py_4()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(div().text_size(px(13.)).text_color(MUTED).child(help)),
        )
        .child(control)
}

fn divider() -> Div {
    div().h(px(1.)).bg(BORDER).mx_5()
}

impl AgentCleanup {
    // ---- welcome -------------------------------------------------------

    pub fn welcome(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let step = |n: &'static str, title: &'static str, text: &'static str| {
            h_flex()
                .gap_4()
                .items_start()
                .child(
                    div()
                        .size(px(32.))
                        .flex_none()
                        .rounded_full()
                        .bg(tint(ACCENT, 0.18))
                        .text_color(ACCENT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(n),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_color(MUTED).text_size(px(13.)).child(text)),
                )
        };
        page_body("welcome").child(
            v_flex()
                .w_full()
                .max_w(px(560.))
                .mx_auto()
                .px_8()
                .py(px(64.))
                .gap_8()
                .child(
                    v_flex()
                        .gap_4()
                        .child(
                            div()
                                .size(px(64.))
                                .rounded(px(18.))
                                .bg(ACCENT)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(IconName::BroomSparkles, 32., TEXT)),
                        )
                        .child(
                            div()
                                .text_size(px(28.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Find what your AI coding tools left behind"),
                        )
                        .child(div().text_color(MUTED).text_size(px(15.)).child(
                            "AI coding tools like Claude Code and Codex often download copies of projects or make scratch files outside your real folders. Agent Cleanup finds them, checks whether anything in them is unsaved, and moves the safe ones to the Recycle Bin for you.",
                        )),
                )
                .child(
                    card().p_6().flex().flex_col().gap_5()
                        .child(step("1", "Add your project folders", "The folders where your real work lives, so the app leaves them alone."))
                        .child(step("2", "Scan", "It finds copies of projects (git repos) everywhere else and checks whether any work in them isn't saved anywhere else."))
                        .child(step("3", "Clean up", "Recycle the ones marked safe. You can always undo.")),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(
                            icon_button("w-add", IconName::FolderPlus, "Add a project folder", ButtonKind::Primary)
                                .h(px(42.))
                                .px_5()
                                .on_click(cx.listener(|this, _, _, cx| this.pick_project_folder(cx))),
                        )
                        .child(
                            button("w-skip", "Skip and scan everything", ButtonKind::Ghost)
                                .h(px(42.))
                                .tip("Everything found will show as a leftover until you add project folders.")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.skipped_setup = true;
                                    this.start_scan();
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    // ---- projects ------------------------------------------------------

    pub fn projects_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut folders = v_flex();
        if self.cfg.project_roots.is_empty() {
            folders = folders.child(div().px_5().py_6().text_color(MUTED).child(
                "No project folders yet. Everything found counts as a leftover until you add one.",
            ));
        }
        for (i, root) in self.cfg.project_roots.iter().enumerate() {
            let repos = self
                .lib
                .repos()
                .iter()
                .filter(|r| path_within(&r.path, root))
                .count();
            let p = root.clone();
            if i > 0 {
                folders = folders.child(divider());
            }
            folders = folders.child(
                h_flex()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .child(
                        div()
                            .size(px(36.))
                            .flex_none()
                            .rounded(px(10.))
                            .bg(tint(ACCENT, 0.15))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::FolderHeart, 18., ACCENT)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.short(root)),
                            )
                            .child(div().text_size(px(12.)).text_color(FAINT).child(format!(
                                "{repos} repo{} found inside",
                                if repos == 1 { "" } else { "s" }
                            ))),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("rm-root-{i}")),
                            IconName::Close,
                            "Stop treating this as a project folder",
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_project(&p);
                            cx.notify();
                        })),
                    ),
            );
        }

        let mut scan_roots = v_flex();
        if self.cfg.scan_roots.is_empty() {
            scan_roots = scan_roots.child(
                div()
                    .px_5()
                    .py_6()
                    .text_color(MUTED)
                    .child("Nothing to scan. Add a folder, such as your home folder."),
            );
        }
        for (i, root) in self.cfg.scan_roots.iter().enumerate() {
            let p = root.clone();
            if i > 0 {
                scan_roots = scan_roots.child(divider());
            }
            scan_roots = scan_roots.child(
                h_flex()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .child(
                        div()
                            .size(px(36.))
                            .flex_none()
                            .rounded(px(10.))
                            .bg(tint(BLUE, 0.15))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::Scan, 18., BLUE)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.short(root)),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("rm-scan-{i}")),
                            IconName::Close,
                            "Stop scanning this folder",
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_scan_root(&p);
                            cx.notify();
                        })),
                    ),
            );
        }

        let mut hidden = v_flex();
        for (i, p) in self.cfg.ignored.iter().enumerate() {
            let p2 = p.clone();
            if i > 0 {
                hidden = hidden.child(divider());
            }
            hidden = hidden.child(
                h_flex()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .child(icon(IconName::EyeOff, 16., FAINT))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(MUTED)
                            .child(self.short(p)),
                    )
                    .child(
                        button(
                            SharedString::from(format!("unhide-{i}")),
                            "Unhide",
                            ButtonKind::Ghost,
                        )
                        .tip("Show it again after the next scan")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.unhide(&p2);
                            cx.notify();
                        })),
                    ),
            );
        }

        page_body("projects").child(
            column()
                .child(
                    h_flex()
                        .items_end()
                        .gap_4()
                        .child(div().flex_1().min_w_0().child(page_header(
                            "Project folders",
                            "Where your real work lives, like C:\\Users\\you\\code. Anything inside these is left alone. Project copies found anywhere else are possible leftovers.",
                        )))
                        .child(
                            icon_button("add-root", IconName::FolderPlus, "Add folder", ButtonKind::Primary)
                                .on_click(cx.listener(|this, _, _, cx| this.pick_project_folder(cx))),
                        ),
                )
                .child(card().child(folders))
                .when(self.needs_rescan, |d| {
                    d.child(
                        h_flex()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .rounded(px(10.))
                            .bg(tint(ORANGE, 0.12))
                            .child(icon(IconName::Info, 16., ORANGE))
                            .child(div().flex_1().min_w_0().child("Rescan to apply these changes to the list."))
                            .child(
                                button("p-rescan", "Rescan now", ButtonKind::Secondary).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.start_scan();
                                        this.go(Page::Leftovers);
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                })
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .items_end()
                                .gap_4()
                                .child(div().flex_1().min_w_0().child(page_header(
                                    "Where to look",
                                    "Folders the app searches for project copies (git repos). Your home folder covers Downloads, Desktop, Documents and the Temp folder, where AI tools often put things.",
                                )))
                                .child(
                                    icon_button("add-scan", IconName::FolderPlus, "Add", ButtonKind::Secondary)
                                        .on_click(cx.listener(|this, _, _, cx| this.pick_scan_folder(cx))),
                                ),
                        )
                        .child(card().child(scan_roots)),
                )
                .child(
                    v_flex()
                        .gap_3()
                        .child(page_header(
                            "Hidden items",
                            "Things you chose to stop seeing. The app never touches them.",
                        ))
                        .child(card().child(if self.cfg.ignored.is_empty() {
                            v_flex().child(div().px_5().py_6().text_color(MUTED).child("Nothing hidden."))
                        } else {
                            hidden
                        })),
                ),
        )
    }

    // ---- activity ------------------------------------------------------

    pub fn activity_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut list = v_flex();
        if self.activity.is_empty() {
            list = list.child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .py_10()
                    .child(icon(IconName::Logs, 32., FAINT))
                    .child(div().text_color(MUTED).child(
                        "Nothing yet. Everything the app moves or puts back shows up here.",
                    )),
            );
        }
        for (i, e) in self.activity.iter().enumerate() {
            let (ic, color) = match e.action.as_str() {
                "recycled" => (IconName::Trash, RED),
                "restored" => (IconName::ArchiveRestore, GREEN),
                _ => (IconName::Download, ACCENT),
            };
            if i > 0 {
                list = list.child(divider());
            }
            list = list.child(
                h_flex()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .child(
                        div()
                            .size(px(32.))
                            .flex_none()
                            .rounded(px(8.))
                            .bg(tint(color, 0.14))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(ic, 16., color)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::MEDIUM).child(e.verb()))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(FAINT)
                                    .child(self.short(&e.path)),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(FAINT)
                            .child(format!("{} UTC", e.when_short())),
                    ),
            );
        }
        page_body("activity").child(
            column()
                .child(
                    h_flex()
                        .items_end()
                        .gap_4()
                        .child(div().flex_1().min_w_0().child(page_header(
                            "Activity",
                            "Everything Agent Cleanup moved to the Recycle Bin or put back. Newest first.",
                        )))
                        .child(
                            icon_button("open-log", IconName::FolderOpen, "Open log folder", ButtonKind::Secondary)
                                .tip("Opens the folder with actions.log and config.toml")
                                .on_click(cx.listener(|_, _, _, _| {
                                    if let Some(dir) = Config::data_dir() {
                                        let _ = std::fs::create_dir_all(&dir);
                                        agent_cleanup::actions::open_folder(&dir);
                                    }
                                })),
                        ),
                )
                .child(card().child(list)),
        )
    }

    // ---- settings ------------------------------------------------------

    pub fn settings_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let days = self.cfg.stale_after_days;
        let stepper = h_flex()
            .flex_none()
            .gap_1()
            .child(
                icon_action("days-minus", IconName::Minus, "Fewer days", false).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.cfg.stale_after_days =
                            this.cfg.stale_after_days.saturating_sub(1).max(1);
                        this.save_config();
                        cx.notify();
                    }),
                ),
            )
            .child(
                div()
                    .w(px(72.))
                    .text_center()
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("{days} day{}", if days == 1 { "" } else { "s" })),
            )
            .child(
                icon_action("days-plus", IconName::Plus, "More days", false).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.cfg.stale_after_days = (this.cfg.stale_after_days + 1).min(365);
                        this.save_config();
                        cx.notify();
                    },
                )),
            );

        let update_status: Option<(String, Rgba)> = match self.update_state() {
            UpdateState::Checking => Some(("Checking...".into(), MUTED)),
            UpdateState::UpToDate => Some(("You're on the latest version.".into(), GREEN)),
            UpdateState::Available(u) => {
                Some((format!("Version {} is available.", u.version), ACCENT))
            }
            UpdateState::Failed(m) => Some((m, RED)),
            _ => None,
        };

        page_body("settings").child(
            column()
                .child(page_header("Settings", "How the app decides what counts as a leftover."))
                .child(
                    card()
                        .child(setting_row(
                            "Stale after",
                            "A leftover you haven't touched for this long is a cleanup candidate. Anything newer shows as Recent in case you still need it.",
                            stepper,
                        ))
                        .child(divider())
                        .child(setting_row(
                            "Only check Temp inside AppData",
                            "AppData is huge and mostly app settings. Leaving this on makes scans much faster while still checking AppData\\Local\\Temp, where throwaway clones end up.",
                            Switch::new("appdata").checked(self.cfg.appdata_temp_only).on_click(cx.listener(
                                |this, checked: &bool, _, cx| {
                                    this.cfg.appdata_temp_only = *checked;
                                    this.save_config();
                                    this.needs_rescan = true;
                                    cx.notify();
                                },
                            )),
                        ))
                        .child(divider())
                        .child(self.weekly_row(cx))
                        .child(divider())
                        .child(setting_row(
                            "Check for updates on startup",
                            "Looks for a new release on GitHub when the app opens. Nothing is downloaded without asking.",
                            Switch::new("updates").checked(self.cfg.check_for_updates).on_click(cx.listener(
                                |this, checked: &bool, _, cx| {
                                    this.cfg.check_for_updates = *checked;
                                    this.save_config();
                                    cx.notify();
                                },
                            )),
                        )),
                )
                .child(
                    card()
                        .child(
                            h_flex()
                                .gap_4()
                                .px_5()
                                .py_4()
                                .child(
                                    v_flex()
                                        .flex_1().min_w_0()
                                        .gap_1()
                                        .child(div().font_weight(FontWeight::MEDIUM).child(format!("Agent Cleanup {}", update::current_version())))
                                        .children(update_status.map(|(t, c)| div().text_size(px(13.)).text_color(c).child(t))),
                                )
                                .child(
                                    icon_button("check-upd", IconName::RefreshCw, "Check for updates", ButtonKind::Secondary)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.check_for_updates();
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(divider())
                        .child(setting_row(
                            "Settings and action log",
                            "config.toml holds these settings. actions.log records everything the app moved.",
                            button("open-cfg", "Open folder", ButtonKind::Secondary).on_click(cx.listener(|_, _, _, _| {
                                if let Some(dir) = Config::data_dir() {
                                    let _ = std::fs::create_dir_all(&dir);
                                    agent_cleanup::actions::open_folder(&dir);
                                }
                            })),
                        ))
                        .child(divider())
                        .child(setting_row(
                            "Remove all app data",
                            "Deletes settings, the last scan, the action log and the weekly check task, then closes the app. Only the .exe is left, which you can delete yourself.",
                            button("wipe", "Remove...", ButtonKind::Danger).on_click(cx.listener(|this, _, _, cx| {
                                this.weekly = Some(agent_cleanup::schedule::status());
                                this.confirm_wipe = true;
                                cx.notify();
                            })),
                        )),
                ),
        )
    }

    /// Confirms "Remove all app data", listing exactly what goes.
    pub fn wipe_dialog(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        use agent_cleanup::schedule::Status;
        if !self.confirm_wipe {
            return None;
        }
        let dir = agent_cleanup::footprint::data_dir();
        let mut rows: Vec<(IconName, String, String)> = dir
            .as_deref()
            .map(agent_cleanup::footprint::data_files)
            .unwrap_or_default()
            .into_iter()
            .map(|(p, size)| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                (IconName::File, name, human_size(size))
            })
            .collect();
        if matches!(self.weekly, Some(Status::On | Status::OtherCopy)) {
            rows.push((
                IconName::Clock,
                "Weekly check (Task Scheduler)".into(),
                String::new(),
            ));
        }
        let mut list = v_flex().gap_1();
        if rows.is_empty() {
            list = list.child(div().text_color(MUTED).child("Nothing is saved yet."));
        }
        for (ic, name, size) in rows {
            list = list.child(
                h_flex()
                    .gap_2()
                    .child(icon(ic, 14., MUTED))
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(div().text_size(px(12.)).text_color(FAINT).child(size)),
            );
        }
        let folder = dir.map(|d| self.short(&d)).unwrap_or_default();
        Some(
            div()
                .id("wipe-modal")
                .absolute()
                .inset_0()
                .occlude()
                .bg(tint(Rgba { r: 0., g: 0., b: 0., a: 1. }, 0.6))
                .flex()
                .items_center()
                .justify_center()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_wipe = false;
                    cx.notify();
                }))
                .child(
                    v_flex()
                        .id("wipe-card")
                        .w(px(500.))
                        .max_h(relative(0.85))
                        .overflow_y_scroll()
                        .p_6()
                        .gap_4()
                        .rounded(px(16.))
                        .bg(SIDEBAR)
                        .border_1()
                        .border_color(BORDER)
                        .shadow_2xl()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .size(px(44.))
                                .rounded(px(12.))
                                .bg(tint(RED, 0.16))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(IconName::Trash, 22., RED)),
                        )
                        .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Remove all of Agent Cleanup's data?"))
                        .child(div().text_color(MUTED).child(format!(
                            "These are deleted for good, not moved to the Recycle Bin, along with the {folder} folder. Then the app closes. If you open it again it starts fresh, like the first time."
                        )))
                        .child(card().p_3().child(list))
                        .child(
                            h_flex()
                                .gap_2()
                                .items_start()
                                .text_size(px(13.))
                                .text_color(MUTED)
                                .child(icon(IconName::ShieldCheck, 15., GREEN))
                                .child(div().flex_1().min_w_0().child(
                                    "Your repos and files aren't touched. Anything the app moved to the Recycle Bin stays there until you empty it. To uninstall completely, delete agent-cleanup.exe afterwards.",
                                )),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .pt_2()
                                .child(button("wipe-cancel", "Cancel", ButtonKind::Ghost).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.confirm_wipe = false;
                                        cx.notify();
                                    },
                                )))
                                .child(
                                    icon_button("wipe-go", IconName::Trash, "Remove and close", ButtonKind::Danger)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.remove_all_data();
                                            cx.notify();
                                        })),
                                ),
                        ),
                ),
        )
    }

    fn weekly_row(&self, cx: &mut Context<Self>) -> Div {
        use agent_cleanup::schedule::Status;
        let status = self.weekly.clone().unwrap_or(Status::Unknown);
        let note: Option<(&str, Rgba)> = match status {
            Status::OtherCopy => Some((
                "It's set up for a different copy of the app. Turn it off and on again to use this one.",
                ORANGE,
            )),
            Status::Unknown => Some(("Couldn't ask Task Scheduler about it.", MUTED)),
            _ => None,
        };
        let on = matches!(status, Status::On | Status::OtherCopy);
        h_flex()
            .gap_4()
            .px_5()
            .py_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().font_weight(FontWeight::MEDIUM).child("Weekly check"))
                    .child(div().text_size(px(13.)).text_color(MUTED).child(
                        "Every Monday (or the next time your PC is on) the app scans quietly and shows a notification if there's 100 MB or an old repo to clean. Nothing stays running and nothing is moved.",
                    ))
                    .children(note.map(|(t, c)| div().text_size(px(13.)).text_color(c).child(t))),
            )
            .child(
                button("weekly-now", "Run now", ButtonKind::Secondary)
                    .tip("Run the check now, in the background")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.run_check_now();
                        cx.notify();
                    })),
            )
            .child(Switch::new("weekly").checked(on).on_click(cx.listener(
                |this, checked: &bool, _, cx| {
                    this.set_weekly(*checked);
                    cx.notify();
                },
            )))
    }

    // ---- help ----------------------------------------------------------

    pub fn help_page(&self, _cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let block = |ic: IconName, color: Rgba, title: &'static str, text: &'static str| {
            card()
                .p_5()
                .flex()
                .gap_4()
                .child(
                    div()
                        .size(px(36.))
                        .flex_none()
                        .rounded(px(10.))
                        .bg(tint(color, 0.15))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(ic, 18., color)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_size(px(13.)).text_color(MUTED).child(text)),
                )
        };
        let status = |v: Verdict| {
            h_flex()
                .gap_3()
                .items_start()
                .child(div().w(px(150.)).flex_none().flex().child(status_pill(v)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(MUTED)
                        .child(v.explanation()),
                )
        };
        let term = |word: &'static str, meaning: &'static str| {
            h_flex()
                .gap_3()
                .items_start()
                .child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .child(word),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(MUTED)
                        .child(meaning),
                )
        };
        let mut words = v_flex().gap_3();
        for (w, m) in GLOSSARY {
            words = words.child(term(w, m));
        }
        page_body("help").child(
            column()
                .child(page_header(
                    "How this works",
                    "AI coding tools like Claude Code and Codex often download copies of projects or make scratch files outside the folder you're working in. This app finds them and helps you clean them up safely. You don't need to be a developer to use it.",
                ))
                .child(block(IconName::FolderHeart, ACCENT, "1. Tell it where your projects are", "Add the folders that hold your real work on the Projects page. Anything inside them is yours and won't be flagged."))
                .child(block(IconName::Scan, BLUE, "2. It looks for project copies", "It searches your home folder for git repos: project folders whose history is tracked by git. That's what AI tools create when they download or start a project. Results are saved, so it opens instantly next time."))
                .child(block(IconName::GitBranch, ORANGE, "3. It checks what's unsaved", "For each one it checks, without changing anything, whether it holds work that isn't saved anywhere else: edits that were never saved into its history, saved versions that were never uploaded, a project that isn't online at all, or private files git leaves out on purpose (like a .env file with passwords, or a local database), which are never uploaded. Downloaded packages and build output don't count: a command brings them back."))
                .child(
                    card()
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("What the statuses mean"))
                        .child(status(Verdict::SafeToRemove))
                        .child(status(Verdict::UnsavedWork))
                        .child(status(Verdict::Unreadable))
                        .child(status(Verdict::StrayActive))
                        .child(status(Verdict::InProject)),
                )
                .child(
                    card()
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Words you'll see"))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(FAINT)
                                .child("You don't need to know any of this to use the app, but it helps to know what the labels mean."),
                        )
                        .child(words),
                )
                .child(block(IconName::Footprints, GREEN, "Agent trail", "Claude Code and Codex keep a log of every session. The Agent trail page reads the last 90 days of them (read-only) and lists files, downloads and clones an agent made outside the folder it was working in. Files an agent only changed, like your .bashrc, are marked Check first: they were there before, so they're probably yours."))
                .child(block(IconName::Download, ACCENT, "Downloads", "The Downloads page guesses which project each recent download was for: one an agent fetched, a repo you committed in, or a project an agent was working in, close to when it arrived. If two projects are equally likely it doesn't guess. Move into project puts the file in that folder and Undo moves it back."))
                .child(block(IconName::Clock, GREEN, "Weekly check", "Turn it on in Settings and Windows runs a quiet scan once a week. If there's at least 100 MB or an old repo to clean, a notification tells you, and clicking it opens the app. The check itself never moves anything."))
                .child(block(IconName::Ghost, ORANGE, "Agent data", "Claude Code and Codex keep every session's history, and it stays after you delete the project. The Agent data page lists history whose project folder no longer exists. Data on a drive that isn't connected is left alone, and anything used in the last week needs a tick first in case the project only moved."))
                .child(block(IconName::Copy, BLUE, "Duplicates", "The same repo cloned into more than one folder, matched by its remote. One copy is kept (the one in a project folder, or the most recently used; pin another to change it) and Clean up recycles the extra copies that are fully pushed."))
                .child(block(IconName::Package, BLUE, "Build junk", "Dependency and build folders like node_modules, target and .venv take a lot of space and one command recreates them. The Build junk page lists them only when git ignores them and the file that rebuilds them is right there. Clean up only takes them from projects you haven't touched in a while."))
                .child(block(IconName::CircleArrowUp, ACCENT, "Updates", "For projects in your project folders, the Updates page asks npm, cargo and pip what's outdated. Update opens a terminal running the update so you can watch it, and only when git shows no uncommitted changes, so you can see and undo what it changed."))
                .child(block(IconName::Bot, ACCENT, "Hand it to your agent", "Most pages have a Copy prompt for your agent button. It copies a ready-to-paste request for Claude Code or Codex that lists those items and says what to do: save unsaved work, explain what something is, or move a file into its project. The prompt tells the agent not to delete anything, so the removing still happens here, through the Recycle Bin. It only contains the paths and notes you can see on the page."))
                .child(block(IconName::ShieldCheck, GREEN, "Is it safe?", "Nothing is permanently deleted: Recycle moves folders to the Recycle Bin. You confirm every removal and can undo it right after. One-click buttons only ever take things the app is sure about. If you pick a mix, the main button recycles just the safe ones; anything with unsaved work or private-looking files (like a .env with passwords, a key or a database) needs an extra tick. Every move is written to the Activity page.")),
        )
    }

    // ---- overlays ------------------------------------------------------

    pub fn update_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let bar = |color: Rgba| {
            h_flex()
                .flex_none()
                .gap_3()
                .px_5()
                .py_2()
                .bg(tint(color, 0.16))
                .border_b_1()
                .border_color(tint(color, 0.35))
        };
        match self.update_state() {
            UpdateState::Available(u) if !self.update_dismissed => {
                let u2 = u.clone();
                let page = u.page_url.clone();
                Some(
                    bar(ACCENT)
                        .child(icon(IconName::Sparkles, 16., ACCENT))
                        .child(div().flex_1().min_w_0().child(format!(
                            "Version {} is available. You have {}.",
                            u.version,
                            update::current_version()
                        )))
                        .child(button("upd-notes", "What's new", ButtonKind::Ghost).on_click(cx.listener(
                            move |_, _, _, cx| cx.open_url(&page),
                        )))
                        .child(button("upd-later", "Not now", ButtonKind::Ghost).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.update_dismissed = true;
                                cx.notify();
                            },
                        )))
                        .child(
                            icon_button("upd-now", IconName::Download, "Update now", ButtonKind::Primary)
                                .tip("Download the new version and swap it in. Takes a few seconds.")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.install_update(u2.clone(), cx);
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                )
            }
            UpdateState::Downloading(done, total) => {
                let frac = total.map(|t| done as f32 / t.max(1) as f32).unwrap_or(0.).clamp(0., 1.);
                Some(
                    bar(ACCENT)
                        .child(icon(IconName::LoaderCircle, 16., ACCENT))
                        .child(div().child(format!("Downloading update... {}", human_size(done))))
                        .child(
                            div()
                                .flex_1().min_w_0()
                                .h(px(6.))
                                .rounded_full()
                                .bg(CARD)
                                .child(div().h_full().rounded_full().bg(ACCENT).w(relative(frac))),
                        )
                        .into_any_element(),
                )
            }
            UpdateState::Installed(v) => Some(
                bar(GREEN)
                    .child(icon(IconName::CircleCheck, 16., GREEN))
                    .child(div().flex_1().min_w_0().child(format!(
                        "Updated to version {v}. Restart now, or it'll be used next time you open the app."
                    )))
                    .child(
                        button("restart", "Restart now", ButtonKind::Primary)
                            .on_click(cx.listener(|_, _, _, _| update::relaunch())),
                    )
                    .into_any_element(),
            ),
            UpdateState::Failed(msg) if !msg.starts_with("Couldn't check") && !self.update_dismissed => {
                Some(
                    bar(RED)
                        .child(icon(IconName::TriangleAlert, 16., RED))
                        .child(div().flex_1().min_w_0().child(msg))
                        .child(button("upd-manual", "Download it manually", ButtonKind::Ghost).on_click(
                            cx.listener(|_, _, _, cx| {
                                cx.open_url(&format!("https://github.com/{}/releases/latest", update::REPO))
                            }),
                        ))
                        .child(icon_action("upd-x", IconName::Close, "Dismiss", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.update_dismissed = true;
                                cx.notify();
                            },
                        )))
                        .into_any_element(),
                )
            }
            _ => None,
        }
    }

    pub fn toast(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let n = self.notice.as_ref()?;
        let color = if n.is_error { RED } else { GREEN };
        let can_undo = !n.undo.is_empty()
            || !n.undo_trail.is_empty()
            || !n.undo_junk.is_empty()
            || !n.undo_downloads.is_empty()
            || !n.undo_agentdata.is_empty();
        Some(
            div()
                .absolute()
                .bottom(px(20.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    h_flex()
                        .id("toast")
                        .occlude()
                        .max_w(px(640.))
                        .gap_3()
                        .pl_4()
                        .pr_2()
                        .py_2()
                        .rounded(px(12.))
                        .bg(ELEVATED)
                        .border_1()
                        .border_color(BORDER)
                        .shadow_lg()
                        .child(icon(
                            if n.is_error {
                                IconName::TriangleAlert
                            } else {
                                IconName::CircleCheck
                            },
                            18.,
                            color,
                        ))
                        .child(div().flex_1().min_w_0().child(n.text.clone()))
                        .when(can_undo, |d| {
                            d.child(
                                icon_button("undo", IconName::Undo2, "Undo", ButtonKind::Secondary)
                                    .tip("Put it back where it was")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.undo();
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            icon_action("toast-x", IconName::Close, "Dismiss", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.notice = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                ),
        )
    }

    pub fn confirm_dialog(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let c = self.confirm.as_ref()?;
        let now = SystemTime::now();
        // One row per thing to recycle, whatever page it came from.
        struct Row {
            path: PathBuf,
            name: String,
            size: Option<u64>,
            icon: IconName,
            color: Rgba,
            risk: Option<String>,
        }
        let rows: Vec<Row> = if c.what == ConfirmFor::Junk {
            let days = self.cfg.stale_after_days;
            c.paths
                .iter()
                .filter_map(|p| self.junk_item(p))
                .map(|it| {
                    let idle = it.is_idle(days, now);
                    Row {
                        path: it.path.clone(),
                        name: format!("{} in {}", it.name(), it.repo_name()),
                        size: Some(it.size_bytes),
                        icon: if idle {
                            IconName::Package
                        } else {
                            IconName::Clock
                        },
                        color: if idle { GREEN } else { ORANGE },
                        risk: (!idle).then(|| {
                            let when = it
                                .repo_activity
                                .map(|t| format!("worked on {}", relative_time(t, now)))
                                .unwrap_or_else(|| "no idea when it was last used".into());
                            format!("{when}; you'd need to run {} again", it.kind.rebuild)
                        }),
                    }
                })
                .collect()
        } else if c.what == ConfirmFor::Duplicates {
            c.paths
                .iter()
                .filter_map(|p| self.lib.get(p))
                .map(|r| {
                    // Another copy is kept, so only work that exists in
                    // this copy alone is a risk.
                    let risk = if r.project_root.is_some() {
                        Some("inside one of your project folders".to_string())
                    } else {
                        match &r.git {
                            None => Some("the app couldn't look inside it".to_string()),
                            Some(g) if g.has_local_work() => {
                                Some(g.local_work_reasons().join(", "))
                            }
                            Some(_) => None,
                        }
                    };
                    Row {
                        path: r.path.clone(),
                        name: self.short(&r.path),
                        size: r.size_bytes,
                        icon: if risk.is_some() {
                            IconName::TriangleAlert
                        } else {
                            IconName::Copy
                        },
                        color: if risk.is_some() { ORANGE } else { GREEN },
                        risk,
                    }
                })
                .collect()
        } else if c.what == ConfirmFor::AgentData {
            c.paths
                .iter()
                .filter_map(|p| self.orphan(p))
                .map(|o| {
                    let risk = super::agentdata::recent_warning(o, now);
                    Row {
                        path: o.key().to_path_buf(),
                        name: format!("{} ({})", o.project_name(), o.agent.label()),
                        size: Some(o.size_bytes),
                        icon: if risk.is_some() {
                            IconName::TriangleAlert
                        } else {
                            IconName::Ghost
                        },
                        color: if risk.is_some() { ORANGE } else { GREEN },
                        risk,
                    }
                })
                .collect()
        } else if c.what == ConfirmFor::Downloads {
            c.paths
                .iter()
                .filter_map(|p| self.download_item(p))
                .map(|d| {
                    let private = d.looks_private();
                    Row {
                        path: d.path.clone(),
                        name: d.name(),
                        size: Some(d.size_bytes),
                        icon: if private { IconName::TriangleAlert } else { download_icon(d) },
                        color: if private { ORANGE } else { MUTED },
                        risk: private.then(|| {
                            "looks like a private file (passwords, a key or a database) that may be the only copy".to_string()
                        }),
                    }
                })
                .collect()
        } else if c.what == ConfirmFor::Trail {
            c.paths
                .iter()
                .filter_map(|p| self.trail_item(p))
                .map(|it| {
                    let w = it.warnings();
                    Row {
                        path: it.path.clone(),
                        name: it.name(),
                        size: Some(it.size_bytes),
                        icon: if w.is_empty() {
                            trail_icon(it.kind)
                        } else {
                            IconName::TriangleAlert
                        },
                        color: if w.is_empty() { GREEN } else { ORANGE },
                        risk: (!w.is_empty()).then(|| w.join(", ")),
                    }
                })
                .collect()
        } else {
            c.paths
                .iter()
                .filter_map(|p| self.lib.get(p))
                .map(|r| {
                    let v = r.verdict(&self.cfg, now);
                    let risk = (v != Verdict::SafeToRemove).then(|| {
                        if !r.is_stray() {
                            "inside one of your project folders".to_string()
                        } else {
                            match &r.git {
                                None => {
                                    let mut s = match &r.error {
                                        Some(e) => format!("the app couldn't look inside it: {e}"),
                                        None => "the app couldn't look inside it".to_string(),
                                    };
                                    if let Some(n) = r.private_files_note() {
                                        s = format!("{s}, and {n}");
                                    }
                                    s
                                }
                                Some(g) if g.has_local_work() => g.local_work_reasons().join(", "),
                                // Everything is pushed; it was just used recently.
                                Some(_) => r
                                    .last_activity
                                    .map(|t| format!("used {}", relative_time(t, now)))
                                    .unwrap_or_else(|| "used recently".into()),
                            }
                        }
                    });
                    Row {
                        path: r.path.clone(),
                        name: r.name(),
                        size: r.size_bytes,
                        icon: verdict_icon(v),
                        color: verdict_color(v),
                        risk,
                    }
                })
                .collect()
        };
        let total: u64 = rows.iter().filter_map(|r| r.size).sum();
        let risky: Vec<&Row> = rows.iter().filter(|r| r.risk.is_some()).collect();
        let needs_ack = !risky.is_empty();
        let can_go = !needs_ack || c.ack;
        let noun = match c.what {
            ConfirmFor::Repos => "repos",
            ConfirmFor::Trail => "items",
            ConfirmFor::Junk => "folders",
            ConfirmFor::Downloads => "downloads",
            ConfirmFor::AgentData => "session histories",
            ConfirmFor::Duplicates => "extra copies",
        };
        let title = if rows.len() == 1 {
            format!("Move \"{}\" to the Recycle Bin?", rows[0].name)
        } else {
            format!("Move {} {noun} to the Recycle Bin?", rows.len())
        };
        let (safe_text, warn_title, ack_text) = if c.what == ConfirmFor::Junk {
            (
                "Each one is rebuilt by a single command, like npm install or cargo build. Empty the Recycle Bin afterwards to get the space back.",
                "These projects are still in use:",
                "I'll rebuild these when I need them",
            )
        } else if c.what == ConfirmFor::Duplicates {
            (
                "Another copy of each is kept, and everything in these is saved online.",
                "These have something the kept copy may not:",
                "I've checked these and they can go",
            )
        } else if c.what == ConfirmFor::AgentData {
            (
                "Their projects are gone, so these sessions can't be resumed anyway. Empty the Recycle Bin afterwards to get the space back.",
                "Used recently, so the project may have just moved:",
                "I don't need these sessions",
            )
        } else if c.what == ConfirmFor::Downloads {
            (
                "Nothing outside your Downloads folder uses these. Empty the Recycle Bin afterwards to get the space back.",
                "These look private, and may be the only copy:",
                "I've saved these somewhere else, or don't need them",
            )
        } else if c.what == ConfirmFor::Trail {
            (
                "An AI agent made all of these outside its project, and none hold unsaved work.",
                "Take a look at these first:",
                "I've checked these and they can go",
            )
        } else {
            (
                "Everything is saved online, so it can be downloaded again any time.",
                "Some work here isn't saved anywhere else:",
                "I understand this work only exists on this PC",
            )
        };

        let mut list = v_flex().gap_1();
        for r in rows.iter().take(8) {
            list = list.child(
                h_flex()
                    .gap_2()
                    .child(icon(r.icon, 14., r.color))
                    .child(div().flex_1().min_w_0().truncate().child(r.name.clone()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(FAINT)
                            .child(r.size.map(human_size).unwrap_or_default()),
                    ),
            );
        }
        if rows.len() > 8 {
            list = list.child(
                div()
                    .text_size(px(12.))
                    .text_color(FAINT)
                    .child(format!("...and {} more", rows.len() - 8)),
            );
        }

        let mut warn = v_flex().gap_1();
        for r in risky.iter().take(6) {
            warn = warn.child(div().text_size(px(13.)).child(format!(
                "{}: {}",
                r.name,
                r.risk.clone().unwrap_or_default()
            )));
        }
        if risky.len() > 6 {
            warn = warn.child(
                div()
                    .text_size(px(13.))
                    .child(format!("...and {} more", risky.len() - 6)),
            );
        }

        let ack = c.ack;
        // When only some need a look, the main button takes just the safe
        // ones, so a quick click never removes anything risky.
        let safe_paths: Vec<PathBuf> = rows
            .iter()
            .filter(|r| r.risk.is_none())
            .map(|r| r.path.clone())
            .collect();
        let mixed = needs_ack && !safe_paths.is_empty();
        let what = c.what;
        let go_label = if mixed {
            format!("Recycle all {}", rows.len())
        } else {
            "Move to Recycle Bin".to_string()
        };
        let go_btn = icon_button(
            "confirm-go",
            IconName::Trash,
            go_label,
            if needs_ack {
                ButtonKind::Danger
            } else {
                ButtonKind::Primary
            },
        );
        let go_btn = if can_go {
            go_btn.on_click(cx.listener(|this, _, _, cx| {
                if let Some(c) = this.confirm.take() {
                    this.run_recycle(c.what, &c.paths);
                }
                cx.notify();
            }))
        } else if mixed {
            disabled(go_btn).tip("Tick the box above to include the ones that need a look")
        } else {
            disabled(go_btn).tip("Tick the box above first")
        };
        let safe_btn = mixed.then(|| {
            let n = safe_paths.len();
            icon_button(
                "confirm-safe",
                IconName::ShieldCheck,
                if n == 1 {
                    "Recycle only the safe one".to_string()
                } else {
                    format!("Recycle only the {n} safe ones")
                },
                ButtonKind::Primary,
            )
            .tip("Leaves the ones marked above where they are")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.confirm = None;
                this.run_recycle(what, &safe_paths);
                cx.notify();
            }))
        });

        Some(
            div()
                .id("modal")
                .absolute()
                .inset_0()
                .occlude()
                .bg(tint(Rgba { r: 0., g: 0., b: 0., a: 1. }, 0.6))
                .flex()
                .items_center()
                .justify_center()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm = None;
                    cx.notify();
                }))
                .child(
                    v_flex()
                        .id("modal-card")
                        .w(px(500.))
                        .max_h(relative(0.85))
                        .overflow_y_scroll()
                        .p_6()
                        .gap_4()
                        .rounded(px(16.))
                        .bg(SIDEBAR)
                        .border_1()
                        .border_color(BORDER)
                        .shadow_2xl()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .size(px(44.))
                                .rounded(px(12.))
                                .bg(tint(if needs_ack { ORANGE } else { ACCENT }, 0.16))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(
                                    if needs_ack { IconName::TriangleAlert } else { IconName::Trash },
                                    22.,
                                    if needs_ack { ORANGE } else { ACCENT },
                                )),
                        )
                        .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_color(MUTED).child(format!(
                            "Frees about {} once the Recycle Bin is emptied. You can undo this right after, or restore from the Recycle Bin later.",
                            human_size(total)
                        )))
                        .child(card().p_3().child(list))
                        .when(!needs_ack, |d| {
                            d.child(
                                h_flex()
                                    .gap_2()
                                    .text_color(GREEN)
                                    .text_size(px(13.))
                                    .child(icon(IconName::ShieldCheck, 15., GREEN))
                                    .child(div().flex_1().min_w_0().child(safe_text)),
                            )
                        })
                        .when(needs_ack, |d| {
                            d.child(
                                v_flex()
                                    .gap_2()
                                    .p_3()
                                    .rounded(px(10.))
                                    .bg(tint(ORANGE, 0.12))
                                    .border_1()
                                    .border_color(tint(ORANGE, 0.35))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(ORANGE)
                                            .child(warn_title),
                                    )
                                    .child(warn),
                            )
                            .child(
                                h_flex()
                                    .id("ack")
                                    .gap_2p5()
                                    .cursor_pointer()
                                    .child(
                                        div()
                                            .size(px(18.))
                                            .rounded(px(5.))
                                            .border_1()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(ack, |d| d.bg(ORANGE).border_color(ORANGE).child(icon(IconName::Check, 13., hex_white())))
                                            .when(!ack, |d| d.border_color(MUTED)),
                                    )
                                    .child(ack_text)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(c) = &mut this.confirm {
                                            c.ack = !c.ack;
                                        }
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .pt_2()
                                .child(button("confirm-cancel", "Cancel", ButtonKind::Ghost).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.confirm = None;
                                        cx.notify();
                                    },
                                )))
                                .child(go_btn)
                                .children(safe_btn),
                        ),
                ),
        )
    }
}

/// Plain-language meanings for the words the app can't avoid.
const GLOSSARY: [(&str, &str); 11] = [
    (
        "Repo",
        "Short for repository: a project folder whose history is tracked by git. Most of what this app finds are repos.",
    ),
    (
        "Git",
        "The tool developers and AI agents use to save versions of a project. This app only reads it and never changes anything with it.",
    ),
    (
        "Commit",
        "A saved version in a repo's history. Edits that aren't committed aren't saved in the history yet.",
    ),
    (
        "Push, online copy",
        "Pushing uploads saved versions to an online copy of the repo, usually on GitHub. Work that's pushed is safe even if this folder is removed.",
    ),
    (
        "Clone",
        "A copy of a project downloaded from GitHub or a similar site. AI agents often clone projects to try them out, then leave them behind.",
    ),
    (
        "Stash",
        "Changes set aside for later inside a repo. They live only in that folder.",
    ),
    (
        "Ignored files (.gitignore)",
        "Files a repo tells git to leave out on purpose, listed in its .gitignore file. Some are clutter a command recreates, like node_modules. Others are private, like a .env file with passwords or a local database. Those are never uploaded, so the folder may hold the only copy.",
    ),
    (
        "Agent, session",
        "An agent is an AI coding tool like Claude Code or Codex. Each conversation with one is a session, and its history is kept on your PC.",
    ),
    (
        "Build folders",
        "Libraries a project downloads and files it builds, like node_modules. They take a lot of space and one command recreates them.",
    ),
    (
        "Temp folder",
        "A Windows folder for short-lived files (AppData\\Local\\Temp). Anything an agent put there was meant to be thrown away.",
    ),
    (
        "Recycle Bin",
        "Everything this app removes goes there first. Use Undo, or restore it from the Recycle Bin, if you change your mind.",
    ),
];
