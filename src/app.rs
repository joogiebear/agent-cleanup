//! The egui window.
//!
//! Layout: settings on the left, toolbar and filters on top, the repo table in
//! the middle, and a right-hand panel that shows either the selected repo's
//! details or the "How this works" guide.

use agent_cleanup::actions;
use agent_cleanup::cache::ScanCache;
use agent_cleanup::config::{Config, path_within};
use agent_cleanup::model::{Repo, Verdict, display_path, human_size, relative_time};
use agent_cleanup::scan::{self, ScanEvent};
use agent_cleanup::update::{self, Update};
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const WARN: Color32 = Color32::from_rgb(230, 190, 90);
const ERROR: Color32 = Color32::from_rgb(230, 110, 100);
/// How long a notice stays at the bottom of the window.
const NOTICE_SECS: u64 = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    Strays,
    SafeToRemove,
    UnsavedWork,
    StrayActive,
    All,
}

impl Filter {
    const ALL: [Filter; 5] = [
        Filter::Strays,
        Filter::SafeToRemove,
        Filter::UnsavedWork,
        Filter::StrayActive,
        Filter::All,
    ];

    fn label(self) -> &'static str {
        match self {
            Filter::Strays => "All leftovers",
            Filter::SafeToRemove => Verdict::SafeToRemove.label(),
            Filter::UnsavedWork => Verdict::UnsavedWork.label(),
            Filter::StrayActive => Verdict::StrayActive.label(),
            Filter::All => "Everything",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Filter::Strays => "Every repo found outside your project folders.",
            Filter::SafeToRemove => Verdict::SafeToRemove.explanation(),
            Filter::UnsavedWork => Verdict::UnsavedWork.explanation(),
            Filter::StrayActive => Verdict::StrayActive.explanation(),
            Filter::All => "Every repo found, including the ones inside your project folders.",
        }
    }

    fn color(self) -> Option<Color32> {
        match self {
            Filter::SafeToRemove => Some(verdict_color(Verdict::SafeToRemove)),
            Filter::UnsavedWork => Some(verdict_color(Verdict::UnsavedWork)),
            Filter::StrayActive => Some(verdict_color(Verdict::StrayActive)),
            _ => None,
        }
    }

    fn keeps(self, v: Verdict, stray: bool) -> bool {
        match self {
            Filter::Strays => stray,
            Filter::SafeToRemove => v == Verdict::SafeToRemove,
            Filter::UnsavedWork => v == Verdict::UnsavedWork,
            Filter::StrayActive => v == Verdict::StrayActive,
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

/// Something the user clicked, applied after the frame's UI is drawn.
enum Action {
    Focus(PathBuf),
    Open(PathBuf),
    Terminal(PathBuf),
    AskRecycle(Vec<PathBuf>),
    AddAsProject(PathBuf),
    RemoveProject(PathBuf),
    Ignore(PathBuf),
    CopyPath(PathBuf),
    Undo,
    PickProjectFolder,
    Rescan,
    CheckForUpdates,
    InstallUpdate(Update),
}

#[derive(Clone)]
enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Update),
    Downloading(u64, Option<u64>),
    Installed(String),
    Failed(String),
}

struct Scan {
    rx: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
    dirs: u64,
    current: PathBuf,
}

struct Notice {
    text: String,
    is_error: bool,
    shown_at: Instant,
    /// Repos that were just recycled and can be put back.
    undo: Vec<Repo>,
}

pub struct App {
    cfg: Config,
    home: Option<PathBuf>,
    repos: Vec<Repo>,
    index: HashMap<PathBuf, usize>,
    scan: Option<Scan>,
    last_scan: Option<String>,
    /// When the results on screen were produced, if they came from the cache.
    cached_at: Option<SystemTime>,
    filter: Filter,
    search: String,
    sort_by: SortBy,
    sort_desc: bool,
    /// Ticked rows, for bulk actions.
    checked: HashSet<PathBuf>,
    /// Repo shown in the details panel.
    focused: Option<PathBuf>,
    show_help: bool,
    /// Repos waiting for the user to confirm "move to Recycle Bin".
    confirm_recycle: Option<Vec<PathBuf>>,
    confirm_ack: bool,
    notice: Option<Notice>,
    /// The user chose to scan before adding any project folders.
    skipped_setup: bool,
    update: Arc<Mutex<UpdateState>>,
    update_dismissed: bool,
    /// Where-to-look settings changed since the results on screen were made.
    needs_rescan: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.global_style_mut(|s| {
            s.interaction.tooltip_delay = 0.25;
            // Plain labels would otherwise swallow clicks meant for table rows.
            s.interaction.selectable_labels = false;
        });
        let cfg = Config::load();
        let cache = ScanCache::load(&cfg);
        let mut app = Self {
            home: directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()),
            repos: Vec::new(),
            index: HashMap::new(),
            scan: None,
            last_scan: None,
            cached_at: None,
            filter: Filter::Strays,
            search: String::new(),
            sort_by: SortBy::Size,
            sort_desc: true,
            checked: HashSet::new(),
            focused: None,
            show_help: false,
            confirm_recycle: None,
            confirm_ack: false,
            notice: None,
            skipped_setup: false,
            update: Arc::new(Mutex::new(UpdateState::Idle)),
            update_dismissed: false,
            needs_rescan: false,
            cfg,
        };
        if app.cfg.check_for_updates {
            app.check_for_updates(&cc.egui_ctx);
        }
        if let Some(cache) = cache {
            app.cached_at = Some(cache.scanned_at);
            app.repos = cache.repos;
            app.reindex();
        } else if !app.cfg.project_roots.is_empty() {
            app.start_scan(&cc.egui_ctx);
        }
        app
    }

    // ---- state helpers -------------------------------------------------

    fn show_welcome(&self) -> bool {
        self.cfg.project_roots.is_empty()
            && !self.skipped_setup
            && self.repos.is_empty()
            && self.scan.is_none()
    }

    fn short(&self, p: &Path) -> String {
        display_path(p, self.home.as_deref())
    }

    fn reindex(&mut self) {
        self.index = self
            .repos
            .iter()
            .enumerate()
            .map(|(i, r)| (r.path.clone(), i))
            .collect();
    }

    fn repo(&self, path: &Path) -> Option<&Repo> {
        self.index.get(path).map(|&i| &self.repos[i])
    }

    fn notify(&mut self, text: impl Into<String>, is_error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            is_error,
            shown_at: Instant::now(),
            undo: Vec::new(),
        });
    }

    fn save_cache(&self) {
        let cache = ScanCache {
            scanned_at: self.cached_at.unwrap_or_else(SystemTime::now),
            repos: self.repos.iter().filter(|r| r.inspected).cloned().collect(),
        };
        let _ = cache.save();
    }

    fn save_config(&mut self) {
        if let Err(e) = self.cfg.save() {
            self.notify(format!("Couldn't save settings: {e}"), true);
        }
        // Project folders may have changed; re-classify without rescanning.
        for r in &mut self.repos {
            r.project_root = self.cfg.project_root_for(&r.path).cloned();
        }
    }

    // ---- updates -------------------------------------------------------

    fn update_state(&self) -> UpdateState {
        self.update
            .lock()
            .map(|s| s.clone())
            .unwrap_or(UpdateState::Idle)
    }

    fn set_update_state(state: &Mutex<UpdateState>, new: UpdateState) {
        if let Ok(mut s) = state.lock() {
            *s = new;
        }
    }

    fn check_for_updates(&mut self, ctx: &egui::Context) {
        self.update_dismissed = false;
        let state = self.update.clone();
        let ctx = ctx.clone();
        Self::set_update_state(&state, UpdateState::Checking);
        std::thread::spawn(move || {
            let new = match update::check() {
                Ok(Some(u)) => UpdateState::Available(u),
                Ok(None) => UpdateState::UpToDate,
                Err(e) => UpdateState::Failed(format!("Couldn't check for updates: {e}")),
            };
            Self::set_update_state(&state, new);
            ctx.request_repaint();
        });
    }

    fn install_update(&mut self, u: Update, ctx: &egui::Context) {
        if !cfg!(windows) {
            ctx.open_url(egui::OpenUrl::new_tab(&u.page_url));
            return;
        }
        let state = self.update.clone();
        let ctx = ctx.clone();
        Self::set_update_state(&state, UpdateState::Downloading(0, Some(u.size)));
        std::thread::spawn(move || {
            let progress_state = state.clone();
            let progress_ctx = ctx.clone();
            let result = update::install(&u, move |done, total| {
                Self::set_update_state(&progress_state, UpdateState::Downloading(done, total));
                progress_ctx.request_repaint();
            });
            let new = match result {
                Ok(()) => UpdateState::Installed(u.version.clone()),
                Err(e) => UpdateState::Failed(format!("Update failed: {e}. Nothing was changed.")),
            };
            Self::set_update_state(&state, new);
            ctx.request_repaint();
        });
    }

    /// Banner under the toolbar while an update is available, downloading or installed.
    fn update_banner(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if self.update_dismissed {
            return None;
        }
        let state = self.update_state();
        let mut action = None;
        let accent = Color32::from_rgb(120, 170, 240);
        match state {
            UpdateState::Available(u) => {
                egui::Panel::top("update").show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            accent,
                            format!(
                                "Version {} is available. You have {}.",
                                u.version,
                                update::current_version()
                            ),
                        );
                        if ui
                            .button("Update now")
                            .on_hover_text(
                                "Download the new version and swap it in. Takes a few seconds.",
                            )
                            .clicked()
                        {
                            action = Some(Action::InstallUpdate(u.clone()));
                        }
                        ui.hyperlink_to("What's new", &u.page_url);
                        if ui.small_button("Not now").clicked() {
                            self.update_dismissed = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            }
            UpdateState::Downloading(done, total) => {
                egui::Panel::top("update").show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Downloading update…");
                        let frac = total.map(|t| done as f32 / t.max(1) as f32);
                        ui.add(
                            egui::ProgressBar::new(frac.unwrap_or(0.0))
                                .desired_width(200.0)
                                .text(human_size(done)),
                        );
                    });
                    ui.add_space(4.0);
                });
            }
            UpdateState::Installed(v) => {
                egui::Panel::top("update").show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.colored_label(accent, format!("Updated to version {v}."));
                        if ui.button("Restart now").clicked() {
                            update::relaunch();
                        }
                        ui.label(
                            RichText::new("or it'll be used next time you open the app.").weak(),
                        );
                    });
                    ui.add_space(4.0);
                });
            }
            UpdateState::Failed(msg) if !msg.starts_with("Couldn't check") => {
                egui::Panel::top("update").show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.colored_label(ERROR, msg);
                        ui.hyperlink_to(
                            "Download it manually",
                            format!("https://github.com/{}/releases/latest", update::REPO),
                        );
                        if ui.small_button("x").clicked() {
                            self.update_dismissed = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            }
            _ => {}
        }
        action
    }

    // ---- scanning ------------------------------------------------------

    fn start_scan(&mut self, ctx: &egui::Context) {
        if let Some(old) = &self.scan {
            old.cancel.store(true, Ordering::Relaxed);
        }
        self.repos.clear();
        self.index.clear();
        self.checked.clear();
        self.cached_at = None;
        self.needs_rescan = false;
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
            current: PathBuf::new(),
        });
    }

    fn poll_scan(&mut self) {
        let Some(scan) = &mut self.scan else { return };
        let mut finished = None;
        while let Ok(ev) = scan.rx.try_recv() {
            match ev {
                ScanEvent::Progress { dirs, current } => {
                    scan.dirs = dirs;
                    scan.current = current;
                }
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
            self.scan = None;
            if cancelled {
                self.last_scan = Some("Scan stopped. Results may be incomplete.".to_string());
            } else {
                self.last_scan = Some(format!(
                    "Checked {} folders in {:.0}s",
                    group_digits(dirs),
                    elapsed.as_secs_f32().max(1.0)
                ));
                self.cached_at = Some(SystemTime::now());
                self.save_cache();
            }
        }
    }

    // ---- actions -------------------------------------------------------

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Focus(p) => {
                self.focused = Some(p);
            }
            Action::Open(p) => actions::open_folder(&p),
            Action::Terminal(p) => actions::open_terminal(&p),
            Action::AskRecycle(paths) => {
                if !paths.is_empty() {
                    self.confirm_recycle = Some(paths);
                    self.confirm_ack = false;
                }
            }
            Action::AddAsProject(p) => {
                self.notify(
                    format!(
                        "{} is now a project folder. Repos in it won't be flagged.",
                        self.short(&p)
                    ),
                    false,
                );
                self.cfg.project_roots.push(p);
                self.save_config();
            }
            Action::RemoveProject(p) => {
                self.cfg.project_roots.retain(|r| r != &p);
                self.save_config();
            }
            Action::Ignore(p) => {
                self.repos.retain(|r| r.path != p);
                self.reindex();
                self.checked.remove(&p);
                if self.focused.as_ref() == Some(&p) {
                    self.focused = None;
                }
                self.notify(
                    format!(
                        "Hidden {}. Unhide it under \"Hidden repos\" on the left.",
                        self.short(&p)
                    ),
                    false,
                );
                self.cfg.ignored.push(p);
                self.save_config();
                self.save_cache();
            }
            Action::CopyPath(p) => {
                ctx.copy_text(p.display().to_string());
                self.notify("Path copied.", false);
            }
            Action::Undo => self.undo(),
            Action::PickProjectFolder => {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title("Choose a folder that holds your projects")
                    .pick_folder()
                {
                    if !self.cfg.project_roots.iter().any(|f| f == &dir) {
                        self.cfg.project_roots.push(dir);
                        self.save_config();
                    }
                    if self.repos.is_empty() && self.scan.is_none() {
                        self.start_scan(ctx);
                    }
                }
            }
            Action::Rescan => self.start_scan(ctx),
            Action::CheckForUpdates => self.check_for_updates(ctx),
            Action::InstallUpdate(u) => self.install_update(u, ctx),
        }
    }

    fn recycle(&mut self, paths: &[PathBuf]) {
        let mut moved = Vec::new();
        let mut failed = Vec::new();
        for p in paths {
            let Some(repo) = self.repo(p).cloned() else {
                continue;
            };
            match actions::recycle(p) {
                Ok(()) => moved.push(repo),
                Err(e) => failed.push(format!("{} ({e})", repo.name())),
            }
        }
        let gone: HashSet<&PathBuf> = moved.iter().map(|r| &r.path).collect();
        self.repos.retain(|r| !gone.contains(&r.path));
        self.checked.retain(|p| !gone.contains(p));
        if self.focused.as_ref().is_some_and(|f| gone.contains(f)) {
            self.focused = None;
        }
        self.reindex();
        self.save_cache();

        let freed: u64 = moved.iter().filter_map(|r| r.size_bytes).sum();
        let mut text = match moved.len() {
            0 => String::new(),
            1 => format!(
                "Moved {} to the Recycle Bin ({}).",
                moved[0].name(),
                human_size(freed)
            ),
            n => format!(
                "Moved {n} repos to the Recycle Bin ({}).",
                human_size(freed)
            ),
        };
        if !failed.is_empty() {
            text.push_str(&format!(" Couldn't move: {}.", failed.join(", ")));
        }
        self.notice = Some(Notice {
            text: text.trim().to_string(),
            is_error: !failed.is_empty(),
            shown_at: Instant::now(),
            undo: moved,
        });
    }

    fn undo(&mut self) {
        let Some(notice) = self.notice.take() else {
            return;
        };
        let mut failed = Vec::new();
        let mut restored = 0;
        for repo in notice.undo {
            match actions::restore(&repo.path) {
                Ok(()) => {
                    restored += 1;
                    self.repos.push(repo);
                }
                Err(e) => failed.push(format!("{} ({e})", repo.name())),
            }
        }
        self.reindex();
        self.save_cache();
        if failed.is_empty() {
            self.notify(
                format!(
                    "Put back {restored} repo{}.",
                    if restored == 1 { "" } else { "s" }
                ),
                false,
            );
        } else {
            self.notify(
                format!(
                    "Couldn't put back {}. Restore it from the Recycle Bin instead.",
                    failed.join(", ")
                ),
                true,
            );
        }
    }

    // ---- panels --------------------------------------------------------

    fn settings_panel(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut changed = false;
        let mut action = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            heading_with_help(
                ui,
                "Your project folders",
                "Folders where your real work lives, like C:\\Users\\you\\code.\n\
                 Repos inside these are left alone. Repos anywhere else are treated as possible leftovers.",
            );
            changed |= folder_list(
                ui,
                "projects",
                &mut self.cfg.project_roots,
                "Add project folder…",
                self.home.as_deref(),
            );
            ui.add_space(14.0);

            heading_with_help(
                ui,
                "Where to look",
                "The folders scanned for git repos. Your home folder covers Downloads, Desktop, \
                 Documents and the Temp folder where tools often clone things.",
            );
            let mut where_changed = folder_list(
                ui,
                "scan",
                &mut self.cfg.scan_roots,
                "Add scan location…",
                self.home.as_deref(),
            );
            where_changed |= ui
                .checkbox(&mut self.cfg.appdata_temp_only, "Only check Temp inside AppData")
                .on_hover_text(
                    "AppData is huge and mostly app settings. Leaving this on makes scans much \
                     faster while still checking AppData\\Local\\Temp, where throwaway clones end up.",
                )
                .changed();
            if where_changed {
                changed = true;
                self.needs_rescan |= !self.repos.is_empty();
            }
            ui.add_space(14.0);

            heading_with_help(
                ui,
                "Stale after",
                "A leftover repo you haven't touched for this long is a cleanup candidate. \
                 Anything newer is shown as \"Stray, recent\" in case you still need it.",
            );
            changed |= ui
                .add(egui::Slider::new(&mut self.cfg.stale_after_days, 1..=365).suffix(" days"))
                .changed();
            ui.add_space(14.0);

            if !self.cfg.ignored.is_empty() {
                ui.collapsing(format!("Hidden repos ({})", self.cfg.ignored.len()), |ui| {
                    ui.label(
                        RichText::new("Remove one to show it again after the next scan.")
                            .small()
                            .weak(),
                    );
                    changed |= folder_list(
                        ui,
                        "ignored",
                        &mut self.cfg.ignored,
                        "",
                        self.home.as_deref(),
                    );
                });
                ui.add_space(8.0);
            }
            if self.needs_rescan {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Search settings changed.").small().color(WARN));
                    if ui.small_button("Rescan now").clicked() {
                        action = Some(Action::Rescan);
                    }
                });
            }
            ui.separator();
            if let Some(path) = Config::data_dir()
                && ui
                    .small_button("Open settings and action log")
                    .on_hover_text(
                        "Opens the folder with config.toml and actions.log, a record of everything this app moved.",
                    )
                    .clicked()
            {
                let _ = std::fs::create_dir_all(&path);
                actions::open_folder(&path);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Version {}", update::current_version())).weak());
                let checking = matches!(self.update_state(), UpdateState::Checking);
                if ui
                    .add_enabled(!checking, egui::Button::new("Check for updates").small())
                    .clicked()
                {
                    action = Some(Action::CheckForUpdates);
                }
            });
            match self.update_state() {
                UpdateState::Checking => {
                    ui.label(RichText::new("Checking…").small().weak());
                }
                UpdateState::UpToDate => {
                    ui.label(RichText::new("You're on the latest version.").small().weak());
                }
                UpdateState::Failed(msg) if msg.starts_with("Couldn't check") => {
                    ui.label(RichText::new(msg).small().color(ERROR));
                }
                _ => {}
            }
            changed |= ui
                .checkbox(&mut self.cfg.check_for_updates, "Check for updates on startup")
                .changed();
            if self.cfg.project_roots.is_empty() && !self.show_welcome() && action.is_none() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Add your project folders so your own work isn't flagged.")
                        .color(WARN),
                );
            }
            if ui.input(|i| i.key_pressed(egui::Key::F5)) {
                action = Some(Action::Rescan);
            }
        });
        if changed {
            self.save_config();
        }
        action
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            if let Some(scan) = &self.scan {
                if ui
                    .button("Stop")
                    .on_hover_text("Stop scanning. You keep what was found so far.")
                    .clicked()
                {
                    scan.cancel.store(true, Ordering::Relaxed);
                }
                ui.spinner();
                let pending = self.repos.iter().filter(|r| !r.inspected).count();
                let mut text = format!(
                    "{} folders checked, {} repos found",
                    group_digits(scan.dirs),
                    self.repos.len()
                );
                if pending > 0 {
                    text.push_str(&format!(", reading {pending}"));
                }
                ui.label(text);
                if !scan.current.as_os_str().is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(self.short(&scan.current)).small().weak())
                            .truncate(),
                    );
                }
            } else {
                if ui
                    .button("Rescan")
                    .on_hover_text("Look for repos again and refresh every row (F5).")
                    .clicked()
                {
                    action = Some(Action::Rescan);
                }
                if let Some(t) = self.cached_at
                    && self.last_scan.is_none()
                {
                    ui.label(
                        RichText::new(format!(
                            "Showing results from {}",
                            relative_time(t, SystemTime::now())
                        ))
                        .weak(),
                    )
                    .on_hover_text("Saved from your last scan. Rescan to refresh.");
                } else if let Some(s) = &self.last_scan {
                    ui.label(RichText::new(s).weak());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.show_help {
                    "Hide guide"
                } else {
                    "How this works"
                };
                if ui
                    .selectable_label(self.show_help, label)
                    .on_hover_text(
                        "What the app looks for, what each status means, and what's safe.",
                    )
                    .clicked()
                {
                    self.show_help = !self.show_help;
                    if self.show_help {
                        self.focused = None;
                    }
                }
            });
        });
        if self.show_welcome() {
            return action;
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let now = SystemTime::now();
            for f in Filter::ALL {
                let count = self
                    .repos
                    .iter()
                    .filter(|r| f.keeps(r.verdict(&self.cfg, now), r.is_stray()))
                    .count();
                let mut text = RichText::new(format!("{}  {count}", f.label()));
                if let Some(c) = f.color() {
                    text = text.color(c);
                }
                if ui
                    .selectable_label(self.filter == f, text)
                    .on_hover_text(f.tooltip())
                    .clicked()
                {
                    self.filter = f;
                }
            }
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search name or path")
                    .desired_width(200.0),
            );
        });
        action
    }

    fn summary(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let now = SystemTime::now();
        let mut action = None;
        let (mut stray_n, mut stray_bytes) = (0, 0u64);
        let mut safe: Vec<PathBuf> = Vec::new();
        let mut safe_bytes = 0u64;
        for r in &self.repos {
            if r.is_stray() {
                stray_n += 1;
                stray_bytes += r.size_bytes.unwrap_or(0);
            }
            if r.verdict(&self.cfg, now) == Verdict::SafeToRemove {
                safe.push(r.path.clone());
                safe_bytes += r.size_bytes.unwrap_or(0);
            }
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{stray_n} leftover repos using {}",
                    human_size(stray_bytes)
                ))
                .strong(),
            );
            if !safe.is_empty() {
                ui.label("·");
                ui.label(
                    RichText::new(format!(
                        "{} safe to remove ({})",
                        safe.len(),
                        human_size(safe_bytes)
                    ))
                    .color(verdict_color(Verdict::SafeToRemove)),
                );
                if ui
                    .small_button("Select all safe")
                    .on_hover_text(
                        "Tick every repo marked \"Safe to remove\" so you can recycle them together.",
                    )
                    .clicked()
                {
                    self.checked.extend(safe.iter().cloned());
                    self.filter = Filter::SafeToRemove;
                }
            }
        });

        if !self.checked.is_empty() {
            let bytes: u64 = self
                .checked
                .iter()
                .filter_map(|p| self.repo(p).and_then(|r| r.size_bytes))
                .sum();
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} selected ({})",
                            self.checked.len(),
                            human_size(bytes)
                        ))
                        .strong(),
                    );
                    if ui
                        .button("Recycle selected…")
                        .on_hover_text(
                            "Move the ticked repos to the Recycle Bin. You'll confirm first.",
                        )
                        .clicked()
                    {
                        let mut paths: Vec<PathBuf> = self.checked.iter().cloned().collect();
                        paths.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
                        action = Some(Action::AskRecycle(paths));
                    }
                    if ui.button("Clear selection").clicked() {
                        self.checked.clear();
                    }
                });
            });
        }
        action
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

    fn table(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let rows = self.visible_rows();
        if rows.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = if self.scan.is_some() {
                    "Scanning… repos will appear here as they're found."
                } else if self.repos.is_empty() {
                    "No repos found yet. Press Rescan to look."
                } else if !self.search.is_empty() {
                    "Nothing matches your search."
                } else {
                    "Nothing in this view. Try another tab above."
                };
                ui.label(RichText::new(msg).weak());
            });
            return None;
        }
        let now = SystemTime::now();
        let mut action = None;
        let mut header_clicks: Vec<SortBy> = Vec::new();
        let mut toggles: Vec<(PathBuf, bool)> = Vec::new();
        let all_checked = rows
            .iter()
            .all(|&i| self.checked.contains(&self.repos[i].path));
        let mut toggle_all = None;

        // With a side panel open there isn't room for every column, so the
        // "What's unsaved" text moves into the status tooltip.
        let compact = ui.available_width() < 960.0;
        let mut builder = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(24.0))
            .column(Column::exact(140.0))
            .column(Column::remainder().at_least(160.0).clip(true))
            .column(Column::initial(72.0))
            .column(Column::initial(98.0));
        if !compact {
            builder = builder.column(Column::initial(210.0).clip(true));
        }
        builder
            .column(Column::exact(150.0))
            .header(26.0, |mut h| {
                h.col(|ui| {
                    let mut all = all_checked;
                    if ui
                        .checkbox(&mut all, "")
                        .on_hover_text("Select or clear every row in this view")
                        .changed()
                    {
                        toggle_all = Some(all);
                    }
                });
                for (label, by, tip) in [
                    ("Status", Some(SortBy::Status), "What the app thinks you should do. Hover a status for details."),
                    ("Repo", Some(SortBy::Name), "Folder name and where it is. Click a row for details."),
                    ("Size", Some(SortBy::Size), "Everything in the folder, including dependencies and build output."),
                    ("Last used", Some(SortBy::Activity), "Newest of the last commit, checkout, pull or fetch."),
                    ("What's unsaved", None, "Work that exists only on this PC: uncommitted changes, unpushed commits, stashes."),
                    ("", None, ""),
                ] {
                    if compact && label == "What's unsaved" {
                        continue;
                    }
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
                            if ui.add(btn).on_hover_text(format!("{tip}\nClick to sort.")).clicked() {
                                header_clicks.push(by);
                            }
                        }
                        None => {
                            let r = ui.strong(label);
                            if !tip.is_empty() {
                                r.on_hover_text(tip);
                            }
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(40.0, rows.len(), |mut row| {
                    let r = &self.repos[rows[row.index()]];
                    let v = r.verdict(&self.cfg, now);
                    let is_focused = self.focused.as_ref() == Some(&r.path);
                    row.set_selected(is_focused);
                    row.col(|ui| {
                        let mut c = self.checked.contains(&r.path);
                        if ui.checkbox(&mut c, "").changed() {
                            toggles.push((r.path.clone(), c));
                        }
                    });
                    row.col(|ui| {
                        status_chip(ui, v);
                    });
                    let summary = r.git_summary();
                    row.col(|ui| {
                        let cell = ui.vertical(|ui| {
                            ui.add_space(3.0);
                            ui.label(RichText::new(r.name()).strong());
                            ui.label(RichText::new(self.short(&r.path)).small().weak());
                        });
                        if compact && !summary.is_empty() {
                            cell.response.on_hover_text(&summary);
                        }
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
                    if !compact {
                        row.col(|ui| {
                            let color = if r.inspected && r.has_local_work() {
                                verdict_color(Verdict::UnsavedWork)
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            ui.label(RichText::new(&summary).color(color))
                                .on_hover_text(&summary);
                        });
                    }
                    row.col(|ui| {
                        if ui
                            .small_button("Details")
                            .on_hover_text("See what's unsaved and what to do about it")
                            .clicked()
                        {
                            action = Some(Action::Focus(r.path.clone()));
                        }
                        if ui
                            .add_enabled(r.inspected, egui::Button::new("Recycle…").small())
                            .on_hover_text("Move to the Recycle Bin. You'll confirm first, and can undo.")
                            .clicked()
                        {
                            action = Some(Action::AskRecycle(vec![r.path.clone()]));
                        }
                    });
                    let resp = row.response();
                    if resp.clicked() {
                        action = Some(Action::Focus(r.path.clone()));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Open in File Explorer").clicked() {
                            action = Some(Action::Open(r.path.clone()));
                        }
                        if ui.button("Open terminal here").clicked() {
                            action = Some(Action::Terminal(r.path.clone()));
                        }
                        if ui.button("Copy path").clicked() {
                            action = Some(Action::CopyPath(r.path.clone()));
                        }
                        ui.separator();
                        if r.is_stray() && ui.button("Mark as a project folder").clicked() {
                            action = Some(Action::AddAsProject(r.path.clone()));
                        }
                        if ui.button("Hide from this list").clicked() {
                            action = Some(Action::Ignore(r.path.clone()));
                        }
                    });
                });
            });

        for by in header_clicks {
            self.toggle_sort(by);
        }
        for (p, on) in toggles {
            if on {
                self.checked.insert(p);
            } else {
                self.checked.remove(&p);
            }
        }
        if let Some(on) = toggle_all {
            for &i in &rows {
                let p = self.repos[i].path.clone();
                if on {
                    self.checked.insert(p);
                } else {
                    self.checked.remove(&p);
                }
            }
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

    fn details_panel(&mut self, ui: &mut egui::Ui, path: &Path) -> Option<Action> {
        let Some(r) = self.repo(path).cloned() else {
            self.focused = None;
            return None;
        };
        let now = SystemTime::now();
        let v = r.verdict(&self.cfg, now);
        let mut action = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(r.name());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Close").clicked() {
                        self.focused = None;
                    }
                });
            });
            status_chip(ui, v);
            ui.label(RichText::new(v.explanation()).weak());
            ui.add_space(10.0);

            egui::Grid::new("facts")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Location").weak());
                    ui.add(egui::Label::new(self.short(&r.path)).wrap());
                    ui.end_row();
                    if let Some(g) = &r.git {
                        ui.label(RichText::new("Remote").weak());
                        ui.add(
                            egui::Label::new(g.remote_url.clone().unwrap_or_else(|| "None".into()))
                                .wrap(),
                        );
                        ui.end_row();
                        ui.label(RichText::new("Branch").weak());
                        ui.label(g.branch.clone().unwrap_or_else(|| "(detached)".into()));
                        ui.end_row();
                    }
                    ui.label(RichText::new("Size").weak());
                    ui.label(r.size_bytes.map(human_size).unwrap_or_default());
                    ui.end_row();
                    ui.label(RichText::new("Last used").weak());
                    ui.label(
                        r.last_activity
                            .map(|t| relative_time(t, now))
                            .unwrap_or_else(|| "Unknown".into()),
                    );
                    ui.end_row();
                    if let Some(root) = &r.project_root {
                        ui.label(RichText::new("Project").weak());
                        ui.label(self.short(root));
                        ui.end_row();
                    }
                });

            if let Some(g) = &r.git {
                let reasons = g.local_work_reasons();
                if !reasons.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("Only on this PC")
                            .strong()
                            .color(verdict_color(Verdict::UnsavedWork)),
                    );
                    for reason in reasons {
                        ui.label(format!("•  {reason}"));
                    }
                }
            }

            let steps = r.next_steps(&self.cfg, now);
            if !steps.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new("What to do").strong());
                for (i, s) in steps.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{}.", i + 1));
                        ui.label(s);
                    });
                }
            }

            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open folder").clicked() {
                    action = Some(Action::Open(r.path.clone()));
                }
                if ui
                    .button("Open terminal here")
                    .on_hover_text("For running git push, git status and so on")
                    .clicked()
                {
                    action = Some(Action::Terminal(r.path.clone()));
                }
                if ui.button("Copy path").clicked() {
                    action = Some(Action::CopyPath(r.path.clone()));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if r.is_stray() {
                    if ui
                        .button("Mark as project")
                        .on_hover_text("Treat this folder as yours. It won't be flagged again.")
                        .clicked()
                    {
                        action = Some(Action::AddAsProject(r.path.clone()));
                    }
                } else if r.project_root.as_ref() == Some(&r.path)
                    && ui
                        .button("Unmark as project")
                        .on_hover_text("Remove this folder from your project folders.")
                        .clicked()
                {
                    action = Some(Action::RemoveProject(r.path.clone()));
                }
                if ui
                    .button("Hide")
                    .on_hover_text("Stop showing this repo. You can unhide it on the left.")
                    .clicked()
                {
                    action = Some(Action::Ignore(r.path.clone()));
                }
                if ui
                    .add_enabled(r.inspected, egui::Button::new("Recycle…"))
                    .on_hover_text("Move to the Recycle Bin. You'll confirm first, and can undo.")
                    .clicked()
                {
                    action = Some(Action::AskRecycle(vec![r.path.clone()]));
                }
            });
        });
        action
    }

    fn help_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("How this works");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Close").clicked() {
                        self.show_help = false;
                    }
                });
            });
            ui.label(
                "AI coding tools often clone repos or start scratch projects outside the \
                 project you're working on. This app finds those leftover repos so you can \
                 clean them up safely.",
            );
            ui.add_space(10.0);
            ui.label(RichText::new("1. Tell it where your projects are").strong());
            ui.label(
                "Add your project folders on the left. Repos inside them are yours and \
                 won't be flagged.",
            );
            ui.add_space(6.0);
            ui.label(RichText::new("2. It scans for repos").strong());
            ui.label(
                "It looks for any folder with a .git inside, skipping dependency caches like \
                 node_modules. Results are saved, so it opens instantly next time.",
            );
            ui.add_space(6.0);
            ui.label(RichText::new("3. It checks what's unsaved").strong());
            ui.label(
                "Using git (read-only), it checks each repo for uncommitted changes, commits \
                 that aren't on GitHub, stashes, and whether it has a remote at all.",
            );
            ui.add_space(10.0);
            ui.label(RichText::new("What the statuses mean").strong());
            for v in [
                Verdict::SafeToRemove,
                Verdict::UnsavedWork,
                Verdict::StrayActive,
                Verdict::InProject,
            ] {
                ui.add_space(4.0);
                status_chip(ui, v);
                ui.label(RichText::new(v.explanation()).weak());
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Is it safe?").strong());
            ui.label(
                "•  Nothing is permanently deleted. Recycle moves folders to the Recycle Bin.",
            );
            ui.label("•  You confirm every removal, and can undo it right after.");
            ui.label("•  Repos with unsaved work need an extra tick before they can be recycled.");
            ui.label("•  Every move is written to actions.log (button on the left).");
            ui.add_space(10.0);
            ui.label(RichText::new("Tips").strong());
            ui.label("•  Click a row to see what to do about it.");
            ui.label("•  Right-click a row for more options.");
            ui.label("•  \"Select all safe\" plus \"Recycle selected\" cleans up in one go.");
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            ui.set_max_width(560.0);
            ui.heading(RichText::new("Find what your AI coding tools left behind").size(24.0));
            ui.add_space(8.0);
            ui.label(
                "Agents like Claude Code and Codex often clone repos or start scratch \
                 projects in Downloads, Desktop or Temp. This app finds them, shows what's \
                 safe to remove, and moves them to the Recycle Bin for you.",
            );
            ui.add_space(24.0);
        });
        ui.vertical_centered(|ui| {
            ui.set_max_width(520.0);
            egui::Frame::group(ui.style()).inner_margin(16.0).show(ui, |ui| {
                ui.set_width(480.0);
                for (n, title, body) in [
                    ("1", "Add your project folders", "So the app knows which repos are your real work."),
                    ("2", "Scan", "It finds every git repo outside those folders and checks what's unsaved."),
                    ("3", "Clean up", "Recycle the ones marked safe. You can always undo."),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(n).size(20.0).strong());
                        ui.vertical(|ui| {
                            ui.label(RichText::new(title).strong());
                            ui.label(RichText::new(body).weak());
                        });
                    });
                    ui.add_space(6.0);
                }
            });
            ui.add_space(18.0);
            if ui
                .add(egui::Button::new(RichText::new("Add a project folder…").size(16.0)).min_size(egui::vec2(220.0, 34.0)))
                .clicked()
            {
                action = Some(Action::PickProjectFolder);
            }
            ui.add_space(6.0);
            if ui
                .link("Skip and scan everything")
                .on_hover_text("Every repo will show as a leftover until you add project folders.")
                .clicked()
            {
                self.skipped_setup = true;
                action = Some(Action::Rescan);
            }
        });
        action
    }

    fn recycle_dialog(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.confirm_recycle.clone() else {
            return;
        };
        let repos: Vec<Repo> = paths.iter().filter_map(|p| self.repo(p).cloned()).collect();
        if repos.is_empty() {
            self.confirm_recycle = None;
            return;
        }
        let risky: Vec<&Repo> = repos.iter().filter(|r| r.has_local_work()).collect();
        let in_project: Vec<&Repo> = repos.iter().filter(|r| !r.is_stray()).collect();
        let total: u64 = repos.iter().filter_map(|r| r.size_bytes).sum();
        let mut close = false;
        let mut go = false;
        let title = if repos.len() == 1 {
            format!("Move \"{}\" to the Recycle Bin?", repos[0].name())
        } else {
            format!("Move {} repos to the Recycle Bin?", repos.len())
        };
        let modal = egui::Modal::new(egui::Id::unique("confirm_recycle")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading(title);
            ui.label(format!(
                "Frees about {} once the Recycle Bin is emptied.",
                human_size(total)
            ));
            ui.add_space(6.0);
            if repos.len() == 1 {
                ui.label(RichText::new(self.short(&repos[0].path)).weak());
            } else {
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for r in &repos {
                            ui.horizontal(|ui| {
                                ui.label(format!("•  {}", r.name()));
                                if r.has_local_work() {
                                    ui.label(
                                        RichText::new("has unsaved work")
                                            .small()
                                            .color(verdict_color(Verdict::UnsavedWork)),
                                    );
                                }
                            });
                        }
                    });
            }
            if !in_project.is_empty() {
                ui.add_space(6.0);
                ui.colored_label(
                    WARN,
                    if in_project.len() == 1 && repos.len() == 1 {
                        "This repo is inside one of your project folders.".to_string()
                    } else {
                        format!(
                            "{} of these are inside your project folders.",
                            in_project.len()
                        )
                    },
                );
            }
            ui.add_space(8.0);
            if risky.is_empty() {
                ui.label("Everything is committed and pushed, so you can clone it again any time.");
            } else {
                ui.colored_label(
                    verdict_color(Verdict::UnsavedWork),
                    "Some work here isn't saved anywhere else:",
                );
                for r in risky.iter().take(6) {
                    let reasons = r
                        .git
                        .as_ref()
                        .map(|g| g.local_work_reasons().join(", "))
                        .unwrap_or_else(|| "git couldn't read it".into());
                    ui.label(format!("•  {}: {reasons}", r.name()));
                }
                if risky.len() > 6 {
                    ui.label(format!("…and {} more", risky.len() - 6));
                }
                ui.checkbox(
                    &mut self.confirm_ack,
                    "I understand this work only exists on this PC",
                );
            }
            ui.label(
                RichText::new(
                    "You can undo this right after, or restore from the Recycle Bin later.",
                )
                .small()
                .weak(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if ui
                    .add_enabled(
                        risky.is_empty() || self.confirm_ack,
                        egui::Button::new("Move to Recycle Bin"),
                    )
                    .clicked()
                {
                    go = true;
                }
            });
        });
        if go {
            self.recycle(&paths);
            close = true;
        }
        if close || modal.should_close() {
            self.confirm_recycle = None;
        }
    }

    fn notice_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let Some(n) = &self.notice else { return None };
        if n.shown_at.elapsed() > Duration::from_secs(NOTICE_SECS) {
            self.notice = None;
            return None;
        }
        let (text, is_error, can_undo) = (n.text.clone(), n.is_error, !n.undo.is_empty());
        egui::Panel::bottom("notice").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let color = if is_error {
                    ERROR
                } else {
                    ui.visuals().text_color()
                };
                ui.colored_label(color, text);
                if can_undo
                    && ui
                        .button("Undo")
                        .on_hover_text("Put it back where it was")
                        .clicked()
                {
                    action = Some(Action::Undo);
                }
                if ui.small_button("x").on_hover_text("Dismiss").clicked() {
                    self.notice = None;
                }
            });
            ui.add_space(4.0);
        });
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        action
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_scan();
        let ctx = ui.ctx().clone();
        let mut actions: Vec<Action> = Vec::new();

        egui::Panel::left("settings")
            .resizable(true)
            .default_size(270.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                actions.extend(self.settings_panel(ui));
            });
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            actions.extend(self.top_bar(ui));
            ui.add_space(6.0);
        });
        actions.extend(self.update_banner(ui));
        actions.extend(self.notice_bar(ui));

        if let Some(path) = self.focused.clone() {
            egui::Panel::right("details")
                .resizable(true)
                .default_size(340.0)
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    actions.extend(self.details_panel(ui, &path));
                });
        } else if self.show_help {
            egui::Panel::right("help")
                .resizable(true)
                .default_size(340.0)
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    self.help_panel(ui);
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if self.show_welcome() {
                actions.extend(self.welcome(ui));
            } else {
                actions.extend(self.summary(ui));
                ui.add_space(6.0);
                actions.extend(self.table(ui));
            }
        });

        for a in actions {
            self.apply(a, &ctx);
        }
        self.recycle_dialog(&ctx);

        // Keep "x minutes ago" labels fresh without burning CPU.
        ctx.request_repaint_after(Duration::from_secs(30));
    }
}

fn heading_with_help(ui: &mut egui::Ui, title: &str, help: &str) {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.label(RichText::new("(?)").weak()).on_hover_text(help);
    });
}

/// Colored dot plus label, with the verdict's explanation on hover.
fn status_chip(ui: &mut egui::Ui, v: Verdict) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 4.5, verdict_color(v));
        ui.label(RichText::new(v.label()).color(verdict_color(v)));
    })
    .response
    .on_hover_text(v.explanation());
}

/// Editable list of folders with remove buttons. Returns true if it changed.
fn folder_list(
    ui: &mut egui::Ui,
    id: &str,
    folders: &mut Vec<PathBuf>,
    add_label: &str,
    home: Option<&Path>,
) -> bool {
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
                let shown = match display_path(f, home) {
                    s if s == "~" => "Your home folder".to_string(),
                    s => s,
                };
                ui.add(egui::Label::new(shown).truncate())
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
