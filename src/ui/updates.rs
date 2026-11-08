//! The Updates page: outdated packages in your projects, with a button that
//! opens a terminal running the update.

use super::app::{AgentCleanup, Page};
use super::theme::*;
use super::trail::empty_state;
use super::widgets::*;
use agent_cleanup::deps::{Eco, Outdated, Report};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Packages shown per card before "and N more".
const SHOWN: usize = 8;

fn eco_color(eco: Eco) -> Rgba {
    match eco {
        Eco::Npm => RED,
        Eco::Cargo => ORANGE,
        Eco::Pip => BLUE,
    }
}

fn package_row(p: &Outdated) -> Div {
    let version = |text: String, color: Rgba| {
        div()
            .flex_none()
            .text_size(px(12.))
            .text_color(color)
            .child(text)
    };
    let mut row = h_flex()
        .gap_2()
        .py(px(5.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .child(p.name.clone()),
        )
        .child(version(p.current.clone(), FAINT));
    if p.updatable() {
        row = row
            .child(icon(IconName::ArrowRight, 12., FAINT))
            .child(version(p.wanted.clone(), GREEN));
    }
    if let Some(latest) = p.latest.as_ref().filter(|_| p.held_back()) {
        row = row.child(
            div()
                .id(SharedString::from(format!("major-{}", p.name)))
                .flex_none()
                .px_1p5()
                .rounded(px(5.))
                .bg(tint(ORANGE, 0.14))
                .text_size(px(11.))
                .text_color(ORANGE)
                .child(format!("{latest} available"))
                .tip("A bigger update that might break things until the code is changed to match. The Update button skips it; do it by hand, or ask your agent, when you're ready."),
        );
    }
    row
}

impl AgentCleanup {
    pub fn updates_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let d = &self.deps;
        let loading = d.loading.is_some();
        let check = icon_button(
            "deps-check",
            IconName::RefreshCw,
            if loading {
                "Checking..."
            } else {
                "Check again"
            },
            ButtonKind::Secondary,
        )
        .tip("Ask npm, cargo and pip what's outdated. Nothing is changed.")
        .on_click(cx.listener(|this, _, _, cx| {
            this.load_deps();
            cx.notify();
        }));
        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Updates",
                "Newer versions of the code libraries your projects use (npm, Cargo and Python packages). Mostly useful if you write code. Update opens a window that runs the update, so you can watch it.",
            )))
            .child(check);

        let note = h_flex()
            .gap_2()
            .text_size(px(12.))
            .text_color(FAINT)
            .child(icon(IconName::ShieldCheck, 13., FAINT))
            .child(div().flex_1().min_w_0().child(
                "Only projects in your project folders are checked, since checking runs their own tools. Update only starts when the project has no unsaved edits, so whatever the update changes can be seen and undone with git.",
            ));

        let body: AnyElement = if self.cfg.project_roots.is_empty() {
            card()
                .child(
                    empty_state(
                        IconName::FolderHeart,
                        "Add a project folder first",
                        "Updates are checked only in your own projects. Add the folder they live in on the Projects page.".into(),
                    )
                    .child(
                        button("deps-go-projects", "Go to Projects", ButtonKind::Secondary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.go(Page::Projects);
                                cx.notify();
                            })),
                    ),
                )
                .into_any_element()
        } else if !d.loaded {
            card()
                .child(empty_state(
                    IconName::Hourglass,
                    "Checking your projects...",
                    format!(
                        "{} of {} projects done. npm and cargo ask their registries, so this needs the internet.",
                        self.deps_progress(),
                        d.repos
                    ),
                ))
                .into_any_element()
        } else if d.reports.is_empty() {
            card()
                .child(empty_state(
                    IconName::CircleCheck,
                    "Everything's up to date",
                    format!(
                        "Checked {} project{}. Only projects with node_modules installed, a Cargo.lock, or a Python venv can be checked.",
                        d.repos,
                        if d.repos == 1 { "" } else { "s" }
                    ),
                ))
                .into_any_element()
        } else {
            let mut cards = v_flex().gap_4();
            for r in &d.reports {
                cards = cards.child(self.report_card(r, cx));
            }
            cards.into_any_element()
        };

        div().id("updates").size_full().overflow_y_scroll().child(
            v_flex()
                .w_full()
                .max_w(px(900.))
                .mx_auto()
                .px_8()
                .pt_8()
                .pb(px(80.))
                .gap_5()
                .child(header)
                .child(note)
                .child(body),
        )
    }

    fn report_card(&self, r: &Report, cx: &mut Context<Self>) -> Div {
        let color = eco_color(r.eco);
        let key = format!("{}-{:?}", r.repo.display(), r.eco);
        let name = r
            .repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let summary = if let Some(e) = &r.error {
            format!("Couldn't check: {e}")
        } else {
            let (up, held) = (r.updatable(), r.held_back());
            let mut parts = Vec::new();
            if up > 0 {
                parts.push(format!("{up} can update"));
            }
            if held > 0 {
                parts.push(format!(
                    "{held} bigger update{} by hand",
                    if held == 1 { "" } else { "s" }
                ));
            }
            parts.join(", ")
        };
        let action = r.update_command().map(|cmd| {
            let (repo, eco) = (r.repo.clone(), r.eco);
            icon_button(
                SharedString::from(format!("upd-{key}")),
                IconName::SquareTerminal,
                "Update",
                ButtonKind::Primary,
            )
            .tip(format!("Opens a terminal in this project running: {cmd}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_update(&repo, eco);
                cx.notify();
            }))
        });
        let prompt = {
            let pk: Vec<_> = r
                .packages
                .iter()
                .map(|p| agent_cleanup::prompts::Package {
                    name: &p.name,
                    current: &p.current,
                    target: p.latest.as_deref().unwrap_or(&p.wanted),
                })
                .collect();
            agent_cleanup::prompts::updates(&r.repo, r.eco.label(), &pk)
        };
        let prompt_btn = prompt.is_some().then(|| {
            prompt_button(SharedString::from(format!("upd-prompt-{key}"))).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.copy_prompt(prompt.clone(), cx);
                    cx.notify();
                },
            ))
        });
        let mut list = v_flex().px_4().pb_3();
        for p in r.packages.iter().take(SHOWN) {
            list = list.child(package_row(p));
        }
        if r.packages.len() > SHOWN {
            list = list.child(
                div()
                    .pt_1()
                    .text_size(px(12.))
                    .text_color(FAINT)
                    .child(format!("...and {} more", r.packages.len() - SHOWN)),
            );
        }
        card()
            .child(
                h_flex()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .size(px(36.))
                            .flex_none()
                            .rounded(px(10.))
                            .bg(tint(color, 0.14))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::Package, 18., color)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(color)
                                            .child(r.eco.label()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(if r.error.is_some() { RED } else { MUTED })
                                    .child(summary),
                            ),
                    )
                    .children(prompt_btn)
                    .children(action),
            )
            .when(!r.packages.is_empty(), |c| {
                c.child(div().h(px(1.)).bg(BORDER).mx_4())
                    .child(list.pt_2())
            })
    }
}
