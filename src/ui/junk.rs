//! The Build junk page: dependency and build folders in your repos that one
//! command recreates.

use super::app::{AgentCleanup, Page};
use super::theme::*;
use super::trail::empty_state;
use super::widgets::*;
use agent_cleanup::junk::{self, Item};
use agent_cleanup::model::{human_size, relative_time};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::SystemTime;

impl AgentCleanup {
    pub fn junk_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let j = &self.junk;
        let now = SystemTime::now();
        let days = self.cfg.stale_after_days;
        let loading = j.loading.is_some();
        let busy = j.busy.as_ref().map(|(_, label)| label.clone());
        let s = junk::stats(&j.items, days, now);
        let idle = junk::idle_paths(&j.items, days, now);

        let refresh = icon_button(
            "junk-refresh",
            IconName::RefreshCw,
            if loading { "Looking..." } else { "Refresh" },
            ButtonKind::Secondary,
        )
        .tip("Look through your repos again")
        .on_click(cx.listener(|this, _, _, cx| {
            this.load_junk();
            cx.notify();
        }));
        let clean = (!idle.is_empty() && !loading).then(|| {
            let btn = icon_button(
                "junk-clean",
                IconName::Broom,
                format!("Clean up {}", human_size(s.idle_bytes)),
                ButtonKind::Primary,
            );
            if busy.is_some() {
                disabled(btn).tip("Wait for the current move to finish")
            } else {
                btn.tip(format!(
                    "Recycle the build folders of projects you haven't touched in {days}+ days. You'll see the list first."
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_recycle_junk(idle.clone());
                    cx.notify();
                }))
            }
        });
        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Build junk",
                "Dependency and build folders inside your repos, like node_modules and target. One command brings each one back.",
            )))
            .child(refresh)
            .children(clean);

        let summary = (j.loaded && !j.items.is_empty()).then(|| {
            h_flex()
                .flex_wrap()
                .gap_x_4()
                .gap_y_1()
                .child(meta(
                    IconName::Package,
                    format!(
                        "{} folder{}, {}",
                        s.count,
                        if s.count == 1 { "" } else { "s" },
                        human_size(s.bytes)
                    ),
                ))
                .child(meta(
                    IconName::Clock,
                    format!(
                        "{} in projects untouched for {days}+ days",
                        human_size(s.idle_bytes)
                    ),
                ))
                .child(
                    div()
                        .id("junk-days")
                        .text_size(px(12.))
                        .text_color(ACCENT)
                        .cursor_pointer()
                        .child("Change in Settings")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.go(Page::Settings);
                            cx.notify();
                        })),
                )
        });

        let busy_bar = busy.map(|label| {
            h_flex()
                .gap_3()
                .px_4()
                .py_3()
                .rounded(px(10.))
                .bg(tint(ACCENT, 0.12))
                .border_1()
                .border_color(tint(ACCENT, 0.35))
                .child(icon(IconName::Hourglass, 16., ACCENT))
                .child(div().flex_1().min_w_0().child(label))
        });

        let body: AnyElement = if self.lib.is_empty() && !loading {
            card()
                .child(
                    empty_state(
                        IconName::FolderSearch,
                        "Scan first",
                        "Build junk looks inside the repos found by a scan. Run one on the Leftovers page, then come back.".into(),
                    )
                    .child(
                        button("junk-go-scan", "Go to Leftovers", ButtonKind::Secondary).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.go(Page::Leftovers);
                                cx.notify();
                            }),
                        ),
                    ),
                )
                .into_any_element()
        } else if !j.loaded {
            card()
                .child(empty_state(
                    IconName::Hourglass,
                    "Looking inside your repos...",
                    format!(
                        "{} of {} repos checked. Nothing is changed.",
                        self.junk_progress(),
                        j.repos
                    ),
                ))
                .into_any_element()
        } else if j.items.is_empty() {
            card()
                .child(empty_state(
                    IconName::ShieldCheck,
                    "No build junk",
                    "None of your repos has a dependency or build folder that git ignores.".into(),
                ))
                .into_any_element()
        } else {
            let mut groups = v_flex().gap_5();
            for (repo, items) in junk::by_repo(&j.items) {
                let size: u64 = items.iter().map(|i| i.size_bytes).sum();
                let first = items[0];
                let idle = first.is_idle(days, now);
                let when = first
                    .repo_activity
                    .map(|t| format!("Last worked on {}", relative_time(t, now)))
                    .unwrap_or_else(|| "Last use unknown".into());
                let mut list = v_flex();
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        list = list.child(div().h(px(1.)).bg(BORDER).mx_4());
                    }
                    list = list.child(self.junk_row(it, cx));
                }
                groups = groups.child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_none()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(first.repo_name()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(FAINT)
                                        .child(self.short(&repo)),
                                )
                                .child(
                                    h_flex()
                                        .flex_none()
                                        .gap_1()
                                        .text_size(px(12.))
                                        .text_color(if idle { GREEN } else { MUTED })
                                        .child(icon(
                                            IconName::Clock,
                                            13.,
                                            if idle { GREEN } else { MUTED },
                                        ))
                                        .child(when),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(12.))
                                        .text_color(FAINT)
                                        .child(human_size(size)),
                                ),
                        )
                        .child(card().child(list)),
                );
            }
            groups.into_any_element()
        };

        div().id("junk").size_full().overflow_y_scroll().child(
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
                .children(busy_bar)
                .child(body),
        )
    }

    fn junk_row(&self, it: &Item, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = it.path.display().to_string();
        let (p_open, p_bin) = (it.path.clone(), it.path.clone());
        let busy = self.junk.busy.is_some();
        let bin = icon_action(
            SharedString::from(format!("j-bin-{key}")),
            IconName::Trash,
            "Move to Recycle Bin",
            true,
        );
        let bin = if busy {
            disabled(bin)
        } else {
            bin.on_click(cx.listener(move |this, _, _, cx| {
                this.ask_recycle_junk(vec![p_bin.clone()]);
                cx.notify();
            }))
        };
        h_flex()
            .id(SharedString::from(format!("junk-{key}")))
            .gap_3()
            .px_4()
            .py_3()
            .hover(|s| s.bg(CARD_HOVER))
            .child(
                div()
                    .size(px(36.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(tint(BLUE, 0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::Package, 18., BLUE)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(it.name()),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x_3()
                            .gap_y_1()
                            .child(meta(IconName::Package, it.kind.label))
                            .child(meta(IconName::HardDrive, human_size(it.size_bytes)))
                            .child(meta(
                                IconName::Terminal,
                                format!("Bring back: {}", it.kind.rebuild),
                            )),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_0p5()
                    .child(
                        icon_action(
                            SharedString::from(format!("j-open-{key}")),
                            IconName::FolderOpen,
                            "Show in File Explorer",
                            false,
                        )
                        .on_click(
                            cx.listener(move |_, _, _, _| agent_cleanup::actions::reveal(&p_open)),
                        ),
                    )
                    .child(bin),
            )
    }
}
