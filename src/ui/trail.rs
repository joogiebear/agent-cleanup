//! The Agent trail page: things Claude Code and Codex made outside the
//! project they were working in, read from their session logs.

use super::app::AgentCleanup;
use super::theme::*;
use super::widgets::*;
use agent_cleanup::model::{human_size, relative_time};
use agent_cleanup::trail::{self, Item, Kind};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::SystemTime;

pub fn trail_icon(kind: Kind) -> IconName {
    match kind {
        Kind::Clone => IconName::FolderGit2,
        Kind::Download => IconName::Download,
        Kind::File => IconName::FileText,
        Kind::Edited => IconName::FilePen,
    }
}

fn kind_color(kind: Kind) -> Rgba {
    match kind {
        Kind::Clone => ACCENT,
        Kind::Download => BLUE,
        Kind::File => GREEN,
        Kind::Edited => ORANGE,
    }
}

fn kind_tip(kind: Kind) -> &'static str {
    match kind {
        Kind::Clone => "The agent downloaded a copy of a project (cloned a repo) here",
        Kind::Download => "The agent downloaded this file from the internet",
        Kind::File => "The agent created this file",
        Kind::Edited => "The agent changed this file, but it was already there",
    }
}

pub fn empty_state(ic: IconName, title: &str, text: String) -> Div {
    v_flex()
        .items_center()
        .gap_2()
        .py_12()
        .px_8()
        .child(icon(ic, 32., FAINT))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(15.))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_color(MUTED)
                .text_size(px(13.))
                .text_center()
                .max_w(px(460.))
                .child(text),
        )
}

impl AgentCleanup {
    pub fn trail_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = &self.trail;
        let loading = t.loading.is_some();
        let safe = trail::safe_paths(&t.items);
        let safe_bytes: u64 = t
            .items
            .iter()
            .filter(|i| !i.needs_care())
            .map(|i| i.size_bytes)
            .sum();
        let total: u64 = t.items.iter().map(|i| i.size_bytes).sum();

        let refresh = icon_button(
            "trail-refresh",
            IconName::RefreshCw,
            if loading { "Reading..." } else { "Refresh" },
            ButtonKind::Secondary,
        )
        .tip("Read the AI tools' history again")
        .on_click(cx.listener(|this, _, _, cx| {
            this.load_trail();
            cx.notify();
        }));
        let clean = (!safe.is_empty() && !loading).then(|| {
            icon_button(
                "trail-clean",
                IconName::Broom,
                format!("Clean up {}", human_size(safe_bytes)),
                ButtonKind::Primary,
            )
            .tip("Move everything without a warning to the Recycle Bin. You'll see the list first.")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ask_recycle_trail(safe.clone());
                cx.notify();
            }))
        });

        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Agent trail",
                "Files, downloads and project copies that Claude Code and Codex created outside the project they were working on.",
            )))
            .child(refresh)
            .children(clean);

        let summary = (t.loaded && !t.items.is_empty()).then(|| {
            let care = t.items.iter().filter(|i| i.needs_care()).count();
            h_flex()
                .flex_wrap()
                .gap_x_4()
                .gap_y_1()
                .text_size(px(13.))
                .text_color(MUTED)
                .child(meta(
                    IconName::Footprints,
                    format!(
                        "{} thing{} left behind, {}",
                        t.items.len(),
                        if t.items.len() == 1 { "" } else { "s" },
                        human_size(total)
                    ),
                ))
                .when(care > 0, |d| {
                    d.child(
                        h_flex()
                            .gap_1()
                            .text_size(px(12.))
                            .text_color(ORANGE)
                            .child(icon(IconName::TriangleAlert, 13., ORANGE))
                            .child(format!("{care} need a look first")),
                    )
                })
                .child(meta(
                    IconName::Logs,
                    format!("From {} session logs, last {} days", t.logs, trail::DAYS),
                ))
        });

        let body: AnyElement = if !t.loaded {
            card()
                .child(empty_state(
                    IconName::Hourglass,
                    "Reading session logs...",
                    format!(
                        "Going through {} chat histories from Claude Code and Codex. Nothing is changed.",
                        t.logs
                    ),
                ))
                .into_any_element()
        } else if t.logs == 0 {
            card()
                .child(empty_state(
                    IconName::Footprints,
                    "No agent logs found",
                    "Claude Code keeps its logs in ~/.claude/projects and Codex in ~/.codex/sessions. Neither has anything from the last 90 days on this PC.".into(),
                ))
                .into_any_element()
        } else if t.items.is_empty() {
            card()
                .child(empty_state(
                    IconName::ShieldCheck,
                    "Nothing left behind",
                    "Everything the agents made is inside a project, already gone, or hidden."
                        .into(),
                ))
                .into_any_element()
        } else {
            let now = SystemTime::now();
            let mut groups = v_flex().gap_5();
            for (project, items) in trail::by_project(&t.items) {
                let size: u64 = items.iter().map(|i| i.size_bytes).sum();
                let label = if project.as_os_str().is_empty() {
                    "Unknown folder".to_string()
                } else {
                    format!("While working in {}", self.short(&project))
                };
                let mut list = v_flex();
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        list = list.child(div().h(px(1.)).bg(BORDER).mx_4());
                    }
                    list = list.child(self.trail_row(it, now, cx));
                }
                groups = groups.child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(section_label(&label)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .text_color(FAINT)
                                        .child(human_size(size)),
                                ),
                        )
                        .child(card().child(list)),
                );
            }
            groups.into_any_element()
        };

        div().id("trail").size_full().overflow_y_scroll().child(
            v_flex()
                .w_full()
                .max_w(px(900.))
                .mx_auto()
                .px_8()
                .pt_8()
                .pb(px(80.))
                .gap_5()
                .child(header)
                .children(summary)
                .child(body),
        )
    }

    fn trail_row(&self, it: &Item, now: SystemTime, cx: &mut Context<Self>) -> Stateful<Div> {
        let warnings = it.warnings();
        let color = kind_color(it.kind);
        let key = it.path.display().to_string();
        let (p_open, p_copy, p_hide, p_bin) = (
            it.path.clone(),
            it.path.clone(),
            it.path.clone(),
            it.path.clone(),
        );
        let care_pill = (!warnings.is_empty()).then(|| {
            let tip = format!("Look before recycling: {}.", warnings.join(", "));
            h_flex()
                .id(SharedString::from(format!("care-{key}")))
                .flex_none()
                .gap_1()
                .px_2()
                .py(px(2.))
                .rounded_full()
                .bg(tint(ORANGE, 0.14))
                .text_color(ORANGE)
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .child(icon(IconName::TriangleAlert, 12., ORANGE))
                .child("Check first")
                .tip(tip)
        });
        let mut facts = h_flex()
            .flex_wrap()
            .gap_x_3()
            .gap_y_1()
            .child(meta(IconName::Bot, it.agent.label()))
            .child(meta(trail_icon(it.kind), it.kind.label()))
            .child(meta(IconName::HardDrive, human_size(it.size_bytes)));
        if let Some(when) = it.when {
            facts = facts.child(meta(IconName::Clock, relative_time(when, now)));
        }
        if it.is_repo()
            && let Some(g) = &it.git
            && warnings.is_empty()
        {
            facts = facts.child(
                h_flex()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(GREEN)
                    .child(icon(IconName::ShieldCheck, 13., GREEN))
                    .child(if g.remote_url.is_some() {
                        "Saved online, nothing unsaved"
                    } else {
                        "Clean"
                    }),
            );
        }

        h_flex()
            .id(SharedString::from(format!("trail-{key}")))
            .group("trail-row")
            .gap_3()
            .px_4()
            .py_3()
            .hover(|s| s.bg(CARD_HOVER))
            .child(
                div()
                    .id(SharedString::from(format!("kind-{key}")))
                    .size(px(36.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(tint(color, 0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(trail_icon(it.kind), 18., color))
                    .tip(kind_tip(it.kind)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(it.name()),
                            )
                            .children(care_pill),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(FAINT)
                            .child(self.short(&it.path)),
                    )
                    .child(facts),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_0p5()
                    .child(
                        icon_action(
                            SharedString::from(format!("t-open-{key}")),
                            IconName::FolderOpen,
                            "Show in File Explorer",
                            false,
                        )
                        .on_click(
                            cx.listener(move |_, _, _, _| agent_cleanup::actions::reveal(&p_open)),
                        ),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("t-copy-{key}")),
                            IconName::Copy,
                            "Copy path",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_path(&p_copy, cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("t-hide-{key}")),
                            IconName::EyeOff,
                            "Hide: keep it and stop showing it here",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.hide_trail(p_hide.clone());
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("t-bin-{key}")),
                            IconName::Trash,
                            "Move to Recycle Bin",
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_recycle_trail(vec![p_bin.clone()]);
                            cx.notify();
                        })),
                    ),
            )
    }
}
