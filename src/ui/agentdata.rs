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
    o.is_recent(now).then(|| {
        format!(
            "last used {}",
            o.last_used
                .map(|t| relative_time(t, now))
                .unwrap_or_default()
        )
    })
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
        let clean = (!safe.is_empty() && !loading).then(|| {
            icon_button(
                "ad-clean",
                IconName::Broom,
                format!("Clean up {}", human_size(safe_bytes)),
                ButtonKind::Primary,
            )
            .tip("Recycle everything not used in the last week. You'll see the list first.")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ask_recycle_agentdata(safe.clone());
                cx.notify();
            }))
        });
        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Agent data",
                "Chat history that Claude Code and Codex kept for projects you've since deleted. Without the project those chats can't be picked up again, so they just take space.",
            )))
            .child(refresh)
            .children(clean);

        let summary = (a.loaded && !a.items.is_empty()).then(|| {
            let total: u64 = a.items.iter().map(|o| o.size_bytes).sum();
            let sessions: usize = a.items.iter().map(|o| o.sessions).sum();
            h_flex()
                .flex_wrap()
                .gap_x_4()
                .gap_y_1()
                .text_size(px(13.))
                .text_color(MUTED)
                .child(meta(
                    IconName::FolderX,
                    format!(
                        "{} deleted project{}, {}",
                        a.items.len(),
                        if a.items.len() == 1 { "" } else { "s" },
                        human_size(total)
                    ),
                ))
                .child(meta(
                    IconName::Logs,
                    format!("{sessions} session{}", if sessions == 1 { "" } else { "s" }),
                ))
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
        let pill = warning.map(|w| {
            h_flex()
                .id(SharedString::from(format!("ad-care-{key}")))
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
                .child("Used recently")
                .tip(format!(
                    "{w}. If you moved the project rather than deleting it, you may still want these."
                ))
        });
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
                            .children(pill),
                    )
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
