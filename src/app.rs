//! The egui window: settings on the left, found repos in a table on the right.

use agent_cleanup::actions;
use agent_cleanup::config::{Config, path_within};
use agent_cleanup::model::{Repo, Verdict, human_size, relative_time};
use agent_cleanup::scan::{self, ScanEvent};
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    Strays,
    SafeToRemove,
    UnsavedWork,
    All,
}

impl Filter {
    const ALL: [Filter; 4] = [
        Filter::Strays,
        Filter::SafeToRemove,
        Filter::UnsavedWork,
        Filter::All,
    ];

    fn label(self) -> &'static str {
        match self {
            Filter::Strays => "Outside projects",
            Filter::SafeToRemove => "Safe to remove",
            Filter::UnsavedWork => "Has unsaved work",
            Filter::All => "Everything",
        }
    }

    fn keeps(self, v: Verdict, stray: bool) -> bool {
        match self {
            Filter::Strays => stray,
            Filter::SafeToRemove => v == Verdict::SafeToRemove,
            Filter::UnsavedWork => v == Verdict::UnsavedWork,
            Filter::All => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Status,
    Name,
    Size,
    Activity,
}

/// Something the user clicked in the table, applied after drawing it.
enum RowAction {
    Open(PathBuf),
    AskRecycle(PathBuf),
    AddAsProject(PathBuf),
    Ignore(PathBuf),
    CopyPath(PathBuf),
}

struct Scan {
    rx: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
    dirs: u64,
}

pub struct App {
    cfg: Config,
    repos: Vec<Repo>,
    index: HashMap<PathBuf, usize>,
    scan: Option<Scan>,
    last_scan: Option<String>,
    filter: Filter,
    search: String,
    sort_by: SortBy,
    sort_desc: bool,
    /// Repo waiting for the user to confirm "move to Recycle Bin".
    confirm_recycle: Option<PathBuf>,
    confirm_ack: bool,
    status: Option<(String, bool)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut app = Self {
            cfg: Config::load(),
            repos: Vec::new(),
            index: HashMap::new(),
            scan: None,
            last_scan: None,
            filter: Filter::Strays,
            search: String::new(),
            sort_by: SortBy::Size,
            sort_desc: true,
            confirm_recycle: None,
            confirm_ack: false,
            status: None,
        };
        app.start_scan(&cc.egui_ctx);
        app
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        if let Some(old) = &self.scan {
            old.cancel.store(true, Ordering::Relaxed);
        }
        self.repos.clear();
        self.index.clear();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let ctx = ctx.clone();
        scan::start(
            self.cfg.clone(),
            tx,
            cancel.clone(),
            Arc::new(move || ctx.request_repaint()),
        );
        self.scan = Some(Scan {
            rx,
            cancel,
            dirs: 0,
        });
    }

    fn poll_scan(&mut self) {
        let Some(scan) = &mut self.scan else { return };
        let mut finished = None;
        while let Ok(ev) = scan.rx.try_recv() {
            match ev {
                ScanEvent::Progress { dirs } => scan.dirs = dirs,
                ScanEvent::Found(path) => {
                    if !self.cfg.is_ignored(&path) && !self.index.contains_key(&path) {
                        self.index.insert(path.clone(), self.repos.len());
                        self.repos.push(Repo::new(path, &self.cfg));
                    }
                }
                ScanEvent::Inspected(repo) => {
                    if let Some(&i) = self.index.get(&repo.path) {
                        self.repos[i] = *repo;
                    }
                }
                ScanEvent::Done {
                    dirs,
                    elapsed,
                    cancelled,
                } => finished = Some((dirs, elapsed, cancelled)),
            }
        }
        if let Some((dirs, elapsed, cancelled)) = finished {
            self.last_scan = Some(if cancelled {
                "Scan stopped".to_string()
            } else {
                format!(
                    "Scanned {} folders in {:.1}s",
                    group_digits(dirs),
                    elapsed.as_secs_f32()
                )
            });
            self.scan = None;
        }
    }

    fn save_config(&mut self) {
        if let Err(e) = self.cfg.save() {
            self.status = Some((format!("Couldn't save settings: {e}"), true));
        }
        // Project folders may have changed; re-classify without rescanning.
        for r in &mut self.repos {
            r.project_root = self.cfg.project_root_for(&r.path).cloned();
        }
    }

    fn remove_repo(&mut self, path: &Path) {
        self.repos.retain(|r| r.path != path);
        self.index = self
            .repos
            .iter()
            .enumerate()
            .map(|(i, r)| (r.path.clone(), i))
            .collect();
    }

    fn apply(&mut self, action: RowAction, ctx: &egui::Context) {
        match action {
            RowAction::Open(p) => actions::open_folder(&p),
            RowAction::AskRecycle(p) => {
                self.confirm_recycle = Some(p);
                self.confirm_ack = false;
            }
            RowAction::AddAsProject(p) => {
                self.cfg.project_roots.push(p);
                self.save_config();
            }
            RowAction::Ignore(p) => {
                self.remove_repo(&p);
                self.status = Some((
                    format!("Hidden {}. You can undo this in Settings.", p.display()),
                    false,
                ));
                self.cfg.ignored.push(p);
                self.save_config();
            }
            RowAction::CopyPath(p) => ctx.copy_text(p.display().to_string()),
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Your project folders");
            ui.label(
                RichText::new("Repos inside these are yours. Anything found elsewhere is a possible leftover.")
                    .small()
                    .weak(),
            );
            changed |= folder_list(ui, "projects", &mut self.cfg.project_roots, "Add project folder…");
            ui.add_space(12.0);

            ui.heading("Where to look");
            changed |= folder_list(ui, "scan", &mut self.cfg.scan_roots, "Add scan location…");
            ui.add_space(12.0);

            ui.heading("Stale after");
            changed |= ui
                .add(egui::Slider::new(&mut self.cfg.stale_after_days, 1..=365).suffix(" days"))
                .changed();
            ui.add_space(12.0);

            if !self.cfg.ignored.is_empty() {
                ui.collapsing(format!("Hidden repos ({})", self.cfg.ignored.len()), |ui| {
                    changed |= folder_list(ui, "ignored", &mut self.cfg.ignored, "");
                });
            }
            ui.add_space(12.0);
            if let Some(path) = Config::data_dir()
                && ui.small_button("Open settings and action log folder").clicked() {
                    let _ = std::fs::create_dir_all(&path);
                    actions::open_folder(&path);
                }
        });
        if changed {
            self.save_config();
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(scan) = &self.scan {
                if ui.button("Stop").clicked() {
                    scan.cancel.store(true, Ordering::Relaxed);
                }
                ui.spinner();
                let pending = self.repos.iter().filter(|r| !r.inspected).count();
                ui.label(format!(
                    "Looked in {} folders, found {} repos{}",
                    group_digits(scan.dirs),
                    self.repos.len(),
                    if pending > 0 {
                        format!(", checking {pending}")
                    } else {
                        String::new()
                    }
                ));
            } else {
                if ui.button("Rescan").clicked() {
                    let ctx = ui.ctx().clone();
                    self.start_scan(&ctx);
                }
                if let Some(s) = &self.last_scan {
                    ui.label(RichText::new(s).weak());
                }
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let now = SystemTime::now();
            for f in Filter::ALL {
                let count = self
                    .repos
                    .iter()
                    .filter(|r| f.keeps(r.verdict(&self.cfg, now), r.is_stray()))
                    .count();
                if ui
                    .selectable_label(self.filter == f, format!("{} ({count})", f.label()))
                    .clicked()
                {
                    self.filter = f;
                }
            }
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Filter by name or path")
                    .desired_width(220.0),
            );
        });
    }

    fn summary(&self, ui: &mut egui::Ui) {
        let now = SystemTime::now();
        let (mut stray_n, mut stray_bytes, mut safe_n, mut safe_bytes) = (0, 0u64, 0, 0u64);
        for r in &self.repos {
            if r.is_stray() {
                stray_n += 1;
                stray_bytes += r.size_bytes.unwrap_or(0);
            }
            if r.verdict(&self.cfg, now) == Verdict::SafeToRemove {
                safe_n += 1;
                safe_bytes += r.size_bytes.unwrap_or(0);
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{stray_n} repos outside your projects")).strong());
            ui.label(format!("using {}.", human_size(stray_bytes)));
            ui.label(
                RichText::new(format!(
                    "{safe_n} look safe to remove ({}).",
                    human_size(safe_bytes)
                ))
                .color(verdict_color(Verdict::SafeToRemove)),
            );
        });
        if self.cfg.project_roots.is_empty() {
            ui.label(
                RichText::new("Tip: add your project folders on the left so the app can tell your work from leftovers.")
                    .color(Color32::from_rgb(230, 190, 90)),
            );
        }
    }

    fn visible_rows(&self) -> Vec<usize> {
        let now = SystemTime::now();
        let needle = self.search.to_lowercase();
        let mut rows: Vec<usize> = (0..self.repos.len())
            .filter(|&i| {
                let r = &self.repos[i];
                self.filter.keeps(r.verdict(&self.cfg, now), r.is_stray())
                    && (needle.is_empty()
                        || r.path.to_string_lossy().to_lowercase().contains(&needle))
            })
            .collect();
        let repos = &self.repos;
        rows.sort_by(|&a, &b| {
            let (a, b) = (&repos[a], &repos[b]);
            let ord = match self.sort_by {
                SortBy::Status => a.verdict(&self.cfg, now).cmp(&b.verdict(&self.cfg, now)),
                SortBy::Name => a.name().to_lowercase().cmp(&b.name().to_lowercase()),
                SortBy::Size => a.size_bytes.cmp(&b.size_bytes),
                SortBy::Activity => a.last_activity.cmp(&b.last_activity),
            };
            if self.sort_desc { ord.reverse() } else { ord }
        });
        rows
    }

    fn table(&mut self, ui: &mut egui::Ui) -> Option<RowAction> {
        let rows = self.visible_rows();
        if rows.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(if self.scan.is_some() {
                        "Scanning…"
                    } else {
                        "Nothing here."
                    })
                    .weak(),
                );
            });
            return None;
        }
        let now = SystemTime::now();
        let mut action = None;
        let mut header_clicks: Vec<SortBy> = Vec::new();
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(120.0))
            .column(Column::remainder().at_least(180.0).clip(true))
            .column(Column::initial(75.0))
            .column(Column::initial(100.0))
            .column(Column::initial(215.0).clip(true))
            .column(Column::exact(195.0))
            .header(24.0, |mut h| {
                for (label, by) in [
                    ("Status", Some(SortBy::Status)),
                    ("Repo", Some(SortBy::Name)),
                    ("Size", Some(SortBy::Size)),
                    ("Last used", Some(SortBy::Activity)),
                    ("Git", None),
                    ("", None),
                ] {
                    h.col(|ui| match by {
                        Some(by) => {
                            let arrow = if self.sort_by == by {
                                if self.sort_desc { " v" } else { " ^" }
                            } else {
                                ""
                            };
                            let btn = egui::Button::new(
                                RichText::new(format!("{label}{arrow}")).strong(),
                            )
                            .frame(false);
                            if ui.add(btn).clicked() {
                                header_clicks.push(by);
                            }
                        }
                        None => {
                            ui.strong(label);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(38.0, rows.len(), |mut row| {
                    let r = &self.repos[rows[row.index()]];
                    let v = r.verdict(&self.cfg, now);
                    row.col(|ui| {
                        ui.label(RichText::new(v.label()).color(verdict_color(v)))
                            .on_hover_text(v.explanation());
                    });
                    row.col(|ui| {
                        ui.vertical(|ui| {
                            ui.add_space(2.0);
                            ui.label(RichText::new(r.name()).strong());
                            let mut hover = r.path.display().to_string();
                            if let Some(url) = r.git.as_ref().and_then(|g| g.remote_url.as_ref()) {
                                hover.push_str(&format!("\nRemote: {url}"));
                            }
                            if let Some(branch) = r.git.as_ref().and_then(|g| g.branch.as_ref()) {
                                hover.push_str(&format!("\nBranch: {branch}"));
                            }
                            ui.label(RichText::new(r.path.display().to_string()).small().weak())
                                .on_hover_text(hover);
                        });
                    });
                    row.col(|ui| {
                        ui.label(r.size_bytes.map(human_size).unwrap_or_default());
                    });
                    row.col(|ui| {
                        ui.label(
                            r.last_activity
                                .map(|t| relative_time(t, now))
                                .unwrap_or_default(),
                        );
                    });
                    row.col(|ui| {
                        let summary = r.git_summary();
                        let color = if r.inspected && r.has_local_work() {
                            verdict_color(Verdict::UnsavedWork)
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.label(RichText::new(&summary).color(color))
                            .on_hover_text(&summary);
                    });
                    row.col(|ui| {
                        if ui
                            .small_button("Open")
                            .on_hover_text("Open in File Explorer")
                            .clicked()
                        {
                            action = Some(RowAction::Open(r.path.clone()));
                        }
                        if ui
                            .add_enabled(r.inspected, egui::Button::new("Recycle…").small())
                            .on_hover_text("Move to Recycle Bin…")
                            .clicked()
                        {
                            action = Some(RowAction::AskRecycle(r.path.clone()));
                        }
                        ui.menu_button("More", |ui| {
                            if r.is_stray() && ui.button("Mark as a project folder").clicked() {
                                action = Some(RowAction::AddAsProject(r.path.clone()));
                            }
                            if ui.button("Hide from this list").clicked() {
                                action = Some(RowAction::Ignore(r.path.clone()));
                            }
                            if ui.button("Copy path").clicked() {
                                action = Some(RowAction::CopyPath(r.path.clone()));
                            }
                        });
                    });
                });
            });
        for by in header_clicks {
            self.toggle_sort(by);
        }
        action
    }

    fn toggle_sort(&mut self, by: SortBy) {
        if self.sort_by == by {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_by = by;
            self.sort_desc = matches!(by, SortBy::Size);
        }
    }

    fn recycle_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.confirm_recycle.clone() else {
            return;
        };
        let Some(repo) = self.repos.iter().find(|r| r.path == path).cloned() else {
            self.confirm_recycle = None;
            return;
        };
        let reasons = repo
            .git
            .as_ref()
            .map(|g| g.local_work_reasons())
            .unwrap_or_else(|| {
                vec![
                    "git state couldn't be read, so it's unknown what's only on this PC"
                        .to_string(),
                ]
            });
        let risky = repo.has_local_work();
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("confirm_recycle")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(format!("Move \"{}\" to the Recycle Bin?", repo.name()));
            ui.label(RichText::new(path.display().to_string()).weak());
            if let Some(size) = repo.size_bytes {
                ui.label(format!(
                    "Frees about {} once the Recycle Bin is emptied.",
                    human_size(size)
                ));
            }
            if let Some(root) = &repo.project_root {
                ui.colored_label(
                    Color32::from_rgb(230, 190, 90),
                    format!("This is inside your project folder {}.", root.display()),
                );
            }
            ui.add_space(8.0);
            if risky {
                ui.colored_label(
                    verdict_color(Verdict::UnsavedWork),
                    "This repo has work that isn't saved anywhere else:",
                );
                for r in &reasons {
                    ui.label(format!("  • {r}"));
                }
                ui.checkbox(
                    &mut self.confirm_ack,
                    "I understand this work only exists on this PC",
                );
            } else {
                ui.label(
                    "Everything in it is committed and pushed, so you can clone it again any time.",
                );
            }
            ui.label(
                RichText::new("You can restore it from the Recycle Bin.")
                    .small()
                    .weak(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                let ok = ui.add_enabled(
                    !risky || self.confirm_ack,
                    egui::Button::new("Move to Recycle Bin"),
                );
                if ok.clicked() {
                    match actions::recycle(&path) {
                        Ok(()) => {
                            self.status =
                                Some((format!("Moved {} to the Recycle Bin.", repo.name()), false));
                            self.remove_repo(&path);
                        }
                        Err(e) => {
                            self.status =
                                Some((format!("Couldn't move {}: {e}", repo.name()), true))
                        }
                    }
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.confirm_recycle = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_scan();
        let ctx = ui.ctx().clone();

        egui::Panel::left("settings")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                self.settings_panel(ui);
            });
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(6.0);
        });
        if let Some((msg, is_error)) = self.status.clone() {
            egui::Panel::bottom("status").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let color = if is_error {
                        Color32::from_rgb(230, 110, 100)
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.colored_label(color, msg);
                    if ui.small_button("x").clicked() {
                        self.status = None;
                    }
                });
            });
        }
        let mut action = None;
        egui::CentralPanel::default().show(ui, |ui| {
            self.summary(ui);
            ui.add_space(6.0);
            action = self.table(ui);
        });
        if let Some(a) = action {
            self.apply(a, &ctx);
        }
        self.recycle_dialog(&ctx);

        // Keep "x minutes ago" labels fresh without burning CPU.
        ctx.request_repaint_after(Duration::from_secs(30));
    }
}

/// Editable list of folders with remove buttons. Returns true if it changed.
fn folder_list(ui: &mut egui::Ui, id: &str, folders: &mut Vec<PathBuf>, add_label: &str) -> bool {
    let mut changed = false;
    let mut remove = None;
    ui.push_id(id, |ui| {
        if folders.is_empty() {
            ui.label(RichText::new("None yet").italics().weak());
        }
        for (i, f) in folders.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("x").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
                ui.add(egui::Label::new(f.display().to_string()).truncate())
                    .on_hover_text(f.display().to_string());
            });
        }
        if !add_label.is_empty()
            && ui.button(add_label).clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
            && !folders
                .iter()
                .any(|f| path_within(&dir, f) && path_within(f, &dir))
        {
            folders.push(dir);
            changed = true;
        }
    });
    if let Some(i) = remove {
        folders.remove(i);
        changed = true;
    }
    changed
}

fn verdict_color(v: Verdict) -> Color32 {
    match v {
        Verdict::SafeToRemove => Color32::from_rgb(110, 200, 120),
        Verdict::UnsavedWork => Color32::from_rgb(240, 150, 80),
        Verdict::StrayActive => Color32::from_rgb(120, 170, 240),
        Verdict::InProject => Color32::GRAY,
        Verdict::Checking => Color32::DARK_GRAY,
    }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
