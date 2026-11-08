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
        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Duplicates",
                "The same project downloaded (cloned) into more than one folder. Keep one copy and recycle the rest.",
            )))
            .child(rescan);

        let summary = (!groups.is_empty()).then(|| {
            let extras: usize = groups.iter().map(|g| g.copies.len() - 1).sum();
            let n_safe = safe.len();
            let headline = if n_safe > 0 {
                format!(
                    "{n_safe} extra cop{} can go, freeing {}",
                    if n_safe == 1 { "y" } else { "ies" },
                    human_size(safe_bytes)
                )
            } else {
                "No extra copy is safe to clean on its own".to_string()
            };
            let text = if n_safe > 0 {
                "For each project one copy is kept (marked Keeping). The extra copies marked Safe to clean have everything saved online, so the kept copy has it all and nothing is lost. Copies with unsaved work, or inside your project folders, are left alone."
            } else {
                "Every extra copy either has work that isn't saved anywhere else, or is inside one of your project folders. Save the work (or ask your agent to), then scan again."
            };
            let action = (n_safe > 0 && !scanning).then(|| {
                let safe = safe.clone();
                icon_button(
                    "dup-clean",
                    IconName::BroomSparkles,
                    format!("Clean up {}", human_size(safe_bytes)),
                    ButtonKind::Primary,
                )
                .tip("Recycle the extra copies marked Safe to clean. You'll see the list first.")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_recycle_dupes(safe.clone());
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
                    None,
                    action,
                ))
                .child(
                    div().text_size(px(12.)).text_color(FAINT).child(format!(
                        "{} project{} on this PC more than once, {extras} extra cop{} in total",
                        groups.len(),
                        if groups.len() == 1 { " is" } else { "s are" },
                        if extras == 1 { "y" } else { "ies" },
                    )),
                )
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
        let entries: Vec<_> = g
            .copies
            .iter()
            .filter_map(|p| self.lib.get(p))
            .map(|r| {
                let note = if r.path == g.keep {
                    "the copy I want to keep".to_string()
                } else {
                    r.git_summary().to_lowercase()
                };
                agent_cleanup::prompts::Entry::new(r.path.clone(), Some(note))
            })
            .collect();
        let prompt = agent_cleanup::prompts::dupes_group(&g.keep, &entries);
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
                    )
                    .child(
                        prompt_button(SharedString::from(format!(
                            "dup-prompt-{}",
                            g.keep.display()
                        )))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_prompt(prompt.clone(), cx);
                            cx.notify();
                        })),
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
                ACCENT,
                "This is the copy that stays. To keep a different one, use the pin button on it."
                    .into(),
            )
        } else if local_work {
            (
                "Check first",
                ORANGE,
                format!(
                    "It has work that isn't saved anywhere else ({}). Removing it would lose that, so save it first or keep this copy.",
                    r.git_summary().to_lowercase()
                ),
            )
        } else if r.project_root.is_some() {
            (
                "In a project folder",
                MUTED,
                "It's in one of your project folders, so Clean up leaves it alone. Remove it yourself if you really don't need it.".into(),
            )
        } else {
            (
                "Safe to clean",
                GREEN,
                "An extra copy. Everything in it is saved online, so the copy you keep has it all and nothing is lost.".into(),
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
            .tip(tip.clone());

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
                    .child(div().text_size(px(12.)).text_color(MUTED).child(tip))
                    .child(facts),
            )
            .child(actions)
    }
}
