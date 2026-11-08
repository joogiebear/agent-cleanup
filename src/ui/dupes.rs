//! The Duplicates page: the same remote repo cloned into more than one
//! folder. One copy is kept; the others can go.

use super::app::{AgentCleanup, Page};
use super::theme::*;
use super::trail::empty_state;
use super::widgets::*;
use agent_cleanup::dupes::{self, Group};
use agent_cleanup::model::{Repo, human_size, relative_time};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::SystemTime;

impl AgentCleanup {
    pub fn dupes_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let groups = dupes::groups(self.lib.repos(), &self.dup_keep);
        let safe = dupes::safe_extras(&groups, |p| self.lib.get(p).cloned());
        let safe_bytes: u64 = safe
            .iter()
            .filter_map(|p| self.lib.get(p).and_then(|r| r.size_bytes))
            .sum();
        let scanning = self.scan.is_some();

        let rescan = icon_button(
            "dup-rescan",
            IconName::RefreshCw,
            if scanning {
                "Scanning..."
            } else {
                "Scan again"
            },
            ButtonKind::Secondary,
        )
        .tip("Duplicates come from the Leftovers scan. Scan again to pick up new copies.")
        .on_click(cx.listener(|this, _, _, cx| {
            if this.scan.is_none() {
                this.start_scan();
            }
            cx.notify();
        }));
        let clean = (!safe.is_empty() && !scanning).then(|| {
            icon_button(
                "dup-clean",
                IconName::Broom,
                format!("Clean up {}", human_size(safe_bytes)),
                ButtonKind::Primary,
            )
            .tip("Recycle the extra copies that are outside your projects and fully saved online. You'll see the list first.")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ask_recycle_dupes(safe.clone());
                cx.notify();
            }))
        });
        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Duplicates",
                "The same project downloaded (cloned) into more than one folder. Keep one copy and recycle the rest.",
            )))
            .child(rescan)
            .children(clean);

        let summary = (!groups.is_empty()).then(|| {
            let extras: usize = groups.iter().map(|g| g.copies.len() - 1).sum();
            let bytes: u64 = groups
                .iter()
                .flat_map(|g| g.extras())
                .filter_map(|p| self.lib.get(p).and_then(|r| r.size_bytes))
                .sum();
            h_flex()
                .flex_wrap()
                .gap_x_4()
                .gap_y_1()
                .text_size(px(13.))
                .text_color(MUTED)
                .child(meta(
                    IconName::Copy,
                    format!(
                        "{} project{} downloaded more than once",
                        groups.len(),
                        if groups.len() == 1 { "" } else { "s" }
                    ),
                ))
                .child(meta(
                    IconName::HardDrive,
                    format!(
                        "{extras} extra cop{}, {}",
                        if extras == 1 { "y" } else { "ies" },
                        human_size(bytes)
                    ),
                ))
        });

        let body: AnyElement = if self.lib.is_empty() {
            card()
                .child(empty_state(
                    IconName::FolderSearch,
                    if scanning { "Scanning..." } else { "No scan yet" },
                    "Duplicates are found among the project copies the Leftovers scan finds. Set up your project folders and scan from the Leftovers page.".into(),
                ))
                .child(
                    h_flex().justify_center().pb_8().child(
                        button("dup-go-leftovers", "Go to Leftovers", ButtonKind::Secondary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.go(Page::Leftovers);
                                cx.notify();
                            })),
                    ),
                )
                .into_any_element()
        } else if groups.is_empty() {
            card()
                .child(empty_state(
                    IconName::ShieldCheck,
                    "No duplicates",
                    "Every project the scan found is on this PC only once.".into(),
                ))
                .into_any_element()
        } else {
            let now = SystemTime::now();
            let mut list = v_flex().gap_5();
            for g in &groups {
                list = list.child(self.dupe_group(g, now, cx));
            }
            list.into_any_element()
        };

        div().id("dupes").size_full().overflow_y_scroll().child(
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

    fn dupe_group(&self, g: &Group, now: SystemTime, cx: &mut Context<Self>) -> Div {
        let mut rows = v_flex();
        for (i, p) in g.copies.iter().enumerate() {
            let Some(r) = self.lib.get(p) else {
                continue;
            };
            if i > 0 {
                rows = rows.child(div().h(px(1.)).bg(BORDER).mx_4());
            }
            rows = rows.child(self.dupe_row(g, r, now, cx));
        }
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
                            .child(section_label(&g.remote)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(FAINT)
                            .child(format!("{} copies", g.copies.len())),
                    ),
            )
            .child(card().child(rows))
    }

    fn dupe_row(
        &self,
        g: &Group,
        r: &Repo,
        now: SystemTime,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = r.path.display().to_string();
        let keep = r.path == g.keep;
        let local_work = r.git.as_ref().is_none_or(|g| g.has_local_work());
        let (label, color, tip): (&str, Rgba, String) = if keep {
            (
                "Keeping",
                GREEN,
                "The copy to keep. Pick another with the pin button.".into(),
            )
        } else if local_work {
            (
                "Has unsaved work",
                ORANGE,
                format!("{}. Save it before recycling.", r.git_summary()),
            )
        } else if r.project_root.is_some() {
            (
                "In a project folder",
                MUTED,
                "It's in one of your project folders, so Clean up leaves it alone.".into(),
            )
        } else {
            (
                "Extra copy",
                BLUE,
                "Everything in it is saved online, so the copy you keep has it all.".into(),
            )
        };
        let pill = h_flex()
            .id(SharedString::from(format!("dup-pill-{key}")))
            .flex_none()
            .gap_1()
            .px_2()
            .py(px(2.))
            .rounded_full()
            .bg(tint(color, 0.14))
            .text_color(color)
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .when(keep, |d| d.child(icon(IconName::BadgeCheck, 12., color)))
            .child(label)
            .tip(tip);

        let mut facts = h_flex().flex_wrap().gap_x_3().gap_y_1().child(meta(
            IconName::HardDrive,
            r.size_bytes.map(human_size).unwrap_or_default(),
        ));
        if let Some(t) = r.last_activity {
            facts = facts.child(meta(
                IconName::Clock,
                format!("Used {}", relative_time(t, now)),
            ));
        }
        if let Some(b) = r.git.as_ref().and_then(|g| g.branch.clone()) {
            facts = facts.child(meta(IconName::GitBranch, b));
        }

        let (p_open, p_bin, p_keep, remote) = (
            r.path.clone(),
            r.path.clone(),
            r.path.clone(),
            g.remote.clone(),
        );
        let mut actions = h_flex().flex_none().gap_0p5();
        if !keep {
            actions = actions.child(
                icon_action(
                    SharedString::from(format!("dup-keep-{key}")),
                    IconName::Pin,
                    "Keep this copy instead",
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dup_keep.insert(remote.clone(), p_keep.clone());
                    cx.notify();
                })),
            );
        }
        actions = actions.child(
            icon_action(
                SharedString::from(format!("dup-open-{key}")),
                IconName::FolderOpen,
                "Show in File Explorer",
                false,
            )
            .on_click(cx.listener(move |_, _, _, _| agent_cleanup::actions::open_folder(&p_open))),
        );
        if !keep {
            actions = actions.child(
                icon_action(
                    SharedString::from(format!("dup-bin-{key}")),
                    IconName::Trash,
                    "Move to Recycle Bin",
                    true,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_recycle_dupes(vec![p_bin.clone()]);
                    cx.notify();
                })),
            );
        }

        h_flex()
            .id(SharedString::from(format!("dup-{key}")))
            .gap_3()
            .px_4()
            .py_3()
            .hover(|s| s.bg(CARD_HOVER))
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
                                    .child(self.short(&r.path)),
                            )
                            .child(pill),
                    )
                    .child(facts),
            )
            .child(actions)
    }
}
