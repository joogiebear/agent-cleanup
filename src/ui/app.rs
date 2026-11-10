//! The root view: state, background work and what each click does.

use super::theme::*;
use super::widgets::*;
use agent_cleanup::actions;
use agent_cleanup::activity::{self, Entry};
use agent_cleanup::agentdata::{self, Orphan};
use agent_cleanup::cache::ScanCache;
use agent_cleanup::config::Config;
use agent_cleanup::config::path_within;
use agent_cleanup::deps::{self, Eco};
use agent_cleanup::downloads::{self, Download};
use agent_cleanup::jobs::{self, Op, Outcome};
use agent_cleanup::junk;
use agent_cleanup::library::{Filter, Library, SortBy};
use agent_cleanup::model::{Repo, display_path, human_size};
use agent_cleanup::scan::{self, ScanEvent};
use agent_cleanup::schedule;
use agent_cleanup::trail::{self, Item};
use agent_cleanup::update::{self, Update};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// How long a notice stays at the bottom of the window.
const NOTICE_SECS: u64 = 12;
/// How often background results are picked up.
const POLL: Duration = Duration::from_millis(60);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Leftovers,
    Trail,
    Downloads,
    AgentData,
    Duplicates,
    Junk,
    Updates,
    Projects,
    Activity,
    Settings,
    Help,
}

#[derive(Clone, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Update),
    Downloading(u64, Option<u64>),
    Installed(String),
    Failed(String),
}

pub struct Scan {
    rx: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
    pub dirs: u64,
    pub current: PathBuf,
}

pub struct Notice {
    pub text: String,
    pub is_error: bool,
    pub shown_at: Instant,
    /// Repos that were just recycled and can be put back.
    pub undo: Vec<Repo>,
    /// Agent trail items that were just recycled.
    pub undo_trail: Vec<Item>,
    /// Build folders that were just recycled.
    pub undo_junk: Vec<junk::Item>,
    /// Downloads that were just recycled (`None`) or moved (`Some(to)`).
    pub undo_downloads: Vec<(Download, Option<PathBuf>)>,
    /// Agent data that was just recycled.
    pub undo_agentdata: Vec<Orphan>,
}

impl Notice {
    fn new(text: impl Into<String>, is_error: bool) -> Self {
        Self {
            text: text.into().trim().to_string(),
            is_error,
            shown_at: Instant::now(),
            undo: Vec::new(),
            undo_trail: Vec::new(),
            undo_junk: Vec::new(),
            undo_downloads: Vec::new(),
            undo_agentdata: Vec::new(),
        }
    }
}

/// What a background job is doing, so its results go to the right list.
enum JobKind {
    Repos,
    Trail,
    Downloads,
    AgentData,
    /// Putting back what this notice offered to undo.
    Undo(Box<Notice>),
}

/// Recycling or restoring on a background thread, so the window never
/// freezes while big folders move.
pub struct Job {
    rx: Receiver<Vec<Outcome>>,
    /// What the toast says while it runs.
    pub label: String,
    kind: JobKind,
    /// What each step was for, in step order.
    keys: Vec<PathBuf>,
}

/// Which page's things a confirm dialog is about.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConfirmFor {
    Repos,
    Trail,
    Junk,
    Downloads,
    AgentData,
    Duplicates,
}

pub struct Confirm {
    pub paths: Vec<PathBuf>,
    pub ack: bool,
    pub what: ConfirmFor,
}

type JunkLoad = (Receiver<Vec<junk::Item>>, Arc<AtomicBool>, Arc<AtomicUsize>);

/// Build folders, found and recycled on background threads (a big
/// `node_modules` can take a while to move).
#[derive(Default)]
pub struct Junk {
    pub items: Vec<junk::Item>,
    pub loading: Option<JunkLoad>,
    pub loaded: bool,
    pub repos: usize,
    /// A recycle or restore in progress, and what it says while it runs.
    pub busy: Option<(Receiver<JunkJob>, String)>,
}

type DepsLoad = (
    Receiver<Vec<deps::Report>>,
    Arc<AtomicBool>,
    Arc<AtomicUsize>,
);

/// Outdated dependencies, checked in the background.
#[derive(Default)]
pub struct Deps {
    pub reports: Vec<deps::Report>,
    pub loading: Option<DepsLoad>,
    pub loaded: bool,
    /// Project repos being checked.
    pub repos: usize,
}

pub struct JunkJob {
    pub restore: bool,
    pub done: Vec<junk::Item>,
    pub failed: Vec<String>,
}

/// Agent session data for deleted projects, found in the background.
#[derive(Default)]
pub struct AgentData {
    pub items: Vec<Orphan>,
    pub loading: Option<(Receiver<Vec<Orphan>>, Arc<AtomicBool>)>,
    pub loaded: bool,
}

/// The Downloads folder, listed and matched in the background.
#[derive(Default)]
pub struct Downloads {
    pub items: Vec<Download>,
    pub loading: Option<(Receiver<Vec<Download>>, Arc<AtomicBool>)>,
    pub loaded: bool,
    pub dir: Option<PathBuf>,
}

/// The agent trail page's data, read in the background.
#[derive(Default)]
pub struct Trail {
    pub items: Vec<Item>,
    pub loading: Option<(Receiver<Vec<Item>>, Arc<AtomicBool>)>,
    pub loaded: bool,
    /// How many session logs there were to read.
    pub logs: usize,
}

pub struct AgentCleanup {
    pub cfg: Config,
    pub home: Option<PathBuf>,
    pub lib: Library,
    pub scan: Option<Scan>,
    pub last_scan: Option<String>,
    pub cached_at: Option<SystemTime>,
    pub page: Page,
    pub filter: Filter,
    pub sort: SortBy,
    pub search: Entity<InputState>,
    pub search_text: String,
    pub focused: Option<PathBuf>,
    pub confirm: Option<Confirm>,
    /// The "Remove all app data" dialog is open.
    pub confirm_wipe: bool,
    pub notice: Option<Notice>,
    /// A recycle or undo in progress.
    pub busy: Option<Job>,
    pub skipped_setup: bool,
    pub needs_rescan: bool,
    pub update: Arc<Mutex<UpdateState>>,
    pub update_dismissed: bool,
    pub activity: Vec<Entry>,
    pub trail: Trail,
    pub downloads: Downloads,
    pub agentdata: AgentData,
    /// Duplicate groups where the user picked which copy to keep.
    pub dup_keep: std::collections::HashMap<String, PathBuf>,
    pub junk: Junk,
    pub deps: Deps,
    /// Whether the weekly check is scheduled; asked when Settings opens.
    pub weekly: Option<schedule::Status>,
    pub list_scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// The last update state the window drew.
    update_seen: UpdateState,
    /// Whether the timer that picks up background results is running. It
    /// only runs while something is in flight, so an idle app stays asleep.
    polling: bool,
    _poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AgentCleanup {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search paths"));
        let sub = cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.search_text = state.read(cx).value().to_string();
                cx.notify();
            }
        });
        let (cfg, damaged) = Config::load_checked();
        let cache = ScanCache::load(&cfg);
        let mut app = Self {
            home: directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()),
            lib: Library::default(),
            scan: None,
            last_scan: None,
            cached_at: None,
            page: Page::Leftovers,
            filter: Filter::Strays,
            sort: SortBy::Size,
            search,
            search_text: String::new(),
            focused: None,
            confirm: None,
            confirm_wipe: false,
            notice: None,
            busy: None,
            skipped_setup: false,
            needs_rescan: false,
            update: Arc::new(Mutex::new(UpdateState::Idle)),
            update_dismissed: false,
            activity: Vec::new(),
            trail: Trail::default(),
            downloads: Downloads {
                dir: downloads::downloads_dir(),
                ..Downloads::default()
            },
            agentdata: AgentData::default(),
            dup_keep: Default::default(),
            junk: Junk::default(),
            deps: Deps::default(),
            weekly: None,
            list_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            update_seen: UpdateState::Idle,
            polling: false,
            _poll: None,
            _subscriptions: vec![sub],
            cfg,
        };
        window.focus(&app.focus, cx);
        if let Some(bad) = damaged {
            let text = format!(
                "Your settings file was damaged, so the app started with default settings. The old file was kept as {}. Add your project folders again on the Projects page.",
                app.short(&bad)
            );
            app.notify_text(text, true);
        }
        if app.cfg.check_for_updates {
            app.check_for_updates();
        }
        // A moved exe would leave the weekly check pointing at nothing.
        std::thread::spawn(agent_cleanup::schedule::repair_if_moved);
        // Temp files from an update or setup that was interrupted.
        std::thread::spawn(agent_cleanup::footprint::sweep_leftovers);
        if let Some(cache) = cache {
            app.cached_at = Some(cache.scanned_at);
            app.lib = Library::new(cache.repos);
        } else if !app.cfg.project_roots.is_empty() {
            app.start_scan();
        }
        app
    }

    // ---- state helpers -------------------------------------------------

    pub fn show_welcome(&self) -> bool {
        self.cfg.project_roots.is_empty()
            && !self.skipped_setup
            && self.lib.is_empty()
            && self.scan.is_none()
    }

    pub fn short(&self, p: &Path) -> String {
        display_path(p, self.home.as_deref())
    }

    pub fn notify_text(&mut self, text: impl Into<String>, is_error: bool) {
        self.notice = Some(Notice::new(text, is_error));
    }

    fn save_cache(&self) {
        let cache = ScanCache {
            format: agent_cleanup::cache::FORMAT,
            scanned_at: self.cached_at.unwrap_or_else(SystemTime::now),
            repos: self
                .lib
                .repos()
                .iter()
                .filter(|r| r.inspected)
                .cloned()
                .collect(),
        };
        let _ = cache.save();
    }

    pub fn save_config(&mut self) {
        if let Err(e) = self.cfg.save() {
            self.notify_text(format!("Couldn't save settings: {e}"), true);
        }
        self.lib.reclassify(&self.cfg);
    }

    /// True while something is running that the window has to hear back
    /// from: a scan, a page loading, a recycle, an update, or a notice
    /// waiting to disappear.
    fn is_waiting(&self) -> bool {
        self.scan.is_some()
            || self.busy.is_some()
            || self.notice.is_some()
            || self.trail.loading.is_some()
            || self.downloads.loading.is_some()
            || self.agentdata.loading.is_some()
            || self.junk.loading.is_some()
            || self.junk.busy.is_some()
            || self.deps.loading.is_some()
            || matches!(
                self.update_state(),
                UpdateState::Checking | UpdateState::Downloading(..)
            )
    }

    /// Starts the [`POLL`] timer if something is in flight and it isn't
    /// already running. Called on every draw, and every click that starts
    /// work ends in a draw. The timer stops itself once nothing is left.
    fn ensure_polling(&mut self, cx: &mut Context<Self>) {
        if self.polling || !self.is_waiting() {
            return;
        }
        self.polling = true;
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let go_on = this.update(cx, |this, cx| {
                    this.tick(cx);
                    this.polling = this.is_waiting();
                    this.polling
                });
                if !matches!(go_on, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Runs every [`POLL`] while work is in flight: picks up scan results,
    /// finished jobs and update progress, and expires the notice. Only
    /// redraws when something changed.
    fn tick(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.poll_scan()
            | self.poll_job()
            | self.poll_trail()
            | self.poll_downloads()
            | self.poll_agentdata()
            | self.poll_junk()
            | self.poll_deps();
        if self.junk.loading.is_some() || self.deps.loading.is_some() {
            changed = true; // progress counter
        }
        if let Some(n) = &self.notice
            && n.shown_at.elapsed() > Duration::from_secs(NOTICE_SECS)
        {
            self.notice = None;
            changed = true;
        }
        let update = self.update_state();
        if update != self.update_seen {
            self.update_seen = update;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    // ---- background recycling ------------------------------------------

    /// Hands the steps to a background thread. The result is picked up by
    /// [`Self::poll_job`]. One job runs at a time.
    fn start_job(&mut self, label: String, kind: JobKind, keys: Vec<PathBuf>, ops: Vec<Op>) {
        if ops.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(jobs::run(ops));
        });
        self.notice = None;
        self.busy = Some(Job {
            rx,
            label,
            kind,
            keys,
        });
    }

    /// Says so and returns true if a recycle or undo is still running.
    fn is_busy(&mut self) -> bool {
        if self.busy.is_some() {
            self.notify_text(
                "Still finishing the last clean-up. Try again in a moment.",
                true,
            );
        }
        self.busy.is_some()
    }

    fn poll_job(&mut self) -> bool {
        let Some(job) = &self.busy else {
            return false;
        };
        let outcomes = match job.rx.try_recv() {
            Ok(o) => o,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        let Some(job) = self.busy.take() else {
            return false;
        };
        if outcomes.is_empty() {
            self.notify_text(
                "The clean-up stopped before it finished. Scan again to see what's still there.",
                true,
            );
            return true;
        }
        let results: Vec<(PathBuf, Outcome)> = job.keys.into_iter().zip(outcomes).collect();
        match job.kind {
            JobKind::Repos => self.finish_repos(results),
            JobKind::Trail => self.finish_trail(results),
            JobKind::Downloads => self.finish_downloads(results),
            JobKind::AgentData => self.finish_agentdata(results),
            JobKind::Undo(notice) => self.finish_undo(*notice, results),
        }
        true
    }

    // ---- updates -------------------------------------------------------

    pub fn update_state(&self) -> UpdateState {
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

    pub fn check_for_updates(&mut self) {
        self.update_dismissed = false;
        let state = self.update.clone();
        Self::set_update_state(&state, UpdateState::Checking);
        std::thread::spawn(move || {
            let new = match update::check() {
                Ok(Some(u)) => UpdateState::Available(u),
                Ok(None) => UpdateState::UpToDate,
                Err(e) => UpdateState::Failed(format!("Couldn't check for updates: {e}")),
            };
            Self::set_update_state(&state, new);
        });
    }

    pub fn install_update(&mut self, u: Update, cx: &mut Context<Self>) {
        if !cfg!(windows) {
            cx.open_url(&u.page_url);
            return;
        }
        let state = self.update.clone();
        Self::set_update_state(&state, UpdateState::Downloading(0, Some(u.size)));
        std::thread::spawn(move || {
            let progress = state.clone();
            let result = update::install(&u, move |done, total| {
                Self::set_update_state(&progress, UpdateState::Downloading(done, total));
            });
            let new = match result {
                Ok(()) => UpdateState::Installed(u.version.clone()),
                Err(e) => UpdateState::Failed(format!("Update failed: {e}. Nothing was changed.")),
            };
            Self::set_update_state(&state, new);
        });
    }

    // ---- scanning ------------------------------------------------------

    pub fn start_scan(&mut self) {
        if let Some(old) = &self.scan {
            old.cancel.store(true, Ordering::Relaxed);
        }
        self.lib.clear();
        self.focused = None;
        self.cached_at = None;
        self.needs_rescan = false;
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        scan::start(self.cfg.clone(), tx, cancel.clone(), Arc::new(|| {}));
        self.scan = Some(Scan {
            rx,
            cancel,
            dirs: 0,
            current: PathBuf::new(),
        });
    }

    pub fn stop_scan(&mut self) {
        if let Some(s) = &self.scan {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Returns true if anything arrived.
    fn poll_scan(&mut self) -> bool {
        let Some(scan) = &mut self.scan else {
            return false;
        };
        let mut any = false;
        let mut finished = None;
        while let Ok(ev) = scan.rx.try_recv() {
            any = true;
            if let ScanEvent::Progress { dirs, current } = &ev {
                scan.dirs = *dirs;
                scan.current = current.clone();
            }
            if let Some(done) = self.lib.apply(ev, &self.cfg) {
                finished = Some(done);
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
        any
    }

    // ---- actions -------------------------------------------------------

    pub fn go(&mut self, page: Page) {
        if page == Page::Activity {
            self.activity = activity::load(500);
        }
        if page == Page::Settings && self.weekly.is_none() {
            self.weekly = Some(schedule::status());
        }
        if page == Page::Trail && !self.trail.loaded && self.trail.loading.is_none() {
            self.load_trail();
        }
        if page == Page::Downloads && !self.downloads.loaded && self.downloads.loading.is_none() {
            self.load_downloads();
        }
        if page == Page::AgentData && !self.agentdata.loaded && self.agentdata.loading.is_none() {
            self.load_agentdata();
        }
        if page == Page::Junk && !self.junk.loaded && self.junk.loading.is_none() {
            self.load_junk();
        }
        if page == Page::Updates && !self.deps.loaded && self.deps.loading.is_none() {
            self.load_deps();
        }
        self.page = page;
    }

    /// Recycles what a confirm dialog was about.
    pub fn run_recycle(&mut self, what: ConfirmFor, paths: &[PathBuf]) {
        match what {
            ConfirmFor::Repos | ConfirmFor::Duplicates => self.recycle(paths),
            ConfirmFor::Trail => self.recycle_trail(paths),
            ConfirmFor::Junk => self.recycle_junk(paths),
            ConfirmFor::Downloads => self.recycle_downloads(paths),
            ConfirmFor::AgentData => self.recycle_agentdata(paths),
        }
    }

    pub fn ask_recycle(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.confirm = Some(Confirm {
                paths,
                ack: false,
                what: ConfirmFor::Repos,
            });
        }
    }

    pub fn ask_recycle_dupes(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.confirm = Some(Confirm {
                paths,
                ack: false,
                what: ConfirmFor::Duplicates,
            });
        }
    }

    pub fn ask_recycle_trail(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.confirm = Some(Confirm {
                paths,
                ack: false,
                what: ConfirmFor::Trail,
            });
        }
    }

    pub fn ask_recycle_downloads(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.confirm = Some(Confirm {
                paths,
                ack: false,
                what: ConfirmFor::Downloads,
            });
        }
    }

    pub fn ask_recycle_agentdata(&mut self, keys: Vec<PathBuf>) {
        if !keys.is_empty() {
            self.confirm = Some(Confirm {
                paths: keys,
                ack: false,
                what: ConfirmFor::AgentData,
            });
        }
    }

    pub fn ask_recycle_junk(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() && self.junk.busy.is_none() {
            self.confirm = Some(Confirm {
                paths,
                ack: false,
                what: ConfirmFor::Junk,
            });
        }
    }

    // ---- dependency updates --------------------------------------------

    pub fn load_deps(&mut self) {
        if let Some((_, cancel, _)) = &self.deps.loading {
            cancel.store(true, Ordering::Relaxed);
        }
        let all: Vec<PathBuf> = self.lib.repos().iter().map(|r| r.path.clone()).collect();
        let repos = deps::eligible(&all, &self.cfg);
        self.deps.repos = repos.len();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        let (stop, count) = (cancel.clone(), done.clone());
        std::thread::spawn(move || {
            let _ = tx.send(deps::check_all(&repos, &stop, &count));
        });
        self.deps.loading = Some((rx, cancel, done));
    }

    pub fn deps_progress(&self) -> usize {
        self.deps
            .loading
            .as_ref()
            .map_or(0, |(_, _, d)| d.load(Ordering::Relaxed))
    }

    fn poll_deps(&mut self) -> bool {
        let Some((rx, _, _)) = &self.deps.loading else {
            return false;
        };
        match rx.try_recv() {
            Ok(reports) => self.deps.reports = reports,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {}
        }
        self.deps.loaded = true;
        self.deps.loading = None;
        true
    }

    /// Opens a terminal running the update, but only when git shows no
    /// uncommitted changes, so `git diff` shows exactly what the update did
    /// and `git checkout .` undoes it.
    pub fn run_update(&mut self, repo: &Path, eco: Eco) {
        let Some(report) = self
            .deps
            .reports
            .iter()
            .find(|r| r.repo == repo && r.eco == eco)
        else {
            return;
        };
        let Some(cmd) = report.update_command() else {
            return;
        };
        let (dir, name) = (report.dir.clone(), self.short(repo));
        match agent_cleanup::git::inspect(repo) {
            Ok(g) if g.changed_files == 0 => match actions::run_in_terminal(&dir, &cmd) {
                Ok(()) => self.notify_text(
                    format!("Opened a terminal running {cmd} in {name}. Check again when it's done."),
                    false,
                ),
                Err(e) => self.notify_text(format!("Couldn't open a terminal: {e}"), true),
            },
            Ok(g) => self.notify_text(
                format!(
                    "{name} has {} unsaved edit{}. Save them in git first (commit or stash), so you can see and undo what the update changes.",
                    g.changed_files,
                    if g.changed_files == 1 { "" } else { "s" }
                ),
                true,
            ),
            Err(e) => self.notify_text(
                format!("Couldn't read git state for {name} ({e}), so the update wasn't started."),
                true,
            ),
        }
    }

    // ---- build junk ----------------------------------------------------

    pub fn load_junk(&mut self) {
        if let Some((_, cancel, _)) = &self.junk.loading {
            cancel.store(true, Ordering::Relaxed);
        }
        let repos: Vec<(PathBuf, Option<SystemTime>)> = self
            .lib
            .repos()
            .iter()
            .map(|r| (r.path.clone(), r.last_activity))
            .collect();
        self.junk.repos = repos.len();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        let (cfg, stop, count) = (self.cfg.clone(), cancel.clone(), done.clone());
        std::thread::spawn(move || {
            let _ = tx.send(junk::find(&repos, &cfg, &stop, &count));
        });
        self.junk.loading = Some((rx, cancel, done));
    }

    pub fn junk_progress(&self) -> usize {
        self.junk
            .loading
            .as_ref()
            .map_or(0, |(_, _, d)| d.load(Ordering::Relaxed))
    }

    pub fn junk_item(&self, p: &Path) -> Option<&junk::Item> {
        self.junk.items.iter().find(|i| i.path == p)
    }

    fn poll_junk(&mut self) -> bool {
        let mut changed = false;
        if let Some((rx, _, _)) = &self.junk.loading {
            match rx.try_recv() {
                Ok(items) => {
                    self.junk.items = items;
                    self.junk.loaded = true;
                    self.junk.loading = None;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.junk.loaded = true;
                    self.junk.loading = None;
                    changed = true;
                }
            }
        }
        let job = match &self.junk.busy {
            Some((rx, _)) => match rx.try_recv() {
                Ok(job) => Some(job),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(JunkJob {
                    restore: false,
                    done: Vec::new(),
                    failed: vec!["the background task stopped".into()],
                }),
            },
            None => None,
        };
        if let Some(job) = job {
            self.junk.busy = None;
            self.finish_junk_job(job);
            changed = true;
        }
        changed
    }

    fn start_junk_job(&mut self, items: Vec<junk::Item>, restore: bool) {
        if items.is_empty() || self.junk.busy.is_some() {
            return;
        }
        let n = items.len();
        let what = if n == 1 {
            format!("{} from {}", items[0].name(), items[0].repo_name())
        } else {
            format!("{n} folders")
        };
        let label = if restore {
            format!("Putting back {what}...")
        } else {
            format!("Moving {what} to the Recycle Bin. Big folders can take a minute.")
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut job = JunkJob {
                restore,
                done: Vec::new(),
                failed: Vec::new(),
            };
            for it in items {
                let res = if restore {
                    actions::restore(&it.path)
                } else {
                    actions::recycle(&it.path)
                };
                match res {
                    Ok(()) => job.done.push(it),
                    Err(e) => job.failed.push(format!("{} ({e})", it.name())),
                }
            }
            let _ = tx.send(job);
        });
        self.junk.busy = Some((rx, label));
    }

    pub fn recycle_junk(&mut self, paths: &[PathBuf]) {
        let items: Vec<junk::Item> = paths
            .iter()
            .filter_map(|p| self.junk_item(p).cloned())
            .collect();
        self.start_junk_job(items, false);
    }

    fn finish_junk_job(&mut self, job: JunkJob) {
        let n = job.done.len();
        let bytes: u64 = job.done.iter().map(|i| i.size_bytes).sum();
        let mut text = match (job.restore, n) {
            (_, 0) => String::new(),
            (true, n) => format!("Put back {n} folder{}.", if n == 1 { "" } else { "s" }),
            (false, 1) => format!(
                "Moved {} from {} to the Recycle Bin ({}). Empty the bin to get the space back.",
                job.done[0].name(),
                job.done[0].repo_name(),
                human_size(bytes)
            ),
            (false, n) => format!(
                "Moved {n} build folders to the Recycle Bin ({}). Empty the bin to get the space back.",
                human_size(bytes)
            ),
        };
        if !job.failed.is_empty() {
            text.push_str(&format!(
                " Couldn't {}: {}.",
                if job.restore { "put back" } else { "move" },
                job.failed.join(", ")
            ));
        }
        if job.restore {
            self.junk.items.extend(job.done);
            self.junk
                .items
                .sort_by_key(|i| std::cmp::Reverse(i.size_bytes));
            self.notify_text(text.trim(), !job.failed.is_empty());
        } else {
            self.junk
                .items
                .retain(|i| !job.done.iter().any(|d| d.path == i.path));
            self.notice = Some(Notice {
                undo_junk: job.done,
                ..Notice::new(text, !job.failed.is_empty())
            });
        }
    }

    // ---- agent data ----------------------------------------------------

    pub fn load_agentdata(&mut self) {
        if let Some((_, cancel)) = &self.agentdata.loading {
            cancel.store(true, Ordering::Relaxed);
        }
        let Some(home) = self.home.clone() else {
            self.agentdata.loaded = true;
            return;
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (cfg, stop) = (self.cfg.clone(), cancel.clone());
        let repos: Vec<PathBuf> = self.lib.repos().iter().map(|r| r.path.clone()).collect();
        std::thread::spawn(move || {
            let known = agentdata::known_projects(&repos, &cfg);
            let _ = tx.send(agentdata::find(&home, &cfg, &known, &stop));
        });
        self.agentdata.loading = Some((rx, cancel));
    }

    fn poll_agentdata(&mut self) -> bool {
        let Some((rx, _)) = &self.agentdata.loading else {
            return false;
        };
        match rx.try_recv() {
            Ok(items) => {
                self.agentdata.items = items;
                self.agentdata.loaded = true;
                self.agentdata.loading = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.agentdata.loading = None;
                self.agentdata.loaded = true;
                true
            }
        }
    }

    pub fn orphan(&self, key: &Path) -> Option<&Orphan> {
        self.agentdata.items.iter().find(|o| o.key() == key)
    }

    pub fn hide_agentdata(&mut self, key: PathBuf) {
        self.agentdata.items.retain(|o| o.key() != key);
        self.notify_text(
            format!(
                "Hidden {}. Unhide it on the Projects page.",
                self.short(&key)
            ),
            false,
        );
        self.cfg.ignored.push(key);
        self.save_config();
    }

    /// Recycles every file and folder of each item. An item that only
    /// partly moved is put back, so nothing is left half removed.
    pub fn recycle_agentdata(&mut self, keys: &[PathBuf]) {
        if self.is_busy() {
            return;
        }
        let items: Vec<&Orphan> = keys.iter().filter_map(|k| self.orphan(k)).collect();
        let label = moving_label(match items[..] {
            [one] => format!("{}'s {} data", one.project_name(), one.agent.label()),
            _ => format!("agent data for {} projects", items.len()),
        });
        let ops = items
            .iter()
            .map(|o| Op::Recycle {
                paths: o.paths.clone(),
                recheck: false,
            })
            .collect();
        let keys = items.iter().map(|o| o.key().to_path_buf()).collect();
        self.start_job(label, JobKind::AgentData, keys, ops);
    }

    fn finish_agentdata(&mut self, results: Vec<(PathBuf, Outcome)>) {
        let mut moved = Vec::new();
        let mut failed = Vec::new();
        for (key, outcome) in results {
            let Some(o) = self.orphan(&key).cloned() else {
                continue;
            };
            match outcome {
                Outcome::Done => moved.push(o),
                Outcome::Failed(e) => failed.push(format!("{} ({e})", o.project_name())),
                Outcome::Changed(_) => {}
            }
        }
        self.agentdata
            .items
            .retain(|o| !moved.iter().any(|m| m.key() == o.key()));
        let freed: u64 = moved.iter().map(|o| o.size_bytes).sum();
        let mut text = match moved.len() {
            0 => String::new(),
            1 => format!(
                "Moved {}'s {} data to the Recycle Bin, freeing {}.",
                moved[0].project_name(),
                moved[0].agent.label(),
                human_size(freed)
            ),
            n => format!(
                "Moved agent data for {n} projects to the Recycle Bin, freeing {}.",
                human_size(freed)
            ),
        };
        if !failed.is_empty() {
            text.push_str(&format!(" Couldn't move: {}.", failed.join(", ")));
        }
        self.notice = Some(Notice {
            undo_agentdata: moved,
            ..Notice::new(text, !failed.is_empty())
        });
    }

    // ---- downloads -----------------------------------------------------

    pub fn load_downloads(&mut self) {
        if let Some((_, cancel)) = &self.downloads.loading {
            cancel.store(true, Ordering::Relaxed);
        }
        let Some(dir) = self.downloads.dir.clone() else {
            self.downloads.loaded = true;
            return;
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (cfg, home, stop) = (self.cfg.clone(), self.home.clone(), cancel.clone());
        let repos: Vec<PathBuf> = self.lib.repos().iter().map(|r| r.path.clone()).collect();
        std::thread::spawn(move || {
            let _ = tx.send(downloads::find(&dir, home.as_deref(), &repos, &cfg, &stop));
        });
        self.downloads.loading = Some((rx, cancel));
    }

    fn poll_downloads(&mut self) -> bool {
        let Some((rx, _)) = &self.downloads.loading else {
            return false;
        };
        match rx.try_recv() {
            Ok(items) => {
                self.downloads.items = items;
                self.downloads.loaded = true;
                self.downloads.loading = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.downloads.loading = None;
                self.downloads.loaded = true;
                true
            }
        }
    }

    pub fn download_item(&self, p: &Path) -> Option<&Download> {
        self.downloads.items.iter().find(|d| d.path == p)
    }

    pub fn hide_download(&mut self, p: PathBuf) {
        self.downloads.items.retain(|d| d.path != p);
        self.notify_text(
            format!("Hidden {}. Unhide it on the Projects page.", self.short(&p)),
            false,
        );
        self.cfg.ignored.push(p);
        self.save_config();
    }

    /// Moves a download into the project it matched. Undo moves it back.
    pub fn move_download(&mut self, p: &Path) {
        let Some(d) = self.download_item(p).cloned() else {
            return;
        };
        let Some(m) = d.matched.clone() else {
            return;
        };
        // Private-looking files are never moved into a project.
        if d.destination().is_none() {
            return;
        }
        match downloads::move_into(&d.path, &m.project) {
            Ok(to) => {
                self.downloads.items.retain(|x| x.path != d.path);
                let text = format!("Moved {} into {}.", d.name(), self.short(&m.project));
                self.notice = Some(Notice {
                    undo_downloads: vec![(d, Some(to))],
                    ..Notice::new(text, false)
                });
            }
            Err(e) => self.notify_text(format!("Couldn't move {}: {e}.", d.name()), true),
        }
    }

    pub fn recycle_downloads(&mut self, paths: &[PathBuf]) {
        if self.is_busy() {
            return;
        }
        let items: Vec<&Download> = paths.iter().filter_map(|p| self.download_item(p)).collect();
        let label = moving_label(match items[..] {
            [one] => one.name(),
            _ => format!("{} downloads", items.len()),
        });
        let keys: Vec<PathBuf> = items.iter().map(|d| d.path.clone()).collect();
        let ops = keys
            .iter()
            .map(|p| Op::Recycle {
                paths: vec![p.clone()],
                recheck: false,
            })
            .collect();
        self.start_job(label, JobKind::Downloads, keys, ops);
    }

    fn finish_downloads(&mut self, results: Vec<(PathBuf, Outcome)>) {
        let mut moved = Vec::new();
        let mut failed = Vec::new();
        for (p, outcome) in results {
            let Some(d) = self.download_item(&p).cloned() else {
                continue;
            };
            match outcome {
                Outcome::Done => moved.push(d),
                Outcome::Failed(e) => failed.push(format!("{} ({e})", d.name())),
                Outcome::Changed(_) => {}
            }
        }
        self.downloads
            .items
            .retain(|d| !moved.iter().any(|m| m.path == d.path));
        let freed: u64 = moved.iter().map(|d| d.size_bytes).sum();
        let mut text = match moved.len() {
            0 => String::new(),
            1 => format!(
                "Moved {} to the Recycle Bin, freeing {}.",
                moved[0].name(),
                human_size(freed)
            ),
            n => format!(
                "Moved {n} downloads to the Recycle Bin, freeing {}.",
                human_size(freed)
            ),
        };
        if !failed.is_empty() {
            text.push_str(&format!(" Couldn't move: {}.", failed.join(", ")));
        }
        self.notice = Some(Notice {
            undo_downloads: moved.into_iter().map(|d| (d, None)).collect(),
            ..Notice::new(text, !failed.is_empty())
        });
    }

    // ---- agent trail ---------------------------------------------------

    pub fn load_trail(&mut self) {
        if let Some((_, cancel)) = &self.trail.loading {
            cancel.store(true, Ordering::Relaxed);
        }
        let Some(home) = self.home.clone() else {
            self.trail.loaded = true;
            return;
        };
        let since = SystemTime::now() - Duration::from_secs(trail::DAYS * 86_400);
        self.trail.logs = trail::log_files(&home, since).len();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (cfg, stop) = (self.cfg.clone(), cancel.clone());
        std::thread::spawn(move || {
            let _ = tx.send(trail::collect(&cfg, &home, since, &stop));
        });
        self.trail.loading = Some((rx, cancel));
    }

    fn poll_trail(&mut self) -> bool {
        let Some((rx, _)) = &self.trail.loading else {
            return false;
        };
        match rx.try_recv() {
            Ok(items) => {
                self.trail.items = items;
                self.trail.loaded = true;
                self.trail.loading = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.trail.loading = None;
                self.trail.loaded = true;
                true
            }
        }
    }

    pub fn trail_item(&self, p: &Path) -> Option<&Item> {
        self.trail.items.iter().find(|i| i.path == p)
    }

    pub fn hide_trail(&mut self, p: PathBuf) {
        self.trail.items.retain(|i| i.path != p);
        self.notify_text(
            format!("Hidden {}. Unhide it on the Projects page.", self.short(&p)),
            false,
        );
        self.cfg.ignored.push(p);
        self.save_config();
    }

    pub fn recycle_trail(&mut self, paths: &[PathBuf]) {
        if self.is_busy() {
            return;
        }
        let items: Vec<&Item> = paths.iter().filter_map(|p| self.trail_item(p)).collect();
        let label = moving_label(match items[..] {
            [one] => one.name(),
            _ => format!("{} items", items.len()),
        });
        // A project copy that looked fully saved is read again first.
        let ops = items
            .iter()
            .map(|i| Op::Recycle {
                paths: vec![i.path.clone()],
                recheck: i.looks_saved(),
            })
            .collect();
        let keys = items.iter().map(|i| i.path.clone()).collect();
        self.start_job(label, JobKind::Trail, keys, ops);
    }

    fn finish_trail(&mut self, results: Vec<(PathBuf, Outcome)>) {
        let mut moved = Vec::new();
        let mut failed = Vec::new();
        let mut kept = Vec::new();
        for (p, outcome) in results {
            let Some(item) = self.trail.items.iter_mut().find(|i| i.path == p) else {
                continue;
            };
            match outcome {
                Outcome::Done => moved.push(item.clone()),
                Outcome::Failed(e) => failed.push(format!("{} ({e})", item.name())),
                Outcome::Changed(found) => {
                    item.apply_recheck(*found);
                    kept.push(item.name());
                }
            }
        }
        self.trail
            .items
            .retain(|i| !moved.iter().any(|m| m.path == i.path));
        // A clone can be on both pages; keep the Leftovers list in step.
        let gone: Vec<PathBuf> = moved.iter().map(|i| i.path.clone()).collect();
        if !self.lib.remove(&gone).is_empty() {
            if self.focused.as_ref().is_some_and(|f| gone.contains(f)) {
                self.focused = None;
            }
            self.save_cache();
        }
        let freed: u64 = moved.iter().map(|i| i.size_bytes).sum();
        let mut text = match moved.len() {
            0 => String::new(),
            1 => format!(
                "Moved {} to the Recycle Bin, freeing {}.",
                moved[0].name(),
                human_size(freed)
            ),
            n => format!(
                "Moved {n} items to the Recycle Bin, freeing {}.",
                human_size(freed)
            ),
        };
        if !failed.is_empty() {
            text.push_str(&format!(" Couldn't move: {}.", failed.join(", ")));
        }
        text.push_str(&left_alone(&kept));
        self.notice = Some(Notice {
            undo_trail: moved,
            ..Notice::new(text, !failed.is_empty() || !kept.is_empty())
        });
    }

    /// Adds or removes the weekly scheduled task.
    pub fn set_weekly(&mut self, on: bool) {
        let res = if on {
            schedule::enable()
        } else {
            schedule::disable()
        };
        match res {
            Ok(()) => self.notify_text(
                if on {
                    "Weekly check is on. It runs on Mondays, or the next time your PC is on."
                } else {
                    "Weekly check is off."
                },
                false,
            ),
            Err(e) => self.notify_text(format!("Couldn't change the weekly check: {e}"), true),
        }
        self.weekly = Some(schedule::status());
    }

    /// Deletes everything the app keeps on this PC and closes it, so nothing
    /// is saved again afterwards. On failure the app stays open and says why.
    pub fn remove_all_data(&mut self) {
        self.confirm_wipe = false;
        match agent_cleanup::footprint::remove_all() {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                self.notify_text(e.to_string(), true);
                self.weekly = Some(schedule::status());
            }
        }
    }

    /// Runs the background check now, as the scheduled task would.
    pub fn run_check_now(&mut self) {
        let started = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg(schedule::CHECK_ARG)
                .spawn()
        });
        match started {
            Ok(_) => self.notify_text(
                "Checking in the background. A notification appears if there's enough to clean.",
                false,
            ),
            Err(e) => self.notify_text(format!("Couldn't start the check: {e}"), true),
        }
    }

    pub fn add_project(&mut self, p: PathBuf) {
        if self.cfg.project_roots.iter().any(|f| f == &p) {
            return;
        }
        self.notify_text(
            format!(
                "{} is now a project folder. Nothing in it will be flagged.",
                self.short(&p)
            ),
            false,
        );
        self.cfg.project_roots.push(p);
        self.save_config();
    }

    pub fn remove_project(&mut self, p: &Path) {
        self.cfg.project_roots.retain(|r| r != p);
        self.save_config();
    }

    pub fn hide(&mut self, p: PathBuf) {
        self.lib.remove(std::slice::from_ref(&p));
        if self.focused.as_ref() == Some(&p) {
            self.focused = None;
        }
        self.notify_text(
            format!("Hidden {}. Unhide it on the Projects page.", self.short(&p)),
            false,
        );
        self.cfg.ignored.push(p);
        self.save_config();
        self.save_cache();
    }

    pub fn unhide(&mut self, p: &Path) {
        self.cfg.ignored.retain(|r| r != p);
        self.save_config();
        self.needs_rescan = true;
    }

    /// Puts a ready-to-paste agent prompt on the clipboard.
    pub fn copy_prompt(&mut self, prompt: Option<String>, cx: &mut Context<Self>) {
        let Some(text) = prompt else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.notify_text(
            "Prompt copied. Paste it into Claude Code or Codex, in any folder.",
            false,
        );
    }

    pub fn copy_path(&mut self, p: &Path, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(p.display().to_string()));
        self.notify_text("Path copied.", false);
    }

    pub fn pick_scan_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Scan this folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if !this.cfg.scan_roots.iter().any(|f| f == &dir) {
                    this.cfg.scan_roots.push(dir);
                    this.save_config();
                    this.needs_rescan = true;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn remove_scan_root(&mut self, p: &Path) {
        self.cfg.scan_roots.retain(|r| r != p);
        self.save_config();
        self.needs_rescan = true;
    }

    pub fn pick_project_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use as project folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.add_project(dir);
                if this.lib.is_empty() && this.scan.is_none() {
                    this.start_scan();
                } else {
                    this.needs_rescan = true;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn recycle(&mut self, paths: &[PathBuf]) {
        if self.is_busy() {
            return;
        }
        // Outer folders first, so a project copy inside another one that's
        // also going is known to have gone with it.
        let mut keys: Vec<PathBuf> = paths
            .iter()
            .filter(|p| self.lib.get(p).is_some())
            .cloned()
            .collect();
        keys.sort_by_key(|p| p.components().count());
        let label = moving_label(match &keys[..] {
            [one] => self.lib.get(one).map(|r| r.name()).unwrap_or_default(),
            _ => format!("{} project copies", keys.len()),
        });
        // The list can come from a scan saved days ago. Anything that looked
        // fully saved is read again first, and left alone if that changed.
        let ops = keys
            .iter()
            .map(|p| Op::Recycle {
                paths: vec![p.clone()],
                recheck: self.lib.looks_saved(p),
            })
            .collect();
        self.start_job(label, JobKind::Repos, keys, ops);
    }

    fn finish_repos(&mut self, results: Vec<(PathBuf, Outcome)>) {
        let mut moved_paths = Vec::new();
        let mut failed = Vec::new();
        let mut kept = Vec::new();
        for (p, outcome) in results {
            let Some(name) = self.lib.get(&p).map(|r| r.name()) else {
                continue;
            };
            match outcome {
                Outcome::Done => moved_paths.push(p),
                Outcome::Failed(e) => failed.push(format!("{name} ({e})")),
                Outcome::Changed(found) => {
                    self.lib.apply_recheck(&p, *found);
                    kept.push(name);
                }
            }
        }
        let moved = self.lib.remove(&moved_paths);
        self.trail.items.retain(|i| !moved_paths.contains(&i.path));
        // Build folders inside a recycled repo went with it.
        self.junk
            .items
            .retain(|i| !moved_paths.iter().any(|p| path_within(&i.path, p)));
        if self
            .focused
            .as_ref()
            .is_some_and(|f| moved_paths.contains(f))
        {
            self.focused = None;
        }
        self.save_cache();

        let freed: u64 = moved.iter().filter_map(|r| r.size_bytes).sum();
        let mut text = match moved.len() {
            0 => String::new(),
            1 => format!(
                "Moved {} to the Recycle Bin, freeing {}.",
                moved[0].name(),
                human_size(freed)
            ),
            n => format!(
                "Moved {n} project copies to the Recycle Bin, freeing {}.",
                human_size(freed)
            ),
        };
        if !failed.is_empty() {
            text.push_str(&format!(" Couldn't move: {}.", failed.join(", ")));
        }
        text.push_str(&left_alone(&kept));
        self.notice = Some(Notice {
            undo: moved,
            ..Notice::new(text, !failed.is_empty() || !kept.is_empty())
        });
    }

    pub fn undo(&mut self) {
        if self.busy.is_some() {
            return;
        }
        let Some(mut notice) = self.notice.take() else {
            return;
        };
        if !notice.undo_junk.is_empty() {
            let items = std::mem::take(&mut notice.undo_junk);
            self.start_junk_job(items, true);
        }
        // Outer folders first: putting one back brings back what's inside.
        notice.undo.sort_by_key(|r| r.path.components().count());
        let mut keys = Vec::new();
        let mut ops = Vec::new();
        for repo in &notice.undo {
            keys.push(repo.path.clone());
            ops.push(Op::Restore {
                paths: vec![repo.path.clone()],
                ok_if_present: true,
            });
        }
        for item in &notice.undo_trail {
            keys.push(item.path.clone());
            ops.push(Op::Restore {
                paths: vec![item.path.clone()],
                ok_if_present: false,
            });
        }
        for (d, moved_to) in &notice.undo_downloads {
            keys.push(d.path.clone());
            ops.push(match moved_to {
                Some(to) => Op::MoveBack {
                    from: to.clone(),
                    to: d.path.clone(),
                },
                None => Op::Restore {
                    paths: vec![d.path.clone()],
                    ok_if_present: false,
                },
            });
        }
        for o in &notice.undo_agentdata {
            keys.push(o.key().to_path_buf());
            ops.push(Op::Restore {
                paths: o.paths.clone(),
                ok_if_present: false,
            });
        }
        self.start_job(
            "Putting it back where it was...".to_string(),
            JobKind::Undo(Box::new(notice)),
            keys,
            ops,
        );
    }

    /// `results` are in the order [`Self::undo`] queued them: project
    /// copies, trail items, downloads, agent data.
    fn finish_undo(&mut self, notice: Notice, results: Vec<(PathBuf, Outcome)>) {
        let mut outcomes = results.into_iter().map(|(_, o)| o);
        let mut failed = Vec::new();
        let mut restored = 0;
        for repo in notice.undo {
            match outcomes.next() {
                Some(Outcome::Done) => {
                    restored += 1;
                    self.lib.push(repo);
                }
                Some(Outcome::Failed(e)) => failed.push(format!("{} ({e})", repo.name())),
                _ => {}
            }
        }
        for item in notice.undo_trail {
            match outcomes.next() {
                Some(Outcome::Done) => {
                    restored += 1;
                    // The Leftovers list only picks a repo up again on a scan.
                    self.needs_rescan |= item.is_repo();
                    self.trail.items.push(item);
                }
                Some(Outcome::Failed(e)) => failed.push(format!("{} ({e})", item.name())),
                _ => {}
            }
        }
        for (d, _) in notice.undo_downloads {
            match outcomes.next() {
                Some(Outcome::Done) => {
                    restored += 1;
                    self.downloads.items.push(d);
                }
                Some(Outcome::Failed(e)) => failed.push(format!("{} ({e})", d.name())),
                _ => {}
            }
        }
        for o in notice.undo_agentdata {
            match outcomes.next() {
                Some(Outcome::Done) => {
                    restored += 1;
                    self.agentdata.items.push(o);
                }
                Some(Outcome::Failed(e)) => failed.push(format!("{} ({e})", o.project_name())),
                _ => {}
            }
        }
        self.agentdata
            .items
            .sort_by_key(|o| std::cmp::Reverse(o.size_bytes));
        self.downloads
            .items
            .sort_by_key(|d| std::cmp::Reverse(d.arrived));
        self.trail.items.sort_by_key(|i| std::cmp::Reverse(i.when));
        self.save_cache();
        if failed.is_empty() {
            self.notify_text(
                format!(
                    "Put back {restored} item{}.",
                    if restored == 1 { "" } else { "s" }
                ),
                false,
            );
        } else {
            self.notify_text(
                format!(
                    "Couldn't put back {}. Restore it from the Recycle Bin instead.",
                    failed.join(", ")
                ),
                true,
            );
        }
    }

    /// Escape closes the topmost thing: dialog, then details, then toast.
    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key != "escape" {
            return;
        }
        if self.confirm_wipe {
            self.confirm_wipe = false;
        } else if self.confirm.is_some() {
            self.confirm = None;
        } else if self.focused.is_some() {
            self.focused = None;
        } else if self.notice.is_some() {
            self.notice = None;
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    // ---- layout --------------------------------------------------------

    fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let item = |id: &'static str, icon: IconName, tip: &'static str, page: Page| {
            rail_button(id, icon, tip, self.page == page).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.go(page);
                    cx.notify();
                },
            ))
        };
        let update_dot = matches!(self.update_state(), UpdateState::Available(_));
        v_flex()
            .w(px(68.))
            .h_full()
            .flex_none()
            .bg(RAIL)
            .items_center()
            .py_3()
            .gap_2()
            .child(
                div()
                    .size(px(44.))
                    .rounded(px(14.))
                    .bg(ACCENT)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::BroomSparkles, 22., TEXT)),
            )
            .child(div().w(px(28.)).h(px(2.)).rounded_full().bg(BORDER).my_1())
            .child(item(
                "nav-leftovers",
                IconName::FolderSearch,
                "Leftovers: copies of projects found outside your project folders",
                Page::Leftovers,
            ))
            .child(item(
                "nav-trail",
                IconName::Footprints,
                "Agent trail: files and downloads AI agents left outside their projects",
                Page::Trail,
            ))
            .child(item(
                "nav-downloads",
                IconName::Download,
                "Downloads: your Downloads folder, matched to the project each file was for",
                Page::Downloads,
            ))
            .child(item(
                "nav-agentdata",
                IconName::Ghost,
                "Agent data: old AI chat history from projects you deleted",
                Page::AgentData,
            ))
            .child(item(
                "nav-dupes",
                IconName::Copy,
                "Duplicates: the same project downloaded more than once",
                Page::Duplicates,
            ))
            .child(item(
                "nav-junk",
                IconName::Package,
                "Build junk: big library and build folders your projects can recreate",
                Page::Junk,
            ))
            .child(item(
                "nav-updates",
                IconName::CircleArrowUp,
                "Updates: newer versions of the libraries your projects use",
                Page::Updates,
            ))
            .child(item(
                "nav-projects",
                IconName::FolderHeart,
                "Projects: tell the app where your real work lives",
                Page::Projects,
            ))
            .child(item(
                "nav-activity",
                IconName::Logs,
                "Activity: everything the app moved or put back",
                Page::Activity,
            ))
            .child(div().flex_1())
            .child(item(
                "nav-help",
                IconName::CircleQuestionMark,
                "How this works, and what the words mean",
                Page::Help,
            ))
            .child(
                div()
                    .relative()
                    .child(item(
                        "nav-settings",
                        IconName::Settings,
                        "Settings",
                        Page::Settings,
                    ))
                    .when(update_dot, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(4.))
                                .right(px(4.))
                                .size(px(10.))
                                .rounded_full()
                                .bg(GREEN)
                                .border_2()
                                .border_color(RAIL),
                        )
                    }),
            )
    }
}

impl Render for AgentCleanup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_polling(cx);
        let body: AnyElement = if self.show_welcome() && self.page == Page::Leftovers {
            self.welcome(cx).into_any_element()
        } else {
            match self.page {
                Page::Leftovers => self.leftovers_page(window, cx).into_any_element(),
                Page::Trail => self.trail_page(cx).into_any_element(),
                Page::Downloads => self.downloads_page(cx).into_any_element(),
                Page::AgentData => self.agentdata_page(cx).into_any_element(),
                Page::Duplicates => self.dupes_page(cx).into_any_element(),
                Page::Junk => self.junk_page(cx).into_any_element(),
                Page::Updates => self.updates_page(cx).into_any_element(),
                Page::Projects => self.projects_page(cx).into_any_element(),
                Page::Activity => self.activity_page(cx).into_any_element(),
                Page::Settings => self.settings_page(cx).into_any_element(),
                Page::Help => self.help_page(cx).into_any_element(),
            }
        };
        h_flex()
            .id("root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .relative()
            .size_full()
            .bg(BG)
            .text_color(TEXT)
            .font_family("Inter")
            .text_size(px(14.))
            .child(self.rail(cx))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .children(self.update_banner(cx))
                    .child(div().flex_1().min_h_0().child(body)),
            )
            .children(self.toast(cx))
            .children(self.confirm_dialog(cx))
            .children(self.wipe_dialog(cx))
    }
}

/// What the toast says while a recycle runs.
fn moving_label(what: String) -> String {
    format!("Moving {what} to the Recycle Bin. Big folders can take a minute.")
}

/// What a notice says about things that were about to be recycled but
/// turned out to have changed since the list was made.
fn left_alone(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => format!(
            " Left {one} where it is: it changed since the last scan and now has work that isn't saved anywhere else."
        ),
        many => format!(
            " Left {} where they are: they changed since the last scan and now have work that isn't saved anywhere else.",
            many.join(", ")
        ),
    }
}

pub fn group_digits(n: u64) -> String {
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
