//! The root view: state, background work and what each click does.

use super::theme::*;
use super::widgets::*;
use agent_cleanup::actions;
use agent_cleanup::activity::{self, Entry};
use agent_cleanup::cache::ScanCache;
use agent_cleanup::config::Config;
use agent_cleanup::library::{Filter, Library, SortBy};
use agent_cleanup::model::{Repo, display_path, human_size};
use agent_cleanup::scan::{self, ScanEvent};
use agent_cleanup::trail::{self, Item};
use agent_cleanup::update::{self, Update};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
    Projects,
    Activity,
    Settings,
    Help,
}

#[derive(Clone)]
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
}

pub struct Confirm {
    pub paths: Vec<PathBuf>,
    pub ack: bool,
    /// The paths are agent trail items, not repos from the scan.
    pub trail: bool,
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
    pub notice: Option<Notice>,
    pub skipped_setup: bool,
    pub needs_rescan: bool,
    pub update: Arc<Mutex<UpdateState>>,
    pub update_dismissed: bool,
    pub activity: Vec<Entry>,
    pub trail: Trail,
    pub list_scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _poll: Task<()>,
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
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });

        let cfg = Config::load();
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
            notice: None,
            skipped_setup: false,
            needs_rescan: false,
            update: Arc::new(Mutex::new(UpdateState::Idle)),
            update_dismissed: false,
            activity: Vec::new(),
            trail: Trail::default(),
            list_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _poll: poll,
            _subscriptions: vec![sub],
            cfg,
        };
        window.focus(&app.focus, cx);
        if app.cfg.check_for_updates {
            app.check_for_updates();
        }
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
        self.notice = Some(Notice {
            text: text.into(),
            is_error,
            shown_at: Instant::now(),
            undo: Vec::new(),
            undo_trail: Vec::new(),
        });
    }

    fn save_cache(&self) {
        let cache = ScanCache {
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

    /// Runs every [`POLL`]: picks up scan results, update progress and
    /// expires the notice. Only redraws when something changed.
    fn tick(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.poll_scan() | self.poll_trail();
        if let Some(n) = &self.notice
            && n.shown_at.elapsed() > Duration::from_secs(NOTICE_SECS)
        {
            self.notice = None;
            changed = true;
        }
        if matches!(
            self.update_state(),
            UpdateState::Checking | UpdateState::Downloading(..)
        ) {
            changed = true;
        }
        if changed {
            cx.notify();
        }
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
        if page == Page::Trail && !self.trail.loaded && self.trail.loading.is_none() {
            self.load_trail();
        }
        self.page = page;
    }

    pub fn ask_recycle(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.confirm = Some(Confirm {
                paths,
                ack: false,
                trail: false,
            });
        }
    }

    pub fn ask_recycle_trail(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.confirm = Some(Confirm {
                paths,
                ack: false,
                trail: true,
            });
        }
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
        let mut moved = Vec::new();
        let mut failed = Vec::new();
        for p in paths {
            let Some(item) = self.trail_item(p).cloned() else {
                continue;
            };
            match actions::recycle(p) {
                Ok(()) => moved.push(item),
                Err(e) => failed.push(format!("{} ({e})", item.name())),
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
        self.notice = Some(Notice {
            text: text.trim().to_string(),
            is_error: !failed.is_empty(),
            shown_at: Instant::now(),
            undo: Vec::new(),
            undo_trail: moved,
        });
    }

    pub fn add_project(&mut self, p: PathBuf) {
        if self.cfg.project_roots.iter().any(|f| f == &p) {
            return;
        }
        self.notify_text(
            format!(
                "{} is now a project folder. Repos in it won't be flagged.",
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
        let mut moved_paths = Vec::new();
        let mut failed = Vec::new();
        for p in paths {
            let Some(repo) = self.lib.get(p) else {
                continue;
            };
            match actions::recycle(p) {
                Ok(()) => moved_paths.push(p.clone()),
                Err(e) => failed.push(format!("{} ({e})", repo.name())),
            }
        }
        let moved = self.lib.remove(&moved_paths);
        self.trail.items.retain(|i| !moved_paths.contains(&i.path));
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
                "Moved {n} repos to the Recycle Bin, freeing {}.",
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
            undo_trail: Vec::new(),
        });
    }

    pub fn undo(&mut self) {
        let Some(notice) = self.notice.take() else {
            return;
        };
        let mut failed = Vec::new();
        let mut restored = 0;
        for repo in notice.undo {
            match actions::restore(&repo.path) {
                Ok(()) => {
                    restored += 1;
                    self.lib.push(repo);
                }
                Err(e) => failed.push(format!("{} ({e})", repo.name())),
            }
        }
        for item in notice.undo_trail {
            match actions::restore(&item.path) {
                Ok(()) => {
                    restored += 1;
                    // The Leftovers list only picks a repo up again on a scan.
                    self.needs_rescan |= item.is_repo();
                    self.trail.items.push(item);
                }
                Err(e) => failed.push(format!("{} ({e})", item.name())),
            }
        }
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
        if self.confirm.is_some() {
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
                "Leftovers: repos found outside your projects",
                Page::Leftovers,
            ))
            .child(item(
                "nav-trail",
                IconName::Footprints,
                "Agent trail: files and clones agents left outside their projects",
                Page::Trail,
            ))
            .child(item(
                "nav-projects",
                IconName::FolderHeart,
                "Projects: the folders that hold your real work",
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
                "How this works",
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
        let body: AnyElement = if self.show_welcome() && self.page == Page::Leftovers {
            self.welcome(cx).into_any_element()
        } else {
            match self.page {
                Page::Leftovers => self.leftovers_page(window, cx).into_any_element(),
                Page::Trail => self.trail_page(cx).into_any_element(),
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
