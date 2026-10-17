//! The program: Yukimi's state, the messages that change it, and the work
//! it starts away from the interface thread.
//!
//! Nothing slow happens on the interface thread. Reading the system, the
//! store, the package index and the sources' newest versions all happen on
//! the kit's worker threads, each answering with a message; searching has
//! a thread of its own that answers only the newest query; an operation's
//! steps run in a thread that sends its progress now and then. Answers
//! from an older reading (its `epoch`) are dropped.
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use aizome_kit::prelude::*;
use yukimi_system::catalog::{self, Application};
use yukimi_system::diff::Diff;
use yukimi_system::generations::Generation;
use yukimi_system::index::{Package, PackageIndex};
use yukimi_system::setup::Kind;

use crate::checks::{self, UpdateCheck};
use crate::job::{Job, JobEvent, Outcome};
use crate::model::{Model, Store};
use crate::ops::{self, Operation, Step, SystemChange};
use crate::paths::Paths;

/// How many search results show.
pub const RESULTS: usize = 60;
/// How long a note stays in the status line.
pub const NOTE_FOR: Duration = Duration::from_secs(6);

/// The places in the sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Page {
    #[default]
    Overview,
    Installed,
    Discover,
    Updates,
    History,
    Storage,
}

impl Page {
    pub const ALL: [Page; 6] =
        [Page::Overview, Page::Installed, Page::Discover, Page::Updates, Page::History, Page::Storage];

    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Installed => "Installed",
            Page::Discover => "Discover",
            Page::Updates => "Updates",
            Page::History => "History",
            Page::Storage => "Storage",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Page::Overview => Icon::Snowflake,
            Page::Installed => Icon::Package,
            Page::Discover => Icon::Compass,
            Page::Updates => Icon::RefreshCw,
            Page::History => Icon::GitCommitHorizontal,
            Page::Storage => Icon::HardDrive,
        }
    }

    /// Its name on the command line (`--page updates`).
    pub fn id(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Installed => "installed",
            Page::Discover => "discover",
            Page::Updates => "updates",
            Page::History => "history",
            Page::Storage => "storage",
        }
    }

    pub fn from_id(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }
}

/// Where the system comes from: this computer (as [`Paths`] say), or a
/// sample read in full already.
#[derive(Clone)]
pub enum Source {
    System(Paths),
    Sample(Arc<Model>),
}

/// What runs an operation's steps: Nix and the helper, or (in tests)
/// stand-ins that print what they would.
#[derive(Clone)]
pub enum Runner {
    Real,
    Mock(Arc<dyn Fn(&Operation) -> Vec<Step> + Send + Sync>),
}

/// What time it is, for "3 days ago": now, or a sample's fixed moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    System,
    /// Unix time, and the hour of the day it is there.
    Fixed(i64, u32),
}

impl Clock {
    pub fn now(self) -> i64 {
        match self {
            Clock::System => yukimi_system::now(),
            Clock::Fixed(t, _) => t,
        }
    }

    pub fn hour(self) -> u32 {
        match self {
            Clock::System => aizome_kit::theme::local_hour(),
            Clock::Fixed(_, h) => h,
        }
    }
}

/// What a generation's row shows once opened: being worked out, or what
/// changed from the one before.
#[derive(Clone, Debug, PartialEq)]
pub enum Changes {
    Working,
    Ready {
        /// The helper kept the configuration it was built from.
        kept: bool,
        diff: Option<Diff>,
        store_read: bool,
        has_previous: bool,
    },
}

/// Choosing another branch for an input to follow.
pub struct BranchChooser {
    pub sheet: WindowKey,
    pub input: String,
    /// GitHub's owner and repository, whose branches can be listed.
    pub repo: Option<(String, String)>,
    /// The branch it follows now.
    pub current: Option<String>,
    pub typed: String,
    /// GitHub's answer, once it comes.
    pub branches: Option<Result<Vec<String>, String>>,
}

/// Something to take out of the system's packages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removal {
    /// A NixOS program switched on in `yukimi.nix`.
    Program(String),
    /// A package in `yukimi.nix`.
    Package(String),
    /// An entry of `environment.systemPackages` (no user) or of a user's
    /// `packages`.
    Listed { attr: String, user: Option<String> },
    /// An application listed in a system catalog's setting.
    App { setting: String, id: String, name: String },
    /// A package in the current user's own profile.
    ForMe(String),
}

#[derive(Clone)]
pub enum Msg {
    // Reading.
    Reload,
    Read { epoch: u64, model: Arc<Model> },
    StoreRead { epoch: u64, store: Arc<Store>, problems: Vec<String> },
    FlakeNixpkgs { epoch: u64, path: String },
    IndexCached { index: Arc<PackageIndex>, current: bool },
    IndexMade(Result<Arc<PackageIndex>, String>),
    /// Make the package index now (Discover's "Make the list").
    MakeIndex,

    // Places.
    Show(Page),

    // Updates.
    CheckUpdates,
    Checked(Result<Arc<UpdateCheck>, String>),
    /// Update these sources.
    Update(Vec<String>),
    ChooseBranch(String),
    Branches(Result<Vec<String>, String>),
    BranchTyped(String),
    Follow(String),

    // Discover.
    Search(String),
    Found { number: u64, query: String, index: usize, results: Arc<Vec<Package>> },
    Open(Package),
    TryIt(Package),
    InstallForMe(Package),
    InstallForEveryone(String),
    InstallApp(String),

    // Installed.
    Filter(String),
    ShowAll,
    Remove(Removal),

    // History.
    ToggleGeneration(u32),
    Changes(u32, Changes),
    ReturnTo(u32),

    // Storage.
    Older(usize),
    CleanUp,
    ToggleRoots,

    // Operations.
    Confirm { heading: String, body: String, action: String, operation: Operation },
    Run(Operation),
    Job(u64, JobEvent),
    StopJob,
    CloseJob,
    JobDetails(bool),
    /// A finished job's sheet closes by itself.
    JobLinger(u64),
    /// Nix has been quiet: look whether to say so.
    JobQuiet,

    // Finishing an update.
    Restart,
    SignOut,
    Finished(Result<(), String>),

    // Everywhere.
    CloseSheet(WindowKey),
    OpenUrl(String),
    Note(String),
    NoteGone(u64),
    /// A later launch's arguments.
    Launched(Vec<String>),
}

/// A query for the search thread: its number, the text, and the index.
type Query = (u64, String, Arc<PackageIndex>);

/// What a search found: for which query, in which index, and the packages.
pub struct Found {
    pub query: String,
    pub index: usize,
    pub results: Arc<Vec<Package>>,
}

/// An index's identity, so results from an index since replaced aren't
/// shown for the new one.
pub fn index_id(index: &Arc<PackageIndex>) -> usize {
    Arc::as_ptr(index) as usize
}

pub struct Yukimi {
    pub source: Source,
    pub runner: Runner,
    pub clock: Clock,
    pub model: Arc<Model>,
    /// Reading the configuration and generations.
    pub loading: bool,
    /// Reading the store.
    pub reading_store: bool,
    epoch: u64,

    pub index: Option<Arc<PackageIndex>>,
    /// The index loaded is for another version of Nixpkgs than the
    /// system's, while the right one is made.
    pub index_stale: bool,
    pub indexing: bool,

    pub check: Option<Arc<UpdateCheck>>,
    pub checking: bool,

    pub page: Page,
    pub window: WindowKey,

    pub query: String,
    asked: u64,
    pub found: Option<Found>,
    searcher: Option<mpsc::Sender<Query>>,
    /// A package's details, in a sheet.
    pub details: Option<(WindowKey, Package)>,

    pub filter: String,
    pub show_all: bool,

    pub opened: BTreeSet<u32>,
    pub changes: HashMap<u32, Changes>,

    /// Delete generations older than: a week, a month, three months, or
    /// keep them all.
    pub older: usize,
    pub roots_open: bool,

    pub branch: Option<BranchChooser>,

    pub job: Option<Job>,
    pub job_sheet: Option<WindowKey>,
    jobs: u64,
    quiet_timer: Option<TimerHandle>,

    /// Notes in the status line, newest last.
    pub notes: VecDeque<(u64, String)>,
    notes_made: u64,
    /// Where later launches hand over their arguments, until Yukimi
    /// starts listening.
    pub listener: Option<std::os::unix::net::UnixListener>,
    /// Time spent in `update` and `view` over this (`YUKIMI_STALLS`).
    stalls: Option<Duration>,
}

impl Yukimi {
    /// Yukimi for this computer.
    pub fn new() -> Yukimi {
        Yukimi::with(Source::System(Paths::from_env()), Runner::Real, Clock::System)
    }

    /// Yukimi for the sample system: nothing is read or changed.
    pub fn sample() -> Yukimi {
        let model = Arc::new(crate::fixture::model());
        let mut app = Yukimi::with(Source::Sample(model.clone()), Runner::Mock(Arc::new(|_| Vec::new())), Clock::Fixed(crate::fixture::NOW, crate::fixture::HOUR));
        app.check = Some(Arc::new(crate::fixture::check(&model)));
        app.index = Some(Arc::new(crate::fixture::index()));
        app
    }

    pub fn with(source: Source, runner: Runner, clock: Clock) -> Yukimi {
        Yukimi {
            source,
            runner,
            clock,
            model: Arc::new(Model::default()),
            loading: true,
            reading_store: true,
            epoch: 0,
            index: None,
            index_stale: false,
            indexing: false,
            check: None,
            checking: false,
            page: Page::Overview,
            window: WindowKey::NONE,
            query: String::new(),
            asked: 0,
            found: None,
            searcher: None,
            details: None,
            filter: String::new(),
            show_all: false,
            opened: BTreeSet::new(),
            changes: HashMap::new(),
            older: 1,
            roots_open: false,
            branch: None,
            job: None,
            job_sheet: None,
            jobs: 0,
            quiet_timer: None,
            notes: VecDeque::new(),
            notes_made: 0,
            listener: None,
            stalls: std::env::var("YUKIMI_STALLS")
                .ok()
                .map(|v| Duration::from_millis(v.trim().parse().unwrap_or(50))),
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    /// Say how long `what` took, if it held up the interface for longer
    /// than `YUKIMI_STALLS` allows.
    pub fn stalled(&self, what: &str, started: Instant) {
        if let Some(limit) = self.stalls {
            let took = started.elapsed();
            if took > limit {
                eprintln!("yukimi: the interface was busy for {} ms ({what})", took.as_millis());
            }
        }
    }

    /// How updating would move each source, as the last check found.
    pub fn movements(&self) -> Option<std::collections::BTreeMap<String, yukimi_config::lock::Movement>> {
        self.check.as_ref().map(|c| checks::movements(&self.model, c))
    }

    /// Whether the configuration is one Sekka's single update knows.
    pub fn sekka(&self) -> bool {
        match &self.source {
            Source::System(_) => checks::is_sekka(&self.model),
            Source::Sample(_) => self.model.inputs.iter().any(|i| i.name == "sekka"),
        }
    }

    fn note(&mut self, text: impl Into<String>, cx: &mut Cx<Msg>) {
        self.notes_made += 1;
        self.notes.push_back((self.notes_made, text.into()));
        while self.notes.len() > 3 {
            self.notes.pop_front();
        }
        cx.after(NOTE_FOR, Msg::NoteGone(self.notes_made));
    }

    // Reading.

    /// Read everything again, away from the interface: the configuration
    /// and generations first, then the store. What was shown stays until
    /// its new version is read.
    fn reload(&mut self, cx: &mut Cx<Msg>) {
        self.epoch += 1;
        self.loading = true;
        let epoch = self.epoch;
        match self.source.clone() {
            Source::System(paths) => cx.spawn(move || Msg::Read { epoch, model: Arc::new(Model::quick(&paths)) }),
            Source::Sample(model) => cx.send(Msg::Read { epoch, model }),
        }
    }

    fn read(&mut self, epoch: u64, model: Arc<Model>, cx: &mut Cx<Msg>) {
        if epoch != self.epoch {
            return;
        }
        let sample = matches!(self.source, Source::Sample(_));
        let mut model = Arc::unwrap_or_clone(model);
        if !sample {
            model.store = self.model.store.clone();
        }
        if self.check.is_none() && !sample {
            self.check = checks::load(&checks::subject(&model)).map(Arc::new);
        }
        let flake_nixpkgs = (model.nixpkgs.is_none() && model.setup.kind == Kind::Flake && !sample).then(|| model.setup.flake());
        let generations = model.generations.clone();
        let store = model.store.clone();
        self.model = Arc::new(model);
        self.loading = false;
        self.ensure_index(false, cx);
        if self.page == Page::Updates {
            self.check_if_stale(cx);
        }
        if sample {
            self.reading_store = store.is_none();
            return;
        }
        self.reading_store = true;
        // A flake whose facts don't say which Nixpkgs it uses: ask Nix.
        if let Some(flake) = flake_nixpkgs {
            cx.spawn(move || match yukimi_system::nix::flake_nixpkgs(&flake) {
                Ok(path) => Msg::FlakeNixpkgs { epoch, path },
                Err(_) => Msg::FlakeNixpkgs { epoch, path: String::new() },
            });
        }
        cx.spawn(move || {
            let (store, problems) = Store::load(&generations);
            Msg::StoreRead { epoch, store: Arc::new(store), problems }
        });
    }

    /// Load the package index for the system's Nixpkgs, or make it (`build`
    /// makes it again even when there is one). While it is made, the index
    /// of an earlier Nixpkgs serves, if there is one. Made only once
    /// Discover has been shown, since it takes a minute and a few gigabytes
    /// of memory.
    fn ensure_index(&mut self, build: bool, cx: &mut Cx<Msg>) {
        if matches!(self.source, Source::Sample(_)) {
            return;
        }
        let wanted = self.page == Page::Discover || self.index.is_some();
        if self.indexing || !wanted || (self.index.is_some() && !self.index_stale && !build) {
            return;
        }
        let (Some(nixpkgs), Some(cache)) = (self.model.nixpkgs.clone(), crate::paths::cache_dir()) else {
            return;
        };
        self.indexing = true;
        let tx = cx.stream();
        cx.spawn(move || {
            // What is cached comes first: this Nixpkgs's index (Yukimi's own,
            // or the GTK Yukimi's), or an older one to use while this one is
            // made.
            let mut current = false;
            if !build {
                let shared = crate::paths::shared_cache_dir();
                let cached = [Some(cache.clone()), shared]
                    .into_iter()
                    .flatten()
                    .filter_map(|dir| yukimi_system::index::load_cached(&dir, &nixpkgs))
                    .max_by_key(|(_, current)| *current);
                if let Some((index, is_current)) = cached {
                    current = is_current;
                    tx.send(Msg::IndexCached { index: Arc::new(index), current });
                }
            }
            if current {
                return Msg::IndexMade(Err(String::new()));
            }
            Msg::IndexMade(yukimi_system::index::make(&cache, &nixpkgs).map(Arc::new))
        });
    }

    // Updates.

    /// Check for updates when the last check is older than a few minutes (or
    /// there hasn't been one), as when Updates is opened.
    fn check_if_stale(&mut self, cx: &mut Cx<Msg>) {
        let stale = self.check.as_ref().is_none_or(|c| c.stale(self.now()));
        let has_sources = !self.model.inputs.is_empty() || !self.model.channels.is_empty();
        if stale && !self.loading && has_sources && !matches!(self.source, Source::Sample(_)) {
            self.check_updates(cx);
        }
    }

    fn check_updates(&mut self, cx: &mut Cx<Msg>) {
        if self.checking {
            return;
        }
        if matches!(self.source, Source::Sample(_)) {
            self.note("The sample system has no sources to ask", cx);
            return;
        }
        self.checking = true;
        let model = self.model.clone();
        cx.spawn(move || Msg::Checked(checks::run(&model).map(Arc::new)));
    }

    // Discover.

    /// Send the current query to the search thread.
    fn ask(&mut self, cx: &mut Cx<Msg>) {
        let Some(index) = self.index.clone() else { return };
        if self.query.trim().is_empty() {
            return;
        }
        if self.found.as_ref().is_some_and(|f| f.query == self.query && f.index == index_id(&index)) {
            return;
        }
        self.asked += 1;
        let searcher = self.searcher.get_or_insert_with(|| {
            let (queries, incoming) = mpsc::channel::<Query>();
            let answer = cx.stream();
            let _ = std::thread::Builder::new().name("yukimi-search".into()).spawn(move || {
                while let Ok(mut query) = incoming.recv() {
                    // Only the newest query is answered.
                    while let Ok(newer) = incoming.try_recv() {
                        query = newer;
                    }
                    let (number, text, index) = query;
                    let results: Vec<Package> = index.search(&text, RESULTS).into_iter().cloned().collect();
                    let id = index_id(&index);
                    if !answer.send(Msg::Found { number, query: text, index: id, results: Arc::new(results) }) {
                        break;
                    }
                }
            });
            queries
        });
        let _ = searcher.send((self.asked, self.query.clone(), index));
    }

    // Operations.

    /// Why nothing can be changed from here, when it can't.
    fn cannot_run(&self, operation: &Operation) -> Option<String> {
        if let Some(job) = &self.job
            && job.running()
        {
            return Some("Yukimi is already changing the system; wait for that to finish".to_owned());
        }
        match &self.source {
            Source::System(paths) if paths.elsewhere => {
                Some(format!("Yukimi is only looking at {}, so it changes nothing", paths.etc.display()))
            }
            _ if operation.is_system() && matches!(operation, Operation::ChangeSystem { .. }) => {
                self.model.cannot_change_system()
            }
            _ => None,
        }
    }

    /// Run an operation in a sheet showing its progress, then read
    /// everything again.
    fn run(&mut self, operation: Operation, cx: &mut Cx<Msg>) {
        if let Some(why) = self.cannot_run(&operation) {
            self.note(why, cx);
            return;
        }
        self.close_details(cx);
        if let Some(sheet) = self.job_sheet.take() {
            cx.close_window(sheet);
        }
        self.jobs += 1;
        let id = self.jobs;
        let job = Job::new(operation.clone(), id);
        let stop = job.stop.clone();
        self.job = Some(job);
        self.job_sheet = Some(cx.show_sheet(self.window));
        let steps = match &self.runner {
            Runner::Real => operation.steps(&self.model),
            Runner::Mock(steps) => steps(&operation),
        };
        let tx = cx.stream();
        let system = operation.is_system();
        let _ = std::thread::Builder::new().name("yukimi-job".into()).spawn(move || {
            let sink = tx.clone();
            crate::job::work(
                steps,
                stop,
                move |event| {
                    sink.send(Msg::Job(id, event));
                },
                move |out| if system { crate::job::finish_report(out) } else { out },
            );
        });
        if let Some(t) = self.quiet_timer.take() {
            cx.cancel(t);
        }
        self.quiet_timer = Some(cx.every(Duration::from_secs(5), Msg::JobQuiet));
    }

    fn job_event(&mut self, id: u64, event: JobEvent, cx: &mut Cx<Msg>) {
        let Some(job) = self.job.as_mut().filter(|j| j.id == id) else { return };
        let Some(outcome) = job.take(event, Instant::now()).cloned() else { return };
        if let Some(t) = self.quiet_timer.take() {
            cx.cancel(t);
        }
        let update = job.operation.is_update();
        match outcome {
            Outcome::Done(text) => {
                if update {
                    self.show(Page::Updates, cx);
                } else {
                    self.note(text, cx);
                }
                cx.after(crate::job::LINGER, Msg::JobLinger(id));
            }
            Outcome::Failed(_) => {}
        }
        self.reload(cx);
    }

    fn close_job(&mut self, cx: &mut Cx<Msg>) {
        if self.job.as_ref().is_some_and(Job::running) {
            return;
        }
        if let Some(sheet) = self.job_sheet.take() {
            cx.close_window(sheet);
        }
        self.job = None;
    }

    /// Ask before an operation that changes the whole system.
    fn confirm(&mut self, heading: String, body: String, action: String, operation: Operation, cx: &mut Cx<Msg>) {
        if let Some(why) = self.cannot_run(&operation) {
            self.note(why, cx);
            return;
        }
        let alert = Alert::new(heading, body).cancel_button("Cancel").default_button(action, Msg::Run(operation));
        cx.alert(self.window, alert);
    }

    fn close_details(&mut self, cx: &mut Cx<Msg>) {
        if let Some((sheet, _)) = self.details.take() {
            cx.close_window(sheet);
        }
    }

    fn show(&mut self, page: Page, cx: &mut Cx<Msg>) {
        self.page = page;
        self.ensure_index(false, cx);
        match page {
            Page::Updates => self.check_if_stale(cx),
            Page::Discover => cx.focus("search"),
            _ => {}
        }
    }

    // What installing and removing change.

    fn install_for_everyone(&mut self, attr: String, cx: &mut Cx<Msg>) {
        let mut packages = self.model.configured.yukimi.packages.clone();
        if !packages.contains(&attr) {
            packages.push(attr.clone());
        }
        self.confirm(
            format!("Install {attr} for everyone?"),
            "Yukimi adds it to the system configuration and builds the new system, which asks for an administrator \
             password and can take a few minutes. The current version stays in History."
                .to_owned(),
            "Install".to_owned(),
            Operation::ChangeSystem {
                change: SystemChange { packages: Some(packages), ..SystemChange::default() },
                what: attr,
                adding: true,
            },
            cx,
        );
    }

    fn install_app(&mut self, app: &Application, cx: &mut Cx<Msg>) {
        let change = app_change(&self.model, app);
        self.confirm(
            format!("Install {}?", app.name),
            "It is installed for everyone on this computer. Yukimi changes the system configuration and builds the \
             new system, which asks for an administrator password. The current version stays in History."
                .to_owned(),
            "Install".to_owned(),
            Operation::ChangeSystem { change, what: app.name.clone(), adding: true },
            cx,
        );
    }

    fn remove(&mut self, removal: Removal, cx: &mut Cx<Msg>) {
        let model = self.model.clone();
        let mine = &model.configured.yukimi;
        let (what, change) = match removal {
            Removal::ForMe(name) => {
                cx.send(Msg::Run(Operation::RemoveForMe { name }));
                return;
            }
            Removal::Program(program) => {
                let name = model
                    .catalog
                    .iter()
                    .find(|a| a.program.as_ref() == Some(&program))
                    .map_or_else(|| program.clone(), |a| a.name.clone());
                let programs = mine.programs.iter().filter(|p| **p != program).cloned().collect();
                (name, SystemChange { programs: Some(programs), ..SystemChange::default() })
            }
            Removal::Package(attr) => {
                let packages = mine.packages.iter().filter(|p| **p != attr).cloned().collect();
                (attr, SystemChange { packages: Some(packages), ..SystemChange::default() })
            }
            Removal::Listed { attr, user: None } => {
                (attr.clone(), SystemChange { unlist_system: vec![attr], ..SystemChange::default() })
            }
            Removal::Listed { attr, user: Some(user) } => {
                (attr.clone(), SystemChange { unlist_user: Some((user, vec![attr])), ..SystemChange::default() })
            }
            Removal::App { setting, id, name } => {
                let ids = model.configured.settings.get(&setting).cloned().unwrap_or_default();
                let ids = ids.into_iter().filter(|i| *i != id).collect();
                (name, SystemChange { applications: Some((setting, ids)), ..SystemChange::default() })
            }
        };
        self.confirm(
            format!("Remove {what}?"),
            "It is removed for everyone on this computer. Yukimi changes the system configuration and builds the \
             new system, which asks for an administrator password. The current version stays in History."
                .to_owned(),
            "Remove".to_owned(),
            Operation::ChangeSystem { change, what, adding: false },
            cx,
        );
    }

    /// Open a generation's row, working out what changed in it.
    fn toggle_generation(&mut self, number: u32, cx: &mut Cx<Msg>) {
        if !self.opened.remove(&number) {
            self.opened.insert(number);
        } else {
            return;
        }
        if matches!(self.changes.get(&number), Some(Changes::Ready { store_read: true, .. })) {
            return;
        }
        let gens = &self.model.generations;
        let Some(i) = gens.iter().position(|g| g.number == number) else { return };
        let this: Generation = gens[i].clone();
        let previous: Option<Generation> = i.checked_sub(1).map(|p| gens[p].clone());
        let store = self.model.store.clone();
        let sample = matches!(self.source, Source::Sample(_));
        self.changes.insert(number, Changes::Working);
        cx.spawn(move || {
            let kept = sample || yukimi_system::saved_configuration(number).is_dir();
            let diff = match (&store, &previous) {
                (Some(store), Some(previous)) => {
                    let (before, after) = (store.closure_of(&previous.target), store.closure_of(&this.target));
                    Some(yukimi_system::diff::diff(&store.graph, &before, &after))
                }
                _ => None,
            };
            Msg::Changes(number, Changes::Ready { kept, diff, store_read: store.is_some(), has_previous: previous.is_some() })
        });
    }

    fn choose_branch(&mut self, input: String, cx: &mut Cx<Msg>) {
        let Some(found) = self.model.inputs.iter().find(|i| i.name == input) else { return };
        let Some(locked) = &found.locked else { return };
        let source = found.original.as_ref().unwrap_or(locked).describe();
        let current = yukimi_config::branch::of(&source);
        let repo = (locked.kind == "github").then(|| locked.owner.clone().zip(locked.repo.clone())).flatten();
        if let Some((owner, name)) = repo.clone()
            && !matches!(self.source, Source::Sample(_))
        {
            cx.spawn(move || Msg::Branches(yukimi_system::updates::github_branches(&owner, &name)));
        }
        let sheet = cx.show_sheet(self.window);
        self.branch = Some(BranchChooser { sheet, input, repo, current, typed: String::new(), branches: None });
        cx.focus("branch");
    }

    fn follow(&mut self, branch: String, cx: &mut Cx<Msg>) {
        let Some(chooser) = self.branch.take() else { return };
        let branch = branch.trim().to_owned();
        if !yukimi_config::branch::valid(&branch) {
            self.note(format!("“{branch}” isn't a branch name Yukimi can use"), cx);
            self.branch = Some(chooser);
            return;
        }
        cx.close_window(chooser.sheet);
        let input = chooser.input;
        if Some(&branch) == chooser.current.as_ref() {
            self.note(format!("{input} already follows {branch}"), cx);
            return;
        }
        self.confirm(
            format!("Follow {branch}?"),
            format!(
                "Yukimi points {input} at the newest commit of {branch} and builds your system with it, which asks for \
                 an administrator password. Updates of {input} come from {branch} from then on. The current version \
                 stays in History."
            ),
            "Follow".to_owned(),
            Operation::FollowBranch { input, branch },
            cx,
        );
    }

    fn finish(&mut self, reboot: bool, cx: &mut Cx<Msg>) {
        if matches!(self.source, Source::Sample(_)) {
            self.note("The sample system doesn't restart", cx);
            return;
        }
        let expected = self.model.completion.action.clone();
        cx.spawn(move || {
            // Respect logind's inhibitors and authorization: never force a
            // restart past another user's session or a running workload.
            if yukimi_system::completion::read().action != expected {
                return Msg::Finished(Err("The update's status changed; checking again".to_owned()));
            }
            let mut command = if reboot {
                let mut c = std::process::Command::new("systemctl");
                c.args(["--check-inhibitors=yes", "reboot"]);
                c
            } else {
                let mut c = std::process::Command::new("loginctl");
                c.arg("terminate-session").arg(std::env::var("XDG_SESSION_ID").unwrap_or_default());
                c
            };
            match command.output() {
                Ok(out) if out.status.success() => Msg::Finished(Ok(())),
                Ok(out) => Msg::Finished(Err(format!("Could not finish: {}", String::from_utf8_lossy(&out.stderr).trim()))),
                Err(_) => Msg::Finished(Err("Could not start that. Your session is still running.".to_owned())),
            }
        });
    }
}

impl Default for Yukimi {
    fn default() -> Yukimi {
        Yukimi::new()
    }
}

/// What installing an application changes: its setting's list for one from
/// a system catalog, otherwise `yukimi.nix`.
pub fn app_change(model: &Model, app: &Application) -> SystemChange {
    if let Some(setting) = &app.setting {
        let mut ids = model.configured.settings.get(setting).cloned().unwrap_or_default();
        for id in std::iter::once(&app.id).chain(&app.requires) {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        return SystemChange { applications: Some((setting.clone(), ids)), ..SystemChange::default() };
    }
    let mine = &model.configured.yukimi;
    let mut change = SystemChange::default();
    if !app.packages.is_empty() {
        let mut packages = mine.packages.clone();
        packages.extend(app.packages.iter().filter(|p| !mine.packages.contains(p)).cloned());
        change.packages = Some(packages);
    }
    if let Some(program) = &app.program {
        let mut programs = mine.programs.clone();
        if !programs.contains(program) {
            programs.push(program.clone());
        }
        change.programs = Some(programs);
    }
    change
}

/// Why something can't be installed for everyone, if it can't.
pub fn blocked(model: &Model, unfree: bool) -> Option<String> {
    model
        .cannot_change_system()
        .or_else(|| (unfree && !model.allow_unfree).then(|| "This system doesn't allow unfree software".to_owned()))
}

impl Program for Yukimi {
    type Msg = Msg;

    fn info(&self) -> AppInfo {
        AppInfo::new("org.aizome.Yukimi", "Yukimi", "Software").icon(Icon::Snowflake).version(env!("CARGO_PKG_VERSION"))
    }

    fn launched(&mut self, cx: &mut Cx<Msg>) {
        self.window = cx.open_window(WindowSpec::new("Yukimi").size(1120.0, 760.0).min_size(760.0, 520.0));
        self.reload(cx);
        if let Some(listener) = self.listener.take() {
            let tx = cx.stream();
            crate::instance::listen(listener, move |args| tx.send(Msg::Launched(args)));
        }
        self.show(self.page, cx);
    }

    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        let started = Instant::now();
        match msg {
            Msg::Reload => self.reload(cx),
            Msg::Read { epoch, model } => self.read(epoch, model, cx),
            Msg::StoreRead { epoch, store, problems } => {
                if epoch == self.epoch {
                    self.model = Arc::new(self.model.with_store(Arc::into_inner(store).unwrap_or_default(), problems));
                    self.reading_store = false;
                    // What generations changed is worked out again with the new store.
                    let stale: Vec<u32> = self.changes.keys().copied().collect();
                    self.changes.clear();
                    let reopen: Vec<u32> = stale.into_iter().filter(|n| self.opened.remove(n)).collect();
                    for number in reopen {
                        self.toggle_generation(number, cx);
                    }
                }
            }
            Msg::FlakeNixpkgs { epoch, path } => {
                if epoch == self.epoch && !path.is_empty() {
                    self.model = Arc::new(Model { nixpkgs: Some(path), ..(*self.model).clone() });
                    self.ensure_index(false, cx);
                }
            }
            Msg::IndexCached { index, current } => {
                self.index = Some(index);
                self.index_stale = !current;
                self.ask(cx);
            }
            Msg::IndexMade(made) => {
                self.indexing = false;
                match made {
                    Ok(index) => {
                        self.index = Some(index);
                        self.index_stale = false;
                        self.ask(cx);
                    }
                    Err(e) if !e.is_empty() => self.note(e, cx),
                    Err(_) => {}
                }
            }
            Msg::MakeIndex => self.ensure_index(true, cx),
            Msg::Show(page) => self.show(page, cx),

            Msg::CheckUpdates => self.check_updates(cx),
            Msg::Checked(result) => {
                self.checking = false;
                match result {
                    Ok(check) => self.check = Some(check),
                    Err(e) => self.note(format!("Couldn't check for updates: {e}"), cx),
                }
            }
            Msg::Update(inputs) => {
                let sekka = self.sekka();
                let operation = checks::update_operation(&self.model, inputs, sekka);
                self.run(operation, cx);
            }
            Msg::ChooseBranch(input) => self.choose_branch(input, cx),
            Msg::Branches(result) => {
                if let Some(chooser) = &mut self.branch {
                    chooser.branches = Some(result);
                }
            }
            Msg::BranchTyped(text) => {
                if let Some(chooser) = &mut self.branch {
                    chooser.typed = text;
                }
            }
            Msg::Follow(branch) => self.follow(branch, cx),

            Msg::Search(text) => {
                self.query = text;
                self.ensure_index(false, cx);
                self.ask(cx);
            }
            Msg::Found { number, query, index, results } => {
                if number == self.asked {
                    self.found = Some(Found { query, index, results });
                }
            }
            Msg::Open(package) => {
                self.close_details(cx);
                let sheet = cx.show_sheet(self.window);
                self.details = Some((sheet, package));
            }
            Msg::TryIt(package) => {
                let command =
                    ops::try_command(&self.model.nixpkgs_ref(), &package.attr, &package.main_program, package.unfree);
                let tried = match &self.runner {
                    Runner::Real => ops::open_terminal(&command),
                    Runner::Mock(_) => Ok(()),
                };
                match tried {
                    Ok(()) => self.note(format!("Trying {} in a terminal", package.attr), cx),
                    Err(e) => self.note(e, cx),
                }
            }
            Msg::InstallForMe(package) => {
                self.run(Operation::InstallForMe { attr: package.attr.clone(), unfree: package.unfree }, cx)
            }
            Msg::InstallForEveryone(attr) => {
                self.close_details(cx);
                match catalog::needing_system(&self.model.catalog, &attr).cloned() {
                    Some(app) => self.install_app(&app, cx),
                    None => self.install_for_everyone(attr, cx),
                }
            }
            Msg::InstallApp(id) => {
                if let Some(app) = self.model.catalog.iter().find(|a| a.id == id).cloned() {
                    self.install_app(&app, cx);
                }
            }

            Msg::Filter(text) => {
                self.filter = text;
                self.show_all = false;
            }
            Msg::ShowAll => self.show_all = true,
            Msg::Remove(removal) => self.remove(removal, cx),

            Msg::ToggleGeneration(number) => self.toggle_generation(number, cx),
            Msg::Changes(number, changes) => {
                if self.opened.contains(&number) {
                    self.changes.insert(number, changes);
                }
            }
            Msg::ReturnTo(number) => {
                let kept = matches!(self.changes.get(&number), Some(Changes::Ready { kept: true, .. }));
                self.confirm(
                    format!("Return to generation {number}?"),
                    if kept {
                        "The running system switches to this version, after an administrator password, and your \
                         choices go back to what they were then (files you have changed by hand since stay as they \
                         are). Newer generations are kept, so you can come back."
                    } else {
                        "The running system switches to this version, after an administrator password. Yukimi didn't \
                         keep the choices it was made from, so your next change builds on today's choices. Newer \
                         generations are kept, so you can come back."
                    }
                    .to_owned(),
                    "Return".to_owned(),
                    Operation::Rollback { generation: number },
                    cx,
                );
            }

            Msg::Older(choice) => self.older = choice.min(3),
            Msg::CleanUp => {
                let days = [Some(7), Some(30), Some(90), None][self.older.min(3)];
                self.confirm(
                    "Clean up the store?".to_owned(),
                    match days {
                        Some(d) => format!(
                            "System generations older than {d} days are deleted, then everything nothing needs is \
                             removed. This asks for an administrator password."
                        ),
                        None => "Everything nothing needs is removed; all generations are kept. This asks for an \
                                 administrator password."
                            .to_owned(),
                    },
                    "Clean up".to_owned(),
                    Operation::Clean { older_than_days: days },
                    cx,
                );
            }
            Msg::ToggleRoots => self.roots_open = !self.roots_open,

            Msg::Confirm { heading, body, action, operation } => self.confirm(heading, body, action, operation, cx),
            Msg::Run(operation) => self.run(operation, cx),
            Msg::Job(id, event) => self.job_event(id, event, cx),
            Msg::StopJob => {
                if let Some(job) = &mut self.job {
                    job.request_stop();
                }
            }
            Msg::CloseJob => self.close_job(cx),
            Msg::JobDetails(on) => {
                if let Some(job) = &mut self.job {
                    job.details = on;
                }
            }
            Msg::JobLinger(id) => {
                if self.job.as_ref().is_some_and(|j| j.id == id && matches!(j.outcome, Some(Outcome::Done(_)))) {
                    self.close_job(cx);
                }
            }
            Msg::JobQuiet => {}

            Msg::Restart => self.finish(true, cx),
            Msg::SignOut => self.finish(false, cx),
            Msg::Finished(result) => {
                if let Err(e) = result {
                    self.note(e, cx);
                }
                self.reload(cx);
            }

            Msg::CloseSheet(sheet) => {
                if self.details.as_ref().is_some_and(|(s, _)| *s == sheet) {
                    self.details = None;
                }
                if self.branch.as_ref().is_some_and(|b| b.sheet == sheet) {
                    self.branch = None;
                }
                cx.close_window(sheet);
            }
            Msg::OpenUrl(url) => {
                if matches!(self.runner, Runner::Real) {
                    cx.open_url(&url);
                }
            }
            Msg::Note(text) => self.note(text, cx),
            Msg::NoteGone(id) => self.notes.retain(|(n, _)| *n != id),
            Msg::Launched(args) => {
                if let Ok(Some(page)) = page_from_args(&args) {
                    self.show(page, cx);
                }
                cx.activate(self.window);
            }
        }
        self.stalled("update", started);
    }

    fn view(&self, window: WindowKey) -> Element<Msg> {
        let started = Instant::now();
        let view = crate::ui::view(self, window);
        self.stalled("view", started);
        view
    }

    fn menus(&self) -> MenuBar<Msg> {
        crate::ui::menus(self)
    }

    fn close_requested(&mut self, window: WindowKey, cx: &mut Cx<Msg>) -> bool {
        if window == self.window && self.job.as_ref().is_some_and(Job::running) {
            cx.alert(
                self.window,
                Alert::new(
                    "Yukimi is changing the system",
                    "Wait for it to finish, or stop it first. Stopping puts your configuration back as it was.",
                )
                .default_button("Keep Going", Msg::Note(String::new()))
                .button("Stop", Msg::StopJob),
            );
            return false;
        }
        if self.details.as_ref().is_some_and(|(s, _)| *s == window) {
            self.details = None;
        }
        if self.branch.as_ref().is_some_and(|b| b.sheet == window) {
            self.branch = None;
        }
        if self.job_sheet == Some(window) {
            if self.job.as_ref().is_some_and(Job::running) {
                return false;
            }
            self.job_sheet = None;
            self.job = None;
        }
        true
    }

    fn open(&mut self, paths: Vec<PathBuf>, cx: &mut Cx<Msg>) {
        let _ = (paths, cx);
    }
}

/// The page asked for on the command line (`--page updates`), if any.
pub fn page_from_args(args: &[String]) -> Result<Option<Page>, String> {
    let mut page = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--page" => match it.next().and_then(|p| Page::from_id(p)) {
                Some(p) => page = Some(p),
                None => {
                    return Err("--page takes one of overview, installed, discover, updates, history, storage".into());
                }
            },
            "--sample" => {}
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            _ => {}
        }
    }
    Ok(page)
}
