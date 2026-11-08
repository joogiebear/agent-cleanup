//! The main page: filter sidebar, stat cards, the clean-up call to action
//! and the list of repo cards.

use super::app::{AgentCleanup, group_digits};
use super::theme::*;
use super::widgets::*;
use agent_cleanup::library::{Filter, SortBy};
use agent_cleanup::model::{Verdict, human_size, relative_time};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::path::PathBuf;
use std::time::SystemTime;

const SIDEBAR_W: f32 = 232.;
const ROW_H: f32 = 76.;

fn filter_icon(f: Filter) -> (IconName, Rgba) {
    match f {
        Filter::Strays => (IconName::FolderSearch, ACCENT),
        Filter::SafeToRemove => (IconName::CircleCheck, GREEN),
        Filter::UnsavedWork => (IconName::TriangleAlert, ORANGE),
        Filter::Unreadable => (IconName::CircleQuestionMark, RED),
        Filter::StrayActive => (IconName::Clock, BLUE),
        Filter::All => (IconName::Folders, MUTED),
    }
}

impl AgentCleanup {
    pub fn leftovers_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let now = SystemTime::now();
        let rows = self
            .lib
            .visible(self.filter, &self.search_text, self.sort, &self.cfg, now);
        let focused = self.focused.clone().and_then(|p| self.lib.get(&p).cloned());
        let sidebar = self.filter_sidebar(now, cx);
        let header = self.leftovers_header(cx);
        let body = self.leftovers_body(rows, now, cx);
        let details = focused.map(|r| self.details_panel(r, window, cx));
        h_flex()
            .size_full()
            .child(sidebar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(header)
                    .child(body),
            )
            .children(details)
    }

    fn filter_sidebar(&self, now: SystemTime, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut list = v_flex().gap_0p5().px_2().py_3();
        for f in Filter::ALL {
            let (ic, color) = filter_icon(f);
            let active = self.filter == f;
            let n = self.lib.count(f, &self.cfg, now);
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("filter-{f:?}")))
                    .gap_2p5()
                    .h(px(36.))
                    .px_2p5()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(CARD_HOVER))
                    .when(!active, |d| d.hover(|s| s.bg(CARD)))
                    .child(icon(ic, 16., if active { color } else { FAINT }))
                    .child(
                        div()
                            .flex_1()
                            .text_color(if active { TEXT } else { MUTED })
                            .font_weight(if active {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .child(f.label()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .px_1p5()
                            .rounded_full()
                            .when(active && n > 0, |d| {
                                d.bg(tint(color, 0.18)).text_color(color)
                            })
                            .when(!active || n == 0, |d| d.text_color(FAINT))
                            .child(n.to_string()),
                    )
                    .tip(f.tooltip())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = f;
                        cx.notify();
                    })),
            );
        }

        v_flex()
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_none()
            .bg(SIDEBAR)
            .child(
                h_flex()
                    .h(px(56.))
                    .px_4()
                    .border_b_1()
                    .border_color(RAIL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.))
                    .child("Leftovers"),
            )
            .child(list)
            .child(div().flex_1())
            .child(self.scan_card(cx))
    }

    fn scan_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (title, detail): (String, String) = if let Some(s) = &self.scan {
            (
                "Scanning...".into(),
                format!("{} folders checked", group_digits(s.dirs)),
            )
        } else if let Some(msg) = &self.last_scan {
            ("Last scan".into(), msg.clone())
        } else if let Some(t) = self.cached_at {
            (
                "Last scan".into(),
                format!(
                    "{}. Rescan to refresh.",
                    relative_time(t, SystemTime::now())
                ),
            )
        } else {
            (
                "Not scanned yet".into(),
                "Scan to look for leftovers.".into(),
            )
        };
        let scanning = self.scan.is_some();
        let btn = if scanning {
            icon_button("scan-btn", IconName::CircleX, "Stop", ButtonKind::Secondary)
                .tip("Stop scanning. You'll keep what was found so far.")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.stop_scan();
                    cx.notify();
                }))
        } else {
            icon_button(
                "scan-btn",
                IconName::RefreshCw,
                "Rescan",
                if self.needs_rescan {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Secondary
                },
            )
            .tip("Look through your home folder again")
            .on_click(cx.listener(|this, _, _, cx| {
                this.start_scan();
                cx.notify();
            }))
        };
        v_flex()
            .m_2()
            .p_3()
            .gap_2()
            .rounded(px(10.))
            .bg(RAIL)
            .child(
                h_flex()
                    .gap_2()
                    .child(icon(
                        if scanning {
                            IconName::LoaderCircle
                        } else {
                            IconName::Scan
                        },
                        16.,
                        if scanning { ACCENT } else { MUTED },
                    ))
                    .child(div().font_weight(FontWeight::MEDIUM).child(title)),
            )
            .child(div().text_size(px(12.)).text_color(FAINT).child(detail))
            .when(self.needs_rescan && !scanning, |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(ORANGE)
                        .child("Settings changed. Rescan to apply them."),
                )
            })
            .child(btn.w_full())
    }

    fn leftovers_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let sort = self.sort;
        let sort_btn = h_flex()
            .id("sort")
            .flex_none()
            .gap_1p5()
            .h(px(34.))
            .px_3()
            .rounded(px(8.))
            .bg(CARD)
            .cursor_pointer()
            .hover(|s| s.bg(CARD_HOVER))
            .text_color(MUTED)
            .child(icon(IconName::SortDescending, 15., MUTED))
            .child(sort.label())
            .tip("Change the order. Click to cycle.")
            .on_click(cx.listener(|this, _, _, cx| {
                let i = SortBy::ALL
                    .iter()
                    .position(|s| *s == this.sort)
                    .unwrap_or(0);
                this.sort = SortBy::ALL[(i + 1) % SortBy::ALL.len()];
                cx.notify();
            }));
        let (ic, color) = filter_icon(self.filter);
        h_flex()
            .h(px(56.))
            .flex_none()
            .px_5()
            .gap_3()
            .border_b_1()
            .border_color(SIDEBAR)
            .child(icon(ic, 20., color))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.))
                    .child(self.filter.label()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(FAINT)
                    .text_size(px(13.))
                    .child(self.filter.tooltip()),
            )
            .child(
                div().w(px(240.)).flex_none().child(
                    Input::new(&self.search)
                        .prefix(icon(IconName::Search, 15., FAINT))
                        .cleanable(true),
                ),
            )
            .child(sort_btn)
    }

    fn leftovers_body(
        &mut self,
        rows: Vec<PathBuf>,
        now: SystemTime,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let stats = self.lib.stats(&self.cfg, now);
        let mut body = v_flex().flex_1().min_h_0().px_5().pt_4().gap_4();

        if let Some(s) = &self.scan {
            body = body.child(
                h_flex()
                    .gap_3()
                    .px_4()
                    .py_2p5()
                    .rounded(px(10.))
                    .bg(tint(ACCENT, 0.12))
                    .border_1()
                    .border_color(tint(ACCENT, 0.35))
                    .child(icon(IconName::LoaderCircle, 16., ACCENT))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("Scanning {} folders", group_digits(s.dirs))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(FAINT)
                            .text_size(px(12.))
                            .child(self.short(&s.current)),
                    ),
            );
        }

        // Stat cards.
        let stat = |id: &'static str,
                    ic: IconName,
                    color: Rgba,
                    value: String,
                    label: &'static str,
                    filter: Filter,
                    cx: &mut Context<Self>| {
            card()
                .id(id)
                .flex_1()
                .min_w(px(140.))
                .p_4()
                .gap_3()
                .flex()
                .items_center()
                .cursor_pointer()
                .hover(|s| s.bg(CARD_HOVER))
                .child(
                    div()
                        .size(px(40.))
                        .flex_none()
                        .rounded(px(10.))
                        .bg(tint(color, 0.15))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(ic, 20., color)),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(22.))
                                .whitespace_nowrap()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(value),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(MUTED)
                                .truncate()
                                .child(label),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter = filter;
                    cx.notify();
                }))
        };
        body = body.child(
            h_flex()
                .flex_wrap()
                .gap_3()
                .child(stat(
                    "stat-free",
                    IconName::HardDrive,
                    GREEN,
                    human_size(stats.safe_bytes),
                    "Space you can free",
                    Filter::SafeToRemove,
                    cx,
                ))
                .child(stat(
                    "stat-safe",
                    IconName::CircleCheck,
                    GREEN,
                    stats.safe.to_string(),
                    "Safe to remove",
                    Filter::SafeToRemove,
                    cx,
                ))
                .child(stat(
                    "stat-unsaved",
                    IconName::TriangleAlert,
                    ORANGE,
                    stats.unsaved.to_string(),
                    "Have unsaved work",
                    Filter::UnsavedWork,
                    cx,
                ))
                .child(stat(
                    "stat-all",
                    IconName::FolderSearch,
                    ACCENT,
                    stats.strays.to_string(),
                    "Leftovers found",
                    Filter::Strays,
                    cx,
                )),
        );

        // Call to action, or the bulk bar while repos are ticked.
        let checked = self.lib.checked_paths();
        if !checked.is_empty() {
            let bytes: u64 = checked
                .iter()
                .filter_map(|p| self.lib.get(p).and_then(|r| r.size_bytes))
                .sum();
            let n = checked.len();
            body = body.child(
                h_flex()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded(px(12.))
                    .bg(ELEVATED)
                    .child(icon(IconName::ListChecks, 18., ACCENT))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("{n} selected, {}", human_size(bytes))),
                    )
                    .child(
                        button("clear-sel", "Clear", ButtonKind::Ghost).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.lib.checked.clear();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        icon_button("recycle-sel", IconName::Trash, "Recycle selected", ButtonKind::Danger)
                            .tip("Move the ticked items to the Recycle Bin. You'll see exactly what's in them first.")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.ask_recycle(checked.clone());
                                cx.notify();
                            })),
                    ),
            );
        } else if self.filter == Filter::Unreadable && stats.unreadable > 0 {
            body = body.child(self.unreadable_guide(now, cx));
        } else if stats.safe > 0 && self.scan.is_none() {
            let safe = self.lib.safe_paths(&self.cfg, now);
            body = body.child(
                h_flex()
                    .gap_4()
                    .px_5()
                    .py_4()
                    .rounded(px(12.))
                    .bg(tint(ACCENT, 0.14))
                    .border_1()
                    .border_color(tint(ACCENT, 0.4))
                    .child(icon(IconName::Sparkles, 24., ACCENT))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!(
                                        "{} repo{} can go, freeing {}",
                                        stats.safe,
                                        if stats.safe == 1 { "" } else { "s" },
                                        human_size(stats.safe_bytes)
                                    )),
                            )
                            .child(
                                div().text_color(MUTED).text_size(px(13.)).child(
                                    "Everything in them is already saved online and they haven't been used in a while. They go to the Recycle Bin, so you can undo.",
                                ),
                            ),
                    )
                    .child(
                        icon_button("clean-all", IconName::BroomSparkles, "Clean up", ButtonKind::Primary)
                            .h(px(40.))
                            .px_5()
                            .tip("Review the safe ones and move them to the Recycle Bin")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.ask_recycle(safe.clone());
                                cx.notify();
                            })),
                    ),
            );
        }

        // The list.
        if rows.is_empty() {
            let (title, sub) = if self.scan.is_some() {
                (
                    "Looking for project copies...",
                    "Results appear here as they're found.",
                )
            } else if self.lib.is_empty() {
                (
                    "Nothing found yet",
                    "Run a scan to look for copies of projects that AI tools left behind.",
                )
            } else if !self.search_text.is_empty() {
                ("No matches", "Try a different search.")
            } else {
                (
                    "All clear",
                    "Nothing in this view. Try another filter on the left.",
                )
            };
            body = body.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(icon(IconName::ShieldCheck, 40., FAINT))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(div().text_color(MUTED).child(sub)),
            );
        } else {
            let count = rows.len();
            let rows = std::rc::Rc::new(rows);
            body = body.child(
                uniform_list(
                    "repos",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        let now = SystemTime::now();
                        range
                            .filter_map(|i| rows.get(i).cloned())
                            .map(|p| this.repo_row(p, now, cx).into_any_element())
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.list_scroll)
                .flex_1()
                // Room for the toast so it never covers the last card.
                .pb(px(72.)),
            );
        }
        body
    }

    /// How to decide about repos git couldn't read, plus a shortcut to tick
    /// the ones in temp folders.
    fn unreadable_guide(
        &self,
        now: SystemTime,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let temp = self.lib.unreadable_temp_paths(&self.cfg, now);
        let n = temp.len();
        let tip_row = |ic: IconName, color: Rgba, text: &'static str| {
            h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .child(div().pt(px(2.)).child(icon(ic, 14., color)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(MUTED)
                        .text_size(px(13.))
                        .child(text),
                )
        };
        h_flex()
            .items_start()
            .gap_4()
            .px_5()
            .py_4()
            .rounded(px(12.))
            .bg(tint(BLUE, 0.10))
            .border_1()
            .border_color(tint(BLUE, 0.35))
            .child(icon(IconName::Info, 22., BLUE))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("How to tell if these are safe to remove"),
                    )
                    .child(tip_row(
                        IconName::CircleQuestionMark,
                        RED,
                        "The app couldn't look inside these, so it can't tell whether they hold unsaved work. They're never marked safe on their own.",
                    ))
                    .child(tip_row(
                        IconName::CircleCheck,
                        GREEN,
                        "In a temp folder (tagged \"Temp folder\"): almost always a throwaway copy an agent made. Usually fine to remove.",
                    ))
                    .child(tip_row(
                        IconName::FolderOpen,
                        ORANGE,
                        "Anywhere else: hover the card and open the folder. If you see files you wrote or changed, keep it or copy them out first.",
                    ))
                    .child(tip_row(
                        IconName::ArchiveRestore,
                        MUTED,
                        "Removing moves them to the Recycle Bin, so you can restore anything you change your mind about.",
                    )),
            )
            .when(n > 0, |d| {
                d.child(
                    icon_button(
                        "select-temp",
                        IconName::ListChecks,
                        format!("Select {n} in temp folders"),
                        ButtonKind::Primary,
                    )
                    .flex_none()
                    .tip("Tick the ones in temp folders. Nothing is removed until you press Recycle selected and confirm.")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.lib.checked.extend(temp.iter().cloned());
                        cx.notify();
                    })),
                )
            })
    }

    fn repo_row(
        &self,
        path: PathBuf,
        now: SystemTime,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let Some(r) = self.lib.get(&path) else {
            return div().h(px(ROW_H));
        };
        let v = r.verdict(&self.cfg, now);
        let color = verdict_color(v);
        let focused = self.focused.as_ref() == Some(&path);
        let checked = self.lib.checked.contains(&path);
        let summary = r.git_summary();
        let p_check = path.clone();
        let p_open = path.clone();
        let p_term = path.clone();
        let p_bin = path.clone();
        let p_focus = path.clone();
        let removable = v != Verdict::InProject && v != Verdict::Checking;
        let temp_hint = v == Verdict::Unreadable && r.in_temp_folder();

        let checkbox = div()
            .id(SharedString::from(format!("chk-{}", path.display())))
            .size(px(18.))
            .flex_none()
            .rounded(px(5.))
            .border_1()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(checked, |d| d.bg(ACCENT).border_color(ACCENT))
            .when(!checked, |d| {
                d.border_color(FAINT).hover(|s| s.border_color(MUTED))
            })
            .when(checked, |d| {
                d.child(icon(IconName::Check, 13., hex_white()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.lib.toggle_checked(&p_check);
                cx.stop_propagation();
                cx.notify();
            }));

        let row = h_flex()
            .id(SharedString::from(format!("row-{}", path.display())))
            .group("repo-row")
            .relative()
            .h(px(ROW_H - 12.))
            .gap_3()
            .px_4()
            .rounded(px(12.))
            .border_1()
            .cursor_pointer()
            .bg(CARD)
            .border_color(if focused { ACCENT } else { tint(BORDER, 0.0) })
            .hover(|s| s.bg(CARD_HOVER))
            .child(checkbox)
            .child(
                div()
                    .size(px(40.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(tint(color, 0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(verdict_icon(v), 18., color)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .max_w(relative(0.75))
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(r.name()),
                            )
                            .when(temp_hint, |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("tmp-{}", path.display())))
                                        .flex_none()
                                        .px_1p5()
                                        .rounded(px(5.))
                                        .bg(tint(GREEN, 0.14))
                                        .text_color(GREEN)
                                        .text_size(px(11.))
                                        .child("Temp folder")
                                        .tip("It's in a temp folder, where agents put throwaway copies. Almost always fine to remove."),
                                )
                            })
                            .when(!summary.is_empty(), |d| {
                                d.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(if r.has_local_work() { ORANGE } else { FAINT })
                                        .child(summary.clone()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(FAINT)
                            .child(self.short(&r.path)),
                    ),
            )
            .child(
                v_flex()
                    .flex_none()
                    .w(px(110.))
                    .items_end()
                    .gap_1()
                    .child(meta(
                        IconName::HardDrive,
                        r.size_bytes.map(human_size).unwrap_or_else(|| "...".into()),
                    ))
                    .child(meta(
                        IconName::Clock,
                        r.last_activity
                            .map(|t| relative_time(t, now))
                            .unwrap_or_else(|| "never".into()),
                    )),
            )
            .child(
                div()
                    .w(px(140.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(status_pill(v)),
            )
            .child(
                h_flex()
                    .absolute()
                    .right(px(12.))
                    .top(px(-12.))
                    .p_0p5()
                    .gap_0p5()
                    .rounded(px(8.))
                    .bg(ELEVATED)
                    .border_1()
                    .border_color(BORDER)
                    .shadow_md()
                    .invisible()
                    .group_hover("repo-row", |s| s.visible())
                    .child(
                        icon_action(
                            SharedString::from(format!("open-{}", path.display())),
                            IconName::FolderOpen,
                            "Open folder",
                            false,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            agent_cleanup::actions::open_folder(&p_open);
                            cx.stop_propagation();
                        })),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("term-{}", path.display())),
                            IconName::SquareTerminal,
                            "Open a command window here (for git users)",
                            false,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            agent_cleanup::actions::open_terminal(&p_term);
                            cx.stop_propagation();
                        })),
                    )
                    .when(removable, |d| {
                        d.child(
                            icon_action(
                                SharedString::from(format!("bin-{}", path.display())),
                                IconName::Trash,
                                "Move to Recycle Bin",
                                true,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.ask_recycle(vec![p_bin.clone()]);
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.focused = if this.focused.as_ref() == Some(&p_focus) {
                    None
                } else {
                    Some(p_focus.clone())
                };
                cx.notify();
            }));
        div().h(px(ROW_H)).pt(px(12.)).child(row)
    }
}
