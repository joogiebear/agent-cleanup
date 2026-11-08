//! The Agent trail page: things Claude Code and Codex made outside the
//! project they were working in, read from their session logs.

use super::app::AgentCleanup;
use super::theme::*;
use super::widgets::*;
use agent_cleanup::model::{human_size, relative_time};
use agent_cleanup::trail::{self, Advice, Item, Kind};
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

pub fn advice_color(a: Advice) -> Rgba {
    match a {
        Advice::Safe => GREEN,
        Advice::CheckFirst => ORANGE,
        Advice::ProbablyYours => BLUE,
    }
}

fn advice_icon(a: Advice) -> IconName {
    match a {
        Advice::Safe => IconName::CircleCheck,
        Advice::CheckFirst => IconName::TriangleAlert,
        Advice::ProbablyYours => IconName::ShieldCheck,
    }
}

fn advice_tip(a: Advice) -> &'static str {
    match a {
        Advice::Safe => {
            "The agent made it outside its project and nothing in it is unsaved. Fine to remove."
        }
        Advice::CheckFirst => {
            "A project copy with work that isn't saved anywhere else, or that the app couldn't look inside. Look before removing."
        }
        Advice::ProbablyYours => {
            "It was there before the agent touched it, so it's probably yours. Keep it."
        }
    }
}

/// A tag with the app's call on a trail item.
pub fn advice_pill(id: impl Into<ElementId>, a: Advice, text: String) -> Stateful<Div> {
    verdict_tag(id, advice_color(a), advice_icon(a), text, advice_tip(a))
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
        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Agent trail",
                "Files, downloads and project copies that Claude Code and Codex created outside the project they were working on.",
            )))
            .child(refresh);

        let summary = (t.loaded && !t.items.is_empty()).then(|| {
            let count = |a: Advice| t.items.iter().filter(|i| i.advice() == a).count();
            let (n_safe, n_check, n_mine) = (
                count(Advice::Safe),
                count(Advice::CheckFirst),
                count(Advice::ProbablyYours),
            );
            let headline = if n_safe > 0 {
                format!(
                    "{n_safe} of {} can go, freeing {}",
                    t.items.len(),
                    human_size(safe_bytes)
                )
            } else {
                "Nothing here is safe to clean on its own".to_string()
            };
            let text = if n_safe > 0 {
                "An AI agent made these outside the project it was working on, and nothing in them is unsaved. Cleaning moves them to the Recycle Bin. It doesn't touch your projects or the agents' chat history."
            } else {
                "Everything here is either a project copy with unsaved work or a file that was probably yours already. Look at each one before removing it."
            };
            let mut tags = h_flex().flex_wrap().gap_2();
            for (a, n) in [
                (Advice::Safe, n_safe),
                (Advice::CheckFirst, n_check),
                (Advice::ProbablyYours, n_mine),
            ] {
                if n > 0 {
                    tags = tags.child(advice_pill(
                        SharedString::from(format!("trail-sum-{a:?}")),
                        a,
                        format!("{n} {}", a.label().to_lowercase()),
                    ));
                }
            }
            let accent = if n_safe > 0 { GREEN } else { ORANGE };
            let action = (n_safe > 0 && !loading).then(|| {
                let safe = safe.clone();
                icon_button(
                    "trail-clean",
                    IconName::BroomSparkles,
                    format!("Clean up {}", human_size(safe_bytes)),
                    ButtonKind::Primary,
                )
                .tip("Move every item marked Safe to clean to the Recycle Bin. You'll see the list first, and you can undo.")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_recycle_trail(safe.clone());
                    cx.notify();
                }))
            });
            v_flex()
                .gap_2()
                .child(advice_banner(
                    accent,
                    if n_safe > 0 { IconName::Sparkles } else { IconName::Info },
                    headline,
                    text,
                    Some(tags),
                    action,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_x_4()
                        .text_size(px(12.))
                        .text_color(FAINT)
                        .child(meta(
                            IconName::Footprints,
                            format!(
                                "{} thing{} left behind in total, {}",
                                t.items.len(),
                                if t.items.len() == 1 { "" } else { "s" },
                                human_size(total)
                            ),
                        ))
                        .child(meta(
                            IconName::Logs,
                            format!(
                                "Found in {} chat histories from the last {} days",
                                t.logs,
                                trail::DAYS
                            ),
                        )),
                )
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
                let group_safe: Vec<_> = items
                    .iter()
                    .filter(|i| i.advice() == Advice::Safe)
                    .map(|i| i.path.clone())
                    .collect();
                let group_safe_bytes: u64 = items
                    .iter()
                    .filter(|i| i.advice() == Advice::Safe)
                    .map(|i| i.size_bytes)
                    .sum();
                let group_key = project.display().to_string();
                let prompt = trail::prompt_for(&project, &items);
                let label = if project.as_os_str().is_empty() {
                    "Unknown folder".to_string()
                } else {
                    format!("Left by an agent working on {}", self.short(&project))
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
                                )
                                .child(
                                    prompt_button(SharedString::from(format!(
                                        "trail-prompt-{group_key}"
                                    )))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy_prompt(prompt.clone(), cx);
                                        cx.notify();
                                    })),
                                )
                                .when(!group_safe.is_empty() && !loading, |d| {
                                    let n = group_safe.len();
                                    d.child(
                                        button(
                                            SharedString::from(format!("trail-gclean-{group_key}")),
                                            format!("Clean {n} safe ({})", human_size(group_safe_bytes)),
                                            ButtonKind::Ghost,
                                        )
                                        .flex_none()
                                        .h(px(26.))
                                        .tip("Recycle only the items in this group marked Safe to clean. You'll see the list first.")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.ask_recycle_trail(group_safe.clone());
                                            cx.notify();
                                        })),
                                    )
                                }),
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
        let advice = it.advice();
        let pill = advice_pill(
            SharedString::from(format!("care-{key}")),
            advice,
            advice.label().to_string(),
        );
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
                            .child(pill),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(MUTED)
                            .child(it.explain()),
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
