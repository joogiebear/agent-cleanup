//! The Downloads page: what's in the Downloads folder, grouped by the project
//! each file was probably for.

use super::app::AgentCleanup;
use super::theme::*;
use super::trail::empty_state;
use super::widgets::*;
use agent_cleanup::downloads::{self, Download, DownloadAdvice};
use agent_cleanup::model::{human_size, relative_time};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::SystemTime;

/// An icon for the file type, from its extension.
pub fn download_icon(d: &Download) -> IconName {
    if d.is_dir {
        return IconName::FolderClosed;
    }
    let ext = d
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "zip" | "7z" | "rar" | "gz" | "tgz" | "tar" | "xz" | "bz2" | "zst" => IconName::FileArchive,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => IconName::FileImage,
        "csv" | "xlsx" | "xls" | "tsv" | "parquet" => IconName::FileSpreadsheet,
        "mp3" | "wav" | "flac" | "ogg" => IconName::FileMusic,
        "mp4" | "mov" | "mkv" | "webm" => IconName::FilePlay,
        "iso" | "img" | "dmg" => IconName::Disc,
        "exe" | "msi" | "msix" => IconName::Package,
        "json" | "yaml" | "yml" | "toml" | "js" | "ts" | "py" | "rs" | "sh" | "ps1" | "sql"
        | "html" | "xml" => IconName::FileCode,
        _ => IconName::FileText,
    }
}

fn advice_tag(id: impl Into<ElementId>, a: DownloadAdvice, text: String) -> Stateful<Div> {
    let (color, ic, tip) = match a {
        DownloadAdvice::Safe => (
            GREEN,
            IconName::CircleCheck,
            "An installer that arrived over a week ago. Not needed once the program is installed.",
        ),
        DownloadAdvice::Move => (
            ACCENT,
            IconName::FolderInput,
            "Looks like it was for one of your projects. Use the move button to put it there.",
        ),
        DownloadAdvice::YourCall => (
            MUTED,
            IconName::CircleQuestionMark,
            "The app can't tell whether you need it. Open it to check.",
        ),
    };
    verdict_tag(id, color, ic, text, tip)
}

impl AgentCleanup {
    pub fn downloads_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let d = &self.downloads;
        let loading = d.loading.is_some();
        let dir_label = d
            .dir
            .as_deref()
            .map(|p| self.short(p))
            .unwrap_or_else(|| "your Downloads folder".into());

        let refresh = icon_button(
            "dl-refresh",
            IconName::RefreshCw,
            if loading { "Looking..." } else { "Refresh" },
            ButtonKind::Secondary,
        )
        .tip("List the Downloads folder and match it again")
        .on_click(cx.listener(|this, _, _, cx| {
            this.load_downloads();
            cx.notify();
        }));
        let open = d.dir.clone().map(|dir| {
            icon_button(
                "dl-open",
                IconName::FolderOpen,
                "Open folder",
                ButtonKind::Secondary,
            )
            .tip("Open the Downloads folder in File Explorer")
            .on_click(cx.listener(move |_, _, _, _| {
                agent_cleanup::actions::open_folder(&dir);
            }))
        });

        let header = h_flex()
            .items_end()
            .gap_3()
            .child(div().flex_1().min_w_0().child(page_header(
                "Downloads",
                "Files in your Downloads folder, each with a guess at which project it was for, so you can move it there or recycle it.",
            )))
            .children(open)
            .child(refresh);

        let summary = (d.loaded && !d.items.is_empty()).then(|| {
            let total: u64 = d.items.iter().map(|i| i.size_bytes).sum();
            let advice: Vec<_> = d.items.iter().map(|i| i.advice().0).collect();
            let count = |a: DownloadAdvice| advice.iter().filter(|x| **x == a).count();
            let (n_safe, n_move, n_yours) = (
                count(DownloadAdvice::Safe),
                count(DownloadAdvice::Move),
                count(DownloadAdvice::YourCall),
            );
            let safe: Vec<_> = d
                .items
                .iter()
                .filter(|i| i.advice().0 == DownloadAdvice::Safe)
                .map(|i| i.path.clone())
                .collect();
            let safe_bytes: u64 = d
                .items
                .iter()
                .filter(|i| i.advice().0 == DownloadAdvice::Safe)
                .map(|i| i.size_bytes)
                .sum();
            let headline = if n_safe > 0 {
                format!(
                    "{n_safe} installer{} can go, freeing {}",
                    if n_safe == 1 { "" } else { "s" },
                    human_size(safe_bytes)
                )
            } else if n_move > 0 {
                format!(
                    "{n_move} download{} look{} like {} for a project",
                    if n_move == 1 { "" } else { "s" },
                    if n_move == 1 { "s" } else { "" },
                    if n_move == 1 { "it's" } else { "they're" }
                )
            } else {
                "Nothing here is clearly junk".to_string()
            };
            let text = "Installers aren't needed once the program is installed. Files that look like they were for a project can be moved into it with one click (and moved back). The rest only you can judge: open them, then keep, hide or recycle. Anything recycled goes to the Recycle Bin first.";
            let mut tags = h_flex().flex_wrap().gap_2();
            for (a, n) in [
                (DownloadAdvice::Safe, n_safe),
                (DownloadAdvice::Move, n_move),
                (DownloadAdvice::YourCall, n_yours),
            ] {
                if n > 0 {
                    tags = tags.child(advice_tag(
                        SharedString::from(format!("dl-sum-{a:?}")),
                        a,
                        format!("{n} {}", a.label().to_lowercase()),
                    ));
                }
            }
            let action = (n_safe > 0 && !loading).then(|| {
                icon_button(
                    "dl-clean",
                    IconName::BroomSparkles,
                    format!("Clean up {}", human_size(safe_bytes)),
                    ButtonKind::Primary,
                )
                .tip("Recycle the installers marked Safe to clean. You'll see the list first.")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_recycle_downloads(safe.clone());
                    cx.notify();
                }))
            });
            v_flex()
                .gap_2()
                .child(advice_banner(
                    if n_safe > 0 { GREEN } else { ACCENT },
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
                            IconName::Download,
                            format!(
                                "{} download{} in total, {}",
                                d.items.len(),
                                if d.items.len() == 1 { "" } else { "s" },
                                human_size(total)
                            ),
                        ))
                        .child(meta(
                            IconName::Clock,
                            format!("Arrived in the last {} days", downloads::DAYS),
                        )),
                )
        });

        let body: AnyElement = if !d.loaded {
            card()
                .child(empty_state(
                    IconName::Hourglass,
                    "Looking at your downloads...",
                    format!(
                        "Listing {dir_label} and comparing when each file arrived with what you and your AI agents were working on. Nothing is changed."
                    ),
                ))
                .into_any_element()
        } else if d.dir.is_none() {
            card()
                .child(empty_state(
                    IconName::Download,
                    "No Downloads folder",
                    "Windows didn't report a Downloads folder for this account.".into(),
                ))
                .into_any_element()
        } else if d.items.is_empty() {
            card()
                .child(empty_state(
                    IconName::ShieldCheck,
                    "Nothing recent",
                    format!(
                        "Nothing arrived in {dir_label} in the last {} days, or you've hidden it.",
                        downloads::DAYS
                    ),
                ))
                .into_any_element()
        } else {
            let now = SystemTime::now();
            let mut groups = v_flex().gap_5();
            for (project, items) in downloads::by_project(&d.items) {
                let size: u64 = items.iter().map(|i| i.size_bytes).sum();
                let prompt = {
                    let entries: Vec<_> = items
                        .iter()
                        .map(|i| {
                            agent_cleanup::prompts::Entry::new(
                                i.path.clone(),
                                if i.looks_private() {
                                    Some("looks private (passwords, a key or data): keep it out of project folders".into())
                                } else {
                                    i.matched.as_ref().map(|m| m.reason())
                                },
                            )
                        })
                        .collect();
                    agent_cleanup::prompts::downloads_group(
                        (!project.as_os_str().is_empty()).then_some(project.as_path()),
                        &entries,
                    )
                };
                let pkey = project.display().to_string();
                let (label, tip) = if project.as_os_str().is_empty() {
                    (
                        "No clear project".to_string(),
                        "Nothing was happening in your projects when these arrived, or two projects were equally busy.".to_string(),
                    )
                } else {
                    (
                        format!(
                            "Probably for {}",
                            project
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| self.short(&project))
                        ),
                        format!(
                            "These arrived while you or an agent were working in {}.",
                            self.short(&project)
                        ),
                    )
                };
                let mut list = v_flex();
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        list = list.child(div().h(px(1.)).bg(BORDER).mx_4());
                    }
                    list = list.child(self.download_row(it, now, cx));
                }
                groups = groups.child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .id(SharedString::from(format!(
                                    "dl-group-{}",
                                    project.display()
                                )))
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
                                    prompt_button(SharedString::from(format!("dl-prompt-{pkey}")))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy_prompt(prompt.clone(), cx);
                                            cx.notify();
                                        })),
                                )
                                .tip(tip),
                        )
                        .child(card().child(list)),
                );
            }
            groups.into_any_element()
        };

        div().id("downloads").size_full().overflow_y_scroll().child(
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

    fn download_row(
        &self,
        it: &Download,
        now: SystemTime,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = it.path.display().to_string();
        let (p_move, p_open, p_hide, p_bin) = (
            it.path.clone(),
            it.path.clone(),
            it.path.clone(),
            it.path.clone(),
        );
        let color = if it.matched.is_some() { ACCENT } else { MUTED };
        let (advice, why) = it.advice();
        let mut facts = h_flex()
            .flex_wrap()
            .gap_x_3()
            .gap_y_1()
            .child(meta(IconName::HardDrive, human_size(it.size_bytes)))
            .child(meta(
                IconName::Clock,
                format!("Arrived {}", relative_time(it.arrived, now)),
            ));
        if let Some(m) = &it.matched {
            facts = facts.child(meta(IconName::FolderInput, m.reason()));
        }

        let move_btn = it.destination().map(|dest| {
            let taken = dest.exists();
            let tip = if taken {
                format!("Can't move: {} already exists", self.short(&dest))
            } else {
                format!("Move into {}", self.short(&dest))
            };
            let btn = icon_action(
                SharedString::from(format!("dl-move-{key}")),
                IconName::FolderInput,
                tip,
                false,
            );
            if taken {
                disabled(btn)
            } else {
                btn.on_click(cx.listener(move |this, _, _, cx| {
                    this.move_download(&p_move);
                    cx.notify();
                }))
            }
        });

        h_flex()
            .id(SharedString::from(format!("dl-{key}")))
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
                    .child(icon(download_icon(it), 18., color)),
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
                            .child(advice_tag(
                                SharedString::from(format!("dl-tag-{key}")),
                                advice,
                                advice.label().to_string(),
                            )),
                    )
                    .child(div().text_size(px(12.)).text_color(MUTED).child(why))
                    .child(facts),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_0p5()
                    .children(move_btn)
                    .child(
                        icon_action(
                            SharedString::from(format!("dl-open-{key}")),
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
                            SharedString::from(format!("dl-hide-{key}")),
                            IconName::EyeOff,
                            "Hide: keep it and stop showing it here",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.hide_download(p_hide.clone());
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_action(
                            SharedString::from(format!("dl-bin-{key}")),
                            IconName::Trash,
                            "Move to Recycle Bin",
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_recycle_downloads(vec![p_bin.clone()]);
                            cx.notify();
                        })),
                    ),
            )
    }
}
