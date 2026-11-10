//! The Agent data page: Claude Code and Codex session history for projects
//! that no longer exist.

use super::app::AgentCleanup;
use super::theme::*;
use super::trail::empty_state;
use super::widgets::*;
use agent_cleanup::agentdata::Orphan;
use agent_cleanup::model::{human_size, relative_time};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::SystemTime;

/// Why to look before recycling, if anything.
pub fn recent_warning(o: &Orphan, now: SystemTime) -> Option<String> {
    o.warning(now)
}

impl AgentCleanup {
    pub fn agentdata_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let a = &self.agentdata;
        let loading = a.loading.is_some();
        let now = SystemTime::now();
        let safe: Vec<_> = a
            .items
            .iter()
            .filter(|o| recent_warning(o, now).is_none())
            .map(|o| o.key().to_path_buf())
            .collect();
        let safe_bytes: u64 = a
            .items
            .iter()
            .filter(|o| recent_warning(o, now).is_none())
            .map(|o| o.size_bytes)
            .sum();

        let refresh = icon_button(
            "ad-refresh",
            IconName::RefreshCw,
            if loading { "Looking..." } else { "Refresh" },
            ButtonKind::Secondary,
        )
        .tip("Look through the agents' data again")
        .on_click(cx.listener(|this, _, _, cx| {
            this.load_agentdata();
            cx.notify();
        }));
        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Agent data",
                "Chat history that Claude Code and Codex kept for projects you've since deleted. Without the project those chats can't be picked up again, so they just take space.",
            )))
            .child(refresh);

        let summary = (a.loaded && !a.items.is_empty()).then(|| {
            let total: u64 = a.items.iter().map(|o| o.size_bytes).sum();
            let sessions: usize = a.items.iter().map(|o| o.sessions).sum();
            let entries: Vec<_> = a
                .items
                .iter()
                .map(|o| {
                    agent_cleanup::prompts::Entry::new(
                        o.project.clone(),
                        Some(format!("{} history", o.agent.label())),
                    )
                })
                .collect();
            let prompt = agent_cleanup::prompts::agentdata(&entries);
            let n_safe = safe.len();
            let n_recent = a.items.len() - n_safe;
            let headline = if n_safe > 0 {
                format!(
                    "{n_safe} of {} can go, freeing {}",
                    a.items.len(),
                    human_size(safe_bytes)
                )
            } else {
                "Everything here needs a look first".to_string()
            };
            let text = if n_safe > 0 {
                "The projects these chats belonged to are gone, so the chats can't be reopened. Removing them only frees space: no project or file of yours is touched, and they go to the Recycle Bin first."
            } else {
                "Their projects are gone from where they were, but they were used in the last week or a project with the same name exists somewhere else. If you moved or renamed a project rather than deleting it, you may still want its chats."
            };
            let mut tags = h_flex().flex_wrap().gap_2();
            if n_safe > 0 {
                tags = tags.child(verdict_tag(
                    "ad-sum-safe",
                    GREEN,
                    IconName::CircleCheck,
                    format!("{n_safe} safe to clean"),
                    "Not used in the last week, the project is gone, and no project with the same name was found elsewhere.",
                ));
            }
            if n_recent > 0 {
                tags = tags.child(verdict_tag(
                    "ad-sum-recent",
                    ORANGE,
                    IconName::TriangleAlert,
                    format!("{n_recent} check first"),
                    "Used in the last week, or a project with the same name exists somewhere else. The project may only have moved.",
                ));
            }
            let action = (n_safe > 0 && !loading).then(|| {
                let safe = safe.clone();
                icon_button(
                    "ad-clean",
                    IconName::BroomSparkles,
                    format!("Clean up {}", human_size(safe_bytes)),
                    ButtonKind::Primary,
                )
                .tip("Recycle the chat history marked Safe to clean. You'll see the list first.")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_recycle_agentdata(safe.clone());
                    cx.notify();
                }))
            });
            v_flex()
                .gap_2()
                .child(advice_banner(
                    if n_safe > 0 { GREEN } else { ORANGE },
                    if n_safe > 0 { IconName::Sparkles } else { IconName::Info },
                    headline,
                    text,
                    Some(tags),
                    action,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x_4()
                        .text_size(px(12.))
                        .text_color(FAINT)
                        .child(meta(
                            IconName::FolderX,
                            format!(
                                "{} deleted project{}, {} in total",
                                a.items.len(),
                                if a.items.len() == 1 { "" } else { "s" },
                                human_size(total)
                            ),
                        ))
                        .child(meta(
                            IconName::Logs,
                            format!(
                                "{sessions} chat session{}",
                                if sessions == 1 { "" } else { "s" }
                            ),
                        ))
                        .child(div().flex_1())
                        .child(prompt_button("ad-prompt").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.copy_prompt(prompt.clone(), cx);
                                cx.notify();
                            },
                        ))),
                )
        });

        let body: AnyElement = if !a.loaded {
            card()
                .child(empty_state(
                    IconName::Hourglass,
                    "Looking through agent data...",
                    "Reading the AI tools' history folders to see which projects still exist. Nothing is changed.".into(),
                ))
                .into_any_element()
        } else if a.items.is_empty() {
            card()
                .child(empty_state(
                    IconName::ShieldCheck,
                    "Nothing left over",
                    "Every project with Claude Code or Codex history still exists, is on a drive that isn't connected, or is hidden.".into(),
                ))
                .into_any_element()
        } else {
            let mut list = v_flex();
            for (i, o) in a.items.iter().enumerate() {
                if i > 0 {
                    list = list.child(div().h(px(1.)).bg(BORDER).mx_4());
                }
                list = list.child(self.orphan_row(o, now, cx));
            }
            card().child(list).into_any_element()
        };

        div().id("agentdata").size_full().overflow_y_scroll().child(
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

    fn orphan_row(&self, o: &Orphan, now: SystemTime, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = o.key().display().to_string();
        let (k_open, k_hide, k_bin) = (
            o.key().to_path_buf(),
            o.key().to_path_buf(),
            o.key().to_path_buf(),
        );
        let warning = recent_warning(o, now);
        let color = if warning.is_some() { ORANGE } else { MUTED };
        let explain = match &warning {
            Some(w) => format!(
                "Its project folder is gone, but it was {w}. If you moved or renamed the project rather than deleting it, keep these chats."
            ),
            None => "Its project folder is gone, so these chats can't be reopened. Removing them only frees space; none of your files are touched.".to_string(),
        };
        let pill = match warning {
            Some(_) => verdict_tag(
                SharedString::from(format!("ad-care-{key}")),
                ORANGE,
                IconName::TriangleAlert,
                "Check first",
                "Used in the last week, or a project with the same name exists somewhere else. The project may only have moved.",
            ),
            None => verdict_tag(
                SharedString::from(format!("ad-care-{key}")),
                GREEN,
                IconName::CircleCheck,
                "Safe to clean",
                "Not used in the last week, the project is gone, and no project with the same name was found elsewhere.",
            ),
        };
        let mut facts = h_flex()
            .flex_wrap()
            .gap_x_3()
            .gap_y_1()
            .child(meta(IconName::Bot, o.agent.label()))
            .child(meta(
                IconName::Logs,
                format!(
                    "{} session{}",
                    o.sessions,
                    if o.sessions == 1 { "" } else { "s" }
                ),
            ))
            .child(meta(IconName::HardDrive, human_size(o.size_bytes)));
        if let Some(t) = o.last_used {
            facts = facts.child(meta(
                IconName::Clock,
                format!("Last used {}", relative_time(t, now)),
            ));
        }

        h_flex()
            .id(SharedString::from(format!("ad-{key}")))
            .gap_3()
            .px_4()
            .py_3()
            .hover(|s| s.bg(CARD_HOVER))
            .child(
                div()
                    .size(px(36.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(tint(color, 0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::Ghost, 18., color)),
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
                                    .child(o.project_name()),
                            )
                            .child(pill),
                    )
                    .child(div().text_size(px(12.)).text_color(MUTED).child(explain))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(FAINT)
                            .child(format!("Was at {}", self.short(&o.project))),
                    )
                    .child(facts),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_0p5()
                    .child(
                        icon_action(
                            SharedString::from(format!("ad-open-{key}")),
                            IconName::FolderOpen,
                            "Show the data in File Explorer",
                            false,
                        )
                        .on_click(
                            cx.listener(move |_, _, _, _| agent_cleanup::actions::reveal(&k_open)),
                        ),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("ad-hide-{key}")),
                            IconName::EyeOff,
                            "Hide: keep it and stop showing it here",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.hide_agentdata(k_hide.clone());
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("ad-bin-{key}")),
                            IconName::Trash,
                            "Move to Recycle Bin",
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_recycle_agentdata(vec![k_bin.clone()]);
                            cx.notify();
                        })),
                    ),
            )
    }
}
