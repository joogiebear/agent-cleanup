//! The right-hand panel for the selected repo: what it is, what's unsaved
//! and what to do next.

use super::app::AgentCleanup;
use super::theme::*;
use super::widgets::*;
use agent_cleanup::model::{Repo, Verdict, human_size, relative_time};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::SystemTime;

impl AgentCleanup {
    pub fn details_panel(
        &self,
        r: Repo,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let now = SystemTime::now();
        let v = r.verdict(&self.cfg, now);
        let color = verdict_color(v);
        let reasons = r
            .git
            .as_ref()
            .map(|g| g.local_work_reasons())
            .unwrap_or_default();
        let steps = r.next_steps(&self.cfg, now);
        let path = r.path.clone();

        let fact = |label: &'static str, value: String| {
            h_flex()
                .gap_3()
                .py_1p5()
                .child(div().w(px(84.)).flex_none().text_color(FAINT).child(label))
                .child(div().flex_1().min_w_0().truncate().child(value))
        };
        let git = r.git.as_ref();

        let mut actions = v_flex().gap_2();
        let removable = v != Verdict::InProject && v != Verdict::Checking;
        if removable {
            let p = path.clone();
            actions = actions.child(
                icon_button(
                    "d-recycle",
                    IconName::Trash,
                    "Move to Recycle Bin",
                    if v == Verdict::SafeToRemove {
                        ButtonKind::Primary
                    } else {
                        ButtonKind::Danger
                    },
                )
                .w_full()
                .tip("You'll confirm first, and you can undo right after.")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_recycle(vec![p.clone()]);
                    cx.notify();
                })),
            );
        }
        let p = path.clone();
        let p2 = path.clone();
        actions = actions.child(
            h_flex()
                .gap_2()
                .child(
                    icon_button(
                        "d-open",
                        IconName::FolderOpen,
                        "Open folder",
                        ButtonKind::Secondary,
                    )
                    .flex_1()
                    .on_click(
                        cx.listener(move |_, _, _, _| agent_cleanup::actions::open_folder(&p)),
                    ),
                )
                .child(
                    icon_button(
                        "d-term",
                        IconName::SquareTerminal,
                        "Terminal",
                        ButtonKind::Secondary,
                    )
                    .flex_1()
                    .tip("Open a command window in this folder, for running git commands")
                    .on_click(
                        cx.listener(move |_, _, _, _| agent_cleanup::actions::open_terminal(&p2)),
                    ),
                ),
        );
        if r.is_stray() {
            let p = path.clone();
            let p2 = path.clone();
            actions = actions.child(
                h_flex()
                    .gap_2()
                    .child(
                        icon_button(
                            "d-project",
                            IconName::FolderHeart,
                            "It's a project",
                            ButtonKind::Ghost,
                        )
                        .flex_1()
                        .tip("Treat this folder as your real work. The app won't flag it again.")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_project(p.clone());
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_button("d-hide", IconName::EyeOff, "Hide", ButtonKind::Ghost)
                            .flex_1()
                            .tip("Stop showing it here. It isn't touched, and you can unhide it on the Projects page.")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.hide(p2.clone());
                                cx.notify();
                            })),
                    ),
            );
        }

        v_flex()
            .id("details")
            .w(px(340.))
            .h_full()
            .flex_none()
            .bg(SIDEBAR)
            .border_l_1()
            .border_color(RAIL)
            .overflow_y_scroll()
            .child(
                h_flex()
                    .h(px(56.))
                    .px_4()
                    .border_b_1()
                    .border_color(RAIL)
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Details"),
                    )
                    .child(
                        icon_action("d-close", IconName::Close, "Close", false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.focused = None;
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .p_5()
                    .gap_5()
                    .child(
                        v_flex()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(52.))
                                    .rounded(px(14.))
                                    .bg(tint(color, 0.15))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(verdict_icon(v), 26., color)),
                            )
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(r.name()),
                            )
                            .child(
                                h_flex()
                                    .id("d-path")
                                    .gap_1p5()
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .text_color(FAINT)
                                    .hover(|s| s.text_color(MUTED))
                                    .child(div().min_w_0().truncate().child(self.short(&path)))
                                    .child(icon(IconName::Copy, 12., FAINT))
                                    .tip("Copy the full path")
                                    .on_click(cx.listener({
                                        let p = path.clone();
                                        move |this, _, _, cx| this.copy_path(&p, cx)
                                    })),
                            )
                            .child(
                                v_flex()
                                    .gap_2()
                                    .p_3()
                                    .rounded(px(10.))
                                    .bg(tint(color, 0.08))
                                    .child(status_pill(v))
                                    .child(
                                        div()
                                            .text_color(MUTED)
                                            .text_size(px(13.))
                                            .child(v.explanation()),
                                    ),
                            ),
                    )
                    .when(!reasons.is_empty(), |d| {
                        let mut list = v_flex().gap_1p5();
                        for reason in &reasons {
                            list = list.child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(icon(IconName::TriangleAlert, 14., ORANGE))
                                    .child(div().flex_1().min_w_0().child(capitalize(reason))),
                            );
                        }
                        d.child(
                            v_flex()
                                .gap_2()
                                .child(section_label("Only on this PC"))
                                .child(list),
                        )
                    })
                    .when(!steps.is_empty(), |d| {
                        let mut list = v_flex().gap_2();
                        for (i, step) in steps.iter().enumerate() {
                            list = list.child(
                                h_flex()
                                    .w_full()
                                    .items_start()
                                    .gap_2p5()
                                    .child(
                                        div()
                                            .size(px(20.))
                                            .flex_none()
                                            .rounded_full()
                                            .bg(ELEVATED)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child((i + 1).to_string()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(13.))
                                            .text_color(MUTED)
                                            .child(step.clone()),
                                    ),
                            );
                        }
                        d.child(
                            v_flex()
                                .gap_2()
                                .child(section_label("What to do"))
                                .child(list),
                        )
                    })
                    .child(
                        v_flex()
                            .gap_0()
                            .child(section_label("Facts"))
                            .child(fact(
                                "Size",
                                r.size_bytes
                                    .map(human_size)
                                    .unwrap_or_else(|| "Measuring...".into()),
                            ))
                            .child(fact(
                                "Last used",
                                r.last_activity
                                    .map(|t| relative_time(t, now))
                                    .unwrap_or_else(|| "Unknown".into()),
                            ))
                            .child(fact(
                                "Branch",
                                git.and_then(|g| g.branch.clone())
                                    .unwrap_or_else(|| "None".into()),
                            ))
                            .child(fact(
                                "Online copy",
                                git.and_then(|g| g.remote_url.clone())
                                    .unwrap_or_else(|| "None, only on this PC".into()),
                            ))
                            .when_some(r.project_root.clone(), |d, root: std::path::PathBuf| {
                                d.child(fact("Project", self.short(&root)))
                            }),
                    )
                    .child(actions),
            )
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
